#!/usr/bin/env python3
"""Tests for the llama-server pieces of paddle_vl.py (no Paddle needed)."""

from __future__ import annotations

import asyncio
import importlib.util
import unittest
from pathlib import Path
from types import SimpleNamespace

SCRIPT = Path(__file__).resolve().parent / "paddle_vl.py"
_spec = importlib.util.spec_from_file_location("paddle_vl", SCRIPT)
assert _spec is not None and _spec.loader is not None
paddle_vl = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(paddle_vl)


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

        create = paddle_vl._streamed_create(original)
        result = asyncio.run(create(None, model="m", messages=[]))

        self.assertEqual(calls, [{"model": "m", "messages": [], "stream": True}])
        self.assertEqual(result.choices[0].message.content, "Hola mundo")
        self.assertEqual(result.choices[0].finish_reason, "stop")

    def test_a_request_that_already_streams_is_left_alone(self) -> None:
        sentinel = object()

        async def original(_self, *args, **kwargs):
            return sentinel

        create = paddle_vl._streamed_create(original)
        result = asyncio.run(create(None, stream=True))

        self.assertIs(result, sentinel)


class LlamaPipelineKwargsTests(unittest.TestCase):
    def test_the_server_backend_points_at_the_v1_api(self) -> None:
        kwargs = paddle_vl._llama_backend_kwargs("http://127.0.0.1:5000/", 4)

        self.assertEqual(kwargs["vl_rec_backend"], "llama-cpp-server")
        self.assertEqual(kwargs["vl_rec_server_url"], "http://127.0.0.1:5000/v1")
        self.assertEqual(kwargs["vl_rec_max_concurrency"], 4)


if __name__ == "__main__":
    unittest.main()
