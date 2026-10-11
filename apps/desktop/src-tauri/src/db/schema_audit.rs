//! Canonical trigger/view audit at startup (plan §4.4b).
//!
//! The renderer could run DDL until Phase 4.4a closed the authorizer, so a
//! trigger or view planted before that window may still be installed — even
//! one wearing the name of a legitimate object, with a body that disables
//! capture or rewrites data under backend writes. Names prove nothing; only
//! the exact definition does.
//!
//! [`audit_and_repair`] rebuilds the canonical trigger/view set in an
//! in-memory database shaped by the same Rust code paths that shape a real
//! archive, then compares `sqlite_master` object by object. Anything absent
//! from the canonical set, or whose `type`/`tbl_name`/`sql` differs, is
//! dropped and the canonical definition is restored — all inside one
//! transaction, before `ui_conn` reaches the renderer.
//!
//! The canonical build replays the migrations instead of `migrate_archive`:
//! no repair creates a trigger or a view (pinned by
//! `the_canonical_build_covers_every_trigger_and_view_the_repairs_leave_behind`),
//! and running the data repairs against an empty database would print a
//! misleading "tables not found" notice on every startup.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use rusqlite::Connection;

use crate::sync::capture;

/// Debug-build budget for the canonical build. The audit runs on every
/// startup; past this the build is reported as over budget.
pub(crate) const CANONICAL_BUILD_BUDGET_MS: u128 = 150;

/// One `sqlite_master` row among the audited object classes.
///
/// `sqlite_master` lets a trigger and a view share the same name, so the audit
/// keys every object on `(type, name)` — never on the name alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaObject {
    pub(crate) object_type: String,
    pub(crate) name: String,
    pub(crate) tbl_name: String,
    pub(crate) sql: String,
}

/// Why a non-canonical object was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropReason {
    /// No canonical object carries this `(type, name)`.
    NotCanonical,
    /// A canonical object carries this `(type, name)`, but `tbl_name` or the
    /// exact `sql` text differs.
    DefinitionDiffers,
}

impl DropReason {
    /// Short English description shown next to the object in the startup log.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::NotCanonical => "not in the canonical set",
            Self::DefinitionDiffers => "definition differs from the canonical one",
        }
    }
}

/// One object the audit dropped, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DroppedObject {
    pub(crate) object_type: String,
    pub(crate) name: String,
    pub(crate) reason: DropReason,
}

/// Outcome of one audit.
///
/// `recreated` lists the canonical objects that were missing, differed or were
/// dropped and needed restoring. A `trg_sync_*` entry means every sync trigger
/// was rebuilt through `drop_all_sync_triggers` + `ensure_capture`, because
/// `create_trigger_sql` uses `IF NOT EXISTS` and would not replace a planted
/// body wearing a legitimate name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaAuditReport {
    pub(crate) canonical_build_ms: u128,
    pub(crate) dropped: Vec<DroppedObject>,
    pub(crate) recreated: Vec<String>,
}

impl SchemaAuditReport {
    /// True when the archive matched the canonical set exactly.
    pub(crate) fn is_clean(&self) -> bool {
        self.dropped.is_empty() && self.recreated.is_empty()
    }
}

/// Objects keyed by `(type, name)`; iteration order is deterministic.
type ObjectMap = BTreeMap<(String, String), SchemaObject>;

