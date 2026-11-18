# Release 1.8.1

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión corrige lo que encontramos al usar **EntropIA Pro 1.8.0** de Microsoft Store sobre un archivo real: descargas, memoria, transcripción, OCR local y diagnóstico. También agrega herramientas para asistentes y ordena la configuración de Pro.

No agrega migraciones: se puede actualizar en cualquier orden entre equipos con 1.8.0.

### Pro: arreglos importantes

- **Dictado local en Windows.** El dictado graba en WAV y Parakeet lo transcribe en el equipo (antes fallaba siempre). Parakeet se descarga aparte desde Configuración, con progreso y reanudación; el dictado ya no descarga 2,5 GB en silencio.
- **Respaldo con Whisper.** La transcripción con faster-whisper vuelve a funcionar con el runtime de IA nuevo.
- **faster-whisper y spaCy en la versión de Store.** Ya cargan: el entorno de Python dejaba de registrar una carpeta de sistema que Windows no deja leer.
- **Embeddings.** Pasar a embeddings locales ya no vuelve a procesar todo el archivo; BGE-M3 local se descarga de memoria tras 10 minutos sin uso.
- **Modelos descargados.** Ya no se dan por faltantes en cada arranque.
- **GLM-OCR y Qwen3-VL locales en Windows.** El servidor llama-server (CUDA) se descarga aparte, solo en equipos con GPU NVIDIA.
- **Chat local con Gemma.** El Chat de investigación y la Ayuda responden con Gemma 4 en el equipo cuando el Modo LLM es Local.

### Pro: comportamiento

- Un OCR pedido por vos ya no espera detrás de tareas de fondo, y sigue visible si salís del documento y volvés.
- Aviso antes de OCRH local en un equipo sin GPU; en los lotes podés elegir OCRL u OCRH local.
- La corrección de OCR respeta el Modo LLM y el historial registra el motor real.
- Un lote sin Embeddings no vectoriza sus páginas.
- Menos errores «database is locked» con varias tareas a la vez.
- Pro ofrece una vez pasar a local los modos en la nube heredados de Lite.
- En una conexión medida, la primera descarga del runtime de IA espera tu confirmación.
- «Instalar todo» no corre dos veces ni repite en cada arranque una instalación que ya falló.

### Configuración y asistentes

- **«Usar esta API»** apaga un proveedor sin borrar su clave.
- En Pro, Configuración separa **Motores**, **APIs remotas** y **Modelos locales**; Dependencias de IA lista cada modelo y servidor local con lo que habilita.
- Nueva herramienta MCP **`engines_setup`**: un asistente puede ver y cambiar dónde corre cada función y qué APIs están encendidas (nunca las claves).

### Diagnóstico

- La línea de arranque del registro dice variante, versión y tipo de instalación; un cierre abrupto deja rastro; menos líneas repetidas.
- El instalador x64 ya no incluye el `uv` ARM64 (38 MB).
- `zip` 9.0.5 (RUSTSEC-2026-0548).

## English

This release fixes what we found running **EntropIA Pro 1.8.0** from the Microsoft Store on a real archive: downloads, memory, transcription, local OCR and diagnostics. It also adds assistant tooling and reorganizes Pro's settings.

It adds no migrations: devices on 1.8.0 can update in any order.

### Pro: important fixes

- **Local dictation on Windows.** Dictation records WAV and Parakeet transcribes it on the computer (it used to fail every time). Parakeet downloads separately from Settings, with progress and resume; dictation no longer downloads 2.5 GB silently.
- **Whisper fallback.** Transcription with faster-whisper works again with the new AI runtime.
- **faster-whisper and spaCy in the Store build.** They load now: the Python environment no longer registers a system folder Windows does not let it read.
- **Embeddings.** Switching to local embeddings no longer reprocesses the whole archive; local BGE-M3 is unloaded after 10 minutes idle.
- **Downloaded models.** No longer reported missing on every launch.
- **Local GLM-OCR and Qwen3-VL on Windows.** The llama-server (CUDA) downloads separately, only on machines with an NVIDIA GPU.
- **Local chat with Gemma.** The Research chat and Help answer with Gemma 4 on the computer when LLM mode is Local.

### Pro: behaviour

- An OCR you request no longer waits behind background work, and stays visible when you leave the document and come back.
- A warning before local OCRH on a machine without a GPU; batches let you choose OCRL or local OCRH.
- OCR correction follows LLM mode and the history records the real engine.
- A batch without Embeddings does not embed its pages.
- Fewer "database is locked" errors under concurrent work.
- Pro offers once to switch the cloud modes inherited from Lite to local.
- On a metered connection, the first AI runtime download waits for your confirmation.
- "Install all" does not run twice nor repeat a failed install on every launch.

### Settings and assistants

- **"Use this API"** switches a provider off without deleting its key.
- In Pro, Settings splits **Engines**, **Remote APIs** and **Local models**; AI Dependencies lists each local model and server with what it enables.
- New **`engines_setup`** MCP tool: an assistant can read and change where each feature runs and which APIs are on (never the keys).

### Diagnostics

- The log's startup line names variant, version and install type; an abrupt exit leaves a trace; fewer repeated lines.
- The x64 installer no longer ships the ARM64 `uv` (38 MB).
- `zip` 9.0.5 (RUSTSEC-2026-0548).

---

## Part 2 — Release checklist

- [x] Version bump — `pnpm version:bump 1.8.1` on `release/1.8.1`; `pnpm version:check` passes.
- [ ] PR to main, CI green, merge.
- [ ] On main: Build Runtime Pack for **windows-x86_64** and **linux-x86_64** (`app_version 1.8.1`, new `pack_version 2026.11.5`, `native_source_release_tag runtime-source-0.1.0`). A new pack is required: the installed app runs `transcribe.py` from the downloaded pack, so the Whisper fix ships only with it.
- [ ] Publish the runtime bootstrap for **app_version 1.8.1** for both platforms (`publish-runtime-bootstrap.yml`: `platform`, `app_version 1.8.1`, `pack_version 2026.11.5`, `runtime_pack_run_id`, `release_tag runtime-bootstrap`) — before tagging; `release.yml` gates each Pro leg on it.
- [ ] Push tag `v1.8.1` on main (Pro installers + Lite MSIX + draft release).
- [ ] Release draft: paste Part 1 as the body, publish.
- [ ] Build the Pro MSI (release build, `--features local-ml --bundles msi --config src-tauri/tauri.connector.pro.conf.json`, the three `ENTROPIA_RUNTIME_BOOTSTRAP_*` values from `release.yml`), then the Store MSIX: `build-pro-store-msix.ps1 -MsiPath <Pro MSI> -IdentityName CONICET.EntropIAPro -IdentityPublisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" -PublisherDisplayName HLab -OutDir .tmp/pro-msix` (Store version 1.8.1.0).
- [ ] Submit both MSIX packages in Partner Center.
