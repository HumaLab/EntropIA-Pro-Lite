# Release 1.8.4

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión corrige lo que encontramos al probar **EntropIA Pro 1.8.3** de Microsoft Store: las dependencias de IA después de actualizar, el OCR de páginas apaisadas y el modelo local que responde en el chat de documentos.

No agrega migraciones: se puede actualizar en cualquier orden entre equipos con 1.8.3.

### Pro

- **Dependencias de IA después de actualizar.** La app busca el runtime de IA en la carpeta correcta. Antes, después de una actualización seguía usando el Python de la versión anterior y no preparaba el entorno del runtime nuevo.
- **PaddlePaddle en equipos con NVIDIA.** PaddlePaddle y PaddleOCR ya no quedan marcados como fallidos en equipos con placa NVIDIA. El runtime instala la versión para CPU, que funciona en cualquier equipo.
- **Páginas apaisadas.** El OCR ya no gira 90 grados las páginas apaisadas de un PDF, como un diario abierto o un escaneo de dos folios. Antes, el texto llegaba de costado y el OCR liviano devolvía apenas unos caracteres.
- **Corrección de páginas densas.** Cuando la corrección local de una página muy densa se traba repitiendo texto, se reintenta por fragmentos más chicos. Los fragmentos que fallan conservan su texto original.
- **Respuestas del modelo local.** Los flujos locales usan el formato de conversación de Gemma 4. Ya no se cuela un `</start_of_turn>` al final de las respuestas del chat sobre tus documentos.
- **Linux.** El paquete `.deb` encuentra su `llama-server`. El OCR local con GLM-OCR y el corrector Qwen ya no lo dan por faltante.

## English

This release fixes what we found testing **EntropIA Pro 1.8.3** from the Microsoft Store: AI dependencies after an update, OCR of landscape pages, and the local model that answers in the document chat.

It adds no migrations: devices on 1.8.3 can update in any order.

### Pro

- **AI dependencies after an update.** The app looks for the AI runtime in the right folder. Before, after an update it kept the previous version's Python and never set up the new runtime's environment.
- **PaddlePaddle on NVIDIA machines.** PaddlePaddle and PaddleOCR no longer stay failed on machines with an NVIDIA card. The runtime installs the CPU build, which works on any machine.
- **Landscape pages.** OCR no longer turns landscape PDF pages 90 degrees, such as an open newspaper or a double-folio scan. Before, the text reached the recognizer sideways and light OCR returned only a few characters.
- **Dense page correction.** When the local correction of a very dense page gets stuck repeating text, it retries in smaller chunks. Chunks that fail keep their original text.
- **Local model answers.** Local flows use Gemma 4's conversation format. A stray `</start_of_turn>` no longer appears at the end of document chat answers.
- **Linux.** The `.deb` package finds its `llama-server`. Local OCR with GLM-OCR and the Qwen corrector no longer report it missing.

---

## Part 2 — Release checklist

- [x] Version bump — `pnpm version:bump 1.8.4`; `pnpm version:check` passes.
- [ ] PR to main, CI green, merge.
- [ ] On main: Build Runtime Pack for **windows-x86_64** and **linux-x86_64** (`app_version 1.8.4`, new `pack_version 2026.11.8`, `native_source_release_tag runtime-source-0.1.0`).
- [ ] Publish the runtime bootstrap for **app_version 1.8.4** for both platforms (`publish-runtime-bootstrap.yml`: `platform`, `app_version 1.8.4`, `pack_version 2026.11.8`, `runtime_pack_run_id`, `release_tag runtime-bootstrap`) — before tagging; `release.yml` gates each Pro leg on it.
- [ ] Push tag `v1.8.4` on main (Pro installers + Lite MSIX + draft release).
- [ ] Release draft: paste Part 1 as the body, publish.
- [ ] Build the Pro Store MSIX 1.8.4.0 (`build-pro-store-msix.ps1`).
- [ ] Submit both MSIX packages in Partner Center.
