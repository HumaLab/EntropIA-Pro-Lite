# Release 1.8.3

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión corrige lo que encontramos al probar **EntropIA Pro 1.8.2** de Microsoft Store: la separación de hablantes en audios cortos, y un runtime de IA que se ve viejo o a medio descargar. También ordena el chat de ayuda.

No agrega migraciones: se puede actualizar en cualquier orden entre equipos con 1.8.2.

### Pro

- **Hablantes en audios cortos.** La transcripción con Parakeet vuelve a separar hablantes en notas de voz y clips cortos. Antes, algunos de esos audios fallaban y la transcripción pasaba a faster-whisper, sin etiquetas de hablante.
- **Runtime de IA después de actualizar.** Dependencias de IA ya no usa un Python que quedó en la carpeta del runtime de la versión anterior. Usa el del runtime actual.
- **Runtime que se está descargando.** Mientras la base de IA se descarga por primera vez, Dependencias de IA lo dice y avisa que las funciones locales que la necesitan quedan en pausa. La nota sobre `uv` se actualiza cuando el runtime cambia de estado.

### Pro y Lite

- **Chat de ayuda.** Las opciones «Usar OpenRouter», «Incluir diagnóstico» y «Buscar en mis documentos» quedan en una sola línea. La nota sobre quién responde aparece debajo del cuadro de la pregunta.

## English

This release fixes what we found testing **EntropIA Pro 1.8.2** from the Microsoft Store: speaker separation in short recordings, and an AI runtime that looked stale or half-downloaded. It also tidies the help chat.

It adds no migrations: devices on 1.8.2 can update in any order.

### Pro

- **Speakers in short recordings.** Parakeet transcription separates speakers again in voice notes and short clips. Before, some of those recordings failed and transcription fell back to faster-whisper, with no speaker labels.
- **AI runtime after an update.** AI Dependencies no longer uses a Python left in the previous version's runtime folder; it uses the current runtime's.
- **A runtime still downloading.** While the AI base downloads for the first time, AI Dependencies says so and warns that the local features that need it are paused. The `uv` note refreshes when the runtime changes state.

### Pro and Lite

- **Help chat.** The "Use OpenRouter", "Include diagnostics" and "Search my documents" options sit on one line, and the note on who answers sits under the question box.

---

## Part 2 — Release checklist

- [x] Version bump — `pnpm version:bump 1.8.3`; `pnpm version:check` passes.
- [ ] PR to main, CI green, merge.
- [ ] On main: Build Runtime Pack for **windows-x86_64** and **linux-x86_64** (`app_version 1.8.3`, new `pack_version 2026.11.7`, `native_source_release_tag runtime-source-0.1.0`). The pack content does not change, but every pack carries its `app_version`, and the app only accepts a bootstrap release for its exact version.
- [ ] Publish the runtime bootstrap for **app_version 1.8.3** for both platforms (`publish-runtime-bootstrap.yml`: `platform`, `app_version 1.8.3`, `pack_version 2026.11.7`, `runtime_pack_run_id`, `release_tag runtime-bootstrap`) — before tagging; `release.yml` gates each Pro leg on it.
- [ ] Push tag `v1.8.3` on main (Pro installers + Lite MSIX + draft release).
- [ ] Release draft: paste Part 1 as the body, publish.
- [ ] Build the Pro Store MSIX 1.8.3.0 (`build-pro-store-msix.ps1`, as in the 1.8.2 checklist).
- [ ] Submit both MSIX packages in Partner Center.
