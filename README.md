# EntropIA — Pro &amp; Lite (monorepo unificado)

**English:** [README.en.md](./README.en.md)

Un solo código fuente que produce **dos variantes** de la app de escritorio para investigación con corpus documentales: **EntropIA Pro** (IA local + remota) y **EntropIA Lite** (100% remota, vía APIs). Ambas se construyen del mismo árbol; la variante se elige en tiempo de compilación.

EntropIA organiza colecciones, procesa imágenes/PDFs/audio, y enriquece resultados con OCR, transcripción, búsqueda, embeddings, entidades y triples semánticos.

## Las dos variantes

|                         | **EntropIA Pro**                                        | **EntropIA Lite**                                     |
| ----------------------- | ------------------------------------------------------- | ----------------------------------------------------- |
| OCR                     | PaddleOCR local (Light) + PaddleOCR-VL o GLM-OCR (High) | GLM-OCR remoto (Light y High usan el mismo proveedor) |
| Transcripción           | faster-whisper local + AssemblyAI                       | AssemblyAI                                            |
| LLM / NER / RAG         | Gemma 4 local + OpenRouter; spaCy local para NER        | OpenRouter (Gemma 4 por defecto)                      |
| Embeddings              | BGE-M3 local (ONNX) + OpenRouter (`baai/bge-m3`)        | OpenRouter (`baai/bge-m3`)                            |
| Runtime ML nativo       | sí (se descarga al 1er uso)                             | no                                                    |
| Instalador              | Windows: NSIS + MSI (GitHub) · Linux: DEB (GitHub)      | Windows: NSIS + MSI (GitHub) · MSIX (Store)           |
| Identificador Tauri     | `com.entropia.pro.desktop`                              | `com.entropia.lite`                                   |
| Identidad MSIX de Store | —                                                       | `CONICET.EntropIALite`                                |
| Se construye con        | `--features local-ml` + `VITE_LOCAL_ML=1`               | features default lean + `VITE_LOCAL_ML=0`             |

**Pro** corre IA en la máquina por defecto (offline-first) y permite seleccionar proveedores remotos por configuración; los modos `auto` aplican fallback donde está implementado. **Lite** es 100% remota (OpenRouter / AssemblyAI / GLM-OCR): sin modelos ni runtime nativo, instalador chico, distribución por Microsoft Store.

## Descarga

