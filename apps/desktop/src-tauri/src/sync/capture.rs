//! Capture triggers (DESIGN §4.1, §6.1). 45 triggers (15 synced tables × 3 ops)
//! mark dirty rows in `sync_oplog`. Capture is gated by session: each trigger
//! only fires when `capture_enabled='1'` and `applying<>'1'` (echo suppression
//! during pull apply). `ensure_capture` is self-healing across schema rebuilds.

use rusqlite::Connection;

use crate::sync::schema::ensure_sync_schema;

/// Compiled version of the trigger template. Bump this whenever the trigger DDL
/// in [`create_trigger_sql`] changes so `ensure_capture` performs a
/// DROP-all-then-create upgrade (DESIGN §6.1.2).
pub const TRIGGERS_VERSION: &str = "1";

/// The 15 synced tables (DESIGN §5). Allowlist enforced on both ends.
pub const SYNCED_TABLES: &[&str] = &[
    "collections",
    "items",
    "assets",
    "notes",
    "annotations",
    "extractions",
    "transcriptions",
    "layouts",
    "entities",
    "triples",
    "topics",
    "item_topics",
    "llm_results",
    "rag_conversations",
    "rag_messages",
];

/// The 15 synced tables ordered parents-before-children along the FK graph
/// (DESIGN §4.10), for use by the pull-apply path (upserts in this order,
/// deletes in reverse). Same set as [`SYNCED_TABLES`], different order.
/// Consumed by the apply slice (push/pull); only tests reference it in C1.
#[allow(dead_code)]
pub const SYNCED_TABLES_FK_ORDER: &[&str] = &[
    // Roots (no synced FK parent).
    "collections",
    "topics",
    "rag_conversations",
    // First generation.
    "items",
    // Children of items / conversations.
    "assets",
    "notes",
    "entities",
    "triples",
    "item_topics",
    "llm_results",
    "rag_messages",
    // Children of assets.
    "annotations",
    "extractions",
    "transcriptions",
    "layouts",
];

/// Returns true when `table` is in the synced allowlist (DESIGN §5). Used by
/// the apply/push slices to validate the wire envelope's `table` field.
#[allow(dead_code)]
pub fn is_synced_table(table: &str) -> bool {
    SYNCED_TABLES.contains(&table)
}

/// The three trigger names for a table: `(insert, update, delete)`.
fn trigger_names(table: &str) -> [String; 3] {
    [
        format!("trg_sync_{table}_i"),
        format!("trg_sync_{table}_u"),
        format!("trg_sync_{table}_d"),
    ]
}

/// Builds the `CREATE TRIGGER IF NOT EXISTS` DDL for one (table, op) per the
/// DESIGN §6.1 template. `op` is one of `'I'`/`'U'`/`'D'`; `event` is the
/// matching SQL event (`INSERT`/`UPDATE`/`DELETE`); `row_ref` is `NEW`/`OLD`.
fn create_trigger_sql(table: &str, op: char, event: &str, row_ref: &str) -> String {
    let suffix = op.to_ascii_lowercase();
    format!(
        "CREATE TRIGGER IF NOT EXISTS trg_sync_{table}_{suffix} AFTER {event} ON {table}\n\
         WHEN COALESCE((SELECT value FROM sync_meta WHERE key='applying'),'0') <> '1'\n\
         \x20AND COALESCE((SELECT value FROM sync_meta WHERE key='capture_enabled'),'0') = '1'\n\
         BEGIN\n\
         \x20\x20INSERT INTO sync_oplog(table_name, row_id, op, changed_at)\n\
         \x20\x20VALUES ('{table}', {row_ref}.id, '{op}', CAST(unixepoch('subsec')*1000 AS INTEGER));\n\
         END;"
    )
}

/// The three `CREATE TRIGGER IF NOT EXISTS` statements for one table.
fn create_triggers_sql(table: &str) -> [String; 3] {
    [
        create_trigger_sql(table, 'I', "INSERT", "NEW"),
        create_trigger_sql(table, 'U', "UPDATE", "NEW"),
        create_trigger_sql(table, 'D', "DELETE", "OLD"),
    ]
}

fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM sync_meta WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(format!("Failed to read sync_meta['{key}']: {other}")),
    })
}

fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .map(|_| ())
    .map_err(|e| format!("Failed to write sync_meta['{key}']: {e}"))
}

/// True when this table currently has all 3 capture triggers installed.
fn table_triggers_present(conn: &Connection, table: &str) -> Result<bool, String> {
    for name in trigger_names(table) {
        let present: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='trigger' AND name = ?1",
                [&name],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if !present {
            return Ok(false);
        }
    }
    Ok(true)
}

