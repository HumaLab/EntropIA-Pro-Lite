# Release 1.9.1

## Part 1 — Release notes (draft for the GitHub release body)

## Español

Esta versión corrige los fallos de OCRH que llegaron de equipos Windows, lleva el reconocimiento de PaddleOCR-VL a cualquier GPU que soporte llama.cpp y agrega un perfil rápido opcional para OCRL.

**No agrega migraciones.** Un equipo con 1.9.0 sigue sincronizando con uno que tenga 1.9.1.

### Pro

- **OCRH sin «Failed to parse input».** llama.cpp pasa de b8783 a b11429. La versión anterior rechazaba una respuesta del modelo que terminaba en un carácter UTF-8 incompleto, y la página fallaba. En Windows, las placas AMD Radeon corren el servidor local con Vulkan.
- **PaddleOCR-VL en cualquier GPU.** OCRH local puede leer con PaddleOCR-VL en el servidor llama-server de la app, no solo con NVIDIA. El análisis de diseño de página sigue en el procesador. Modelos locales tiene una tarjeta para descargar los modelos (unos 1,8 GB, con tamaño y SHA-256 verificados) y borrarlos. GLM-OCR sigue teniendo prioridad en el modo automático.
- **Perfil rápido para OCRL.** En Motores se puede elegir PP-OCRv6 Tiny en lugar de PP-OCRv5 Latin, que sigue siendo el predeterminado. En dos páginas de libro en español fue unas 2,8 veces más rápido, con alrededor de un punto menos de precisión por palabra. Los archivos (unos 3 MB) se descargan desde Modelos locales; sin ellos, OCRL usa v5.
- **Descargas que se retoman.** Si se corta la conexión, la descarga de un modelo se reintenta y sigue desde el byte al que había llegado, en lugar de empezar de cero. «Preparar EntropIA Pro» ya no arranca las seis descargas grandes a la vez.
- **Espacio en disco.** El reranker, llama-server y Parakeet revisan el espacio libre antes de descargar, en lugar de fallar a mitad de camino.
- **Instalación desde la Microsoft Store.** El runtime de Python queda en una ruta corta (`%USERPROFILE%\.entropia\runtime`), así los módulos más profundos de Paddle no superan el límite de 260 caracteres de Windows. Una carpeta de usuario con espacios en el nombre ya no rompe la instalación de PaddlePaddle.
- **OCRH en Windows.** Un llama-server que quedó abierto de una sesión anterior se cierra antes de arrancar otro, así no ocupa la memoria de la placa. PaddleOCR-VL espera las importaciones lentas (hasta 120 s) en lugar de pasar en silencio al OCR liviano durante toda la sesión.
- **Mensajes de diagnóstico.** Cuando OCRH pasa al OCR liviano, el registro nombra el motor que falló (GLM-OCR, PaddleOCR-VL o PaddleOCR-VL en llama-server). Si falla la instalación del reranker local, el registro guarda la causa.

### Pro y Lite

- **Internet Archive.** La pestaña Colección filtra por texto y por rango de años. Se corrigieron los filtros de año en Buscar.
- **OpenRouter.** «Probar conexión» verifica la clave. Antes daba verde con cualquier clave.
- **Windows sin WebView2.** Si falta el WebView2 Runtime (puede pasar con la versión de la Microsoft Store), la app muestra un aviso que explica qué instalar, en lugar de cerrarse sin mensaje.

## English

This release fixes the OCRH failures reported from Windows machines, brings PaddleOCR-VL recognition to any GPU llama.cpp supports, and adds an optional fast profile for OCRL.

**No migrations.** A machine on 1.9.0 keeps syncing with one on 1.9.1.

### Pro

