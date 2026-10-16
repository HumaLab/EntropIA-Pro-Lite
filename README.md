# EntropIA — Pro &amp; Lite (monorepo unificado)

**English:** [README.en.md](./README.en.md)

Un solo código fuente que produce **dos variantes** de la app de escritorio para investigación con corpus documentales: **EntropIA Pro** (IA local + remota) y **EntropIA Lite** (100% remota, vía APIs). Ambas se construyen del mismo árbol; la variante se elige en tiempo de compilación. Pensada para investigadoras, investigadores y equipos que organizan, procesan, analizan y escriben a partir de colecciones de imágenes, PDFs y audio.

EntropIA organiza colecciones, procesa imágenes/PDFs/audio, y enriquece resultados con OCR, transcripción, búsqueda, embeddings, entidades y triples semánticos.

## Las dos variantes

|                         | **EntropIA Pro**                                        | **EntropIA Lite**                                                                                 |
| ----------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| OCR                     | PaddleOCR local (Light) + PaddleOCR-VL o GLM-OCR (High) | GLM-OCR remoto (Light y High usan el mismo proveedor)                                             |
| Transcripción           | faster-whisper local + AssemblyAI                       | AssemblyAI                                                                                        |
| LLM / NER / RAG         | Gemma 4 local + OpenRouter; spaCy local para NER        | OpenRouter (Gemma 4 por defecto); Vercel AI Gateway como respaldo opcional                        |
| Embeddings              | BGE-M3 local (ONNX) + OpenRouter (`baai/bge-m3`)        | OpenRouter (`baai/bge-m3`)                                                                        |
| Runtime ML nativo       | sí (se descarga solo al iniciar)                        | no                                                                                                |
| Instalador              | Windows: NSIS + MSI · Linux: DEB (GitHub)               | Windows: NSIS + MSI (GitHub) · MSIX (Store) · macOS: DMG universal (GitHub) · Linux: DEB (GitHub) |
| Identificador Tauri     | `com.entropia.pro.desktop`                              | `com.entropia.lite`                                                                               |
| Identidad MSIX de Store | —                                                       | `CONICET.EntropIALite`                                                                            |
| Se construye con        | `--features local-ml` + `VITE_LOCAL_ML=1`               | features default lean + `VITE_LOCAL_ML=0`                                                         |

**Pro** corre IA en la máquina por defecto (offline-first) y permite seleccionar proveedores remotos por configuración; los modos `auto` aplican fallback donde está implementado. **Lite** es 100% remota (OpenRouter / AssemblyAI / GLM-OCR, con Vercel AI Gateway como respaldo opcional de OpenRouter): sin modelos ni runtime nativo, instalador chico, distribución por Microsoft Store.

## Guía rápida

