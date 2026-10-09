# Plan de implementación de las mejoras de la auditoría

**Base:** `auditoria.md` (commit `0ea30b3`, 2026-10-09). Cada tarea cita el ID del hallazgo que resuelve.
**Estado:** plan propuesto, **sin aplicar**. Nada de lo descrito acá está implementado todavía.

---

## 1. Objetivo y criterios generales

Cerrar los 45 hallazgos de la auditoría en orden de riesgo, sin romper ninguna de las dos variantes (Pro y Lite) ni el sync entre dispositivos, y dejando la CI preparada para que no vuelvan a aparecer.

Reglas que valen para todas las fases:

1. **Un PR por tarea (o por grupo chico de tareas afines).** Ningún PR mezcla seguridad con refactors. Cada PR lleva en la descripción el ID del hallazgo.
2. **Primero el test que falla, después el arreglo.** Cada hallazgo de seguridad se cierra con un test que hoy fallaría (en `tests/app_acl.rs`, en los tests de `db/commands.rs` o en `settings.rs`).
3. **Las dos variantes en cada PR que toque Rust o frontend:**
   - `pnpm lint && pnpm typecheck && VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck`
   - `pnpm test && VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test`
   - `cargo clippy --all-targets -- -D warnings` y `cargo test`, sin features (Lite) y con `--features local-ml` (Pro)
4. **Nunca probar una migración nueva contra el archivo real.** Toda prueba manual de la app va con `ENTROPIA_DEV_PROFILE=<nombre>` (ver `AGENTS.md`); confirmar que la línea de arranque diga `profile=dev:<nombre> … sync=disabled`.
5. **Sync y `schema_tag`:** el `schema_tag` **no** se deriva del esquema sino del nombre máximo (lexicográfico) de `_migrations` (`apps/desktop/src-tauri/src/sync/engine.rs:864-880`). Por lo tanto, **cualquier migración numerada nueva** (`0059_…` en adelante) sube el `schema_tag` de la cuenta en el primer sync, aunque el esquema resultante sea idéntico, y los dispositivos con versiones anteriores instaladas reciben `426` y pierden el sync hasta actualizarse. Las fases que tocan migraciones (Fase 4) **no cambian el esquema resultante ni agregan filas a `_migrations`**: solo cambian _quién_ y _cómo_ lo aplica, y los parches se portan como reparaciones idempotentes. Si algún paso necesitara una migración numerada nueva, se separa en un PR propio con revisión de sync y una nota de despliegue que advierta que los dispositivos viejos quedan en `426` hasta actualizar.
6. **Tablero:** si se quiere seguir el avance en hlab.com.ar, crear una tarjeta por fase (el plan no las crea; `AGENTS.md` pide crear tarjetas solo a pedido).

Escala de esfuerzo usada: **S** ≤ medio día · **M** 1-3 días · **L** 1-2 semanas.

---

## 2. Mapa de fases

| Fase | Tema                                       | Hallazgos                                          | Esfuerzo | Depende de |
| ---- | ------------------------------------------ | -------------------------------------------------- | -------- | ---------- |
| 0    | Base: CI en Linux y verificaciones previas | C-01, Q-05, E-02 (verificación)                    | M        | —          |
| 1    | Seguridad crítica (cambios chicos)         | S-03, S-01, S-04                                   | M        | 0          |
| 2    | Dependencias vulnerables                   | D-01, D-02, D-08, D-03, D-04                       | M-L      | 0          |
| 3    | Endurecer el IPC SQL y el protocolo asset  | S-02, S-05, A-04, A-05, S-06                       | M        | 1          |
| 4    | Migraciones unificadas en Rust             | A-01, A-03, A-02                                   | L        | 3          |
| 5    | Rendimiento y observabilidad               | P-01, P-02, P-03, P-04                             | M        | 0          |
| 6    | Empaquetado y release                      | E-02 (arreglo), E-01, E-03, C-02, C-04, D-07, E-04 | M        | 0          |
| 7    | Cadena de suministro en la CI              | C-03, D-05, D-06, C-05                             | S-M      | 0          |
| 8    | Calidad, higiene y documentación           | Q-01..Q-04, R-01..R-05, A-06, S-07, C-06           | S-M      | —          |

Las fases 1, 2, 5, 6 y 7 pueden avanzar en paralelo una vez cerrada la Fase 0. La 3 conviene después de la 1 (comparten tests de ACL) y la 4 después de la 3 (la 4 es la que permite cerrar el DDL que la 3 deja acotado).

```
0 ──┬── 1 ── 3 ── 4
    ├── 2
    ├── 5
    ├── 6
    └── 7
8 (en cualquier momento, PRs chicos)
```

---

## 3. Fases en detalle

### Fase 0 — Base: CI en Linux y verificaciones previas

Va primero porque varios arreglos posteriores tocan código `cfg(not(feature = "local-ml"))` y Linux/macOS, que hoy la CI no compila ni prueba.

#### 0.1 Arreglar los dos tests Rust que fallan en Linux (Q-05) · S

- **Archivos:** `apps/desktop/src-tauri/src/navegador/download.rs:2043-2058`, `apps/desktop/src-tauri/tests/bibliography_processing.rs:8808-8812`.
- **Pasos:**
  1. Leer el código de producción que ejercitan (`Registry::take_by_path` y la decodificación de `data.path` de adjuntos _linked_ de Zotero) y decidir, caso por caso, si el comportamiento esperado es propio de Windows o debería valer en todas las plataformas.
  2. Si es propio de Windows (rutas `Z:\…`, `C:/…`, nombres sin distinguir mayúsculas): marcar el caso con `#[cfg(windows)]` y agregar el equivalente POSIX (`/home/ana/Libros/externo.pdf`, nombres que distinguen mayúsculas).
  3. Si el código de producción también falla en Linux/macOS (por ejemplo, si Zotero en Linux guarda rutas absolutas `/…` y no se reconocen), arreglar la lógica y no solo el test.
- **Aceptación:** `cargo test` sin features pasa completo en Linux.

#### 0.2 Job de CI para Rust en Linux, variante Lite (C-01) · S

