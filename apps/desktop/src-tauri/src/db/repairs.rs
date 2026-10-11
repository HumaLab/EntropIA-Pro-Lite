//! Idempotent schema repairs around the store migrations (Phase 4.3b).
//!
//! [`migrate_archive`] is the one startup entry point: the pre-migration
//! repairs run first, then [`super::migrations::apply_migrations`], then the
//! post-migration repairs. The post pass repeats the pre repairs so a fresh
//! install reaches on its first boot the schema older installs reached on their
//! second; on a correctly migrated database they are no-ops.
//!
//! Every repair is idempotent, returns `Err` instead of panicking, and uses no
//! `.expect` outside tests. The deduplications that drop rows run inside
//! [`crate::sync::capture::without_sync_capture`]: deleting a duplicate is a
//! local repair, so the removed row must not be queued for sync.

use rusqlite::{Connection, OptionalExtension};

use super::migrations::{apply_migrations, MigrationReport};
use crate::sync::capture::without_sync_capture;

/// A startup pipeline failure, naming the repair step or migration that broke.
#[derive(Debug)]
pub(crate) enum ArchiveMigrationError {
    Repair {
        /// Spanish label of the repair stage shown in the error detail.
        step: &'static str,
        message: String,
    },
    Migration {
        migration: &'static str,
        message: String,
    },
}

impl ArchiveMigrationError {
    /// Spanish context of the startup dialog, one per pipeline phase.
    pub(crate) fn context(&self) -> &'static str {
        match self {
            Self::Repair { .. } => "No se pudo reparar la base de datos.",
            Self::Migration { .. } => "No se pudo actualizar la base de datos.",
        }
    }

    /// Spanish detail naming the failing step, shown under the context.
    pub(crate) fn detail(&self) -> String {
        match self {
            Self::Repair { step, message } => {
                format!("El paso \"{step}\" de la reparación falló: {message}")
            }
            Self::Migration { migration, message } => {
                format!("La migración \"{migration}\" de la base de datos falló: {message}")
            }
        }
    }
}

impl std::fmt::Display for ArchiveMigrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repair { step, message } => {
                write!(formatter, "repair step \"{step}\" failed: {message}")
            }
            Self::Migration { migration, message } => {
                write!(formatter, "migration \"{migration}\" failed: {message}")
            }
        }
    }
}

impl std::error::Error for ArchiveMigrationError {}

/// Labels of the repair stages, as they appear in the startup error detail.
const REPAIR_DEDUP_ASSET_ROWS: &str = "deduplicar extractos y transcripciones";
const REPAIR_EXTRACTIONS_METHOD: &str = "reconstruir extractions.method";
const REPAIR_LLM_RESULTS: &str = "asegurar llm_results";
const REPAIR_LEGACY_ASSET_COLUMNS: &str = "columnas heredadas de assets y notas";
const REPAIR_APP_SETTINGS: &str = "asegurar app_settings";
const REPAIR_LAYOUTS: &str = "asegurar layouts";

/// One startup pipeline: repairs before the migrations, every pending migration,
/// then the repairs once more. Repairs work on the migration connection, before
/// the worker/UI connections and the sync capture bootstrap open.
pub(crate) fn migrate_archive(
    conn: &mut Connection,
) -> Result<MigrationReport, ArchiveMigrationError> {
    apply_pre_migration_repairs(conn)?;
    let report = apply_migrations(conn).map_err(|error| ArchiveMigrationError::Migration {
        migration: error.migration,
        message: error.message,
    })?;
    apply_post_migration_repairs(conn)?;
    Ok(report)
}

