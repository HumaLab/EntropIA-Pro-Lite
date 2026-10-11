//! Rust port of the renderer migration runner (Phase 4.2b).
//!
//! `packages/store/src/runner.ts::runMigrations` is the behaviour of record.
//! This module replays every pending migration in name order, each one inside
//! its own `BEGIN IMMEDIATE … COMMIT` together with its `_migrations` row, and
//! is pinned to the TypeScript runner by the canonical dump fixture
//! (`tests/fixtures/schema_canonical.json`, plan §3).
//!
//! It is not wired into `setup` yet: 4.2c does that and removes the
//! `#[allow(dead_code)]` in `db/mod.rs`. Plan issue A-03 is fixed here: the
//! TypeScript runner commits each statement before the registry row, so a
//! crash mid-migration leaves a half-applied file, while every Rust migration
//! is atomic.

use std::collections::HashSet;

use rusqlite::{params, params_from_iter, Connection};

use super::sql_split::split_sql_statements;

// ---------------------------------------------------------------------------
// Runtime DDL shared with canonical_dump's equivalence tests
// ---------------------------------------------------------------------------

/// `_migrations` bootstrap, byte-identical to the text `runner.ts` executes
/// before `0001_initial`. `0001_initial.sql` also creates the table with
/// `IF NOT EXISTS`, but SQLite keeps the FIRST definition's text in
/// `sqlite_master`, and the canonical fixture was recorded with this one.
pub(crate) const MIGRATIONS_TABLE_DDL: &str = r#"CREATE TABLE IF NOT EXISTS _migrations (
      id    INTEGER PRIMARY KEY AUTOINCREMENT,
      name  TEXT    NOT NULL UNIQUE,
      applied_at INTEGER NOT NULL
    );"#;

/// `layouts` runtime DDL from `applyLayoutsMigration()` (`runner.ts`). The
/// `0020_layouts.sql` file documents the same final shape, but it is not what
/// the runtime executes, so the canonical fixture carries this exact text.
pub(crate) const LAYOUTS_RUNTIME_DDL: &str = r#"CREATE TABLE IF NOT EXISTS layouts (
      id TEXT PRIMARY KEY,
      asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
      regions TEXT NOT NULL,
      blocks TEXT NOT NULL DEFAULT '[]',
      model TEXT NOT NULL,
      image_width INTEGER NOT NULL,
      image_height INTEGER NOT NULL,
      created_at INTEGER NOT NULL
    );"#;

// ---------------------------------------------------------------------------
// The 0032 repair data (runner.ts): sentinels, drop order, triggers
// ---------------------------------------------------------------------------

const BATCH_PROCESSING: &str = "0032_batch_processing";

/// Objects the atomic 0032 batch creates. A database holding all of them but
/// no 0032 registry row was burned by the earlier non-atomic registration:
/// repair the row instead of re-running DDL that would collide.
const PROCESSING_0032_SENTINELS: &[&str] = &[
    "processing_batches",
    "processing_batch_members",
    "processing_tasks",
    "processing_batch_tasks",
    "processing_requests",
    "processing_attempts",
    "processing_checkpoints",
    "processing_asset_revisions",
    "processing_meta",
    "trg_processing_extractions_ai",
];

/// Every table 0032 creates, children first so the repair's DROPs never trip a
/// foreign key. Superset of the sentinels above: the sentinels answer "was
/// this applied?", this list answers "what has to go?".
const PROCESSING_0032_TABLES_CHILD_FIRST: &[&str] = &[
    "processing_asset_revisions",
    "processing_checkpoints",
    "processing_attempts",
    "processing_requests",
    "processing_batch_tasks",
    "processing_tasks",
    "processing_batch_members",
    "processing_batch_collections",
    "processing_batches",
    "processing_meta",
];

/// Triggers 0032 creates; dropped before the tables they watch.
const PROCESSING_0032_TRIGGERS: &[&str] = &[
    "trg_processing_extractions_ai",
    "trg_processing_extractions_au",
    "trg_processing_extractions_ad",
    "trg_processing_transcriptions_ai",
    "trg_processing_transcriptions_au",
    "trg_processing_transcriptions_ad",
];

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrationKind {
    /// The whole file runs as one `execute_batch` inside the migration
    /// transaction, mirroring the TypeScript runner's atomic list. Trigger
    /// bodies (`BEGIN … END`) only survive this path.
    Atomic,
    /// Statements are split with [`split_sql_statements`] and executed one by
    /// one; a `duplicate column name` failure is tolerated for that statement.
    PerStatement,
    /// `0020_layouts`, ported from `applyLayoutsMigration` because it dedups
    /// existing rows and backfills `created_at`.
    Layouts,
}

/// One registry entry. The `kind` field pins the TypeScript atomic list: a
/// cross-language test compares every `Atomic` entry with the `name === '…'`
/// chain in `runner.ts`, so the two runners cannot drift.
struct Migration {
    name: &'static str,
    sql: &'static str,
    kind: MigrationKind,
}

