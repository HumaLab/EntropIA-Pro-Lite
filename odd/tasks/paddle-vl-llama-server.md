# PaddleOCR-VL on llama-server

Objective: run PaddleOCR-VL's recognition on the app's llama-server (any GPU llama.cpp supports) instead of Paddle's native engine, when its GGUF model is installed.
Branch: `feat/paddle-vl-llama-server` (from main after #443). Delivery: two PRs to main, backend first (T1-T3), then Settings UI (T4). Test runner: `CARGO_TARGET_DIR=target-test cargo test --features local-ml --lib` (Rust), desktop Vitest for the UI.

## Specs

- S1. When the OCRH local engine resolves to PaddleOCR-VL (setting `paddle_vl`, or `auto` without usable GLM-OCR), the PaddleOCR-VL GGUF model is installed, llama-server is available and the GPU passes the llama gate, the page runs `PaddleOCRVL(vl_rec_backend="llama-cpp-server")` against the app's shared llama-server. Otherwise the existing native path runs unchanged.
- S2. The model is the official `PaddlePaddle/PaddleOCR-VL-1.6-GGUF` (Apache-2.0), pinned to revision `511b09642bb324401f15f97cc23bc67e8f0a291d`: `PaddleOCR-VL-1.6-GGUF.gguf` (935 769 056 bytes, sha256 `f3ae46ec885050acf4b3d31944431e1fd90d50664fb09126af4a3c050ba14ee8`) and `PaddleOCR-VL-1.6-GGUF-mmproj.gguf` (881 770 560 bytes, sha256 `204d757d7610d9b3faab10d506d69e5b244e32bf765e2bab2d0167e65e0a058a`). A file whose size or hash differs is refused and removed. Download, delete and status commands follow the GLM-OCR local ones, including the free-space check.
- S3. Recognition requests are streamed and reassembled, so llama-server's final chat parse (llama.cpp #20814, HTTP 500 "Failed to parse input at pos 0") cannot fail a page. Streamed and non-streamed answers must be identical.
- S4. The server reuses `local_server` (one model in VRAM; OCRH, correction and chat share it). Context per slot covers PaddleX's 8192 max new tokens plus the image tokens.
- S5. Settings shows the PaddleOCR-VL GGUF model next to GLM-OCR in "Modelos locales" and "Dependencias de IA", with download, progress, delete and size. (Assumption: same card shape as GLM-OCR.)

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S1, S3 | inline | done (RED then GREEN: test_paddle_vl.py, 3 tests) | backend PR |
| T2 | S2, S4 | inline | done (7 unit tests) | backend PR |
| T3 | S1 | inline | done (4 selection tests) | backend PR |
| T4 | S5 | inline | pending | |

- T1: `scripts/paddle_vl.py` (installer copy only): `--vl-server-url`, `--vl-max-concurrency`; streaming shim; build `PaddleOCRVL` with the llama-cpp-server backend.
- T2: `ocr/paddle_vl_local.rs`: constants, assets, info, server config; download with size + sha256 check; delete; Tauri commands.
- T3: `paddle_vl.rs` engine selection and the run path that starts the server and passes the URL.
- T4: Settings cards, `lib` wrapper, i18n, tests.

## Log

- L1 (2026-11-26, user): "arrancá con PaddleOCR-VL sobre llama-server o con lo que consideres mas oportuno". Earlier: "anótalo como tarea prioritaria en pendientes y arranquemos con esta".
- L2 (2026-11-26): prior measurement (Engram #7310, RTX 4060): native 256 s/page, llama-server 18.6 s/page, 3.3 GB VRAM; 500 parse errors from llama-server b8783 worked around with streaming.
- L3 (2026-11-26, this machine, RX 560 4 GB Vulkan, b8783, PaddleOCR 3.7.0 / PaddleX 3.7.2): two scanned book pages ran end to end; no 500 reproduced (non-streamed 156 s and 144 s; streamed 80 s and 77 s, warm). Streamed and non-streamed text identical on one page and 0.9997 similar on the other (one block ending differed between runs). A direct request at temperature 0 gave byte-identical streamed and non-streamed answers (1374 chars, finish `stop`).
- L4 (2026-11-26): `image_max_pixels` is not the quality cause suggested in #7310: PaddleX's native preprocessor also uses `max_pixels` 1003520 for OCR blocks (`preprocessor_config.json`, `pipeline.py:666`); only `spotting` uses 1605632. No `--image-max-tokens` change.
- L5 (2026-11-26): KV cache measured at 144 MiB per 8192 tokens (18 layers, 2 KV heads); 4 slots x 10240 ctx is about 720 MiB.
- L6 (2026-11-26): delivery decision: backend and UI as two PRs to main, to stay near 400 lines each.
- L7 (2026-11-26): T3 design: `DesiredBackend` {PaddleVlNative, PaddleVlServer, GlmOcr}; GLM-OCR keeps priority under `auto`; PaddleOCR-VL on llama-server runs under `auto`/`paddle_vl` when its GGUF and server are installed and `engine_runs(Engine::PaddleVl)`. The llama path always uses the bundled `paddle_vl.py` (managed root skipped), layout on CPU. The cached engine is rebuilt on any backend change and a server-backed engine stops its llama-server when switched away. `test_paddle_vl.py` added to the CI `runtime-pack-smoke` step.
- L8 (2026-11-26): the runtime-pack copies of `paddle_vl.py` stay unchanged: their manifest pins the script hash, and changing it would make every installed runtime non-reusable and require republishing the pack (4 `runtime::manager` tests failed with the copies changed). The llama path already runs the installer's script. The download now refuses up front when the disk cannot hold both missing files (`ensure_room_in`, from #444). Checks: clippy Pro and Lite clean, `ocr::` + `runtime::manager` 433 passed, full Pro suite otherwise green (4101 passed before the revert), Python 19 tests OK.
