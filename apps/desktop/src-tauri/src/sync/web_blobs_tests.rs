//! Tests for the files of saved web captures on the sync wire: key validation,
//! upload-before-row, verified download, queue behaviour and remote deletes.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::apply::{apply_page, ApplyContext};
use super::engine::{build_status, run_cycle, SyncState};
use super::http::{PullResponse, PullRow, PushChange};
use super::session::meta_set;
use super::test_support::{
    new_synced_test_db, set_session_with_capture, MockBlobFailure, MockSyncApi,
};
use super::web_blobs::{
    drain_pending_web_blobs, enqueue_downloads, has_pending_download, prepare_web_capture_push,
    rewrite_inbound, validate_rel_key, Role, WebPushOutcome, MAX_DOWNLOAD_RETRIES,
};
use super::web_capture::{drain_folder_removals, queue_folder_removal, record_capability};
use rusqlite::Connection;

const SOURCE: &str = "11111111-1111-4111-8111-111111111111";
const CAPTURE: &str = "22222222-2222-4222-8222-222222222222";
const EPOCH: &str = "mock-epoch";

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn key(ext: &str) -> String {
    format!("web-captures/{SOURCE}/{CAPTURE}.{ext}")
}

fn write_file(dir: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
    let path = rel
        .split('/')
        .fold(dir.to_path_buf(), |acc, part| acc.join(part));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    path
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// A page capture payload as the push reads it from the table.
fn page_payload(html: &[u8], text_rel: Option<&str>) -> Value {
    json!({
        "id": CAPTURE, "web_source_id": SOURCE, "accessed_at": "2026-10-02T00:00:00Z",
        "final_url": "https://a.test/", "kind": "page", "mime_type": "text/html",
        "text": null, "text_rel_path": text_rel, "quote_prefix": null, "quote_suffix": null,
        "rel_path": key("html"), "sha256": sha(html), "hash_of": "html",
        "size_bytes": html.len(), "extractor_version": "x", "title": null, "created_at": 1
    })
}

fn change(payload: Value) -> PushChange {
    PushChange {
        table: "web_captures".to_string(),
        row_id: CAPTURE.to_string(),
        op: "upsert".to_string(),
        changed_at: 1,
        base_seq: 0,
        payload: Some(payload),
    }
}

fn insert_source(conn: &Connection) {
    conn.execute(
        "INSERT INTO web_sources(id, original_url, final_url, first_accessed_at, created_at, updated_at)
         VALUES (?1, 'https://a.test/', 'https://a.test/', '2026-10-02T00:00:00Z', 1, 1)",
        [SOURCE],
    )
    .unwrap();
}

fn insert_capture_row(conn: &Connection, html: &[u8], text_rel: Option<&str>) {
    conn.execute(
        "INSERT INTO web_captures(id, web_source_id, accessed_at, final_url, kind, mime_type,
                                  text_rel_path, rel_path, sha256, hash_of, size_bytes, created_at)
         VALUES (?1, ?2, '2026-10-02T00:00:00Z', 'https://a.test/', 'page', 'text/html',
                 ?3, ?4, ?5, 'html', ?6, 1)",
        rusqlite::params![
            CAPTURE,
            SOURCE,
            text_rel,
            key("html"),
            sha(html),
            html.len() as i64
        ],
    )
    .unwrap();
}

fn session_db() -> Connection {
    let conn = new_synced_test_db();
    super::capture::ensure_capture(&conn).unwrap();
    set_session_with_capture(&conn);
    meta_set(&conn, "account_id", "acc").unwrap();
    meta_set(&conn, "server_url", "https://sync.test").unwrap();
    meta_set(&conn, "seeded_account", "acc").unwrap();
    meta_set(&conn, "server_epoch", EPOCH).unwrap();
    record_capability(&conn, EPOCH, true).unwrap();
    conn
}

// --------------------------------------------------------------------------
// Key validation
// --------------------------------------------------------------------------