/// Maps a migration name to its `.sql` file, inlined at compile time so the
/// packaged app never needs filesystem access (the same reason the TypeScript
/// registry inlines the files).
macro_rules! migration {
    ($name:literal, $kind:ident) => {
        Migration {
            name: $name,
            sql: include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../packages/store/src/migrations/",
                $name,
                ".sql"
            )),
            kind: MigrationKind::$kind,
        }
    };
}

/// Every migration in `packages/store/src/migrations`, in name order. A test
/// reads the directory with `std::fs` and fails if this list drifts.
static REGISTRY: &[Migration] = &[
    migration!("0001_initial", PerStatement),
    migration!("0002_metadata_search", PerStatement),
    migration!("0003_extractions", PerStatement),
    migration!("0004_fts5", PerStatement),
    migration!("0005_nlp_tables", PerStatement),
    migration!("0006_triples", PerStatement),
    migration!("0007_annotations", PerStatement),
    migration!("0008_transcriptions", PerStatement),
    migration!("0009_entities_provenance", PerStatement),
    migration!("0010_entities_type_expansion", PerStatement),
    migration!("0011_entities_geocoding", PerStatement),
    migration!("0012_llm_results", PerStatement),
    migration!("0013_assets_sort_index", PerStatement),
    migration!("0014_asset_scoping", PerStatement),
    migration!("0015_topics", PerStatement),
    migration!("0016_asset_unique_ocr_transcription", PerStatement),
    migration!("0017_vec_assets", PerStatement),
    migration!("0018_fts_rowid_canonical", PerStatement),
    migration!("0019_llm_results_target_type", PerStatement),
    migration!("0020_layouts", Layouts),
    migration!("0021_drop_unused_processing_table", PerStatement),
    migration!("0022_rag_conversations", PerStatement),
    migration!("0023_sync_ids", PerStatement),
    migration!("0024_pdf_page_assets", PerStatement),
    migration!("0025_document_view_edits", Atomic),
    migration!("0026_entity_manual_coordinates", PerStatement),
    migration!("0027_collection_activity", Atomic),
    migration!("0028_vec_assets_embedding_contract", PerStatement),
    migration!("0029_rag_chunks", Atomic),
    migration!("0030_items_collection_title_index", PerStatement),
    migration!("0031_items_source_directory", PerStatement),
    migration!("0032_batch_processing", Atomic),
    migration!("0033_processing_source_invalidation", PerStatement),
    migration!("0034_fts_contentless_delete", Atomic),
    migration!("0035_writing_workspace", Atomic),
    migration!("0036_writing_journal", Atomic),
    migration!("0037_fts_vocab", PerStatement),
    migration!("0038_processing_settle_on_terminal", Atomic),
    migration!("0039_items_import_source_index", PerStatement),
    migration!("0040_bibliography_catalog", Atomic),
    migration!("0041_bibliography_relations", Atomic),
    migration!("0042_bibliography_reconciliation", Atomic),
    migration!("0043_processing_task_subject_identity", Atomic),
    migration!("0044_processing_task_subject_cutover", Atomic),
    migration!("0045_bibliography_sync_tasks", Atomic),
    migration!("0046_processing_priority", Atomic),
    migration!("0047_bibliographic_semantic_profiles", Atomic),
    migration!("0048_bibliography_profile_tasks", Atomic),
    migration!("0049_bibliographic_index_generations", Atomic),
    migration!("0050_bibliographic_embedding_generations", Atomic),
    migration!("0051_bibliographic_profile_fts", Atomic),
    migration!("0052_bibliographic_extraction_tasks", Atomic),
    migration!("0053_bibliographic_page_texts", Atomic),
    migration!("0054_bibliographic_chunks", Atomic),
    migration!("0055_bibliographic_chunk_embeddings", Atomic),
    migration!("0056_bibliographic_ingest_operations", Atomic),
    migration!("0057_web_captures", Atomic),
    migration!("0058_processing_ner_tasks", Atomic),
];

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Name used for `_migrations` bootstrap/read failures, which belong to no
/// migration.
const BOOTSTRAP: &str = "<bootstrap>";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MigrationReport {
    /// Names of the migrations committed by this call, in name order.
    pub(crate) applied: Vec<&'static str>,
}

#[derive(Debug)]
pub(crate) struct MigrationError {
    /// The migration whose transaction failed (`<bootstrap>` when the
    /// `_migrations` table itself could not be created or read).
    pub(crate) migration: &'static str,
    pub(crate) message: String,
}

impl MigrationError {
    fn new(migration: &'static str, error: rusqlite::Error) -> Self {
        // rusqlite's Display for input errors appends the statement and the
        // offset; for `execute_batch` that is the whole migration file, far
        // too much for the startup dialog. Keep SQLite's own message.
        let message = match error {
            rusqlite::Error::SqlInputError { msg, .. } => msg,
            other => other.to_string(),
        };
        Self { migration, message }
    }
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Migration \"{}\" failed: {}",
            self.migration, self.message
        )
    }
}

impl std::error::Error for MigrationError {}

