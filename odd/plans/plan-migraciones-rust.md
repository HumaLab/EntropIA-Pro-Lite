# Plan: migraciones unificadas en Rust (Fase 4)

Estado: **implementado** (4.0 a 4.4b, PRs #46 a #54; detalle y evidencia en
`odd/tasks/audit-remediation.md`, Fase 4). Fecha: 2026-10-11.
Origen: `plan.md`, Fase 4 (A-01, A-02, A-03). Este documento es el paso 4.0: fija las reglas
antes de tocar código. Las referencias `archivo:línea` son de la rama `feat/phase4-migrations-design`.

## 1. Qué cambia y qué no

Hoy las migraciones las aplica el **renderer**. `initDb` (`apps/desktop/src/lib/db.ts:49`) llama a
`runMigrations` (`packages/store/src/runner.ts:3042`), que manda el SQL por IPC (`db_execute_batch`,
`db_execute`). Además, `setup` en `lib.rs:848-970` aplica parches de esquema propios, con `.expect`.

Al final de la fase, **Rust aplica todo en el arranque**, antes de crear `AppDbState`. El renderer
solo verifica, y el IPC deja de aceptar DDL.

### Invariantes (no se negocian)

1. **Nombres idénticos.** Las filas de `_migrations` siguen siendo `0001_initial` …
   `0058_processing_ner_tasks`. **No se agrega ninguna fila nueva** en esta fase.
2. **`schema_tag` sin cambios.** `read_schema_tag` (`sync/engine.rs:864-880`) toma el **nombre
   máximo** de `_migrations`. Con la regla 1, ninguna base existente ve subir su tag y ningún
   dispositivo con una versión anterior recibe `426`.
3. **Mismo esquema resultante.** Una base vacía migrada por el runner Rust tiene el mismo volcado
   canónico (sección 3) que una migrada por el runner TS.
4. **`applied_at` en segundos.** El runner TS guarda `Math.floor(Date.now() / 1000)`
   (`runner.ts:3080-3083`); Rust usa la misma unidad.
5. **La captura de sync se comporta igual.** Las migraciones corren con los triggers `trg_sync_*`
   activos, como hoy: si una migración reescribe filas capturadas, eso produce filas en
   `sync_oplog` igual que ahora. Solo las **deduplicaciones** de los parches de `setup` pasan a
   correr dentro de `without_sync_capture` (paso 4.3).
6. **Bajar de versión sigue funcionando.** Como los nombres son los mismos, una versión anterior
   ve todo aplicado y su runner TS no hace nada.

## 2. Una sola fuente de SQL (paso 4.1)

- **Los `.sql` de `packages/store/src/migrations/` son la fuente.** Faltan 7: `0007`, `0011`-`0014`,
  `0019` y `0025`. Se agregan con el SQL exacto del registro (`runner.ts:89`, con el mismo `.trim()`).
- **El registro TS se genera.** Un script `packages/store/scripts/generate-migrations.mjs` escribe
  `src/migrations.generated.ts` (`nombre → sql`). No usamos `?raw` de Vite porque
  `export-schema.mjs` y `export-migration-ipc.mjs` cargan `runner.ts` con Node puro, sin Vite.
  Un test de vitest falla si el archivo generado no coincide con los `.sql` (como
  `schema-fixture.test.ts` hace con `schema_full.sql`).
- **`0027_collection_activity`** sigue generándose desde `COLLECTION_ACTIVITY_DDL` (`runner.ts:518`) y
  ya tiene su test de frescura (`runner.test.ts:153-156`). El `.sql` generado pasa a ser lo que se
  aplica.
- **`0020_layouts` es la única excepción.** El registro aplica un marcador vacío
  (`runner.ts:436-441`) y después la función `applyLayoutsMigration` (`runner.ts:2945-2983`), que
  depende de datos (deduplica y rellena `created_at`). Su `.sql` documenta el DDL final y lo usa
  `buildSchemaFixture`. El test de identidad nombra esta excepción en vez de saltearla en silencio.