#[test]
fn only_the_exact_capture_key_shape_is_accepted() {
    let ok = |rel: &str, role, kind| validate_rel_key(rel, SOURCE, CAPTURE, role, kind).is_ok();
    assert!(ok(&key("html"), Role::File, Some("page")));
    assert!(ok(&key("pdf"), Role::File, Some("pdf")));
    assert!(ok(&key("txt"), Role::Text, Some("page")));
    assert!(ok(&key("txt"), Role::Text, Some("selection")));
    assert!(ok(&key("pdf"), Role::File, None));

    for bad in [
        format!("/web-captures/{SOURCE}/{CAPTURE}.html"),
        format!("C:/web-captures/{SOURCE}/{CAPTURE}.html"),
        format!("C:\\web-captures\\{SOURCE}\\{CAPTURE}.html"),
        format!("//server/web-captures/{SOURCE}/{CAPTURE}.html"),
        format!("web-captures/../{SOURCE}/{CAPTURE}.html"),
        format!("web-captures/{SOURCE}/../{CAPTURE}.html"),
        format!("web-captures\\{SOURCE}\\{CAPTURE}.html"),
        format!("assets/{SOURCE}/{CAPTURE}.html"),
        format!("web-captures/{SOURCE}/sub/{CAPTURE}.html"),
        format!("web-captures/{CAPTURE}.html"),
        format!("web-captures/{SOURCE}/{CAPTURE}"),
        format!("web-captures/{SOURCE}/{CAPTURE}.exe"),
        format!("web-captures/{SOURCE}/other.html"),
        format!("web-captures/22222222-0000-4000-8000-000000000000/{CAPTURE}.html"),
        format!("web-captures/{SOURCE}/{CAPTURE}.html:stream"),
        String::new(),
    ] {
        assert!(!ok(&bad, Role::File, Some("page")), "must reject {bad:?}");
    }
    // The extension must agree with the role and the kind.
    assert!(!ok(&key("pdf"), Role::File, Some("page")));
    assert!(!ok(&key("html"), Role::File, Some("pdf")));
    assert!(!ok(&key("html"), Role::Text, Some("page")));
    assert!(!ok(&key("html"), Role::File, Some("selection")));
}

#[test]
fn inbound_rewrite_rejects_bad_keys_and_digests_and_strips_wire_keys() {
    let html = b"<html></html>";
    let mut payload = page_payload(html, Some(&key("txt")));
    {
        let obj = payload.as_object_mut().unwrap();
        obj.insert("text_sha256".into(), Value::String(sha(b"long text")));
        obj.insert("text_size".into(), json!(9));
    }
    let mut obj: Map<String, Value> = payload.as_object().unwrap().clone();
    let files = rewrite_inbound(&mut obj, CAPTURE).expect("valid");
    assert_eq!(files.len(), 2);
    assert!(!obj.contains_key("text_sha256") && !obj.contains_key("text_size"));

    // A text file without its wire digest is not accepted.
    let mut no_digest = page_payload(html, Some(&key("txt")))
        .as_object()
        .unwrap()
        .clone();
    assert!(rewrite_inbound(&mut no_digest, CAPTURE).is_err());

    // A traversal key is refused before anything is queued.
    let mut traversal = page_payload(html, None).as_object().unwrap().clone();
    traversal.insert(
        "rel_path".into(),
        Value::String(format!("web-captures/{SOURCE}/../../../x.html")),
    );
    assert!(rewrite_inbound(&mut traversal, CAPTURE).is_err());

    // A digest that is not a sha256 is refused.
    let mut bad_sha = page_payload(html, None).as_object().unwrap().clone();
    bad_sha.insert("sha256".into(), Value::String("ABC".into()));
    assert!(rewrite_inbound(&mut bad_sha, CAPTURE).is_err());
}

// --------------------------------------------------------------------------
// Push: files before the row
// --------------------------------------------------------------------------

#[tokio::test]
async fn push_uploads_each_file_and_adds_the_text_digest() {
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>snapshot</html>";
    let text = b"a long text that did not fit in the row";
    write_file(dir.path(), &key("html"), html);
    write_file(dir.path(), &key("txt"), text);

    let api = MockSyncApi::default();
    let mut change = change(page_payload(html, Some(&key("txt"))));
    let outcome = prepare_web_capture_push(&api, "tok", dir.path(), &mut change)
        .await
        .expect("prepare");
    assert!(matches!(outcome, WebPushOutcome::Ready));

    let puts = api.put_blobs.lock().unwrap().clone();
    assert!(
        puts.contains(&sha(html)) && puts.contains(&sha(text)),
        "{puts:?}"
    );
    let payload = change.payload.unwrap();
    assert_eq!(payload["text_sha256"], sha(text));
    assert_eq!(payload["text_size"], text.len());
    assert_eq!(payload["sha256"], sha(html), "row digest is untouched");
}

