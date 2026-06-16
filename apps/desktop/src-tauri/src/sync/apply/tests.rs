//! Apply-semantics tests (DESIGN §13.3 apply matrix, PROTOCOL "Semántica de
//! apply"). Exercises the full reconciliation against the real schema fixture.

use super::*;
use crate::sync::capture::ensure_capture;
use crate::sync::session::meta_get_i64;
use crate::sync::test_support::{new_synced_test_db, set_session_with_capture};

use rusqlite::Connection;
use serde_json::json;

// --------------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------------

/// A capturing DB: full schema + sync schema + triggers + active session.
fn capturing_db() -> Connection {
    let conn = new_synced_test_db();
    ensure_capture(&conn).expect("ensure capture");
    set_session_with_capture(&conn);
    conn
}

fn seed_collection(conn: &Connection) {
    conn.execute(
        "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
        [],
    )
    .expect("seed collection");
    conn.execute_batch("DELETE FROM sync_oplog;")
        .expect("clear oplog");
}

fn upsert_row(table: &str, row_id: &str, server_seq: i64, payload: serde_json::Value) -> PullRow {
    PullRow {
        table: table.to_string(),
        row_id: row_id.to_string(),
        server_seq,
        deleted: false,
        changed_at: 1,
        device_id: "remote".to_string(),
        payload: Some(payload),
    }
}

fn delete_row(table: &str, row_id: &str, server_seq: i64) -> PullRow {
    PullRow {
        table: table.to_string(),
        row_id: row_id.to_string(),
        server_seq,
        deleted: true,
        changed_at: 1,
        device_id: "remote".to_string(),
        payload: None,
    }
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count")
}

fn tmp_app_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("assets")).expect("assets dir");
    dir
}

// --------------------------------------------------------------------------
// rel_path attack vectors (DESIGN §7, PROTOCOL "Transformación de assets")
// --------------------------------------------------------------------------

#[test]
fn validate_inbound_rel_path_accepts_clean_paths() {
    let dir = tmp_app_dir();
    let resolved =
        validate_inbound_rel_path("assets/col-1/item-1/uuid_foto.png", dir.path()).expect("ok");
    assert!(resolved.starts_with(dir.path()));
    assert!(resolved.ends_with("uuid_foto.png"));
}

#[test]
fn validate_inbound_rel_path_rejects_attack_vectors() {
    let dir = tmp_app_dir();
    let app = dir.path();
    // Traversal.
    assert_eq!(
        validate_inbound_rel_path("assets/../secret.txt", app),
        Err(InboundRelPathError::Traversal)
    );
    assert_eq!(
        validate_inbound_rel_path("assets/a/../../x", app),
        Err(InboundRelPathError::Traversal)
    );
    // Absolute (unix).
    assert_eq!(
        validate_inbound_rel_path("/etc/passwd", app),
        Err(InboundRelPathError::Absolute)
    );
    // Drive letter (Windows).
    assert_eq!(
        validate_inbound_rel_path("C:\\Windows\\system32", app),
        Err(InboundRelPathError::DriveOrUnc)
    );
    // UNC.
    assert_eq!(
        validate_inbound_rel_path("\\\\server\\share\\x", app),
        Err(InboundRelPathError::DriveOrUnc)
    );
    assert_eq!(
        validate_inbound_rel_path("//server/share/x", app),
        Err(InboundRelPathError::DriveOrUnc)
    );
    // Not under assets/.
    assert_eq!(
        validate_inbound_rel_path("logs/app.log", app),
        Err(InboundRelPathError::NotUnderAssets)
    );
    // Empty.
    assert_eq!(
        validate_inbound_rel_path("   ", app),
        Err(InboundRelPathError::Empty)
    );
    // A single-dot component.
    assert_eq!(
        validate_inbound_rel_path("assets/./x.png", app),
        Err(InboundRelPathError::Traversal)
    );
}

// --------------------------------------------------------------------------
// Envelope validation
// --------------------------------------------------------------------------

