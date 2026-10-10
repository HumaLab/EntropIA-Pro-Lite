# Auditoría integral del repositorio EntropIA-Pro-Lite

**Fecha:** 2026-10-09
**Commit auditado:** `0ea30b3` (rama `claude/admiring-ride-mo7mzw`, idéntica a `main` al momento de la auditoría)
**Alcance:** todo el monorepo (`apps/desktop` frontend + backend Tauri/Rust, `packages/ui`, `packages/store`, `crates/vecscan`, workflows de GitHub Actions, configuración de empaquetado, documentación e higiene del repositorio).
**Regla seguida:** no se modificó ningún archivo del proyecto. Este documento es el único cambio: registra los hallazgos y una recomendación para cada uno, sin aplicarla.

---

## 1. Resumen ejecutivo

El código está muy por encima de la media: tiene una cobertura de tests muy alta (más de 4.700 tests JS en verde y una suite Rust extensa), comentarios que explican el _por qué_, y defensas pensadas en casi todos los bordes (sanitización de HTML, política de URLs del navegador embebido, firma ed25519 del runtime, `enclosed_name` al descomprimir, llavero del sistema para secretos, `ensure_within_dir` para rutas, ACL de Tauri probada con tests).

Los hallazgos más importantes **no son bugs funcionales**. Son brechas de _defensa en profundidad_: si alguna vez se ejecutara JavaScript no confiable dentro del webview principal (por un XSS hoy desconocido, por ejemplo vía una dependencia vulnerable), ese código tendría más poder del que el diseño declara:

1. **El plugin `fs` permite escribir y borrar en todo `$HOME`**, no solo leer (S-01). Es el hallazgo de mayor impacto.
2. **Los validadores SQL del IPC (`db_*`) se pueden evadir** con un comentario inicial, lo que habilita `ATTACH`, `VACUUM INTO` y `PRAGMA` (S-02, confirmado empíricamente con SQLite).
3. **La fuente de confianza del runtime de Pro (URL del manifiesto + clave pública) se puede pisar desde `settings_set`** (S-03): un renderer comprometido podría llevar a Pro a instalar y ejecutar binarios firmados por un tercero.
4. **Dependencias de producción con avisos conocidos**, entre ellos un XSS en el pegado de `prosemirror-view` (lo usa el editor de Escritura) y `html-docx-js`, que está abandonado (D-01, D-02).

En operación y mantenibilidad: la CI no ejecuta clippy ni tests de Rust para Lite ni para Linux/macOS (C-01), y de hecho `cargo test` falla hoy en Linux (Q-05); el log de la app crece sin límite (P-02); hay errores de migración que hacen `panic` al arrancar sin mostrar diálogo (A-02); y el esquema se migra desde dos lugares, Rust y TS (A-01).

### Conteo por severidad

| Severidad | Cantidad |
| --------- | -------- |
| Alta      | 4        |
| Media     | 15       |
| Baja      | 18       |
| Info      | 8        |

> Criterio de severidad: **Alta** = explotable o con impacto serio sobre los datos o el sistema del usuario si se da una condición plausible. **Media** = riesgo real pero acotado, o deuda que ya está costando. **Baja** = mejora recomendable. **Info** = observación sin acción urgente.

---

## 2. Metodología y resultados de las verificaciones automáticas

Se instalaron dependencias con `pnpm install --frozen-lockfile` (Node 22.22, pnpm 9.15.4, Rust 1.90.0 según `rust-toolchain.toml`) y se ejecutó:

| Verificación                                                     | Resultado                                                                                                                                                                                         |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pnpm lint`                                                      | ✅ 0 errores, 5 warnings (3 `no-explicit-any` en `writing-image-paste.test.ts`, 1 en `document-explorer.test.ts`, 1 directiva `eslint-disable` sin uso en `navegador-capture-script.test.ts:259`) |
| `pnpm typecheck` (Pro)                                           | ✅ 0 errores, 8 warnings de Svelte (`state_referenced_locally` en fixtures/`WorkPane.svelte`, `non_reactive_update` en `EntropicConstellation.svelte:53`)                                         |
| `VITE_LOCAL_ML=0 … typecheck` (Lite)                             | ✅ 0 errores, los mismos 8 warnings                                                                                                                                                               |
| `pnpm format:check`                                              | ✅ todo formateado                                                                                                                                                                                |
| `pnpm test:run` (Pro)                                            | ✅ store 374/374 · ui 847/847 · desktop 3480 pasan + 7 skipped (3487)                                                                                                                             |
| `VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test`       | ✅ 3459 pasan + 28 skipped (3487), 247 archivos                                                                                                                                                   |
| `cargo test` (Lite, Linux, sin features)                         | ❌ 2692 pasan, **2 fallan**, 34 ignorados. Los dos fallos asumen semántica de rutas de Windows. Ver Q-05                                                                                          |
| `cargo clippy --all-targets` (Lite, Linux) + `cargo fmt --check` | ✅ 0 warnings · formato OK                                                                                                                                                                        |
| `cargo audit`                                                    | ⚠️ 6 vulnerabilidades (`lopdf 0.34`, `rustls 0.23.39`, `quick-xml 0.38.4` ×2, `crossbeam-epoch 0.9.18`, `quinn-proto 0.11.14`) + 12 warnings (crates sin mantenimiento o _unsound_). Ver D-08     |
| `pnpm audit --prod`                                              | ⚠️ 21 avisos (9 high, 11 moderate, 1 low). Ver D-01/D-02/D-03                                                                                                                                     |
| `pnpm audit` (incluye dev)                                       | ⚠️ 70 avisos (4 critical, 37 high, 26 moderate, 3 low). Los critical son de tooling de test (vitest/tinypool/happy-dom). Ver D-04                                                                 |
| Búsqueda de secretos (regex de claves conocidas)                 | ✅ sin claves reales en archivos versionados                                                                                                                                                      |

Además se hizo revisión manual de: superficie IPC/ACL de Tauri, CSP, plugin `fs`, protocolo `asset`, validadores SQL, gestión de secretos, bootstrap/descarga del runtime de Pro, navegador embebido, sanitización de HTML en el frontend, apertura de URLs y procesos, sincronización (TLS), runner de migraciones, workflows de CI/release, empaquetado por variante, binarios versionados e higiene del repo.

Métricas de tamaño (líneas, sin dependencias): Rust `src` ≈ 190.800 · tests Rust de integración ≈ 25.500 · frontend desktop TS/Svelte ≈ 79.500 (+ ≈ 72.000 de tests) · `packages/ui` ≈ 19.000 · `packages/store` ≈ 8.900 · 234 comandos `#[tauri::command]`.

