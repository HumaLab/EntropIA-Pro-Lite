"""
EntropIA local GLM-OCR engine — layout-aware OCR subprocess.

Layout: PP-DocLayoutV3 on CPU (the same layout model the official GLM-OCR SDK
uses). Recognition: GLM-OCR served by a llama-server the Rust side keeps alive
on the GPU, called once per layout box over its OpenAI-compatible API.

Every detected box is transcribed and returned with its label. The official
SDK discards header/footer/number/footnote/aside_text/reference boxes before
recognition (its `label_task_mapping.abandon` list); EntropIA keeps them on
purpose: journal names, volumes, dates and page numbers are metadata the RAG
index needs.

Usage:
    python glm_ocr_local.py <image_path> --server-url http://127.0.0.1:PORT

Output (stdout): sentinelled JSON with the same schema as paddle_vl.py, so the
Rust side parses and persists both engines through one path.
Exit codes: 0=success, 1=error
"""

import os

# Layout runs on CPU: the GPU belongs to llama-server, and Paddle-GPU inference
# is unreliable on some driver/wheel combinations. Must be set before paddle
# is imported.
os.environ["CUDA_VISIBLE_DEVICES"] = ""
os.environ.setdefault("FLAGS_use_mkldnn", "1")
os.environ.setdefault("FLAGS_enable_pir_api", "0")
os.environ.setdefault("PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK", "True")

import argparse  # noqa: E402
import base64  # noqa: E402
import io  # noqa: E402
import json  # noqa: E402
import sys  # noqa: E402
import time  # noqa: E402
import traceback  # noqa: E402
import urllib.error  # noqa: E402
import urllib.request  # noqa: E402
import warnings  # noqa: E402
from concurrent.futures import ThreadPoolExecutor  # noqa: E402

warnings.filterwarnings("ignore")
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", line_buffering=True)
sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", line_buffering=True)

SENTINEL_BEGIN = "===VL_JSON_BEGIN==="
SENTINEL_END = "===VL_JSON_END==="

LAYOUT_MODEL = "PP-DocLayoutV3"
# Official GLM-OCR task prompts; every other label is read as text.
TASK_PROMPTS = {
    "table": "Table Recognition:",
    "display_formula": "Formula Recognition:",
    "inline_formula": "Formula Recognition:",
}
TEXT_PROMPT = "Text Recognition:"
# Boxes thinner than this cannot hold a readable glyph; they are kept in the
# layout but not sent to the model.
MIN_SIDE_PX = 8


def map_label_to_category(label: str) -> str:
    """Map PP-DocLayoutV3 labels to the Rust LayoutCategory names."""
    return {
        "doc_title": "title",
        "paragraph_title": "title",
        "image": "figure",
        "chart": "figure",
        "table": "table",
        "figure_title": "caption",
        "header": "header",
        "header_image": "header",
        "footer": "footer",
        "footer_image": "footer",
        "number": "footer",
        "footnote": "footnote",
        "vision_footnote": "footnote",
        "reference": "reference",
        "reference_content": "reference",
        "algorithm": "code",
    }.get(label, "plain_text")


def format_block_text(label: str, content: str) -> str | None:
    """Format one block for the concatenated text. Nothing is dropped by label."""
    if not content.strip():
        return None
    if label in ("doc_title", "paragraph_title"):
        return f"## {content}"
    if label == "table":
        return f"---\n{content}\n---"
    return content