#[test]
fn envelope_rejects_id_mismatch_and_bad_table() {
    let payload = json!({"id": "WRONG", "title": "x"});
    assert_eq!(
        validate_upsert_envelope("items", "i1", &payload),
        Err(EnvelopeError::IdMismatch)
    );
    let good = json!({"id": "i1", "title": "x"});
    assert!(validate_upsert_envelope("items", "i1", &good).is_ok());
    assert_eq!(
        validate_upsert_envelope("app_settings", "i1", &good),
        Err(EnvelopeError::BadTable)
    );
}

#[test]
fn apply_row_journals_envelope_mismatch() {
    let conn = capturing_db();
    let mut ctx = ApplyContext::new(Path::new("."));
    conn.execute_batch("BEGIN;").unwrap();
    let row = upsert_row("items", "i1", 5, json!({"id": "OTHER", "title": "x"}));
    let outcome = apply_row(&conn, &mut ctx, &row).expect("apply");
    assert_eq!(outcome, RowOutcome::Journaled);
    conn.execute_batch("COMMIT;").unwrap();
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='apply_error'"
        ),
        1
    );
}

// --------------------------------------------------------------------------
// Schema drift: unknown column dropped + journaled, cursor advances
// --------------------------------------------------------------------------

#[test]
fn unknown_column_is_dropped_and_drift_journaled_once() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    let rows = vec![
        upsert_row(
            "items",
            "i1",
            10,
            json!({"id":"i1","title":"A","collection_id":"c1","created_at":1,"updated_at":1,"ghost_col":"x"}),
        ),
        upsert_row(
            "items",
            "i2",
            11,
            json!({"id":"i2","title":"B","collection_id":"c1","created_at":1,"updated_at":1,"ghost_col":"y"}),
        ),
    ];
    let outcome = apply_page(&conn, &mut ctx, &rows, 11).expect("apply page");
    assert_eq!(outcome.applied, 2);

    // Both rows applied (the unknown column was dropped, not fatal).
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM items WHERE id IN('i1','i2')"),
        2
    );
    // Drift journaled ONCE per (table, column), not per row.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='schema_drift' AND row_id='ghost_col'"
        ),
        1
    );
    // Cursor advanced.
    assert_eq!(meta_get_i64(&conn, "last_pull_seq").unwrap(), 11);
}

// --------------------------------------------------------------------------
// Missing column preserved on UPDATE
// --------------------------------------------------------------------------

#[test]
fn missing_payload_column_preserved_on_update() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    // Local row with metadata set.
    conn.execute(
        "INSERT INTO items(id,title,collection_id,metadata,created_at,updated_at)
         VALUES('i1','Local','c1','{\"k\":1}',1,1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    // Remote upsert OMITS metadata.
    let row = upsert_row(
        "items",
        "i1",
        20,
        json!({"id":"i1","title":"Remote","collection_id":"c1","created_at":1,"updated_at":2}),
    );
    apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 20).expect("apply");

    let (title, metadata): (String, Option<String>) = conn
        .query_row("SELECT title, metadata FROM items WHERE id='i1'", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(title, "Remote", "provided column updated");
    assert_eq!(
        metadata.as_deref(),
        Some("{\"k\":1}"),
        "omitted column preserved"
    );
}

// --------------------------------------------------------------------------
// items.rowid unchanged after pulled update + no ghost FTS + cascade survive
// --------------------------------------------------------------------------

#[test]
fn pulled_update_preserves_items_rowid_and_cascade_children() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entities(id,item_id,entity_type,value,created_at) VALUES('e1','i1','person','x',1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    let rowid_before: i64 = conn
        .query_row("SELECT rowid FROM items WHERE id='i1'", [], |r| r.get(0))
        .unwrap();

    let row = upsert_row(
        "items",
        "i1",
        30,
        json!({"id":"i1","title":"Updated","collection_id":"c1","created_at":1,"updated_at":2}),
    );
    apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 30).expect("apply");

    let rowid_after: i64 = conn
        .query_row("SELECT rowid FROM items WHERE id='i1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        rowid_before, rowid_after,
        "ON CONFLICT DO UPDATE keeps rowid"
    );
    // Cascade child survives (no INSERT OR REPLACE → no cascade delete).
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM entities WHERE id='e1'"),
        1
    );
    // An FTS reindex was queued for the item (not executed here).
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_pending_fts WHERE item_id='i1'"
        ),
        1
    );
}

