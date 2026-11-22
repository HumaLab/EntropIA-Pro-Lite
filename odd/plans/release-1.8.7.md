# Release 1.8.7

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión corrige los problemas encontrados al probar **EntropIA Pro 1.8.6**. Los dos más graves estaban en la transcripción y en el OCR: Parakeet pasaba al inglés el audio en español, y el OCR liviano podía perder una línea entera sin avisar. Además, una transcripción ya no se pierde al volver a transcribir: queda guardada como versión anterior y se puede restaurar.

**Agrega una migración (0066).** Conviene actualizar todos los equipos que sincronizan la misma cuenta: el primero que sincronice con 1.8.7 sube la versión de esquema de la cuenta, y un equipo con 1.8.6 deja de poder sincronizar hasta actualizar.

### Pro y Lite

- **Versiones anteriores de una transcripción.** Antes de reemplazar una transcripción, la app la guarda, con los nombres de hablantes que se hayan escrito a mano. El panel de la transcripción lista las versiones anteriores y restaura cualquiera; la actual pasa a ser una versión, así que no se pierde nada. Las versiones quedan en el equipo: no se sincronizan.
- **Confirmación antes de volver a transcribir.** Si el audio ya tiene una transcripción, la app pregunta antes de empezar.
- **Transcripciones de AssemblyAI con tiempos y hablantes.** Cada intervención guarda su tiempo y su hablante. Antes, todo el texto quedaba en un solo segmento sin hablante.
- **Búsqueda sin marcas de diseño.** El texto de GLM-OCR conserva su formato en la solapa Texto, pero la búsqueda y las fuentes del Chat usan solo las palabras. Antes, marcas como `<div align="center">` o las regiones de imagen aparecían en las fuentes y coincidían con las búsquedas. Al iniciar, la app vuelve a indexar el archivo una vez, en segundo plano.
- **Orden de la búsqueda global.** Los resultados aparecen por relevancia. Antes se ordenaban por título.
- **Cambios sin guardar en Configuración.** Al salir de Motores con cambios sin guardar, la app pregunta. Antes, elegir OpenRouter y salir sin guardar lo descartaba sin aviso.
- **Sincronizar ahora.** El botón muestra «Sincronizando…» mientras corre la sincronización, avisa cuando termina y la tarjeta muestra la hora de la última sincronización.
- **Estado de cada proceso.** Las etiquetas de extracción, transcripción y Análisis dicen «sin hacer», «en cola», «en curso», «hecho» o «error». Antes decían `idle`, `running` o `done`, en inglés, y `idle` parecía «no hecho» aunque el documento ya estuviera indexado.
- **Borrar una investigación.** El diálogo nombra la investigación que se borra y se cierra al terminar.
- **Ayuda de Mis documentos.** La ayuda nombra la casilla **OpenRouter** con su nombre actual.

### Pro

- **Parakeet y el audio en español.** Cuando Parakeet pasa al inglés un audio en español, la app vuelve a transcribirlo con Whisper.
- **Líneas que perdía el OCR liviano.** Una línea que el análisis de diseño no cubría quedaba fuera del texto. Ahora entra en su lugar, en el orden de lectura.
- **OCRL en páginas de PDF.** Las páginas de un PDF ofrecen OCRL directamente. Antes había que pasar por OCRH y su aviso.
- **Sin GPU, OCRL.** El manual indica OCRL como la opción rápida en un equipo sin GPU NVIDIA.
- **Registro del OCR.** Una corrida de OCRL ya no figura en el registro como OCRH ni como PaddleOCR-VL.

## English

This release fixes the problems found while testing **EntropIA Pro 1.8.6**. The two worst were in transcription and OCR: Parakeet turned Spanish audio into English, and light OCR could drop a whole line without a warning. A transcript is also no longer lost when you transcribe again: it is kept as a previous version you can restore.

**It adds a migration (0066).** Update every device that syncs the same account: the first one to sync with 1.8.7 raises the account's schema version, and a device on 1.8.6 cannot sync until it updates.

### Pro and Lite

- **Previous versions of a transcript.** Before replacing a transcript, the app keeps it, with any speaker names typed by hand. The transcript panel lists the previous versions and restores any of them; the current one becomes a version, so nothing is lost. Versions stay on the device: they do not sync.
- **Confirmation before transcribing again.** When the audio already has a transcript, the app asks before starting.
- **AssemblyAI transcripts with times and speakers.** Each utterance keeps its time and speaker. Before, the whole text was one segment with no speaker.
- **Search without layout markup.** GLM-OCR text keeps its formatting in the Text tab, but search and Chat sources use only the words. Before, markup such as `<div align="center">` or image regions showed up in sources and matched searches. On start, the app re-indexes the archive once, in the background.
- **Global search order.** Results come by relevance. Before, they were sorted by title.
- **Unsaved changes in Settings.** Leaving Engines with unsaved changes asks first. Before, picking OpenRouter and leaving without saving dropped the choice silently.
- **Sync now.** The button reads "Syncing…" while the sync runs, says when it is done, and the card shows the time of the last sync.
- **State of each process.** The extraction, transcription and Analysis labels read "not run", "queued", "running", "done" or "error", in the interface language. Before, they read `idle`, `running` or `done` in English, and `idle` looked like "not done" even for a document already indexed.
- **Deleting an investigation.** The dialog names the investigation it deletes and closes when it is done.
- **My documents help.** The help names the **OpenRouter** checkbox by its current label.

### Pro

- **Parakeet and Spanish audio.** When Parakeet turns Spanish audio into English, the app transcribes it again with Whisper.
- **Lines that light OCR dropped.** A line that the layout analysis did not cover was left out of the text. It now goes in its place, in reading order.
- **OCRL on PDF pages.** PDF pages offer OCRL directly. Before, you had to go through OCRH and its warning.
- **Without a GPU, OCRL.** The manual points to OCRL as the fast option on a computer without an NVIDIA GPU.
- **OCR log.** An OCRL run no longer shows in the log as OCRH or PaddleOCR-VL.

---

## Part 2 — Release checklist

- [x] Version bump — `pnpm version:bump 1.8.7`; `pnpm version:check` passes.
- [ ] PR to main, CI green, merge.
- [ ] Runtime: 1.8.7 changes nothing in the Python runtime (`git diff v1.8.6 main` touches no runtime script or lock), so the installed `2026.11.10` should be reused. Confirm whether the runtime bootstrap still needs publishing for **app_version 1.8.7** on both platforms (`publish-runtime-bootstrap.yml`, `pack_version 2026.11.10`) before tagging.
- [ ] Check in the real app (Pro build) before tagging: a Spanish audio with Parakeet stays Spanish (finding 13); the test image `prueba-importacion.png` keeps all five lines with OCRL (finding 15); re-transcribe an audio, then restore the previous version; «Sincronizar ahora» shows the run and the time; the Análisis badges read «hecho» on an indexed document.
- [ ] Push tag `v1.8.7` on main (Pro installers + Lite MSIX + draft release).
- [ ] Release draft: title «EntropIA Pro & Lite v1.8.7», paste Part 1 as the body, publish.
- [ ] Build the Pro Store MSIX 1.8.7.0 (`build-pro-store-msix.ps1`); the Store number does not move with the app version on its own.
- [ ] Update every synced device: migration 0066 raises the account `schema_tag` on the first sync.