---

## 3. Hallazgos

Formato de cada hallazgo: **ID — título** · severidad · ubicación · descripción · impacto · recomendación.

### 3.1 Seguridad

#### S-01 — El plugin `fs` permite escribir y borrar en todo `$HOME`, no solo leer · **Alta**

- **Ubicación:** `apps/desktop/src-tauri/capabilities/default.json:243-259`.
- **Descripción:** la capability incluye `fs:allow-home-read-recursive`, `fs:allow-desktop-read-recursive`, `fs:allow-document-read-recursive` y `fs:allow-download-read-recursive`. En `tauri-plugin-fs` 2.6.0 esos _sets_ se componen de `read-all` + `scope-home-recursive` (y equivalentes). Los permisos `scope-*` no declaran comandos, así que el ACL de Tauri los trata como **scope global del plugin**: valen para _todos_ los comandos `fs` permitidos, incluidos `fs:allow-write-file`, `fs:allow-remove`, `fs:allow-copy-file` y `fs:allow-mkdir`. El `fs:scope` explícito (solo `$DATA/com.entropia.shared/**` y `$LOCALDATA/…`) sugiere que la intención era limitar la escritura al directorio de datos, pero en los hechos el webview puede escribir y borrar cualquier archivo bajo `$HOME`.
- **Impacto:** si se ejecutara JS arbitrario en el webview principal, podría persistir código (por ejemplo en `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup`, `~/.bashrc`, `~/.config/autostart`), borrar documentos del usuario o sobrescribir `entropia.sqlite`. También vuelve irrelevantes las protecciones de S-02 y S-03, porque el archivo SQLite se puede leer o reemplazar directamente.
- **Recomendación:** quitar los `*-read-recursive` y usar los paths que devuelve `dialog:allow-open/save` (Tauri los agrega al scope en tiempo de ejecución), o mover la lectura de archivos importados a comandos Rust que validen la ruta. Agregar un test a `tests/app_acl.rs` que pruebe que `plugin:fs|write_file` y `plugin:fs|remove` fallan fuera de `$DATA/com.entropia.shared`.

#### S-02 — Los validadores SQL del IPC se evaden con un comentario inicial (y tienen otras brechas) · **Alta**

- **Ubicación:** `apps/desktop/src-tauri/src/db/commands.rs:480-662` (`normalize_sql`, `validate_sql_batch`, `validate_sql_row_query`, `validate_sql_execute`, `statement_writes_sync_objects`).
- **Descripción:** el renderer tiene acceso a `db_execute_batch`, `db_select` y `db_select_rows`, y la protección depende de mirar la _primera palabra_ de cada sentencia. `normalize_sql` no quita comentarios, así que:
  - `/**/VACUUM INTO '/ruta/copia.db'` o `--x\nATTACH DATABASE '/ruta/x.db' AS e` pasan `validate_sql_batch` (el primer token es `/**/vacuum` o `--`). **Verificado** con SQLite: `VACUUM INTO` copia la base entera, `app_settings` incluida, a la ruta que se elija; `ATTACH` crea archivos arbitrarios en disco. El chequeo de `app_settings` no se dispara porque la sentencia no nombra la tabla.
  - `validate_sql_row_query` (usado por `db_select*`) acepta `INSERT/UPDATE/DELETE … RETURNING` **sin** llamar a `statement_writes_sync_objects`, así que se puede escribir `sync_oplog`, `sync_meta`, etc., algo que el diseño (DESIGN §6.2) prohíbe.
  - En `db_execute_batch`, `WITH … INSERT INTO sync_x …` y `UPDATE OR REPLACE sync_x …` tampoco se detectan (el _leading keyword_ es `with`, o el token siguiente a `update` es `or`).
  - `db_execute_batch` permite DDL arbitrario (`DROP TABLE`, `CREATE TRIGGER`) porque el runner de migraciones TS lo necesita (ver A-01).
- **Impacto:** defensa en profundidad rota: el comentario de `sql_references_sensitive_table` dice que "el renderer nunca debe alcanzar" los secretos, y eso no se cumple. El daño real es acotado porque las claves viven en el llavero del sistema (en `app_settings` solo quedan referencias `secret_ref:`), salvo cuando falló la migración al llavero y quedan claves _legacy_ en texto plano (Linux sin Secret Service).
- **Recomendación:** quitar comentarios antes de normalizar (o, mejor, usar el parser de SQLite: `sqlite3_stmt_readonly` y `sqlite3_set_authorizer` vía `rusqlite::Connection::authorizer`, para negar `SQLITE_ATTACH`, `SQLITE_PRAGMA`, operaciones sobre `app_settings` y escrituras a `sync_*` desde la conexión de la UI). Aplicar el chequeo de `sync_*` también en `validate_sql_row_query`. A mediano plazo, mover las migraciones a Rust para que el renderer no necesite DDL (A-01).

#### S-03 — La URL y la clave pública del runtime de Pro se pueden pisar desde el renderer · **Alta (solo Pro)**