// --------------------------------------------------------------------------
// Skip-if-dirty (pull case)
// --------------------------------------------------------------------------

#[test]
fn skip_if_dirty_pull_does_not_overwrite_local_edit() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','Local','c1',1,1)",
        [],
    )
    .unwrap();
    // A pending local edit is in the oplog (capture is on).
    assert!(count(&conn, "SELECT COUNT(*) FROM sync_oplog WHERE row_id='i1'") >= 1);

    let row = upsert_row(
        "items",
        "i1",
        40,
        json!({"id":"i1","title":"Remote","collection_id":"c1","created_at":1,"updated_at":2}),
    );
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 40).expect("apply");
    assert_eq!(outcome.skipped, 1);

    let title: String = conn
        .query_row("SELECT title FROM items WHERE id='i1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(title, "Local", "dirty row not overwritten");
    // Row version NOT advanced (forces server LWW next push).
    assert_eq!(known_version(&conn, "items", "i1").unwrap(), 0);
}

// --------------------------------------------------------------------------
// Tombstone deferred on dirty cascade child
// --------------------------------------------------------------------------

#[test]
fn tombstone_deferred_when_cascade_child_dirty() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    // Item + asset + extraction. The asset has a CASCADE child (extraction).
    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets(id,item_id,path,type,created_at) VALUES('a1','i1','/p','image',1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO extractions(id,asset_id,text_content,method,created_at) VALUES('ext-a1','a1','t','ocr',1)",
        [],
    )
    .unwrap();
    // Make ONLY the extraction dirty, then clear the asset's own oplog so the
    // skip is attributable to the cascade child (not the asset row itself).
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();
    conn.execute(
        "UPDATE extractions SET text_content='edited' WHERE id='ext-a1'",
        [],
    )
    .unwrap();
    assert!(row_has_pending_oplog(&conn, "extractions", "ext-a1").unwrap());
    assert!(!row_has_pending_oplog(&conn, "assets", "a1").unwrap());

    // Remote tombstone for the asset.
    let row = delete_row("assets", "a1", 50);
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 50).expect("apply");
    assert_eq!(outcome.skipped, 1, "tombstone deferred");
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM assets WHERE id='a1'"),
        1,
        "asset survives"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM extractions WHERE id='ext-a1'"),
        1
    );
}

#[test]
fn tombstone_applied_when_no_dirty_child() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets(id,item_id,path,type,created_at) VALUES('a1','i1','/p','image',1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO extractions(id,asset_id,text_content,method,created_at) VALUES('ext-a1','a1','t','ocr',1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    let row = delete_row("assets", "a1", 50);
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 50).expect("apply");
    assert_eq!(outcome.applied, 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM assets WHERE id='a1'"), 0);
    // Cascade fired on the clean delete (extraction gone).
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM extractions WHERE id='ext-a1'"),
        0
    );
}

// --------------------------------------------------------------------------
// Parking: child page applied before parent → parked → drained when parent lands
// --------------------------------------------------------------------------

#[test]
fn child_parked_until_parent_arrives_then_drained() {
    let conn = capturing_db();
    // Note: NO collection seeded — the item's parent collection arrives later.
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    // Page 1: the item references collection c1 which does not exist yet → FK
    // violation on commit → parked.
    let page1 = vec![upsert_row(
        "items",
        "i1",
        60,
        json!({"id":"i1","title":"Orphan","collection_id":"c1","created_at":1,"updated_at":1}),
    )];
    let out1 = apply_page(&conn, &mut ctx, &page1, 60).expect("apply page1");
    assert_eq!(out1.parked, 1, "item parked pending its collection");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM items WHERE id='i1'"), 0);
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_pending_rows WHERE row_id='i1'"
        ),
        1
    );

    // Page 2: the parent collection arrives. The page apply persists it, then the
    // post-page retry drains the parked item.
    let page2 = vec![upsert_row(
        "collections",
        "c1",
        61,
        json!({"id":"c1","name":"C","created_at":1,"updated_at":1}),
    )];
    apply_page(&conn, &mut ctx, &page2, 61).expect("apply page2");
    let drained = retry_pending_rows(&conn, &mut ctx, false).expect("retry");
    assert_eq!(drained, 1, "parked item drains once parent exists");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM items WHERE id='i1'"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_pending_rows"), 0);
}