/// True when `table` exists as a real table (skip trigger creation for tables a
/// given DB hasn't migrated in yet — fresh installs apply JS migrations first).
fn base_table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?1",
        [table],
        |_| Ok(true),
    )
    .unwrap_or(false)
}

/// Drops all `trg_sync_*` triggers currently installed (DESIGN §6.1.2 upgrade).
fn drop_all_sync_triggers(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE 'trg_sync_%'")
        .map_err(|e| format!("Failed to list sync triggers: {e}"))?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("Failed to read sync trigger names: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect sync trigger names: {e}"))?;
    drop(stmt);
    for name in names {
        conn.execute_batch(&format!("DROP TRIGGER IF EXISTS \"{name}\";"))
            .map_err(|e| format!("Failed to drop trigger {name}: {e}"))?;
    }
    Ok(())
}

/// Re-seeds a table's oplog with `'U'` entries for every existing row that the
/// server hasn't already versioned (DESIGN §6.1.4). Runs only when a session is
/// active and the table's triggers had to be (re)created — covers the residual
/// window where writes landed while triggers were missing after a rebuild.
fn reseed_table_oplog(conn: &Connection, table: &str, now_ms: i64) -> Result<(), String> {
    let sql = format!(
        "INSERT INTO sync_oplog(table_name, row_id, op, changed_at)\n\
         SELECT '{table}', id, 'U', ?1 FROM \"{table}\"",
    );
    conn.execute(&sql, rusqlite::params![now_ms])
        .map(|_| ())
        .map_err(|e| format!("Failed to re-seed oplog for {table}: {e}"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Ensures the sync schema and all 45 capture triggers, with self-healing
/// semantics (DESIGN §6.1):
///
/// 1. ALWAYS run 45 `CREATE TRIGGER IF NOT EXISTS` (auto-cure after `DROP+RENAME`
///    rebuilds that destroy triggers, e.g. migrations 0010/0019).
/// 2. If `sync_meta['triggers_version']` differs from [`TRIGGERS_VERSION`]:
///    DROP every `trg_sync_*` then CREATE (template upgrade).
/// 3. If no session is configured (`device_id` absent): `DELETE FROM sync_oplog`
///    (always safe — the oplog carries no payload and is redundant with table state).
/// 4. If a table's triggers had to be recreated while a session exists:
///    re-seed that table's oplog with `'U'` entries.
pub fn ensure_capture(conn: &Connection) -> Result<(), String> {
    ensure_sync_schema(conn)?;

    let has_session = meta_get(conn, "device_id")?.is_some();
    let stored_version = meta_get(conn, "triggers_version")?;
    let version_mismatch = stored_version.as_deref() != Some(TRIGGERS_VERSION);

    // Step 2: template upgrade — drop all sync triggers so the IF NOT EXISTS
    // pass recreates them from the current template.
    if version_mismatch {
        drop_all_sync_triggers(conn)?;
    }

    let now = now_ms();

    // Steps 1 + 4: create missing triggers per table; if a table had missing
    // triggers AND a session is active, re-seed its oplog (self-heal).
    for table in SYNCED_TABLES {
        if !base_table_exists(conn, table) {
            // Table not migrated in yet (fresh install before JS migrations).
            continue;
        }

        // After a version-mismatch drop, every table's triggers are gone, so
        // they are recreated below and must be re-seeded.
        let needs_recreate = version_mismatch || !table_triggers_present(conn, table)?;

        if needs_recreate {
            for stmt in create_triggers_sql(table) {
                conn.execute_batch(&stmt)
                    .map_err(|e| format!("Failed to create triggers for {table}: {e}"))?;
            }
            if has_session {
                reseed_table_oplog(conn, table, now)?;
            }
        }
    }

    // Step 3: no session ⇒ the oplog is meaningless; truncate it.
    if !has_session {
        conn.execute_batch("DELETE FROM sync_oplog;")
            .map_err(|e| format!("Failed to clear sync_oplog: {e}"))?;
    }

    meta_set(conn, "triggers_version", TRIGGERS_VERSION)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_support::{
        new_synced_test_db, oplog_count, oplog_count_for, set_session, set_session_with_capture,
        trg_sync_count,
    };

    #[test]
    fn synced_table_sets_match() {
        assert_eq!(SYNCED_TABLES.len(), 15);
        assert_eq!(SYNCED_TABLES_FK_ORDER.len(), 15);
        let mut a: Vec<&str> = SYNCED_TABLES.to_vec();
        let mut b: Vec<&str> = SYNCED_TABLES_FK_ORDER.to_vec();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b, "FK-order list must be a permutation of the allowlist");
    }

    #[test]
    fn fk_order_places_parents_before_children() {
        let pos = |t: &str| SYNCED_TABLES_FK_ORDER.iter().position(|x| *x == t).unwrap();
        // Spot-check the DESIGN §4.10 ordering constraints.
        assert!(pos("collections") < pos("items"));
        assert!(pos("topics") < pos("item_topics"));
        assert!(pos("rag_conversations") < pos("rag_messages"));
        assert!(pos("items") < pos("assets"));
        assert!(pos("assets") < pos("extractions"));
        assert!(pos("assets") < pos("annotations"));
        assert!(pos("items") < pos("entities"));
    }

    #[test]
    fn create_trigger_sql_matches_design_template() {
        let sql = create_trigger_sql("items", 'U', "UPDATE", "NEW");
        assert!(sql.contains("CREATE TRIGGER IF NOT EXISTS trg_sync_items_u AFTER UPDATE ON items"));
        assert!(
            sql.contains("COALESCE((SELECT value FROM sync_meta WHERE key='applying'),'0') <> '1'")
        );
        assert!(sql.contains(
            "COALESCE((SELECT value FROM sync_meta WHERE key='capture_enabled'),'0') = '1'"
        ));
        assert!(sql.contains("CAST(unixepoch('subsec')*1000 AS INTEGER)"));
        assert!(sql.contains("VALUES ('items', NEW.id, 'U'"));
    }

    #[test]
    fn ensure_capture_installs_exactly_45_triggers() {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");
        assert_eq!(trg_sync_count(&conn), 45);
    }

    #[test]
    fn no_session_clears_oplog_and_blocks_capture() {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");
        // Manually stuff an oplog row to prove the no-session path truncates it.
        conn.execute(
            "INSERT INTO sync_oplog(table_name, row_id, op, changed_at) VALUES ('items','x','U',1)",
            [],
        )
        .expect("insert oplog");
        ensure_capture(&conn).expect("re-run ensure capture");
        assert_eq!(oplog_count(&conn), 0, "no-session ensure truncates oplog");

        // With capture disabled (no session), domain writes do not capture.
        conn.execute(
            "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
            [],
        )
        .expect("insert collection");
        assert_eq!(oplog_count(&conn), 0, "capture gated off without session");
    }

    #[test]
    fn capture_records_inserts_updates_and_deletes_with_session() {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");
        set_session_with_capture(&conn);

        conn.execute(
            "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
            [],
        )
        .expect("insert");
        conn.execute("UPDATE collections SET name='C2' WHERE id='c1'", [])
            .expect("update");
        conn.execute("DELETE FROM collections WHERE id='c1'", [])
            .expect("delete");

        assert_eq!(oplog_count_for(&conn, "collections"), 3);
    }

    #[test]
    fn rebuild_self_heal_recreates_triggers_and_reseeds() {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");
        set_session(&conn);

        // Seed a pre-existing row directly while capture is off (no triggers fire
        // because capture_enabled is unset), then turn capture on.
        conn.execute(
            "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
            [],
        )
        .expect("seed row");
        set_session_with_capture(&conn);

        // Simulate a DROP+RENAME rebuild destroying this table's triggers.
        conn.execute_batch(
            "DROP TRIGGER trg_sync_collections_i;
             DROP TRIGGER trg_sync_collections_u;
             DROP TRIGGER trg_sync_collections_d;",
        )
        .expect("drop triggers");
        assert_eq!(trg_sync_count(&conn), 42);

        // Re-run with UNCHANGED version: IF NOT EXISTS restores the 3 triggers
        // and re-seeds the table's oplog ('U' for each row).
        ensure_capture(&conn).expect("self-heal ensure");
        assert_eq!(trg_sync_count(&conn), 45, "triggers restored");
        assert_eq!(
            oplog_count_for(&conn, "collections"),
            1,
            "rebuilt table re-seeded with one 'U' entry"
        );
    }

    #[test]
    fn version_mismatch_drops_all_and_reseeds_when_session_active() {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");
        set_session(&conn);
        conn.execute(
            "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
            [],
        )
        .expect("seed row");
        conn.execute(
            "INSERT INTO topics(id,name,created_at) VALUES('t1','T',1)",
            [],
        )
        .expect("seed topic");
        set_session_with_capture(&conn);

        // Force a template-version downgrade so the next ensure performs the
        // DROP-all-then-create upgrade and re-seeds every populated table.
        meta_set(&conn, "triggers_version", "0").expect("set stale version");
        ensure_capture(&conn).expect("upgrade ensure");

        assert_eq!(trg_sync_count(&conn), 45);
        assert_eq!(oplog_count_for(&conn, "collections"), 1);
        assert_eq!(oplog_count_for(&conn, "topics"), 1);
        let version = meta_get(&conn, "triggers_version").unwrap();
        assert_eq!(version.as_deref(), Some(TRIGGERS_VERSION));
    }

    #[test]
    fn echo_suppression_flag_blocks_capture_during_apply() {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");
        set_session_with_capture(&conn);

        // Simulate the pull-apply path setting applying='1'.
        meta_set(&conn, "applying", "1").expect("set applying");
        conn.execute(
            "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
            [],
        )
        .expect("insert during apply");
        assert_eq!(oplog_count(&conn), 0, "applying='1' suppresses capture");

        meta_set(&conn, "applying", "0").expect("clear applying");
        conn.execute(
            "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c2','D',1,1)",
            [],
        )
        .expect("insert after apply");
        assert_eq!(
            oplog_count(&conn),
            1,
            "capture resumes once applying clears"
        );
    }

    #[test]
    fn cascade_delete_captures_child_deletes() {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");
        set_session_with_capture(&conn);

        // Build an item with two ON DELETE CASCADE children (entity + triple).
        // entities/triples cascade from items in the real schema; assets/notes
        // are RESTRICT, so a realistic item delete cascades exactly these.
        conn.execute_batch(
            "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1);
             INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','I','c1',1,1);
             INSERT INTO entities(id,item_id,entity_type,value,created_at) VALUES('e1','i1','person','x',1);
             INSERT INTO triples(id,item_id,subject,predicate,object,created_at) VALUES('tr1','i1','s','p','o',1);",
        )
        .expect("seed graph");

        // Clear the inserts so we measure only the cascade deletes.
        conn.execute_batch("DELETE FROM sync_oplog;")
            .expect("clear");

        // Deleting the item cascades to its entity and triple (foreign_keys=ON,
        // and the cascade DELETEs fire the children's 'D' triggers).
        conn.execute("DELETE FROM items WHERE id='i1'", [])
            .expect("delete item");

        // The item delete plus both child cascade deletes are captured.
        assert_eq!(oplog_count_for(&conn, "items"), 1);
        assert_eq!(
            oplog_count_for(&conn, "triples"),
            1,
            "cascade child captured"
        );
        assert_eq!(
            oplog_count_for(&conn, "entities"),
            1,
            "cascade child captured"
        );
    }

    #[test]
    fn asset_cascade_delete_captures_extraction_and_annotation() {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");
        set_session_with_capture(&conn);

        // extractions/transcriptions/layouts/annotations cascade from assets.
        conn.execute_batch(
            "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1);
             INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','I','c1',1,1);
             INSERT INTO assets(id,item_id,path,type,created_at) VALUES('a1','i1','/p','image',1);
             INSERT INTO extractions(id,asset_id,text_content,method,created_at) VALUES('ext-a1','a1','t','ocr',1);
             INSERT INTO annotations(id,asset_id,page,kind,color,x,y,width,height,created_at,updated_at)
               VALUES('an1','a1',1,'rectangle','#fff',0,0,1,1,1,1);",
        )
        .expect("seed asset graph");
        conn.execute_batch("DELETE FROM sync_oplog;")
            .expect("clear");

        conn.execute("DELETE FROM assets WHERE id='a1'", [])
            .expect("delete asset");

        assert_eq!(oplog_count_for(&conn, "assets"), 1);
        assert_eq!(oplog_count_for(&conn, "extractions"), 1, "cascade captured");
        assert_eq!(oplog_count_for(&conn, "annotations"), 1, "cascade captured");
    }

    #[test]
    fn oplog_row_lookup_uses_index() {
        let conn = new_synced_test_db();
        ensure_capture(&conn).expect("ensure capture");
        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN SELECT seq FROM sync_oplog WHERE table_name='items' AND row_id='r'",
                [],
                |row| row.get::<_, String>(3),
            )
            .expect("query plan");
        assert!(
            plan.contains("idx_sync_oplog_row"),
            "row lookup should use idx_sync_oplog_row, got: {plan}"
        );
    }
}
