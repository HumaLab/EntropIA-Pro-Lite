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
// Tombstone of a PDF container asset: its page assets hang off it through the
// `assets.parent_asset_id -> assets ON DELETE CASCADE` self-edge.
// --------------------------------------------------------------------------

/// Item + PDF container `pdf` + pages `p1`/`p2` (+ an extraction on `p1`), with
/// the oplog cleared so every row starts clean.
fn seed_pdf_container(conn: &Connection) {
    seed_collection(conn);
    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets(id,item_id,path,type,created_at) VALUES('pdf','i1','/pdf','pdf',1)",
        [],
    )
    .unwrap();
    for (id, page) in [("p1", 1), ("p2", 2)] {
        conn.execute(
            "INSERT INTO assets(id,item_id,path,type,parent_asset_id,page_number,created_at)
             VALUES(?1,'i1','/pg','image','pdf',?2,1)",
            rusqlite::params![id, page],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO extractions(id,asset_id,text_content,method,created_at) VALUES('ext-p1','p1','t','ocr',1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();
}

#[test]
fn container_tombstone_deletes_clean_pages_without_oplog_echo() {
    let conn = capturing_db();
    seed_pdf_container(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    let row = delete_row("assets", "pdf", 50);
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 50).expect("apply");
    assert_eq!(outcome.applied, 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM assets"), 0, "pages go");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM extractions"), 0);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM sync_oplog"),
        0,
        "a pulled delete never echoes back to the server"
    );
}

#[test]
fn container_tombstone_deferred_when_page_asset_is_dirty() {
    let conn = capturing_db();
    seed_pdf_container(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    conn.execute("UPDATE assets SET path='/pg2' WHERE id='p2'", [])
        .unwrap();
    assert!(row_has_pending_oplog(&conn, "assets", "p2").unwrap());
    assert!(!row_has_pending_oplog(&conn, "assets", "pdf").unwrap());

    let row = delete_row("assets", "pdf", 50);
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 50).expect("apply");
    assert_eq!(outcome.skipped, 1, "tombstone deferred");
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM assets"),
        3,
        "the dirty page edit is not destroyed by SQLite's own cascade"
    );
}

#[test]
fn container_tombstone_deferred_when_page_extraction_is_dirty() {
    let conn = capturing_db();
    seed_pdf_container(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    conn.execute(
        "UPDATE extractions SET text_content='edited' WHERE id='ext-p1'",
        [],
    )
    .unwrap();

    let row = delete_row("assets", "pdf", 50);
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 50).expect("apply");
    assert_eq!(outcome.skipped, 1, "tombstone deferred");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM assets"), 3);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM extractions"), 1);
}

#[test]
fn self_referencing_asset_does_not_loop_the_dirty_walk() {
    let conn = capturing_db();
    seed_collection(&conn);
    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO assets(id,item_id,path,type,parent_asset_id,created_at)
         VALUES('loop','i1','/p','image','loop',1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    assert!(!tombstone_has_dirty_cascade_child(&conn, "assets", "loop").unwrap());
}

// --------------------------------------------------------------------------
// A pulled asset delete also removes the local file (and its bookkeeping).
// --------------------------------------------------------------------------

fn put_file(dir: &std::path::Path, rel: &str) -> std::path::PathBuf {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"bytes").unwrap();
    path
}

fn seed_item(conn: &Connection) {
    seed_collection(conn);
    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
}