// Regression (Pro first-pull, real-data e2e): a single page can carry a
// multi-LEVEL FK chain — item -> collection AND entity -> item — whose common
// ancestor (collection c1) is absent. Parking the item surfaces the entity as a
// FRESH violator on re-apply, so a single park round errored with
// "[sync] page still violates FK after parking violators". The iterative park
// must converge: park the item, then the entity, committing the page with the
// whole subtree parked and NO error. This never fired in Lite (always the
// pushing peer) — it only shows up on Pro's first bulk pull of the full graph.
#[test]
fn multi_level_fk_chain_parks_whole_subtree_in_one_page() {
    let conn = capturing_db();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    let page = vec![
        upsert_row(
            "items",
            "i1",
            60,
            json!({"id":"i1","title":"Orphan","collection_id":"c1","created_at":1,"updated_at":1}),
        ),
        upsert_row(
            "entities",
            "en1",
            61,
            json!({"id":"en1","item_id":"i1","entity_type":"person","value":"Belgrano","created_at":1}),
        ),
    ];
    let out =
        apply_page(&conn, &mut ctx, &page, 61).expect("multi-level FK chain must park, not error");
    assert_eq!(
        out.applied, 0,
        "nothing applies while the ancestor is missing"
    );
    assert_eq!(
        out.parked, 2,
        "the whole subtree (item + entity) parks in one page"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_pending_rows"), 2);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM items"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM entities"), 0);

    // The ancestor lands; the parked subtree drains over successive passes.
    let page2 = vec![upsert_row(
        "collections",
        "c1",
        62,
        json!({"id":"c1","name":"C","created_at":1,"updated_at":1}),
    )];
    apply_page(&conn, &mut ctx, &page2, 62).expect("apply ancestor page");
    for _ in 0..5 {
        if retry_pending_rows(&conn, &mut ctx, false).expect("retry") == 0 {
            break;
        }
    }
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM sync_pending_rows"),
        0,
        "subtree fully drains once the ancestor exists"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM items WHERE id='i1'"), 1);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM entities WHERE id='en1'"),
        1
    );
}

#[test]
fn parked_child_journals_parent_deleted_only_when_parent_confirmed_tombstoned() {
    let conn = capturing_db();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    // Park an orphan item (collection never arrives).
    let page = vec![upsert_row(
        "items",
        "i1",
        70,
        json!({"id":"i1","title":"Orphan","collection_id":"missing","created_at":1,"updated_at":1}),
    )];
    let out = apply_page(&conn, &mut ctx, &page, 70).expect("apply");
    assert_eq!(out.parked, 1);

    // Final-pass retry: the parent collection is absent locally AND not parked →
    // confirmed tombstoned → parent_deleted journaled + row removed.
    retry_pending_rows(&conn, &mut ctx, true).expect("final retry");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='parent_deleted' AND row_id='i1'"
        ),
        1
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_pending_rows"), 0);
}

// --------------------------------------------------------------------------
// Topic alias rewrite — both directions (DESIGN §4.7)
// --------------------------------------------------------------------------