- **Ubicación:** `apps/desktop/src-tauri/src/settings.rs:105-122` (`settings_set` acepta cualquier `key`), `settings.rs:452-484` (`persist_setting_with`), `settings.rs:673-698` y `settings.rs:790-822` (`get_runtime_bootstrap_remote_source_with_builtin`, `get_runtime_bootstrap_public_key_with_builtin`).
- **Descripción:** si en `app_settings` existen `runtime_bootstrap_manifest_url` y `runtime_bootstrap_public_key_id`, tienen **prioridad sobre los valores compilados** (`option_env!`). La clave pública se busca primero en `app_settings` (`runtime_bootstrap_public_key.<id>`). `settings_set` y `settings_delete` no tienen lista blanca de claves, así que el webview puede fijar las tres. La UI nunca escribe esas claves (no aparecen en `apps/desktop/src`).
- **Impacto:** un renderer comprometido puede apuntar Pro a un manifiesto firmado con una clave propia. La firma "valida" y el runtime descargado (Python, uv y binarios nativos) se ejecuta: es una escalada de JS en el webview a ejecución de código nativo. La única condición es que haga HTTPS.
- **Recomendación:** que la fuente compilada sea la única confiable en builds release (o que `app_settings` solo pueda _agregar_ claves y nunca reemplazar la compilada), y poner una lista blanca de claves en `settings_set` y `settings_delete` (rechazar `runtime_bootstrap_*`). Si la anulación se necesita para desarrollo, restringirla a `cfg(debug_assertions)` o a una variable de entorno.

#### S-04 — XSS en el pegado de `prosemirror-view` (dependencia del editor de Escritura) · **Alta**

- **Ubicación:** `prosemirror-view@1.41.8` (transitiva de `@tiptap/pm@2.27.2`, usada por `packages/ui/src/components/WritingEditor` y `NoteEditor`).
- **Descripción:** `pnpm audit` reporta "ProseMirror has a XSS vulnerability in prosemirror-view's paste handling" (corregido en `>=1.42.3`). El editor acepta pegado desde el portapapeles, que puede venir de páginas web.
- **Impacto:** es exactamente la condición que activa S-01 a S-03. El CSP (`script-src 'self'`) mitiga mucho, pero no anula los vectores que no requieren script inline.
- **Recomendación:** actualizar `@tiptap/*` o forzar `prosemirror-view >=1.42.3` con `pnpm.overrides`, y verificar con los tests de pegado existentes (`writing-image-paste.test.ts`).

#### S-05 — El protocolo `asset` y `fs:allow-read-file` exponen el archivo SQLite entero · **Media**

- **Ubicación:** `tauri.conf.json` (`assetProtocol.scope`: `$DATA/com.entropia.shared/**/*`) y `capabilities/default.json:243-254`.
- **Descripción:** `entropia.sqlite` (con `-wal` y `-shm`) vive en `$DATA/com.entropia.shared/`. Tanto `asset://` como `plugin:fs|read_file` pueden leerlo crudo, sin pasar por los validadores SQL. Las capturas HTML del Navegador (con sus `<script>` originales, ver `navegador/capture.rs:19`) también quedan dentro del scope del protocolo `asset`.
- **Impacto:** el _hardening_ de `app_settings` en los comandos `db_*` no protege nada si el archivo se puede leer directo. Las capturas no se renderizan hoy en el webview principal (`frame-src 'none'` también ayuda), pero quedan servibles desde un origen con IPC.
- **Recomendación:** acotar el scope de `asset` a los subdirectorios de medios (`assets/**`, miniaturas) y excluir `*.sqlite*` y `web-captures/**`, o servirlos con un protocolo propio que valide la extensión y el `Content-Type`.

#### S-06 — Operaciones del keyring con el lock de la conexión de UI tomado · **Baja**

- **Ubicación:** `writing/publish.rs:58-66`, `settings.rs:92-103` y en general `get_setting()` sobre `ui_conn`.
- **Descripción:** `get_setting` resuelve el secreto en el llavero del sistema (D-Bus/Keychain/Credential Manager) mientras se tiene `db.ui_conn.lock()`. En Linux, Secret Service puede pedir desbloquear el llavero de forma interactiva.
- **Impacto:** mientras tanto, toda consulta de la UI queda bloqueada.
- **Recomendación:** leer la referencia bajo el lock, soltarlo y recién ahí consultar el llavero.

#### S-07 — `validate_external_url` valida con prefijos de texto, sin parsear · **Info**

- **Ubicación:** `apps/desktop/src-tauri/src/lib.rs:409-432`.
- **Descripción:** se aceptan cadenas que empiezan con `http://` o `https://` y no tienen caracteres prohibidos. En Windows se pasan a `rundll32 url.dll,FileProtocolHandler`. Hoy es seguro gracias a la lista de caracteres prohibidos, pero parsear con `url::Url` y re-serializar sería más robusto ante variantes futuras.

### 3.2 Dependencias y cadena de suministro

#### D-01 — `html-docx-js@0.3.1` abandonado, con dependencias vulnerables · **Media**

- **Ubicación:** `apps/desktop/package.json`, `apps/desktop/src/lib/ocr-export.ts:3,140-185`.
- **Descripción:** el paquete no se publica desde 2016 y arrastra `lodash.merge@3.3.2` (prototype pollution, 2 avisos _high_) y `jszip@2.7.0` (path traversal). Además se carga **inyectando un `<script>`** con el bundle en tiempo de ejecución. Ya existe `docx@9.7.1` para Escritura (`export-docx.ts`, cuyo comentario dice que `html-docx-js` "no tiene modelo de documento").
- **Recomendación:** migrar la exportación DOCX del OCR a `docx` y eliminar `html-docx-js`.

#### D-02 — Avisos en dependencias de producción del frontend · **Media**

- `drizzle-orm@0.40.1`: SQLi por identificadores mal escapados (corregido en `>=0.45.2`). El riesgo es bajo porque los identificadores son estáticos, pero conviene actualizar.
- `svelte@5.55.3` (vía `@tabler/icons-svelte-runes`): XSS por DOM clobbering, ReDoS en `<svelte:element>` y avisos de SSR (`>=5.55.7`). El proyecto declara `svelte ^5`, pero el lock resuelve 5.55.3.
- `devalue@5.7.1`: varios DoS (`>=5.9.3`).
- `markdown-it@14.1.1` y `linkify-it@5.0.0`: complejidad cuadrática (`markdown-it >=14.3.1`, `linkify-it >=5.0.2`). Se usan para renderizar el OCR (`ocr-rich-text.ts`) con texto que viene de proveedores remotos.
- `@tiptap/core@2.27.2`: `mergeAttributes()` y `__proto__` (`>=3.30.4`, requiere migrar a Tiptap 3).
- **Recomendación:** `pnpm update` dirigido y/o `pnpm.overrides` para las transitivas, y agregar `pnpm audit --prod --audit-level high` a la CI (ver C-03).

