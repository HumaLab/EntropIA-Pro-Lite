# Third-Party Notices and Release Payload Policy

**Español:** [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md)

EntropIA (Pro and Lite) depends on Rust, Node, native libraries and, in Pro, on Python, AI models, and runtime payload artifacts. This file records the release-time review policy; it is not yet a complete generated SBOM.

## Release rule

A self-contained installer must not be published unless every bundled runtime artifact is traceable and redistributable.

Before signing or publishing a final installer, verify:

- [ ] Rust and Node dependency licenses are acceptable for redistribution.
- [ ] Python wheels in the release wheelhouse are license-reviewed.
- [ ] Native libraries are license-reviewed and version-pinned.
- [ ] Bundled or seeded model caches have license terms compatible with redistribution.
- [ ] `runtime-pack-smoke.py --release --install-probe` passes on the assembled runtime-pack.
- [ ] The release notes include installer hashes.

## Known bundled/runtime components

| Component | Purpose | Current source/path | Review status |
| --------- | ------- | ------------------- | ------------- |
| Pdfium | PDF rendering and text reading (Pro and Lite) | committed `resources/lib/pdfium.dll`; on Linux and macOS, downloaded at build time by `apps/desktop/scripts/fetch-pdfium.sh` | Pinned to `bblanchon/pdfium-binaries` `chromium/7543` (BSD-3-Clause/Apache-2.0); the Windows copy is recorded byte for byte in `binary-provenance.json`. |
| ONNX Runtime | Active ONNX consumers: PP-DocLayout-L layout detection, local BGE-M3 embeddings, RAG reranker | release payload `resources/lib/onnxruntime.dll` or `resources/lib/libonnxruntime.so` | Required in release payloads; the legacy ONNX NER is gone. |
| uv | Managed Python environment bootstrap (Pro) | `resources/tools/uv/*`, runtime payload `uv/` | `resources/tools/uv/*` pinned to uv 0.6.14 (MIT OR Apache-2.0) in `binary-provenance.json`; the runtime payload copy follows the runtime-pack manifest. |
| Python runtime | OCR/NLP/transcription subprocess runtime | release runtime payload `python/` | Must be redistributable and version-stamped. |
| Python wheelhouse | Offline install for AI dependencies | release runtime payload `wheelhouse/` | Must be generated from reviewed packages. |
| Hugging Face caches | faster-whisper/model cache seeds | release runtime payload `caches/hf/` | Each model license must be reviewed. |
| PaddleX caches | PaddleOCR-VL/layout model cache seeds | release runtime payload `caches/paddlex/` | Each model license must be reviewed. |
| Gemma GGUF | Local LLM downloaded by user/app | Hugging Face URL configured in LLM settings | Downloaded model terms must be visible to users before relying on redistribution. |

## Dependencies compiled into the app

These do not travel as separate runtime-payload files: they are compiled or bundled into the binary and the bundle. Licenses verified against the installed packages.

| Component | Purpose | License |
| --------- | ------- | ------- |
| `keyring` | Stores the sync session token in the operating system credential manager | MIT OR Apache-2.0 |
| `ed25519-dalek` | Verifies the managed-runtime bootstrap signature | BSD-3-Clause |
| `symphonia` | Audio decoding for preview | **MPL-2.0** |
| `hound` | WAV read/write for the audio path | Apache-2.0 |
| `leaflet` | Location map | BSD-2-Clause |
| `@tiptap/*` | Rich-text editor behind notes and Writing | MIT |
| `3d-force-graph` and `three` | 3D relation graph of a collection | MIT |
| `pdfjs-dist` | PDF viewer in the UI | Apache-2.0 |
| `markdown-it` | Rendering OCR text with tables | MIT |
| `@tabler/icons-svelte-runes` | UI icons | MIT |
| `pdfmake` | PDF export | MIT |
| `docx` | DOCX export | MIT |
| `drizzle-orm` | Local database access layer | Apache-2.0 |
| `@fontsource*/*` (Source Sans 3, Source Serif 4, JetBrains Mono, Inter, Literata, IBM Plex Sans, IBM Plex Mono, Lora, Atkinson Hyperlegible Next, Noto Serif, Source Code Pro) | Typography presets; latin/latin-ext subsets only | OFL-1.1 — notices bundled in `resources/fonts/LICENSE` |

## Committed binaries

`apps/desktop/src-tauri/binary-provenance.json` records every committed binary (hash, size, origin and, for upstream ones, the archive and release they come from). `pnpm binaries:check` verifies it offline and `pnpm binaries:verify-upstream` re-derives the upstream ones; the **Verify binary provenance** workflow runs both checks. Pro's MNN OCR models are listed as *vendored*: the source model version and the conversion tool were not recorded.

## License risks already identified

- spaCy and the `es_core_news_md` model are already in the managed runtime wheelhouse (spec in `deps/registry.rs`; release smoke requires them). Verify the terms before redistributing: spaCy is MIT, `es_core_news_md` is share-alike (CC BY-SA).
- `symphonia` is **MPL-2.0**, weak per-file copyleft: it can be linked from a proprietary product, but any modification to its own files must be published. Do not fork it inside this repo without deciding that first.
- Some Hugging Face models may not expose clear license metadata; do not bundle them until the license is confirmed.
- Large PaddleOCR-VL cache artifacts can break Windows installer tooling; do not include oversized files unless the bundler has been validated.

## SBOM expectation

The target release process should produce or attach an SBOM covering:

- Cargo dependencies;
- pnpm dependencies;
- Python wheels;
- native DLL/shared libraries;
- AI model files and seeded caches;
- release runtime-pack manifest checksums.

Until that SBOM exists, this file is the human-readable checklist for release reviewers.