- **Rust incluye los mismos archivos** con `include_str!` desde `db/migrations.rs`. Un test de Rust
  compara la lista del módulo con el contenido del directorio, así una migración nueva no puede
  quedar fuera de un runner.
- Los tests de espejo parciales de `runner.test.ts` (`0023` por contenido, `0028` negativo) quedan
  cubiertos por el test de identidad. Se conservan porque siguen documentando el contenido
  esperado de esas migraciones.
- **Hecho en 4.1a.** El chequeo encontró 7 archivos faltantes y 19 distintos. Normalizados
  (sin comentarios, espacios ni `;` finales), solo `0020` difería en SQL: le faltaba
  `DEFAULT '[]'` en `layouts.blocks`. Manda el registro, porque es lo que aplicó cada base.

## 3. Cómo se prueba la equivalencia: volcado canónico

`tests/fixtures/schema_full.sql` es la concatenación del SQL de las migraciones
(`runner.ts:2986-3018`), no el esquema resultante: compararlo no prueba nada. En su lugar:

- **Volcado canónico.** Es un JSON con:
  - `sqlite_master` sin `sqlite_%`, ordenado por `type, name`, con `type`, `name`, `tbl_name` y `sql`;
  - por cada tabla, `PRAGMA table_xinfo` (incluye columnas generadas y ocultas) y
    `PRAGMA foreign_key_list`;
  - por cada índice, `PRAGMA index_xinfo`;
  - los **nombres** de `_migrations`, sin `applied_at`.
- **Fixture desde el runner TS.** `packages/store/scripts/export-schema-canonical.mjs` corre
  `runMigrations` sobre `node:sqlite` en memoria (el mismo motor que ya usa `runner.test.ts:7`),
  con el reloj fijo del grabador de S-02b, y escribe
  `apps/desktop/src-tauri/tests/fixtures/schema_canonical.json`. Un test de vitest verifica que el
  fixture esté al día.
- **Comparación en Rust.** Un test aplica el runner Rust sobre una base vacía, calcula el mismo
  volcado y lo compara con el fixture. La CI de Rust no necesita Node.
- La comparación es **después de las migraciones y antes de la captura de sync**. La captura
  (`ensure_capture`) y las reparaciones de `setup` tienen sus propios tests.
- **Hallazgo de 4.2a: el texto de `_migrations` y `layouts`.** En una base real, `sqlite_master.sql`
  de esas dos tablas es el DDL que `runner.ts` ejecuta en tiempo de ejecución (la creación de
  `_migrations` antes de `0001` y la de `applyLayoutsMigration`), con su propia indentación, no el
  texto de los `.sql`. El runner Rust tiene que ejecutar ese mismo texto byte a byte; si no, el
  volcado canónico difiere.
- **Riesgo conocido.** `node:sqlite` y el SQLite de `rusqlite` 0.31 (`bundled`) son versiones
  distintas (3.53.4 y 3.45.0). El cruce de 4.2a no mostró diferencias en ningún `PRAGMA`. `sqlite_master.sql` guarda el texto tal como se escribió, así que no varía. Si algún
  `PRAGMA` cambiara de formato entre versiones, el test lo mostraría y se normaliza ese campo,
  documentándolo en el test.

## 4. El runner de Rust (paso 4.2)

- **Módulo nuevo `apps/desktop/src-tauri/src/db/migrations.rs`.** Expone
  `apply_migrations(&mut Connection) -> Result<MigrationReport, MigrationError>`. Corre sobre una
  conexión propia abierta con `open_archive_connection` (`db/open.rs`), no sobre `ui_conn`, así
  que el authorizer de S-02 no interviene.
