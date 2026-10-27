//! Live round trips against the Zotero Web API (plan section 11).
//!
//! Opt-in like the other live Zotero tests: they run only with
//! `ZSB_LIVE_ZOTERO_WRITE=1` and the user-configured key file, against the
//! test group, and delete what they create. Run one at a time:
//! `ZSB_LIVE_ZOTERO_WRITE=1 cargo test --lib annotations::live_tests -- --test-threads=1`.

use serde_json::{json, Value};

use super::api::{AnnotationApi, CreateResult};
use super::engine::{sync_library, ParentCatalog};
use super::http::{WebAnnotationApi, API_BASE};
use super::machine::{local_delete, local_edit, local_restore};
use super::model::{AnnotationRow, SyncState};
use super::repo::{self, Scope};
use super::repo_test_support::archive_with_library;
use super::wire::{self, Data};

const TEST_GROUP: u64 = 6680944;
const KEY_FILE: &str = "C:/Users/agusn/.zsb/zotero_key.txt";
const FIXTURE_TITLE: &str = "EntropIA annotations live test BORRAR";

fn live_key() -> Option<String> {
    if std::env::var("ZSB_LIVE_ZOTERO_WRITE").is_err() {
        println!("skipping live annotation test: ZSB_LIVE_ZOTERO_WRITE is not set");
        return None;
    }
    let key = std::fs::read_to_string(KEY_FILE).expect("key file");
    Some(key.trim().to_string())
}

/// Raw calls for what the engine never does: making the parent, editing as
/// Zotero would, and cleaning up.
struct Raw {
    client: reqwest::blocking::Client,
    key: String,
}

impl Raw {
    fn new(key: &str) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("http");
        Raw { client, key: key.to_string() }
    }

    fn url(path: &str) -> String {
        format!("{API_BASE}/groups/{TEST_GROUP}{path}")
    }

    fn create(&self, item: Value) -> String {
        let answer: Value = self
            .client
            .post(Self::url("/items"))
            .header("Zotero-API-Key", &self.key)
            .json(&json!([item]))
            .send()
            .expect("create")
            .json()
            .expect("create answer");
        answer["successful"]["0"]["key"]
            .as_str()
            .unwrap_or_else(|| panic!("not created: {answer}"))
            .to_string()
    }

    fn get(&self, key: &str) -> Option<Value> {
        let response = self
            .client
            .get(Self::url(&format!("/items/{key}?includeTrashed=1")))
            .header("Zotero-API-Key", &self.key)
            .send()
            .expect("get");
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return None;
        }
        Some(response.json().expect("item"))
    }

    fn patch(&self, key: &str, fields: Value) {
        let version = self.get(key).expect("item")["version"].as_i64().unwrap();
        let status = self
            .client
            .patch(Self::url(&format!("/items/{key}")))
            .header("Zotero-API-Key", &self.key)
            .header("If-Unmodified-Since-Version", version.to_string())
            .json(&fields)
            .send()
            .expect("patch")
            .status();
        assert!(status.is_success(), "patch {key}: {status}");
    }

    fn delete(&self, key: &str) {
        if let Some(item) = self.get(key) {
            let version = item["version"].as_i64().unwrap();
            let _ = self
                .client
                .delete(Self::url(&format!("/items/{key}")))
                .header("Zotero-API-Key", &self.key)
                .header("If-Unmodified-Since-Version", version.to_string())
                .send();
        }
    }
}

/// A book with one PDF attachment to annotate; deleted on drop, children too.
struct Fixture {
    raw: Raw,
    book: String,
    attachment: String,
}

