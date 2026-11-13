"""
EntropIA PaddleOCR-VL Engine — layout-aware OCR subprocess.

Called by the Rust backend via std::process::Command.
Receives an image file path, outputs JSON to stdout.

Uses PaddleOCR-VL which does layout detection + OCR in a single pass.
Returns both text content (per block, in reading order) and layout regions.

Usage:
    python paddle_vl.py <image_path> [--device cpu|gpu]

Output (stdout): Sentinelled JSON — starts with SENTINEL_BEGIN\n and ends with \nSENTINEL_END
                  This allows the Rust side to extract clean JSON even if other
                  libraries write to stdout unexpectedly.
Errors (stderr): Human-readable error messages.
Exit codes: 0=success, 1=error
"""

import sys
import os
import json
import warnings
import argparse
import io
import tempfile
import time
import traceback

# Suppress warnings that could corrupt JSON output on stdout
warnings.filterwarnings("ignore")

# Disable HuggingFace Hub symlink-based download and transfer.
os.environ["HF_HUB_ENABLE_HF_TRANSFER"] = "0"
os.environ["HF_HUB_SYMLINK_STORAGE"] = "0"
os.environ.setdefault("PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK", "True")
if os.environ.get("PADDLE_PDX_CACHE_HOME") and not os.environ.get("MODELSCOPE_CACHE"):
    _modelscope_cache = os.path.join(os.environ["PADDLE_PDX_CACHE_HOME"], "modelscope")
    os.environ["MODELSCOPE_CACHE"] = _modelscope_cache
    os.environ.setdefault("MODELSCOPE_HOME", _modelscope_cache)

# CPU performance tuning — apply BEFORE importing paddle/numpy.
# These envvars only take effect at library import time. Setting them after
# import is a no-op. The Rust caller may override via subprocess env, but we
# set sane defaults here in case the script is invoked standalone.
_cpu_threads = os.environ.get("OMP_NUM_THREADS")
if _cpu_threads is None:
    _cpu_count = os.cpu_count() or 4
    # Use all physical cores but cap at 8 to avoid oversubscription on CPUs with SMT
    _threads = str(min(_cpu_count, 8))
    os.environ["OMP_NUM_THREADS"] = _threads
    os.environ["MKL_NUM_THREADS"] = _threads
    os.environ["OPENBLAS_NUM_THREADS"] = _threads
sys.stderr.write(f"[paddle_vl] CPU threads: OMP={os.environ.get('OMP_NUM_THREADS')}, MKL={os.environ.get('MKL_NUM_THREADS')}\n")

# Enable Paddle's MKL-DNN (oneDNN) acceleration for CPU inference.
# This typically gives 2-5x speedup on Intel/AMD CPUs.
os.environ.setdefault("FLAGS_use_mkldnn", "1")
os.environ.setdefault("FLAGS_use_avx", "1")

# Disable Paddle's new PIR (Paddle Intermediate Representation) executor.
# PIR is unstable and crashes with:
#   (Unimplemented) ConvertPirAttribute2RuntimeAttribute not support [...]
# on certain paddlepaddle / paddleocr version combinations.
# Falling back to the legacy executor avoids this.
os.environ.setdefault("FLAGS_enable_pir_api", "0")

# Force stdout to be unbuffered so the Rust side sees output promptly
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", line_buffering=True)
sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", line_buffering=True)

SENTINEL_BEGIN = "===VL_JSON_BEGIN==="
SENTINEL_END = "===VL_JSON_END==="


def _format_block_text(block_label: str, block_content: str) -> str | None:
    """Format block text based on its label for the full concatenated output.

    Returns None for blocks that should be skipped (e.g. images).
    """
    label = block_label.lower().strip()
    if label in ("doc_title", "paragraph_title"):
        return f"## {block_content}"
    if label == "table":
        return f"---\n{block_content}\n---"
    if label == "image":
        return None
    if label == "vision_footnote":
        return f"Note: {block_content}"
    return block_content