#### D-08 — Avisos RustSec en el backend · **Media**

- **`lopdf 0.34.0`** (RUSTSEC-2026-0187, _stack overflow_ con objetos PDF profundamente anidados), que entra vía `pdf-extract 0.7.12`; el proyecto usa directamente `lopdf 0.45`, así que hay dos versiones en el árbol. Los PDFs los importa el usuario o llegan desde el Navegador o Zotero. Un _stack overflow_ **no** lo atrapa `catch_unwind` (`ocr::pdf::extract_pdf_text`), así que el proceso entero aborta, a pesar del `panic = "unwind"` que el `Cargo.toml` configura justamente para contener fallos de `pdf-extract`.
- **`rustls 0.23.39`** (RUSTSEC-2026-0285, mensajes de handshake TLS 1.3 aceptados entre niveles de cifrado; corregido en `>=0.23.45`). Lo usa `reqwest` para todo el tráfico remoto (OpenRouter, AssemblyAI, GLM-OCR, sync).
- `quick-xml 0.38.4` (2 DoS), `crossbeam-epoch 0.9.18` y `quinn-proto 0.11.14` (este último está en el lock pero no en el árbol activo de Linux).
- Warnings: `imageproc 0.25.0` (3 avisos de chequeo de límites, dependencia directa), `anyhow` (`downcast_mut` _unsound_), `glib 0.18.5` y `rand 0.8.5` _unsound_; `core2` (_yanked_), `paste`, `proc-macro-error` y `ttf-parser` sin mantenimiento.
- **Recomendación:** `cargo update -p rustls -p crossbeam-epoch -p quick-xml`; evaluar reemplazar `pdf-extract` (o fijar su `lopdf` a `>=0.42` si una versión nueva lo permite) y procesar PDFs no confiables en un subproceso o en un hilo con pila acotada; actualizar `imageproc`. Agregar `cargo audit` a la CI (C-03).

#### D-03 — Dependencias declaradas sin uso o duplicadas · **Baja**

- `@tauri-apps/plugin-sql` está declarado en `apps/desktop/package.json`, pero nunca se importa y el backend no registra `tauri-plugin-sql`.
- Las dependencias `@tiptap/*` y `leaflet` están duplicadas entre `apps/desktop` y `packages/ui` (riesgo de que las versiones diverjan; `ui` debería ser la única dueña).
- Hay dos generadores DOCX (`docx` y `html-docx-js`, ver D-01) y dos renderizadores Markdown (`lib/markdown.ts` propio para salidas de LLM y `markdown-it` para el OCR). Lo segundo tiene justificación, pero conviene documentarlo.
- `jsdom` y `happy-dom` conviven como entornos de test (22 archivos fuerzan `jsdom`).
- **Recomendación:** quitar `plugin-sql` y centralizar las versiones de Tiptap en un solo `package.json` (o en un catálogo de pnpm).

#### D-04 — Tooling de desarrollo con avisos _critical_ · **Baja**

- `vitest@3.2.4`/`@vitest/ui` (lectura y ejecución de archivos cuando el servidor UI escucha), `tinypool@1.1.1` y `happy-dom@17.6.3` (escape del contexto VM): 4 _critical_. Además `vite@6.4.2` (bypass de `server.fs.deny` en Windows), `postcss`, `brace-expansion`, `js-yaml`, `nanoid` y `ws`.
- **Impacto:** afecta solo a las máquinas de desarrollo y a la CI, no al binario distribuido. El servidor de Vite en `1420` y `vitest --ui` son superficie local.
- **Recomendación:** actualizar a `vitest` ≥ 3.2.6 (o 4.x), `happy-dom` ≥ 20 y `vite` ≥ 6.4.3.

#### D-05 — Motor de investigación con dependencia `git` a `branch = "main"` y avance automático a `main` · **Media**

- **Ubicación:** `apps/desktop/src-tauri/Cargo.toml` (`entropia-agent = { git = …, branch = "main" }`) y `.github/workflows/engine-pin-bump.yml`.
- **Descripción:** el `Cargo.lock` fija el commit, pero un workflow diario actualiza ese pin y **hace fast-forward de `main` del repo sin revisión humana** si la CI pasa. Quien pueda escribir en `HumaLab/EntropIA-Agent` mete código en `main` de esta app, y de ahí en el próximo release, sin PR.
- **Recomendación:** que el bot abra un PR (o exija aprobación de un entorno protegido) en lugar de empujar a `main`, o fijar `rev =` en `Cargo.toml` y subirlo por PR.

#### D-06 — Acciones de GitHub fijadas por tag, no por SHA · **Baja**

- **Ubicación:** todos los workflows (`tauri-apps/tauri-action@v0`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, `pnpm/action-setup@v6`, etc.).
- **Descripción:** `@v0` y `@stable` son referencias móviles. El job de release tiene `contents: write` y empaqueta los instaladores. Además hay versiones mezcladas de `upload-artifact` y `download-artifact` (`v4`, `v6` y `v7`).
- **Recomendación:** fijar por SHA (con Dependabot para actualizarlas) y unificar las versiones de `*-artifact`.

#### D-07 — `uv.exe` (≈ 90 MB) y `pdfium.dll` versionados sin manifiesto de procedencia · **Baja**

- **Ubicación:** `apps/desktop/src-tauri/resources/tools/uv/windows-{x86_64,aarch64}/uv.exe`, `resources/lib/pdfium.dll`, modelos `.mnn` y `msix/EntropIALite-base.msix`.
- **Descripción:** son binarios ejecutables dentro de git, sin Git LFS y sin un archivo con versión y SHA-256 junto a cada uno (para `pdfium.dll`, la procedencia solo figura en un comentario de `fetch-pdfium.sh`). Inflan el clon (`.git` ≈ 80 MB aun en un clon superficial de 145 commits) y no se pueden verificar de forma automática.
- **Recomendación:** que se descarguen en el build con hash fijado (como ya hace `fetch-pdfium.sh` para macOS y Linux), o al menos un `CHECKSUMS` versionado y verificado en la CI.

### 3.3 CI/CD y release

