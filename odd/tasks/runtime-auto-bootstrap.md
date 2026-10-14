# Runtime auto-bootstrap for the lean Pro installer

The lean Pro installer ships a fixture runtime-pack and never starts the signed ~2.44 GB runtime download: the fixture status hardcodes an unavailable source and nothing triggers the bootstrap at startup. This feature makes the download automatic and visible.
Branch `feat/runtime-auto-bootstrap` (from `feat/internet-archive` @ `6b18d9f3`). Diagnosis: Engram 4928; design map: explore task mv8b1beu.

## Specs
- S1: Automatic download at first launch. User: "yo creo que automatico es mejor". Approved design (user answered "si"): "Al primer inicio arranca sola, después de verificar que hay espacio en disco (2,4 GB más margen para descomprimir). Si no hay espacio, no arranca y lo explica."
- S2: "Barra de progreso visible en el banner, con tamaño total, avance y botón Pausar/Cancelar."
- S3: "Reanuda donde quedó si se corta o se cierra la app. El código ya tiene staging en `.downloads`; hay que confirmar que soporta reanudar."
- S4: "Si falla o no hay conexión, aparece "Reintentar descarga". Esto también cubre a los usuarios que hoy ven el banner sin botón."
- S5: "Mientras descarga, la app sigue usable para todo lo que no sea OCR, NLP o transcripción."
- S6: "Texto corregido: nada de "modo desarrollo activo" en un build de release."
- S7 (derived prerequisite, found during exploration): the download must not fail because of a total request timeout; `download.rs:127-129` sets a 120 s total timeout that a 2.44 GB transfer cannot meet. Replace it with connect/read-stall timeouts.
- S8 (safety constraint): never auto-download in debug builds, dev profiles (`ENTROPIA_DEV_PROFILE`) or tests; tests must never reach the network.

## Tasks
- T1 (S3, S7) Resumable downloader: DONE. Per-part `Range: bytes=N-` resume with durable atomic state under `.downloads`; longer staged file truncates to recorded offset, shorter/oversized discards; guard keeps partial on network/stall, deletes on verification failure; connect 30 s + per-read stall 60 s timeouts replace the 120 s total. Independent re-verification PASS S3/S7/S8: `runtime::` 74/74 (baseline 64), zero new clippy lints, rustfmt clean. First verification had caught a hard-kill resume defect, fixed with a behavioral RED test.
- T2 (S2, S5) Cancel/pause token and non-blocking entry: cancellation checked in the read loop, lock released; async/background start + cancel commands. Files: `runtime/download.rs`, `ops_lock.rs`, `runtime/mod.rs`, `lib.rs` command registration, capabilities if needed. Route: worker. Commit: pending.
- T3 (S1, S4, S6) Truthful fixture status + disk pre-flight + copy: status consults the configured source so a fixture pack with a source is bootstrap-eligible and repairable/retryable; free-space check before the first byte with an explanatory status; release copy without "modo desarrollo activo". Files: `runtime/manager.rs`, `runtime/status.rs`, related tests. Route: worker. Commit: pending.
- T4 (S1, S8) Startup auto-bootstrap: after `validate_startup`, start the bootstrap once in the background when the pack is a fixture and a source is configured; skip in debug/dev-profile/tests. Files: `lib.rs`, `runtime/manager.rs`. Route: worker. Commit: pending.
- T5 (S2, S4, S5) Banner UI: progress bar with total size and percent, Pause/Cancel, "Reintentar descarga" on failure/offline. Files: `layout/AppShell.svelte`, `lib/runtime.ts`, tests. Route: worker. Commit: pending.
- T6 Verification: focused Rust + frontend suites (Pro and Lite), independent verifier, native review per work unit; Pro CI build and VM validation need a later decision (the artifact workflow only triggers on `feat/internet-archive`). Commit: pending.

## Log
- L1 (user, verbatim): "que te parece mejor, yo creo que automatico es mejor"
- L2 (user, verbatim, approving the six-point design): "si"
- L4 Correction: `runtime/download.rs` and most of `runtime/*` are `#[cfg(feature = "local-ml")]`; the lean runner compiles 0 of these tests (worker task mv8b6jj8, no files changed). Project CI runs `cargo test --features local-ml` only on Windows (`ci.yml:93-101`). Decision: build the local-ml test binary on the Linux host (32 cores, 123 GB RAM, cmake, g++; cargo at `~/.cargo/bin`, not on the background shell PATH) and run all runtime tests with `--features local-ml`. Warm build started in the background.
- L3 Exploration evidence: no Range/resume (`download.rs:236` truncates; `resume.json` written only after full download, never read); no cancel token in `runtime/*`; blocking reqwest in non-async commands; only `DependenciasTab` listens to `runtime://progress`; disk check exists (`download.rs:21-56`) but runs after signature verify; dev builds are safe only because `option_env!` is empty, not by design; tests asserting the hardcoded fixture plan listed in the explore report.
