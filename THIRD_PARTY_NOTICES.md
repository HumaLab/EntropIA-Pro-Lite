# Avisos de terceros y política de payloads de release

**English:** [THIRD_PARTY_NOTICES.en.md](./THIRD_PARTY_NOTICES.en.md)

EntropIA Pro depende de Rust, Node, Python, librerías nativas, modelos de IA y artefactos de runtime payload. Este archivo registra la política de revisión para release; todavía no es un SBOM generado completo.

## Regla de release

No se debe publicar un instalador self-contained salvo que cada artefacto de runtime bundleado sea trazable y redistribuible.

Antes de firmar o publicar un instalador final, verificar:

- [ ] Las licencias de dependencias Rust y Node son aceptables para redistribución.
- [ ] Los wheels Python del wheelhouse de release fueron revisados por licencia.
- [ ] Las librerías nativas fueron revisadas por licencia y fijadas por versión.
- [ ] Los modelos bundleados o caches presembrados tienen términos compatibles con redistribución.
- [ ] `runtime-pack-smoke.py --release --install-probe` pasa sobre el runtime-pack ensamblado.
- [ ] Las notas de release incluyen hashes de instaladores.

## Componentes bundleados/runtime conocidos

| Componente | Propósito | Fuente/ruta actual | Estado de revisión |
| ---------- | --------- | ------------------ | ------------------ |
| Pdfium | Renderizado PDF | `resources/lib/pdfium.dll`, librerías nativas del release runtime payload | Necesita traza de versión/licencia en notas de release o SBOM. |
| ONNX Runtime | Consumidores ONNX activos: layout PP-DocLayout-L, embeddings BGE-M3 local, reranker RAG | release payload `resources/lib/onnxruntime.dll` o `resources/lib/libonnxruntime.so` | Requerido en payloads de release; el NER ONNX legacy ya no existe. |
| uv | Bootstrap del entorno Python administrado | `resources/tools/uv/*`, runtime payload `uv/` | Necesita traza de versión/licencia. |
| Runtime Python | Runtime subprocess para OCR/NLP/transcripción | release runtime payload `python/` | Debe ser redistribuible y estar versionado. |
| Wheelhouse Python | Instalación offline para dependencias IA | release runtime payload `wheelhouse/` | Debe generarse desde paquetes revisados. |
| Caches Hugging Face | Seeds de faster-whisper/model cache | release runtime payload `caches/hf/` | Cada licencia de modelo debe revisarse. |
| Caches PaddleX | Seeds de PaddleOCR-VL/layout model cache | release runtime payload `caches/paddlex/` | Cada licencia de modelo debe revisarse. |
| Gemma GGUF | LLM local descargado por usuario/app | URL de Hugging Face configurada en LLM settings | Los términos del modelo descargado deben ser visibles para usuarios antes de depender de redistribución. |

## Riesgos de licencia ya identificados

- spaCy y el modelo `es_core_news_md` ya están en el wheelhouse del runtime managed (spec en `deps/registry.rs`; el smoke release los exige). Verificar los términos antes de redistribuir: spaCy es MIT, `es_core_news_md` es share-alike (CC BY-SA).
- Algunos modelos de Hugging Face pueden no exponer metadata de licencia clara; no bundlearlos hasta confirmar la licencia.
- Artefactos grandes de cache PaddleOCR-VL pueden romper tooling de instalador Windows; no incluir archivos sobredimensionados salvo que el bundler haya sido validado.

## Expectativa de SBOM

El proceso de release objetivo debería producir o adjuntar un SBOM que cubra:

- dependencias Cargo;
- dependencias pnpm;
- wheels Python;
- DLLs/librerías compartidas nativas;
- archivos de modelos IA y caches presembrados;
- checksums del manifest del release runtime-pack.

Hasta que exista ese SBOM, este archivo es el checklist legible por humanos para revisores de release.
