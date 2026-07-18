"""
EntropIA Transcription Engine — faster-whisper subprocess.

Called by the Rust backend via std::process::Command.
Receives an audio file path, outputs JSON to stdout.

Usage:
    python transcribe.py <audio_path> [--model base] [--language es] [--compute-type int8]

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

# Suppress warnings that could corrupt JSON output on stdout
warnings.filterwarnings("ignore")

# Disable HuggingFace Hub symlink-based download and transfer.
# On Windows, symlinks in the HuggingFace cache can't be read from
# MSI-installed app contexts (WinError 448: "untrusted mount point").
# Using local_dir=some_path in snapshot_download() downloads regular files
# instead, which avoids the issue entirely.
os.environ["HF_HUB_ENABLE_HF_TRANSFER"] = "0"
os.environ["HF_HUB_SYMLINK_STORAGE"] = "0"
# Fail fast on a stalled socket instead of hanging the download forever on a flaky
# network / captive portal; the retry loop in download_model() then recovers.
os.environ.setdefault("HF_HUB_DOWNLOAD_TIMEOUT", "30")

# If --model-dir is provided (from Rust), use it for HuggingFace cache and
# local model download. We set this BEFORE any HF imports so
# huggingface_hub picks it up.
_pre_model_dir = None
for _i, _arg in enumerate(sys.argv):
    if _arg == "--model-dir" and _i + 1 < len(sys.argv):
        _pre_model_dir = sys.argv[_i + 1]
        break

if _pre_model_dir:
    os.makedirs(_pre_model_dir, exist_ok=True)
    os.environ["HF_HUB_CACHE"] = _pre_model_dir
    os.environ["HF_HOME"] = _pre_model_dir

# Force stdout to be unbuffered so the Rust side sees output promptly
# and to avoid partial writes on crash.
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", line_buffering=True)

SENTINEL_BEGIN = "===TRANSCRIPTION_JSON_BEGIN==="
SENTINEL_END = "===TRANSCRIPTION_JSON_END==="

# Mapping from faster-whisper model size names to HuggingFace model IDs.
MODEL_TO_HF_ID = {
    "tiny": "Systran/faster-whisper-tiny",
    "base": "Systran/faster-whisper-base",
    "small": "Systran/faster-whisper-small",
    "medium": "Systran/faster-whisper-medium",
    "large-v1": "Systran/faster-whisper-large-v1",
    "large-v2": "Systran/faster-whisper-large-v2",
    "large-v3": "Systran/faster-whisper-large-v3",
    "large": "Systran/faster-whisper-large-v3",
}


def _download_model_as_regular_files(
    model_size: str, model_dir: str | None
) -> str | None:
    """Pre-download the model as regular files (not symlinks) using huggingface_hub.

    On Windows, symlinks in the HuggingFace cache can't be read from certain
    process contexts (WinError 448: "untrusted mount point"). Using snapshot_download()
    with local_dir forces the download to create regular file copies instead of symlinks.

    Returns the local directory path containing the model files (suitable for passing
    directly to WhisperModel as a local model path), or None on failure.
    """
    hf_model_id = MODEL_TO_HF_ID.get(model_size)
    if not hf_model_id:
        sys.stderr.write(
            f"[transcribe] Unknown model size '{model_size}' for pre-download, "
            f"letting faster_whisper handle it\n"
        )
        return None

    try:
        from huggingface_hub import snapshot_download

        # Build a local directory path for the model.
        # We use a simple flat directory (not the HF cache structure) because
        # WhisperModel can load directly from a local directory path.
        if model_dir:
            local_dir = os.path.join(model_dir, hf_model_id.replace("/", "--"))
        else:
            local_dir = os.path.join(
                os.path.expanduser("~"),
                ".cache",
                "faster-whisper",
                hf_model_id.replace("/", "--"),
            )

        # If model files already exist as regular files, skip download.
        # We check for model.bin which is the core CTranslate2 model file.
        model_bin = os.path.join(local_dir, "model.bin")
        if os.path.isfile(model_bin) and os.path.getsize(model_bin) > 0:
            if not os.path.islink(model_bin):
                sys.stderr.write(
                    f"[transcribe] Model already cached as regular files: {local_dir}\n"
                )
                return local_dir

        sys.stderr.write(
            f"[transcribe] Downloading model '{hf_model_id}' as regular files "
            f"(symlink-safe)...\n"
        )

        # local_dir downloads files directly as regular files, not symlinks.
        # This is the critical difference from using cache_dir which creates symlinks.
        os.makedirs(local_dir, exist_ok=True)

        # Retry transient network failures (dropped connection, slow/flaky Wi-Fi,
        # VPN reconnect, captive portal) with exponential backoff. Permanent errors
        # (missing/gated repo) stop immediately instead of spinning.
        import time as _time

        last_exc = None
        for _attempt in range(3):
            try:
                snapshot_download(hf_model_id, local_dir=local_dir)
                last_exc = None
                break
            except Exception as exc:  # noqa: BLE001
                if type(exc).__name__ in (
                    "RepositoryNotFoundError",
                    "GatedRepoError",
                    "EntryNotFoundError",
                ):
                    raise
                last_exc = exc
                _wait = 2**_attempt
                sys.stderr.write(
                    f"[transcribe] Download attempt {_attempt + 1}/3 failed "
                    f"({type(exc).__name__}: {exc}); retrying in {_wait}s...\n"
                )
                _time.sleep(_wait)
        if last_exc is not None:
            raise last_exc

        sys.stderr.write("[transcribe] Download complete.\n")
        return local_dir

    except ImportError:
        sys.stderr.write(
            "[transcribe] huggingface_hub not available for pre-download, "
            "letting faster_whisper handle download\n"
        )
        return None
    except Exception as e:
        sys.stderr.write(
            f"[transcribe] Pre-download failed ({e}), "
            f"letting faster_whisper handle download\n"
        )
        return None


def main():
    parser = argparse.ArgumentParser(
        description="Transcribe audio using faster-whisper"
    )
    parser.add_argument("audio_path", help="Path to the audio file")
    parser.add_argument(
        "--model",
        default="base",
        help="Whisper model size (tiny, base, small, medium, large-v3)",
    )
    parser.add_argument(
        "--language",
        default="es",
        help="Language code (es, en, etc.) or 'auto' for detection",
    )
    parser.add_argument(
        "--compute-type",
        default="int8",
        help="Compute type: int8, int8_float16, float16, float32",
    )
    parser.add_argument(
        "--model-dir",
        default=None,
        help="Directory to cache/download models (default: ~/.cache/faster-whisper)",
    )
    args = parser.parse_args()

    try:
        from faster_whisper import WhisperModel

        # Pre-download model as regular files to avoid symlink issues on Windows.
        # snapshot_download(local_dir=...) creates real file copies (not symlinks),
        # which avoids WinError 448 in MSI-installed app contexts.
        local_model_dir = _download_model_as_regular_files(args.model, args.model_dir)

        # Configure model loading.
        # If pre-download succeeded, pass the local directory path directly as the model
        # argument — WhisperModel accepts a local directory path and will load from it
        # without going through the HuggingFace cache at all.
        # If pre-download failed, fall back to normal download (which may use symlinks).
        model_kwargs = {
            "device": "cpu",
            "compute_type": args.compute_type,
        }

        if local_model_dir and os.path.isdir(local_model_dir):
            # Use the local pre-downloaded directory directly as model path.
            # WhisperModel loads from a local directory without touching HF cache.
            model_name_or_path = local_model_dir
            sys.stderr.write(
                f"[transcribe] Loading model from local path: {local_model_dir}\n"
            )
        else:
            # Fallback: let WhisperModel download normally.
            model_name_or_path = args.model
            if args.model_dir:
                model_kwargs["download_root"] = args.model_dir
            sys.stderr.write(
                f"[transcribe] Loading model '{args.model}' "
                f"(pre-download unavailable, using normal download)\n"
            )

        sys.stderr.write(f"[transcribe] compute_type={args.compute_type}\n")

        model = WhisperModel(model_name_or_path, **model_kwargs)

        # Configure transcription
        language = args.language if args.language != "auto" else None
        transcribe_kwargs = {
            "language": language,
            "condition_on_previous_text": False,  # Avoid hallucination on long audio
            "vad_filter": True,  # Voice activity detection — skip silence
        }

        sys.stderr.write(f"[transcribe] Transcribing: {args.audio_path}\n")
        segments_iter, info = model.transcribe(args.audio_path, **transcribe_kwargs)

        # Collect segments
        result = []
        for segment in segments_iter:
            result.append(
                {
                    "start": round(segment.start, 3),
                    "end": round(segment.end, 3),
                    "text": segment.text.strip(),
                }
            )

        # Output JSON to stdout wrapped in sentinels for safe parsing.
        json_str = json.dumps(result, ensure_ascii=False)
        sys.stdout.write(f"{SENTINEL_BEGIN}\n{json_str}\n{SENTINEL_END}\n")
        sys.stdout.flush()
        sys.stderr.write(
            f"[transcribe] Done: {len(result)} segments, "
            f"language={info.language}, duration={info.duration:.1f}s\n"
        )

    except ImportError:
        sys.stderr.write(
            "Error: faster-whisper not installed. Run: pip install faster-whisper\n"
        )
        sys.exit(1)
    except Exception as e:
        sys.stderr.write(f"Error: {e}\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
