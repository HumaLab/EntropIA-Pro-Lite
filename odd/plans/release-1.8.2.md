# Release 1.8.2

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión corrige lo que encontramos al usar **EntropIA Pro 1.8.1** de Microsoft Store: el runtime de IA que no terminaba de prepararse, el OCR local, los mensajes que no decían la causa real y el estado de los procesos largos. También suma las notas de voz en Opus, en Pro y en Lite.

No agrega migraciones: se puede actualizar en cualquier orden entre equipos con 1.8.1.

### Pro: arreglos importantes

- **Runtime de IA.** Una carpeta vieja del runtime que Windows no deja borrar ya no frena la preparación en cada arranque: queda registrada en el log y la preparación sigue.
- **La causa real, a la vista.** Si preparar el runtime falla, Inicio y Dependencias de IA muestran el error. Dependencias ya no dice «todo listo» mientras el runtime todavía tiene que descargarse, aunque haya un Python viejo con las dependencias instaladas.
- **Dictado y transcripción.** Solo Whisper local necesita el runtime de IA; Parakeet y AssemblyAI funcionan aunque el runtime no esté listo. Si el dictado falla, la nota muestra el motivo y un botón a Dependencias de IA.
- **OCR local.** El OCR liviano encuentra los modelos que trae el instalador, y el OCRH local encuentra sus scripts.
- **Reintentar un OCR.** Pedir de nuevo un OCR que quedó trabado por la configuración lo vuelve a poner en la cola.
- **Dependencias.** Reinstalar después de cambiar de runtime arma el entorno nuevo; «Resetear entorno» no queda en un ciclo por una carpeta bloqueada; PaddlePaddle se instala junto con `setuptools`.

### Pro y Lite

- **Notas de voz en Opus** (WhatsApp, Telegram): se importan, se escuchan y se transcriben, también como `.opus` suelto. Si Parakeet falla y la transcripción pasa a faster-whisper, avisa que el texto sale sin etiquetas de hablante.
- **Procesos en curso.** Un botón de un proceso largo sigue mostrando «en curso» si salís de la pantalla y volvés.
- **Configuración** no muestra valores por defecto mientras carga, y no deja guardar hasta terminar de cargar.
- **Chat de ayuda.** El historial sigue la última respuesta mientras estás abajo.
- **Archivos huérfanos.** El barrido también quita las carpetas vacías de ítems borrados.

## English

This release fixes what we found running **EntropIA Pro 1.8.1** from the Microsoft Store: an AI runtime that never finished preparing, local OCR, messages that hid the real cause, and the state of long-running processes. It also adds Opus voice notes, in Pro and Lite.

It adds no migrations: devices on 1.8.1 can update in any order.

### Pro: important fixes

- **AI runtime.** An old runtime folder Windows will not let the app delete no longer stops the preparation on every launch: it is logged and the preparation continues.
- **The real cause, on screen.** When preparing the runtime fails, Home and AI Dependencies show the error. AI Dependencies no longer says "all ready" while the runtime still has to download, even with an old Python that has the dependencies installed.
- **Dictation and transcription.** Only local Whisper needs the AI runtime; Parakeet and AssemblyAI work while the runtime is not ready. When dictation fails, the note shows the reason and a button to AI Dependencies.
- **Local OCR.** Lightweight OCR finds the models the installer ships, and local OCRH finds its scripts.
- **Retrying an OCR.** Requesting again an OCR stuck on its configuration puts it back in the queue.
- **Dependencies.** Reinstalling after a runtime change builds the new environment; "Reset environment" no longer loops on a locked folder; PaddlePaddle installs together with `setuptools`.

### Pro and Lite

- **Opus voice notes** (WhatsApp, Telegram): they import, play and transcribe, also as a standalone `.opus` file. When Parakeet fails and transcription falls back to faster-whisper, it warns that the text has no speaker labels.
- **Work in progress.** A long-running process's button keeps showing "in progress" after you leave the screen and come back.
- **Settings** shows no default values while it loads, and cannot save until it has loaded.
- **Help chat.** The history follows the newest answer while you stay at the bottom.
- **Orphan files.** The sweep also removes empty folders of deleted items.

---

## Part 2 — Release checklist

- [x] Version bump — `pnpm version:bump 1.8.2`; `pnpm version:check` passes.
- [ ] PR to main, CI green, merge.
- [ ] On main: Build Runtime Pack for **windows-x86_64** and **linux-x86_64** (`app_version 1.8.2`, new `pack_version 2026.11.6`, `native_source_release_tag runtime-source-0.1.0`). The pack content does not change, but every pack carries its `app_version`, and the app only accepts a bootstrap release for its exact version.
- [ ] Publish the runtime bootstrap for **app_version 1.8.2** for both platforms (`publish-runtime-bootstrap.yml`: `platform`, `app_version 1.8.2`, `pack_version 2026.11.6`, `runtime_pack_run_id`, `release_tag runtime-bootstrap`) — before tagging; `release.yml` gates each Pro leg on it.
- [ ] Push tag `v1.8.2` on main (Pro installers + Lite MSIX + draft release).
- [ ] Release draft: paste Part 1 as the body, publish.
- [ ] Build the Pro Store MSIX 1.8.2.0 (`build-pro-store-msix.ps1`, as in the 1.8.1 checklist).
- [ ] Submit both MSIX packages in Partner Center.
