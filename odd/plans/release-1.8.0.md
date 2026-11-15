# Release 1.8.0

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión publica **EntropIA Pro** y **EntropIA Lite**. Agrega el **asistente de ayuda en la app (Ayuda)**, la **transcripción local con Parakeet y diarización opcional**, y un **importador de Internet Archive por lotes**, además de mejoras en el **OCR local** y las **opciones de modelos**.

No agrega migraciones: se puede actualizar en cualquier orden entre equipos con 1.7.x.

EntropIA Pro también se ofrece en Microsoft Store como MSIX (publicación en Partner Center pendiente).

### Asistente de ayuda (Ayuda)

- **Burbuja flotante y vista Ayuda.** El chat se abre desde una burbuja arrastrable abajo a la derecha y desde la vista Ayuda, con página de bienvenida cuando no hay clave configurada.
- **Responde sin clave lo básico.** Primeros pasos y preguntas frecuentes se contestan en local con el manual, sin OpenRouter.
- **Busca en el manual y en tus documentos.** Índice FTS5 del manual (se reconstruye por hash) más mapa de pantallas generado desde la interfaz; la búsqueda en documentos propios es opt-in.
- **Actúa con herramientas y permiso.** Modo agente con herramientas MCP controladas, pasos en vivo con botón Detener, límite de rondas y aprobaciones en diálogo que nombran qué se va a tocar.
- **Diagnóstico que no filtra nada.** El informe opt-in recorta rutas, tokens y URLs; los resultados de herramientas ocultan rutas de archivo.
- **Historial y ajustes.** Conversación local con preguntas iniciales, hasta 12 turnos de contexto, y pestaña de ajustes (modelo, contexto, respuestas locales).

### Transcripción local (STT, solo Pro)

- **Parakeet TDT 0.6B v3 como motor local por defecto** (Whisper queda como alternativa), con selección por idioma y la misma ruta para dictado.
- **Diarización opcional con Sortformer.** Casilla Detectar hablantes (apagada por defecto); etiqueta Hablante A/B/… y avisa si hay más de 4 voces.
- **Renombrar hablantes** desde el panel de transcripción, para Parakeet y AssemblyAI.

### OCR local (solo Pro)

- **Motor GLM-OCR persistente** sobre llama-server, con cambio de motor sin reiniciar, PDF a resolución completa y descarga de modelos.
- **Corrección local con Qwen3-VL-4B** en llama-server compartido, por fragmentos que ya no corrompen páginas largas (penalización de repetición 1.05, colas plegadas).
- **Light OCR con layout** y orden de lectura.

### Modelos y generación

- **URL base compatible con OpenAI (puerta vLLM/Ollama).** Apuntar la generación remota a un servidor propio reemplaza la cadena de OpenRouter, no pide clave y falla en voz alta si la URL es mala.
- **Herramienta models_setup (MCP)** para instalaciones de modelos locales, usada por el agente de ayuda.
- Contextos ajustados, resúmenes sin truncado y detección de intención más tolerante.

### Importador de Internet Archive

- **Diálogo ancho maestro-detalle** con tamaño de página, filtro por tipo de archivo y selección de registros enteros, páginas o todos los resultados (acotado).
- **Importa JPEG, PNG y TIFF originales** (acepta BigTIFF) manteniendo nombres y criterio de completitud de los PDF.
- **Cola e importación por lotes** con preparación más rápida, progreso honesto y orden estable.

### Relaciones 3D y detalles

- **El grafo Relaciones 3D se guarda como GEXF** para Gephi y ya no desaparece al cambiar de pestaña.
- Enlace de GitHub centrado en la barra de estado.
- Manual actualizado (mapa de pantallas, preguntas frecuentes y ayuda).

## English

This release publishes **EntropIA Pro** and **EntropIA Lite**. It adds the **in-app help assistant (Ayuda)**, **local transcription with Parakeet and optional diarization**, and a **batch Internet Archive importer**, plus improvements to **local OCR** and **model options**.

It adds no migrations: devices on 1.7.x can update in any order.

EntropIA Pro is also offered on the Microsoft Store as an MSIX (pending Partner Center publication).

### Help assistant (Ayuda)

