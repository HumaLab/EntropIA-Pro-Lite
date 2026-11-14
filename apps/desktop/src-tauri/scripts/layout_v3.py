#!/usr/bin/env python3
"""EntropIA layout-only subprocess: PP-DocLayoutV3 boxes in reading order.

Runs the same layout model the GLM-OCR engine uses, without any recognition.
The lightweight OCRH path (plain PaddleOCR) reads text segment by segment
from these boxes instead of the full page, so columns come out in order.

Usage:
    python layout_v3.py <image_path> [--device cpu]

Output (stdout): sentinelled JSON, one entry per box:
    {"method": "layout_v3", "image_width": W, "image_height": H,
     "actual_device": "cpu",
     "boxes": [{"label": ..., "category": ..., "bbox": {"x","y","width","height"},
                "confidence": ..., "order": ...}]}
Exit codes: 0=success, 1=error (sentinelled {"error": ...} on stdout).
"""

import os

# Layout runs on CPU: on GPU machines the GPU belongs to llama-server, and
# Paddle-GPU inference is unreliable on some driver/wheel combinations. Must
# be set before paddle is imported.
os.environ["CUDA_VISIBLE_DEVICES"] = ""
os.environ.setdefault("FLAGS_use_mkldnn", "1")
os.environ.setdefault("FLAGS_enable_pir_api", "0")
os.environ.setdefault("PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK", "True")

import argparse  # noqa: E402
import io  # noqa: E402
import json  # noqa: E402
import sys  # noqa: E402
import time  # noqa: E402
import traceback  # noqa: E402
import warnings  # noqa: E402

warnings.filterwarnings("ignore")
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", line_buffering=True)
sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", line_buffering=True)

SENTINEL_BEGIN = "===VL_JSON_BEGIN==="
SENTINEL_END = "===VL_JSON_END==="

LAYOUT_MODEL = "PP-DocLayoutV3"


def map_label_to_category(label: str) -> str:
    """Map PP-DocLayoutV3 labels to Rust LayoutCategory-like names.

    Mirrors glm_ocr_local.py so both engines agree on what each box is.
    """
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


def main() -> None:
    parser = argparse.ArgumentParser(description="EntropIA layout-only subprocess")
    parser.add_argument("image_path")
    parser.add_argument("--device", default="cpu")
    args = parser.parse_args()

    try:
        started = time.time()
        from paddleocr import LayoutDetection
        from PIL import Image

        device = args.device if args.device in ("cpu", "gpu") else "cpu"
        layout = LayoutDetection(model_name=LAYOUT_MODEL, device=device)
        boxes = []
        for res in layout.predict(args.image_path):
            boxes.extend(res["boxes"])
        t_layout = time.time()
        sys.stderr.write(f"[layout_v3] Layout: {len(boxes)} boxes ({t_layout - started:.1f}s)\n")

        image = Image.open(args.image_path).convert("RGB")
        width, height = image.size

        out_boxes = []
        for index, box in enumerate(boxes):
            label = str(box.get("label") or "text")
            x1, y1, x2, y2 = [int(round(float(v))) for v in (box.get("coordinate") or [0, 0, 0, 0])]
            x1, y1 = max(0, x1), max(0, y1)
            x2, y2 = min(width, x2), min(height, y2)
            if x2 <= x1 or y2 <= y1:
                continue
            raw_order = box.get("order")
            out_boxes.append({
                "label": label,
                "category": map_label_to_category(label),
                "bbox": {"x": x1, "y": y1, "width": x2 - x1, "height": y2 - y1},
                "confidence": round(float(box.get("score") or 0.0), 4),
                "order": int(raw_order) if raw_order is not None else index + 1,
            })
        out_boxes.sort(key=lambda b: b["order"])

        result = {
            "method": "layout_v3",
            "image_width": width,
            "image_height": height,
            "actual_device": device,
            "boxes": out_boxes,
        }
        sys.stdout.write(f"{SENTINEL_BEGIN}\n{json.dumps(result, ensure_ascii=False)}\n{SENTINEL_END}\n")
        sys.stdout.flush()
        sys.stderr.write(f"[layout_v3] Done: {len(out_boxes)} boxes, total {time.time() - started:.1f}s\n")
    except Exception as exc:  # noqa: BLE001 - reported to Rust as structured JSON
        tb = traceback.format_exc()
        message = f"{type(exc).__name__}: {exc}"
        sys.stderr.write(f"Error: {message}\nTraceback:\n{tb}\n")
        error = {"error": message, "exception_type": type(exc).__name__,
                 "traceback_tail": "\n".join(tb.splitlines()[-12:])}
        sys.stdout.write(f"{SENTINEL_BEGIN}\n{json.dumps(error, ensure_ascii=False)}\n{SENTINEL_END}\n")
        sys.stdout.flush()
        sys.exit(1)


if __name__ == "__main__":
    main()