/// Repairs that fix the shapes migrations assume, run before the migrations.
///
/// On a fresh archive most of these are no-ops because the tables do not exist
/// yet; the untouched-by-migrations ones (`llm_results`, `app_settings`) create
/// their table so the settings migrations after the pipeline have it.
pub(crate) fn apply_pre_migration_repairs(conn: &Connection) -> Result<(), ArchiveMigrationError> {
    named(
        REPAIR_DEDUP_ASSET_ROWS,
        dedup_asset_rows_and_unique_indexes(conn),
    )?;
    named(
        REPAIR_EXTRACTIONS_METHOD,
        migrate_extractions_method_check(conn),
    )?;
    named(
        REPAIR_LLM_RESULTS,
        crate::llm::ensure_llm_results_schema(conn),
    )?;
    named(
        REPAIR_LEGACY_ASSET_COLUMNS,
        apply_legacy_asset_column_fallbacks(conn),
    )?;
    named(REPAIR_APP_SETTINGS, ensure_app_settings_table(conn))?;
    Ok(())
}

/// Repairs that need the migrated schema (`assets`, `layouts`), then the pre
/// repairs once more. After a successful migration the second pass is a no-op;
/// its purpose is to act on the first boot for what migrations left alone.
pub(crate) fn apply_post_migration_repairs(conn: &Connection) -> Result<(), ArchiveMigrationError> {
    named(REPAIR_LAYOUTS, ensure_layouts_schema(conn))?;
    apply_pre_migration_repairs(conn)
}

fn named(step: &'static str, outcome: Result<(), String>) -> Result<(), ArchiveMigrationError> {
    outcome.map_err(|message| ArchiveMigrationError::Repair { step, message })
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?1 LIMIT 1",
        rusqlite::params![table],
        |_row| Ok(true),
    )
    .unwrap_or(false)
}

/// Normalizes legacy duplicates and enforces one-row-per-asset semantics for
/// `extractions`/`transcriptions`, so Rust workers can use real UPSERT.
///
/// Fresh installs do not have these tables yet (the migrations create them), so
/// the repair is conditional. The deduplication runs with capture disabled: the
/// dropped duplicates are local repairs and must not queue as synced deletes.
fn dedup_asset_rows_and_unique_indexes(conn: &Connection) -> Result<(), String> {
    let has_extractions_table = table_exists(conn, "extractions");
    let has_transcriptions_table = table_exists(conn, "transcriptions");

    if !has_extractions_table && !has_transcriptions_table {
        eprintln!(
            "[setup] extractions/transcriptions tables not found — skipping legacy unique-index enforcement"
        );
        return Ok(());
    }

    if has_extractions_table {
        without_sync_capture(conn, || {
            conn.execute_batch(
                "DELETE FROM extractions
                 WHERE rowid NOT IN (
                   SELECT MAX(rowid) FROM extractions GROUP BY asset_id
                 );
                 CREATE UNIQUE INDEX IF NOT EXISTS idx_extractions_asset_id_unique
                 ON extractions(asset_id);",
            )
            .map_err(|error| format!("Failed to enforce unique extractions indexes: {error}"))
        })?;
    }

    if has_transcriptions_table {
        without_sync_capture(conn, || {
            conn.execute_batch(
                "DELETE FROM transcriptions
                 WHERE rowid NOT IN (
                   SELECT MAX(rowid) FROM transcriptions GROUP BY asset_id
                 );
                 CREATE UNIQUE INDEX IF NOT EXISTS idx_transcriptions_asset_id_unique
                 ON transcriptions(asset_id);",
            )
            .map_err(|error| format!("Failed to enforce unique transcriptions indexes: {error}"))
        })?;
    }

    Ok(())
}