#### C-01 — La CI no compila ni prueba Rust en Lite ni en Linux/macOS · **Media**

- **Ubicación:** `.github/workflows/ci.yml:59-101`.
- **Descripción:** clippy y `cargo test` corren **solo en Windows con `--features local-ml`** (Pro). El build _lean_ (Lite, sin features) se valida solo como "contrato de features", y Linux/macOS solo en `lite-preview.yml` (release). Además, los jobs Rust dependen de `detect-rust-changes`.
- **Impacto:** el código `#[cfg(not(feature = "local-ml"))]`, `#[cfg(target_os = "linux"/"macos")]` y `deps/mod_lite.rs` puede romperse sin que nadie se entere hasta el release.
- **Recomendación:** agregar un job `ubuntu-latest` con `cargo clippy --all-targets -- -D warnings` y `cargo test` sin features (Lite). En esta auditoría ese build corrió en Linux en menos de una hora en frío: clippy dio 0 warnings, pero `cargo test` mostró 2 fallos que la CI no ve (Q-05).

#### C-02 — URL del manifiesto del runtime apunta a otro repo (`HumaLab/EntropIA-Pro`) · **Media**

- **Ubicación:** `.github/workflows/release.yml:31`, `.github/workflows/lite-preview.yml:27`.
- **Descripción:** los binarios compilan con `https://github.com/HumaLab/EntropIA-Pro/releases/download/runtime-bootstrap/manifest.json`, pero `publish-runtime-bootstrap.yml` publica en `${{ github.repository }}` (= `EntropIA-Pro-Lite`). Si `EntropIA-Pro` es el nombre viejo del repo, hoy funciona por la redirección de GitHub, pero esa redirección se pierde si alguien crea un repo nuevo con ese nombre.
- **Impacto:** disponibilidad. Pro no podría descargar su runtime en una instalación limpia. La firma ed25519 sigue protegiendo la integridad.
- **Recomendación:** apuntar a la URL canónica del repo actual o a un dominio propio, y agregar un test que compare `release.yml` con la salida de `publish-runtime-bootstrap.yml`.

#### C-03 — Falta auditoría de dependencias en la CI · **Baja**

- No hay `pnpm audit`, `cargo audit`/`cargo deny` ni Dependabot/Renovate configurado (`.github/` no tiene `dependabot.yml`). Los avisos de D-01 a D-04 se acumularon sin alerta.
- **Recomendación:** agregar Dependabot (npm, cargo, github-actions) y un job no bloqueante de auditoría.

#### C-04 — Versión de la app duplicada en ~9 lugares · **Baja**

- **Ubicación:** `Cargo.toml`, `tauri.conf.json`, `tauri.lite.conf.json`, `apps/desktop/package.json`, `release.yml:225` (`-StoreVersion "1.0.20.0"` y nombre del `.msix`), `scripts/repack-store-msix.ps1`, `msix/README.md` y los dos `runtime-pack/*/manifest.json`.
- **Descripción:** `runtime-packaging.test.ts` solo compara `package.json` con `tauri.conf.json` y los manifiestos. `Cargo.toml`, `tauri.lite.conf.json` y `release.yml` quedan sin chequear. El `.msix` versionado en `artifacts/msix/` es de la `1.0.5.0`.
- **Recomendación:** que `release.yml` derive `StoreVersion` del tag, y extender el test a todos los archivos.

#### C-05 — `ci.yml` es muy largo y repite pasos de diagnóstico · **Info**

- 701 líneas, con pasos de _forensics_ de pnpm repetidos en cada job (post-checkout, post-pnpm-setup, post-setup-node, pre-install, clasificación, parse YAML). Parecen restos de una investigación de un problema de lockfile.
- **Recomendación:** si el problema ya está resuelto, extraerlos a una acción compuesta o eliminarlos.

#### C-06 — Instaladores de GitHub sin firmar; DMG sin notarizar · **Info**

- Está documentado (`CODE_SIGNING.md`), pero sigue siendo un riesgo para el usuario (que se acostumbre a saltear SmartScreen o Gatekeeper). Se registra para que quede en el backlog.

### 3.4 Arquitectura y datos

#### A-01 — Dos sistemas de migración del esquema (Rust y TypeScript) · **Media**

- **Ubicación:** `packages/store/src/runner.ts` (≈ 3.200 líneas, 58 migraciones en strings) y `apps/desktop/src-tauri/src/lib.rs:850-970` (parches de esquema en `setup`: índices únicos, CHECK de `extractions.method`, `llm_results`, `layouts`, `sort_index`, `asset_id`, `app_settings`).
- **Descripción:** el esquema se crea desde el renderer vía `db_execute_batch` (eso explica por qué ese comando acepta DDL, ver S-02), y Rust aplica correcciones propias antes y en paralelo ("Fresh installs may not have these tables yet (created later by JS migrations)"). Además, `packages/store/src/migrations/*.sql` son _espejos para revisión_ de algunas migraciones (faltan 0007, 0011-0014, 0019 y 0025), y solo algunos tienen test de igualdad.
- **Impacto:** el orden de ejecución depende de quién llega primero; se repiten fuentes de verdad; el renderer necesita privilegios de DDL.
- **Recomendación:** centralizar las migraciones en Rust (con `include_str!` de los `.sql`) antes de exponer la conexión a la UI. Así se puede cerrar el DDL en el IPC.

#### A-02 — `panic` en el arranque ante errores de migración, sin diálogo · **Media**

- **Ubicación:** `apps/desktop/src-tauri/src/lib.rs:884, 895, 897, 901, 922, 945, 964, 968, 998` (`.expect("Failed to …")`).
- **Descripción:** en el mismo `setup` ya existe un helper `fail(...)` que devuelve errores legibles ("No se pudo abrir el estado de investigaciones."), pero estos pasos hacen `panic`. En Windows release no hay consola, así que la app se cierra sin explicación. Además el `DELETE FROM extractions WHERE rowid NOT IN (SELECT MAX(rowid) … GROUP BY asset_id)` corre **en cada arranque**, fuera de `without_sync_capture`.
- **Recomendación:** convertirlos a `fail(...)` con mensaje para el usuario y mover la deduplicación a una migración que se ejecute una sola vez.