#[test]
fn topic_name_collision_aliases_and_rewrites_item_topics() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    // Local topic 'History' with id local-t.
    conn.execute(
        "INSERT INTO topics(id,name,created_at) VALUES('local-t','History',1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    // Remote pushes a topic with the SAME name but a different id, then an
    // item_topics row referencing the remote topic id.
    let rows = vec![
        upsert_row(
            "topics",
            "remote-t",
            80,
            json!({"id":"remote-t","name":"History","created_at":1}),
        ),
        upsert_row(
            "item_topics",
            "it1",
            81,
            json!({"id":"it1","item_id":"i1","topic_id":"remote-t","created_at":1}),
        ),
    ];
    apply_page(&conn, &mut ctx, &rows, 81).expect("apply");

    // No duplicate topic inserted.
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM topics WHERE name='History'"),
        1
    );
    // Alias recorded.
    assert_eq!(
        topic_alias(&conn, "remote-t").unwrap().as_deref(),
        Some("local-t")
    );
    // item_topics row rewritten to the LOCAL topic id.
    let topic_id: String = conn
        .query_row("SELECT topic_id FROM item_topics WHERE id='it1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(topic_id, "local-t", "apply rewrites topic_id via alias");
    // unique_collision journaled for observability.
    assert!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='unique_collision'"
        ) >= 1
    );
}

#[test]
fn push_rewrites_item_topics_topic_id_via_reverse_alias() {
    let conn = capturing_db();
    seed_collection(&conn);

    // Seed an alias remote-t → local-t and a local item_topics using local-t.
    conn.execute(
        "INSERT INTO topics(id,name,created_at) VALUES('local-t','History',1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO item_topics(id,item_id,topic_id,created_at) VALUES('it1','i1','local-t',1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sync_topic_aliases(remote_id,local_id) VALUES('remote-t','local-t')",
        [],
    )
    .unwrap();

    let mut payload = json!({"id":"it1","item_id":"i1","topic_id":"local-t","created_at":1});
    crate::sync::push::rewrite_item_topics_topic_id_for_push(&conn, &mut payload).unwrap();
    assert_eq!(
        payload["topic_id"], "remote-t",
        "push rewrites local topic_id back to the canonical server id"
    );
}

// --------------------------------------------------------------------------
// Asset apply: rel_path rewritten to local path + blob/fts enqueued
// --------------------------------------------------------------------------

#[test]
fn asset_apply_rewrites_path_and_enqueues_blob_and_fts() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    let row = upsert_row(
        "assets",
        "a1",
        90,
        json!({
            "id":"a1","item_id":"i1","type":"image","sort_index":0,"created_at":1,
            "rel_path":"assets/c1/i1/uuid_foto.png","sha256":"deadbeef","size":1234
        }),
    );
    apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 90).expect("apply");

    let path: String = conn
        .query_row("SELECT path FROM assets WHERE id='a1'", [], |r| r.get(0))
        .unwrap();
    assert!(
        path.contains("assets"),
        "path rewritten to a local absolute path"
    );
    assert!(!path.contains("rel_path"));
    // Blob enqueued (file missing locally).
    let (sha, size): (String, i64) = conn
        .query_row(
            "SELECT sha256, size FROM sync_pending_blobs WHERE asset_id='a1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(sha, "deadbeef");
    assert_eq!(size, 1234);
    // FTS enqueued via asset → item.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_pending_fts WHERE item_id='i1'"
        ),
        1
    );
}

#[test]
fn asset_apply_with_bad_rel_path_journals_and_skips() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    let row = upsert_row(
        "assets",
        "a1",
        91,
        json!({
            "id":"a1","item_id":"i1","type":"image","sort_index":0,"created_at":1,
            "rel_path":"assets/../../etc/passwd","sha256":"x","size":1
        }),
    );
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 91).expect("apply");
    assert_eq!(outcome.journaled, 1);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM assets WHERE id='a1'"),
        0,
        "row not written"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='apply_error' AND row_id='a1'"
        ),
        1
    );
    // Cursor still advances (PROTOCOL step 5).
    assert_eq!(meta_get_i64(&conn, "last_pull_seq").unwrap(), 91);
}

// --------------------------------------------------------------------------
// Deterministic id convergence for asset-keyed tables (DESIGN §4.6)
// --------------------------------------------------------------------------