fn seed_asset(conn: &Connection, id: &str, path: &str, parent: Option<&str>) {
    conn.execute(
        "INSERT INTO assets(id,item_id,path,type,parent_asset_id,created_at)
         VALUES(?1,'i1',?2,'image',?3,1)",
        rusqlite::params![id, path, parent],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sync_pending_blobs(asset_id,sha256,rel_path,size) VALUES(?1,'h',?2,1)",
        rusqlite::params![id, path],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sync_blob_index(asset_id,sha256,size,file_mtime_ms,uploaded) VALUES(?1,'h',1,1,1)",
        [id],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();
}

fn apply_and_drain(conn: &Connection, dir: &std::path::Path, rows: &[PullRow]) -> usize {
    let mut ctx = ApplyContext::new(dir);
    apply_page(conn, &mut ctx, rows, 99).expect("apply");
    crate::sync::asset_files::drain_asset_file_removals(conn, dir).expect("drain")
}

#[test]
fn pulled_asset_delete_removes_its_file_and_bookkeeping() {
    let conn = capturing_db();
    seed_item(&conn);
    let dir = tmp_app_dir();
    let file = put_file(dir.path(), "assets/c1/i1/a.png");
    seed_asset(&conn, "a1", "assets/c1/i1/a.png", None);

    let removed = apply_and_drain(&conn, dir.path(), &[delete_row("assets", "a1", 50)]);

    assert_eq!(removed, 1);
    assert!(!file.exists(), "the orphan file is gone");
    assert!(
        !dir.path().join("assets/c1/i1").exists(),
        "its emptied folder goes too"
    );
    assert!(dir.path().join("assets").exists(), "the root stays");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_pending_blobs"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_blob_index"), 0);
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_meta WHERE key LIKE 'asset_file_remove:%'"
        ),
        0,
        "the queue is drained"
    );
}

#[test]
fn pulled_container_delete_removes_every_page_file() {
    let conn = capturing_db();
    seed_item(&conn);
    let dir = tmp_app_dir();
    let pdf = put_file(dir.path(), "assets/c1/i1/doc.pdf");
    let p1 = put_file(dir.path(), "assets/c1/i1/doc.pages/1.png");
    let p2 = put_file(dir.path(), "assets/c1/i1/doc.pages/2.png");
    seed_asset(&conn, "pdf", "assets/c1/i1/doc.pdf", None);
    seed_asset(&conn, "p1", "assets/c1/i1/doc.pages/1.png", Some("pdf"));
    seed_asset(&conn, "p2", "assets/c1/i1/doc.pages/2.png", Some("pdf"));

    let removed = apply_and_drain(&conn, dir.path(), &[delete_row("assets", "pdf", 50)]);

    assert_eq!(removed, 3);
    assert!(!pdf.exists() && !p1.exists() && !p2.exists());
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_pending_blobs"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_blob_index"), 0);
}

#[test]
fn pulled_item_delete_removes_the_files_of_its_assets() {
    let conn = capturing_db();
    seed_item(&conn);
    let dir = tmp_app_dir();
    let file = put_file(dir.path(), "assets/c1/i1/a.png");
    seed_asset(&conn, "a1", "assets/c1/i1/a.png", None);

    let removed = apply_and_drain(&conn, dir.path(), &[delete_row("items", "i1", 50)]);

    assert_eq!(count(&conn, "SELECT COUNT(*) FROM assets"), 0);
    assert_eq!(removed, 1);
    assert!(!file.exists());
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_pending_blobs"), 0);
}

#[test]
fn a_file_still_referenced_by_another_asset_is_kept() {
    let conn = capturing_db();
    seed_item(&conn);
    let dir = tmp_app_dir();
    let file = put_file(dir.path(), "assets/c1/i1/shared.png");
    seed_asset(&conn, "a1", "assets/c1/i1/shared.png", None);
    seed_asset(&conn, "a2", "assets/c1/i1/shared.png", None);

    let removed = apply_and_drain(&conn, dir.path(), &[delete_row("assets", "a1", 50)]);

    assert_eq!(removed, 0);
    assert!(file.exists(), "a2 still needs it");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_meta WHERE key LIKE 'asset_file_remove:%'"
        ),
        0,
        "the entry is dropped, not retried forever"
    );
}