/// Migrates the `extractions` table to remove the legacy CHECK constraint
/// on the `method` column that only allowed 'native' and 'ocr'.
/// PaddleOCR uses methods like 'paddle', 'paddle_vl', 'pdf_paddle', and 'pdf_paddle_vl'.
/// SQLite does not support ALTER TABLE DROP CONSTRAINT, so we recreate the table.
fn migrate_extractions_method_check(conn: &Connection) -> Result<(), String> {
    // Check if the CHECK constraint exists by reading the stored table SQL.
    let has_check: bool = conn
        .prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='extractions'")
        .and_then(|mut stmt| {
            stmt.query_row([], |row| {
                let sql: String = row.get(0)?;
                Ok(sql.contains("CHECK(method IN"))
            })
        })
        .unwrap_or(false);

    if !has_check {
        eprintln!(
            "[setup] extractions.method: no legacy CHECK constraint found — skipping migration"
        );
        return Ok(());
    }

    eprintln!("[setup] Migrating extractions table to remove legacy method CHECK constraint...");

    conn.execute_batch(
        "BEGIN TRANSACTION;
         CREATE TABLE extractions_new (
           id TEXT PRIMARY KEY,
           asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
           text_content TEXT NOT NULL,
           method TEXT NOT NULL,
           confidence REAL,
           created_at INTEGER NOT NULL
         );
         INSERT INTO extractions_new SELECT * FROM extractions;
         DROP TABLE extractions;
         ALTER TABLE extractions_new RENAME TO extractions;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_extractions_asset_id_unique ON extractions(asset_id);
         COMMIT;",
    )
    .map_err(|error| format!("Failed to migrate extractions table: {error}"))?;

    eprintln!("[setup] extractions.method CHECK constraint removed successfully");
    Ok(())
}

/// Legacy-column fallbacks for databases that have not run the JS migrations
/// that own those columns. Gated on `0017_vec_assets` exactly as before: once
/// the modern schema is in, the fallback must never touch the tables again.
fn apply_legacy_asset_column_fallbacks(conn: &Connection) -> Result<(), String> {
    let modern_schema_bootstrapped = migration_applied(conn, "0017_vec_assets")?;

    if modern_schema_bootstrapped {
        return Ok(());
    }

    if !table_exists(conn, "assets") {
        eprintln!("[setup] assets table not found — skipping legacy fallback schema patching");
        return Ok(());
    }

    // Legacy fallback for databases that haven't run JS migrations yet.
    let has_sort_index: bool = conn
        .prepare("SELECT sort_index FROM assets LIMIT 0")
        .map(|mut stmt| {
            let _ = stmt.query_map([], |_| Ok(()));
            true
        })
        .unwrap_or(false);

    if !has_sort_index {
        conn.execute_batch(
            "ALTER TABLE assets ADD COLUMN sort_index INTEGER NOT NULL DEFAULT 0;
             CREATE INDEX IF NOT EXISTS idx_assets_item_sort ON assets(item_id, sort_index);",
        )
        .map_err(|error| format!("Failed to add sort_index column to assets: {error}"))?;
        eprintln!("[setup] Added sort_index column to assets table");
    }

    let has_notes_asset_id: bool = conn
        .prepare("SELECT asset_id FROM notes LIMIT 0")
        .map(|mut stmt| {
            let _ = stmt.query_map([], |_| Ok(()));
            true
        })
        .unwrap_or(false);

    if !has_notes_asset_id {
        conn.execute_batch(
            "ALTER TABLE notes ADD COLUMN asset_id TEXT;
             ALTER TABLE entities ADD COLUMN asset_id TEXT;
             ALTER TABLE triples ADD COLUMN asset_id TEXT;
             CREATE INDEX IF NOT EXISTS idx_notes_asset_id ON notes(asset_id);
             CREATE INDEX IF NOT EXISTS idx_entities_asset_id ON entities(asset_id);
             CREATE INDEX IF NOT EXISTS idx_triples_asset_id ON triples(asset_id);",
        )
        .map_err(|error| format!("Failed to add asset_id columns: {error}"))?;
        eprintln!("[setup] Added asset_id columns to notes, entities, triples");
    }

    Ok(())
}

/// True when `_migrations` records `name`. A database without the registry
/// table has applied nothing.
fn migration_applied(conn: &Connection, name: &str) -> Result<bool, String> {
    let has_migrations_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='_migrations' LIMIT 1",
            [],
            |_row| Ok(true),
        )
        .unwrap_or(false);

    if !has_migrations_table {
        return Ok(false);
    }

    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM _migrations WHERE name = ?1 LIMIT 1",
            rusqlite::params![name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed to check migration '{name}': {error}"))?;

    Ok(found.is_some())
}