#[test]
fn extraction_upsert_converges_stray_id_via_asset_id_conflict() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets(id,item_id,path,type,created_at) VALUES('a1','i1','/p','image',1)",
        [],
    )
    .unwrap();
    // A local stray extraction with a non-deterministic id.
    conn.execute(
        "INSERT INTO extractions(id,asset_id,text_content,method,created_at) VALUES('stray-uuid','a1','old','ocr',1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    // Remote extraction with the deterministic id ext-a1, same asset_id.
    let row = upsert_row(
        "extractions",
        "ext-a1",
        100,
        json!({"id":"ext-a1","asset_id":"a1","text_content":"new","method":"ocr","created_at":2}),
    );
    apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 100).expect("apply");

    // Exactly one extraction for the asset, with the converged deterministic id.
    let (id, text): (String, String) = conn
        .query_row(
            "SELECT id, text_content FROM extractions WHERE asset_id='a1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        id, "ext-a1",
        "id converged via ON CONFLICT(asset_id) ... id=excluded.id"
    );
    assert_eq!(text, "new");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM extractions WHERE asset_id='a1'"
        ),
        1
    );
}

// --------------------------------------------------------------------------
// Version cursor recording
// --------------------------------------------------------------------------

#[test]
fn applied_row_records_version_and_advances_cursor() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    let row = upsert_row(
        "items",
        "i1",
        42,
        json!({"id":"i1","title":"A","collection_id":"c1","created_at":1,"updated_at":1}),
    );
    apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 42).expect("apply");
    assert_eq!(known_version(&conn, "items", "i1").unwrap(), 42);
    assert_eq!(meta_get_i64(&conn, "last_pull_seq").unwrap(), 42);
}

#[test]
fn already_seen_version_is_skipped() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    conn.execute(
        "INSERT INTO sync_row_versions(table_name,row_id,server_seq) VALUES('items','i1',50)",
        [],
    )
    .unwrap();
    let row = upsert_row(
        "items",
        "i1",
        50,
        json!({"id":"i1","title":"A","collection_id":"c1","created_at":1,"updated_at":1}),
    );
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 50).expect("apply");
    assert_eq!(outcome.skipped, 1, "row already at this version is skipped");
}

// --------------------------------------------------------------------------
// FTS drain (PROTOCOL flow step 8)
// --------------------------------------------------------------------------

#[test]
fn drain_pending_fts_reindexes_queued_items_and_clears_queue() {
    let conn = capturing_db();
    seed_collection(&conn);

    // An item with extraction text the FTS index should pick up.
    conn.execute(
        "INSERT INTO items(id,title,collection_id,metadata,created_at,updated_at)
         VALUES('i1','Acta Colonial','c1','{}',1,1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets(id,item_id,path,type,created_at) VALUES('a1','i1','/p','image',1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO extractions(id,asset_id,text_content,method,created_at)
         VALUES('ext-a1','a1','Buenos Aires Belgrano','ocr',1)",
        [],
    )
    .unwrap();
    // Clear any FTS the fixture seeded for i1, then queue a reindex.
    conn.execute_batch("DELETE FROM fts_items;").unwrap();
    conn.execute("INSERT INTO sync_pending_fts(item_id) VALUES('i1')", [])
        .unwrap();

    let reindexed = drain_pending_fts(&conn).expect("drain fts");
    assert_eq!(reindexed, 1);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM sync_pending_fts"),
        0,
        "queue drained"
    );

    // The reindexed item is searchable, and its FTS rowid matches items.rowid (no
    // ghost rows — the contentless FTS5 contract is preserved).
    let item_rowid: i64 = conn
        .query_row("SELECT rowid FROM items WHERE id='i1'", [], |r| r.get(0))
        .unwrap();
    let fts_rowid: i64 = conn
        .query_row(
            "SELECT rowid FROM fts_items WHERE fts_items MATCH 'Belgrano'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fts_rowid, item_rowid, "fts_items.rowid == items.rowid");
}

#[test]
fn drain_pending_fts_handles_missing_item_as_noop() {
    let conn = capturing_db();
    conn.execute("INSERT INTO sync_pending_fts(item_id) VALUES('ghost')", [])
        .unwrap();
    let reindexed = drain_pending_fts(&conn).expect("drain");
    assert_eq!(reindexed, 1, "missing item still drained (no-op reindex)");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_pending_fts"), 0);
}