#[test]
fn a_deferred_tombstone_keeps_the_file() {
    let conn = capturing_db();
    seed_item(&conn);
    let dir = tmp_app_dir();
    let file = put_file(dir.path(), "assets/c1/i1/a.png");
    seed_asset(&conn, "a1", "assets/c1/i1/a.png", None);
    conn.execute(
        "UPDATE assets SET path='assets/c1/i1/a.png' , size=2 WHERE id='a1'",
        [],
    )
    .unwrap();
    assert!(row_has_pending_oplog(&conn, "assets", "a1").unwrap());

    let removed = apply_and_drain(&conn, dir.path(), &[delete_row("assets", "a1", 50)]);

    assert_eq!(removed, 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM assets"), 1);
    assert!(file.exists(), "a skipped delete never touches the file");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_pending_blobs"), 1);
}

#[test]
fn a_path_outside_assets_is_never_deleted() {
    let conn = capturing_db();
    seed_item(&conn);
    let dir = tmp_app_dir();
    let outside = put_file(dir.path(), "elsewhere/secret.txt");
    let traversal = put_file(dir.path(), "keep.txt");
    let foreign = tempfile::tempdir().unwrap();
    let absolute = put_file(foreign.path(), "assets/c1/i1/x.png");
    seed_asset(&conn, "a1", "elsewhere/secret.txt", None);
    seed_asset(&conn, "a2", "assets/../keep.txt", None);
    seed_asset(&conn, "a3", &absolute.to_string_lossy(), None);

    let rows = [
        delete_row("assets", "a1", 50),
        delete_row("assets", "a2", 51),
        delete_row("assets", "a3", 52),
    ];
    let removed = apply_and_drain(&conn, dir.path(), &rows);

    assert_eq!(removed, 0);
    assert!(outside.exists() && traversal.exists() && absolute.exists());
}

#[test]
fn a_file_that_cannot_be_removed_never_fails_the_drain() {
    let conn = capturing_db();
    seed_item(&conn);
    let dir = tmp_app_dir();
    // The stored path names a folder: remove_file fails, the drain carries on.
    std::fs::create_dir_all(dir.path().join("assets/c1/i1/odd.png")).unwrap();
    seed_asset(&conn, "a1", "assets/c1/i1/odd.png", None);

    let removed = apply_and_drain(&conn, dir.path(), &[delete_row("assets", "a1", 50)]);

    assert_eq!(removed, 0);
    assert!(dir.path().join("assets/c1/i1/odd.png").is_dir());
}

fn apply_and_drain_with_cache(
    conn: &Connection,
    dir: &std::path::Path,
    cache: &std::path::Path,
    rows: &[PullRow],
) -> usize {
    let mut ctx = ApplyContext::new(dir);
    apply_page(conn, &mut ctx, rows, 99).expect("apply");
    crate::sync::asset_files::drain_with_cache(conn, dir, Some(cache)).expect("drain")
}

fn file_name_hash(asset_id: &str, path: &str) -> String {
    crate::ocr::commands::image_thumbnail_file_name(asset_id, path)
}

#[test]
fn pulled_asset_delete_removes_the_edit_versions_of_the_file() {
    let conn = capturing_db();
    seed_item(&conn);
    let dir = tmp_app_dir();
    let original = put_file(dir.path(), "assets/c1/i1/a.jpg");
    let v2 = put_file(dir.path(), "assets/c1/i1/a_v2.png");
    let v3 = put_file(dir.path(), "assets/c1/i1/a_v3.png");
    let near_miss = put_file(dir.path(), "assets/c1/i1/a_final.png");
    let other = put_file(dir.path(), "assets/c1/i1/b.png");
    let not_image = put_file(dir.path(), "assets/c1/i1/a_v4.txt");
    seed_asset(&conn, "a1", "assets/c1/i1/a_v3.png", None);
    seed_asset(&conn, "b1", "assets/c1/i1/b.png", None);

    let removed = apply_and_drain(&conn, dir.path(), &[delete_row("assets", "a1", 50)]);

    assert_eq!(removed, 3, "the current file and its two older versions");
    assert!(!original.exists() && !v2.exists() && !v3.exists());
    assert!(near_miss.exists(), "another family is never touched");
    assert!(other.exists(), "a live asset's file stays");
    assert!(not_image.exists(), "only image versions are swept");
}

#[test]
fn a_version_another_asset_still_uses_keeps_the_whole_family() {
    let conn = capturing_db();
    seed_item(&conn);
    let dir = tmp_app_dir();
    let original = put_file(dir.path(), "assets/c1/i1/a.png");
    let v2 = put_file(dir.path(), "assets/c1/i1/a_v2.png");
    let v3 = put_file(dir.path(), "assets/c1/i1/a_v3.png");
    seed_asset(&conn, "a1", "assets/c1/i1/a.png", None);
    seed_asset(&conn, "a2", "assets/c1/i1/a_v2.png", None);

    apply_and_drain(&conn, dir.path(), &[delete_row("assets", "a1", 50)]);

    assert!(!original.exists(), "the deleted asset's own file goes");
    assert!(v2.exists(), "a2 still points at it");
    assert!(v3.exists(), "its family is live, so nothing else is swept");
}

#[test]
fn pulled_asset_delete_removes_its_cached_thumbnails() {
    let conn = capturing_db();
    seed_item(&conn);
    let dir = tmp_app_dir();
    let cache = tmp_app_dir();
    put_file(dir.path(), "assets/c1/i1/a.png");
    seed_asset(&conn, "a1", "assets/c1/i1/a.png", None);
    seed_asset(&conn, "a2", "assets/c1/i1/b.png", None);
    let thumbs = cache.path().join("thumbnails");
    let pdf_thumb = put_file(cache.path(), "thumbnails/a1.png");
    let img_old = put_file(
        cache.path(),
        &format!("thumbnails/{}", file_name_hash("a1", "assets/c1/i1/a.png")),
    );
    let img_new = put_file(
        cache.path(),
        &format!(
            "thumbnails/{}",
            file_name_hash("a1", "assets/c1/i1/a_v2.png")
        ),
    );
    let other = put_file(
        cache.path(),
        &format!("thumbnails/{}", file_name_hash("a2", "assets/c1/i1/b.png")),
    );
    let lookalike = put_file(cache.path(), "thumbnails/image-a1-notahash.png");
    let sibling_id = put_file(cache.path(), "thumbnails/a10.png");

    apply_and_drain_with_cache(
        &conn,
        dir.path(),
        cache.path(),
        &[delete_row("assets", "a1", 50)],
    );

    assert!(!pdf_thumb.exists() && !img_old.exists() && !img_new.exists());
    assert!(other.exists(), "another asset's thumbnail stays");
    assert!(lookalike.exists(), "only exact thumbnail names match");
    assert!(
        sibling_id.exists(),
        "an id that merely starts the same stays"
    );
    assert!(thumbs.exists());
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_meta WHERE key LIKE 'asset_thumb_remove:%'"
        ),
        0,
        "the queue is drained"
    );
}