- **Archivo:** `.github/workflows/ci.yml`.
- **Pasos:**
  1. Nuevo job `rust-lite-linux` en `ubuntu-22.04` (la misma imagen que usa el release), con las dependencias de sistema que ya instala `release.yml` (`libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `pkg-config`) y `Swatinem/rust-cache`.
  2. Pasos: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` y `cargo test`, todo **sin features**.
  3. Engancharlo a la misma condición `detect-rust-changes` que los jobs de Windows.
  4. (Opcional, después) Un job macOS solo de `cargo check`, porque compilar en macOS es caro.
- **Aceptación:** el job corre en verde en un PR que toque un `.rs`; un error introducido a propósito en código `#[cfg(not(feature = "local-ml"))]` lo hace fallar.

#### 0.3 Verificar el _fixture_ de `libpdfium.so` en Pro Linux (E-02) · S

- **Pasos:**
  1. Generar el `.deb` de Pro (`release.yml` con `release_platform: all`, o build local) e instalarlo en una VM Ubuntu 22.04 limpia.
  2. **Sin** descargar el runtime, abrir la Biblioteca y leer un PDF de un adjunto de Zotero. Registrar si falla al cargar Pdfium.
  3. Anotar el resultado en el PR de la Fase 6 (6.1), que decide el arreglo.
- **Aceptación:** está documentado si el problema existe o no, con evidencia (log de `app_logs` o captura).

---

### Fase 1 — Seguridad crítica (cambios chicos)

Son los tres cambios que impiden que un XSS escale. Son chicos y se pueden revertir con facilidad.

#### 1.1 Que el renderer no pueda pisar la fuente de confianza del runtime (S-03) · S

- **Archivos:** `apps/desktop/src-tauri/src/settings.rs` (`settings_set`, `settings_delete`, `get_runtime_bootstrap_remote_source_with_builtin`, `get_runtime_bootstrap_public_key_with_builtin`).
- **Pasos:**
  1. **Test primero:** en `settings.rs`, un test que llama a la lógica de `settings_set` con `runtime_bootstrap_manifest_url`, `runtime_bootstrap_public_key_id` y `runtime_bootstrap_public_key.<id>` y espera **error**.
  2. Agregar una función `is_renderer_writable_setting(key)` usada por `settings_set` y `settings_delete`. Recomendado: **denylist explícita** de prefijos internos (`runtime_bootstrap_`; 1.2 paso 5 agrega `backend_grant.` y el nombre legado `zotero_data_dir`), porque la UI escribe ~80 claves (`lib/settings.ts`) y una allowlist completa es más frágil. Dejar anotada como decisión abierta la allowlist estricta (§4).
  3. En builds release (`#[cfg(not(debug_assertions))]`), que la fuente compilada (`option_env!`) **tenga prioridad** sobre `app_settings` cuando está definida. En debug se puede mantener la anulación para probar manifiestos de staging.
  4. Que la clave pública guardada en `app_settings` solo se acepte si su `id` **no** coincide con el `id` compilado: nunca reemplazar la clave oficial.
- **Tests:** los del paso 1, más los existentes `configured_bootstrap_catalog_*` adaptados a la nueva precedencia.
- **Aceptación:** con `--features local-ml`, no hay forma de cambiar la URL ni la clave usada en release desde IPC.
- **Riesgo:** algún instalador viejo con valores en `app_settings`. Mitigación: al arrancar, loguear (sin borrar) si existen esas claves.

#### 1.2 Acotar el scope del plugin `fs` (S-01) · M

- **Archivos:** `apps/desktop/src-tauri/capabilities/default.json`, `apps/desktop/src-tauri/tests/app_acl.rs`, y los usos de `@tauri-apps/plugin-fs` en `apps/desktop/src` (`lib/file-import.ts`, `lib/collection-import.ts`, `lib/export-images.ts`, `lib/writing-export.ts`, `lib/rag-chat-export.ts`, `views/CollectionAnalysisPanel.svelte`, `views/BatchSchemaPanel.svelte`, `views/CollectionView.svelte`, `views/CollectionsView.svelte`, `layout/WorkPane.svelte`).
- **Pasos:**
  1. **Inventario:** para cada llamada a `readFile`, `stat`, `copyFile`, `writeFile`, `remove` y `mkdir`, anotar de dónde sale la ruta: (a) un diálogo `open`/`save`, (b) el directorio de datos (`resolve_data_dir`), (c) otra fuente (importar carpeta, rutas guardadas de Zotero, `source_directory` de un ítem). El caso (c) es el que hoy depende de `*-read-recursive`.
  2. **Tests primero** en `tests/app_acl.rs`: `plugin:fs|write_file`, `plugin:fs|remove` y `plugin:fs|read_file` sobre una ruta de `$HOME` fuera de `com.entropia.shared` deben fallar con error de scope; sobre `$DATA/com.entropia.shared/assets/...` deben andar; y `plugin:fs|read_file`, `plugin:fs|write_file` y `plugin:fs|remove` sobre `$DATA/com.entropia.shared/entropia.sqlite` (y `-wal`/`-shm`) deben fallar con error de scope.
  3. Quitar `fs:allow-home-read-recursive`, `fs:allow-desktop-read-recursive`, `fs:allow-document-read-recursive` y `fs:allow-download-read-recursive`. Las rutas del caso (a) siguen funcionando porque `tauri-plugin-dialog` las agrega al scope en tiempo de ejecución.
  4. Acotar también el `fs:scope` global (`capabilities/default.json:243-249`): hoy `$DATA/com.entropia.shared/**/*` y `$LOCALDATA/com.entropia.shared/**/*` cubren `entropia.sqlite` con `read_file`/`write_file`/`remove` permitidos, así que un renderer comprometido puede leer, pisar o borrar la base. Reemplazar `**/*` por los subdirectorios concretos que salgan del inventario del paso 1 (por ejemplo `…/assets/**`, `…/thumbnails/**`, `…/writing/**`) y agregar `"deny"` explícito para `**/*.sqlite*` en ambos roots. Usar la misma lista de subdirectorios que 3.3 para que `fs` y `asset` queden alineados.
  5. **Concesión del directorio de Zotero controlada por el backend (prerrequisito de la opción (ii) del paso 6).** Hoy `zotero_data_dir` (`bibliography/processing.rs:2613`) es una fila común de `app_settings` que `settings_set` (`settings.rs:105-119`) deja escribir al renderer, y la denylist de 1.1 solo cubre `runtime_bootstrap_*`: tratarla como raíz autorizada permitiría que un renderer comprometido fije `zotero_data_dir` a cualquier carpeta y reabra la lectura de los archivos soportados que cuelguen de ella. Entonces: (a) cambiar el valor de `ZOTERO_DATA_DIR_SETTING_KEY` a `backend_grant.zotero_data_dir` (todos los lectores lo consultan por la constante: `bibliography/commands.rs:211,585`, `bibliography/reprocess.rs:871`, `bibliography/processing.rs:3034`, `processing/repository.rs:2517`); (b) agregar a la denylist de `is_renderer_writable_setting` (1.1 paso 2) el prefijo `backend_grant.` **y** el nombre legado `zotero_data_dir`, de modo que `settings_set` y `settings_delete` los rechacen; (c) la única vía de escritura es un comando Rust dedicado (por ejemplo `zotero_data_dir_grant`) que **no recibe ninguna ruta del renderer**: abre el selector de carpeta nativo desde Rust (`tauri-plugin-dialog`, `blocking_pick_folder` dentro de `spawn_blocking`), canonicaliza la carpeta elegida, comprueba que contiene `storage/` o `zotero.sqlite` y la guarda con `persist_setting` desde el backend; (d) un valor legado en `zotero_data_dir` **no** se migra ni se honra como raíz (pudo plantarse antes del parche): se loguea sin borrar y el usuario debe concederla de nuevo. Hoy ningún componente de la UI escribe esa clave (el doc-comment de la constante dice «no UI binds it yet»), así que no hay flujo visible que se rompa. Registrar el comando igual que el del paso 6 (`build.rs`, capability y `generate_handler!`). Tests: `settings_set` y `settings_delete` con `zotero_data_dir` y con `backend_grant.zotero_data_dir` devuelven error; con una fila legada `zotero_data_dir` presente, la raíz concedida es `None`; y el test de integración «fijar `zotero_data_dir` por IPC e intentar importar un PDF no autorizado bajo esa carpeta» falla en el `settings_set` y, aunque la fila legada exista, el comando de importación del paso 6 devuelve error para ese PDF.
  6. Para el caso (c), mover la lectura a un comando Rust dedicado (por ejemplo `import_copy_into_archive(src, dest_rel)`) que valide extensión y tamaño, siga symlinks con cuidado (patrón `cap-std` ya usado en el repo) y escriba solo dentro del archivo. El comando **no acepta rutas arbitrarias del renderer**: `src` debe provenir de (i) una ruta concedida por `tauri-plugin-dialog` en esta sesión (consultar el scope del plugin desde Rust), (ii) una raíz concedida por el backend: el directorio de datos de Zotero **solo** si se concedió con el selector nativo del paso 5 (`backend_grant.zotero_data_dir`), nunca el valor de `zotero_data_dir` ni ninguna otra clave que `settings_set` o `db_execute*` puedan escribir, o (iii) una ruta que **el propio backend** resolvió en Rust a partir de los datos de Zotero (la respuesta de la API local de Zotero o `zotero.sqlite`, vía `resolve_attachment_file` en `bibliography/attachment.rs:40`), **nunca** a partir de filas de `entropia.sqlite`: `items.source_directory`, el `data.path` de un adjunto y cualquier otro registro de la base de la app son escribibles por el renderer mediante el IPC SQL (`db_execute*`), así que confiar en ellos reabre la lectura arbitraria. Un `source_directory` guardado solo se honra si cae bajo una raíz autorizada de (ii) o fue concedido por diálogo según (i) en esta sesión; si `src` no cae en ninguna de las tres fuentes, devuelve error. Sin esa condición, un XSS podría copiar al archivo cualquier PDF/TXT de ruta conocida y leerlo, reintroduciendo la lectura arbitraria. Registrar el comando en `build.rs` (`APP_COMMANDS`), en la capability **y** en `tauri::generate_handler!` (`apps/desktop/src-tauri/src/lib.rs:1281`); sin esto último, los flujos migrados fallan con _command not found_. Tests (ACL y de comando): el comando con un `src` fuera de las tres fuentes anteriores devuelve error; una ruta fuera de toda raíz autorizada inyectada vía `db_execute` en `items.source_directory` (o en el `data` de un adjunto) y luego pasada al comando también devuelve error; y un PDF bajo una carpeta que el renderer intentó fijar con `settings_set("zotero_data_dir", …)` devuelve error.
  7. Revisar `fs:allow-download-write`: si se usa solo para exportar con diálogo `save`, también se puede quitar.
- **Aceptación:** pasan los tests ACL nuevos (incluidos los de `entropia.sqlite` y los del comando de importación) y siguen funcionando las importaciones (archivo suelto, carpeta, desde Zotero), los exports y la eliminación de colecciones e ítems. Verificar a mano en un dev profile, en Windows y en Linux.
- **Riesgo:** medio; puede romper un flujo de importación no inventariado. Mitigación: el inventario del paso 1 va en la descripción del PR, y se prueba a mano cada flujo.

#### 1.3 Parchear el XSS de pegado de ProseMirror (S-04) · S

- **Archivos:** `package.json` (raíz) y `pnpm-lock.yaml`.
- **Pasos:**
  1. Agregar a la raíz `"pnpm": { "overrides": { "prosemirror-view": ">=1.42.3" } }` (o actualizar `@tiptap/pm` si una 2.x reciente ya la trae).
  2. `pnpm install` para regenerar el lock; nunca editarlo a mano.
  3. Correr los tests del editor (`packages/ui/src/components/WritingEditor/**`, `NoteEditor/**`), en especial `writing-image-paste.test.ts`.
- **Aceptación:** `pnpm audit --prod` deja de reportar `prosemirror-view`; los tests del editor pasan; se puede pegar a mano HTML de una web y de Word.

---

### Fase 2 — Dependencias vulnerables

#### 2.1 Reemplazar `html-docx-js` por `docx` (D-01) · M

- **Archivos:** `apps/desktop/src/lib/ocr-export.ts`, `apps/desktop/src/types/ocr-export-libraries.d.ts`, `apps/desktop/package.json`; reutilizar lo que ya existe en `apps/desktop/src/lib/export-docx.ts`.
- **Pasos:**
  1. Fijar el comportamiento actual con un test de exportación DOCX del OCR (estructura: párrafos, encabezados, tablas e imágenes del OCR enriquecido) usando `fflate` para abrir el `.docx`, como ya hacen otros tests.
  2. Implementar la conversión HTML del OCR → modelo `docx` (o pasar por el mismo AST que usa `export-docx.ts`, si es compatible).
  3. Eliminar la carga por `<script>` inyectado (`ocr-export.ts:140-185`) y la dependencia.
- **Aceptación:** el test del paso 1 pasa con la implementación nueva; `pnpm audit --prod` ya no muestra `lodash.merge` ni `jszip`.

#### 2.2 Actualizar las dependencias JS con avisos (D-02) · M

- **Pasos (un PR por grupo):**
  1. `svelte` ≥ 5.55.7 y `devalue` ≥ 5.9.3 (override si hace falta). Correr las tres suites de tests y `svelte-check`.
  2. `markdown-it` ≥ 14.3.1 y `linkify-it` ≥ 5.0.2 (override).
  3. `drizzle-orm` 0.40 → ≥ 0.45.2: leer el changelog por cambios en `sqlite-proxy`; correr `packages/store` completo.
  4. Tiptap 2 → 3 (`@tiptap/core` ≥ 3.30.4): es una **migración mayor**. Va en un PR propio, con lectura de la guía de migración y revisión de las extensiones propias de `packages/ui` (footnotes, imágenes, citas). Si no entra en este ciclo, documentar el riesgo aceptado (el aviso es de severidad media).
- **Aceptación:** `pnpm audit --prod --audit-level high` sin hallazgos.

#### 2.3 Actualizar dependencias Rust con avisos (D-08) · M-L

- **Pasos:**
  1. `cargo update -p rustls -p crossbeam-epoch -p quick-xml -p quinn-proto` (cambios menores de versión; `quinn-proto 0.11.14` es la sexta vulnerabilidad de `cargo audit` según `auditoria.md:50,139`: está en el lock vía `quinn 0.11.9` pero no en el árbol activo de Linux, así que si `-p quinn-proto` no la mueve, actualizar su padre `quinn`). Correr clippy y tests en Lite y Pro.
  2. `imageproc` 0.25 → la versión corregida; revisar los usos (`image_edit.rs`, OCR).
  3. **`pdf-extract` / `lopdf 0.34`:**
     - Ver si hay una versión de `pdf-extract` que use `lopdf ≥ 0.42`. Si existe, actualizar.
     - Si no, aislar la extracción: correrla en un hilo con pila propia (`std::thread::Builder::stack_size`) **y** con un límite de profundidad. Como un _stack overflow_ aborta el proceso y no se puede atrapar, la protección real es correrla en un **subproceso**. Hoy no existe ningún patrón de subcomando en producción: lo único parecido (`llm/mod.rs:3100-3118`) es un `#[cfg(feature = "local-ml")] #[test]` que relanza el binario de tests con `--exact`. Hace falta un **punto de entrada de producción nuevo** (por ejemplo `entropia --extract-pdf <ruta>` detectado en `main.rs` antes de arrancar Tauri, o un sidecar) y un protocolo mínimo (ruta de entrada por argumento, texto por `stdout`, código de salida y _timeout_ del lado del padre). Esto sube el esfuerzo de esta subtarea a **M-L**. Evaluar ambas opciones con un PDF de prueba con objetos anidados.
     - Agregar ese PDF como _fixture_ de test.
  4. Revisar `anyhow`, `rand`, `glib` (_unsound_) y las crates sin mantenimiento: anotar cuáles vienen de dependencias propias y cuáles de Tauri.
- **Aceptación:** `cargo audit` sin vulnerabilidades (los warnings que quedan, justificados en `.cargo/audit.toml` con comentario).

#### 2.4 Limpiar dependencias sin uso o duplicadas (D-03) · S

- Quitar `@tauri-apps/plugin-sql` de `apps/desktop/package.json`.
- Que las versiones de `@tiptap/*` y `leaflet` vivan en un solo lugar: o `packages/ui` las declara y `apps/desktop` no, o se usa un [catálogo de pnpm](https://pnpm.io/catalogs) (requiere pnpm ≥ 9.5, compatible con 9.15).
- Documentar en `AGENTS.md` por qué conviven `lib/markdown.ts` y `markdown-it`.
- **Aceptación:** `pnpm build` y los tests pasan; `pnpm why` muestra una sola versión de cada `@tiptap/*`.

#### 2.5 Actualizar el tooling de test (D-04) · S-M

- `vitest` ≥ 3.2.6 (o 4.x), `@vitest/ui`, `@vitest/coverage-v8`, `happy-dom` ≥ 20, `vite` ≥ 6.4.3.
- `happy-dom` 17 → 20 es un salto grande: correr las tres suites y revisar los tests que dependen de layout o de APIs DOM específicas. Aprovechar para decidir si se unifica `jsdom` vs `happy-dom` (hay 22 archivos que fuerzan `jsdom`).
- **Aceptación:** `pnpm audit` sin _critical_.

---

### Fase 3 — Endurecer el IPC SQL y el protocolo `asset`

#### 3.1 Authorizer de SQLite en la conexión de UI (S-02) · M

- **Archivos:** `apps/desktop/src-tauri/src/db/commands.rs`, `db/open.rs`, `db/state.rs`.
- **Diseño:** en lugar de seguir parcheando el análisis de texto, registrar un _authorizer_ con `rusqlite::Connection::authorizer` (feature `hooks` de rusqlite) **solo mientras se ejecuta una sentencia que viene del renderer**. Se activa al entrar a `db_execute*`/`db_select*` y se desactiva al salir, sobre el mismo `Mutex`. El authorizer niega:
  - `SQLITE_ATTACH`, `SQLITE_DETACH` y `SQLITE_PRAGMA`, salvo una lista explícita: los pragmas de solo lectura que use el frontend y, **mientras exista el runner TS (hasta 4.4)**, `PRAGMA defer_foreign_keys`, que las migraciones 0045/0048/0050/0052/0058 ejecutan (`packages/store/src/runner.ts:1606,1880,2119,2200,2697`); sin esa excepción, una instalación nueva o una actualización pendiente fallaría al migrar. Es el único pragma de escritura que emite el runner (verificar con `grep PRAGMA packages/store/src`). 4.4 lo quita de la lista;
  - cualquier lectura o escritura sobre la tabla `app_settings` (`SQLITE_READ` / `SQLITE_INSERT` / `SQLITE_UPDATE` / `SQLITE_DELETE` con `table == "app_settings"`);
  - escrituras sobre tablas `sync_*` y creación o borrado de triggers `trg_sync_*`, **excepto** los accesos cuyo quinto argumento del authorizer (el trigger o vista de origen) sea un trigger `trg_sync_*`: la captura de sync se implementa con esos triggers, que hacen `INSERT INTO sync_oplog` en cada escritura capturada cuando `capture_enabled='1'` (`sync/capture.rs:104-112`), y SQLite invoca al authorizer también para las sentencias del cuerpo del trigger. Sin la excepción, **toda escritura del renderer sobre un archivo con sync fallaría**;
  - `SQLITE_CREATE_TRIGGER`, `SQLITE_CREATE_TEMP_TRIGGER`, `SQLITE_CREATE_VIEW` y `SQLITE_CREATE_TEMP_VIEW` desde el renderer, **salvo** las sentencias exactas de la lista de operaciones de migración conocidas y solo dentro de la ventana de migración de un solo uso que controla el backend (ver paso 3); la ventana sola no alcanza. El cuerpo de un trigger se autoriza cuando el trigger se dispara, no cuando se prepara el `CREATE TRIGGER`; si se permitiera, el renderer podría plantar un trigger con otro nombre sobre una tabla común que copie `app_settings` o escriba `sync_*`, y una escritura del backend fuera del authorizer lo dispararía después;
  - `VACUUM` (el authorizer no tiene código propio para `VACUUM INTO`; se niega vía `SQLITE_ATTACH`, que es como lo implementa SQLite; **verificarlo con un test**).
- **Pasos:**
  1. **Tests primero** (en `db/commands.rs`) con las evasiones de la auditoría: `/**/VACUUM INTO '…'`, `--x\nATTACH …`, `INSERT INTO sync_oplog … RETURNING *` vía `db_select`, `WITH … INSERT INTO sync_meta …`, `UPDATE OR REPLACE sync_meta …`, y `SELECT * FROM app_settings` con comentarios intercalados.
  2. Implementar el authorizer y mantener los validadores de texto actuales como primera capa, con mensajes de error claros.
  3. Mientras el runner de migraciones TS siga existiendo (hasta la Fase 4), el authorizer **permite DDL de tablas e índices** sobre tablas que no sean `sync_*` ni `app_settings`. Los `CREATE TRIGGER`/`CREATE VIEW`, los `DROP TRIGGER`/`DROP VIEW` y `PRAGMA defer_foreign_keys` quedan permitidos **solo** si se cumplen **las dos** condiciones siguientes; la ventana por sí sola nunca autoriza nada, porque el renderer puede invocar `begin` y es quien escribe el SQL que corre mientras la ventana está `Open`:
     - **Lista exacta de operaciones conocidas (es la autoridad real).** El backend embebe la lista de huellas SHA-256 de cada sentencia de ese tipo que emiten las migraciones conocidas (los `CREATE TRIGGER`/`CREATE VIEW`, el `DROP TRIGGER IF EXISTS` de la 0032 y `PRAGMA defer_foreign_keys`), con el texto normalizado solo en espacios en blanco inicial/final y `;` final (**sin** quitar comentarios: un comentario inicial o intercalado cambia la huella y la sentencia se niega). La lista incluye también los `DROP TRIGGER IF EXISTS` de reparación que `runMigrations` antepone a la 0032 (`repairDrops`, `runner.ts:3102-3105`). La lista se genera con **un único divisor sin preparar**, `split_sql_statements` (en Rust, ver más abajo): el script (por ejemplo `packages/store/scripts/export-migration-ddl-allowlist.mjs`, junto a `export-schema.mjs`) solo exporta el SQL de `MIGRATIONS` (y las sentencias de `repairDrops`) y delega el corte y el cálculo de huellas en esa función Rust (por ejemplo vía un test `#[ignore]` de regeneración), sin reimplementar el divisor en JS; el resultado se versiona como `src-tauri/src/db/migration_ddl_allowlist.txt`. Un test Rust de consistencia vuelve a partir las migraciones exportadas con **esa misma función** y exige que la lista coincida, de modo que cambiar `runner.ts` sin regenerarla rompe la CI. **Orden obligatorio en `db_execute_batch`:** (i) partir el lote en sentencias **sin preparar SQL**: `split_sql_statements` es un divisor léxico que respeta literales de cadena, identificadores entrecomillados, comentarios y cuerpos `BEGIN…END` de triggers (o, equivalente, acumula hasta que `sqlite3_complete` devuelva verdadero); queda **prohibido** usar `rusqlite::Batch` o cualquier `prepare` para descubrir los límites, y **prohibido** desactivar el authorizer para prepararlas, porque SQLite invoca al authorizer durante la preparación (`Batch::next()` llama a `conn.prepare`) y un trigger legítimo se negaría antes de que su huella pudiera autorizarlo; (ii) calcular la huella de cada sentencia y comprobarla contra la lista **antes de tocar SQLite**; (iii) si está en la lista, activar la excepción acotada de esa sentencia (solo el código de acción y el objeto que ella declara, con un guard RAII) **antes** de prepararla, mantenerla durante su ejecución y limpiarla al terminar, también si falla; (iv) las sentencias fuera de la lista (incluidas `BEGIN IMMEDIATE`, `COMMIT` y el `INSERT INTO _migrations` que envuelven cada migración, `runner.ts:3142-3145`) se ejecutan una por una bajo el authorizer normal, sin excepción. Una sentencia `CREATE TRIGGER`/`CREATE VIEW`/`DROP TRIGGER`/`DROP VIEW`/`PRAGMA` cuya huella no está en la lista se niega aunque la ventana esté `Open`, incluso si lleva el nombre de un trigger legítimo. No hay ningún parámetro «del runner» que el backend deba creer: el runner TS envía las mismas sentencias de siempre y el backend decide por el contenido.
       **Cobertura de todas las rutas IPC del runner** (`runner.ts`, `tauri-db-client.ts:13-20`): el runner usa `db_execute_batch` (la tabla `_migrations`, el lote `BEGIN IMMEDIATE; [repairDrops] <migración> INSERT … COMMIT;` de las migraciones de la lista fija de `runner.ts:3111-3137` (0025, 0027, 0029, 0032, 0034-0036, 0038, 0040-0058; ahí están todas las que crean triggers) y las sentencias sueltas del resto tras `splitStatements`, y `ROLLBACK;`), `db_execute` (`INSERT INTO _migrations`, solo DML) y `db_select` (lecturas de `_migrations` y `sqlite_master`). Solo `db_execute_batch` puede levantar la excepción; `db_execute`, `db_select*`, `db_execute_transaction` y `db_browser_*` **nunca** la levantan, con la ventana `Open` o no (los `CREATE TRIGGER`/`CREATE VIEW` ya los rechazan los validadores de texto de `db_execute*` y el authorizer es la segunda capa). Como `splitStatements` de `runner.ts:3026` parte por `;` y fragmentaría los cuerpos de trigger, las migraciones con triggers deben seguir viajando completas por ese lote (así la 0032 histórica no se vuelve a romper); el test de consistencia comprueba que toda sentencia de la lista llega a `db_execute_batch` intacta por alguna de esas dos rutas.
     - **Ventana de migración de un solo uso, impuesta por el backend** (defensa en profundidad y acotación temporal). Máquina de estados en `AppDbState` (`NotStarted → Open → Closed`, nunca de vuelta a `Open`). El backend acepta `begin` **una sola vez por proceso**, solo desde `NotStarted` y antes de que la UI se considere lista (cualquier `db_execute*`/`db_select*` recibido en `NotStarted` pasa el estado a `Closed` sin abrir la ventana, porque la UI ya está operando sin migrar); `end` o cualquier error dentro de la ventana pasan a `Closed`, y todo `begin` posterior devuelve error. **No** se usa _timeout_: las migraciones TS no son todas atómicas (A-03), y cerrar la ventana a mitad de una migración larga en un archivo grande dejaría una migración aplicada a medias. Si el frontend nunca llama a `end`, el backend cierra la ventana por sí mismo cuando `_migrations` contiene el nombre de la última migración conocida (`0058_processing_ner_tasks`), comprobado tras cada lote en estado `Open`. Fuera de la ventana todo lo anterior se niega.
       Tests Rust (en `db/commands.rs`): un segundo `begin` (o un `begin` después de `end`) falla; con la ventana `Open` y mientras corre una migración legítima, un `CREATE TRIGGER` arbitrario (por ejemplo uno que haga `UPDATE sync_meta SET value='0' WHERE key='capture_enabled'`) es **denegado**; lo mismo para un `CREATE TRIGGER collection_activity_items_update …` con el nombre legítimo pero otro cuerpo, y para la sentencia legítima precedida de un comentario `/* x */`; un renderer que llama a `begin` en `NotStarted` e inyecta un trigger propio también es denegado; y una instalación nueva migra completa con la lista exacta. Un `CREATE TRIGGER` vía `db_execute_batch` después de cerrar la ventana es denegado. Tests agregados por la corrección del divisor (todos en `db/commands.rs`): (a) `split_sql_statements_does_not_prepare`: el divisor corta correctamente literales con `;`, identificadores entrecomillados, comentarios y cuerpos `BEGIN…END` (incluido el trigger de `runner.ts:554-559`), y sobre una conexión con un authorizer que niega todo y cuenta llamadas el contador queda en **cero** (prueba que no prepara); (b) `fresh_install_migrates_under_authorizer`: sobre una base vacía, con el authorizer activo y la ventana `Open`, se reproduce la secuencia IPC exacta del runner (lote de `_migrations`, lote `BEGIN IMMEDIATE … COMMIT` por migración, sentencias sueltas del resto, `db_execute` del registro) sobre todo el `MIGRATIONS` exportado y se verifica que termina en `0058_processing_ner_tasks` y que existen los triggers legítimos (por ejemplo `rag_chunks_fts_insert`); (c) `unlisted_create_trigger_in_migration_batch_is_denied`: el mismo lote de una migración legítima con un `CREATE TRIGGER` extra no listado (por ejemplo uno que haga `UPDATE sync_meta SET value='0' WHERE key='capture_enabled'`) falla con denegación del authorizer, el trigger extra no existe y el lote hace `ROLLBACK`; (d) `exception_is_cleared_after_listed_statement`: tras una sentencia listada que falla o termina, la excepción queda apagada (el siguiente `CREATE TRIGGER` no listado se niega); (e) `non_batch_ipc_never_lifts_exception`: con la ventana `Open`, `db_execute`, `db_select` y `db_execute_transaction` niegan un `CREATE TRIGGER` aunque su texto esté en la lista. La auditoría de definiciones canónicas de 4.4 queda como red de contención. La Fase 4 cierra todo el DDL.
  4. **Test Rust con captura activa:** sobre una base con los triggers `trg_sync_*` instalados y `sync_meta.capture_enabled='1'`, una escritura vía `db_execute*` con el authorizer activo debe producir su fila en `sync_oplog`. Las suites JS con IPC simulado y un dev profile (sync desactivado) **no cubren este caso**; solo lo cubre este test.
- **Aceptación:** pasan todos los tests de evasión y el test de captura del paso 4; una instalación nueva en un dev profile migra completa (ejercita `PRAGMA defer_foreign_keys` bajo el authorizer); la suite completa de store y desktop sigue verde (esas suites ejercitan los repos reales contra el mock de IPC; además correr la app en un dev profile y recorrer las vistas principales).

#### 3.2 Rollback del lado de Rust y transacciones atómicas (A-04) · S-M

- **Archivos:** `db/commands.rs` (`db_execute_batch`), `packages/store/src/repos/item.repo.ts:1513-1563`, `asset.repo.ts:~383` y `collection.repo.ts:~141`.
- **Pasos:**
  1. En `db_execute_batch`: si `execute_batch` falla y `conn.is_autocommit()` es `false`, ejecutar `ROLLBACK` **antes de soltar el lock**. Así nunca queda una transacción abierta entre dos llamadas IPC.
  2. Migrar `deleteWithCascade` y las otras dos cascadas a `db_execute_transaction` (ya existe, es atómico y usa parámetros), eliminando la interpolación `replace(/'/g, "''")`.
  3. Meter `DELETE FROM vec_assets WHERE item_id = ?` dentro de la misma transacción (con un chequeo previo de que la tabla exista, para que no falle).
- **Tests:** un test Rust que provoque un fallo a mitad de un batch con `BEGIN` y verifique `is_autocommit()` después; los tests de cascada existentes en `item.repo.test.ts` y `asset.repo.test.ts`.

#### 3.3 Acotar el protocolo `asset` (S-05) · M

- **Archivos:** `apps/desktop/src-tauri/tauri.conf.json:57` (y los overlays `tauri.lite.conf.json:9` y `tauri.dev.conf.json:7`, que repiten `assetProtocol`), y `apps/desktop/src-tauri/src/lib.rs:758-766`.
- **Pasos:**
  1. Inventariar qué rutas pide el frontend vía `convertFileSrc`/`asset:` (assets de colecciones, miniaturas, PDFs de la Biblioteca, imágenes de Escritura, capturas del Navegador). Incluir el segundo root del scope, `$LOCALDATA/com.entropia.shared/**/*`: los PDFs del Navegador se sirven desde la caché (`<cache>/navegador/downloads`, `navegador/download.rs:475-477`) con `convertFileSrc` en `views/NavegadorPdfViewer.svelte:133`; si se acota solo `$DATA`, o se quita `$LOCALDATA` sin reemplazo, el visor de PDFs del Navegador deja de funcionar.
  2. Reemplazar `$DATA/com.entropia.shared/**/*` por los subdirectorios concretos (por ejemplo `…/assets/**`, `…/thumbnails/**`, `…/writing/**`) y `$LOCALDATA/com.entropia.shared/**/*` por los suyos (al menos `…/navegador/downloads/**`), y agregar `"deny"` explícito para `**/*.sqlite*` y `**/web-captures/**/*.html`.
  3. Acotar también la concesión **dinámica**: `setup` hace `asset_protocol_scope().allow_directory(&app_dir, true)` y `allow_directory(&cache_dir, true)` (`lib.rs:758-766`), que vuelve a habilitar los dos directorios completos en tiempo de ejecución y deja sin efecto cualquier recorte estático. Cambiarlo por `allow_directory` sobre los mismos subdirectorios del paso 2 (y `forbid_file` para `entropia.sqlite*`), de modo que la lista viva en un solo lugar (el comentario de `lib.rs` ya pide eso).
  4. Mantener los tres archivos de configuración sincronizados y agregar un test (en `runtime-packaging.test.ts` o similar) que verifique que el scope es igual en los tres. Ese test **no detecta** la concesión dinámica del paso 3; para eso, un test Rust que, tras `setup`, consulte `asset_protocol_scope().is_allowed(<data>/entropia.sqlite)` y espere `false`.
- **Aceptación:** la app muestra imágenes, PDFs, audio y miniaturas en un dev profile, **y abre un PDF descargado en el Navegador**; `fetch(convertFileSrc('<data>/entropia.sqlite'))` desde la consola de devtools falla.
- **Nota:** el acceso por `plugin:fs|read_file`/`write_file`/`remove` al `.sqlite` se cierra en 1.2 paso 4, que acota el `fs:scope` a estos mismos subdirectorios y niega `**/*.sqlite*`; usar la misma lista en ambos lugares.

#### 3.4 Comandos `async` que bloquean y keyring con el lock tomado (A-05, S-06) · S-M

- **Archivos:** `settings.rs` (`settings_get`, `settings_set`, `settings_get_all`, `settings_delete`), `writing/publish.rs` y otros comandos que toman `ui_conn.lock()` en un `async fn` (grep `ui_conn` + `lock()` fuera de `spawn_blocking`).
- **Pasos:**
  1. Llevarlos a `run_blocking_db_task` (o `spawn_blocking`).
  2. Separar `get_setting` en dos pasos: leer la referencia con el lock tomado, soltarlo y recién ahí resolver el llavero.
  3. Hacer lo mismo con las escrituras y los borrados: `settings_set` llama a `persist_setting` (que escribe en el llavero) con `ui_conn` tomado (`settings.rs:105-119`) y `settings_delete` llama a `delete_secret` bajo el mismo lock (`settings.rs:270-274`). Hoy `persist_setting_with` (`settings.rs:452-475`) escribe el secreto con `store(key, value)?` **antes** de la fila de referencia y, si el llavero falla, devuelve error sin tocar la fila. Conservar ese orden y esa semántica, pero sin el lock: en `settings_set`, para una clave secreta, escribir primero el secreto en el llavero **sin `ui_conn` tomado** (llavero con error → el comando devuelve error y la fila queda intacta), y recién después tomar el lock para `forget_zotero_user_id_for`, la fila de referencia y `resume_work_after_setting_change`. En `settings_delete`: borrar la fila con el lock, soltarlo y recién ahí llamar a `delete_secret`, logueando si falla sin revertir la fila (eso sí es el comportamiento actual, `settings.rs:270-274`; igual que el borrado por valor vacío de `settings.rs:464-466`).
  4. **Serializar por clave toda la secuencia base de datos + llavero, sin retener `ui_conn` durante la E/S.** Hoy `settings_set` y `settings_delete` son atómicos entre sí porque ambos corren completos bajo `ui_conn`; `APP_CREDENTIAL_LOCK` (`settings.rs:47`, usado en `store_secret`, `read_secret` y `delete_secret`) protege solo cada llamada individual al llavero. Con el orden del paso 3 y sin otro candado, `settings_set` puede guardar el secreto, `settings_delete` borrar la fila y el secreto, y `settings_set` insertar recién entonces una `secret_ref` que apunta a un secreto inexistente. Por eso se agrega un candado por operación y por clave, por ejemplo `SettingsKeyLocks` (`Mutex<HashMap<String, Arc<Mutex<()>>>>`, en `AppDbState` o como `static` junto a `APP_CREDENTIAL_LOCK`) con una función `lock_setting_key(key)` que devuelve el guard. Reglas: (a) `settings_set`, `settings_delete` y `settings_get` de claves secretas toman el guard de la clave **antes** de cualquier otra cosa y lo sostienen hasta terminar la secuencia completa (llavero + fila de referencia); cualquier otro escritor de una clave de `SECRET_SETTING_KEYS` pasa por las mismas funciones; (b) orden de adquisición fijo: candado de clave → `ui_conn` (solo mientras dura la sentencia SQL, nunca durante la E/S del llavero) → `APP_CREDENTIAL_LOCK` (por llamada); jamás se pide el candado de clave con `ui_conn` tomado, así que no hay ciclos; (c) las claves no secretas no toman el candado. Para poder probarlo, refactorizar `remove_setting_row`/`settings_delete` en una variante `…_with(conn, key, delete)` análoga a `persist_setting_with`, de modo que los tests inyecten un llavero falso.
     **Test de concurrencia con barrera** (en `settings.rs`): un llavero falso en memoria cuyos `store`/`delete` esperan en un `std::sync::Barrier` (o canales) para forzar la intercalación `set.store → delete completo → set.insert`; dos hilos, `settings_set` y `settings_delete` de la misma clave, probados en ambos órdenes de llegada; al terminar, una función `assert_no_dangling_secret_ref(conn, fake_keyring)` verifica que **ninguna** fila de `app_settings` cuyo valor sea una `secret_ref` apunte a un secreto ausente del llavero falso (el estado final válido es «sin fila y sin secreto» o «fila y secreto»). El test debe fallar si se quita el candado de clave (se comprueba una vez desactivándolo a propósito) y pasar con él. Otro test: dos `settings_set` concurrentes de claves distintas no se bloquean entre sí.
- **Aceptación:** sin cambio de comportamiento visible (en particular, un fallo del llavero en `settings_set` sigue devolviendo error sin dejar fila de referencia); tests de `settings.rs` verdes, incluido uno con un `store` que falla y verifica que `app_settings` no tiene la fila; ningún `get_setting`, `persist_setting` ni `delete_secret` toca el llavero con `ui_conn` tomado (grep en `settings.rs`), y pasa el test de concurrencia con barrera del paso 4 (sin referencias colgantes tras `set`/`delete` concurrentes).

---

### Fase 4 — Migraciones unificadas en Rust

Es la fase más grande y la que más cuidado requiere. Aplicar las migraciones fuera del renderer permite, al final, cerrar el DDL en el IPC.

#### 4.0 Documento de diseño corto · S

Antes de tocar código, un `odd/plans/plan-migraciones-rust.md` que fije:

- que los nombres de `_migrations` se mantienen **idénticos** (`0001_initial` … `0058_processing_ner_tasks`) y que **no se agrega ninguna fila nueva**, así una base existente no reaplica nada y el `schema_tag` (nombre máximo de `_migrations`, regla 5) no cambia;
- que el **esquema resultante no cambia**. `tests/fixtures/schema_full.sql` es la concatenación del SQL de las migraciones que genera `export-schema.mjs` (`packages/store/src/runner.ts:2986-3018`, `packages/store/scripts/export-schema.mjs:20-22`), no un volcado canónico, así que compararlo byte a byte no prueba equivalencia. La verificación es: aplicar el runner TS y el runner Rust sobre dos bases vacías separadas y comparar un **volcado canónico** de cada una (`sqlite_master` ordenado por tipo y nombre, `PRAGMA table_info` de cada tabla, `PRAGMA index_list`/`index_info` y los triggers), que debe ser idéntico;
- dónde va cada parche que hoy está en `lib.rs:850-970`: por defecto queda como **reparación idempotente** dentro del runner (no agrega filas a `_migrations`). Convertirlo en migración numerada nueva **sí** sube el `schema_tag` (regla 5) y deja en `426` a los dispositivos con versiones anteriores; solo se hace en un PR propio con revisión de sync y nota de despliegue;
- el orden de arranque: Rust aplica migraciones → crea `AppDbState` → el frontend arranca sin correr `runMigrations`;
- el plan de convivencia: durante una versión, el runner TS queda como no-op que solo verifica que `_migrations` esté al día.

#### 4.1 Mover el registro de migraciones a archivos `.sql` · M

- Hacer de `packages/store/src/migrations/*.sql` la **única fuente**: completar las que faltan (0007, 0011-0014, 0019 y 0025) y generar el `MIGRATIONS` de TS desde esos archivos (o importarlos con `?raw` de Vite) para que no haya dos copias.
- Test: cada `.sql` es idéntico al registro que se aplica (reemplaza los tests de espejo parciales que hay hoy en `runner.test.ts`).

#### 4.2 Runner de migraciones en Rust · L

- **Archivos nuevos:** `apps/desktop/src-tauri/src/db/migrations.rs` (con `include_str!` de los `.sql`).
- **Comportamiento:**
  - **Toda** migración corre en `BEGIN IMMEDIATE … COMMIT` junto con su fila en `_migrations` (resuelve A-03). Las excepciones (por ejemplo `0020_layouts`, que hoy es programática) se implementan en Rust con la misma semántica.
  - Se porta la reparación de la `0032` (estado parcial) con sus tests.
  - Si falla, devuelve `Err` legible, no `panic`.
- **Tests:**
  - aplicar todo sobre una base vacía y comparar el volcado canónico (4.0) con el de una base migrada por el runner TS;
  - aplicar sobre bases "históricas" (fixtures con `_migrations` parciales, incluido el estado roto de la 0032) y verificar que llegan al mismo esquema;
  - idempotencia: correr dos veces no hace nada la segunda vez.

#### 4.3 Mover los parches de `setup` y quitar los `panic` (A-02) · M

- **Archivo:** `apps/desktop/src-tauri/src/lib.rs:850-1000`.
- **Pasos:**
  1. Cada bloque (`legacy_uniques_sql`, `migrate_extractions_method_check`, `ensure_llm_results_schema`, `ensure_layouts_schema`, `sort_index`, `asset_id`, `app_settings`) pasa a ser una reparación idempotente dentro del runner de 4.2, según lo decidido en 4.0. Si alguno necesitara ser migración numerada, va en un PR propio con revisión de sync, porque sube el `schema_tag` (regla 5).
  2. La deduplicación `DELETE FROM extractions/transcriptions WHERE rowid NOT IN …` corre **una sola vez** y dentro de `without_sync_capture`.
  3. Reemplazar los `.expect("Failed to …")` por el helper `fail(...)` que ya existe en `setup`, con mensajes en español para el usuario.
- **Aceptación:** si se fuerza un fallo (por ejemplo, base de solo lectura), la app muestra el diálogo de error en vez de cerrarse. Se verifica en un dev profile.

#### 4.4 Desactivar el runner TS y cerrar el DDL en el IPC · S

- `runMigrations` (TS) pasa a solo leer `_migrations` y fallar si falta alguna (señal de que el backend no migró).
- `db_execute_batch`: rechazar DDL (`CREATE`, `ALTER`, `DROP`) desde el renderer, en el validador **y** en el authorizer de 3.1 (`SQLITE_CREATE_*`, `SQLITE_DROP_*`, `SQLITE_ALTER_TABLE`); quitar `PRAGMA defer_foreign_keys` de la lista de pragmas permitidos y eliminar la ventana de migración y la lista de huellas de operaciones conocidas de 3.1 paso 3.
- Revisar que ningún repo de `packages/store` emita DDL fuera de las migraciones (grep de `CREATE`/`DROP` en `src/repos`).
- **Auditoría de triggers y vistas plantados, por definición canónica (no solo por nombre):** un trigger plantado puede llevar el **nombre de uno legítimo** (por ejemplo `collection_activity_items_update`, generado en `packages/store/src/runner.ts:16-30`, o un `trg_sync_*`) con un cuerpo que, por ejemplo, ponga `sync_meta.capture_enabled` en `'0'`; una allowlist de nombres lo dejaría pasar, SQLite no autoriza el cuerpo al preparar el `CREATE TRIGGER`, y un comentario inicial esquiva el chequeo textual de `db/commands.rs:639-658`. Por eso, en el arranque (dentro del runner Rust), antes de abrir la conexión de UI: (1) construir una **base canónica en memoria** aplicando el runner Rust de 4.2 y `sync::capture::ensure_capture`, de modo que sus triggers y vistas sean exactamente los que el código genera; (2) listar `sqlite_master WHERE type IN ('trigger','view')` de la base real y comparar, nombre por nombre, `tbl_name` y el texto **exacto** de `sql` contra la canónica (ambos textos los normaliza SQLite al guardarlos, así que no hace falta normalizar a mano); (3) todo objeto que no esté en la canónica **o** cuyo `sql`/`tbl_name` difiera se registra en `app_logs` y se elimina (`DROP TRIGGER`/`DROP VIEW`), y todo objeto canónico que falte o se haya eliminado se **recrea** desde la definición canónica (para `trg_sync_*`, `drop_all_sync_triggers` + `ensure_capture`, porque `create_trigger_sql` usa `IF NOT EXISTS` y no pisaría un trigger plantado con ese nombre). Motivo: pudo haberse creado o reemplazado desde el renderer durante la ventana entre la Fase 3 y la 4 (o antes de la Fase 3) y seguiría ejecutándose con escrituras del backend. Tests: (a) una base con un trigger extra sobre `items` arranca sin él; (b) una base con `collection_activity_items_update` **reemplazado** por un cuerpo malicioso (con y sin comentario inicial) arranca con la definición canónica restaurada y `sqlite_master.sql` idéntico al de la base canónica; (c) lo mismo con un `trg_sync_items_i` de cuerpo alterado; (d) después del arranque, `sync_meta.capture_enabled` sigue en `'1'` y una escritura del backend sobre `items` produce su fila en `sync_oplog`.
- **Aceptación:** pasa un test que intenta `DROP TABLE items` vía `db_execute_batch` y espera error; pasan los tests (a)-(d) de la auditoría de definiciones canónicas, incluido el de un trigger malicioso con nombre legítimo; las suites siguen verdes.

---

### Fase 5 — Rendimiento y observabilidad

#### 5.1 Rotar el log de la app (P-02) · S

- **Archivo:** `apps/desktop/src-tauri/src/app_logs.rs`.
- **Pasos:**
  1. En `AppLogsState::new`, leer el archivo **desde el final** (o en streaming línea por línea, sin `read_to_string`) y, si supera `MAX_LOG_ENTRIES` líneas o N MB, reescribirlo con las últimas entradas (escribiendo a un temporal y haciendo `rename` atómico).
  2. En `append_to_file`, cada K escrituras chequear el tamaño y compactar si pasó el umbral.
- **Tests:** un log de 50.000 líneas queda en `MAX_LOG_ENTRIES` después de arrancar; una línea corrupta no rompe la carga.

#### 5.2 Búsqueda vectorial del RAG con índice en memoria (P-01) · M

- **Archivos:** `apps/desktop/src-tauri/src/rag/retrieval.rs`, `crates/vecscan`.
- **Pasos:**
  1. Benchmark base con el perfil `measure` (ya existe en `Cargo.toml`): 10k, 50k y 200k vectores sintéticos (`packages/store/scripts/seed-synthetic-collection.mjs`).
  2. Primer paso barato: cambiar el `sort_by` completo por una selección parcial. **No** alcanza con un `BinaryHeap` fijo de `k × factor` antes del filtro de texto: el algoritmo actual recorre el ranking completo hasta juntar `k` assets con texto (`rag/retrieval.rs:215-233`), así que con más de `k × factor` vectores sin texto el heap acotado descartaría candidatos válidos y rompería la equivalencia. Hacer la selección incremental (heap acotado que se amplía y sigue recorriendo hasta encontrar `k` con texto) o, más simple, usar el heap de `k × factor` y, si al filtrar quedan menos de `k`, caer al recorrido completo actual.
  3. Segundo paso: reutilizar `vecscan` con una caché invalidada por un contador de generación de `vec_assets`, igual que en la búsqueda de pasajes de la Biblioteca. El contador lo incrementan triggers `AFTER INSERT`, `AFTER UPDATE` y `AFTER DELETE` sobre `vec_assets` (o una columna de versión que se actualiza en cada escritura). **No** usar `MAX(rowid)` + `COUNT(*)`: el _upsert_ de embeddings es `ON CONFLICT(asset_id) DO UPDATE` (`nlp/embeddings.rs:2379-2382`), que conserva `rowid` y cantidad de filas, así que el RAG rankearía con vectores viejos. Test: tras actualizar en el lugar el embedding de un asset, la siguiente consulta refleja el vector nuevo.
- **Aceptación:** los mismos ids en el mismo orden que la implementación actual (test de equivalencia) y una mejora medida y documentada en el PR.

#### 5.3 Hash del runtime en streaming (P-03) · S

- **Archivos:** `runtime/download.rs:560-590` y `runtime/manager.rs:1257`.
- Una sola función `sha256_file_streaming(path)` con un `BufReader` de 1 MB, usada en ambos lugares.
- **Aceptación:** los tests existentes de verificación del runtime pasan.

#### 5.4 Logging unificado del backend (P-04) · M

- Introducir `tracing` (o `log`) con dos _sinks_: `stderr` (dev) y `app_logs` para `warn` y `error`.
- Migrar los `eprintln!` de forma incremental, empezando por `setup`, sync, runtime y processing; no en un único PR gigante.
- Agregar una regla en `clippy.toml` o en un test (`print_stderr` como warning) para que no se sumen `eprintln!` nuevos.

---

### Fase 6 — Empaquetado y release

#### 6.1 `libpdfium.so` real en Pro Linux (E-02) · S

Según el resultado de 0.3:

- Si el problema se confirma: que el job Pro Linux de `release.yml` corra `fetch-pdfium.sh linux-x64` y empaquete `resources/pdfium/libpdfium.so`, como Lite.
- Siempre: mover los _fixtures_ de texto `resources/lib/linux-x86_64/lib*.so` a un directorio de tests o renombrarlos (`*.fixture`), para que nunca entren a un bundle; y que el resolutor de Pdfium descarte candidatos que no sean ELF/PE/Mach-O o que pesen menos de 1 MB.

#### 6.2 Lite Linux sin _payload_ de Pro (E-01) · S

- `tauri.lite.linux.conf.json` → `resources` con el **mínimo neutral de plataforma**: `resources/provider-compatibility.json`, `resources/lib/LICENSE`, `resources/fonts/LICENSE`, `resources/pdfium/libpdfium.so` y `resources/pdfium/LICENSE` (los que deja `apps/desktop/scripts/fetch-pdfium.sh linux-x64`). **No** copiar la lista de `tauri.lite.windows.conf.json` tal cual: sus entradas `resources/lib/pdfium.dll` y `target/release/vc-runtime/*` son solo de Windows (`build.rs` crea y puebla `target/release/vc-runtime` únicamente con `target_os == "windows"` y `target_env == "msvc"`), así que en Linux el glob `vc-runtime/*` no encuentra nada y el bundle falla. `libpdfium.so` reemplaza a `pdfium.dll`, y `vc-runtime` se omite en Linux (y en macOS, que ya usa `libpdfium.dylib` en `tauri.lite.macos.conf.json`).
- Tests en `runtime-packaging.test.ts`: ninguna configuración Lite incluye `models/`, `runtime-pack/`, `tools/uv/` ni `scripts/*.py`; `tauri.lite.linux.conf.json` y `tauri.lite.macos.conf.json` no mencionan `.dll` ni `vc-runtime`; y toda entrada de `resources` de `tauri.lite.linux.conf.json` es una ruta literal que existe en un checkout limpio (`git ls-files`) o que produce `fetch-pdfium.sh linux-x64`.
- **Verificación:** desde un checkout limpio en Linux (`git clone` o `git worktree` nuevo, sin `target/`), correr `bash apps/desktop/scripts/fetch-pdfium.sh linux-x64` y `pnpm exec tauri build --config src-tauri/tauri.lite.conf.json --config src-tauri/tauri.lite.linux.conf.json --bundles deb` (igual que `lite-preview.yml`); el build debe terminar sin error de glob sin coincidencias y `dpkg -c` del `.deb` debe listar `libpdfium.so` y ninguna entrada de `models/`, `runtime-pack/`, `uv`, `*.py` ni `*.dll`.

#### 6.3 Guard de `build.rs` (E-03) · S

- Saltear el guard si no está `CARGO_FEATURE_LOCAL_ML` (Lite no tiene runtime) y exigir las **tres** variables en Pro.
- Al mismo tiempo, quitar las variables del runtime del job `build-lite` en `release.yml` y de `lite-preview.yml`, y actualizar el README y `AGENTS.md`.

#### 6.4 URL canónica del manifiesto del runtime (C-02) · S

- Confirmar si `HumaLab/EntropIA-Pro` es el nombre viejo de este repo.
- Cambiar `ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL` en `release.yml` y `lite-preview.yml` a la URL de `EntropIA-Pro-Lite` **solo después** de verificar que el release `runtime-bootstrap` existe ahí con `manifest.json` (si no, primero correr `publish-runtime-bootstrap.yml`).
- Test de contrato: la URL compilada coincide con la que publica `publish-runtime-bootstrap.yml`.
- **Riesgo:** los binarios ya instalados siguen usando la URL vieja. No dejar de publicar en la ubicación que resuelve la URL vieja mientras haya versiones viejas en uso.

#### 6.5 Una sola fuente para la versión (C-04) · S

- `release.yml`: derivar `StoreVersion` y el nombre del `.msix` de `apps/desktop/package.json` (o del tag) en lugar de dejarlos fijos.
- Extender `runtime-packaging.test.ts` para que compare `Cargo.toml`, `tauri.conf.json`, `tauri.lite.conf.json`, `package.json`, los manifiestos, `repack-store-msix.ps1` y `msix/README.md`.
- Opcional: un script `pnpm version:bump <x.y.z>` que los actualice todos juntos.

#### 6.6 Procedencia de los binarios versionados (D-07) · M

- Corto plazo: `apps/desktop/src-tauri/resources/CHECKSUMS.sha256` (`uv.exe` ×2, `pdfium.dll`, modelos `.mnn`, `EntropIALite-base.msix`), con versión y URL de origen en comentarios, y un test o paso de CI que lo verifique.
- Mediano plazo: descargar `uv` y `pdfium.dll` en el build (como `fetch-pdfium.sh`) y sacarlos del árbol. **No** reescribir la historia de git para achicar el repo sin acordarlo antes con el equipo.

#### 6.7 Inconsistencias menores de configuración (E-04) · S

- Título de ventana de `tauri.dev.conf.json`.
- `additionalWatchFolders`: moverlo a una configuración local (o documentarlo y que no falle si la carpeta no existe).
- Una constante `USER_AGENT` con `env!("CARGO_PKG_VERSION")`, usada por los 5 clientes HTTP.
- `rust:quality:report`: documentar que requiere `pwsh`, o agregar una variante bash.

---

### Fase 7 — Cadena de suministro en la CI

#### 7.1 Dependabot y auditoría (C-03) · S

- `.github/dependabot.yml` con `npm` (raíz), `cargo` (`apps/desktop/src-tauri`) y `github-actions`, agrupando las actualizaciones menores.
- Un job `audit` en `ci.yml`: `pnpm audit --prod --audit-level high` y `cargo audit` (con `.cargo/audit.toml` para las excepciones justificadas). Arranca como **no bloqueante** y pasa a bloqueante cuando se cierre la Fase 2.

#### 7.2 Que el bump del motor pase por un PR (D-05) · S

- `engine-pin-bump.yml`: en lugar de `git push origin "$BUMP_SHA:refs/heads/main"`, abrir un PR con `gh pr create` (habilitar auto-merge si el equipo lo quiere, pero con la protección de rama exigiendo aprobación) y bajar los permisos a `contents: write` + `pull-requests: write` + `actions: write`, sin empuje directo a `main`. **Mantener `actions: write` y el `gh workflow run ci.yml --ref "$BUMP_BRANCH"` explícito** (`.github/workflows/engine-pin-bump.yml:3-6,15,84`): un PR abierto con `GITHUB_TOKEN` no dispara los workflows de `pull_request`, así que sin ese despacho el PR quedaría sin checks y bloqueado por la protección de rama. Alternativa: crear el PR con un token de GitHub App o un PAT, que sí dispara la CI, y recién entonces quitar `actions: write`.
- Configurar la protección de `main` para que exija PR también a `github-actions[bot]`.

#### 7.3 Acciones fijadas por SHA (D-06) · S

- Reemplazar cada `uses: owner/action@vX` por `@<sha> # vX.Y.Z`, con Dependabot (7.1) para mantenerlas al día.
- Unificar `actions/upload-artifact` y `actions/download-artifact` en la misma versión mayor.
- En `release.yml`, que `dtolnay/rust-toolchain` use la versión de `rust-toolchain.toml` (ya pasa `toolchain: 1.90.0`; hacer lo mismo en `ci.yml` y `engine-pin-bump.yml`).

#### 7.4 Simplificar `ci.yml` (C-05) · S

- Si el problema de lockfile que motivó los pasos de _forensics_ de pnpm está resuelto, eliminarlos. Si se quieren conservar, extraerlos a `.github/actions/pnpm-forensics/action.yml` para no repetirlos por job.

---

### Fase 8 — Calidad, higiene y documentación

PRs chicos e independientes, que se pueden hacer en cualquier momento.

| Tarea | Hallazgo | Qué hacer                                                                                                                                                                                                                  | Esfuerzo |
| ----- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- |
| 8.1   | Q-01     | `reducedMotion` con `$state` en `EntropicConstellation.svelte`; corregir los warnings de `state_referenced_locally` (`$derived` o comentario justificado); limpiar los 5 warnings de ESLint; evaluar `--max-warnings 0`.   | S        |
| 8.2   | Q-02     | `renderMarkdown`: proteger código inline y enlaces con _placeholders_ antes del énfasis; tests con `[x](https://a.com/*b*)` y `` `**no**` ``.                                                                              | S        |
| 8.3   | Q-03     | Helper `registerDownloadListeners()` compartido por `SettingsView.svelte` y `DependenciasTab.svelte`, con limpieza si falla un `listen` intermedio.                                                                        | S        |
| 8.4   | Q-04     | `sanitizeNoteHtml` sin DOM: escapar en lugar de devolver el HTML crudo.                                                                                                                                                    | S        |
| 8.5   | S-07     | `validate_external_url`: parsear con `url::Url`, exigir `http`/`https` y _host_, y re-serializar.                                                                                                                          | S        |
| 8.6   | R-01     | Decidir si `docs/` es público. Si no, `git rm -r --cached docs/` y revisar si lo ya publicado necesita otro tratamiento. Quitar `artifacts/msix/EntropIALite-Store-HLab-1.0.5.0.msix`.                                     | S        |
| 8.7   | R-02     | `opencode.json`: fijar versiones de los MCP (`@playwright/mcp@x.y.z`, etc.), quitar la ruta personal (`serena.exe`) o moverla a una configuración local ignorada.                                                          | S        |
| 8.8   | R-03     | Mover `plan-editor.md` y `plan-lote.md` a `odd/plans/`; dejar `Manual-de-Uso.md` como un enlace al manual publicado.                                                                                                       | S        |
| 8.9   | R-05     | Alinear `AGENTS.md` y el README (variables de `build.rs`, job de tests Lite en la CI).                                                                                                                                     | S        |
| 8.10  | A-06     | Partir archivos grandes **solo cuando se toquen por otra razón**: `i18n.ts` → `i18n/es.ts` + `i18n/en.ts` (el que más rinde, por los conflictos de merge); `lib.rs` → `setup.rs` + `close.rs` (sale natural de la Fase 4). | M        |
| 8.11  | C-06     | Firma de código: presupuestar un certificado OV/EV (o Azure Trusted Signing) para Windows y Apple Developer ID para notarizar el DMG. Es una decisión de presupuesto (ver §4).                                             | —        |
| 8.12  | R-04     | Sin acción en el código: agregar una nota en `AGENTS.md` sobre auditar con clones superficiales.                                                                                                                           | S        |

---

## 4. Decisiones abiertas para el equipo

1. **Settings: ¿denylist o allowlist?** (1.1) Se recomienda la denylist de prefijos internos por simplicidad; la allowlist es más segura, pero obliga a registrar cada clave nueva.
2. **`pdf-extract`: ¿subproceso o reemplazo?** (2.3) El subproceso aísla también los _panics_ y _stack overflows_ futuros, a cambio de algo de latencia por documento.
3. **Tiptap 3** (2.2 paso 4): ¿entra en este ciclo o se acepta el riesgo medio documentado?
4. **¿`docs/` es público?** (8.6)
5. **Firma de código** (8.11): presupuesto y responsable.
6. **¿Auto-merge del bump del motor?** (7.2): si se habilita, aunque sea con CI verde, sigue siendo código de otro repo entrando sin revisión humana.
7. **Binarios en git** (6.6): ¿se acepta seguir versionándolos con un `CHECKSUMS`, o se pasa a descargarlos en el build?

---

## 5. Checklist de cada PR

- [ ] Cita el ID del hallazgo de `auditoria.md`.
- [ ] Tiene un test que fallaba antes del cambio (si es un hallazgo de seguridad o un bug).
- [ ] `pnpm lint`, `pnpm typecheck` (Pro y Lite), `pnpm format:check`.
- [ ] `pnpm test` y `VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test`.
- [ ] Si toca Rust: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` y `cargo test`, sin features **y** con `--features local-ml`.
- [ ] Si toca migraciones o arranque: probado en `ENTROPIA_DEV_PROFILE`, nunca en el archivo real; el esquema resultante se compara con el volcado canónico de 4.0 y no se agregan filas a `_migrations` sin revisión de sync (regla 5).
- [ ] Si toca capabilities, CSP o scopes: hay test nuevo en `tests/app_acl.rs` y se probaron a mano los flujos afectados.
- [ ] Si toca empaquetado: se generó el instalador de la variante afectada y se instaló en una máquina o VM limpia.
- [ ] Se actualiza `auditoria.md`, marcando el hallazgo como resuelto con el enlace al PR.

---

## 6. Cómo se mide el avance

Al final de cada fase, volver a correr la batería de verificaciones de §2 de `auditoria.md` y actualizar la tabla. Estado objetivo al cerrar el plan:

| Verificación                                  | Hoy                  | Objetivo                                  |
| --------------------------------------------- | -------------------- | ----------------------------------------- |
| `cargo test` Lite en Linux                    | 2 fallos             | 0 fallos, y en CI                         |
| `pnpm audit --prod`                           | 21 avisos (9 high)   | 0 high/critical                           |
| `pnpm audit` (dev)                            | 4 critical           | 0 critical                                |
| `cargo audit`                                 | 6 vulnerabilidades   | 0 (warnings justificados en `audit.toml`) |
| Tests de evasión SQL / ACL `fs`               | no existen           | existen y pasan                           |
| Escritura `fs` fuera de `com.entropia.shared` | permitida            | denegada (test ACL)                       |
| Fuentes de la versión de la app               | ~9, sin test cruzado | 1 fuente o test que las compara todas     |
| `eprintln!` en el backend                     | 371                  | en baja, sin nuevos (lint)                |
