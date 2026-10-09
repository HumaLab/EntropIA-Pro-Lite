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
5. **Sync y `schema_tag`:** cualquier cambio que altere el esquema sube el `schema_tag` de la cuenta en el primer sync. Las fases que tocan migraciones (Fase 4) **no cambian el esquema resultante**: solo cambian _quién_ y _cómo_ lo aplica. Si algún paso necesitara cambiar el esquema, se separa en un PR propio con revisión de sync.
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
  2. Agregar una función `is_renderer_writable_setting(key)` usada por `settings_set` y `settings_delete`. Recomendado: **denylist explícita** de prefijos internos (`runtime_bootstrap_`), porque la UI escribe ~80 claves (`lib/settings.ts`) y una allowlist completa es más frágil. Dejar anotada como decisión abierta la allowlist estricta (§4).
  3. En builds release (`#[cfg(not(debug_assertions))]`), que la fuente compilada (`option_env!`) **tenga prioridad** sobre `app_settings` cuando está definida. En debug se puede mantener la anulación para probar manifiestos de staging.
  4. Que la clave pública guardada en `app_settings` solo se acepte si su `id` **no** coincide con el `id` compilado: nunca reemplazar la clave oficial.
- **Tests:** los del paso 1, más los existentes `configured_bootstrap_catalog_*` adaptados a la nueva precedencia.
- **Aceptación:** con `--features local-ml`, no hay forma de cambiar la URL ni la clave usada en release desde IPC.
- **Riesgo:** algún instalador viejo con valores en `app_settings`. Mitigación: al arrancar, loguear (sin borrar) si existen esas claves.

#### 1.2 Acotar el scope del plugin `fs` (S-01) · M

- **Archivos:** `apps/desktop/src-tauri/capabilities/default.json`, `apps/desktop/src-tauri/tests/app_acl.rs`, y los usos de `@tauri-apps/plugin-fs` en `apps/desktop/src` (`lib/file-import.ts`, `lib/collection-import.ts`, `lib/export-images.ts`, `lib/writing-export.ts`, `lib/rag-chat-export.ts`, `views/CollectionAnalysisPanel.svelte`, `views/BatchSchemaPanel.svelte`, `views/CollectionView.svelte`, `views/CollectionsView.svelte`, `layout/WorkPane.svelte`).
- **Pasos:**
  1. **Inventario:** para cada llamada a `readFile`, `stat`, `copyFile`, `writeFile`, `remove` y `mkdir`, anotar de dónde sale la ruta: (a) un diálogo `open`/`save`, (b) el directorio de datos (`resolve_data_dir`), (c) otra fuente (importar carpeta, rutas guardadas de Zotero, `source_directory` de un ítem). El caso (c) es el que hoy depende de `*-read-recursive`.
  2. **Tests primero** en `tests/app_acl.rs`: `plugin:fs|write_file`, `plugin:fs|remove` y `plugin:fs|read_file` sobre una ruta de `$HOME` fuera de `com.entropia.shared` deben fallar con error de scope; sobre `$DATA/com.entropia.shared/assets/...` deben andar.
  3. Quitar `fs:allow-home-read-recursive`, `fs:allow-desktop-read-recursive`, `fs:allow-document-read-recursive` y `fs:allow-download-read-recursive`. Las rutas del caso (a) siguen funcionando porque `tauri-plugin-dialog` las agrega al scope en tiempo de ejecución.
  4. Para el caso (c), mover la lectura a un comando Rust dedicado (por ejemplo `import_read_source(path)` o `import_copy_into_archive(src, dest_rel)`) que valide extensión y tamaño, siga symlinks con cuidado (patrón `cap-std` ya usado en el repo) y escriba solo dentro del archivo. Registrar el comando en `build.rs` (`APP_COMMANDS`) y en la capability.
  5. Revisar `fs:allow-download-write`: si se usa solo para exportar con diálogo `save`, también se puede quitar.
- **Aceptación:** pasan los tests ACL nuevos y siguen funcionando las importaciones (archivo suelto, carpeta, desde Zotero), los exports y la eliminación de colecciones e ítems. Verificar a mano en un dev profile, en Windows y en Linux.
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

#### 2.3 Actualizar dependencias Rust con avisos (D-08) · M