impl Fixture {
    fn new(key: &str) -> Self {
        let raw = Raw::new(key);
        let stamp = crate::processing::repository::now_ms();
        let book = raw.create(json!({
            "itemType": "book",
            "title": format!("{FIXTURE_TITLE} {stamp}"),
            "tags": [{"tag": "zsb-probe"}],
        }));
        // Owned before the next request, so a failure below still cleans up.
        let mut fixture = Fixture { raw, book, attachment: String::new() };
        // Group libraries take no linked files: a stored-file attachment whose
        // file is never uploaded is enough to annotate.
        fixture.attachment = fixture.raw.create(json!({
            "itemType": "attachment",
            "parentItem": fixture.book,
            "linkMode": "imported_file",
            "title": "probe.pdf",
            "filename": "probe.pdf",
            "contentType": "application/pdf",
        }));
        fixture
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.attachment.is_empty() {
            self.raw.delete(&self.attachment);
        }
        self.raw.delete(&self.book);
    }
}

struct OnServer;

impl ParentCatalog for OnServer {
    fn has_attachment(&self, _parent_key: &str) -> bool {
        true
    }
}

fn highlight(parent: &str, key: &str, comment: &str) -> Data {
    json!({
        "key": key,
        "itemType": "annotation",
        "parentItem": parent,
        "annotationType": "highlight",
        "annotationText": "live probe",
        "annotationComment": comment,
        "annotationColor": "#ffd400",
        "annotationPageLabel": "1",
        "annotationSortIndex": "00000|000000|00100",
        "annotationPosition": "{\"pageIndex\":0,\"rects\":[[100,600,200,612]]}",
        "tags": [],
    })
    .as_object()
    .cloned()
    .unwrap()
}

fn sync(conn: &rusqlite::Connection, scope: &Scope, api: &WebAnnotationApi) {
    // Each test is its own "first run in the process" for its scope.
    let report = sync_library(conn, scope, api, &OnServer, crate::processing::repository::now_ms())
        .expect("sync");
    assert_eq!(report.paused, None, "the run went through: {report:?}");
}

fn edit_comment(conn: &rusqlite::Connection, scope: &Scope, key: &str, comment: &str) {
    let mut row = repo::load(conn, scope, key).unwrap().expect("row");
    let mut desired = row.desired().cloned().unwrap();
    desired.insert("annotationComment".into(), json!(comment));
    local_edit(&mut row, desired);
    repo::save(conn, scope, &row, 1).unwrap();
}

fn scope(name: &str) -> Scope {
    Scope { library_row_id: name.into(), web_user_id: format!("group-{TEST_GROUP}") }
}

#[test]
fn live_a_client_key_create_keeps_its_key_and_is_never_duplicated() {
    let Some(key) = live_key() else { return };
    let fixture = Fixture::new(&key);
    let api = WebAnnotationApi::for_group(API_BASE, &key, TEST_GROUP).unwrap();
    let annotation_key = wire::generate_key();
    let mut body = highlight(&fixture.attachment, &annotation_key, "contract");
    body.insert("version".into(), json!(0));

    let first = api.create(std::slice::from_ref(&body)).expect("first post");
    assert!(matches!(first[0], CreateResult::Created { .. }), "{first:?}");
    let stored = fixture.raw.get(&annotation_key).expect("created under the client key");
    assert_eq!(stored["key"], json!(annotation_key));

    let again = api.create(std::slice::from_ref(&body)).expect("second post");
    assert_eq!(again[0], CreateResult::Exists, "version 0 means \"must not exist\"");
}

