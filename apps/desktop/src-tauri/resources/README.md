# Recursos bundleados de Tauri

**English:** [README.en.md](./README.en.md)

Este directorio está reservado para recursos Tauri bundleados.

## Procedencia de los binarios

Cada binario commiteado está pinneado (sha256, tamaño y origen) en `apps/desktop/src-tauri/binary-provenance.json`.

- `pnpm binaries:check` valida el manifiesto y que cada binario trackeado esté listado con su hash correcto.
- `pnpm binaries:verify-upstream` descarga los binarios upstream y comprueba que coincidan byte a byte con su release pinneado.

CI corre ambos comandos cada semana y en cada PR que toque estos paths.

`lib/pdfium.dll` (bblanchon/pdfium-binaries `chromium/7543`) y los `uv.exe` (Astral `0.6.14`) están verificados byte-idénticos a sus releases. Por ahora siguen en git: `tauri.windows.conf.json` se fusiona en todo build de Windows (incluido `cargo test` en CI), así que moverlos a una descarga de build-time exige un paso de fetch en cada job de Windows y en dev local, más validación en Windows. Borrarlos no achicaría el repositorio sin una reescritura de historia acordada con el equipo.

Los modelos `.mnn` y el MSIX base están vendoreados sin fuente pública reproducible: sus hashes están pinneados y su origen debería documentarse la próxima vez que se regeneren. Los nombres indican modelos PaddleOCR PP-OCRv5 / PP-LCNet convertidos a MNN; no se registró la versión del modelo fuente ni la herramienta/versión de conversión.

## Librerías nativas

- `lib/pdfium.dll` — librería nativa Pdfium para renderizado PDF (Windows x86_64).
  Proveniente de bblanchon/pdfium-binaries `chromium/7543`; hash y origen pinneados en `binary-provenance.json`.
  La DLL se resuelve en runtime con una búsqueda de 3 niveles (bundleado → dev → librería del sistema).
  Ver `resources/lib/.gitkeep` para más detalles.
- `lib/linux-x86_64/` — **eliminado**. Sus antiguos _fixtures_ de texto (49/54 B) se movieron a `tests/fixtures/resource-audit/` como artefactos de auditoría. Los bundles Linux ya no los incluyen: Pdfium para Linux se descarga con `apps/desktop/scripts/fetch-pdfium.sh linux-x64` a `resources/pdfium/libpdfium.so` (+ `LICENSE`) y lo bundlean los overlays explícitos de release `tauri.pro.linux.conf.json` y `tauri.lite.linux.conf.json` (no el `tauri.linux.conf.json` que se fusiona solo, así `cargo build` en Linux no necesita la descarga); el resolutor de `src/ocr/pdf.rs` descarta candidatos que no sean una librería nativa (magic ELF/PE/Mach-O/fat y ≥ 1 MiB).
- Los runtime payloads de release también pueden inyectar ONNX Runtime bajo `runtime-pack/<platform>/resources/lib/` cuando consumidores ONNX validados lo requieran.

## Herramientas bundleadas

- `tools/uv/windows-x86_64/uv.exe` — `uv` 0.6.14 bundleado para Windows x64.
- `tools/uv/windows-aarch64/uv.exe` — `uv` 0.6.14 bundleado para Windows ARM64.
  La resolución de runtime prefiere recursos Tauri bundleados, luego recursos dev, luego la copia administrada legacy en app-data y finalmente `PATH` del sistema.

## Fixtures de runtime-pack

- `runtime-pack/windows-x86_64/` y `runtime-pack/linux-x86_64/` existen en repo como **fixture packs mínimos viables**.
- Cada pack incluye `manifest.json`, launchers placeholder de Python/uv, scripts administrados, placeholders de cache, notas de wheelhouse y rutas espejo para librerías nativas.
- `payload_profile: fixture` significa que estos packs son estructuralmente reales y bundleables, pero NO son los payloads pesados finales de release.
- `release_injection_required: true` en el fixture indica que el pack en repo NO es el runtime pesado final: la app lo obtiene en runtime, no del instalador.
- El runtime administrado y su bootstrap pertenecen exclusivamente a **EntropIA Pro** (`local-ml`). **EntropIA Lite** usa proveedores remotos y no descarga ni consume este runtime local.
- **Modelo de distribución: instalador liviano.** El instalador ship solo el fixture; el runtime de IA (~2.2GB) se hostea aparte (tag `runtime-bootstrap`, partido bajo 2 GiB/asset + firmado ed25519) y la app lo descarga y verifica (firma + sha256) al iniciar, en partes reanudables y con avance, pausa y reintento en el aviso del runtime; con el runtime listo instala sola las dependencias de IA desde el wheelhouse offline. No se inyecta el runtime en el instalador, porque NSIS/WiX no soportan bundles >2GB.
- **En repo ahora**: layout de runtime-pack, contrato de manifest, bundle globs del fixture, wiring de assembly, smoke checks y límites explícitos de ownership offline.

### Flujo de release (lean)

1. **Build Runtime Pack** (una vez por plataforma: `windows-x86_64`, `linux-x86_64`) → arma el runtime-pack fresco y sube el artifact `runtime-archive`.
2. **Publish Runtime Bootstrap** con ese `runtime_pack_run_id` → parte el archivo bajo 2 GiB/asset, sube las partes al tag `runtime-bootstrap` y publica un `manifest.json` firmado.
3. Pushear tag `v*` → **Release** construye los instaladores Pro livianos: NSIS + MSI en Windows y DEB en Linux, con la URL del manifiesto + la clave pública horneadas en el binario. Antes de compilar, el workflow exige que el manifiesto firmado tenga la entrada exacta de versión y plataforma (`apps/desktop/scripts/check-runtime-bootstrap.mjs`), y `build.rs` falla cerrado si un build de release embebe el fixture sin fuente horneada, así que nunca se publica un instalador Pro que no pueda descargar el runtime.

Ver `scripts/prepare_runtime_payload.py`, `scripts/materialize_windows_runtime_payload.py`, `scripts/build_runtime_pack.py`, `scripts/runtime-pack-smoke.py` y cada `ASSEMBLY_NOTES.md` de plataforma para el contrato de handoff de release.

## Modelos OCR

El fallback OCR nativo usa los assets PaddleOCR/MNN bundleados en `models/ocr/`:

- `PP-OCRv5_mobile_det.mnn`
- `latin_PP-OCRv5_mobile_rec_infer.mnn`
- `PP-LCNet_x1_0_doc_ori.mnn`
- `ppocr_keys_latin.txt`

OCR de alta calidad con PaddleOCR-VL corre mediante el path de runtime/script Python administrado y no requiere archivos de modelo separados commiteados en este directorio.