- **Pasos:**
  1. `cargo update -p rustls -p crossbeam-epoch -p quick-xml` (cambios menores de versión). Correr clippy y tests en Lite y Pro.
  2. `imageproc` 0.25 → la versión corregida; revisar los usos (`image_edit.rs`, OCR).
  3. **`pdf-extract` / `lopdf 0.34`:**
     - Ver si hay una versión de `pdf-extract` que use `lopdf ≥ 0.42`. Si existe, actualizar.
     - Si no, aislar la extracción: correrla en un hilo con pila propia (`std::thread::Builder::stack_size`) **y** con un límite de profundidad. Como un _stack overflow_ aborta el proceso y no se puede atrapar, la protección real es correrla en un **subproceso** (el mismo binario con un subcomando, patrón que ya usa `llm/mod.rs:3110`). Evaluar ambas opciones con un PDF de prueba con objetos anidados.
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
  - `SQLITE_ATTACH`, `SQLITE_DETACH` y `SQLITE_PRAGMA` (salvo una lista de pragmas de solo lectura si alguno se usa);
  - cualquier lectura o escritura sobre la tabla `app_settings` (`SQLITE_READ` / `SQLITE_INSERT` / `SQLITE_UPDATE` / `SQLITE_DELETE` con `table == "app_settings"`);
  - escrituras sobre tablas `sync_*` y creación o borrado de triggers `trg_sync_*`;
  - `VACUUM` (el authorizer no tiene código propio para `VACUUM INTO`; se niega vía `SQLITE_ATTACH`, que es como lo implementa SQLite; **verificarlo con un test**).
- **Pasos:**
  1. **Tests primero** (en `db/commands.rs`) con las evasiones de la auditoría: `/**/VACUUM INTO '…'`, `--x\nATTACH …`, `INSERT INTO sync_oplog … RETURNING *` vía `db_select`, `WITH … INSERT INTO sync_meta …`, `UPDATE OR REPLACE sync_meta …`, y `SELECT * FROM app_settings` con comentarios intercalados.
  2. Implementar el authorizer y mantener los validadores de texto actuales como primera capa, con mensajes de error claros.
  3. Mientras el runner de migraciones TS siga existiendo (hasta la Fase 4), el authorizer **permite DDL** sobre tablas que no sean `sync_*` ni `app_settings`. La Fase 4 lo cierra.
- **Aceptación:** pasan todos los tests de evasión; la suite completa de store y desktop sigue verde (esas suites ejercitan los repos reales contra el mock de IPC; además correr la app en un dev profile y recorrer las vistas principales).

#### 3.2 Rollback del lado de Rust y transacciones atómicas (A-04) · S-M

- **Archivos:** `db/commands.rs` (`db_execute_batch`), `packages/store/src/repos/item.repo.ts:1513-1563`, `asset.repo.ts:~383` y `collection.repo.ts:~141`.
- **Pasos:**
  1. En `db_execute_batch`: si `execute_batch` falla y `conn.is_autocommit()` es `false`, ejecutar `ROLLBACK` **antes de soltar el lock**. Así nunca queda una transacción abierta entre dos llamadas IPC.
  2. Migrar `deleteWithCascade` y las otras dos cascadas a `db_execute_transaction` (ya existe, es atómico y usa parámetros), eliminando la interpolación `replace(/'/g, "''")`.
  3. Meter `DELETE FROM vec_assets WHERE item_id = ?` dentro de la misma transacción (con un chequeo previo de que la tabla exista, para que no falle).
- **Tests:** un test Rust que provoque un fallo a mitad de un batch con `BEGIN` y verifique `is_autocommit()` después; los tests de cascada existentes en `item.repo.test.ts` y `asset.repo.test.ts`.

#### 3.3 Acotar el protocolo `asset` (S-05) · M

- **Archivos:** `apps/desktop/src-tauri/tauri.conf.json` (y los overlays `tauri.lite.conf.json` y `tauri.dev.conf.json`, que repiten `assetProtocol`).
- **Pasos:**
  1. Inventariar qué rutas pide el frontend vía `convertFileSrc`/`asset:` (assets de colecciones, miniaturas, PDFs de la Biblioteca, imágenes de Escritura, capturas del Navegador).
  2. Reemplazar `$DATA/com.entropia.shared/**/*` por los subdirectorios concretos (por ejemplo `…/assets/**`, `…/thumbnails/**`, `…/writing/**`) y agregar `"deny"` explícito para `**/*.sqlite*` y `**/web-captures/**/*.html`.
  3. Mantener los tres archivos de configuración sincronizados y agregar un test (en `runtime-packaging.test.ts` o similar) que verifique que el scope es igual en los tres.
- **Aceptación:** la app muestra imágenes, PDFs, audio y miniaturas en un dev profile; `fetch(convertFileSrc('<data>/entropia.sqlite'))` desde la consola de devtools falla.
- **Nota:** el acceso por `plugin:fs|read_file` al `.sqlite` queda cerrado si en 1.2 el scope de lectura se limita a los mismos subdirectorios.

#### 3.4 Comandos `async` que bloquean y keyring con el lock tomado (A-05, S-06) · S