- **Manual de usuario**: [Manual de usuario](https://humalab.github.io/EntropIA-Pro-Lite/manual-usuario/manual-usuario.html) — guía completa de uso, en español.
- **EntropIA Pro** — Windows x64: `.exe` (NSIS) + `.msi`; Linux x64 (Ubuntu 22.04 o posterior): `.deb`. En ambos, el runtime local de IA se descarga solo al iniciar —un aviso muestra el avance y permite pausar o reintentar— y después se instalan solas las dependencias de IA. Disponibles en [Releases del repo](https://github.com/HumaLab/EntropIA-Pro-Lite/releases).
- **EntropIA Lite** — Windows x64: Microsoft Store (<https://apps.microsoft.com/detail/9N328K9L95JD>) o `.exe`/`.msi`; macOS (Apple Silicon e Intel): `.dmg` universal; Linux x64 (Ubuntu 22.04 o posterior): `.deb`. Todos en [Releases del repo](https://github.com/HumaLab/EntropIA-Pro-Lite/releases).
  - **macOS**: el `.dmg` no está notarizado por Apple. La primera vez, macOS bloquea la apertura: abrí **Configuración del Sistema → Privacidad y seguridad** y tocá **Abrir de todos modos**.
  - **Linux**: las claves de API se guardan en el llavero del sistema (gnome-keyring o KWallet). Ubuntu de escritorio ya lo trae; en instalaciones mínimas, instalá `gnome-keyring` y creá un llavero predeterminado.
- **Instaladores de Windows de GitHub (`.exe`/`.msi`, Pro y Lite)**: no están firmados. SmartScreen muestra "Windows protegió tu PC": tocá **Más información → Ejecutar de todos modos**. Algunos antivirus pueden demorar o bloquear la primera ejecución. Para Lite, la versión de Microsoft Store sí está firmada y no muestra esos avisos. Detalle en [CODE_SIGNING.md](./CODE_SIGNING.md).

## Desarrollo

### Requisitos

- Node.js 22+, pnpm 9
- Rust 1.90.0 (fijado en `rust-toolchain.toml`) / toolchain MSVC en Windows

### Instalación

```bash
git clone git@github.com:HumaLab/EntropIA-Pro-Lite.git
cd EntropIA-Pro-Lite
pnpm install --frozen-lockfile
```

### Manual de usuario

Las fuentes del manual (Markdown + generador) viven en `manual/`; se publican en GitHub Pages automáticamente vía `.github/workflows/manual-pages.yml`. Para regenerarlo en local: `python manual/manual-usuario/build_html.py --out _site` (requiere `pip install -r manual/manual-usuario/requirements.txt`).

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
pnpm exec tauri build --config src-tauri/tauri.lite.conf.json --config src-tauri/tauri.lite.windows.conf.json --bundles nsis,msi
```

> - `tauri.lite.windows.conf.json` deja afuera de los instaladores de Windows lo que solo usa Pro (uv, modelos, runtime pack, scripts de Python). Sin él, el build funciona pero pesa unos 100 MB más.
> - Usá **`pnpm exec tauri`** (no `pnpm tauri … -- …`): pnpm se come el primer `--` y rompe el pasaje de args a Cargo.
> - Por defecto no se vigila ningún checkout hermano: para hacer hot-reload sobre un checkout local de `EntropIA-Agent`, corré `pnpm exec tauri dev --config '{"build":{"additionalWatchFolders":["../../../../EntropIA-Agent/src"]}}'` (las rutas son relativas a `apps/desktop/src-tauri`).
> - Si querés correrlo desde la **raíz** sin hacer `cd`, usá `pnpm --filter @entropia-pro/desktop exec tauri ...`.
> - Lite es el default lean de Cargo. No pases `--features local-ml` cuando uses `tauri.lite.conf.json`.
> - En PowerShell `$env:VITE_LOCAL_ML` **persiste en la sesión** → seteálo en cada cambio de variante (o abrí terminal nueva). En bash va adelante: `VITE_LOCAL_ML=0 pnpm exec tauri …`.
> - Lite usa `identifier com.entropia.lite` → **datos de app separados** de Pro (podés correr ambas sin pisarte).
> - `tauri build` de Lite genera el **`.exe` (NSIS) + `.msi`**; el **MSIX** final de Store sale del repack (ver _Release e instaladores_).
> - Con el `runtime-pack` fixture commiteado, un `tauri build` de release de **Pro** (`--features local-ml`) para Windows/Linux exige definir las tres variables `ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL`, `ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID` y `ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64` con los valores de [`.github/workflows/release.yml`](./.github/workflows/release.yml). **Lite no necesita ninguna**: sin `local-ml` el runtime se compila afuera y el guard de `build.rs` no corre. `tauri dev` tampoco las necesita.

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

## Capacidades

Ambas variantes cubren los mismos flujos principales de investigación; cambia el motor (local vs remoto, ver la tabla de arriba). No tienen literalmente el mismo conjunto de runtime y UI: Pro agrega motores locales y su gestión de dependencias/modelos.

- Inicio: vista de arranque con lo último retomable, el estado del corpus (OCR, embeddings, pendientes) y accesos rápidos a Colecciones, Chat, Investigación y Escritura. Un archivo nuevo trae tres colecciones de ejemplo con el análisis hecho, que no se sincronizan.
- Organización de corpus en colecciones, ítems y assets locales (SQLite).
- Ingesta de imágenes, PDFs y audio; importación de PDF públicos desde Internet Archive (búsqueda, colección o enlace, con verificación, deduplicación y reanudación); exportación de una colección a JSON (documentos, textos, notas, entidades, layout).
- OCR Light + OCR High con persistencia de layout (bloques, regiones, páginas, bounding boxes).
- Transcripción de audio.
- Corrección, resumen y extracción semántica asistida por LLM.
- Entidades, triples, NER, FTS y embeddings asset-level (RAG).
- Procesamiento por lote: OCR, embeddings, entidades, tripletes y extracción con un esquema propio (tabla descargable como CSV) sobre colecciones enteras desde Configuración, en segundo plano, con prioridad, flujos guardados, reintentos por elemento y reanudación ante cierres o cortes.
- Notas, anotaciones y edición manual de resultados: entidades y triples se crean, editan y borran a mano, no solo se leen.
- Estado por proceso sobre cada documento (indexado, embeddings, NER, triples), para ver qué ya corrió sin volver a lanzarlo.
- Grafo 3D de las relaciones (tripletas) de una colección, con el detalle de cada nombre y los documentos de donde sale.
- Chat de investigación sobre el corpus, la Biblioteca o ambos, con título automático de cada conversación nueva y fuentes que abren el fragmento citado.
- Investigación por etapas (agente) con modelo elegible, alcance de colecciones y/o Biblioteca, presupuesto y un informe con fuentes verificables.
- Biblioteca: sincronización de bibliotecas de Zotero (catálogo, texto de PDF y capturas web, búsqueda híbrida por texto y significado), vista por obra y reprocesamiento del texto dañado con costo estimado.
- Escritura: editor de manuscritos con esquema, panel de investigación y citas ancladas al corpus, a pasajes de la Biblioteca o a una biblioteca Zotero local (estilo CSL); agente de redacción asistido por IA que nunca escribe solo; historial de versiones con restauración; sincronización propia, manuscritos compartidos entre cuentas y envío al blog de hlab.com.ar; exportación a Markdown, HTML o Word con notas al pie y bibliografía.
- Pestañas estilo navegador (hasta 4) y vista dividida de dos paneles, cada uno con su propia navegación.
- Grilla de colecciones con paginación keyset y thumbnails generados para lo que se mira, no para todo lo cargado.
- Panel lateral en árbol, con las colecciones paginadas de a tandas.
- Apariencia: temas (oscuro, cálido, claro, Lite, Papel, Bosque y Vibrante), niveles de contraste y presets tipográficos.
- Zoom de interfaz estilo navegador (75 %–125 %), desde Configuración → Apariencia o `Ctrl +/-/0`.
- Sincronización cross-device (ids deterministas para convergencia sin duplicados), incluidos manuscritos e investigaciones terminadas.
- Copias de seguridad de todo el archivo (base y archivos) a una carpeta elegida, a mano o diarias/semanales, conservando las últimas N.
- Mantenimiento del archivo: migraciones aplicadas en Rust al iniciar, auditoría de triggers y vistas, y compactación al cerrar cuando conviene.
- Aviso de actualización de Microsoft Store (Lite en Windows): detecta si hay una versión más nueva publicada y abre la ficha de la Store; no descarga ni instala nada por sí mismo.

## Cómo funciona el flag de variante

La unificación es un **strangler** sobre el código de Pro: toda la inferencia local vive detrás del feature de Cargo `local-ml` (con un sub-feature `paddle-ocr` para MNN/PaddleOCR), espejado por el flag de frontend `VITE_LOCAL_ML`.

- **`cargo build --features local-ml`** = **Pro** (motores locales + remotos).
- **`cargo build` (default)** = lean → **Lite** (solo remoto). Dropea `ort`/onnxruntime, `llama-cpp-2`, MNN/`ocr-rs`, `tokenizers` y la descarga del runtime firmado.
- El **frontend** lee `VITE_LOCAL_ML`: en Lite esconde DependenciasTab, los banners de deps y la UI de modelos locales, y la marca pasa a "EntropIA Lite".
- La **lista de comandos Tauri es idéntica** en ambas variantes; solo ramifican los cuerpos (el brazo Lite devuelve healthy/no-op, como hacía EntropIA Lite).

En cada push/PR, CI ejecuta lint, typecheck y tests del workspace con el frontend Pro, además del typecheck y los tests desktop con `VITE_LOCAL_ML=0`; también construye el frontend Pro y audita las dependencias (`pnpm audit --prod --audit-level high` y `cargo audit`, bloqueante). Lint y svelte-check fallan ante cualquier warning. Cuando cambian archivos Rust/Tauri relevantes, el contrato de features de Windows compila y enlaza Pro y Lite, el job Rust Lite de Linux corre `rustfmt`, `clippy` y `cargo test`, y el job Rust Pro de Linux corre `clippy --features local-ml`, como gates bloqueantes. El workflow **Verify binary provenance** comprueba que cada binario commiteado coincida con `apps/desktop/src-tauri/binary-provenance.json`.

## Release e instaladores

**Pro — instalador liviano + descarga al iniciar.** El runtime de IA (~2.2GB) no entra en un instalador Windows (NSIS y WiX fallan por encima de ~2GB). El instalador incluye el fixture chico de `runtime-pack` y la app descarga el runtime real al iniciar (una vez, fuera de dev y tests) desde una fuente remota firmada (ed25519), en partes reanudables, verificando firma + sha256 antes de confiar en él; el aviso del runtime muestra el avance y permite pausar y reintentar. Con el runtime listo, las dependencias de IA se instalan solas desde su wheelhouse offline. Para Windows/Linux, `build.rs` falla cerrado si un build de release de Pro (`local-ml`) embebe el fixture sin una fuente de bootstrap horneada; Lite no descarga runtime y no pasa por ese chequeo. El workflow **Release** define las variables solo para Pro.

Flujo de release de Pro:

1. **Build Runtime Pack** (plataforma `windows-x86_64` o `linux-x86_64`) → arma el runtime-pack fresco (artifact `runtime-archive`).
2. **Publish Runtime Bootstrap** con ese `runtime_pack_run_id` → parte el archivo bajo el límite de 2 GiB por asset, sube las partes al tag `runtime-bootstrap` y publica un `manifest.json` firmado.
3. Push del tag `v*` → el workflow **Release** construye NSIS + MSI en Windows y el `.deb` de Pro en Linux, con la URL del manifiesto + la clave pública **horneadas** en el binario. Antes de compilar, el workflow **se niega a construir Pro** si el manifiesto firmado no tiene una entrada para la versión de app y la plataforma exactas (`apps/desktop/scripts/check-runtime-bootstrap.mjs`).

La versión vive en un solo lugar: `apps/desktop/package.json`. Para subirla, `pnpm version:bump x.y.z` reescribe todas las declaraciones derivadas (Cargo.toml/Cargo.lock, configs de Tauri, manifests del runtime y el empaquetado del MSIX) y `pnpm version:check` verifica que ninguna se haya desincronizado.

**Lite — instaladores en GitHub + MSIX para la Store.** El job `build-lite` del workflow **Release** construye la variante lean con `--bundles nsis,msi`; el job `attach-lite-installers` adjunta el `.exe` (NSIS) + `.msi` al release de GitHub (descargables igual que los de Pro). En paralelo, el `.msi` alimenta el **repack** de un MSIX base capturado (`apps/desktop/src-tauri/msix/`), reescribiendo la identidad a `CONICET.EntropIALite` + la versión; el `.msix` sin firmar (la Store lo firma) queda sólo como artifact de Actions para Partner Center, no como asset del release.

**Lite — macOS y Linux.** El job `lite-unix` del workflow **Release** llama a **Lite preview** (`lite-preview.yml`), que construye el `.dmg` universal y el `.deb`, y prueba que la app arranque en Ubuntu 22.04 y 24.04 y en macOS arm64 e Intel. El job `attach-lite-unix` los adjunta al release solo si pasaron todas esas pruebas. El `.dmg` lleva firma ad-hoc, sin notarizar. **Lite preview** también se puede lanzar a mano para obtener los instaladores sin publicar nada.

- Para probar **solo** el MSIX de Lite sin la build de Pro: dispatch manual del workflow **Release** con la opción `lite_only=true` (o `gh workflow run release.yml -f lite_only=true`).
- El MSIX base se re-captura (VM Hyper-V, manual) **solo** si cambia la forma del paquete (assets/capabilities); los releases de rutina solo cambian el exe + suben la versión.

## Documentación útil

- [SQLite](./SQLite.md) — esquema y guía de inspección de la base local.
- [Debugging de base de datos](./DATABASE_DEBUGGING.md) — consultas operativas para diagnosticar persistencia.
- [Firma de código](./CODE_SIGNING.md) — política de firma para releases.
- [Privacidad](./PRIVACY.md) — comportamiento de datos, runtimes y proveedores externos.
- [Avisos de terceros](./THIRD_PARTY_NOTICES.md) — dependencias, modelos y runtime payloads.
- Licencia: MIT — ver [LICENSE](./LICENSE).

---

<p align="center">
  <a href="https://github.com/Gentleman-Programming/gentle-ai">
    <img
      src="https://raw.githubusercontent.com/Gentleman-Programming/gentle-ai/main/docs/assets/brand/built-with-gentle-ai.png"
      alt="Built with Gentle-AI"
      width="25%"
    >
  </a>
</p>

---

**IA local en Pro. APIs remotas en Lite.**