/// Creates the settings table when absent. Kept out of the `0017` fallback:
/// modern-schema databases need it too, and the settings migrations after the
/// pipeline write into it.
fn ensure_app_settings_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .map_err(|error| format!("Failed to create app_settings table: {error}"))?;
    eprintln!("[setup] app_settings table ensured");
    Ok(())
}

/// Ensures the `layouts` table exists in its current shape and that one row per
/// asset survives.
///
/// `layouts.asset_id` references `assets`, which the migrations create. Until
/// they have run, store migration 0020 owns `layouts`; touching it here would
/// leave a table whose writes fail with `no such table: main.assets` while
/// foreign keys are on.
fn ensure_layouts_schema(conn: &Connection) -> Result<(), String> {
    if !table_exists(conn, "assets") {
        eprintln!("[setup] assets table not found — leaving layouts to the store migrations");
        return Ok(());
    }

    let has_layouts_table: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='layouts' LIMIT 1",
            [],
            |_row| Ok(true),
        )
        .unwrap_or(false);

    if !has_layouts_table {
        conn.execute_batch(
            "CREATE TABLE layouts (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                regions TEXT NOT NULL,
                blocks TEXT NOT NULL,
                model TEXT NOT NULL,
                image_width INTEGER NOT NULL,
                image_height INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_layouts_asset_id ON layouts(asset_id);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_layouts_asset_id_unique ON layouts(asset_id);",
        )
        .map_err(|error| format!("Failed to create layouts table: {error}"))?;
        eprintln!("[setup] layouts table created with blocks column");
        return Ok(());
    }

    let mut stmt = conn
        .prepare("PRAGMA table_info(layouts)")
        .map_err(|error| format!("Failed to inspect layouts schema: {error}"))?;

    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("Failed to read layouts schema: {error}"))?;

    let mut has_blocks = false;
    for column in columns {
        if column.map_err(|error| format!("Failed to read layouts column: {error}"))? == "blocks" {
            has_blocks = true;
            break;
        }
    }
    drop(stmt);

    if !has_blocks {
        eprintln!("[setup] Migrating legacy layouts table to add blocks column...");
        conn.execute_batch(
            "BEGIN TRANSACTION;
             CREATE TABLE layouts_new (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                regions TEXT NOT NULL,
                blocks TEXT NOT NULL,
                model TEXT NOT NULL,
                image_width INTEGER NOT NULL,
                image_height INTEGER NOT NULL,
                created_at INTEGER NOT NULL
             );
             INSERT INTO layouts_new (
                id, asset_id, regions, blocks, model, image_width, image_height, created_at
             )
             SELECT
                id,
                asset_id,
                regions,
                '[]' AS blocks,
                model,
                image_width,
                image_height,
                created_at
             FROM layouts;
             DROP TABLE layouts;
             ALTER TABLE layouts_new RENAME TO layouts;
             COMMIT;",
        )
        .map_err(|error| format!("Failed to migrate layouts table: {error}"))?;
    }

    // Dropping duplicate layout rows is a local repair too: keep it out of the
    // sync oplog like the extraction/transcription dedup.
    without_sync_capture(conn, || {
        conn.execute_batch(
            "DELETE FROM layouts
             WHERE rowid NOT IN (
               SELECT MAX(rowid) FROM layouts GROUP BY asset_id
             );
             CREATE INDEX IF NOT EXISTS idx_layouts_asset_id ON layouts(asset_id);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_layouts_asset_id_unique ON layouts(asset_id);",
        )
        .map_err(|error| format!("Failed to finalize layouts schema: {error}"))
    })?;

    eprintln!("[setup] layouts schema ensured");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::canonical_dump::{canonical_schema_dump, CanonicalSchemaDump};
    use crate::db::migrations::apply_migrations_through;
    use crate::sync::test_support::set_session_with_capture;
    use rusqlite::OpenFlags;
    use serde::Serialize;
    use serde_json::Value;
    use std::collections::BTreeMap;

    const CANONICAL_FIXTURE: &str = include_str!("../../tests/fixtures/schema_canonical.json");

    fn fk_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn
    }

    fn registry_row_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM _migrations", [], |row| row.get(0))
            .expect("count _migrations")
    }

    fn extraction_rows(conn: &Connection) -> Vec<(String, String)> {
        let mut statement = conn
            .prepare("SELECT asset_id, text_content FROM extractions ORDER BY asset_id")
            .expect("prepare extraction rows");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query extraction rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect extraction rows")
    }

    fn dedup_oplog_rows(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name = 'extractions' AND op = 'D'",
            [],
            |row| row.get(0),
        )
        .expect("count 'D' oplog rows")
    }

    fn total_changes(conn: &Connection) -> i64 {
        conn.query_row("SELECT total_changes()", [], |row| row.get(0))
            .expect("read total_changes()")
    }

    #[test]
    fn an_empty_database_run_matches_the_canonical_fixture() {
        // Tables the backend owns outright: the pre repairs create them
        // (`app_settings`, and `ocr_correction_backups` through
        // `ensure_llm_results_schema`), never a store migration, so the
        // TS-migrated fixture cannot hold them. Every archive has had both
        // since its first launch: setup created them before Phase 4 too.
        const BACKEND_OWNED_TABLES: [&str; 2] = ["app_settings", "ocr_correction_backups"];

        let mut conn = fk_connection();
        migrate_archive(&mut conn).expect("migrate an empty archive");

        let expected: CanonicalSchemaDump =
            serde_json::from_str(CANONICAL_FIXTURE).expect("parse the canonical fixture");
        let mut actual = canonical_schema_dump(&conn).expect("dump the migrated schema");

        let backend_owned = |table: &str| BACKEND_OWNED_TABLES.contains(&table);
        let mut found: Vec<&str> = actual
            .tables
            .iter()
            .map(|table| table.name.as_str())
            .filter(|name| backend_owned(name))
            .collect();
        found.sort_unstable();
        assert_eq!(
            found, BACKEND_OWNED_TABLES,
            "the repairs create both tables"
        );
        let owned_indexes: Vec<String> = actual
            .objects
            .iter()
            .filter(|object| object.object_type == "index" && backend_owned(&object.tbl_name))
            .map(|object| object.name.clone())
            .collect();
        actual
            .objects
            .retain(|object| !backend_owned(&object.tbl_name));
        actual.tables.retain(|table| !backend_owned(&table.name));
        actual
            .indexes
            .retain(|index| !owned_indexes.contains(&index.name));

        let differences = dump_differences(&expected, &actual);
        assert!(
            differences.is_empty(),
            "migrate_archive on an empty database diverges from schema_canonical.json:\n{}",
            differences.join("\n")
        );
    }

    #[test]
    fn post_migration_repairs_change_nothing_on_a_freshly_migrated_database() {
        // The parent's oracle condition: if the POST pass altered a database
        // the migrations had just produced, that would be a real divergence
        // between migrations and patches. Isolate it from the pre pass.
        let mut conn = fk_connection();
        apply_pre_migration_repairs(&conn).expect("pre repairs");
        crate::db::migrations::apply_migrations(&mut conn).expect("store migrations");
        let before = canonical_schema_dump(&conn).expect("dump after migrations");
        let changes_before = total_changes(&conn);

        apply_post_migration_repairs(&conn).expect("post repairs");

        let after = canonical_schema_dump(&conn).expect("dump after post repairs");
        let differences = dump_differences(&before, &after);
        assert!(
            differences.is_empty(),
            "the post repairs changed the freshly migrated schema:\n{}",
            differences.join("\n")
        );
        assert_eq!(
            total_changes(&conn),
            changes_before,
            "the post repairs changed data"
        );
    }

    #[test]
    fn a_second_migrate_archive_is_a_no_op() {
        let mut conn = fk_connection();
        migrate_archive(&mut conn).expect("first run");
        let first_dump = canonical_schema_dump(&conn).expect("first dump");
        let changes_before = total_changes(&conn);

        let report = migrate_archive(&mut conn).expect("second run");
        assert!(
            report.applied.is_empty(),
            "the second run must find nothing pending"
        );
        assert_eq!(
            total_changes(&conn),
            changes_before,
            "no row may change on the second run"
        );
        let second_dump = canonical_schema_dump(&conn).expect("second dump");
        let differences = dump_differences(&first_dump, &second_dump);
        assert!(
            differences.is_empty(),
            "the second run changed the schema:\n{}",
            differences.join("\n")
        );
    }

    #[test]
    fn a_legacy_archive_dedups_without_capturing_and_finishes_migrating() {
        let mut conn = fk_connection();
        apply_migrations_through(&mut conn, "0015_topics").expect("build the pre-0016 cut");
        conn.execute_batch(
            "INSERT INTO collections (id, name, created_at, updated_at)
               VALUES ('collection-1', 'Collection', 0, 0);
             INSERT INTO items (id, title, collection_id, created_at, updated_at)
               VALUES ('item-1', 'Item', 'collection-1', 0, 0);
             INSERT INTO assets (id, item_id, path, type, created_at)
               VALUES ('asset-1', 'item-1', 'assets/a.pdf', 'pdf', 0);
             INSERT INTO extractions (id, asset_id, text_content, method, created_at)
               VALUES ('ext-old', 'asset-1', 'old', 'native', 1);
             INSERT INTO extractions (id, asset_id, text_content, method, created_at)
               VALUES ('ext-new', 'asset-1', 'new', 'paddle', 2);",
        )
        .expect("seed the duplicate extraction");

        crate::sync::capture::ensure_capture(&conn).expect("install capture on the legacy shape");
        set_session_with_capture(&conn);

        // The pre pass dedups with capture stood down: otherwise the DELETE
        // trigger queues the dropped duplicate as a synced delete.
        apply_pre_migration_repairs(&conn).expect("pre repairs");
        assert_eq!(
            dedup_oplog_rows(&conn),
            0,
            "the dedup must not queue the removed duplicate for sync"
        );

        migrate_archive(&mut conn).expect("finish the startup pipeline");

        assert_eq!(
            extraction_rows(&conn),
            vec![("asset-1".to_string(), "new".to_string())],
            "MAX(rowid) survives, one row per asset"
        );
        let unique_index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_extractions_asset_id_unique'",
                [],
                |row| row.get(0),
            )
            .expect("check the unique index");
        assert_eq!(unique_index, 1);
        assert_eq!(registry_row_count(&conn), 58, "every migration is applied");
        assert_eq!(
            dedup_oplog_rows(&conn),
            0,
            "no migration may capture the dedup either"
        );
    }

    #[test]
    fn a_read_only_archive_reports_the_repair_step_instead_of_panicking() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("read-only.sqlite");
        {
            let conn = Connection::open(&path).expect("create the archive");
            conn.execute_batch(
                "CREATE TABLE assets (id TEXT PRIMARY KEY);
                 CREATE TABLE extractions (
                   id TEXT PRIMARY KEY,
                   asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                   text_content TEXT NOT NULL,
                   method TEXT NOT NULL,
                   confidence REAL,
                   created_at INTEGER NOT NULL
                 );
                 INSERT INTO assets(id) VALUES ('asset-1');
                 INSERT INTO extractions(id, asset_id, text_content, method, created_at)
                   VALUES ('ext-1', 'asset-1', 'a', 'native', 1),
                          ('ext-2', 'asset-1', 'b', 'native', 2);",
            )
            .expect("seed the legacy archive");
        }

        let mut conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open the archive read-only");
        let error = migrate_archive(&mut conn).expect_err("a read-only archive must fail");

        match error {
            ArchiveMigrationError::Repair { step, message } => {
                assert_eq!(step, REPAIR_DEDUP_ASSET_ROWS);
                assert!(
                    message.contains("readonly") || message.contains("read-only"),
                    "the detail must keep SQLite's write failure: {message}"
                );
            }
            other => panic!("expected a repair failure, got {other:?}"),
        }
    }

    #[test]
    fn migrate_archive_names_the_failing_migration_in_the_detail() {
        let mut conn = fk_connection();
        // 0020 recreates `layouts` with `IF NOT EXISTS`, then ALTERs it and
        // dedups by `asset_id`; this planted table lacks that column, so the
        // migration fails and its name must reach the dialog detail.
        conn.execute_batch("CREATE TABLE layouts (id TEXT PRIMARY KEY);")
            .unwrap();

        let error = migrate_archive(&mut conn).expect_err("0020 must fail on this archive");
        let detail = error.detail();

        assert!(
            detail.contains("0020_layouts"),
            "the detail must name the migration: {detail}"
        );
        assert!(
            detail.contains("asset_id"),
            "the detail must keep the SQLite message: {detail}"
        );
        assert_eq!(
            error.context(),
            "No se pudo actualizar la base de datos.",
            "a migration failure keeps the migrations context"
        );
    }

    #[test]
    fn ensure_layouts_schema_waits_for_the_store_to_create_assets() {
        let conn = fk_connection();

        ensure_layouts_schema(&conn).unwrap();

        assert!(!table_exists(&conn, "layouts"));
    }

    #[test]
    fn ensure_layouts_schema_survives_a_half_bootstrapped_archive() {
        // A previous launch created `layouts` but the store never created `assets`.
        let conn = fk_connection();
        conn.execute_batch(
            "CREATE TABLE layouts (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                regions TEXT NOT NULL,
                blocks TEXT NOT NULL,
                model TEXT NOT NULL,
                image_width INTEGER NOT NULL,
                image_height INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );",
        )
        .unwrap();

        ensure_layouts_schema(&conn).unwrap();
    }

    #[test]
    fn ensure_layouts_schema_creates_layouts_once_assets_exists() {
        let conn = fk_connection();
        conn.execute_batch("CREATE TABLE assets (id TEXT PRIMARY KEY);")
            .unwrap();

        ensure_layouts_schema(&conn).unwrap();
        ensure_layouts_schema(&conn).unwrap();

        assert!(table_exists(&conn, "layouts"));
    }

    #[test]
    fn migrate_extractions_method_check_removes_legacy_check_and_preserves_upsert_target() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE assets (id TEXT PRIMARY KEY);
             INSERT INTO assets(id) VALUES ('asset-1');
             CREATE TABLE extractions (
               id TEXT PRIMARY KEY,
               asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
               text_content TEXT NOT NULL,
               method TEXT NOT NULL CHECK(method IN ('native', 'ocr')),
               confidence REAL,
               created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_extractions_asset_id ON extractions(asset_id);",
        )
        .expect("create legacy schema");

        migrate_extractions_method_check(&conn).expect("migrate extractions schema");

        let create_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='extractions'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated table sql");
        assert!(!create_sql.contains("CHECK(method IN"));

        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, method, confidence, created_at)
             VALUES ('ext-1', 'asset-1', 'first', 'paddle_vl', NULL, 1)
             ON CONFLICT(asset_id) DO UPDATE SET
               text_content = excluded.text_content,
               method = excluded.method,
               confidence = excluded.confidence,
               created_at = excluded.created_at",
            [],
        )
        .expect("insert modern OCR method");

        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, method, confidence, created_at)
             VALUES ('ext-2', 'asset-1', 'updated', 'pdf_paddle_vl', NULL, 2)
             ON CONFLICT(asset_id) DO UPDATE SET
               text_content = excluded.text_content,
               method = excluded.method,
               confidence = excluded.confidence,
               created_at = excluded.created_at",
            [],
        )
        .expect("upsert by asset_id");

        let (text, method, count): (String, String, i64) = conn
            .query_row(
                "SELECT text_content, method, (SELECT COUNT(*) FROM extractions) FROM extractions WHERE asset_id = 'asset-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read upserted extraction");
        assert_eq!(text, "updated");
        assert_eq!(method, "pdf_paddle_vl");
        assert_eq!(count, 1);
    }

    /// Names every schema object that differs between two canonical dumps, so
    /// an oracle failure points at the exact object instead of a JSON offset.
    fn dump_differences(
        expected: &CanonicalSchemaDump,
        actual: &CanonicalSchemaDump,
    ) -> Vec<String> {
        let mut differences = Vec::new();
        name_differences(
            "objects",
            &expected.objects,
            &actual.objects,
            |object| format!("{} {}", object.object_type, object.name),
            &mut differences,
        );
        name_differences(
            "tables",
            &expected.tables,
            &actual.tables,
            |table| table.name.clone(),
            &mut differences,
        );
        name_differences(
            "indexes",
            &expected.indexes,
            &actual.indexes,
            |index| index.name.clone(),
            &mut differences,
        );
        if expected.migrations != actual.migrations {
            differences.push(format!(
                "migrations: fixture has {} names, Rust has {}",
                expected.migrations.len(),
                actual.migrations.len()
            ));
        }
        differences
    }

    fn name_differences<T: PartialEq + Serialize>(
        what: &str,
        expected: &[T],
        actual: &[T],
        key: impl Fn(&T) -> String,
        out: &mut Vec<String>,
    ) {
        let expected_by_name: BTreeMap<String, &T> =
            expected.iter().map(|item| (key(item), item)).collect();
        let actual_by_name: BTreeMap<String, &T> =
            actual.iter().map(|item| (key(item), item)).collect();

        for (name, expected_item) in &expected_by_name {
            match actual_by_name.get(name) {
                None => out.push(format!("{what}: {name} missing from the Rust dump")),
                Some(actual_item) if *expected_item != *actual_item => {
                    let expected_value =
                        serde_json::to_value(expected_item).expect("serialize the fixture entry");
                    let actual_value =
                        serde_json::to_value(actual_item).expect("serialize the Rust entry");
                    collect_diffs(
                        &format!("{what}.{name}"),
                        &expected_value,
                        &actual_value,
                        out,
                    );
                }
                Some(_) => {}
            }
        }
        for name in actual_by_name.keys() {
            if !expected_by_name.contains_key(name) {
                out.push(format!("{what}: {name} unexpected in the Rust dump"));
            }
        }
    }

    /// Reports field-level differences for one pair of aligned values.
    fn collect_diffs(path: &str, expected: &Value, actual: &Value, out: &mut Vec<String>) {
        const MAX_DIFFS: usize = 40;
        if out.len() >= MAX_DIFFS {
            return;
        }
        match (expected, actual) {
            (Value::Object(expected_map), Value::Object(actual_map)) => {
                for (key, expected_value) in expected_map {
                    match actual_map.get(key) {
                        Some(actual_value) => collect_diffs(
                            &format!("{path}.{key}"),
                            expected_value,
                            actual_value,
                            out,
                        ),
                        None => out.push(format!("{path}.{key}: missing from the Rust dump")),
                    }
                }
                for key in actual_map.keys() {
                    if !expected_map.contains_key(key) {
                        out.push(format!("{path}.{key}: unexpected in the Rust dump"));
                    }
                }
            }
            (Value::Array(expected_items), Value::Array(actual_items)) => {
                if expected_items.len() != actual_items.len() {
                    out.push(format!(
                        "{path}: {} entries in the fixture vs {} in the Rust dump",
                        expected_items.len(),
                        actual_items.len()
                    ));
                }
                for (index, (expected_item, actual_item)) in
                    expected_items.iter().zip(actual_items).enumerate()
                {
                    collect_diffs(&format!("{path}[{index}]"), expected_item, actual_item, out);
                }
            }
            _ if expected != actual => {
                out.push(format!("{path}: fixture={expected} Rust={actual}"));
            }
            _ => {}
        }
    }
}