#### A-03 — La mayoría de las migraciones _legacy_ no son atómicas · **Baja**

- **Ubicación:** `packages/store/src/runner.ts:3108-3162`.
- **Descripción:** solo las migraciones listadas a mano en un `if (name === … || …)` de 27 nombres corren en `BEGIN IMMEDIATE … COMMIT` junto con el registro en `_migrations`. El resto se parte por `;` (`splitStatements`, que rompe cuerpos de `TRIGGER`; así nació el incidente de la 0032 documentado en el código) y se aplica sentencia por sentencia en _autocommit_, ignorando `duplicate column name`.
- **Impacto:** una migración nueva que no se agregue a la lista hereda el comportamiento no atómico.
- **Recomendación:** invertir el default (todas atómicas salvo una lista explícita de excepciones) o resolverlo con A-01.

#### A-04 — Transacciones que abarcan varias llamadas IPC sobre la conexión compartida · **Baja**

- **Ubicación:** `packages/store/src/repos/item.repo.ts:1513-1563` (`deleteWithCascade`), el runner de migraciones (`ROLLBACK` en una llamada aparte) y patrones similares en `asset.repo.ts`.
- **Descripción:** `BEGIN; …; COMMIT;` va en un `executeBatch`; si falla a mitad, el `ROLLBACK` sale en _otra_ llamada IPC. Entre ambas, cualquier otra consulta de la UI sobre la misma `ui_conn` corre **dentro de la transacción abierta** y se revierte con ella. Además el SQL se arma interpolando `id.replace(/'/g, "''")` en lugar de parámetros (7 lugares), y `vec_assets` se borra fuera de la transacción ("best-effort"), lo que puede dejar vectores huérfanos (la tabla no tiene FK).
- **Recomendación:** que el comando `db_execute_batch` haga el `ROLLBACK` del lado de Rust si falla (o usar `db_execute_transaction` con parámetros, que ya existe).

#### A-05 — Comandos `async` que bloquean el runtime de Tokio · **Baja**

- **Ubicación:** `settings.rs` (`settings_get`, `settings_set`, `settings_get_all`, `settings_delete`), `writing/publish.rs` y otros que toman `std::sync::Mutex` y hacen I/O SQLite y de llavero directo en un `async fn`.
- **Descripción:** los comandos `db_*` usan `spawn_blocking` a propósito ("so IPC commands never execute SQL on the main thread"), pero estos no.
- **Recomendación:** aplicar el mismo `run_blocking_db_task` a todos.

#### A-06 — Archivos "dios" difíciles de mantener · **Info**

- `processing/repository.rs` (10.899 líneas), `bibliography/processing.rs` (4.426), `ocr/pdf.rs` (3.916), `lib.rs` (3.533, mezcla `setup`, migraciones legacy, cierre ordenado y apertura de URLs), `i18n.ts` (5.097, ambos idiomas en un archivo), `SettingsView.svelte` (3.713), `ItemView.svelte` (3.432), `ItemView.test.ts` (5.751).
- **Recomendación:** partirlos por responsabilidad cuando se vuelvan a tocar (por ejemplo `i18n/es.ts` + `i18n/en.ts`, y `lib.rs` → `setup/migrations.rs`).

### 3.5 Rendimiento

#### P-01 — La búsqueda vectorial del RAG recorre toda `vec_assets` en cada pregunta · **Media**

- **Ubicación:** `apps/desktop/src-tauri/src/rag/retrieval.rs:191-232`.
- **Descripción:** `SELECT v.asset_id, v.embedding FROM vec_assets v` sin filtro, cálculo de coseno fila por fila leyendo BLOBs de SQLite y, después, `sort_by` de **todos** los puntajes para quedarse con `limit`. La bibliografía ya resolvió lo mismo con un índice en memoria cuantizado (`crates/vecscan`), pero el RAG del corpus no lo reutiliza.
- **Impacto:** el costo es O(N·D) en I/O y CPU por pregunta. El comentario del código menciona 200k páginas: son segundos por consulta en máquinas modestas.
- **Recomendación:** reutilizar `vecscan` (con caché invalidada por generación) y una selección parcial (`select_nth_unstable_by` o un heap de tamaño k) en lugar del ordenamiento completo.

#### P-02 — El log de la app crece sin límite y se relee entero al arrancar · **Media**

- **Ubicación:** `apps/desktop/src-tauri/src/app_logs.rs:126-145` (append sin rotación) y `:220-236` (`fs::read_to_string` del archivo completo en `AppLogsState::new`).
- **Descripción:** en memoria se recortan 2.000 entradas, pero el archivo en disco solo se vacía con `logs_clear`. Con meses de uso, el arranque lee y parsea un JSONL de cientos de MB.
- **Recomendación:** reescribir el archivo con las últimas `MAX_LOG_ENTRIES` al iniciar, o rotar por tamaño.

#### P-03 — La verificación del runtime de Pro lee archivos enormes completos en memoria · **Baja**

- **Ubicación:** `apps/desktop/src-tauri/src/runtime/download.rs:560-590` (`verify_extracted_runtime` usa `fs::read(&target)` por entrada).
- **Descripción:** el runtime pesa ≈ 2,2 GB, así que un archivo grande (una librería CUDA, por ejemplo) se carga entero en RAM para hacerle SHA-256. `runtime/manager.rs:1257` (`file_sha256`) hace lo mismo.
- **Recomendación:** calcular el hash en streaming con un `BufReader`.

#### P-04 — 371 `eprintln!` frente a 106 llamadas a `app_logs` · **Baja**

- **Descripción:** en Windows release (subsistema GUI) `stderr` se descarta, así que la mayoría de los diagnósticos del backend se pierden justo donde más se necesitan.
- **Recomendación:** pasar a `tracing` o `log` con un _sink_ hacia `app_logs` para niveles `warn` y `error`.

### 3.6 Empaquetado y variantes

#### E-01 — Lite para Linux empaqueta _payload_ exclusivo de Pro · **Media**

