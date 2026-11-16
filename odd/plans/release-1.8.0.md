# Release 1.8.0

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión publica **EntropIA Pro** y **EntropIA Lite**. Agrega el **asistente de ayuda en la app (Ayuda)**, la **transcripción local con Parakeet y diarización opcional**, y un **importador de Internet Archive por lotes**, además de mejoras en el **OCR local** y las **opciones de modelos**, y **homogeneiza la interfaz** en toda la app.

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
- Manual unificado Pro/Lite con edición en inglés (mapa de pantallas, preguntas frecuentes y ayuda).

### Interfaz homogénea

- **Mismos diálogos, botones y barras de búsqueda en toda la app.** Visores y celdas en el Dialog unificado (con ConfirmDialog), citas y selectores en SelectTrigger, filtro lateral y búsqueda global en SearchBar, botones con variante enlace y casillas con tamaño compacto y variante tarjeta.
- **Ajustes y formularios unificados.** Pestañas de ajustes sobre RadioGroup/RadioCard, Switch, Disclosure y Textarea; destinos, dependencias y lote con RadioGroup, ProgressBar y StatusBadge; claves y rutas en monoespaciada, foco visible y estados coherentes.
- **Resaltados de PDF fiables.** Las selecciones de varios rangos guardan todas sus líneas y la conversión usa el viewport del momento de la selección, así un zoom o re-render intermedio ya no deforma los resaltados.

## English

This release publishes **EntropIA Pro** and **EntropIA Lite**. It adds the **in-app help assistant (Ayuda)**, **local transcription with Parakeet and optional diarization**, and a **batch Internet Archive importer**, plus improvements to **local OCR** and **model options**, and **homogenizes the interface** across the app.

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
- Unified Pro/Lite manual with an English edition (screen map, FAQs, help).

### Consistent interface

- **Same dialogs, buttons and search bars everywhere.** Viewers and cell dialogs on the unified Dialog (with ConfirmDialog), citation and other pickers on SelectTrigger, sidebar filter and global search on SearchBar, buttons with a link variant and checkboxes with a compact size and card variant.
- **Unified settings and forms.** Settings tabs on RadioGroup/RadioCard, Switch, Disclosure and Textarea; destinations, dependencies and batch on RadioGroup, ProgressBar and StatusBadge; keys and paths in monospace, visible focus and consistent states.
- **Reliable PDF highlights.** Multi-range selections save all their lines and conversion uses the selection-time viewport, so an intermediate zoom or re-render no longer distorts highlights.

---

## Part 2 — Release checklist (local → publication)

> Local-only so far: main is merged in locally (`c3ba72b9`); nothing is pushed, tagged or published from here. Push / PR / merge / Build Runtime Pack / bootstrap / tag `v1.8.0` / publish the draft are manual decisions of the user.

- [x] Version bump done — `pnpm version:bump 1.8.0` committed on `release/1.8.0` (`9f30d122`); `pnpm version:check` passes.
- [x] `origin/main` merged into `release/1.8.0` (`c3ba72b9`); Part 1 refreshed to cover v1.7.1..HEAD (no new migrations: horizon stays at 0065, already in v1.7.1).
- [x] Read the Partner Center identity of EntropIA Pro (Product management > Product identity): `CONICET.EntropIAPro`, `CN=89DF40E5-581A-4120-9A24-F701205485D6`, `HLab` → package family `CONICET.EntropIAPro_b16na7gwepwme`. They stay mandatory parameters.
- [ ] Push the branch, open the PR to main, wait for CI green, merge (T6).
- [ ] On main: Build Runtime Pack for 1.8.0 for **windows-x86_64** and **linux-x86_64** (new `pack_version` 2026.11.4) (T7).
- [ ] Publish the runtime bootstrap for **app_version 1.8.0** for **windows-x86_64** and **linux-x86_64** (two dispatches of `publish-runtime-bootstrap.yml`: `platform` + `app_version: 1.8.0` + `pack_version` + `runtime_pack_run_id`, onto tag `runtime-bootstrap`) — **after the merge to main, before tagging**. `release.yml` gates each Pro leg with `node apps/desktop/scripts/check-runtime-bootstrap.mjs --platform <windows-x86_64|linux-x86_64>` (exact `app_version` + `platform` match in the signed manifest; the leg fails without it).
- [ ] Push tag `v1.8.0` on main (builds Pro `.deb`/installers + Lite MSIX + draft release) (T8).
- [ ] Release workflow draft: attach backfills if needed, paste Part 1 as the release body, publish (T9).
- [ ] Rebuild the Pro MSI from the merged code (Pro release build with `--features local-ml`, `--bundles msi`, `--config src-tauri/tauri.connector.pro.conf.json` and the three `ENTROPIA_RUNTIME_BOOTSTRAP_*` values from `release.yml`), then rebuild the Store MSIX with `apps/desktop/src-tauri/scripts/build-pro-store-msix.ps1 -MsiPath <Pro MSI> -IdentityName CONICET.EntropIAPro -IdentityPublisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" -PublisherDisplayName HLab -OutDir .tmp/pro-msix` (`-StoreVersion 1.8.0.0` by default; needs `makeappx.exe` from the Windows SDK). A pre-merge MSIX already exists locally (`.tmp/pro-msix/EntropIAPro-Store-1.8.0.0.msix`, SHA256 `E297F21FF80A9993EC1B6CFA1FDE0050CAF9F12DC9FA34C3A09580AF711286A0`) but is stale — it predates `c3ba72b9`. Output is unsigned — the Store signs it.
- [ ] Optional: WACK (Windows App Certification Kit) on the unsigned MSIX.
- [ ] Lite MSIX: comes from the Release workflow `build-lite` artifact (`EntropIALite-msix`) — not built locally.
