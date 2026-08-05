# Contrato de mantenimiento del runtime-pack

**English:** [MAINTENANCE.en.md](./MAINTENANCE.en.md)

## Qué cubre este repo

- Manifiestos versionados por plataforma.
- Estructura bundleable para `windows-x86_64` y `linux-x86_64`.
- Scripts de assembly y smoke (`scripts/build_runtime_pack.py`, `scripts/runtime-pack-smoke.py`).
- Fixtures chicos para validar wiring sin subir payloads pesados al repo.

## Qué debe contener el runtime pack real

El fixture del repo solo valida wiring; el runtime real lo arma por fuera **Build Runtime Pack** con un payload externo y se hostea vía **Publish Runtime Bootstrap**. Un pack de release DEBE incluir:

1. Python relocatable redistribuible por plataforma.
2. `uv` auditado si cambia respecto del fixture.
3. Wheelhouse offline real para OCR/transcripción.
4. Caches/modelos presembrados (HF y PaddleX) requeridos por los flujos core. Embeddings: OpenRouter `baai/bge-m3` o BGE-M3 local vía ONNX; NER local (Pro): spaCy `es_core_news_md` con fallback Gemma/OpenRouter. No existe `scripts/embed.py`; spaCy sí forma parte del runtime managed.
5. Shared libraries Linux auditadas (`libpdfium.so`, `libonnxruntime.so`, y cualquier dependencia adicional que resulte obligatoria).

## Contrato de payload externo

- El script `scripts/build_runtime_pack.py` acepta `--payload-root`.
- Ese directorio puede venir como layout directo (`python/`, `uv/`, `wheelhouse/`, `caches/`, `resources/lib/`) o como `<payload-root>/<platform>/...`.
- Si existe `manifest.overrides.json`, el script aplica esos overrides al manifest final y **recalcula** listados/checksums/tamaños a partir de los archivos realmente ensamblados.
- Pipeline de release vigente: **Build Runtime Pack** (manual) arma el pack desde un payload externo y sube el artifact `runtime-archive`; **Publish Runtime Bootstrap** lo parte bajo 2 GiB/asset, sube las partes al tag `runtime-bootstrap` y publica un `manifest.json` firmado ed25519; **Release** (tag `v*`) construye el instalador liviano con la URL del manifiesto y la clave pública horneadas y embebe el fixture tal cual — la app detecta “runtime no listo” y bootstrapea desde la fuente firmada. `build.rs` falla cerrado si un build de release embebe el fixture sin fuente horneada.
- La inyección de payloads pesados ya no ocurre en Release; el armado con payload externo (más abajo) es cómo se produce el artifact del runtime pack. El armado y el smoke corren dentro de Build Runtime Pack, nunca en Release, y jamás reemplazan el fixture del repo.

### Layouts aceptados para `--payload-root`

Layout directo:

```text
runtime-payloads/
├── manifest.overrides.json
├── python/
├── uv/
├── wheelhouse/
├── caches/
└── resources/lib/
```

Layout por plataforma:

```text
runtime-payloads/
├── windows-x86_64/
│   ├── manifest.overrides.json
│   ├── python/
│   ├── uv/
│   ├── wheelhouse/
│   ├── caches/
│   └── resources/lib/
└── linux-x86_64/
    ├── manifest.overrides.json
    ├── python/
    ├── uv/
    ├── wheelhouse/
    ├── caches/
    └── resources/lib/
```

### Handoff real por plataforma

| Plataforma | `python_relpath` esperado | `uv_relpath` esperado | Native assets mínimos | Artefactos externos mínimos |
| ---------- | ------------------------- | --------------------- | --------------------- | --------------------------- |
| `windows-x86_64` | `python/python.exe` | `uv/uv.exe` | `resources/lib/pdfium.dll`, `resources/lib/onnxruntime.dll` | `relocatable-python-windows-x86_64`, `offline-wheelhouse-core`, `seeded-model-caches` |
| `linux-x86_64` | `python/bin/python3` | `uv/bin/uv` | `resources/lib/libpdfium.so`, `resources/lib/libonnxruntime.so` | `relocatable-python-linux-x86_64`, `offline-wheelhouse-core`, `seeded-model-caches`, `linux-native-libs` |

### Output verificable del armado

- Cada corrida de `build_runtime_pack.py` deja `target/runtime-pack/<platform>/assembly-summary.json` con el `payload_root` resuelto, el perfil final y el listado de archivos ensamblados.
- `runtime-pack-smoke.py` acepta como `--root` tanto el directorio padre (`target/runtime-pack/`) como el directorio puntual de plataforma (`target/runtime-pack/<platform>`).
- La validación útil para handoff real es: **armar con payload externo → revisar `assembly-summary.json` → correr smoke sobre ese output**.

