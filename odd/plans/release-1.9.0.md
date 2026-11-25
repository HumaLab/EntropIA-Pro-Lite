# Release 1.9.0

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión lleva **PaddleOCR-VL a la GPU NVIDIA** en Windows y Linux, hace más rápido el modelo de texto local y reúne las correcciones de la auditoría posterior a 1.8.7.

**No agrega migraciones.** Un equipo con 1.8.8 sigue sincronizando con uno que tenga 1.9.0.

### Pro

- **PaddleOCR-VL en la GPU.** Con una placa NVIDIA de 8 GB o más y un driver reciente (576 o superior en Windows, 575 en Linux), la app descarga un runtime con soporte CUDA y lee las páginas en la GPU. Por ejemplo, una página densa de diario de 1965 tarda unos 30 segundos en una RTX 5090 y menos de 2 minutos en una A10G. En los demás equipos se descarga el runtime de siempre y PaddleOCR-VL sigue en el procesador, así que nadie baja lo que no puede usar. Si una página se traba en la GPU, la app la corta y la vuelve a intentar una vez.
- **Gemma en la GPU.** El chat, las respuestas directas y la ayuda con el modelo local corren en la GPU (NVIDIA, AMD, Intel o Apple) con el mismo servidor que ya usa GLM-OCR. Sin GPU, el modelo usa todos los núcleos físicos del procesador en lugar de cuatro.
- **Modelos de visión sin GPU.** Las tarjetas de GLM-OCR y Qwen ofrecen «Borrar modelos» y dicen cuánto espacio liberan cuando la placa no los puede correr.

### Pro y Lite

- **Leer todas las páginas.** La pestaña Texto de un documento con varias páginas tiene una fila «Todas las páginas»: manda a la cola cada página que todavía no tiene texto. Las que ya tienen texto, sobre todo las corregidas a mano, quedan como están.
- **Chat.** Muestra en qué etapa está la respuesta y cuánto lleva esperando. Avisa cuántos documentos esperan sus pasajes en lugar de responder sin ellos.
- **Escritura.** Los manuscritos borrados van a una papelera y se pueden restaurar. Una cita del corpus muestra en el editor de qué documento y página viene. Las notas al pie nombran la colección. Con «Incluir bibliografía», la exportación termina con «Fuentes citadas». En Corpus, cada resultado muestra dos líneas con la palabra marcada.
- **Búsqueda y análisis.** La búsqueda une las palabras cortadas con guion al final de línea; se desactiva en Configuración → Mantenimiento. Los resultados de un documento muestran el fragmento encontrado. El análisis de la colección ya no cuenta como palabras las marcas de formato de GLM-OCR. Las colocaciones no listan la palabra buscada. La nube de palabras deja afuera interrogativos con tilde, formas de voseo y muletillas orales.
- **Investigación.** Una colección borrada ya no queda elegida. El encabezado nombra las colecciones. Borrar una investigación ya no deja un cambio que nunca se sincroniza.
- **Lotes y mantenimiento.** El análisis de un lote dice cuántos documentos va a procesar, cuántos ya están y cuántos omite. Un borrador descartado no aparece en el historial. Mantenimiento lista las imágenes que conviene volver a procesar, con un botón para abrir cada una.
- **Transcripciones.** Restaurar una versión anterior ya no choca con un proceso que escribe al mismo tiempo. Se guardan hasta 10 versiones por audio. Si el micrófono graba silencio, el dictado lo dice en lugar de devolver un texto vacío.
- **Sincronización.** Una página recibida que falla a la mitad se deshace entera, en lugar de dejar la base bloqueada.
- **Ventanas.** En Windows, abrir la app por segunda vez trae al frente la ventana que ya estaba abierta, sin dibujar otra en negro.
- **Interfaz.** Todos los diálogos de borrado confirman con un botón «Eliminar». La Biblioteca ordena los títulos por su primera letra, sin tener en cuenta comillas ni signos iniciales. Inicio cuenta bien los embeddings y muestra qué le pasó a cada documento en «Actividad reciente». La pestaña Logs sigue la última línea. El panel lateral conserva la pestaña al cambiar de página. Exportar texto avisa dónde quedó el archivo. El botón de ayuda queda semitransparente para no tapar contenido.
- **Modelos remotos por defecto.** En una instalación nueva, la corrección de OCR usa `google/gemini-2.5-flash-lite` y los demás procesos usan `openai/gpt-6-luna`. Las instalaciones que ya guardaron sus modelos los conservan.
- **Seguridad.** Actualiza la biblioteca `cookie` a 0.18.3 (RUSTSEC-2026-0594).

## English

This release brings **PaddleOCR-VL to NVIDIA GPUs** on Windows and Linux, speeds up the local text model, and gathers the fixes from the audit that followed 1.8.7.

**It adds no migration.** A device on 1.8.8 still syncs with one on 1.9.0.

### Pro