#[test]
fn a_thumbnail_queue_waits_while_the_cache_dir_is_unknown() {
    let conn = capturing_db();
    seed_item(&conn);
    let dir = tmp_app_dir();
    put_file(dir.path(), "assets/c1/i1/a.png");
    seed_asset(&conn, "a1", "assets/c1/i1/a.png", None);
    let mut ctx = ApplyContext::new(dir.path());
    apply_page(&conn, &mut ctx, &[delete_row("assets", "a1", 50)], 99).expect("apply");

    crate::sync::asset_files::drain_with_cache(&conn, dir.path(), None).expect("drain");

    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_meta WHERE key LIKE 'asset_thumb_remove:%'"
        ),
        1,
        "kept for a later cycle"
    );
}

// --------------------------------------------------------------------------
// Tombstone of a parent with a local RESTRICT (non-cascade) dependent.
// --------------------------------------------------------------------------

#[test]
fn item_tombstone_deletes_clean_restrict_asset_and_journals_parent_deleted() {
    let conn = capturing_db();
    seed_collection(&conn);
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    // Item + a clean asset (RESTRICT dependent) + the asset's own CASCADE child.
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

    // Remote tombstone for the item. Run through apply_page (not apply_row
    // directly) so the deferred-FK COMMIT is actually exercised.
    let row = delete_row("items", "i1", 50);
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 50).expect("apply page");
    assert_eq!(outcome.applied, 1, "item tombstone applies");

    assert_eq!(count(&conn, "SELECT COUNT(*) FROM items WHERE id='i1'"), 0);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM assets WHERE id='a1'"),
        0,
        "RESTRICT dependent deleted alongside the tombstoned parent"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM extractions WHERE id='ext-a1'"),
        0,
        "the dependent's own cascade child is gone too (SQLite ON DELETE CASCADE)"
    );

    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='parent_deleted' AND table_name='assets' AND row_id='a1'"
        ),
        1,
        "exactly one parent_deleted conflict for the asset"
    );
    // No conflict is journaled for the extraction — it is a cascade child, not a
    // RESTRICT dependent.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE table_name='extractions'"
        ),
        0
    );
    let loser: String = conn
        .query_row(
            "SELECT loser_payload FROM sync_conflicts WHERE reason='parent_deleted' AND row_id='a1'",
            [],
            |r| r.get(0),
        )
        .expect("loser payload");
    let payload: serde_json::Value = serde_json::from_str(&loser).expect("parse loser payload");
    assert_eq!(payload["id"], "a1");
    assert_eq!(payload["item_id"], "i1");
}

