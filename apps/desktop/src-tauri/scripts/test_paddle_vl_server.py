#!/usr/bin/env python3
"""Tests for paddle_vl_server.py (no Paddle or OpenAI client needed)."""

from __future__ import annotations

import asyncio
import importlib.util
import inspect
import unittest
from pathlib import Path
from types import SimpleNamespace

SCRIPT = Path(__file__).resolve().parent / "paddle_vl_server.py"
_spec = importlib.util.spec_from_file_location("paddle_vl_server", SCRIPT)
assert _spec is not None and _spec.loader is not None
server = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(server)


def chunk(content=None, finish=None):
    delta = SimpleNamespace(content=content)
    return SimpleNamespace(choices=[SimpleNamespace(delta=delta, finish_reason=finish)])


class AsyncChunks:
    def __init__(self, chunks):
        self._chunks = list(chunks)

    def __aiter__(self):
        return self

    async def __anext__(self):
        if not self._chunks:
            raise StopAsyncIteration
        return self._chunks.pop(0)


class StreamedCreateTests(unittest.TestCase):
    def test_a_plain_request_is_streamed_and_returned_whole(self) -> None:
        calls = []

        async def original(_self, *args, **kwargs):
            calls.append(kwargs)
            return AsyncChunks([
                chunk("Hola "),
                SimpleNamespace(choices=[]),
                chunk("mundo"),
                chunk(None, "stop"),
            ])

        create = server._streamed_create(original)
        result = asyncio.run(create(None, model="m", messages=[]))

        self.assertEqual(calls, [{"model": "m", "messages": [], "stream": True}])
        self.assertEqual(result.choices[0].message.content, "Hola mundo")
        self.assertEqual(result.choices[0].finish_reason, "stop")

    def test_a_request_that_already_streams_is_left_alone(self) -> None:
        sentinel = object()

        async def original(_self, *args, **kwargs):
            return sentinel

        create = server._streamed_create(original)
        result = asyncio.run(create(None, stream=True))

        self.assertIs(result, sentinel)


class FakePipeline:
    def __init__(self, device="cpu", vl_rec_backend=None, vl_rec_server_url=None,
                 vl_rec_max_concurrency=1, use_queues=True):
        self.kwargs = {
            "device": device,
            "vl_rec_backend": vl_rec_backend,
            "vl_rec_server_url": vl_rec_server_url,
            "vl_rec_max_concurrency": vl_rec_max_concurrency,
            "use_queues": use_queues,
        }


class OldPipeline:
    def __init__(self, device="cpu"):
        self.device = device


class BackendTests(unittest.TestCase):
    def test_the_server_backend_points_at_the_v1_api(self) -> None:
        kwargs = server._llama_backend_kwargs("http://127.0.0.1:5000/", 4)

        self.assertEqual(kwargs["vl_rec_backend"], "llama-cpp-server")
        self.assertEqual(kwargs["vl_rec_server_url"], "http://127.0.0.1:5000/v1")
        self.assertEqual(kwargs["vl_rec_max_concurrency"], 4)

    def test_the_pipeline_gets_the_server_over_paddle_vl_defaults(self) -> None:
        create = server._with_backend(
            FakePipeline, server._llama_backend_kwargs("http://127.0.0.1:5000", 4)
        )
        # paddle_vl.py filters its options by this signature.
        self.assertIn("use_queues", inspect.signature(create).parameters)

        pipeline = create(device="cpu", use_queues=False, vl_rec_max_concurrency=1)

        self.assertEqual(pipeline.kwargs["vl_rec_backend"], "llama-cpp-server")
        self.assertEqual(pipeline.kwargs["vl_rec_server_url"], "http://127.0.0.1:5000/v1")
        self.assertEqual(pipeline.kwargs["vl_rec_max_concurrency"], 4)
        self.assertFalse(pipeline.kwargs["use_queues"])

    def test_a_paddleocr_without_server_support_is_refused(self) -> None:
        with self.assertRaises(RuntimeError):
            server._with_backend(OldPipeline, server._llama_backend_kwargs("http://x", 1))


class ArgumentTests(unittest.TestCase):
    def test_the_wrapper_options_are_taken_out_of_paddle_vl_arguments(self) -> None:
        options, rest = server._split_args([
            "page.png", "--device", "cpu",
            "--vl-server-url", "http://127.0.0.1:5000", "--vl-max-concurrency", "4",
        ])

        self.assertEqual(options.vl_server_url, "http://127.0.0.1:5000")
        self.assertEqual(options.vl_max_concurrency, 4)
        self.assertEqual(rest, ["page.png", "--device", "cpu"])


if __name__ == "__main__":
    unittest.main()
