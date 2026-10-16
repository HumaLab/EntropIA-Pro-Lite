# Bundled Tauri resources

**Español:** [README.md](./README.md)

This directory is reserved for bundled Tauri resources.

## Binary provenance

Every committed binary is pinned (sha256, size and origin) in `apps/desktop/src-tauri/binary-provenance.json`.

- `pnpm binaries:check` validates the manifest and that every tracked binary is listed with the right hash.
- `pnpm binaries:verify-upstream` downloads the upstream binaries and checks that they match their pinned release byte for byte.

CI runs both commands weekly and on every PR that touches these paths.

`lib/pdfium.dll` (bblanchon/pdfium-binaries `chromium/7543`) and the `uv.exe` files (Astral `0.6.14`) are verified byte-identical to their releases. They stay in git for now: `tauri.windows.conf.json` is merged into every Windows build (CI `cargo test` included), so moving them to a build-time download needs a fetch step in every Windows job and in local dev, plus Windows validation. Deleting them would not shrink the repository without a history rewrite agreed with the team.

The `.mnn` models and the base MSIX are vendored with no reproducible public source: their hashes are pinned and their origin should be documented the next time they are regenerated. The names indicate PaddleOCR PP-OCRv5 / PP-LCNet models converted to MNN; the source model version and the conversion tool/version were not recorded.

## Native libraries

- `lib/pdfium.dll` — Pdfium native library for PDF rendering (Windows x86_64).
  Sourced from bblanchon/pdfium-binaries `chromium/7543`; hash and origin pinned in `binary-provenance.json`.
  The DLL is resolved at runtime with a 3-tier search (bundled → dev → system library).
  See `resources/lib/.gitkeep` for details.
- `lib/linux-x86_64/` — **removed**. Its old text fixtures (49/54 B) moved to `tests/fixtures/resource-audit/` as audit artifacts. Linux bundles no longer include them: Linux Pdfium is downloaded by `apps/desktop/scripts/fetch-pdfium.sh linux-x64` into `resources/pdfium/libpdfium.so` (+ `LICENSE`) and bundled by the explicit release overlays `tauri.pro.linux.conf.json` and `tauri.lite.linux.conf.json` (not by the auto-merged `tauri.linux.conf.json`, so `cargo build` on Linux does not need the download); the resolver in `src/ocr/pdf.rs` rejects candidates that are not native libraries (ELF/PE/Mach-O/fat magic and >= 1 MiB).
- Release runtime payloads may also inject ONNX Runtime under `runtime-pack/<platform>/resources/lib/` when validated ONNX consumers require it.

## Bundled tools

- `tools/uv/windows-x86_64/uv.exe` — bundled `uv` 0.6.14 for Windows x64.
- `tools/uv/windows-aarch64/uv.exe` — bundled `uv` 0.6.14 for Windows ARM64.
  Runtime resolution prefers bundled Tauri resources, then dev resources, then the legacy app-data managed copy, then system `PATH`.

## Runtime-pack fixtures

- `runtime-pack/windows-x86_64/` and `runtime-pack/linux-x86_64/` exist in-repo as **minimal viable fixture packs**.
- Each pack ships `manifest.json`, placeholder Python/uv launchers, managed scripts, cache placeholders, wheelhouse notes, and mirrored native-lib paths.
- `payload_profile: fixture` means these packs are structurally real and bundleable, but they are NOT the final heavy release payloads.
- `release_injection_required: true` in the fixture means the in-repo pack is NOT the final heavy runtime: the app obtains it at runtime, not from the installer.
- The managed runtime and its bootstrap belong exclusively to **EntropIA Pro** (`local-ml`). **EntropIA Lite** uses remote providers and does not download or consume this local runtime.
- **Distribution model: lean installer.** The installer ships only the fixture; the AI runtime (~2.2GB) is hosted separately (tag `runtime-bootstrap`, split under 2 GiB per asset + ed25519-signed) and the app downloads and verifies it (signature + sha256) at startup, in resumable parts with progress, pause, and retry in the runtime banner; once the runtime is ready it installs the AI dependencies from the offline wheelhouse on its own. The runtime is not injected into the installer, because NSIS/WiX do not support bundles >2GB.
- **In-repo now**: runtime-pack layout, manifest contract, fixture bundle globs, assembly wiring, smoke checks, and explicit offline ownership boundaries.

### Release flow (lean)

1. **Build Runtime Pack** (once per platform: `windows-x86_64`, `linux-x86_64`) → assembles a fresh runtime-pack and uploads the `runtime-archive` artifact.
2. **Publish Runtime Bootstrap** with that `runtime_pack_run_id` → splits the archive under 2 GiB per asset, uploads the parts to the `runtime-bootstrap` tag, and publishes a signed `manifest.json`.
3. Push a `v*` tag → **Release** builds the lean Pro installers: NSIS + MSI on Windows and DEB on Linux, with the manifest URL + public key baked into the binary. Before compiling, the workflow requires the signed manifest to have the exact version and platform entry (`apps/desktop/scripts/check-runtime-bootstrap.mjs`), and `build.rs` fails closed if a release build embeds the fixture without a baked source, so a Pro installer that cannot download the runtime is never published.

See `scripts/prepare_runtime_payload.py`, `scripts/materialize_windows_runtime_payload.py`, `scripts/build_runtime_pack.py`, `scripts/runtime-pack-smoke.py`, and each platform `ASSEMBLY_NOTES.md` for the release handoff contract.

## OCR models

The native OCR fallback uses the bundled PaddleOCR/MNN assets in `models/ocr/`:

- `PP-OCRv5_mobile_det.mnn`
- `latin_PP-OCRv5_mobile_rec_infer.mnn`
- `PP-LCNet_x1_0_doc_ori.mnn`
- `ppocr_keys_latin.txt`

PaddleOCR-VL high-quality OCR runs through the managed Python runtime/script path and does not require separate model files committed in this directory.
