# Third-Party Notices and Release Payload Policy

**Español:** [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md)

EntropIA Pro depends on Rust, Node, Python, native libraries, AI models, and runtime payload artifacts. This file records the release-time review policy; it is not yet a complete generated SBOM.

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
| Pdfium | PDF rendering | `resources/lib/pdfium.dll`, release runtime payload native libs | Needs version/license trace in release notes or SBOM. |
| ONNX Runtime | Active ONNX consumers: PP-DocLayout-L layout detection, local BGE-M3 embeddings, RAG reranker | release payload `resources/lib/onnxruntime.dll` or `resources/lib/libonnxruntime.so` | Required in release payloads; the legacy ONNX NER is gone. |
| uv | Managed Python environment bootstrap | `resources/tools/uv/*`, runtime payload `uv/` | Needs version/license trace. |
| Python runtime | OCR/NLP/transcription subprocess runtime | release runtime payload `python/` | Must be redistributable and version-stamped. |
| Python wheelhouse | Offline install for AI dependencies | release runtime payload `wheelhouse/` | Must be generated from reviewed packages. |
| Hugging Face caches | faster-whisper/model cache seeds | release runtime payload `caches/hf/` | Each model license must be reviewed. |
| PaddleX caches | PaddleOCR-VL/layout model cache seeds | release runtime payload `caches/paddlex/` | Each model license must be reviewed. |
| Gemma GGUF | Local LLM downloaded by user/app | Hugging Face URL configured in LLM settings | Downloaded model terms must be visible to users before relying on redistribution. |

## License risks already identified

- spaCy and the `es_core_news_md` model are already in the managed runtime wheelhouse (spec in `deps/registry.rs`; release smoke requires them). Verify the terms before redistributing: spaCy is MIT, `es_core_news_md` is share-alike (CC BY-SA).
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