def _map_label_to_category(block_label: str) -> str:
    """Map PaddleOCR-VL block labels to Rust LayoutCategory-like names."""
    label = block_label.lower().strip()
    mapping = {
        "doc_title": "title",
        "paragraph_title": "title",
        "text": "plain_text",
        "image": "figure",
        "table": "table",
        "table_caption": "table_caption",
        "vision_footnote": "abandoned",
    }
    return mapping.get(label, "plain_text")


def _check_paddle_version():
    """Verify paddlepaddle is within the supported range."""
    import paddle
    ver_str = paddle.__version__.split("+")[0].split("-")[0]
    parts = ver_str.split(".")
    try:
        major, minor, patch = int(parts[0]), int(parts[1]), int(parts[2])
    except (IndexError, ValueError):
        return False, f"Unrecognized paddlepaddle version: {paddle.__version__}"
    if (major, minor) < (3, 2):
        return False, (
            f"paddlepaddle {paddle.__version__} is too old. "
            "paddleocr>=3.x requires paddlepaddle>=3.2.1 (missing AnalysisConfig.set_optimization_level). "
            "Install: pip install 'paddlepaddle>=3.2.1,<3.3.0'"
        )
    if (major, minor) >= (3, 3):
        return False, (
            f"paddlepaddle {paddle.__version__} has a confirmed CPU inference bug "
            "(oneDNN/PIR ConvertPirAttribute2RuntimeAttribute crash). "
            "Use: pip install 'paddlepaddle>=3.2.1,<3.3.0'"
        )
    return True, ""


def _tail_lines(text: str, max_lines: int = 12) -> str:
    lines = text.splitlines()
    return "\n".join(lines[-max_lines:])


def _describe_exception(exc: Exception, tb: str) -> str:
    """Keep the real root cause visible while adding actionable context."""
    raw = str(exc)
    combined = f"{raw}\n{tb}".lower()
    exc_name = type(exc).__name__

    if "permission denied" in combined and (
        ".paddlex" in combined
        or "official_models" in combined
        or "paddle_pdx_cache_home" in combined
    ):
        detail = f"{exc_name}: {raw}" if raw else exc_name
        return (
            "PaddleOCR-VL cannot read/write the PaddleX model cache "
            f"({detail}). Check the folder configured by PADDLE_PDX_CACHE_HOME "
            "or clear the corrupted PaddleX cache."
        )

    if "vlm" in combined and "worker" in combined:
        root = f"{exc_name}: {raw}" if raw else _tail_lines(tb, 6)
        return (
            "PaddleOCR-VL VLM inference failed in a worker. "
            f"Root cause: {root}"
        )

    if raw:
        return f"{exc_name}: {raw}"
    return f"{exc_name}: {_tail_lines(tb, 8)}"


def _init_pipeline(requested_device: str):
    """Initialize PaddleOCRVL pipeline with the requested device.

    If GPU is requested but fails (e.g. paddlepaddle-gpu not installed),
    falls back to CPU and returns the actual device used.
    """
    from paddleocr import PaddleOCRVL
    import inspect

    kwargs = {
        "use_doc_orientation_classify": False,
        "use_doc_unwarping": False,
        "use_layout_detection": True,
    }
    supported_params = inspect.signature(PaddleOCRVL).parameters
    if "use_queues" in supported_params:
        # Queue workers hide child exceptions on Windows/CPU and collapse them
        # into opaque "parallel worker" failures. EntropIA already isolates each
        # OCRH run in its own subprocess, so single-process inference is safer.
        kwargs["use_queues"] = False
    if "vl_rec_max_concurrency" in supported_params:
        kwargs["vl_rec_max_concurrency"] = 1

    # Try requested device first
    try:
        pipeline = PaddleOCRVL(device=requested_device, **kwargs)
        if requested_device == "gpu":
            _smoke_test_gpu(pipeline)
        sys.stderr.write(f"[paddle_vl] Pipeline initialized on device='{requested_device}'\n")
        return pipeline, requested_device
    except Exception as e:
        if requested_device == "gpu":
            sys.stderr.write(
                f"[paddle_vl] GPU unusable ({e.__class__.__name__}: {e}); "
                "falling back to CPU. GPU init can succeed while inference is "
                "broken (e.g. Paddle 3.2 event_waiter nullptr on some "
                "driver/wheel combos), so the smoke test runs a tiny predict. "
                "If you expected GPU acceleration, ensure paddlepaddle-gpu "
                "matches your CUDA driver.\n"
            )
            pipeline = PaddleOCRVL(device="cpu", **kwargs)
            sys.stderr.write("[paddle_vl] Pipeline initialized on device='cpu' (GPU fallback)\n")
            return pipeline, "cpu"
        raise


