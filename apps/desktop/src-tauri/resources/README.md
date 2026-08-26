# Recursos bundleados de Tauri

**English:** [README.en.md](./README.en.md)

Este directorio está reservado para recursos Tauri bundleados.

## Librerías nativas

- `lib/pdfium.dll` — librería nativa Pdfium para renderizado PDF (Windows x86_64).
  Descargar desde [pdfium-render releases](https://github.com/ajrcarey/pdfium-render/releases).
  La DLL se resuelve en runtime con una búsqueda de 3 niveles (bundleado → dev → librería del sistema).
  Ver `resources/lib/.gitkeep` para más detalles.
- `lib/linux-x86_64/` — placeholders de librerías nativas Linux y rutas de handoff documentadas para payloads de runtime de release.
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
- **Modelo de distribución: instalador liviano.** El instalador ship solo el fixture; el runtime de IA (~2.2GB) se hostea aparte (tag `runtime-bootstrap`, partido bajo 2 GiB/asset + firmado ed25519) y la app lo descarga y verifica (firma + sha256) al primer uso. No se inyecta el runtime en el instalador, porque NSIS/WiX no soportan bundles >2GB.
- **En repo ahora**: layout de runtime-pack, contrato de manifest, bundle globs del fixture, wiring de assembly, smoke checks y límites explícitos de ownership offline.

### Flujo de release (lean)

1. **Build Runtime Pack** → arma el runtime-pack fresco y sube el artifact `runtime-archive`.
2. **Publish Runtime Bootstrap** con ese `runtime_pack_run_id` → parte el archivo bajo 2 GiB/asset, sube las partes al tag `runtime-bootstrap` y publica un `manifest.json` firmado.
3. Pushear tag `v*` → **Release** construye los instaladores Pro livianos: NSIS + MSI en Windows y DEB en Linux, con la URL del manifiesto + la clave pública horneadas en el binario. `build.rs` falla cerrado si un build de release embebe el fixture sin fuente horneada, así que nunca se publica un instalador Pro que no pueda descargar el runtime.

Ver `scripts/prepare_runtime_payload.py`, `scripts/materialize_windows_runtime_payload.py`, `scripts/build_runtime_pack.py`, `scripts/runtime-pack-smoke.py` y cada `ASSEMBLY_NOTES.md` de plataforma para el contrato de handoff de release.

## Modelos OCR

El fallback OCR nativo usa los assets PaddleOCR/MNN bundleados en `models/ocr/`:

- `PP-OCRv5_mobile_det.mnn`
- `latin_PP-OCRv5_mobile_rec_infer.mnn`
- `PP-LCNet_x1_0_doc_ori.mnn`
- `ppocr_keys_latin.txt`

OCR de alta calidad con PaddleOCR-VL corre mediante el path de runtime/script Python administrado y no requiere archivos de modelo separados commiteados en este directorio.