- **Ubicación:** `apps/desktop/src-tauri/tauri.lite.linux.conf.json` (`resources`).
- **Descripción:** a diferencia de `tauri.lite.windows.conf.json`, que excluye explícitamente lo exclusivo de Pro, la configuración Lite de Linux incluye `resources/models/ocr/*` (≈ 15 MB de modelos MNN), `resources/runtime-pack/linux-x86_64/**`, `resources/lib/linux-x86_64/**` y los scripts Python (`paddle_vl.py`, `spacy_ner.py`, `transcribe.py`), que Lite no usa.
- **Recomendación:** replicar la lista mínima de Windows.

#### E-02 — `resources/lib/linux-x86_64/libpdfium.so` y `libonnxruntime.so` son _fixtures_ de texto que se empaquetan · **Media (a verificar)**

- **Ubicación:** `apps/desktop/src-tauri/resources/lib/linux-x86_64/` (archivos de 49 y 54 bytes con el texto "fixture bundled pdfium for linux resource audit"), `tauri.linux.conf.json` y `src/ocr/pdf.rs:403-432`.
- **Descripción:** el `.deb` de Pro (y el de Lite) los incluye. `host_pdfium_candidate_paths` (usado por el lector de bibliografía, que es _runtime-free_) prueba `resources/lib/linux-x86_64/libpdfium.so` y lo elige si existe. En Pro Linux no hay `resources/pdfium/` (solo lo descarga `lite-preview.yml`), así que el resolutor devolvería un archivo de texto y la carga de Pdfium fallaría. La cadena del corpus prueba primero el runtime gestionado, por eso el problema se esconde una vez descargado el runtime.
- **Recomendación:** verificar en un `.deb` de Pro recién instalado sin runtime si la lectura de PDFs de la Biblioteca funciona. Si se confirma, empaquetar el `libpdfium.so` real (`fetch-pdfium.sh linux-x64`) también en Pro, y que el resolutor rechace archivos menores a cierto tamaño o que no sean ELF.

#### E-03 — El guard de `build.rs` exige variables de Pro también en Lite y no valida `PUBLIC_KEY_ID` · **Baja**

- **Ubicación:** `apps/desktop/src-tauri/build.rs:292-330`.
- **Descripción:** el guard solo exige `…MANIFEST_URL` y `…PUBLIC_KEY_BASE64`. Si falta `…PUBLIC_KEY_ID`, el build pasa y el error recién aparece en tiempo de ejecución ("partially configured"). Lite paga el requisito aunque no tenga runtime (está documentado en el README, pero es fricción). Además, `AGENTS.md` lista solo dos de las tres variables.
- **Recomendación:** validar las tres y saltear el guard si `CARGO_FEATURE_LOCAL_ML` no está definido.

#### E-04 — Pequeñas inconsistencias de configuración · **Baja**

- `tauri.dev.conf.json` usa el identificador `com.entropia.pro.desktop.dev`, pero el título de ventana es `"EntropIA Lite"`.
- `tauri.conf.json:6` → `additionalWatchFolders: ["../../../../EntropIA-Agent/src"]` apunta fuera del repo (asume un checkout hermano).
- El `user_agent` está fijado en `"EntropIA-Desktop/0.1 (historical-research-app)"` en 5 clientes HTTP, mientras la app va por la versión 1.0.20.
- `pnpm rust:quality:report` exige PowerShell y no corre en Linux o macOS sin `pwsh`.

### 3.7 Calidad de código y tests

#### Q-05 — Dos tests Rust fallan en Linux (rutas con semántica de Windows) · **Media**

- **Ubicación:** `src/navegador/download.rs:2043-2058` (`a_finished_folder_download_is_found_by_its_file_name_alone`) y `tests/bibliography_processing.rs:8808-8812` (`attachment_page_keeps_pdfs_and_stored_web_snapshots_with_a_parent_and_decodes_the_enclosure`).
- **Descripción:** el primero espera que `Path::new("Z:\\elsewhere\\data.zip")` tenga como nombre de archivo `data.zip` y que se compare sin distinguir mayúsculas. El segundo espera que `"C:/Libros/externo.pdf"` se reconozca como ruta absoluta. Ninguno de los dos se cumple en Linux ni en macOS, y los tests no están marcados con `#[cfg(windows)]`. La CI no lo detecta porque solo corre `cargo test` en Windows (C-01).
- **Impacto:** `cargo test` falla para cualquier persona que desarrolle en Linux o macOS. Además conviene revisar si el código de producción detrás de esos tests se comporta bien en esas plataformas (por ejemplo, adjuntos _linked_ de Zotero con rutas `/home/...`, o descargas cuyo nombre solo difiere en mayúsculas).
- **Recomendación:** marcar los casos con `#[cfg(windows)]` y agregar sus equivalentes POSIX, o hacer la lógica independiente de la plataforma del host; después incorporar Linux a la CI.

#### Q-01 — Warnings pendientes de lint y Svelte · **Baja**

- 5 warnings de ESLint y 8 de `svelte-check` (detalle en §2). En `EntropicConstellation.svelte:53`, `reducedMotion` se asigna en `onMount` pero no es `$state`, así que `class:constellation--motion={!reducedMotion}` (línea 432) nunca se actualiza. Hoy el efecto visual es nulo, porque con movimiento reducido el CSS igual queda sin animación, pero es una trampa latente: cualquier estilo futuro que dependa de esa clase va a ignorar la preferencia del usuario.
- **Recomendación:** declarar `reducedMotion` con `$state` y considerar `--max-warnings 0`.

#### Q-02 — Fragilidad en `renderMarkdown` (inline) · **Baja**

- **Ubicación:** `apps/desktop/src/lib/markdown.ts:45-55`.
- **Descripción:** las sustituciones de `**`, `*` y enlaces se aplican también _dentro_ de `<code>` y dentro del `href` ya generado (por ejemplo `[x](https://a.com/*b*)` termina con `<em>` dentro del atributo). No es explotable, porque el escape previo neutraliza `"` y `<`, pero produce enlaces rotos.
- **Recomendación:** tokenizar el código inline y los enlaces antes de aplicar énfasis.

#### Q-03 — Lógica duplicada de _listeners_ de descarga de modelos · **Info**