- **OCRH without "Failed to parse input".** llama.cpp moves from b8783 to b11429. The old version rejected a model answer that ended in an incomplete UTF-8 character, and the page failed. On Windows, AMD Radeon cards run the local server on Vulkan.
- **PaddleOCR-VL on any GPU.** Local OCRH can read with PaddleOCR-VL on the app's llama-server, not only on NVIDIA. Page layout analysis stays on the processor. Local models has a card to download the models (about 1.8 GB, size and SHA-256 checked) and delete them. GLM-OCR keeps priority in automatic mode.
- **Fast OCRL profile.** Engines can pick PP-OCRv6 Tiny instead of PP-OCRv5 Latin, which stays the default. On two Spanish book pages it was about 2.8 times faster, with about one point less word accuracy. The files (about 3 MB) download from Local models; without them, OCRL uses v5.
- **Downloads that resume.** When the connection drops, a model download retries and continues from the byte it reached instead of starting over. "Preparar EntropIA Pro" no longer starts all six large downloads at once.
- **Disk space.** The reranker, llama-server and Parakeet check free space before downloading instead of failing half-way.
- **Microsoft Store install.** The Python runtime lives on a short path (`%USERPROFILE%\.entropia\runtime`), so Paddle's deepest modules stay under Windows' 260-character limit. A user folder with spaces in its name no longer breaks the PaddlePaddle install.
- **OCRH on Windows.** A llama-server left over from an earlier session is closed before another starts, so it does not hold the card's memory. PaddleOCR-VL waits for slow imports (up to 120 s) instead of silently falling back to the light OCR for the whole session.
- **Diagnostic messages.** When OCRH falls back to the light OCR, the log names the engine that failed (GLM-OCR, PaddleOCR-VL or PaddleOCR-VL on llama-server). When the local reranker install fails, the log keeps the cause.

### Pro and Lite

- **Internet Archive.** The Collection tab filters by text and year range. Year filters in Search are fixed.
- **OpenRouter.** "Probar conexión" checks the key. It used to show green with any key.
- **Windows without WebView2.** When the WebView2 Runtime is missing (possible with the Microsoft Store version), the app shows a notice that says what to install instead of closing without a message.

---

## Part 2 — Release checklist

- [x] Version bump — `pnpm version:bump 1.9.1`; `pnpm version:check` passes. `Cargo.lock` carries only the version line (the local entropia-agent patch stays out).
- [x] `cargo audit`: exit 0, no vulnerabilities (9 allowed warnings: unmaintained, unsound or yanked crates).
- [x] No migration: the last one is still `0066_transcription_versions.sql` (1.8.7).
- [x] Runtime packs unchanged since v1.9.0: nothing under `resources/runtime-pack` or the pack build scripts moved, and `deps/` asks for no new package version (only the runtime root path changed, #443). The same four packs are reused.
- [ ] Runtime bootstrap for **app_version 1.9.1**, all four entries in production:
  - CPU `2026.11.10` Windows (build run 44877086448) and Linux (44877088996)
  - CUDA `2026.11.24-cuda129` Windows (45526407341) and Linux (45469847198)

  `check-runtime-bootstrap.mjs --app-version 1.9.1` passes on both platforms.
- [x] Pre-release check on a clean Windows Server 2022 VM (no WebView2, LongPathsEnabled=0): WebView2 notice, runtime 2026.11.10 on the short path (longest path 222 characters), paddle 3.2.2 imports, layout_v3 ran. llama.cpp b11429 checked on an RX 560 (`odd/tasks/llama-cpp-upgrade.md` L8).
- [ ] PR to main, CI green, merge.
- [ ] Push tag `v1.9.1` on main (Pro installers + Lite MSIX + draft release).
- [ ] Release draft: title «EntropIA Pro & Lite v1.9.1», paste Part 1 as the body, publish.
- [ ] Build the Pro Store MSIX 1.9.1.0 (`build-pro-store-msix.ps1`, identity `CONICET.EntropIAPro`) from the release MSI. Lite MSIX 1.9.1.0 from the release run.
- [ ] Upload Pro and Lite MSIX 1.9.1.0 to Partner Center with their release notes.