- **EntropIA Pro** — Windows x64: `.exe` (NSIS) + `.msi`; Linux x64: `.deb`. Disponibles en [Releases del repo](https://github.com/HumaLab/EntropIA-Pro-Lite/releases).
- **EntropIA Lite** (Windows x64) — Microsoft Store: <https://apps.microsoft.com/detail/9N328K9L95JD>, o `.exe`/`.msi` desde [Releases del repo](https://github.com/HumaLab/EntropIA-Pro-Lite/releases).

## Capacidades

Ambas variantes cubren los mismos flujos principales de investigación; cambia el motor (local vs remoto, ver la tabla de arriba). No tienen literalmente el mismo conjunto de runtime y UI: Pro agrega motores locales y su gestión de dependencias/modelos.

- Organización de corpus en colecciones, ítems y assets locales (SQLite).
- Ingesta de imágenes, PDFs y audio.
- OCR Light + OCR High con persistencia de layout (bloques, regiones, páginas, bounding boxes).
- Transcripción de audio.
- Corrección, resumen y extracción semántica asistida por LLM.
- Entidades, triples, NER, FTS y embeddings asset-level (RAG).
- Notas, anotaciones y edición manual de resultados.
- Sincronización cross-device (ids deterministas para convergencia sin duplicados).

## Desarrollo

### Requisitos

- Node.js 22+, pnpm 9
- Rust estable / toolchain MSVC en Windows

### Instalación

```bash
git clone git@github.com:HumaLab/EntropIA-Pro-Lite.git
cd EntropIA-Pro-Lite
pnpm install --frozen-lockfile
```

### Correr y buildear cada variante

Todo se corre desde **`apps/desktop/`**. Si estás en la raíz del repo, primero hacé `cd apps/desktop`; si no, `pnpm exec tauri` no encuentra el CLI de Tauri porque está instalado en el workspace desktop. La variante se elige con tres cosas: el feature de Cargo (`local-ml` explícito para Pro; default lean para Lite), el flag de frontend `VITE_LOCAL_ML`, y (en Lite) el config de Tauri `tauri.lite.conf.json`.

**EntropIA Pro** (compila MNN desde fuente la 1ra vez → ~30 min):

```powershell
cd apps/desktop
$env:VITE_LOCAL_ML='1'
pnpm exec tauri dev   --features local-ml      # dev con hot-reload
pnpm exec tauri build --features local-ml --bundles nsis,msi  # instaladores NSIS + MSI
```

**EntropIA Lite** (lean, sin MNN → arranca rápido):

```powershell
cd apps/desktop
$env:VITE_LOCAL_ML='0'
pnpm exec tauri dev   --config src-tauri/tauri.lite.conf.json
pnpm exec tauri build --config src-tauri/tauri.lite.conf.json --bundles nsis,msi
```

> - Usá **`pnpm exec tauri`** (no `pnpm tauri … -- …`): pnpm se come el primer `--` y rompe el pasaje de args a Cargo.
> - Si querés correrlo desde la **raíz** sin hacer `cd`, usá `pnpm --filter @entropia-pro/desktop exec tauri ...`.
> - Lite es el default lean de Cargo. No pases `--features local-ml` cuando uses `tauri.lite.conf.json`.
> - En PowerShell `$env:VITE_LOCAL_ML` **persiste en la sesión** → seteálo en cada cambio de variante (o abrí terminal nueva). En bash va adelante: `VITE_LOCAL_ML=0 pnpm exec tauri …`.
> - Lite usa `identifier com.entropia.lite` → **datos de app separados** de Pro (podés correr ambas sin pisarte).
> - `tauri build` de Lite genera el **`.exe` (NSIS) + `.msi`**; el **MSIX** final de Store sale del repack (ver _Release e instaladores_).
> - Con el `runtime-pack` fixture commiteado, un `tauri build` de release para Windows/Linux exige definir `ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL`, `ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID` y `ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64` con los valores de [`.github/workflows/release.yml`](./.github/workflows/release.yml). El workflow los define para Pro y Lite; `tauri dev` no los necesita. El guard actual de `build.rs` se ejecuta para ambas variantes aunque Lite no use IA local.

### Validar

```bash
pnpm lint                                                       # todo el workspace
pnpm typecheck                                                  # workspace; frontend desktop Pro
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck   # frontend desktop Lite
pnpm test                                                       # workspace; tests desktop Pro
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test        # tests desktop Lite
cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml --features local-ml  # Pro (Rust)
cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml                      # Lite (Rust)
```

## Cómo funciona el flag de variante

La unificación es un **strangler** sobre el código de Pro: toda la inferencia local vive detrás del feature de Cargo `local-ml` (con un sub-feature `paddle-ocr` para MNN/PaddleOCR), espejado por el flag de frontend `VITE_LOCAL_ML`.

- **`cargo build --features local-ml`** = **Pro** (motores locales + remotos).
- **`cargo build` (default)** = lean → **Lite** (solo remoto). Dropea `ort`/onnxruntime, `llama-cpp-2`, MNN/`ocr-rs`, `tokenizers` y la descarga del runtime firmado.
- El **frontend** lee `VITE_LOCAL_ML`: en Lite esconde DependenciasTab, los banners de deps y la UI de modelos locales, y la marca pasa a "EntropIA Lite".
- La **lista de comandos Tauri es idéntica** en ambas variantes; solo ramifican los cuerpos (el brazo Lite devuelve healthy/no-op, como hacía EntropIA Lite).

En cada push/PR, CI ejecuta lint, typecheck y tests del workspace con el frontend Pro, además del typecheck y los tests desktop con `VITE_LOCAL_ML=0`; también construye el frontend Pro. Cuando cambian archivos Rust/Tauri relevantes, el contrato de features de Windows compila y enlaza Pro y Lite como gates bloqueantes.

## Release e instaladores

**Pro — instalador liviano + descarga al 1er uso.** El runtime de IA (~2.2GB) no entra en un instalador Windows (NSIS y WiX fallan por encima de ~2GB). El instalador incluye el fixture chico de `runtime-pack` y la app descarga el runtime real al primer uso desde una fuente remota firmada (ed25519), verificando firma + sha256 antes de confiar en él. Para Windows/Linux, `build.rs` falla cerrado si cualquier build de release embebe el fixture sin una fuente de bootstrap horneada; el workflow **Release** la define para Pro y Lite.

Flujo de release de Pro:

1. **Build Runtime Pack** → arma el runtime-pack fresco (artifact `runtime-archive`).
2. **Publish Runtime Bootstrap** con ese `runtime_pack_run_id` → parte el archivo bajo el límite de 2 GiB por asset, sube las partes al tag `runtime-bootstrap` y publica un `manifest.json` firmado.
3. Push del tag `v*` → el workflow **Release** construye NSIS + MSI en Windows y DEB en Linux, con la URL del manifiesto + la clave pública **horneadas** en el binario.

**Lite — instaladores en GitHub + MSIX para la Store.** El job `build-lite` del workflow **Release** construye la variante lean con `--bundles nsis,msi`; el job `attach-lite-installers` adjunta el `.exe` (NSIS) + `.msi` al release de GitHub (descargables igual que los de Pro). En paralelo, el `.msi` alimenta el **repack** de un MSIX base capturado (`apps/desktop/src-tauri/msix/`), reescribiendo la identidad a `CONICET.EntropIALite` + la versión; el `.msix` sin firmar (la Store lo firma) queda sólo como artifact de Actions para Partner Center, no como asset del release.

- Para probar **solo** el MSIX de Lite sin la build de Pro: dispatch manual del workflow **Release** con la opción `lite_only=true` (o `gh workflow run release.yml -f lite_only=true`).
- El MSIX base se re-captura (VM Hyper-V, manual) **solo** si cambia la forma del paquete (assets/capabilities); los releases de rutina solo cambian el exe + suben la versión.

## Documentación útil

- [SQLite](./SQLite.md) — esquema y guía de inspección de la base local.
- [Debugging de base de datos](./DATABASE_DEBUGGING.md) — consultas operativas para diagnosticar persistencia.
- [Firma de código](./CODE_SIGNING.md) — política de firma para releases.
- [Privacidad](./PRIVACY.md) — comportamiento de datos, runtimes y proveedores externos.
- [Avisos de terceros](./THIRD_PARTY_NOTICES.md) — dependencias, modelos y runtime payloads.

---

**IA local en Pro. APIs remotas en Lite.**