- **Cada migración pendiente** (orden por nombre, como `runner.ts:3060`) corre en
  `BEGIN IMMEDIATE`, sus sentencias y el `INSERT INTO _migrations` con `applied_at` en segundos,
  y por último `COMMIT`. Esto resuelve A-03: hoy solo la lista atómica (`runner.ts:3111-3145`)
  inserta su fila en la misma transacción.
- **Las sentencias se separan con `split_sql_statements`** (`db/sql_split.rs`), que respeta los
  cuerpos `BEGIN … END` de los triggers. El `split(';')` ingenuo del runner TS
  (`runner.ts:3026-3031`) es la razón por la que hoy esas migraciones tienen que ir enteras.
- **`duplicate column name` se tolera igual que hoy.** Solo en las migraciones que el runner TS
  aplica sentencia por sentencia (las que no están en la lista atómica) y solo esa sentencia. Dentro
  de una transacción, SQLite revierte únicamente la sentencia que falló, así que la migración sigue.
  La lista queda fija en el código y un test la compara con la de `runner.ts`.
- **`PRAGMA defer_foreign_keys`** de `0045`, `0048`, `0050`, `0052` y `0058` funciona dentro de la
  transacción, igual que hoy.
- **`0020_layouts` se porta a Rust con la misma semántica:**
  - `CREATE TABLE IF NOT EXISTS`;
  - `ALTER … ADD blocks`, tolerando solo `duplicate column name`;
  - el `DELETE` que conserva `MAX(rowid)`;
  - los dos índices;
  - el relleno de `created_at`.
- **La reparación de `0032` se porta con sus tres casos** (`runner.ts:3067-3109`):
  - están todos los objetos y falta la fila: inserta la fila;
  - estado parcial sin filas sobrevivientes: borra los objetos dentro de la transacción de la
    migración;
  - estado parcial con filas: error legible que pide restaurar una copia de seguridad.
- **Si algo falla**, se hace `ROLLBACK` y se devuelve un `Err` con el nombre de la migración y el
  mensaje de SQLite. Nunca `panic`.
- **Tests:**
  - una base vacía da el volcado canónico (sección 3);
  - bases históricas (`_migrations` parcial en varios cortes, incluido el estado roto de `0032`)
    llegan al mismo volcado;
  - correr dos veces no hace nada la segunda;
  - una migración que falla a mitad no deja ni su fila ni sus efectos.

## 5. Los parches de `setup` (paso 4.3, A-02)

Todos pasan al runner como **reparaciones idempotentes**, sin fila en `_migrations`. Convertir uno en
migración numerada subiría el `schema_tag` (invariante 2); si alguno lo necesitara, va en un PR
propio con revisión de sync y nota de despliegue.

| Parche hoy (`lib.rs`) | Dónde corre en el runner |
|---|---|
| `legacy_uniques_sql` (848-875): deduplica `extractions`/`transcriptions` y crea índices únicos | Antes de las migraciones pendientes, como hoy. La deduplicación corre **una sola vez** y dentro de `without_sync_capture` |
| `migrate_extractions_method_check` (885, def. 2190) | Antes, como hoy |
| `llm::ensure_llm_results_schema` (887, def. `llm/mod.rs:911`) | Antes, como hoy |
| `sort_index` y `asset_id` de respaldo (898-935), condicionados a que falte `0017_vec_assets` | Antes, como hoy |
| `app_settings` `CREATE TABLE IF NOT EXISTS` (~947) | Antes, como hoy |
| `ensure_layouts_schema` (890, def. 2234): no hace nada si no existe `assets` | **Después** de las migraciones, porque necesita `assets` y `layouts`. Su deduplicación va dentro de `without_sync_capture`. Se conservan sus tests `ensure_layouts_schema_*` |

- **Las reparaciones de "antes" se repiten una vez después de las migraciones.** Hoy, en una
  instalación nueva, los parches no hacen nada en el primer arranque (las tablas no existen) y
  recién actúan en el segundo. La segunda pasada deja la base en ese estado desde el primer
  arranque. Un test verifica que, sobre una base recién migrada, la segunda pasada no cambia nada;
  si cambiara algo, es una divergencia real entre migración y parche y se documenta.
