# Release 1.8.5

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión corrige lo que encontramos al probar **EntropIA Pro 1.8.4** de Microsoft Store: la preparación de las dependencias de IA, la limpieza de runtimes viejos, las miniaturas de PDF apaisados y el panel de ayuda. También mejora la transcripción de algunos archivos WAV.

No agrega migraciones: se puede actualizar en cualquier orden entre equipos con 1.8.4.

### Pro y Lite

- **Miniaturas de PDF apaisados.** La miniatura de un PDF apaisado ya no aparece girada 90 grados. Las miniaturas viejas se regeneran solas la primera vez.
- **Panel de ayuda.** Las opciones debajo de la pregunta tienen nombres cortos y entran completas en el panel chico: **OpenRouter**, **Diagnóstico**, **Mis documentos** y **Ver input**.
- **Borrar colecciones.** Si Windows no deja borrar una carpeta de una sola vez, EntropIA la borra archivo por archivo.

### Pro

- **Dependencias de IA después de descargar el runtime.** Cuando el runtime de IA se descarga con la app abierta, las dependencias se instalan en ese momento. Antes figuraban como faltantes hasta reiniciar la app.
- **Runtimes viejos.** La app borra los runtimes de versiones anteriores. Antes, en algunos equipos Windows el borrado fallaba y quedaban ocupando espacio.
- **Transcripción de WAV comprimidos.** Parakeet transcribe los WAV comprimidos (por ejemplo, ADPCM de grabadoras de voz) con identificación de hablantes. Antes pasaban a faster-whisper sin etiquetas de hablante.

## English

This release fixes what we found testing **EntropIA Pro 1.8.4** from the Microsoft Store: AI dependency setup, cleanup of old runtimes, landscape PDF thumbnails and the help panel. It also improves the transcription of some WAV files.

It adds no migrations: devices on 1.8.4 can update in any order.

### Pro and Lite

- **Landscape PDF thumbnails.** A landscape PDF's thumbnail no longer appears turned 90 degrees. Old thumbnails regenerate on their own the first time.
- **Help panel.** The options below the question have short names that fit in the small panel: **OpenRouter**, **Diagnostics**, **My documents** and **View input**.
- **Deleting collections.** When Windows refuses to delete a folder in one go, EntropIA deletes it file by file.

### Pro

- **AI dependencies after the runtime downloads.** When the AI runtime downloads while the app is open, the dependencies install right away. Before, they showed as missing until the app restarted.
- **Old runtimes.** The app deletes the runtimes of previous versions. Before, on some Windows machines the deletion failed and they kept taking up space.
- **Compressed WAV transcription.** Parakeet transcribes compressed WAV files (for example, ADPCM from voice recorders) with speaker labels. Before, they fell back to faster-whisper without speaker labels.

---

## Part 2 — Release checklist

- [x] Version bump — `pnpm version:bump 1.8.5`; `pnpm version:check` passes.
- [ ] PR to main, CI green, merge.
- [ ] On main: Build Runtime Pack for **windows-x86_64** and **linux-x86_64** (`app_version 1.8.5`, new `pack_version 2026.11.9`, `native_source_release_tag runtime-source-0.1.0`). The pack content does not change, but every pack carries its `app_version`, and the app only accepts a bootstrap release for its exact version.
- [ ] Publish the runtime bootstrap for **app_version 1.8.5** for both platforms (`publish-runtime-bootstrap.yml`: `platform`, `app_version 1.8.5`, `pack_version 2026.11.9`, `runtime_pack_run_id`, `release_tag runtime-bootstrap`) — before tagging; `release.yml` gates each Pro leg on it.
- [ ] Push tag `v1.8.5` on main (Pro installers + Lite MSIX + draft release).
- [ ] Release draft: paste Part 1 as the body, publish.
- [ ] Build the Pro Store MSIX 1.8.5.0 (`build-pro-store-msix.ps1`).
- [ ] Submit both MSIX packages in Partner Center.