#[test]
fn item_tombstone_defers_when_restrict_asset_is_dirty() {
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
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();
    // Make the asset locally dirty (unpushed edit).
    conn.execute("UPDATE assets SET path='/p2' WHERE id='a1'", [])
        .unwrap();
    assert!(row_has_pending_oplog(&conn, "assets", "a1").unwrap());

    let row = delete_row("items", "i1", 50);
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 50).expect("apply page");
    assert_eq!(outcome.skipped, 1, "tombstone deferred");

    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM items WHERE id='i1'"),
        1,
        "item survives while its dirty dependent is unresolved"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM assets WHERE id='a1'"),
        1,
        "dirty dependent is not deleted"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='parent_deleted'"
        ),
        0,
        "nothing journaled — the tombstone did not run"
    );
}

#[test]
fn collection_tombstone_deletes_item_and_its_asset_journaling_both() {
    let conn = capturing_db();
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());

    conn.execute(
        "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
        [],
    )
    .unwrap();
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
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    let row = delete_row("collections", "c1", 50);
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 50).expect("apply page");
    assert_eq!(outcome.applied, 1);

    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM collections WHERE id='c1'"),
        0
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM items WHERE id='i1'"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM assets WHERE id='a1'"), 0);

    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='parent_deleted' AND table_name='items' AND row_id='i1'"
        ),
        1
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='parent_deleted' AND table_name='assets' AND row_id='a1'"
        ),
        1
    );
}

#[test]
fn item_tombstone_deletes_clean_note_and_journals_parent_deleted() {
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
        "INSERT INTO notes(id,item_id,content,created_at,updated_at) VALUES('n1','i1','hello',1,1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap();

    let row = delete_row("items", "i1", 50);
    let outcome = apply_page(&conn, &mut ctx, std::slice::from_ref(&row), 50).expect("apply page");
    assert_eq!(outcome.applied, 1);

    assert_eq!(count(&conn, "SELECT COUNT(*) FROM notes WHERE id='n1'"), 0);
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='parent_deleted' AND table_name='notes' AND row_id='n1'"
        ),
        1
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

// --------------------------------------------------------------------------
// vec_assets: non-`id` PK (asset_id) + embedding BLOB round-trip (DESIGN §5)
// --------------------------------------------------------------------------