- **Archivos:** `settings.rs` (`settings_get`, `settings_set`, `settings_get_all`, `settings_delete`), `writing/publish.rs` y otros comandos que toman `ui_conn.lock()` en un `async fn` (grep `ui_conn` + `lock()` fuera de `spawn_blocking`).
- **Pasos:**
  1. Llevarlos a `run_blocking_db_task` (o `spawn_blocking`).
  2. Separar `get_setting` en dos pasos: leer la referencia con el lock tomado, soltarlo y recién ahí resolver el llavero.
- **Aceptación:** sin cambio de comportamiento visible; tests de `settings.rs` verdes.

---

### Fase 4 — Migraciones unificadas en Rust

Es la fase más grande y la que más cuidado requiere. Aplicar las migraciones fuera del renderer permite, al final, cerrar el DDL en el IPC.

#### 4.0 Documento de diseño corto · S

Antes de tocar código, un `odd/plans/plan-migraciones-rust.md` que fije:

- que los nombres de `_migrations` se mantienen **idénticos** (`0001_initial` … `0058_processing_ner_tasks`), así una base existente no reaplica nada;
- que el **esquema resultante no cambia**, verificado byte a byte contra `tests/fixtures/schema_full.sql` (que ya genera `export-schema.mjs`);
- dónde va cada parche que hoy está en `lib.rs:850-970`: o se convierte en migración numerada nueva (esto **sí** cambia `_migrations`, pero no el esquema) o queda como verificación de reparación idempotente;
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
  - aplicar todo sobre una base vacía y comparar con `schema_full.sql`;
  - aplicar sobre bases "históricas" (fixtures con `_migrations` parciales, incluido el estado roto de la 0032) y verificar que llegan al mismo esquema;
  - idempotencia: correr dos veces no hace nada la segunda vez.

#### 4.3 Mover los parches de `setup` y quitar los `panic` (A-02) · M

- **Archivo:** `apps/desktop/src-tauri/src/lib.rs:850-1000`.
- **Pasos:**
  1. Cada bloque (`legacy_uniques_sql`, `migrate_extractions_method_check`, `ensure_llm_results_schema`, `ensure_layouts_schema`, `sort_index`, `asset_id`, `app_settings`) pasa a ser una migración numerada o una reparación idempotente dentro del runner de 4.2, según lo decidido en 4.0.
  2. La deduplicación `DELETE FROM extractions/transcriptions WHERE rowid NOT IN …` corre **una sola vez** y dentro de `without_sync_capture`.
  3. Reemplazar los `.expect("Failed to …")` por el helper `fail(...)` que ya existe en `setup`, con mensajes en español para el usuario.
- **Aceptación:** si se fuerza un fallo (por ejemplo, base de solo lectura), la app muestra el diálogo de error en vez de cerrarse. Se verifica en un dev profile.

#### 4.4 Desactivar el runner TS y cerrar el DDL en el IPC · S

- `runMigrations` (TS) pasa a solo leer `_migrations` y fallar si falta alguna (señal de que el backend no migró).
- `db_execute_batch`: rechazar DDL (`CREATE`, `ALTER`, `DROP`) desde el renderer, en el validador **y** en el authorizer de 3.1 (`SQLITE_CREATE_*`, `SQLITE_DROP_*`, `SQLITE_ALTER_TABLE`).
- Revisar que ningún repo de `packages/store` emita DDL fuera de las migraciones (grep de `CREATE`/`DROP` en `src/repos`).
- **Aceptación:** pasa un test que intenta `DROP TABLE items` vía `db_execute_batch` y espera error; las suites siguen verdes.

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
  2. Primer paso barato: cambiar el `sort_by` completo por una selección parcial (un `BinaryHeap` de tamaño `k × factor`, porque después se filtra por texto).
  3. Segundo paso: reutilizar `vecscan` con una caché invalidada por un contador de generación de `vec_assets` (trigger, o `MAX(rowid)` + `COUNT(*)`), igual que en la búsqueda de pasajes de la Biblioteca.
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

- `tauri.lite.linux.conf.json` → `resources` igual a la lista mínima de Windows más `resources/pdfium/*`.
- Test en `runtime-packaging.test.ts`: ninguna configuración Lite incluye `models/`, `runtime-pack/`, `tools/uv/` ni `scripts/*.py`.

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

- `engine-pin-bump.yml`: en lugar de `git push origin "$BUMP_SHA:refs/heads/main"`, abrir un PR con `gh pr create` (habilitar auto-merge si el equipo lo quiere, pero con la protección de rama exigiendo aprobación) y bajar los permisos a `contents: write` + `pull-requests: write`, sin empuje directo a `main`.
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
- [ ] Si toca migraciones o arranque: probado en `ENTROPIA_DEV_PROFILE`, nunca en el archivo real; el esquema resultante se compara con `schema_full.sql`.
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