#[tokio::test]
async fn a_blob_the_server_already_holds_is_not_uploaded_again() {
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>same</html>";
    write_file(dir.path(), &key("html"), html);
    let api = MockSyncApi::default();
    api.existing_blobs.lock().unwrap().insert(sha(html));
    let mut change = change(page_payload(html, None));
    let outcome = prepare_web_capture_push(&api, "tok", dir.path(), &mut change)
        .await
        .unwrap();
    assert!(matches!(outcome, WebPushOutcome::Ready));
    assert!(api.put_blobs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_changed_or_missing_file_skips_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>original</html>";
    let path = write_file(dir.path(), &key("html"), html);
    let api = MockSyncApi::default();

    std::fs::write(&path, b"<html>tampered</html>").unwrap();
    let mut changed = change(page_payload(html, None));
    let outcome = prepare_web_capture_push(&api, "tok", dir.path(), &mut changed)
        .await
        .unwrap();
    assert!(
        matches!(outcome, WebPushOutcome::Skip(_)),
        "digest mismatch"
    );
    assert!(api.put_blobs.lock().unwrap().is_empty(), "nothing uploaded");

    std::fs::remove_file(&path).unwrap();
    let mut missing = change(page_payload(html, None));
    let outcome = prepare_web_capture_push(&api, "tok", dir.path(), &mut missing)
        .await
        .unwrap();
    assert!(matches!(outcome, WebPushOutcome::Skip(_)), "file missing");

    // ...unless the server already holds the digest.
    api.existing_blobs.lock().unwrap().insert(sha(html));
    let mut held = change(page_payload(html, None));
    let outcome = prepare_web_capture_push(&api, "tok", dir.path(), &mut held)
        .await
        .unwrap();
    assert!(matches!(outcome, WebPushOutcome::Ready));
}

#[tokio::test]
async fn an_invalid_key_in_a_local_row_is_never_read_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("secret.html");
    std::fs::write(&outside, b"secret").unwrap();
    let html = b"secret";
    let mut payload = page_payload(html, None);
    payload["rel_path"] = json!(format!("web-captures/{SOURCE}/../../secret.html"));
    let api = MockSyncApi::default();
    let mut change = change(payload);
    let outcome = prepare_web_capture_push(&api, "tok", dir.path(), &mut change)
        .await
        .unwrap();
    assert!(matches!(outcome, WebPushOutcome::Skip(_)));
    assert!(api.blob_events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_cycle_uploads_the_file_before_the_row_and_holds_the_row_on_failure() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>cycle</html>";
    write_file(dir.path(), &key("html"), html);
    insert_source(&conn);
    insert_capture_row(&conn, html, None);

    // The upload fails: the row must not be pushed and stays in the oplog.
    let api = MockSyncApi::default();
    api.server_capabilities
        .lock()
        .unwrap()
        .push("web-capture-v1".to_string());
    *api.blob_failure.lock().unwrap() = Some(MockBlobFailure::Network("down".to_string()));
    let result = run_cycle(&api, "tok", &conn, dir.path(), &|_| {}).await;
    assert!(result.is_err(), "a failed upload backs the cycle off");
    assert!(api
        .pushed
        .lock()
        .unwrap()
        .iter()
        .all(|c| c.table != "web_captures"));
    assert!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name='web_captures'"
        ) > 0
    );

    // The network is back: file first, then the row.
    *api.blob_failure.lock().unwrap() = None;
    run_cycle(&api, "tok", &conn, dir.path(), &|_| {})
        .await
        .expect("cycle");
    assert!(api.put_blobs.lock().unwrap().contains(&sha(html)));
    assert!(api
        .pushed
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.table == "web_captures" && c.row_id == CAPTURE));
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name='web_captures'"
        ),
        0
    );
}