- **Floating bubble and Ayuda view.** The chat opens from a draggable bubble at the bottom-right and from the Ayuda view, with a welcome page when no key is set.
- **Answers the basics with no key.** First steps and FAQs are answered locally from the manual, without OpenRouter.
- **Searches the manual and your documents.** Hash-rebuilt FTS5 manual index plus a screen map generated from the UI; searching your own documents is opt-in.
- **Acts with gated tools.** Agent mode with controlled MCP tools, live steps with a Stop button, a max-rounds setting, and approval dialogs that name the target.
- **Leak-free diagnostics.** The opt-in report scrubs paths, tokens and URLs; tool results hide file paths.
- **History and settings.** Local conversation with starter questions, up to 12 history turns, and a settings tab (model, context, local answers).

### Local transcription (STT, Pro only)

- **Parakeet TDT 0.6B v3 as the default local engine** (Whisper stays as fallback), with per-language selection and the same path for dictation.
- **Optional Sortformer diarization.** Detect-speakers checkbox (off by default); labels Speaker A/B/… and warns past 4 voices.
- **Rename speakers** from the transcript panel, for Parakeet and AssemblyAI alike.

### Local OCR (Pro only)

- **Persistent GLM-OCR engine** on llama-server, with no-restart engine switching, full-resolution PDFs and model downloads.
- **Local correction on Qwen3-VL-4B** over the shared llama-server, chunked so long pages are no longer corrupted (1.05 repetition penalty, folded stub tails).
- **Layout-assisted Light OCR** with reading-order assembly.

### Models and generation

- **OpenAI-compatible base URL (vLLM/Ollama door).** Pointing remote generation at your own server replaces the OpenRouter chain, needs no key, and fails loudly on a bad URL.
- **models_setup MCP tool** for local-model installs, used by the help agent.
- Fitted contexts, truncation-free summaries, and more tolerant intent detection.

### Internet Archive importer

- **Wide master-detail dialog** with page size, file-kind filter, and selection of whole records, pages, or all results (bounded).
- **Imports original JPEG, PNG and TIFF** (BigTIFF accepted) while keeping PDF names and completeness rules.
- **Queue plus batch import** with faster preparation, honest progress, and stable ordering.

### 3D relations and details

- **The Relaciones 3D graph saves as GEXF** for Gephi and no longer vanishes across tab switches.
- GitHub link centred in the status bar.
- Updated manual (screen map, FAQs, help).

---

## Part 2 — Release checklist (local → publication)

> Local-only so far: nothing is pushed, tagged or published from here. Push / PR / merge / tag `v1.8.0` / publish the draft are manual decisions of the user.

- [x] Version bump done — `pnpm version:bump 1.8.0` committed on `release/1.8.0` (`9f30d122`); `pnpm version:check` passes.
- [ ] Publish the runtime bootstrap for **app_version 1.8.0** for **windows-x86_64** and **linux-x86_64** (two dispatches of `publish-runtime-bootstrap.yml`: `platform` + `app_version: 1.8.0` + `pack_version` + `runtime_pack_run_id`, onto tag `runtime-bootstrap`) — **before tagging**. `release.yml` gates each Pro leg with `node apps/desktop/scripts/check-runtime-bootstrap.mjs --platform <windows-x86_64|linux-x86_64>` (exact `app_version` + `platform` match in the signed manifest; the leg fails without it).
- [x] Read the Partner Center identity of EntropIA Pro (Product management > Product identity): `CONICET.EntropIAPro`, `CN=89DF40E5-581A-4120-9A24-F701205485D6`, `HLab` → package family `CONICET.EntropIAPro_b16na7gwepwme`. They stay mandatory parameters.
- [ ] Local Pro MSI build (Pro release build with `--features local-ml`, `--bundles msi`, `--config src-tauri/tauri.connector.pro.conf.json` and the three `ENTROPIA_RUNTIME_BOOTSTRAP_*` values from `release.yml`), then `apps/desktop/src-tauri/scripts/build-pro-store-msix.ps1 -MsiPath <Pro MSI> -IdentityName CONICET.EntropIAPro -IdentityPublisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" -PublisherDisplayName HLab -OutDir .tmp/pro-msix` (`-StoreVersion 1.8.0.0` by default; needs `makeappx.exe` from the Windows SDK). Output is unsigned — the Store signs it.
- [ ] Optional: WACK (Windows App Certification Kit) on the unsigned MSIX.
- [ ] Lite MSIX: comes from the Release workflow `build-lite` artifact (`EntropIALite-msix`) — not built locally.
- [ ] User's manual calls: push the branch, open/merge the PR, push tag `v1.8.0` (builds Pro `.deb`/installers + Lite MSIX + draft release), attach backfills if needed, publish the draft.