- `views/SettingsView.svelte:690-760` y `views/DependenciasTab.svelte:170-215` registran los mismos eventos `llm:*`, `embedding:*` y `reranker:*`. Si un `listen` lanza una excepción a mitad del array, los anteriores quedan registrados sin `unlisten`.
- **Recomendación:** extraer un helper `registerDownloadListeners()` con limpieza parcial.

#### Q-04 — `sanitizeNoteHtml` devuelve HTML sin sanear si no hay `document` · **Info**

- **Ubicación:** `packages/ui/src/components/NoteEditor/note-content.ts:216-222`. Hoy siempre hay DOM (webview o happy-dom), pero si se reutiliza en Node o SSR, sale el HTML crudo.
- **Recomendación:** escapar por defecto en esa rama.

### 3.8 Repositorio, higiene y documentación

#### R-01 — Archivos versionados que `.gitignore` declara ignorados · **Media**

- `docs/` está en `.gitignore` con el comentario _"Local research notes (not for the public repo)"_, pero `docs/superpowers/**` (29 archivos) está versionado y es público.
- `artifacts/msix/*.msix` está ignorado, pero `artifacts/msix/EntropIALite-Store-HLab-1.0.5.0.msix` (9 MB, versión vieja) está versionado.
- **Recomendación:** decidir si `docs/` debe ser público. Si no, `git rm --cached` (con revisión de lo ya publicado) y quitar el `.msix` viejo.

#### R-02 — Configuración personal de herramientas de IA en la raíz · **Baja**

- `opencode.json` incluye una ruta personal (`C:\Users\agusn\.local\bin\serena.exe`) y servidores MCP sin versión fijada (`npx -y @playwright/mcp@latest`, `figma-developer-mcp`, `@hypothesi/tauri-mcp-server`). También hay `.agents/`, `.opencode/`, `.pi/`, `.windsurf/`, `.codegraph/` y `skills-lock.json`.
- **Impacto:** quien clone y use esas herramientas ejecuta paquetes npm sin versión fijada. Además hay ruido en la raíz.
- **Recomendación:** fijar versiones y moverlo a una configuración local ignorada, o documentarlo como opcional.

#### R-03 — Raíz sobrecargada y documentación duplicada · **Baja**

- En la raíz hay `plan-editor.md` (127 KB), `plan-lote.md` (62 KB) y `odd/` (planes, reportes y tareas), además de `docs/superpowers/`. Hay dos manuales: `Manual-de-Uso.md` (452 líneas) y `manual/manual-usuario/manual-usuario.md` (1.190 líneas, el que se publica en GitHub Pages).
- **Recomendación:** mover los planes a `docs/` u `odd/`, y dejar `Manual-de-Uso.md` como enlace al manual publicado o eliminarlo.

#### R-04 — Mensajes de commit del historial superficial engañosos · **Info**

- Por el clon superficial, el primer commit visible (`de4b2f6`) aparece como _"patch tao…"_ con 1.391 archivos. Es un artefacto del _graft_, no un problema del repo, pero conviene tenerlo presente al auditar con `--depth`.

#### R-05 — Inconsistencias menores de documentación · **Info**

- `AGENTS.md` dice que el guard de `build.rs` requiere `MANIFEST_URL` y `PUBLIC_KEY_BASE64`. El README (correctamente) pide también `PUBLIC_KEY_ID`.
- El README menciona `pnpm test` para los tests de Pro; la CI usa además el job Lite (`VITE_LOCAL_ML=0`). Está bien, pero el README no dice que la CI lo corre.

---

## 4. Aspectos positivos destacados

Para equilibrar el informe, lo que está bien resuelto y conviene preservar:

- **ACL de Tauri probada con tests** (`tests/app_acl.rs`): las pestañas del Navegador y los popups no tienen capabilities, y se verifica con el entrypoint IPC real.
- **Navegador embebido** con política de URLs exhaustiva (`navegador/url_policy.rs`): bloquea IP privadas, link-local y metadata de la nube en todas sus variantes de escritura, impide bajar a `http`, usa perfiles incógnito y cuarentena de descargas.
- **Sanitización de HTML** con listas blancas en todos los `{@html}` (`ocr-rich-text.ts`, `note-content.ts`, `markdown.ts`), con escape previo y protocolos de `href` restringidos.
- **Secretos en el llavero del sistema** con migración desde texto plano, `secure_delete` y `VACUUM` posterior; redacción de secretos en `settings_get_all` y en los logs (`sanitize_field`).
- **Runtime de Pro**: manifiesto firmado con ed25519, SHA-256 del archivo completo y de cada entrada, `enclosed_name` contra zip-slip, chequeo de espacio libre y limpieza de descargas parciales.
- **Sync**: TLS obligatorio salvo en loopback (`validate_server_url`), tokens en el llavero y _dev profiles_ que desactivan sync para no contaminar archivos reales.
- **Pinning reproducible**: `rust-toolchain.toml`, `Cargo.lock`, `pnpm-lock.yaml` con `--frozen-lockfile`, y `fetch-pdfium.sh` con SHA-256.
- **Volumen y calidad de tests**: más de 4.700 tests JS, más de 25.000 líneas de tests Rust de integración y tests de contrato sobre los workflows (Pester).

---

## 5. Priorización sugerida (sin aplicar)

| Orden | IDs                                | Por qué primero                                                                   |
| ----- | ---------------------------------- | --------------------------------------------------------------------------------- |
| 1     | S-01, S-03                         | Son los que convierten un XSS en compromiso del sistema; los cambios son chicos.  |
| 2     | S-04, D-02, D-01                   | Cierran el vector de XSS más probable (pegado) y quitan dependencias abandonadas. |
| 3     | S-02, S-05                         | Endurecen el IPC SQL y el protocolo `asset` (mejor si se hace junto con A-01).    |
| 4     | D-08, C-01, Q-05, C-03, D-05, D-06 | Evitan regresiones en Lite/Linux y en la cadena de suministro.                    |
| 5     | E-02, E-01, C-02                   | Riesgos de empaquetado y disponibilidad del runtime.                              |
| 6     | A-02, P-02, P-01                   | Robustez del arranque y rendimiento a escala.                                     |
| 7     | Resto (Baja/Info)                  | Mantenibilidad e higiene.                                                         |