def recognize(server_url: str, png_bytes: bytes, prompt: str, max_tokens: int, timeout: float) -> str:
    image_url = "data:image/png;base64," + base64.b64encode(png_bytes).decode("ascii")
    body = json.dumps({
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image_url", "image_url": {"url": image_url}},
                {"type": "text", "text": prompt},
            ],
        }],
        "temperature": 0,
        "max_tokens": max_tokens,
        "stream": False,
    }).encode("utf-8")
    request = urllib.request.Request(
        server_url.rstrip("/") + "/v1/chat/completions",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        payload = json.loads(response.read().decode("utf-8"))
    return (payload["choices"][0]["message"].get("content") or "").strip()


def main():
    parser = argparse.ArgumentParser(description="Local GLM-OCR subprocess for EntropIA")
    parser.add_argument("image_path")
    parser.add_argument("--server-url", required=True)
    parser.add_argument("--max-tokens", type=int, default=2048)
    parser.add_argument("--parallel", type=int, default=4)
    parser.add_argument("--request-timeout", type=float, default=300.0)
    args = parser.parse_args()

    try:
        started = time.time()
        from paddleocr import LayoutDetection
        from PIL import Image

        layout = LayoutDetection(model_name=LAYOUT_MODEL, device="cpu")
        boxes = []
        for res in layout.predict(args.image_path):
            boxes.extend(res["boxes"])
        t_layout = time.time()
        sys.stderr.write(f"[glm_ocr_local] Layout: {len(boxes)} boxes ({t_layout - started:.1f}s)\n")

        image = Image.open(args.image_path).convert("RGB")
        width, height = image.size

        entries = []
        for index, box in enumerate(boxes):
            label = str(box.get("label") or "text")
            x1, y1, x2, y2 = [int(round(float(v))) for v in (box.get("coordinate") or [0, 0, 0, 0])]
            x1, y1 = max(0, x1), max(0, y1)
            x2, y2 = min(width, x2), min(height, y2)
            raw_order = box.get("order")
            entries.append({
                "label": label,
                "bbox": (x1, y1, x2, y2),
                "score": float(box.get("score") or 0.0),
                "order": int(raw_order) if raw_order is not None else index + 1,
                "index": index,
            })

        def read(entry):
            x1, y1, x2, y2 = entry["bbox"]
            if x2 - x1 < MIN_SIDE_PX or y2 - y1 < MIN_SIDE_PX:
                return ""
            buffer = io.BytesIO()
            image.crop((x1, y1, x2, y2)).save(buffer, format="PNG")
            prompt = TASK_PROMPTS.get(entry["label"], TEXT_PROMPT)
            return recognize(args.server_url, buffer.getvalue(), prompt,
                             args.max_tokens, args.request_timeout)

        with ThreadPoolExecutor(max_workers=max(1, args.parallel)) as pool:
            contents = list(pool.map(read, entries))
        t_ocr = time.time()

        blocks, regions, parts = [], [], []
        for entry, content in sorted(zip(entries, contents), key=lambda p: (p[0]["order"], p[0]["index"])):
            x1, y1, x2, y2 = entry["bbox"]
            bbox = {"x": x1, "y": y1, "width": x2 - x1, "height": y2 - y1}
            blocks.append({
                "label": entry["label"],
                "content": content,
                "bbox": bbox,
                "order": entry["order"],
                "group_id": 0,
            })
            regions.append({
                "category": map_label_to_category(entry["label"]),
                "bbox": bbox,
                "confidence": round(entry["score"], 4),
            })
            formatted = format_block_text(entry["label"], content)
            if formatted is not None:
                parts.append(formatted)

        result = {
            "text": "\n\n".join(parts),
            "method": "glm_ocr_local",
            "blocks": blocks,
            "regions": regions,
            "image_width": width,
            "image_height": height,
            "actual_device": "llama_server",
        }
        sys.stdout.write(f"{SENTINEL_BEGIN}\n{json.dumps(result, ensure_ascii=False)}\n{SENTINEL_END}\n")
        sys.stdout.flush()
        sys.stderr.write(
            f"[glm_ocr_local] Done: {len(blocks)} blocks, OCR {t_ocr - t_layout:.1f}s, "
            f"total {time.time() - started:.1f}s\n"
        )
    except Exception as exc:  # noqa: BLE001 - reported to Rust as structured JSON
        tb = traceback.format_exc()
        if isinstance(exc, urllib.error.URLError):
            message = f"GLM-OCR server unreachable at {args.server_url}: {exc}"
        else:
            message = f"{type(exc).__name__}: {exc}"
        sys.stderr.write(f"Error: {message}\nTraceback:\n{tb}\n")
        error = {"error": message, "exception_type": type(exc).__name__,
                 "traceback_tail": "\n".join(tb.splitlines()[-12:])}
        sys.stdout.write(f"{SENTINEL_BEGIN}\n{json.dumps(error, ensure_ascii=False)}\n{SENTINEL_END}\n")
        sys.stdout.flush()
        sys.exit(1)


if __name__ == "__main__":
    main()