#[tokio::test]
async fn a_row_with_an_unreadable_file_is_journaled_and_does_not_block_the_rest() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    // The capture row exists but its file was never written on this device.
    insert_source(&conn);
    insert_capture_row(&conn, b"<html>gone</html>", None);

    let api = MockSyncApi::default();
    api.server_capabilities
        .lock()
        .unwrap()
        .push("web-capture-v1".to_string());
    run_cycle(&api, "tok", &conn, dir.path(), &|_| {})
        .await
        .expect("cycle");
    assert!(api
        .pushed
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.table == "web_sources"));
    assert!(api
        .pushed
        .lock()
        .unwrap()
        .iter()
        .all(|c| c.table != "web_captures"));
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE table_name='web_captures' AND reason='apply_error'"
        ),
        1
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_oplog WHERE table_name='web_captures'"
        ),
        0
    );
}

// --------------------------------------------------------------------------
// Pull: queue, verified install, failures
// --------------------------------------------------------------------------

fn capture_pull_row(html: &[u8], text: Option<&[u8]>, seq: i64) -> PullRow {
    let mut payload = page_payload(html, text.map(|_| key("txt")).as_deref());
    if let Some(text) = text {
        let obj = payload.as_object_mut().unwrap();
        obj.insert("text_sha256".into(), Value::String(sha(text)));
        obj.insert("text_size".into(), json!(text.len()));
    }
    PullRow {
        table: "web_captures".to_string(),
        row_id: CAPTURE.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: 1,
        device_id: "remote".to_string(),
        payload: Some(payload),
    }
}

fn source_pull_row(seq: i64) -> PullRow {
    PullRow {
        table: "web_sources".to_string(),
        row_id: SOURCE.to_string(),
        server_seq: seq,
        deleted: false,
        changed_at: 1,
        device_id: "remote".to_string(),
        payload: Some(json!({
            "id": SOURCE, "original_url": "https://a.test/", "final_url": "https://a.test/",
            "canonical_url": null, "title": "T", "site_name": null,
            "first_accessed_at": "2026-10-02T00:00:00Z", "created_at": 1, "updated_at": 1
        })),
    }
}

fn apply_remote(conn: &Connection, dir: &Path, rows: Vec<PullRow>) {
    let mut ctx = ApplyContext::new(dir);
    apply_page(conn, &mut ctx, &rows, 100).expect("apply");
}

#[tokio::test]
async fn a_pulled_capture_queues_its_files_and_the_drain_installs_them_verified() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>remote page</html>";
    let text = b"remote long text";
    apply_remote(
        &conn,
        dir.path(),
        vec![source_pull_row(1), capture_pull_row(html, Some(text), 2)],
    );

    assert_eq!(count(&conn, "SELECT COUNT(*) FROM web_captures"), 1);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM sync_conflicts"),
        0,
        "wire-only keys are stripped, so no schema drift is journaled"
    );
    assert!(has_pending_download(&conn, CAPTURE));
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM sync_web_pending_blobs"),
        2
    );

    let api = MockSyncApi::default();
    api.put_blob_bytes(&sha(html), html.to_vec());
    api.put_blob_bytes(&sha(text), text.to_vec());
    let installed = drain_pending_web_blobs(&api, "tok", &conn, dir.path())
        .await
        .unwrap();
    assert_eq!(installed, 2);
    assert!(!has_pending_download(&conn, CAPTURE));
    let html_path = dir
        .path()
        .join("web-captures")
        .join(SOURCE)
        .join(format!("{CAPTURE}.html"));
    assert_eq!(std::fs::read(&html_path).unwrap(), html);
    let txt_path = dir
        .path()
        .join("web-captures")
        .join(SOURCE)
        .join(format!("{CAPTURE}.txt"));
    assert_eq!(std::fs::read(&txt_path).unwrap(), text);
    let leftovers: Vec<_> = std::fs::read_dir(html_path.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".part"))
        .collect();
    assert!(leftovers.is_empty(), "no temp files remain: {leftovers:?}");
}

#[tokio::test]
async fn a_file_already_on_disk_with_the_right_size_is_not_queued() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>already here</html>";
    write_file(dir.path(), &key("html"), html);
    apply_remote(
        &conn,
        dir.path(),
        vec![source_pull_row(1), capture_pull_row(html, None, 2)],
    );
    assert!(!has_pending_download(&conn, CAPTURE));
}

