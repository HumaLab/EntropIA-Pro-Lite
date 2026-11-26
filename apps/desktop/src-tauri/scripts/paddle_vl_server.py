#!/usr/bin/env python3
"""
EntropIA PaddleOCR-VL on llama-server.

Runs `paddle_vl.py` unchanged, with PaddleOCR-VL's recognition sent to a
llama-server that serves the PaddleOCR-VL GGUF model (PaddleOCR's
`llama-cpp-server` backend). Layout detection stays in Paddle on --device.
Same arguments and output contract as `paddle_vl.py`, plus:

    python paddle_vl_server.py <image_path> --vl-server-url http://127.0.0.1:PORT
        [--vl-max-concurrency N] [--device cpu|gpu]

This wrapper ships with the installer only: `paddle_vl.py` also lives in the
runtime pack, whose manifest pins its hash, so it is left as it is.
"""

from __future__ import annotations

import argparse
import functools
import importlib.util
import inspect
import sys
from pathlib import Path
from types import SimpleNamespace

PADDLE_VL_SCRIPT = Path(__file__).resolve().parent / "paddle_vl.py"


def _streamed_create(original):
    """Wrap an async `chat.completions.create` so every request streams.

    llama-server's chat parser can fail the final parse of a finished answer
    with HTTP 500 "Failed to parse input at pos 0" (llama.cpp #20814). A
    streamed request returns the same tokens as deltas; they are joined back
    into the single message PaddleX reads (`choices[0].message.content`).
    """

    async def create(self, *args, **kwargs):
        if kwargs.get("stream"):
            return await original(self, *args, **kwargs)
        kwargs["stream"] = True
        stream = await original(self, *args, **kwargs)
        parts = []
        finish_reason = None
        async for chunk in stream:
            if not chunk.choices:
                continue
            choice = chunk.choices[0]
            if choice.delta is not None and choice.delta.content:
                parts.append(choice.delta.content)
            if choice.finish_reason:
                finish_reason = choice.finish_reason
        message = SimpleNamespace(role="assistant", content="".join(parts))
        return SimpleNamespace(
            choices=[SimpleNamespace(index=0, message=message, finish_reason=finish_reason)]
        )

    return create


def _install_streaming_requests():
    """Make every OpenAI-client chat request in this process stream."""
    from openai.resources.chat.completions import AsyncCompletions

    AsyncCompletions.create = _streamed_create(AsyncCompletions.create)


def _llama_backend_kwargs(server_url: str, max_concurrency: int) -> dict:
    """PaddleOCRVL kwargs that send recognition to a llama-server."""
    return {
        "vl_rec_backend": "llama-cpp-server",
        "vl_rec_server_url": server_url.rstrip("/") + "/v1",
        "vl_rec_max_concurrency": max_concurrency,
    }


def _with_backend(pipeline_class, backend_kwargs: dict):
    """Return a stand-in for `PaddleOCRVL` that always adds `backend_kwargs`.

    `paddle_vl.py` filters its options by `inspect.signature(PaddleOCRVL)`;
    `functools.wraps` keeps that signature. A PaddleOCR without the
    `vl_rec_backend` option cannot use a server, so it fails loudly here
    instead of silently running recognition in Paddle.
    """
    if "vl_rec_backend" not in inspect.signature(pipeline_class).parameters:
        raise RuntimeError(
            "The installed PaddleOCR has no vl_rec_backend option; "
            "PaddleOCR-VL cannot run on llama-server."
        )

    @functools.wraps(pipeline_class)
    def create(*args, **kwargs):
        kwargs.update(backend_kwargs)
        return pipeline_class(*args, **kwargs)

    return create


def _split_args(argv: list[str]) -> tuple[argparse.Namespace, list[str]]:
    """Take this wrapper's options out of `argv`; the rest go to paddle_vl.py."""
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--vl-server-url", required=True)
    parser.add_argument("--vl-max-concurrency", type=int, default=1)
    return parser.parse_known_args(argv)


def _load_paddle_vl():
    # Importing it applies its environment setup (threads, oneDNN, caches),
    # which must happen before paddleocr is imported.
    spec = importlib.util.spec_from_file_location("paddle_vl", PADDLE_VL_SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> None:
    options, rest = _split_args(sys.argv[1:])
    sys.argv = [str(PADDLE_VL_SCRIPT), *rest]
    paddle_vl = _load_paddle_vl()

    import paddleocr

    paddleocr.PaddleOCRVL = _with_backend(
        paddleocr.PaddleOCRVL,
        _llama_backend_kwargs(options.vl_server_url, options.vl_max_concurrency),
    )
    _install_streaming_requests()
    sys.stderr.write(f"[paddle_vl] Recognition on llama-server {options.vl_server_url}\n")
    paddle_vl.main()


if __name__ == "__main__":
    main()