- **PaddleOCR-VL on the GPU.** With an NVIDIA card of 8 GB or more and a recent driver (576 or newer on Windows, 575 on Linux), the app downloads a CUDA runtime and reads pages on the GPU. For example, a dense 1965 newspaper page takes about 30 seconds on an RTX 5090 and under 2 minutes on an A10G. Other machines get the usual runtime and PaddleOCR-VL keeps running on the processor, so nobody downloads what they cannot use. If a page stalls on the GPU, the app stops it and tries it once more.
- **Gemma on the GPU.** Chat, direct answers and help with the local model run on the GPU (NVIDIA, AMD, Intel or Apple) through the same server GLM-OCR already uses. Without a GPU, the model uses every physical processor core instead of four.
- **Vision models without a GPU.** The GLM-OCR and Qwen cards offer "Borrar modelos" and say how much space that frees when the card cannot run them.

### Pro and Lite

- **Read every page.** The Texto tab of a document with several pages has an "All pages" row: it queues every page that has no text yet. Pages that already have text, above all those corrected by hand, are left as they are.
- **Chat.** Shows which stage the answer is in and how long it has been waiting. Says how many documents are waiting for their passages instead of answering without them.
- **Writing.** Deleted manuscripts go to a trash and can be restored. A corpus quote shows in the editor which document and page it comes from. Footnotes name the collection. With "Include bibliography", the export ends with "Sources cited". In Corpus, each result shows two lines with the word marked.
- **Search and analysis.** Search joins words split by a hyphen at the end of a line; this can be turned off in Settings → Maintenance. A document's results show the passage found. Collection analysis no longer counts GLM-OCR layout markup as words. Collocates no longer list the word searched. The word cloud leaves out accented question words, voseo forms and oral filler words.
- **Investigation.** A deleted collection no longer stays selected. The header names the collections. Deleting an investigation no longer leaves a change that never syncs.
- **Batches and maintenance.** Analysing a batch says how many documents it will process, how many are already done and how many it skips. A discarded draft does not appear in the history. Maintenance lists the images worth processing again, with a button to open each one.
- **Transcriptions.** Restoring an earlier version no longer collides with a process writing at the same time. Up to 10 versions are kept per audio. When the microphone records silence, dictation says so instead of returning empty text.
- **Sync.** A received page that fails half-way is rolled back whole instead of leaving the database locked.
- **Windows.** On Windows, opening the app a second time brings the open window to the front instead of drawing a black one.
- **Interface.** Every delete dialog confirms with a "Delete" button. The Library sorts titles by their first letter, ignoring leading quotes and punctuation. Home counts embeddings correctly and shows what last happened to each document under "Recent activity". The Logs tab follows the newest line. The side panel keeps its tab when moving between pages. Exporting text says where the file went. The help button is translucent so it does not cover content.
- **Default remote models.** On a fresh install, OCR correction uses `google/gemini-2.5-flash-lite` and the other flows use `openai/gpt-6-luna`. Installs that already saved their models keep them.
- **Security.** Updates the `cookie` library to 0.18.3 (RUSTSEC-2026-0594).

---

## Part 2 — Release checklist

- [x] Version bump — `pnpm version:bump 1.9.0`; `pnpm version:check` passes. `Cargo.lock` carries only the version line (the local entropia-agent patch stays out).
- [x] No migration: the last one is still `0066_transcription_versions.sql` (1.8.7).
- [x] Runtime bootstrap for **app_version 1.9.0**, all four entries in production (see `odd/tasks/paddle-cuda-runtime-pack.md` L27, L31, L32):
  - CPU `2026.11.10` for Windows and Linux (same packs as 1.8.8)
  - CUDA `2026.11.24-cuda129` for Windows and Linux

  `check-runtime-bootstrap.mjs --app-version 1.9.0` passes on both platforms.
- [x] Windows CUDA checked end to end on an A10G: the installer picked the CUDA pack, installed it, and OCRH ran on the GPU (L30). Linux CUDA checked on an RTX 5090 (L24).
- [x] PR to main, CI green, merge (#441, c3aa217a).
- [x] Push tag `v1.9.0` on main (Pro installers + Lite MSIX + draft release). Release run 45644843021: the first Windows Pro attempt built but failed silently while creating the draft release; the Linux job then created it, and a rerun of the Windows job uploaded its installers. The draft holds the same 7 assets as 1.8.8.
- [x] Release draft: title «EntropIA Pro & Lite v1.9.0», paste Part 1 as the body, publish (Latest, 2026-11-25).
- [x] Build the Pro Store MSIX 1.9.0.0 (`build-pro-store-msix.ps1`, identity `CONICET.EntropIAPro`): `G:\EntropIA-Stack\pro-msix-1.9.0\EntropIAPro-Store-1.9.0.0.msix` (70,543,556 B). Lite MSIX 1.9.0.0 from the release run: `G:\EntropIA-Stack\lite-msix-1.9.0\EntropIALite-Store-HLab-1.9.0.0.msix`.
- [ ] Upload Pro and Lite MSIX 1.9.0.0 to Partner Center with their release notes; the Store number does not move with the app version on its own.