Diagnóstico rápido de readiness sin falsificar payloads:

```bash
python3 apps/desktop/src-tauri/scripts/build_runtime_pack.py --platform windows-x86_64 --output-dir apps/desktop/src-tauri/target/runtime-pack --require-release-payload
```

Si no se pasó `--payload-root`, este comando debe fallar con `--require-release-payload requires --payload-root`. Ese fallo es correcto: confirma que el release real todavía necesita un artifact externo y evita publicar fixtures como runtime self-contained.

Ejemplos de validación manual con payload real:

```bash
python3 apps/desktop/src-tauri/scripts/build_runtime_pack.py --platform windows-x86_64 --payload-root /abs/path/runtime-payloads --output-dir apps/desktop/src-tauri/target/runtime-pack
python3 apps/desktop/src-tauri/scripts/runtime-pack-smoke.py --platform windows-x86_64 --root apps/desktop/src-tauri/target/runtime-pack

python3 apps/desktop/src-tauri/scripts/build_runtime_pack.py --platform linux-x86_64 --payload-root /abs/path/runtime-payloads --output-dir apps/desktop/src-tauri/target/runtime-pack
python3 apps/desktop/src-tauri/scripts/runtime-pack-smoke.py --platform linux-x86_64 --root apps/desktop/src-tauri/target/runtime-pack
```

### Windows x86_64 desde el venv administrado local

Cuando una máquina Windows ya tiene `managed_venv` funcionando, se puede materializar un payload release reproducible desde ese entorno sin subir binarios pesados a git:

```powershell
python apps/desktop/src-tauri/scripts/materialize_windows_runtime_payload.py `
  --pack-version 2026.05.0 `
  --app-version 0.1.1 `
  --output-dir apps/desktop/src-tauri/target/runtime-payloads

python apps/desktop/src-tauri/scripts/build_runtime_pack.py `
  --platform windows-x86_64 `
  --payload-root apps/desktop/src-tauri/target/runtime-payloads `
  --output-dir apps/desktop/src-tauri/target/runtime-pack `
  --require-release-payload

python apps/desktop/src-tauri/scripts/runtime-pack-smoke.py `
  --platform windows-x86_64 `
  --root apps/desktop/src-tauri/target/runtime-pack `
  --release `
  --install-probe
```

Ese output queda bajo `target/`: es artefacto de release, no fuente commiteable.

### Criterio de cierre Windows x86_64

Windows se considera cerrado cuando se cumplen estas condiciones:

1. `materialize_windows_runtime_payload.py` genera `target/runtime-payloads/windows-x86_64` desde un `managed_venv` funcional.
2. `build_runtime_pack.py --require-release-payload` genera `target/runtime-pack/windows-x86_64` con `payload_profile=release`, `release_injection_required=false` y `external_artifacts_required=[]`.
3. `runtime-pack-smoke.py --release --install-probe` pasa en Windows.
4. El pack release no contiene ningún `CACHE_NOT_SEEDED.txt`; si aparece ese marcador, el smoke release debe fallar.
5. En dev, `ENTROPIA_RUNTIME_PACK_ROOT` puede apuntar a `target/runtime-pack` para validar la app sin copiar el payload pesado a `resources/`.

### Política OCRH / PaddleOCR-VL en CPU

En Windows sin GPU NVIDIA, PaddleOCR-VL puede usar CPU. El timeout de 900s es aceptado: no es señal de runtime roto por sí mismo. Si vence, OCRH debe fallar de forma controlada y caer a OCR plano; no hay que bajar este timeout salvo nueva decisión de producto.

Para probarlo en dev sin copiar 3GB a `resources/`, arrancá Tauri con:

```powershell
$env:ENTROPIA_RUNTIME_PACK_ROOT = "<repo>\apps\desktop\src-tauri\target\runtime-pack"
pnpm --filter @entropia-pro/desktop tauri dev
```

El override acepta tanto el directorio padre (`target/runtime-pack`) como el pack directo (`target/runtime-pack/windows-x86_64`).

Ejemplo mínimo de `manifest.overrides.json` para una inyección completa:

```json
{
  "payload_profile": "release",
  "release_injection_required": false,
  "external_artifacts_required": []
}
```

## Regla de verdad

Si `payload_profile != release` o `release_injection_required = true`, el runtime NO debe presentarse como listo para flujo offline core.
Además, un pack `release` no puede seguir declarando `external_artifacts_required`.

## Ownership sugerido

- Producto/app: define qué capacidades entran en “core offline”.
- Release engineering: inyecta artefactos, recalcula checksums y publica instaladores.
- Maintainers de OCR/NLP: validan licencias, tamaño y compatibilidad de los modelos/caches incluidos.