- **Fuera de alcance:** `migrate_legacy_asset_paths` y `migrate_asset_paths_to_relative` siguen
  donde están, con su propia conexión y `without_sync_capture`. `migrate_legacy_default_openrouter_model`
  y `migrate_legacy_api_keys` también quedan igual.
- **Los `.expect("Failed to …")` se reemplazan por el `fail(...)` que ya existe en `setup`**
  (`lib.rs:662`), con mensajes en español. Aceptación en un dev profile: con la base en solo
  lectura, la app muestra el diálogo de error en lugar de cerrarse.

## 6. Orden de arranque

1. Migraciones de rutas de assets (como hoy).
2. Conexión de migración: reparaciones de "antes", luego `apply_migrations`, luego reparaciones de
   "después". La conexión se cierra al terminar.
3. `ensure_capture` sobre la ruta (como hoy, `lib.rs:~1196-1205`). Ahora cubre todas las tablas
   desde el primer arranque.
4. En el paso 4.4, la auditoría de triggers y vistas por definición canónica, antes de abrir la
   conexión de UI (detalle en `plan.md` 4.4).
5. `ui_conn`, `worker_conn`, `AppDbState`, motor de sync (como hoy).
6. El frontend arranca. Hasta 4.4, `initDb` sigue llamando a `runMigrations`: no encuentra nada
   pendiente, la ventana de migración de S-02 se abre y se cierra sola (ya está `0058`) y
   `ensureSyncCapture` no tiene nada que agregar.

**Riesgo a verificar en 4.2c:** si una actualización trae migraciones pesadas (por ejemplo, la
reconstrucción de un índice FTS en un archivo grande), ahora el trabajo ocurre en `setup`, antes de
que el frontend muestre algo. Se mide con un archivo sintético grande
(`scripts/seed-synthetic-collection.mjs`) y, si tarda, se decide cómo informarlo antes de activar
el runner.

## 7. Convivencia y cierre (paso 4.4)

- Durante una versión, `runMigrations` (TS) solo lee `_migrations` y falla si falta alguna: es la
  señal de que el backend no migró.
- `db_execute_batch` rechaza DDL en el validador y en el authorizer (`SQLITE_CREATE_*`,
  `SQLITE_DROP_*`, `SQLITE_ALTER_TABLE`).
- Se eliminan la ventana de migración, las huellas de `migration_allowlist.rs` y
  `migration_ddl_allowlist.txt`, el grabador `migration-ipc-recorder.ts` y su fixture, y
  `defer_foreign_keys` de los pragmas permitidos.
- Auditoría de triggers y vistas plantados, según `plan.md` 4.4, con sus tests (a) a (d).

## 8. Unidades de trabajo

Cada una es una rama apilada con su PR y su revisión RDD. Todas mantienen la app funcionando y las
suites en verde.

| Paso | Contenido | Activa algo en el arranque |
|---|---|---|
| 4.0 | Este documento | No |
| 4.1a | Los 7 `.sql` que faltan y el test de identidad `.sql` = registro | No |
| 4.1b | Registro TS generado desde los `.sql` y test de frescura | No |
| 4.2a | Volcado canónico: script TS, fixture, test de frescura y función de volcado en Rust | No |
| 4.2b | `db/migrations.rs` con todas sus pruebas, sin conectarlo a `setup` | No |
| 4.2c | Conectarlo en `setup` (orden de la sección 6) y medir el tiempo con un archivo grande | Sí |
| 4.3 | Parches de `setup` al runner, deduplicación sin captura y `fail(...)`; prueba en dev profile | Sí |
| 4.4a | `runMigrations` solo verifica; DDL cerrado en el IPC; fuera la ventana y las huellas | Sí |
| 4.4b | Auditoría canónica de triggers y vistas, con los tests (a) a (d) | Sí |
