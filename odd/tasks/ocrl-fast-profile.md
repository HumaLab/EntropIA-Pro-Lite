# OCRL fast profile (PP-OCRv6 Tiny)

Objective: offer PP-OCRv6 Tiny as an optional, downloadable OCRL recognition profile next to the bundled PP-OCRv5 Latin, which stays the default.
Branch: `feat/ocrl-fast-profile` (from main after #453). Delivery: backend PR (T1-T3), then Settings UI PR (T4). Test runner: `CARGO_TARGET_DIR=target-test cargo test --features local-ml --lib` (Rust), desktop Vitest for the UI.

## Specs

- S1. OCRL has two recognition profiles: `v5_latin` (bundled PP-OCRv5 mobile det + Latin rec, the default) and `v6_tiny` (PP-OCRv6 Tiny det + rec + its own keys file). Both run through the same `PaddleOcrProvider` contract and output; the orientation model keeps coming from the bundled OCR model directory.
- S2. The v6 Tiny files are pinned to `zibo-chen/rust-paddle-ocr` commit `664a945953e46186b37203fa4902a20d60a32809` (Apache-2.0), path `models/`: `PP-OCRv6_tiny_det.mnn` (901 896 bytes, sha256 `7fab7b858f136bc93a760bdca66aaf25f0ff10accabb31e6ef853a897fb9cfec`), `PP-OCRv6_tiny_rec.mnn` (2 251 616 bytes, sha256 `0a43c3c979a98b905f5e84913209998f510189419b5a5d4152bbb01ce8d17a93`), `ppocr_keys_v6_tiny.txt` (27 156 bytes, sha256 `c5cbe34ef40c29c4df07ed012bf96569cb69a2d2a01a07027e9f13cb832bd9cd`). A file whose size or hash differs is refused and removed. Download, delete and status follow the PaddleOCR-VL local ones, including the free-space check.
- S3. A setting `ocrl_profile` (`v5_latin` | `v6_tiny`, default `v5_latin`) chooses the profile for batch OCR and selective OCR. A cached provider is rebuilt when the setting changes. When `v6_tiny` is chosen but its files are missing, OCRL runs `v5_latin` and logs why (never fails the page).
- S4. OCR history records the recognition profile of a light OCR run.
- S5. Settings shows the OCRL profile choice with the v6 Tiny download, progress, delete and size; es/en strings. (Assumption: same card shape as the PaddleOCR-VL one.)
- S6. User decision (2026-11-26): "Optional fast profile" — v5 Latin stays the default; v6 Tiny is opt-in.

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S1 | inline | done (3 unit tests + ignored v6 integration, passed locally) | see git log |
| T2 | S2 | inline | pending | |
| T3 | S3, S4 | inline | pending | |
| T4 | S5 | inline | pending | |

- T1: `ocr/paddle.rs` profile type with per-profile filenames; constructor takes the recognition dir and the orientation dir; `Cargo.toml` `ocr-rs` spec `"2.4"` (the lock already resolves 2.4.1).
- T2: `ocr/ppocr_v6_local.rs` (pinned files, missing/verify/info), Tauri commands in `ocr/commands.rs` with non-`local-ml` stubs, registration in `lib.rs`.
- T3: setting constant + getter; `processing/ocr.rs` `ensure_local` and `bibliography/processing.rs` selective OCR pick the profile and rebuild on change; history params record the profile.
- T4: `lib/ppocr-v6-local.ts`, Settings card and radio, `settings.ts` key, i18n, tests.

## Log

- L1 (2026-11-26, user): "Los dos planes grandes (preprocesamiento de imágenes y OCRL con PP-OCRv6 Tiny): necesitan bajar modelos nuevos, Te doy mi OK". Plan: `S:\Descargas\plan_OCRL_PaddleOCR_Tiny_EntropIA.md`.
- L2 (2026-11-26): F0 measured (Engram `ocr/ppocrv6-tiny-f0-results`): v6 Tiny ~2.8x faster than v5 Latin on two Kerbo book pages (1.1 vs 3.0-3.5 s, 4 threads), word F1 about 1 point lower, accented-word recall about 3 points lower.
- L3 (2026-11-26, user): "Optional fast profile" (answer to: adopt v6 Tiny as an optional fast profile or as the default).
- L4 (2026-11-26): mapping: model filenames hardcoded in `PaddleOcrProvider::new` (`paddle.rs:92-94`); providers cached in `OcrExecutor.local` (`processing/ocr.rs:238`) and `ProductionSelectiveOcr.paddle` (`bibliography/processing.rs:4180`), neither rebuilt on settings change; `Cargo.lock` already resolves `ocr-rs` 2.4.1. Lite enablement (plan phase 2) is out of scope here.
