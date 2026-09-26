# EntropIA — Pro &amp; Lite (unified monorepo)

[![Built with Gentle-AI](https://raw.githubusercontent.com/Gentleman-Programming/gentle-ai/main/docs/assets/brand/built-with-gentle-ai.png)](https://github.com/Gentleman-Programming/gentle-ai)

**Español:** [README.md](./README.md)

A single source tree that produces **two variants** of the desktop app for research with document corpora: **EntropIA Pro** (local + remote AI) and **EntropIA Lite** (100% remote, via APIs). Both are built from the same tree; the variant is chosen at compile time. Built for researchers and teams who organize, process, analyze, and write from collections of images, PDFs, and audio.

EntropIA organizes collections, processes images/PDFs/audio, and enriches results with OCR, transcription, search, embeddings, entities, and semantic triples.

## The two variants

|                     | **EntropIA Pro**                                         | **EntropIA Lite**                                                                                 |
| ------------------- | -------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| OCR                 | local PaddleOCR (Light) + PaddleOCR-VL or GLM-OCR (High) | remote GLM-OCR (Light and High use the same provider)                                             |
| Transcription       | local faster-whisper + AssemblyAI                        | AssemblyAI                                                                                        |
| LLM / NER / RAG     | local Gemma 4 + OpenRouter; local spaCy for NER          | OpenRouter (Gemma 4 by default)                                                                   |
| Embeddings          | local BGE-M3 (ONNX) + OpenRouter (`baai/bge-m3`)         | OpenRouter (`baai/bge-m3`)                                                                        |
| Native ML runtime   | yes (downloaded on first use)                            | no                                                                                                |
| Installer           | Windows: NSIS + MSI (GitHub) · Linux: DEB (GitHub)       | Windows: NSIS + MSI (GitHub) · MSIX (Store) · macOS: universal DMG (GitHub) · Linux: DEB (GitHub) |
| Tauri identifier    | `com.entropia.pro.desktop`                               | `com.entropia.lite`                                                                               |
| Store MSIX identity | —                                                        | `CONICET.EntropIALite`                                                                            |
| Built with          | `--features local-ml` + `VITE_LOCAL_ML=1`                | lean default features + `VITE_LOCAL_ML=0`                                                         |

**Pro** runs AI on the machine by default (offline-first) and lets users select remote providers in settings; `auto` modes apply fallback where implemented. **Lite** is 100% remote (OpenRouter / AssemblyAI / GLM-OCR): no native models or runtime, small installer, Microsoft Store distribution.

## Quick start

- **User manual**: [User manual](https://humalab.github.io/EntropIA-Pro-Lite/manual-usuario/manual-usuario.html) — full usage guide, in Spanish.
- **EntropIA Pro** — Windows x64: `.exe` (NSIS) + `.msi`; Linux x64: `.deb`. Available from [repo Releases](https://github.com/HumaLab/EntropIA-Pro-Lite/releases).
- **EntropIA Lite** — Windows x64: Microsoft Store (<https://apps.microsoft.com/detail/9N328K9L95JD>) or `.exe`/`.msi`; macOS (Apple Silicon and Intel): universal `.dmg`; Linux x64 (Ubuntu 22.04 or later): `.deb`. All from [repo Releases](https://github.com/HumaLab/EntropIA-Pro-Lite/releases).
  - **macOS**: the `.dmg` is not notarized by Apple. On first launch macOS blocks it: open **System Settings → Privacy & Security** and click **Open Anyway**.
  - **Linux**: API keys are stored in the system keyring (gnome-keyring or KWallet). Desktop Ubuntu ships one; on minimal installs, install `gnome-keyring` and create a default keyring.

## Development

### Requirements

- Node.js 22+, pnpm 9
- Rust 1.88.0 (pinned in `rust-toolchain.toml`) / MSVC toolchain on Windows

### Install

```bash
git clone git@github.com:HumaLab/EntropIA-Pro-Lite.git
cd EntropIA-Pro-Lite
pnpm install --frozen-lockfile
```

### User manual

The manual's sources (Markdown + generator) live under `manual/`; they publish to GitHub Pages automatically via `.github/workflows/manual-pages.yml`. To rebuild it locally: `python manual/manual-usuario/build_html.py --out _site` (needs `pip install -r manual/manual-usuario/requirements.txt`).

### Run &amp; build each variant

Everything runs from **`apps/desktop/`**. If you are at the repo root, run `cd apps/desktop` first; otherwise `pnpm exec tauri` cannot find the Tauri CLI because it is installed in the desktop workspace. The variant is selected by three things: the Cargo feature (`local-ml` explicit for Pro; lean default for Lite), the `VITE_LOCAL_ML` frontend flag, and (for Lite) the `tauri.lite.conf.json` Tauri config.

**EntropIA Pro** (compiles MNN from source the first time → ~30 min):

```powershell
cd apps/desktop
$env:VITE_LOCAL_ML='1'
pnpm exec tauri dev   --features local-ml      # dev with hot-reload
pnpm exec tauri build --features local-ml --bundles nsis,msi  # NSIS + MSI installers
```

**EntropIA Lite** (lean, no MNN → starts fast):

```powershell
cd apps/desktop
$env:VITE_LOCAL_ML='0'
pnpm exec tauri dev   --config src-tauri/tauri.lite.conf.json
pnpm exec tauri build --config src-tauri/tauri.lite.conf.json --bundles nsis,msi
```

> - Use **`pnpm exec tauri`** (not `pnpm tauri … -- …`): pnpm eats the first `--` and breaks arg passing to Cargo.
> - To run it from the **repo root** without `cd`, use `pnpm --filter @entropia-pro/desktop exec tauri ...`.
> - Lite is Cargo's lean default. Do not pass `--features local-ml` when using `tauri.lite.conf.json`.
> - In PowerShell `$env:VITE_LOCAL_ML` **persists for the session** → set it on every variant switch (or open a new terminal). In bash it goes inline: `VITE_LOCAL_ML=0 pnpm exec tauri …`.
> - Lite uses `identifier com.entropia.lite` → **separate app data** from Pro (you can run both without clobbering each other).
> - Lite's `tauri build` produces the **`.exe` (NSIS) + `.msi`**; the final Store **MSIX** comes from the repack (see _Release &amp; installers_).
> - With the committed fixture `runtime-pack`, a Windows/Linux release `tauri build` requires `ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL`, `ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID`, and `ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64` to be set to the values in [`.github/workflows/release.yml`](./.github/workflows/release.yml). The workflow sets them for both Pro and Lite; `tauri dev` does not need them. The current `build.rs` guard runs for both variants even though Lite does not use local AI.

### Validate

```bash
pnpm lint                                                       # entire workspace
pnpm typecheck                                                  # workspace; Pro desktop frontend
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck   # Lite desktop frontend
pnpm test                                                       # workspace; Pro desktop tests
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test        # Lite desktop tests
cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml --features local-ml  # Pro (Rust)
cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml                      # Lite (Rust)
```

## Capabilities

Both variants cover the same core research workflows; the engine changes (local vs remote, see the table above). Their runtime and UI feature sets are not literally identical: Pro adds local engines and dependency/model management.

- Home: startup view with what to resume, corpus status (OCR, embeddings, pending work), and quick access to Collections, Chat, Research, and Writing.
- Corpus organization into collections, items, and local assets (SQLite).
- Image, PDF, and audio ingestion; exporting a collection to JSON (documents, text, notes, entities, layout).
- OCR Light + OCR High with layout persistence (blocks, regions, pages, bounding boxes).
- Audio transcription.
- LLM-assisted correction, summary, and semantic extraction.
- Entities, triples, NER, FTS, and asset-level embeddings (RAG).
- Batch processing: OCR and embeddings over entire collections from Settings, in the background, with per-item retries and resume after closes or outages.
- Notes, annotations, and manual result editing: entities and triples are created, edited and deleted by hand, not only read.
- Per-process status on each document (indexing, embeddings, NER, triples), so what already ran is visible without running it again.
- Research chat over the corpus, with each new conversation titled automatically.
- Writing: a manuscript editor with an outline, a research panel, and citations anchored to the corpus or to a local Zotero library (CSL styling); an AI writing agent that never edits on its own; export to Markdown, HTML, or Word with footnotes and a bibliography.
- Browser-style tabs (up to 4) and a two-pane split view, each pane with its own navigation.
- Collection grid with keyset pagination and thumbnails generated for what is looked at, not for everything loaded.
- Sidebar collection tree, with each collection paged in batches.
- Appearance: themes (dark, warm, light, and Lite), contrast levels, and typography presets.
- Browser-style UI zoom (75%-125%), from the topbar or `Ctrl +/-/0`.
- Cross-device sync (deterministic ids for duplicate-free convergence).
- Microsoft Store update notice (Lite on Windows): detects a newer published version and opens the Store listing; it does not download or install anything on its own.

## How the variant flag works

The unification is a **strangler** over the Pro code: all local inference lives behind the `local-ml` Cargo feature (with a `paddle-ocr` sub-feature for MNN/PaddleOCR), mirrored by the `VITE_LOCAL_ML` frontend flag.

- **`cargo build --features local-ml`** = **Pro** (local + remote engines).
- **`cargo build` (default)** = lean → **Lite** (remote only). Drops `ort`/onnxruntime, `llama-cpp-2`, MNN/`ocr-rs`, `tokenizers`, and the signed runtime download.
- The **frontend** reads `VITE_LOCAL_ML`: in Lite it hides DependenciasTab, the deps banners, and the local-model UI, and the brand becomes "EntropIA Lite".
- The **Tauri command list is identical** in both variants; only the bodies branch (the Lite arm returns healthy/no-op, like EntropIA Lite did).

On every push/PR, CI runs workspace lint, typecheck, and tests with the Pro frontend, plus desktop typecheck and tests with `VITE_LOCAL_ML=0`; it also builds the Pro frontend. When relevant Rust/Tauri files change, the Windows feature contract compiles and links Pro and Lite as blocking gates.

## Release &amp; installers

**Pro — lean installer + download-on-first-use.** The AI runtime (~2.2GB) does not fit inside a Windows installer (NSIS and WiX fail above ~2GB). The installer ships the small `runtime-pack` fixture and the app downloads the real runtime on first use from a signed remote source (ed25519), verifying signature + sha256 before trusting it. On Windows/Linux, `build.rs` fails closed if any release build embeds the fixture without a baked bootstrap source; the **Release** workflow sets it for both Pro and Lite.

Pro release flow:

1. **Build Runtime Pack** → builds a fresh runtime-pack (`runtime-archive` artifact).
2. **Publish Runtime Bootstrap** with that `runtime_pack_run_id` → splits the archive under GitHub's 2 GiB per-asset limit, uploads the parts to the `runtime-bootstrap` tag, and publishes a signed `manifest.json`.
3. Push a `v*` tag → the **Release** workflow builds NSIS + MSI on Windows and DEB on Linux, with the manifest URL + public key **baked** into the binary.

**Lite — GitHub installers + MSIX for the Store.** The `build-lite` job in the **Release** workflow builds the lean variant with `--bundles nsis,msi`; the `attach-lite-installers` job attaches the `.exe` (NSIS) + `.msi` to the GitHub release (downloadable like Pro's). In parallel, the `.msi` feeds the **repack** of a captured base MSIX (`apps/desktop/src-tauri/msix/`), rewriting the identity to `CONICET.EntropIALite` + the version; the unsigned `.msix` (the Store signs it) remains an Actions artifact for Partner Center only, not a release asset.

**Lite — macOS and Linux.** The `lite-unix` job in the **Release** workflow calls **Lite preview** (`lite-preview.yml`), which builds the universal `.dmg` and the `.deb` and checks that the app starts on Ubuntu 22.04 and 24.04 and on macOS arm64 and Intel. The `attach-lite-unix` job attaches them to the release only if every one of those checks passed. The `.dmg` carries an ad-hoc signature and is not notarized. **Lite preview** can also be dispatched by hand to get the installers without publishing anything.

- To test **only** the Lite MSIX without the Pro build: manually dispatch the **Release** workflow with `lite_only=true` (or `gh workflow run release.yml -f lite_only=true`).
- The base MSIX is re-captured (Hyper-V VM, manual) **only** if the package shape changes (assets/capabilities); routine releases just swap the exe + bump the version.

## Useful documentation

- [SQLite](./SQLite.en.md) — schema and inspection guide for the local database.
- [Database Debugging](./DATABASE_DEBUGGING.en.md) — operational queries for diagnosing persistence.
- [Code Signing](./CODE_SIGNING.en.md) — release signing policy.
- [Privacy](./PRIVACY.en.md) — data, runtime, and external provider behavior.
- [Third Party Notices](./THIRD_PARTY_NOTICES.en.md) — dependencies, models, and runtime payloads.
- License: MIT — see [LICENSE](./LICENSE).

---

**Local AI in Pro. Remote APIs in Lite.**