#[tokio::test]
async fn corrupt_bytes_are_never_installed_and_stay_queued() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>expected</html>";
    apply_remote(
        &conn,
        dir.path(),
        vec![source_pull_row(1), capture_pull_row(html, None, 2)],
    );
    let api = MockSyncApi::default();
    api.put_blob_bytes(&sha(html), b"<html>EVIL!!!!</html>".to_vec());
    let installed = drain_pending_web_blobs(&api, "tok", &conn, dir.path())
        .await
        .unwrap();
    assert_eq!(installed, 0);
    let target = dir
        .path()
        .join("web-captures")
        .join(SOURCE)
        .join(format!("{CAPTURE}.html"));
    assert!(!target.exists(), "a mismatching file is not installed");
    assert!(!target.with_extension("html.part").exists());
    assert_eq!(
        count(&conn, "SELECT retry_count FROM sync_web_pending_blobs"),
        1
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='blob_hash_mismatch' AND table_name='web_captures'"
        ),
        1
    );
}

#[tokio::test]
async fn a_blob_the_server_never_has_is_given_up_after_bounded_retries() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>lost</html>";
    apply_remote(
        &conn,
        dir.path(),
        vec![source_pull_row(1), capture_pull_row(html, None, 2)],
    );
    let api = MockSyncApi::default(); // serves no bytes: 404
    for _ in 0..MAX_DOWNLOAD_RETRIES {
        drain_pending_web_blobs(&api, "tok", &conn, dir.path())
            .await
            .unwrap();
    }
    assert!(!has_pending_download(&conn, CAPTURE));
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='blob_missing' AND table_name='web_captures'"
        ),
        1
    );
    // The row stays; the UI reports the file as not available.
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM web_captures"), 1);
}

#[tokio::test]
async fn an_unsafe_key_in_a_pulled_row_is_journaled_and_nothing_is_queued() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let html = b"x";
    let mut row = capture_pull_row(html, None, 2);
    row.payload.as_mut().unwrap()["rel_path"] =
        json!(format!("web-captures/{SOURCE}/../../escape.html"));
    apply_remote(&conn, dir.path(), vec![source_pull_row(1), row]);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM web_captures"), 0);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM sync_web_pending_blobs"),
        0
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_conflicts WHERE reason='apply_error' AND table_name='web_captures'"
        ),
        1
    );
}

#[tokio::test]
async fn nothing_is_written_when_the_source_folder_is_not_a_folder() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>blocked</html>";
    apply_remote(
        &conn,
        dir.path(),
        vec![source_pull_row(1), capture_pull_row(html, None, 2)],
    );
    // A regular file squats on the source folder name.
    write_file(
        dir.path(),
        &format!("web-captures/{SOURCE}"),
        b"not a folder",
    );
    let api = MockSyncApi::default();
    api.put_blob_bytes(&sha(html), html.to_vec());
    let installed = drain_pending_web_blobs(&api, "tok", &conn, dir.path())
        .await
        .unwrap();
    assert_eq!(installed, 0);
    assert!(has_pending_download(&conn, CAPTURE), "retried later");
}

#[tokio::test]
async fn queue_entries_of_a_deleted_capture_are_dropped() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>doomed</html>";
    apply_remote(
        &conn,
        dir.path(),
        vec![source_pull_row(1), capture_pull_row(html, None, 2)],
    );
    assert!(has_pending_download(&conn, CAPTURE));
    apply_remote(
        &conn,
        dir.path(),
        vec![PullRow {
            table: "web_sources".to_string(),
            row_id: SOURCE.to_string(),
            server_seq: 9,
            deleted: true,
            changed_at: 2,
            device_id: "remote".to_string(),
            payload: None,
        }],
    );
    let api = MockSyncApi::default();
    drain_pending_web_blobs(&api, "tok", &conn, dir.path())
        .await
        .unwrap();
    assert!(!has_pending_download(&conn, CAPTURE));
    assert!(
        api.blob_events.lock().unwrap().is_empty(),
        "nothing fetched"
    );
}

