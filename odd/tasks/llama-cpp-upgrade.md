# llama.cpp upgrade: b8783 to b11429

Objective: move the in-process `llama-cpp-2` crate and the managed `llama-server` to one newer llama.cpp revision that tolerates malformed UTF-8 in model output. Branch: `feat/llama-cpp-b11429`. Delivery: ask-on-risk; every task touches dependencies or delivery (high-risk item 5). Test runner: `CARGO_TARGET_DIR=target-test cargo test --features local-ml --lib` (Rust), plus hardware checks.

## Specs

- S1. Root cause to remove: OCRH on llama-server fails on DSC04629 with `APIError: Failed to parse input at pos 0: El delito fue cometido a las 3 del 31 de diciembre en Jacinto Pera …` and the page falls back to OCRL. In b8783 every chat completion goes through the PEG parser (`common/chat.cpp` `common_chat_peg_parse`), and its until-parser (`common/peg-parser.cpp`) fails at the start position on invalid or incomplete UTF-8, so a reply ending in a lone UTF-8 lead byte throws at pos 0. Upstream fix: ggml-org/llama.cpp 3d82ef62 (2026-09-20), "common/peg : handle invalid utf-8 sequences in the AST" (#29161).
- S2. Target revision: llama.cpp `b11429` = `d81235049384534c167caea52b85a694f6103d14` (also tagged `v0.6.0`), published 2026-10-05. It is exactly the revision `llama-cpp-sys-2` 0.1.160 and 0.1.161 vendor, and it contains 3d82ef62. The app keeps its rule that the managed `llama-server` and the linked crate are the same revision.
- S3. Crate: `llama-cpp-2` 0.1.145 to 0.1.161. All use is in `src/llm/engine.rs`. API changes found:
  - tokenization and end-of-generation moved to `LlamaVocab` (`vocab.tokenize(bytes, add_special, parse_special)`, `vocab.is_eog(token)`);
  - `token_to_piece(token, special, lstrip)` no longer takes an `encoding_rs::Decoder`;
  - `LlamaSampler::accept` uses `try_accept` under the `common` feature, which is now in the crate's default features.
  Unchanged: `LlamaBackend::init`, `LlamaModel::load_from_file`, `new_context`, `apply_chat_template`, `chat_template`, `LlamaBatch`, the `with_n_*` setters, `chain_simple`/`dist`/`temp`/`greedy`, `void_logs`.
- S4. Managed `llama-server` assets at b11429 (size, sha256 from the release digests):
  - `llama-b11429-bin-win-cuda-12.4-x64.zip` 264478423 `dd6df685c1024e6aa55ca55ad35038692d5e284d4dc895e7a646c3245d3d14ff`
  - `cudart-llama-bin-win-cuda-12.4-x64.zip` 391443627 `8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6` (identical to the b8783 pin)
  - `llama-b11429-bin-win-vulkan-x64.zip` 33337769 `1bfe78ad9168b79fa02bf67f6af9f5e17a966d824d77238517f7bef12ac73b36`
  - `llama-b11429-bin-win-rocm-10.0-x64.zip` 256958369 `289a8453bb222f0648f91eff3d55133c934119c841eb369b704aafae1d4719d2` (replaces `win-hip-radeon`; the zip now bundles `amdhip64_7.dll`, `rocm_kpack.dll`, `amd_comgr.dll`)
  - `llama-b11429-bin-ubuntu-rocm-10.0-x64.tar.gz` 243892368 `bd73e52146551f07a4c539bbe9da42f3b680c44ddc252fce61ab86d567a5932a` (was rocm-7.2)
  - `llama-b11429-bin-ubuntu-vulkan-x64.tar.gz` 31636673 `632c4e98feba2b94407a2130e3133e0c3aefb0ea1ab41337e926d8bfafdd0b74`
  - `llama-b11429-bin-macos-arm64.tar.gz` 11971406 `740288ec6887be94280a5dfa25b5e23a78285cab104519e6c7e218904ee82459`
  `scripts/fetch-llama-server.sh` pins the same revision for the Linux `.deb` CUDA build.
- S5. Existing installs must move to the new revision. Today `backend.txt` records only the build kind (`vulkan`, `cuda`, ...), not the tag, so a b8783 install would never be replaced. The marker must also record the revision, and a mismatch must count as "not installed" for the existing download flow.
- S6. HIP target table (`ocr/llama_gpu.rs` `hip_target`): b11429's release.yml builds a superset of b8783's targets (Windows ROCm 10.0 adds gfx1010-1012, gfx1033-1036, gfx1103, gfx1152, gfx1153; Linux now includes gfx1031/1032). No GPU loses HIP. Widening the table is optional and stays out of scope unless the user asks.
- S8. Windows AMD runs on Vulkan. b11429's `win-hip-radeon` zip ships only `amdhip64_7.dll`, `rocm_kpack.dll`, `amd_comgr.dll` and `ggml-hip.dll` (no rocBLAS/hipBLASLt; b8783 shipped 1976 such files), so `hip_target` answers only on Linux and the Windows HIP asset is dropped. Linux HIP needs a system ROCm 10.0; without it the device list is empty and the existing fallback picks Vulkan. No GPU loses GPU OCR: the VRAM thresholds do not depend on the backend. Pending, needs a price OK first: an AWS g4ad (Radeon Pro V520, gfx1011, Windows) run comparing Vulkan and HIP speed on the same page.
- S7. Hardware acceptance: on the RX 560 (Vulkan0), OCRH on DSC04629 completes on llama-server without falling back to OCRL; GLM-OCR and the Qwen corrector still start. NVIDIA, HIP and macOS are checked by CI and the download contract tests only; no such hardware is available here.

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S2, S3 | inline | done | feat/llama-cpp-b11429 |
| T2 | S2, S4, S8 | inline | done | feat/llama-cpp-b11429 |
| T3 | S5 | inline | done | feat/llama-cpp-b11429 |
| T4 | S7 | inline (user runs the app) | done | main 30c39805 |

- T1: bump `llama-cpp-2` to 0.1.161, migrate `src/llm/engine.rs`, keep the `cuda` feature mapping; Pro clippy and tests.
- T2: `RELEASE_TAG`, the asset table, `fetch-llama-server.sh`, the HIP comment in `llama_gpu.rs`, `THIRD_PARTY_NOTICES*`.
- T3: revision in the install marker; a b8783 install reads as outdated and is replaced through the existing download.
- T4: user check in the dev app (profile `pro-audit`), then CI.

## Log

- L1 (2026-11-26, user): "que es lo mejor en terminos de buenas practicas y deuda técnica??" Answer given: upgrade llama.cpp as its own feature instead of a client-side patch that parses the error string.
- L2 (2026-11-26, user): "si" (to: start phase 0 of the upgrade).
- L3 (2026-11-26): phase 0 done. Findings are S1-S6. Not yet measured: the Pro build time of the new vendored llama.cpp. Next step: T1 on a new branch from main.
- L4 (2026-11-27): T1 done. `llama-cpp-2` and `llama-cpp-sys-2` 0.1.145 to 0.1.161; `cargo update` also moved `cc` 1.2.60 to 1.11.3, `find-msvc-tools` 0.1.9 to 0.1.15, and added `shlex` 2.0.1 (build dependencies of the new sys crate; `cc` is shared with other build scripts). `engine.rs`: own `AddBos` (the crate dropped it), tokenization and end-of-generation through `model.vocab()` with `parse_special = true` as before, and `token_to_piece` now returns bytes that `push_piece` feeds to the same `encoding_rs` decoder. Checks: Pro clippy clean (28 min cold, llama.cpp rebuilt from source), Pro `llm::` tests 213 passed including 3 new (split character, lone lead byte, `AddBos`), Lite clippy clean, fmt clean. The `pro-audit` dev app crashed while these builds ran (its watcher sees `target-test`, and the concurrent `ocr-rs` MNN rebuild in `target/` exited 101 with no message); the same `ocr-rs` built fine in `target-test` with the new `cc`, so the crash points at the concurrent build, not at the change. To confirm when the dev app is relaunched.
- L5 (2026-11-27, user): "si, hacelo, y deaja anotado una prueba via AWS con una placa AMD en windows via Vulkan y via HIP, te parece?" (to: drop Windows HIP, AMD on Windows runs on Vulkan). Recorded as S8.
- L6 (2026-11-27): T2 and T3 done. `RELEASE_TAG` follows `SERVER_REVISION`; asset sizes and SHA-256 updated to b11429; the marker now reads `<backend> b11429`, and `install_serves` treats an older or missing revision as not installed, so a b8783 install is downloaded again. Checks: fmt clean, Lite clippy clean, Pro `ocr::`/`llm::` tests 592 passed, Pro clippy clean. The first Pro run failed only because drive G: was full (os error 112); rerun with `CARGO_INCREMENTAL=0`.
- L7 (2026-11-27): PR #458 merged as 30c39805. CI found one more break on the Windows debug link: `_CrtSetReportMode` exists only in the debug CRT, and no dependency pulls it in any more (llama-cpp-sys-2 0.1.161 builds llama.cpp as RelWithDebInfo). `lib.rs` now looks it up in `ucrtbased.dll` at run time (commit 76c96125). The `pro-audit` dev app failure was a 0-byte MNN object (`StaticModule.cpp.obj`) left when drive G: filled; deleting it fixed the build.
- L8 (2026-11-27): T4 done on the RX 560. The app downloaded llama-server b11429 (vulkan) over the b8783 install. OCRH on DSC04629 (collection SOIP 1961) with GLM-OCR local: llama-server ready on Vulkan0 in 7.5 s (b8783: 10.5 s), PaddleOCR-VL layout 44 blocks in 181 s on CPU, finished with `método=glm_ocr_local`, 11070 characters, 283 s, no `Failed to parse input` and no OCRL fallback. Still pending, needs a price OK: the AWS g4ad Vulkan vs HIP run (S8).