/// Audits `conn`'s triggers and views against the canonical set and repairs
/// every divergence inside a single transaction.
///
/// Must run while no other connection reads or writes the archive (in `setup`,
/// right after `migrate_archive`) so a write cannot slip in between the
/// comparison and the repair.
pub(crate) fn audit_and_repair(
    conn: &mut Connection,
    data_dir: &Path,
) -> Result<SchemaAuditReport, String> {
    let started = Instant::now();
    let canonical = build_canonical(data_dir)?;
    let canonical_build_ms = started.elapsed().as_millis();

    let canonical_objects = read_objects(&canonical)?;
    let real_objects = read_objects(conn)?;

    let mut dropped = Vec::new();
    let mut dropped_keys = BTreeSet::new();
    for (key, real) in &real_objects {
        let reason = match canonical_objects.get(key) {
            None => Some(DropReason::NotCanonical),
            Some(expected) if expected != real => Some(DropReason::DefinitionDiffers),
            Some(_) => None,
        };
        if let Some(reason) = reason {
            dropped.push(DroppedObject {
                object_type: real.object_type.clone(),
                name: real.name.clone(),
                reason,
            });
            dropped_keys.insert(key.clone());
        }
    }

    let mut recreate = Vec::new();
    let mut sync_recreate = Vec::new();
    for (key, expected) in &canonical_objects {
        let needs_repair = match real_objects.get(key) {
            None => true,
            Some(real) => dropped_keys.contains(key) || real != expected,
        };
        if !needs_repair {
            continue;
        }
        if expected.object_type == "trigger" && expected.name.starts_with("trg_sync_") {
            sync_recreate.push(expected.name.clone());
        } else {
            recreate.push(expected.clone());
        }
    }

    let mut recreated: Vec<String> = recreate
        .iter()
        .map(|object| object.name.clone())
        .chain(sync_recreate.iter().cloned())
        .collect();
    recreated.sort();

    let report = SchemaAuditReport {
        canonical_build_ms,
        dropped,
        recreated,
    };

    if report.is_clean() {
        return Ok(report);
    }

    // One transaction for the whole repair: either the archive ends up exactly
    // canonical or it is left exactly as it was.
    let transaction = conn
        .transaction()
        .map_err(|error| format!("Failed to open the schema-audit transaction: {error}"))?;
    for object in &report.dropped {
        drop_object(&transaction, object)?;
    }
    for object in &recreate {
        transaction.execute_batch(&object.sql).map_err(|error| {
            format!(
                "Failed to restore canonical {} \"{}\": {error}",
                object.object_type, object.name
            )
        })?;
    }
    if !sync_recreate.is_empty() {
        capture::drop_all_sync_triggers(&transaction)?;
        capture::ensure_capture(&transaction)?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Failed to commit the schema audit: {error}"))?;

    Ok(report)
}

/// Builds the canonical trigger/view set in memory: the migrations the runner
/// applies to an empty archive, the relative asset-path guard, and the sync
/// capture bootstrap — the three production sources of triggers and views.
fn build_canonical(data_dir: &Path) -> Result<Connection, String> {
    let mut canonical = Connection::open_in_memory()
        .map_err(|error| format!("Failed to open the canonical database: {error}"))?;
    canonical
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|error| format!("Failed to configure the canonical database: {error}"))?;
    crate::db::migrations::apply_migrations(&mut canonical)
        .map_err(|error| format!("Failed to migrate the canonical database: {error}"))?;
    crate::install_relative_asset_path_guard(&canonical, data_dir)?;
    capture::ensure_capture(&canonical)?;
    Ok(canonical)
}

/// Reads every trigger and view from `sqlite_master`, keyed by `(type, name)`.
fn read_objects(conn: &Connection) -> Result<ObjectMap, String> {
    let mut statement = conn
        .prepare(
            "SELECT type, name, tbl_name, sql FROM sqlite_master
             WHERE type IN ('trigger', 'view') AND name NOT LIKE 'sqlite_%'
             ORDER BY type, name",
        )
        .map_err(|error| format!("Failed to list schema objects: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(SchemaObject {
                object_type: row.get(0)?,
                name: row.get(1)?,
                tbl_name: row.get(2)?,
                sql: row.get(3)?,
            })
        })
        .map_err(|error| format!("Failed to read schema objects: {error}"))?;

    let mut objects = ObjectMap::new();
    for row in rows {
        let object = row.map_err(|error| format!("Failed to read a schema object: {error}"))?;
        objects.insert((object.object_type.clone(), object.name.clone()), object);
    }
    Ok(objects)
}