#[test]
fn live_create_edit_and_delete_round_trip() {
    let Some(key) = live_key() else { return };
    let fixture = Fixture::new(&key);
    let api = WebAnnotationApi::for_group(API_BASE, &key, TEST_GROUP).unwrap();
    let scope = scope("live-roundtrip");
    let conn = archive_with_library(&scope.library_row_id);
    let annotation_key = wire::generate_key();
    let row = AnnotationRow::new_local(&annotation_key, highlight(&fixture.attachment, &annotation_key, "made in EntropIA"));
    repo::save(&conn, &scope, &row, 1).unwrap();

    sync(&conn, &scope, &api);
    let server = fixture.raw.get(&annotation_key).expect("created");
    assert_eq!(server["data"]["annotationComment"], json!("made in EntropIA"));
    assert_eq!(repo::load(&conn, &scope, &annotation_key).unwrap().unwrap().sync_state, SyncState::Synced);

    edit_comment(&conn, &scope, &annotation_key, "edited in EntropIA");
    sync(&conn, &scope, &api);
    assert_eq!(
        fixture.raw.get(&annotation_key).unwrap()["data"]["annotationComment"],
        json!("edited in EntropIA")
    );

    fixture.raw.patch(&annotation_key, json!({"annotationComment": "edited in Zotero"}));
    sync(&conn, &scope, &api);
    let pulled = repo::load(&conn, &scope, &annotation_key).unwrap().unwrap();
    assert_eq!(pulled.desired().unwrap()["annotationComment"], json!("edited in Zotero"));

    let mut doomed = repo::load(&conn, &scope, &annotation_key).unwrap().unwrap();
    local_delete(&mut doomed).unwrap();
    repo::save(&conn, &scope, &doomed, 1).unwrap();
    sync(&conn, &scope, &api);
    assert!(fixture.raw.get(&annotation_key).is_none(), "deleted on the server");
    assert!(repo::load(&conn, &scope, &annotation_key).unwrap().is_none());
}

#[test]
fn live_trash_and_restore_round_trip() {
    let Some(key) = live_key() else { return };
    let fixture = Fixture::new(&key);
    let api = WebAnnotationApi::for_group(API_BASE, &key, TEST_GROUP).unwrap();
    let scope = scope("live-trash");
    let conn = archive_with_library(&scope.library_row_id);
    let annotation_key = wire::generate_key();
    repo::save(
        &conn,
        &scope,
        &AnnotationRow::new_local(&annotation_key, highlight(&fixture.attachment, &annotation_key, "trash me")),
        1,
    )
    .unwrap();
    sync(&conn, &scope, &api);

    fixture.raw.patch(&annotation_key, json!({"deleted": 1}));
    sync(&conn, &scope, &api);
    let mut trashed = repo::load(&conn, &scope, &annotation_key).unwrap().unwrap();
    assert!(trashed.remote_trashed, "the pull saw the trash through includeTrashed=1");

    local_restore(&mut trashed).unwrap();
    repo::save(&conn, &scope, &trashed, 1).unwrap();
    sync(&conn, &scope, &api);
    let server = fixture.raw.get(&annotation_key).unwrap();
    assert!(!wire::is_trashed(server["data"].as_object().unwrap()), "restored in Zotero");
    assert!(!repo::load(&conn, &scope, &annotation_key).unwrap().unwrap().remote_trashed);
}

#[test]
fn live_editing_the_same_comment_on_both_sides_is_a_conflict() {
    let Some(key) = live_key() else { return };
    let fixture = Fixture::new(&key);
    let api = WebAnnotationApi::for_group(API_BASE, &key, TEST_GROUP).unwrap();
    let scope = scope("live-conflict");
    let conn = archive_with_library(&scope.library_row_id);
    let annotation_key = wire::generate_key();
    repo::save(
        &conn,
        &scope,
        &AnnotationRow::new_local(&annotation_key, highlight(&fixture.attachment, &annotation_key, "A")),
        1,
    )
    .unwrap();
    sync(&conn, &scope, &api);

    edit_comment(&conn, &scope, &annotation_key, "B in EntropIA");
    fixture.raw.patch(&annotation_key, json!({"annotationComment": "C in Zotero"}));
    sync(&conn, &scope, &api);

    let row = repo::load(&conn, &scope, &annotation_key).unwrap().unwrap();
    assert_eq!(row.sync_state, SyncState::Conflict);
    assert_eq!(row.local_json.as_ref().unwrap()["annotationComment"], json!("B in EntropIA"));
    assert_eq!(row.conflict_json.as_ref().unwrap()["annotationComment"], json!("C in Zotero"));
    assert_eq!(
        fixture.raw.get(&annotation_key).unwrap()["data"]["annotationComment"],
        json!("C in Zotero"),
        "nothing was overwritten in Zotero"
    );
}