def _smoke_test_gpu(pipeline):
    """Run a tiny predict to prove GPU inference works before trusting it.

    Pipeline construction downloads/loads models but does not execute the
    kernels that break on some GPU setups; a failed smoke test must fall
    back to CPU instead of failing the whole OCRH run.
    """
    import tempfile

    from PIL import Image

    with tempfile.TemporaryDirectory() as tmpdir:
        probe_path = os.path.join(tmpdir, "probe.png")
        Image.new("RGB", (32, 32), "white").save(probe_path)
        for _ in pipeline.predict(probe_path):
            pass  # Drain the generator; content is irrelevant, execution is.
    sys.stderr.write("[paddle_vl] GPU smoke test passed\n")


def main():
    parser = argparse.ArgumentParser(
        description="PaddleOCR-VL layout-aware OCR subprocess for EntropIA"
    )
    parser.add_argument("image_path", help="Path to the image file")
    parser.add_argument(
        "--device",
        default="cpu",
        choices=["cpu", "gpu"],
        help="Compute device: cpu or gpu (default: cpu). GPU falls back to CPU if unavailable.",
    )
    args = parser.parse_args()

    image_path = args.image_path
    requested_device = args.device

    try:
        t_start = time.time()
        sys.stderr.write(f"[paddle_vl] Importing PaddleOCRVL... (t+0.0s)\n")
        from paddleocr import PaddleOCRVL
        t_import = time.time()
        sys.stderr.write(f"[paddle_vl] Import done (took {t_import - t_start:.1f}s)\n")

        ok, msg = _check_paddle_version()
        if not ok:
            raise RuntimeError(msg)

        sys.stderr.write(
            f"[paddle_vl] Initializing PaddleOCRVL pipeline (requested_device={requested_device})... "
            f"(t+{t_import - t_start:.1f}s)\n"
        )
        pipeline, actual_device = _init_pipeline(requested_device)
        t_pipeline = time.time()
        sys.stderr.write(
            f"[paddle_vl] Pipeline ready (took {t_pipeline - t_import:.1f}s, total {t_pipeline - t_start:.1f}s)\n"
        )

        sys.stderr.write(f"[paddle_vl] Processing: {image_path}\n")
        import inspect
        if "use_queues" in inspect.signature(pipeline.predict).parameters:
            output = pipeline.predict(image_path, use_queues=False)
        else:
            output = pipeline.predict(image_path)
        t_predict = time.time()
        sys.stderr.write(
            f"[paddle_vl] Predict done (took {t_predict - t_pipeline:.1f}s, total {t_predict - t_start:.1f}s)\n"
        )

        # Use save_to_json to get a clean dict serialization,
        # then parse it back. This avoids dealing with custom objects
        # that may have different attribute access patterns.
        # save_to_json writes to a directory; we read the file back.
        with tempfile.TemporaryDirectory() as tmpdir:
            for res in output:
                res.save_to_json(save_path=tmpdir)
                # Find the JSON file in tmpdir
                json_files = [f for f in os.listdir(tmpdir) if f.endswith("_res.json")]
                if not json_files:
                    raise RuntimeError("PaddleOCR-VL did not produce a JSON result file")
                json_path = os.path.join(tmpdir, json_files[0])
                with open(json_path, "r", encoding="utf-8") as f:
                    data = json.load(f)

        # Extract image dimensions
        image_width = int(data.get("width", 0))
        image_height = int(data.get("height", 0))

        # Process parsing_res_list — blocks with OCR text
        blocks = []
        text_parts = []
        parsing_res_list = data.get("parsing_res_list", [])

        for block in parsing_res_list:
            block_label = str(block.get("block_label", ""))
            block_content = str(block.get("block_content", ""))
            block_bbox = block.get("block_bbox", [0, 0, 0, 0])
            block_order = int(block.get("block_order", 0))
            group_id = int(block.get("group_id", 0))

            x1, y1, x2, y2 = [float(v) for v in block_bbox]

            blocks.append({
                "label": block_label,
                "content": block_content,
                "bbox": {
                    "x": int(x1),
                    "y": int(y1),
                    "width": int(x2 - x1),
                    "height": int(y2 - y1),
                },
                "order": block_order,
                "group_id": group_id,
            })

            formatted = _format_block_text(block_label, block_content)
            if formatted is not None:
                text_parts.append((block_order, formatted))

        # Sort by reading order and join with double newlines
        text_parts.sort(key=lambda t: t[0])
        full_text = "\n\n".join([t[1] for t in text_parts])

        # Build regions from layout detection results
        regions = []
        layout_det_res = data.get("layout_det_res", {})
        if isinstance(layout_det_res, dict):
            layout_boxes = layout_det_res.get("boxes", [])
            for box in layout_boxes:
                label = str(box.get("label", ""))
                score = float(box.get("score", 0.0))
                coord = box.get("coordinate", [0, 0, 0, 0])

                x1, y1, x2, y2 = [float(v) for v in coord]
                regions.append({
                    "category": _map_label_to_category(label),
                    "bbox": {
                        "x": int(x1),
                        "y": int(y1),
                        "width": int(x2 - x1),
                        "height": int(y2 - y1),
                    },
                    "confidence": round(score, 4),
                })

        result = {
            "text": full_text,
            "method": "paddle_vl",
            "blocks": blocks,
            "regions": regions,
            "image_width": image_width,
            "image_height": image_height,
            "actual_device": actual_device,
        }

        json_str = json.dumps(result, ensure_ascii=False)
        sys.stdout.write(f"{SENTINEL_BEGIN}\n{json_str}\n{SENTINEL_END}\n")
        sys.stdout.flush()
        sys.stderr.write(
            f"[paddle_vl] Done: {len(blocks)} blocks, {len(regions)} regions, device={actual_device}\n"
        )

    except ImportError:
        error_msg = "paddleocr not installed. Run: pip install 'paddleocr[doc-parser]'"
        sys.stderr.write(f"Error: {error_msg}\n")
        error_json = json.dumps({"error": error_msg})
        sys.stdout.write(f"{SENTINEL_BEGIN}\n{error_json}\n{SENTINEL_END}\n")
        sys.stdout.flush()
        sys.exit(1)
    except Exception as e:
        tb = traceback.format_exc()
        error_msg = _describe_exception(e, tb)
        sys.stderr.write(f"Error: {error_msg}\nTraceback:\n{tb}\n")
        error_json = json.dumps({
            "error": error_msg,
            "exception_type": type(e).__name__,
            "traceback_tail": _tail_lines(tb, 12),
        }, ensure_ascii=False)
        sys.stdout.write(f"{SENTINEL_BEGIN}\n{error_json}\n{SENTINEL_END}\n")
        sys.stdout.flush()
        sys.exit(1)


if __name__ == "__main__":
    main()