/// Applies every pending migration on `conn` with the real wall clock.
pub(crate) fn apply_migrations(conn: &mut Connection) -> Result<MigrationReport, MigrationError> {
    apply_migrations_with(conn, REGISTRY, &|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0)
    })
}

/// Applies the registry through `name` (inclusive) on `conn`, so a test can
/// build an archive at a historical migration cut. Test-only: production code
/// always applies every pending migration through [`apply_migrations`].
#[cfg(test)]
pub(crate) fn apply_migrations_through(
    conn: &mut Connection,
    name: &str,
) -> Result<MigrationReport, MigrationError> {
    let index = REGISTRY
        .iter()
        .position(|migration| migration.name == name)
        .unwrap_or_else(|| panic!("the registry is missing {name}"));
    apply_migrations_with(conn, &REGISTRY[..=index], &|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0)
    })
}

/// Injectable core: `registry` pins the migration set and `clock` returns the
/// current unix time in milliseconds, so tests control both.
fn apply_migrations_with(
    conn: &mut Connection,
    registry: &[Migration],
    clock: &dyn Fn() -> i64,
) -> Result<MigrationReport, MigrationError> {
    conn.execute_batch(MIGRATIONS_TABLE_DDL)
        .map_err(|error| MigrationError::new(BOOTSTRAP, error))?;
    let applied = read_applied(conn).map_err(|error| MigrationError::new(BOOTSTRAP, error))?;

    let mut pending: Vec<&Migration> = registry
        .iter()
        .filter(|migration| !applied.contains(migration.name))
        .collect();
    pending.sort_by_key(|migration| migration.name);

    let mut report = MigrationReport {
        applied: Vec::new(),
    };
    for migration in pending {
        run_migration(conn, migration, clock)?;
        report.applied.push(migration.name);
    }
    Ok(report)
}

fn read_applied(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut statement = conn.prepare("SELECT name FROM _migrations")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Preflight shared by every migration entry.
enum RepairPlan {
    /// Run the migration body unchanged.
    None,
    /// All 0032 sentinels exist: only the registry row is missing.
    RegistryRowOnly,
    /// A partial, empty 0032: these drops run inside the migration transaction
    /// before the file is replayed.
    Drops(String),
}

fn run_migration(
    conn: &mut Connection,
    migration: &Migration,
    clock: &dyn Fn() -> i64,
) -> Result<(), MigrationError> {
    let plan = plan_repair(conn, migration)?;
    let applied_at = clock() / 1000;

    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| MigrationError::new(migration.name, error))?;

    let outcome = run_migration_body(conn, migration, &plan, applied_at, clock);
    match outcome {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .map_err(|error| MigrationError::new(migration.name, error)),
        Err(error) => {
            // A failed body or registry INSERT leaves the transaction open; a
            // failed BEGIN returned above. Only roll back a live transaction.
            if !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            Err(error)
        }
    }
}

fn run_migration_body(
    conn: &Connection,
    migration: &Migration,
    plan: &RepairPlan,
    applied_at: i64,
    clock: &dyn Fn() -> i64,
) -> Result<(), MigrationError> {
    if let RepairPlan::Drops(drops) = plan {
        conn.execute_batch(drops)
            .map_err(|error| MigrationError::new(migration.name, error))?;
    }
    if !matches!(plan, RepairPlan::RegistryRowOnly) {
        apply_statement_set(conn, migration, clock)
            .map_err(|error| MigrationError::new(migration.name, error))?;
    }
    conn.execute(
        "INSERT INTO _migrations (name, applied_at) VALUES (?1, ?2)",
        params![migration.name, applied_at],
    )
    .map_err(|error| MigrationError::new(migration.name, error))?;
    Ok(())
}

fn apply_statement_set(
    conn: &Connection,
    migration: &Migration,
    clock: &dyn Fn() -> i64,
) -> rusqlite::Result<()> {
    match migration.kind {
        // `runner.ts` branches on `0020_layouts` and calls
        // `applyLayoutsMigration`; its registry marker SQL (a temp-table no-op)
        // is never executed there and is not executed here either.
        MigrationKind::Layouts => apply_layouts_migration(conn, clock()),
        MigrationKind::Atomic => conn.execute_batch(executable_sql(migration)),
        MigrationKind::PerStatement => {
            for statement in split_sql_statements(executable_sql(migration)) {
                if let Err(error) = conn.execute_batch(&statement) {
                    // SQLite rolls back the failed statement only, so the
                    // transaction stays usable and the migration continues.
                    if !is_duplicate_column(&error) {
                        return Err(error);
                    }
                }
            }
            Ok(())
        }
    }
}