/// The embedding BLOB and its compatibility metadata must survive push/apply
/// byte-for-byte so receivers can safely decide whether the vector is queryable.
#[test]
fn vec_assets_embedding_and_contract_metadata_round_trip() {
    use crate::sync::push::read_row_payload;
    use serde_json::Value;

    let src = new_synced_test_db();
    src.execute_batch(
        "CREATE TABLE IF NOT EXISTS vec_assets(\
         asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, embedding BLOB NOT NULL);",
    )
    .expect("create vec_assets (src)");
    let original: Vec<f32> = vec![0.10, -0.20, 0.30, 0.40, -0.50, 0.0, 1.0];
    let blob: Vec<u8> = original.iter().flat_map(|f| f.to_le_bytes()).collect();
    src.execute(
        "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions) VALUES('a1','i1',?1,?2,?3,?4)",
        rusqlite::params![
            blob,
            "baai/bge-m3",
            "bge-m3-6000-char-weighted-mean-l2-v1",
            1024_i64
        ],
    )
    .expect("insert embedding");

    // PUSH read: embedding leaves as a base64 string; a synthetic `id` mirrors
    // asset_id so the server's payload.id == row_id check passes.
    let payload = read_row_payload(&src, "vec_assets", "a1")
        .expect("read ok")
        .expect("row present");
    let obj = payload.as_object().expect("payload is an object");
    assert_eq!(
        obj.get("id"),
        Some(&json!("a1")),
        "synthetic id mirrors asset_id"
    );
    assert_eq!(obj.get("asset_id"), Some(&json!("a1")));
    assert_eq!(obj.get("embedding_model"), Some(&json!("baai/bge-m3")));
    assert_eq!(
        obj.get("embedding_contract"),
        Some(&json!("bge-m3-6000-char-weighted-mean-l2-v1"))
    );
    assert_eq!(obj.get("dimensions"), Some(&json!(1024)));
    assert!(
        matches!(obj.get("embedding"), Some(Value::String(_))),
        "embedding travels as a string"
    );

    // APPLY into a fresh DB: embedding must reconstruct a byte-identical BLOB.
    let dst = new_synced_test_db();
    dst.execute_batch(
        "CREATE TABLE IF NOT EXISTS vec_assets(\
         asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, embedding BLOB NOT NULL);",
    )
    .expect("create vec_assets (dst)");
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());
    apply_upsert(&dst, &mut ctx, "vec_assets", obj).expect("apply upsert");

    let got: (Vec<u8>, String, String, i64) = dst
        .query_row(
            "SELECT embedding, embedding_model, embedding_contract, dimensions FROM vec_assets WHERE asset_id='a1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("read back embedding");
    assert_eq!(got.0, blob, "embedding BLOB round-trips byte-identical");
    assert_eq!(got.1, "baai/bge-m3");
    assert_eq!(got.2, "bge-m3-6000-char-weighted-mean-l2-v1");
    assert_eq!(got.3, 1024);

    let decoded: Vec<f32> = got
        .0
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(decoded, original, "f32 vector decodes identically");
}

/// A vec_assets tombstone must delete by `asset_id`, not `id` (no such column).
#[test]
fn vec_assets_delete_keys_on_asset_id_not_id() {
    let dst = new_synced_test_db();
    dst.execute_batch(
        "CREATE TABLE IF NOT EXISTS vec_assets(\
         asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, embedding BLOB NOT NULL);\
         INSERT INTO vec_assets(asset_id,item_id,embedding) VALUES('a1','i1',x'00010203');",
    )
    .expect("seed vec_assets");
    apply_delete(&dst, "vec_assets", "a1").expect("delete by asset_id");
    assert_eq!(
        count(&dst, "SELECT COUNT(*) FROM vec_assets"),
        0,
        "delete keyed on asset_id removed the row"
    );
}

/// A real inbound vec_assets row (through apply_row) carries the synthetic
/// `id` = asset_id for the envelope/server checks; apply must drop it BEFORE the
/// column intersection so it does NOT journal a spurious schema_drift, while the
/// embedding (base64 → BLOB) still lands.
#[test]
fn vec_assets_synthetic_id_dropped_without_drift_through_apply_row() {
    let conn = capturing_db();
    let dir = tmp_app_dir();
    let mut ctx = ApplyContext::new(dir.path());
    // `embedding` is the base64 of the 4 bytes 0x00 0x01 0x02 0x03.
    let row = upsert_row(
        "vec_assets",
        "a1",
        10,
        json!({"id":"a1","asset_id":"a1","item_id":"i1","embedding":"AAECAw=="}),
    );
    conn.execute_batch("BEGIN;").unwrap();
    let outcome = apply_row(&conn, &mut ctx, &row).expect("apply");
    conn.execute_batch("COMMIT;").unwrap();

    assert_eq!(outcome, RowOutcome::Applied);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM vec_assets WHERE asset_id='a1'"),
        1,
        "embedding row applied"
    );
    // The synthetic `id` must NOT have produced a schema_drift conflict.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE table_name='vec_assets' AND reason='schema_drift'"
        ),
        0,
        "synthetic id is dropped before intersect — no drift"
    );
    // And the embedding decoded back to the exact bytes.
    let blob: Vec<u8> = conn
        .query_row(
            "SELECT embedding FROM vec_assets WHERE asset_id='a1'",
            [],
            |r| r.get(0),
        )
        .expect("read embedding");
    assert_eq!(blob, vec![0u8, 1, 2, 3], "embedding decodes byte-identical");
}