#[tokio::test]
async fn the_cycle_pulls_a_capture_and_installs_its_file() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>via cycle</html>";
    let api = MockSyncApi::default();
    api.put_blob_bytes(&sha(html), html.to_vec());
    api.queue_pull_page(PullResponse {
        rows: vec![source_pull_row(1), capture_pull_row(html, None, 2)],
        next_since: 2,
        has_more: false,
        schema_tag: String::new(),
        server_epoch: EPOCH.to_string(),
        server_now_ms: 1_700_000_000_000,
        capabilities: vec!["web-capture-v1".to_string()],
    });
    run_cycle(&api, "tok", &conn, dir.path(), &|_| {})
        .await
        .expect("cycle");
    let path = dir
        .path()
        .join("web-captures")
        .join(SOURCE)
        .join(format!("{CAPTURE}.html"));
    assert_eq!(std::fs::read(path).unwrap(), html);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM sync_web_pending_blobs"),
        0
    );
}

#[test]
fn the_status_counts_pending_web_files() {
    let conn = session_db();
    conn.execute(
        "INSERT INTO sync_web_pending_blobs(capture_id, role, sha256, rel_path, size)
         VALUES ('c', 'file', ?1, 'web-captures/s/c.html', 3)",
        [sha(b"abc")],
    )
    .unwrap();
    let status = build_status(&conn, SyncState::Idle, None);
    assert_eq!(status.blobs_pending, 1);
}

#[tokio::test]
async fn enqueue_is_idempotent_and_roles_are_separate() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let mut obj: Map<String, Value> = page_payload(b"h", Some(&key("txt")))
        .as_object()
        .unwrap()
        .clone();
    obj.insert("text_sha256".into(), Value::String(sha(b"t")));
    obj.insert("text_size".into(), json!(1));
    let files = rewrite_inbound(&mut obj, CAPTURE).unwrap();
    enqueue_downloads(&conn, CAPTURE, &files, dir.path()).unwrap();
    enqueue_downloads(&conn, CAPTURE, &files, dir.path()).unwrap();
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM sync_web_pending_blobs"),
        2
    );
}

// --------------------------------------------------------------------------
// Remote deletes: the source folder
// --------------------------------------------------------------------------

#[tokio::test]
async fn a_remote_source_tombstone_removes_the_local_folder_after_the_page_commits() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let html = b"<html>to delete</html>";
    write_file(dir.path(), &key("html"), html);
    apply_remote(
        &conn,
        dir.path(),
        vec![source_pull_row(1), capture_pull_row(html, None, 2)],
    );
    let folder = dir.path().join("web-captures").join(SOURCE);
    assert!(folder.exists());

    apply_remote(
        &conn,
        dir.path(),
        vec![PullRow {
            table: "web_sources".to_string(),
            row_id: SOURCE.to_string(),
            server_seq: 9,
            deleted: true,
            changed_at: 2,
            device_id: "remote".to_string(),
            payload: None,
        }],
    );
    assert!(folder.exists(), "the delete itself does not touch the disk");
    assert_eq!(drain_folder_removals(&conn, dir.path()).unwrap(), 1);
    assert!(!folder.exists());
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_meta WHERE key LIKE 'web_capture_remove_dir:%'"
        ),
        0
    );
}

#[test]
fn a_folder_is_never_removed_while_its_source_row_exists() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    insert_source(&conn);
    write_file(dir.path(), &key("html"), b"keep me");
    queue_folder_removal(&conn, SOURCE).unwrap();
    assert_eq!(drain_folder_removals(&conn, dir.path()).unwrap(), 0);
    assert!(dir.path().join("web-captures").join(SOURCE).exists());
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_meta WHERE key LIKE 'web_capture_remove_dir:%'"
        ),
        0,
        "the stale entry is dropped"
    );
}

#[test]
fn only_plain_ids_can_name_a_folder_to_remove() {
    let conn = session_db();
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("keep.txt");
    std::fs::write(&outside, b"x").unwrap();
    queue_folder_removal(&conn, "..").unwrap();
    queue_folder_removal(&conn, "../..").unwrap();
    drain_folder_removals(&conn, dir.path()).unwrap();
    assert!(outside.exists());
    assert!(dir.path().exists());
}