/// Port of `applyLayoutsMigration` (`runner.ts`): every statement follows the
/// TypeScript order and text, including the millisecond `created_at` backfill.
fn apply_layouts_migration(conn: &Connection, now_ms: i64) -> rusqlite::Result<()> {
    conn.execute_batch(LAYOUTS_RUNTIME_DDL)?;

    if let Err(error) =
        conn.execute_batch("ALTER TABLE layouts ADD COLUMN blocks TEXT NOT NULL DEFAULT '[]'")
    {
        if !is_duplicate_column(&error) {
            return Err(error);
        }
    }

    conn.execute_batch(
        "DELETE FROM layouts
         WHERE rowid NOT IN (
           SELECT MAX(rowid) FROM layouts GROUP BY asset_id
         )",
    )?;
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_layouts_asset_id_unique ON layouts(asset_id)",
    )?;
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_layouts_asset_id ON layouts(asset_id)")?;
    conn.execute(
        "UPDATE layouts SET created_at = ?1 WHERE created_at IS NULL OR created_at = 0",
        params![now_ms],
    )?;
    Ok(())
}

fn plan_repair(conn: &Connection, migration: &Migration) -> Result<RepairPlan, MigrationError> {
    if migration.name != BATCH_PROCESSING {
        return Ok(RepairPlan::None);
    }
    let present = count_named_objects(conn, PROCESSING_0032_SENTINELS)
        .map_err(|error| MigrationError::new(migration.name, error))?;
    if present as usize == PROCESSING_0032_SENTINELS.len() {
        return Ok(RepairPlan::RegistryRowOnly);
    }
    if present == 0 {
        return Ok(RepairPlan::None);
    }

    let occupied = count_surviving_processing_rows(conn)
        .map_err(|error| MigrationError::new(migration.name, error))?;
    if occupied > 0 {
        return Err(MigrationError {
            migration: migration.name,
            message: format!(
                "incomplete 0032 state: {present}/{} objects exist without a registry row and still hold {occupied} queue row(s); restore from a backup before retrying",
                PROCESSING_0032_SENTINELS.len()
            ),
        });
    }

    let drops = PROCESSING_0032_TRIGGERS
        .iter()
        .map(|trigger| format!("DROP TRIGGER IF EXISTS {trigger};"))
        .chain(
            PROCESSING_0032_TABLES_CHILD_FIRST
                .iter()
                .map(|table| format!("DROP TABLE IF EXISTS {table};")),
        )
        .collect::<Vec<_>>()
        .join("\n");
    Ok(RepairPlan::Drops(drops))
}

fn count_named_objects(conn: &Connection, names: &[&str]) -> rusqlite::Result<i64> {
    let placeholders = vec!["?"; names.len()].join(",");
    conn.query_row(
        &format!("SELECT COUNT(*) FROM sqlite_master WHERE name IN ({placeholders})"),
        params_from_iter(names.iter().copied()),
        |row| row.get(0),
    )
}

/// `countSurvivingProcessingRows` (`runner.ts`): total rows held by whichever
/// 0032 tables survive. A partial 0032 is only safe to replay when this is 0.
fn count_surviving_processing_rows(conn: &Connection) -> rusqlite::Result<i64> {
    let placeholders = vec!["?"; PROCESSING_0032_TABLES_CHILD_FIRST.len()].join(",");
    let mut statement = conn.prepare(&format!(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name IN ({placeholders})"
    ))?;
    let present: Vec<String> = statement
        .query_map(
            params_from_iter(PROCESSING_0032_TABLES_CHILD_FIRST.iter().copied()),
            |row| row.get(0),
        )?
        .collect::<rusqlite::Result<_>>()?;

    let mut total = 0i64;
    for table in present {
        // The names come from the constant above, never from user input.
        total += conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })?;
    }
    Ok(total)
}

/// The SQL the TypeScript registry holds: the file content minus one trailing
/// newline (`sql.replace(/\n$/, '')`).
fn executable_sql(migration: &Migration) -> &str {
    migration.sql.strip_suffix('\n').unwrap_or(migration.sql)
}