/// Drops one audited object, quoting its identifier safely.
fn drop_object(conn: &Connection, object: &DroppedObject) -> Result<(), String> {
    let keyword = match object.object_type.as_str() {
        "trigger" => "TRIGGER",
        "view" => "VIEW",
        other => {
            return Err(format!(
                "Unsupported schema object type \"{other}\" for \"{}\"",
                object.name
            ))
        }
    };
    let identifier = format!("\"{}\"", object.name.replace('"', "\"\""));
    conn.execute_batch(&format!("DROP {keyword} IF EXISTS {identifier};"))
        .map_err(|error| format!("Failed to drop {keyword} \"{}\": {error}", object.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::canonical_dump::canonical_schema_dump;
    use crate::db::repairs::migrate_archive;
    use crate::install_relative_asset_path_guard;
    use crate::sync::capture::ensure_capture;
    use crate::sync::test_support::{
        oplog_count, oplog_count_for, set_session_with_capture, trg_sync_count,
    };
    use rusqlite::OptionalExtension;

    /// Mirrors the shape `setup` builds before the audit: the full archive
    /// pipeline, the relative asset-path guard, and the capture triggers.
    fn archived_db(data_dir: &Path) -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory archive");
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .expect("configure pragmas");
        migrate_archive(&mut conn).expect("migrate the archive");
        install_relative_asset_path_guard(&conn, data_dir).expect("install the guard");
        ensure_capture(&conn).expect("ensure capture");
        conn
    }

    fn sql_of(conn: &Connection, object_type: &str, name: &str) -> Option<String> {
        conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type = ?1 AND name = ?2",
            rusqlite::params![object_type, name],
            |row| row.get(0),
        )
        .optional()
        .expect("read sqlite_master")
        .flatten()
    }

    fn plant_malicious_collection_activity_trigger(conn: &Connection, leading_comment: bool) {
        let comment = if leading_comment { "-- planted\n" } else { "" };
        conn.execute_batch(&format!(
            "DROP TRIGGER collection_activity_items_update;
             {comment}CREATE TRIGGER collection_activity_items_update
             AFTER UPDATE ON items
             BEGIN
               UPDATE sync_meta SET value = '0' WHERE key = 'capture_enabled';
             END;"
        ))
        .expect("plant the malicious trigger");
    }

    #[test]
    fn a_clean_archive_is_left_untouched_and_the_audit_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = archived_db(dir.path());
        let canonical = build_canonical(dir.path()).expect("build canonical");

        let before = canonical_schema_dump(&conn).expect("dump before");
        let report = audit_and_repair(&mut conn, dir.path()).expect("audit");
        eprintln!(
            "[schema-audit] clean-path canonical build: {} ms",
            report.canonical_build_ms
        );

        assert!(
            report.is_clean(),
            "a clean archive must not report findings: {report:?}"
        );
        let after = canonical_schema_dump(&conn).expect("dump after");
        assert_eq!(before, after, "the audit changed a clean schema");
        assert_eq!(
            read_objects(&conn).expect("real objects"),
            read_objects(&canonical).expect("canonical objects"),
            "the clean archive must match the canonical trigger/view set"
        );

        // The runtime-installed guard triggers are canonical and must survive.
        assert!(sql_of(&conn, "trigger", "trg_assets_path_must_be_relative").is_some());
        assert!(sql_of(&conn, "trigger", "trg_assets_path_must_be_relative_update").is_some());

        let second = audit_and_repair(&mut conn, dir.path()).expect("second audit");
        assert!(second.is_clean(), "the audit must be idempotent");
    }

    #[test]
    fn an_extra_trigger_on_items_is_dropped_and_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = archived_db(dir.path());
        conn.execute_batch(
            "CREATE TRIGGER planted_items_extra AFTER INSERT ON items BEGIN SELECT 1; END;",
        )
        .expect("plant trigger");

        let report = audit_and_repair(&mut conn, dir.path()).expect("audit");

        assert!(report.dropped.iter().any(|object| {
            object.name == "planted_items_extra"
                && object.object_type == "trigger"
                && object.reason == DropReason::NotCanonical
        }));
        assert!(report.recreated.is_empty());
        assert!(sql_of(&conn, "trigger", "planted_items_extra").is_none());
        assert!(audit_and_repair(&mut conn, dir.path())
            .expect("second audit")
            .is_clean());
    }

    #[test]
    fn an_extra_view_is_dropped_and_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = archived_db(dir.path());
        conn.execute_batch("CREATE VIEW planted_view AS SELECT id FROM items;")
            .expect("plant view");

        let report = audit_and_repair(&mut conn, dir.path()).expect("audit");

        assert!(report.dropped.iter().any(|object| {
            object.name == "planted_view"
                && object.object_type == "view"
                && object.reason == DropReason::NotCanonical
        }));
        assert!(sql_of(&conn, "view", "planted_view").is_none());
        assert!(audit_and_repair(&mut conn, dir.path())
            .expect("second audit")
            .is_clean());
    }

    fn assert_replaced_collection_activity_trigger_is_restored(leading_comment: bool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = archived_db(dir.path());
        plant_malicious_collection_activity_trigger(&conn, leading_comment);

        let canonical = build_canonical(dir.path()).expect("build canonical");
        let expected = sql_of(&canonical, "trigger", "collection_activity_items_update")
            .expect("canonical sql");

        let report = audit_and_repair(&mut conn, dir.path()).expect("audit");

        assert!(report.dropped.iter().any(|object| {
            object.name == "collection_activity_items_update"
                && object.reason == DropReason::DefinitionDiffers
        }));
        assert!(
            report
                .recreated
                .iter()
                .any(|name| name == "collection_activity_items_update"),
            "the restored trigger must be reported: {report:?}"
        );
        assert_eq!(
            sql_of(&conn, "trigger", "collection_activity_items_update").as_deref(),
            Some(expected.as_str()),
            "sqlite_master.sql must be the canonical definition"
        );
        assert!(audit_and_repair(&mut conn, dir.path())
            .expect("second audit")
            .is_clean());
    }

    #[test]
    fn a_replaced_collection_activity_trigger_is_restored_to_the_canonical_definition() {
        assert_replaced_collection_activity_trigger_is_restored(false);
    }

    #[test]
    fn a_replaced_collection_activity_trigger_with_a_leading_comment_is_still_restored() {
        assert_replaced_collection_activity_trigger_is_restored(true);
    }

    #[test]
    fn an_altered_sync_trigger_is_rebuilt_from_the_canonical_definition() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = archived_db(dir.path());
        conn.execute_batch(
            "DROP TRIGGER trg_sync_items_i;
             CREATE TRIGGER trg_sync_items_i
             AFTER INSERT ON items
             BEGIN
               SELECT 1;
             END;",
        )
        .expect("plant the altered sync trigger");

        let canonical = build_canonical(dir.path()).expect("build canonical");
        let expected = sql_of(&canonical, "trigger", "trg_sync_items_i").expect("canonical sql");

        let report = audit_and_repair(&mut conn, dir.path()).expect("audit");

        assert!(report
            .recreated
            .iter()
            .any(|name| name == "trg_sync_items_i"));
        assert_eq!(
            sql_of(&conn, "trigger", "trg_sync_items_i").as_deref(),
            Some(expected.as_str()),
            "sqlite_master.sql must be the canonical definition"
        );
        assert_eq!(trg_sync_count(&conn), 54, "every sync trigger is back");
        assert!(audit_and_repair(&mut conn, dir.path())
            .expect("second audit")
            .is_clean());
    }

    #[test]
    fn capture_stays_enabled_and_backend_writes_are_queued_after_an_audit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = archived_db(dir.path());
        set_session_with_capture(&conn);
        plant_malicious_collection_activity_trigger(&conn, false);

        let report = audit_and_repair(&mut conn, dir.path()).expect("audit");
        assert!(!report.is_clean(), "the planted trigger is a finding");

        let capture_enabled: String = conn
            .query_row(
                "SELECT value FROM sync_meta WHERE key = 'capture_enabled'",
                [],
                |row| row.get(0),
            )
            .expect("read capture flag");
        assert_eq!(capture_enabled, "1");

        conn.execute(
            "INSERT INTO collections(id, name, created_at, updated_at) VALUES('c1', 'C', 1, 1)",
            [],
        )
        .expect("insert collection");
        conn.execute(
            "INSERT INTO items(id, title, collection_id, created_at, updated_at)
             VALUES('i1', 'I', 'c1', 1, 1)",
            [],
        )
        .expect("insert item");

        assert_eq!(oplog_count_for(&conn, "items"), 1);
    }

    #[test]
    fn a_first_boot_archive_gets_the_guard_and_every_sync_trigger_restored() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Mirror a first boot: the guard runs before the migrations create
        // `assets` (so it is a no-op) and `ensure_capture_on_path` has not run
        // yet, so neither the guard triggers nor the sync triggers exist.
        let mut conn = Connection::open_in_memory().expect("open in-memory archive");
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .expect("configure pragmas");
        migrate_archive(&mut conn).expect("migrate the archive");
        assert!(sql_of(&conn, "trigger", "trg_assets_path_must_be_relative").is_none());
        assert_eq!(trg_sync_count(&conn), 0);

        let report = audit_and_repair(&mut conn, dir.path()).expect("audit");

        assert!(report
            .recreated
            .iter()
            .any(|name| name == "trg_assets_path_must_be_relative"));
        assert!(report
            .recreated
            .iter()
            .any(|name| name == "trg_sync_items_i"));
        assert_eq!(trg_sync_count(&conn), 54, "every sync trigger is installed");
        let canonical = build_canonical(dir.path()).expect("build canonical");
        for name in [
            "trg_assets_path_must_be_relative",
            "trg_assets_path_must_be_relative_update",
        ] {
            assert_eq!(
                sql_of(&conn, "trigger", name),
                sql_of(&canonical, "trigger", name),
                "{name} must be restored from the canonical definition"
            );
        }
        assert!(audit_and_repair(&mut conn, dir.path())
            .expect("second audit")
            .is_clean());
    }

    #[test]
    fn a_planted_view_sharing_a_canonical_trigger_name_is_dropped_without_touching_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = archived_db(dir.path());
        let trigger_before = sql_of(&conn, "trigger", "trg_sync_items_i");
        conn.execute_batch("CREATE VIEW trg_sync_items_i AS SELECT id FROM items;")
            .expect("plant the view");

        let report = audit_and_repair(&mut conn, dir.path()).expect("audit");

        assert!(report.dropped.iter().any(|object| {
            object.name == "trg_sync_items_i"
                && object.object_type == "view"
                && object.reason == DropReason::NotCanonical
        }));
        assert!(sql_of(&conn, "view", "trg_sync_items_i").is_none());
        assert_eq!(
            sql_of(&conn, "trigger", "trg_sync_items_i"),
            trigger_before,
            "the canonical trigger must survive dropping its namesake view"
        );
        assert!(report.recreated.is_empty());
    }

    #[test]
    fn an_archive_migrated_with_historical_trigger_text_is_repaired_without_touching_data_or_oplog()
    {
        // Historical divergence (f324927a, 2026-10-01): 0045, 0048 and 0052
        // rebuild processing_tasks/processing_batch_tasks, and before that fix
        // the rebuilds dropped processing_tasks_settle_dependents (created by
        // 0038) without restoring it. Archives that applied those migrations
        // pre-fix — and never re-ran a rebuild — carry no such trigger row in
        // sqlite_master. No fixture file holds the old text: the historical
        // shape is the ABSENCE of the row, reproduced with one DROP.
        let dir = tempfile::tempdir().expect("tempdir");
        // archived_db runs the app's own pipeline (migrate_archive, the guard,
        // ensure_capture), so trg_sync_* exist exactly as at startup.
        let mut conn = archived_db(dir.path());

        // Seed data rows in tables the affected triggers touch, BEFORE capture
        // is enabled: the collection_activity cascade would otherwise queue
        // its own capture rows and the oplog could not serve as a precise
        // delta gauge below.
        conn.execute(
            "INSERT INTO collections(id, name, created_at, updated_at)
             VALUES('c-hist', 'Historical', 1, 1)",
            [],
        )
        .expect("seed collection");
        conn.execute(
            "INSERT INTO collections(id, name, created_at, updated_at)
             VALUES('c-raw', 'Unversioned', 1, 1)",
            [],
        )
        .expect("seed unversioned collection");
        conn.execute(
            "INSERT INTO items(id, title, collection_id, created_at, updated_at)
             VALUES('i-hist', 'Item', 'c-hist', 1, 1)",
            [],
        )
        .expect("seed item");
        conn.execute(
            "INSERT INTO processing_tasks(id, kind, asset_id_snapshot, state, created_at, updated_at)
             VALUES('t-hist', 'ocr', 'a-hist', 'pending', 1, 1)",
            [],
        )
        .expect("seed queue task");
        // Mark sync as capturing, the way the code detects it.
        set_session_with_capture(&conn);
        // Simulate already-synced rows: a reseed must never re-enqueue them.
        // c-raw is deliberately left unversioned: any rebuild-triggered reseed
        // would enqueue exactly one 'U' row for it and turn the delta nonzero.
        conn.execute(
            "INSERT INTO sync_row_versions(table_name, row_id, server_seq)
             VALUES('collections', 'c-hist', 7), ('items', 'i-hist', 8)",
            [],
        )
        .expect("version the seeded rows");
        let oplog_before = oplog_count(&conn);
        assert_eq!(oplog_before, 0, "the oplog starts empty");

        // Historical shape: the settle trigger is gone (pre-f324927a 0045/0048/0052).
        conn.execute_batch("DROP TRIGGER IF EXISTS processing_tasks_settle_dependents;")
            .expect("drop the settle trigger");
        assert!(sql_of(&conn, "trigger", "processing_tasks_settle_dependents").is_none());

        // Whitespace-only divergence: byte-identical modulo spaces must still
        // be caught — names prove nothing, only the exact definition does.
        let canonical = build_canonical(dir.path()).expect("build canonical");
        let activity_sql = sql_of(&canonical, "trigger", "collection_activity_items_update")
            .expect("canonical activity sql");
        let whitespaced = activity_sql.replacen("AFTER UPDATE", "AFTER  UPDATE", 1);
        assert_ne!(whitespaced, activity_sql);
        assert_eq!(
            whitespaced.split_whitespace().collect::<Vec<_>>(),
            activity_sql.split_whitespace().collect::<Vec<_>>(),
            "the planted difference must be whitespace-only"
        );
        conn.execute_batch(&format!(
            "DROP TRIGGER collection_activity_items_update; {whitespaced}"
        ))
        .expect("plant the whitespace trigger");

        // a) the audit reports the divergent objects — and nothing unrelated.
        let report = audit_and_repair(&mut conn, dir.path()).expect("audit");
        assert_eq!(
            report.dropped,
            vec![DroppedObject {
                object_type: "trigger".to_string(),
                name: "collection_activity_items_update".to_string(),
                reason: DropReason::DefinitionDiffers,
            }],
            "only the whitespace-divergent trigger is dropped: {report:?}"
        );
        assert_eq!(
            report.recreated,
            vec![
                "collection_activity_items_update".to_string(),
                "processing_tasks_settle_dependents".to_string(),
            ],
            "the missing historical trigger is recreated too: {report:?}"
        );

        // b) every trigger/view now equals the canonical set.
        assert_eq!(
            read_objects(&conn).expect("real objects"),
            read_objects(&canonical).expect("canonical objects"),
            "sqlite_master must match the canonical trigger/view set"
        );

        // c) seeded data rows are unchanged (counts and values).
        for (table, id) in [
            ("collections", "c-hist"),
            ("collections", "c-raw"),
            ("items", "i-hist"),
            ("processing_tasks", "t-hist"),
        ] {
            let count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE id = '{id}'"),
                    [],
                    |row| row.get(0),
                )
                .expect("count the seeded row");
            assert_eq!(count, 1, "the seeded {table} row must survive");
        }
        let collection: (String, String) = conn
            .query_row(
                "SELECT id, name FROM collections WHERE id = 'c-hist'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read the collection");
        assert_eq!(collection, ("c-hist".to_string(), "Historical".to_string()));
        let item: (String, String, String) = conn
            .query_row(
                "SELECT id, title, collection_id FROM items WHERE id = 'i-hist'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read the item");
        assert_eq!(
            item,
            (
                "i-hist".to_string(),
                "Item".to_string(),
                "c-hist".to_string()
            )
        );
        let task: (String, String, String) = conn
            .query_row(
                "SELECT id, kind, state FROM processing_tasks WHERE id = 't-hist'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read the queue task");
        assert_eq!(
            task,
            (
                "t-hist".to_string(),
                "ocr".to_string(),
                "pending".to_string()
            )
        );

        // d) sync triggers exist and the oplog reseed is bounded: neither the
        // versioned rows nor the unversioned c-raw row are re-enqueued, so
        // the oplog delta is exactly zero.
        assert_eq!(trg_sync_count(&conn), 54, "every sync trigger is back");
        assert_eq!(
            oplog_count(&conn),
            oplog_before,
            "the audit must not re-enqueue any row"
        );
        assert_eq!(oplog_count_for(&conn, "collections"), 0);
        assert_eq!(oplog_count_for(&conn, "items"), 0);

        // e) a second audit run reports no divergence (idempotent).
        assert!(
            audit_and_repair(&mut conn, dir.path())
                .expect("second audit")
                .is_clean(),
            "the audit must be idempotent"
        );
    }

    #[test]
    fn the_canonical_build_covers_every_trigger_and_view_the_repairs_leave_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let canonical = build_canonical(dir.path()).expect("build canonical");

        let mut archived = Connection::open_in_memory().expect("open in-memory archive");
        archived
            .execute_batch("PRAGMA foreign_keys=ON;")
            .expect("configure pragmas");
        migrate_archive(&mut archived).expect("migrate the archive");
        install_relative_asset_path_guard(&archived, dir.path()).expect("install the guard");
        ensure_capture(&archived).expect("ensure capture");

        assert_eq!(
            read_objects(&canonical).expect("canonical objects"),
            read_objects(&archived).expect("archived objects"),
            "a repair that creates a trigger or view would be dropped by every audit"
        );
    }
}