fn is_duplicate_column(error: &rusqlite::Error) -> bool {
    error
        .to_string()
        .to_lowercase()
        .contains("duplicate column name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::canonical_dump::{canonical_schema_dump, CanonicalSchemaDump};
    use serde_json::Value;
    use std::time::{Duration, Instant};

    const CANONICAL_FIXTURE: &str = include_str!("../../tests/fixtures/schema_canonical.json");
    const FIXED_CLOCK_MS: i64 = 1_700_000_000_123;

    fn fixed_clock() -> impl Fn() -> i64 {
        || FIXED_CLOCK_MS
    }

    fn test_connection() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .expect("enable foreign keys");
        conn
    }

    fn registry_index_after(name: &str) -> usize {
        REGISTRY
            .iter()
            .position(|migration| migration.name == name)
            .unwrap_or_else(|| panic!("the registry is missing {name}"))
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![name],
            |row| row.get::<_, i64>(0),
        )
        .expect("query sqlite_master")
            > 0
    }

    fn registry_row_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM _migrations WHERE name = ?1",
            params![name],
            |row| row.get::<_, i64>(0),
        )
        .expect("query _migrations")
            > 0
    }

    fn assert_matches_canonical_fixture(conn: &Connection, context: &str) {
        let expected: CanonicalSchemaDump =
            serde_json::from_str(CANONICAL_FIXTURE).expect("parse the canonical fixture");
        let actual = canonical_schema_dump(conn).expect("dump the Rust schema");
        if actual == expected {
            return;
        }
        let expected_value = serde_json::to_value(&expected).expect("serialize the fixture");
        let actual_value = serde_json::to_value(&actual).expect("serialize the Rust dump");
        let mut differences = Vec::new();
        collect_differences("$", &expected_value, &actual_value, &mut differences);
        panic!(
            "{context}: Rust runner dump differs from the TS fixture:\n{}",
            differences.join("\n")
        );
    }

    const MAX_DIFFERENCES: usize = 40;

    /// Reports the first differences instead of dumping two huge JSON blobs;
    /// arrays are deterministically ordered on both sides, so positions align.
    fn collect_differences(path: &str, expected: &Value, actual: &Value, out: &mut Vec<String>) {
        if out.len() >= MAX_DIFFERENCES {
            return;
        }
        match (expected, actual) {
            (Value::Object(expected_map), Value::Object(actual_map)) => {
                for (key, expected_value) in expected_map {
                    match actual_map.get(key) {
                        Some(actual_value) => {
                            collect_differences(
                                &format!("{path}.{key}"),
                                expected_value,
                                actual_value,
                                out,
                            );
                        }
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
                    collect_differences(
                        &format!("{path}[{index}]"),
                        expected_item,
                        actual_item,
                        out,
                    );
                }
            }
            _ if expected != actual => {
                out.push(format!("{path}: fixture={expected} Rust={actual}"));
            }
            _ => {}
        }
    }

    /// Extracts the names in runner.ts's atomic `else if` chain. Anchored on
    /// the first entry (0025) so the unrelated `name === '0032_batch_processing'`
    /// and `name === '0020_layouts'` mentions earlier in the file are skipped.
    fn atomic_names_from_runner(source: &str) -> Vec<String> {
        let anchor = "name === '0025_document_view_edits'";
        let start = source
            .find(anchor)
            .expect("runner.ts must contain the atomic condition");
        let condition = &source[start..];
        let end = condition
            .find("\n      ) {")
            .expect("the atomic condition must close with a parenthesized block");

        let mut names = Vec::new();
        let mut rest = &condition[..end];
        while let Some(position) = rest.find("name === '") {
            let after = &rest[position + "name === '".len()..];
            let quote = after
                .find('\'')
                .expect("an atomic name literal must close its quote");
            names.push(after[..quote].to_string());
            rest = &after[quote + 1..];
        }
        names
    }

    #[test]
    fn registry_names_match_the_migration_directory() {
        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../packages/store/src/migrations"
        );
        let mut from_directory: Vec<String> = std::fs::read_dir(directory)
            .expect("read the migrations directory")
            .map(|entry| {
                entry
                    .expect("read a directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.ends_with(".sql"))
            .map(|name| name.trim_end_matches(".sql").to_string())
            .collect();
        from_directory.sort();

        let from_registry: Vec<String> = REGISTRY
            .iter()
            .map(|migration| migration.name.to_string())
            .collect();
        assert_eq!(
            from_registry, from_directory,
            "the registry and src/migrations must list the same migrations in name order"
        );
        assert_eq!(
            from_registry.len(),
            58,
            "the Phase 4 shell must keep all 58 migrations"
        );
    }

    #[test]
    fn atomic_kinds_match_the_runner_atomic_condition() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../packages/store/src/runner.ts"
        ))
        .expect("read runner.ts");
        let expected = atomic_names_from_runner(&source);
        let actual: Vec<&str> = REGISTRY
            .iter()
            .filter(|migration| migration.kind == MigrationKind::Atomic)
            .map(|migration| migration.name)
            .collect();
        assert_eq!(
            actual, expected,
            "the Rust atomic list drifted from runner.ts"
        );
        assert_eq!(
            expected.len(),
            27,
            "the TS atomic condition changed shape; re-check the extractor"
        );
    }

    #[test]
    fn an_empty_database_matches_the_canonical_fixture() {
        let mut conn = test_connection();
        let report =
            apply_migrations_with(&mut conn, REGISTRY, &fixed_clock()).expect("apply everything");
        assert_eq!(
            report.applied.len(),
            58,
            "every registry migration must run once"
        );
        assert_matches_canonical_fixture(
            &conn,
            "after applying every migration to an empty database",
        );
    }

    #[test]
    fn a_second_run_applies_nothing_and_keeps_the_schema() {
        let mut conn = test_connection();
        apply_migrations_with(&mut conn, REGISTRY, &fixed_clock()).expect("first run");
        let second =
            apply_migrations_with(&mut conn, REGISTRY, &fixed_clock()).expect("second run");
        assert!(
            second.applied.is_empty(),
            "the second run must find nothing pending"
        );
        assert_matches_canonical_fixture(&conn, "after a second, no-op run");
    }

    #[test]
    fn incremental_runs_match_the_one_shot_dump() {
        for cut in [
            "0001_initial",
            "0019_llm_results_target_type",
            "0020_layouts",
            "0031_items_source_directory",
            "0032_batch_processing",
            "0044_processing_task_subject_cutover",
            "0057_web_captures",
        ] {
            let index = registry_index_after(cut);
            let mut conn = test_connection();
            apply_migrations_with(&mut conn, &REGISTRY[..=index], &fixed_clock())
                .expect("apply the prefix");
            let report = apply_migrations_with(&mut conn, REGISTRY, &fixed_clock())
                .expect("apply the remainder");
            assert!(
                !report.applied.is_empty(),
                "cut after {cut}: the remainder must be non-empty"
            );
            assert_matches_canonical_fixture(&conn, &format!("incremental run cut after {cut}"));
        }
    }

    #[test]
    fn a_legacy_layouts_table_is_deduped_and_backfilled() {
        let mut conn = test_connection();
        let index = registry_index_after("0020_layouts");
        apply_migrations_with(&mut conn, &REGISTRY[..index], &fixed_clock())
            .expect("apply everything before 0020");

        conn.execute_batch(
            "CREATE TABLE layouts (
                 id TEXT PRIMARY KEY,
                 asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                 regions TEXT NOT NULL,
                 model TEXT NOT NULL,
                 image_width INTEGER NOT NULL,
                 image_height INTEGER NOT NULL,
                 created_at INTEGER NOT NULL
             );
             INSERT INTO collections (id, name, created_at, updated_at) VALUES ('collection-1', 'Collection', 0, 0);
             INSERT INTO items (id, title, collection_id, created_at, updated_at) VALUES ('item-1', 'Item', 'collection-1', 0, 0);
             INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('asset-1', 'item-1', 'a.pdf', 'pdf', 0);
             INSERT INTO layouts (id, asset_id, regions, model, image_width, image_height, created_at)
               VALUES ('layout-old', 'asset-1', '[]', 'legacy-model', 10, 10, 0);
             INSERT INTO layouts (id, asset_id, regions, model, image_width, image_height, created_at)
               VALUES ('layout-new', 'asset-1', '[]', 'current-model', 10, 10, 0);",
        )
        .expect("create the legacy layouts state");

        let report =
            apply_migrations_with(&mut conn, REGISTRY, &fixed_clock()).expect("apply from 0020 on");
        assert!(report.applied.contains(&"0020_layouts"));

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM layouts", [], |row| row.get(0))
            .expect("count layouts");
        assert_eq!(count, 1, "the dedup must keep exactly one row per asset");
        // 0023 later rewrites every layout id to 'lay-' || asset_id, so the
        // dedup pick is observed through a column it leaves untouched.
        let survivor: (String, String, i64) = conn
            .query_row("SELECT model, blocks, created_at FROM layouts", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .expect("read the surviving row");
        assert_eq!(
            survivor.0, "current-model",
            "MAX(rowid) must be the one kept"
        );
        assert_eq!(
            survivor.1, "[]",
            "the ALTER must add blocks with its default"
        );
        assert_eq!(survivor.2, FIXED_CLOCK_MS, "created_at is backfilled in ms");
    }

    #[test]
    fn a_burned_0032_row_is_repaired_without_replaying_ddl() {
        let mut conn = test_connection();
        apply_migrations_with(&mut conn, REGISTRY, &fixed_clock()).expect("apply everything");
        conn.execute(
            "DELETE FROM _migrations WHERE name = '0032_batch_processing'",
            [],
        )
        .expect("burn the registry row");

        let report =
            apply_migrations_with(&mut conn, REGISTRY, &fixed_clock()).expect("repair the row");
        assert_eq!(
            report.applied,
            vec!["0032_batch_processing"],
            "only the registry row may be repaired"
        );
        assert_matches_canonical_fixture(&conn, "after the 0032 row-only repair");
    }

    #[test]
    fn a_partial_empty_0032_is_replayed_cleanly() {
        let mut conn = test_connection();
        let index = registry_index_after("0031_items_source_directory");
        apply_migrations_with(&mut conn, &REGISTRY[..=index], &fixed_clock())
            .expect("apply through 0031");
        conn.execute_batch("CREATE TABLE processing_batches (id TEXT PRIMARY KEY)")
            .expect("plant a partial 0032");

        let report =
            apply_migrations_with(&mut conn, REGISTRY, &fixed_clock()).expect("replay 0032");
        assert!(report.applied.contains(&"0032_batch_processing"));
        assert_matches_canonical_fixture(&conn, "after replaying a partial, empty 0032");
    }

    #[test]
    fn an_occupied_partial_0032_is_refused_without_committing() {
        let mut conn = test_connection();
        let index = registry_index_after("0031_items_source_directory");
        apply_migrations_with(&mut conn, &REGISTRY[..=index], &fixed_clock())
            .expect("apply through 0031");
        conn.execute_batch(
            "CREATE TABLE processing_batches (id TEXT PRIMARY KEY);
             INSERT INTO processing_batches (id) VALUES ('batch-1');",
        )
        .expect("plant an occupied partial 0032");

        let error = apply_migrations_with(&mut conn, REGISTRY, &fixed_clock())
            .expect_err("must refuse to drop queue rows");
        assert_eq!(error.migration, "0032_batch_processing");
        assert!(
            error.message.contains("incomplete 0032 state"),
            "unexpected message: {}",
            error.message
        );
        assert!(
            error.message.contains("1 queue row"),
            "unexpected message: {}",
            error.message
        );

        let survived: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_batches WHERE id = 'batch-1'",
                [],
                |row| row.get(0),
            )
            .expect("count the queue row");
        assert_eq!(survived, 1, "the occupied table must survive the refusal");
        assert!(
            !registry_row_exists(&conn, "0032_batch_processing"),
            "the refusal must not register 0032"
        );
    }

    #[test]
    fn a_failing_batch_reports_sqlite_message_without_the_migration_sql() {
        // rusqlite's Display for input errors appends the whole statement,
        // which for an atomic migration is the entire file; the startup dialog
        // must show SQLite's message only.
        static CUSTOM: &[Migration] = &[Migration {
            name: "0001_broken_batch",
            sql: "CREATE TABLE batch_t (id INTEGER PRIMARY KEY);\nCREATE TABLE batch_t (id INTEGER PRIMARY KEY);",
            kind: MigrationKind::Atomic,
        }];
        let mut conn = test_connection();
        let error = apply_migrations_with(&mut conn, CUSTOM, &fixed_clock())
            .expect_err("the duplicate table must fail");
        assert_eq!(error.migration, "0001_broken_batch");
        assert_eq!(error.message, "table batch_t already exists");
    }

    #[test]
    fn a_failing_migration_rolls_back_its_statements_and_keeps_earlier_work() {
        static CUSTOM: &[Migration] = &[
            Migration {
                name: "0001_first",
                sql: "CREATE TABLE first_t (id INTEGER PRIMARY KEY);",
                kind: MigrationKind::PerStatement,
            },
            Migration {
                name: "0002_broken",
                sql: "CREATE TABLE second_t (id INTEGER PRIMARY KEY);\nCREATE TABLE second_t (id INTEGER PRIMARY KEY);",
                kind: MigrationKind::PerStatement,
            },
        ];
        let mut conn = test_connection();
        let error = apply_migrations_with(&mut conn, CUSTOM, &fixed_clock())
            .expect_err("the second statement must fail");
        assert_eq!(error.migration, "0002_broken");
        assert!(
            error.message.contains("already exists"),
            "unexpected message: {}",
            error.message
        );
        assert!(
            table_exists(&conn, "first_t"),
            "the earlier migration must stay committed"
        );
        assert!(registry_row_exists(&conn, "0001_first"));
        assert!(
            !table_exists(&conn, "second_t"),
            "the failed migration's first statement must roll back"
        );
        assert!(
            !registry_row_exists(&conn, "0002_broken"),
            "the failed migration must not be registered"
        );
    }

    #[test]
    fn duplicate_column_is_tolerated_per_statement() {
        static CUSTOM: &[Migration] = &[Migration {
            name: "0001_duplicate_column",
            sql: "CREATE TABLE dup_t (x INTEGER);\nALTER TABLE dup_t ADD COLUMN x INTEGER;\nCREATE TABLE after_dup_t (id INTEGER PRIMARY KEY);",
            kind: MigrationKind::PerStatement,
        }];
        let mut conn = test_connection();
        let report = apply_migrations_with(&mut conn, CUSTOM, &fixed_clock())
            .expect("duplicate column must be tolerated");
        assert_eq!(report.applied, vec!["0001_duplicate_column"]);
        assert!(
            table_exists(&conn, "after_dup_t"),
            "the transaction must stay usable after the tolerated failure"
        );
        assert!(registry_row_exists(&conn, "0001_duplicate_column"));
    }

    #[test]
    fn duplicate_column_fails_inside_an_atomic_migration() {
        static CUSTOM: &[Migration] = &[Migration {
            name: "0001_atomic_duplicate",
            sql: "CREATE TABLE atomic_dup_t (x INTEGER);\nALTER TABLE atomic_dup_t ADD COLUMN x INTEGER;",
            kind: MigrationKind::Atomic,
        }];
        let mut conn = test_connection();
        let error = apply_migrations_with(&mut conn, CUSTOM, &fixed_clock())
            .expect_err("atomic migrations do not tolerate duplicate columns");
        assert_eq!(error.migration, "0001_atomic_duplicate");
        assert!(
            error
                .message
                .to_lowercase()
                .contains("duplicate column name"),
            "unexpected message: {}",
            error.message
        );
        assert!(
            !table_exists(&conn, "atomic_dup_t"),
            "the atomic batch must roll back"
        );
        assert!(!registry_row_exists(&conn, "0001_atomic_duplicate"));
    }

    /// Fills the pre-0034 schema with a synthetic large archive: one
    /// collection, `items` items each carrying one asset, one extraction and
    /// one transcription, every text about `text_bytes` long. 0034's rebuild
    /// reads exactly items/assets/extractions/transcriptions (0018 and 0034
    /// SQL), so all four tables carry rows.
    fn seed_synthetic_archive(conn: &Connection, items: u64, text_bytes: usize) {
        const FILLER: &str = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore";
        let mut body = String::with_capacity(text_bytes + FILLER.len());
        while body.len() < text_bytes {
            body.push_str(FILLER);
            body.push(' ');
        }

        conn.execute_batch("BEGIN")
            .expect("begin the seed transaction");
        conn.execute(
            "INSERT INTO collections (id, name, created_at, updated_at)
             VALUES ('collection-1', 'Synthetic', 0, 0)",
            [],
        )
        .expect("insert the collection");
        {
            let mut insert_item = conn
                .prepare(
                    "INSERT INTO items (id, title, collection_id, created_at, updated_at)
                     VALUES (?1, ?2, 'collection-1', 0, 0)",
                )
                .expect("prepare the item insert");
            let mut insert_asset = conn
                .prepare(
                    "INSERT INTO assets (id, item_id, path, type, created_at)
                     VALUES (?1, ?2, ?3, 'pdf', 0)",
                )
                .expect("prepare the asset insert");
            let mut insert_extraction = conn
                .prepare(
                    "INSERT INTO extractions (id, asset_id, text_content, method, created_at)
                     VALUES (?1, ?2, ?3, 'native', 0)",
                )
                .expect("prepare the extraction insert");
            let mut insert_transcription = conn
                .prepare(
                    "INSERT INTO transcriptions (id, asset_id, text_content, model, created_at)
                     VALUES (?1, ?2, ?3, 'synthetic', 0)",
                )
                .expect("prepare the transcription insert");

            for index in 0..items {
                let id = index.to_string();
                insert_item
                    .execute(params![format!("item-{id}"), format!("Item {id}")])
                    .expect("insert an item");
                insert_asset
                    .execute(params![
                        format!("asset-{id}"),
                        format!("item-{id}"),
                        format!("assets/{id}.pdf")
                    ])
                    .expect("insert an asset");
                insert_extraction
                    .execute(params![
                        format!("extraction-{id}"),
                        format!("asset-{id}"),
                        format!("item {id} {body}")
                    ])
                    .expect("insert an extraction");
                insert_transcription
                    .execute(params![
                        format!("transcription-{id}"),
                        format!("asset-{id}"),
                        format!("audio {id} {body}")
                    ])
                    .expect("insert a transcription");
            }
        }
        conn.execute_batch("COMMIT")
            .expect("commit the seed transaction");
    }

    /// Plan §6 risk probe, not a regression gate: times the migrations left
    /// after the 0033 cut on a synthetic large archive (10k items, one asset,
    /// one ~2 KB extraction and one ~2 KB transcription each). Run it
    /// deliberately, in release:
    ///
    /// ```text
    /// cargo test --release --lib -- --ignored measure_upgrade --nocapture
    /// ```
    #[test]
    #[ignore = "measurement probe; run explicitly with cargo test --release -- --ignored"]
    fn measure_upgrade_from_0033_on_a_large_archive() {
        const ITEM_COUNT: u64 = 10_000;
        const TEXT_BYTES: usize = 2_048;

        let cut = registry_index_after("0033_processing_source_invalidation");
        let mut conn = test_connection();
        apply_migrations_with(&mut conn, &REGISTRY[..=cut], &fixed_clock())
            .expect("apply through the 0033 cut");
        seed_synthetic_archive(&conn, ITEM_COUNT, TEXT_BYTES);

        let mut total = Duration::ZERO;
        for index in cut + 1..REGISTRY.len() {
            let name = REGISTRY[index].name;
            let started = Instant::now();
            let report = apply_migrations_with(&mut conn, &REGISTRY[..=index], &fixed_clock())
                .expect("apply the next migration");
            let elapsed = started.elapsed();
            assert_eq!(
                report.applied,
                vec![name],
                "the step must apply only {name}"
            );
            total += elapsed;
            if elapsed.as_millis() > 50 {
                println!("measure_upgrade: {name} took {} ms", elapsed.as_millis());
            }
        }
        println!(
            "measure_upgrade_from_0033_on_a_large_archive: {} migrations, {ITEM_COUNT} items, {TEXT_BYTES} bytes/text, total {} ms",
            REGISTRY.len() - cut - 1,
            total.as_millis()
        );
    }

    #[test]
    fn applied_at_is_stored_in_seconds() {
        let mut conn = test_connection();
        apply_migrations_with(&mut conn, &REGISTRY[..1], &fixed_clock()).expect("apply 0001");
        let applied_at: i64 = conn
            .query_row(
                "SELECT applied_at FROM _migrations WHERE name = '0001_initial'",
                [],
                |row| row.get(0),
            )
            .expect("read applied_at");
        assert_eq!(applied_at, FIXED_CLOCK_MS / 1000);
        assert_eq!(applied_at, 1_700_000_000);
    }
}
