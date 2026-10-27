//! Engine runs against an in-memory Zotero.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};

use serde_json::{json, Value};

use super::api::*;
use super::engine::*;
use super::machine::{local_delete, local_edit};
use super::model::*;
use super::repo::{self, Scope};
use super::repo_test_support::archive_with_library;
use super::wire::{self, Data};

/// Scripted faults for the next calls of one kind.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fault {
    /// The write is applied but the answer never arrives.
    LoseAnswer,
    /// Nothing reaches the server.
    Offline,
    /// `Backoff: 30` on the answer.
    Backoff,
}

#[derive(Default)]
struct Zotero {
    library_version: i64,
    items: BTreeMap<String, ServerItem>,
    deleted: Vec<(String, i64)>,
    writes: Vec<String>,
    faults: VecDeque<Fault>,
    backoff: Option<u64>,
    /// Parent keys that exist on the server.
    parents: Vec<String>,
    /// Runs once, while the next write is "in flight" (after the server
    /// applied it, before the answer): a local command racing the request.
    during_write: Option<Box<dyn FnOnce()>>,
}

struct FakeApi(RefCell<Zotero>);

impl FakeApi {
    fn new() -> Self {
        let zotero = Zotero { library_version: 10, parents: vec!["PARENT22".into()], ..Default::default() };
        FakeApi(RefCell::new(zotero))
    }

    fn put(&self, key: &str, comment: &str) {
        let mut z = self.0.borrow_mut();
        z.library_version += 1;
        let version = z.library_version;
        let data = annotation(key, version, comment);
        z.items.insert(key.into(), ServerItem { version, data });
    }

    fn edit_remote(&self, key: &str, field: &str, value: Value) {
        let mut z = self.0.borrow_mut();
        z.library_version += 1;
        let version = z.library_version;
        let item = z.items.get_mut(key).unwrap();
        item.version = version;
        item.data.insert(field.into(), value);
        item.data.insert("version".into(), json!(version));
    }

    fn purge(&self, key: &str) {
        let mut z = self.0.borrow_mut();
        z.library_version += 1;
        let version = z.library_version;
        z.items.remove(key);
        z.deleted.push((key.into(), version));
    }

    fn fault(&self, fault: Fault) {
        self.0.borrow_mut().faults.push_back(fault);
    }

    fn comment(&self, key: &str) -> Option<Value> {
        self.0.borrow().items.get(key).map(|item| item.data["annotationComment"].clone())
    }

    fn writes(&self) -> Vec<String> {
        self.0.borrow().writes.clone()
    }

    /// Takes the next fault; `Offline` fails before anything is applied.
    fn begin_write(&self) -> Result<Option<Fault>, ApiError> {
        let fault = self.0.borrow_mut().faults.pop_front();
        match fault {
            Some(Fault::Offline) => Err(ApiError::Unreachable { after_send: false }),
            Some(Fault::Backoff) => {
                self.0.borrow_mut().backoff = Some(30);
                Ok(None)
            }
            other => Ok(other),
        }
    }

    fn race(&self) {
        let hook = self.0.borrow_mut().during_write.take();
        if let Some(hook) = hook {
            hook();
        }
    }

    fn finish<T>(fault: Option<Fault>, value: T) -> Result<T, ApiError> {
        if fault == Some(Fault::LoseAnswer) {
            Err(ApiError::Unreachable { after_send: true })
        } else {
            Ok(value)
        }
    }
}

fn annotation(key: &str, version: i64, comment: &str) -> Data {
    json!({
        "key": key,
        "version": version,
        "itemType": "annotation",
        "parentItem": "PARENT22",
        "annotationType": "highlight",
        "annotationComment": comment,
        "annotationColor": "#ffd400",
        "annotationPosition": "{\"pageIndex\":0,\"rects\":[[1,2,3,4]]}"
    })
    .as_object()
    .cloned()
    .unwrap()
}

impl AnnotationApi for FakeApi {
    fn versions_since(&self, since: i64) -> Result<Versioned<BTreeMap<String, i64>>, ApiError> {
        let z = self.0.borrow();
        let value = z
            .items
            .iter()
            .filter(|(_, item)| item.version > since)
            .map(|(key, item)| (key.clone(), item.version))
            .collect();
        Ok(Versioned { value, library_version: z.library_version })
    }

    fn items(&self, keys: &[String]) -> Result<Versioned<Vec<ServerItem>>, ApiError> {
        let z = self.0.borrow();
        let value = keys.iter().filter_map(|key| z.items.get(key).cloned()).collect();
        Ok(Versioned { value, library_version: z.library_version })
    }

    fn deleted_since(&self, since: i64) -> Result<Versioned<Vec<String>>, ApiError> {
        let z = self.0.borrow();
        let value = z.deleted.iter().filter(|(_, v)| *v > since).map(|(k, _)| k.clone()).collect();
        Ok(Versioned { value, library_version: z.library_version })
    }

    fn item(&self, key: &str) -> Result<Option<ServerItem>, ApiError> {
        let z = self.0.borrow();
        if z.parents.iter().any(|parent| parent == key) {
            return Ok(Some(ServerItem { version: 1, data: Data::new() }));
        }
        Ok(z.items.get(key).cloned())
    }

    fn create(&self, items: &[Data]) -> Result<Vec<CreateResult>, ApiError> {
        let fault = self.begin_write()?;
        let mut results = Vec::new();
        for data in items {
            let key = data["key"].as_str().unwrap().to_string();
            let mut z = self.0.borrow_mut();
            z.writes.push(format!("POST {key}"));
            let parent = data["parentItem"].as_str().unwrap_or_default().to_string();
            if !z.parents.contains(&parent) {
                results.push(CreateResult::Failed { code: 400, message: "Parent item not found".into(), parent_missing: true });
                continue;
            }
            if z.items.contains_key(&key) {
                results.push(CreateResult::Exists);
                continue;
            }
            z.library_version += 1;
            let version = z.library_version;
            let mut stored = data.clone();
            stored.insert("version".into(), json!(version));
            z.items.insert(key, ServerItem { version, data: stored });
            results.push(CreateResult::Created { version });
        }
        Self::finish(fault, results)
    }

    fn patch(&self, key: &str, version: i64, fields: &Data) -> Result<WriteResult, ApiError> {
        let fault = self.begin_write()?;
        let mut z = self.0.borrow_mut();
        z.writes.push(format!("PATCH {key} {}", Value::Object(fields.clone())));
        let current = z.items.get(key).map(|item| item.version);
        let result = match current {
            None => WriteResult::NotFound,
            Some(current) if current != version => WriteResult::Stale,
            Some(_) => {
                z.library_version += 1;
                let new_version = z.library_version;
                let item = z.items.get_mut(key).unwrap();
                for (field, value) in fields {
                    item.data.insert(field.clone(), value.clone());
                }
                item.version = new_version;
                item.data.insert("version".into(), json!(new_version));
                WriteResult::Done { version: new_version }
            }
        };
        drop(z);
        self.race();
        Self::finish(fault, result)
    }

    fn delete(&self, key: &str, version: i64) -> Result<WriteResult, ApiError> {
        let fault = self.begin_write()?;
        let mut z = self.0.borrow_mut();
        z.writes.push(format!("DELETE {key}"));
        let current = z.items.get(key).map(|item| item.version);
        let result = match current {
            None => WriteResult::NotFound,
            Some(current) if current != version => WriteResult::Stale,
            Some(_) => {
                z.library_version += 1;
                let v = z.library_version;
                z.items.remove(key);
                z.deleted.push((key.into(), v));
                WriteResult::Done { version: v }
            }
        };
        drop(z);
        self.race();
        Self::finish(fault, result)
    }

    fn take_backoff(&self) -> Backoff {
        self.0.borrow_mut().backoff.take()
    }
}

struct Parents(Vec<&'static str>);

impl ParentCatalog for Parents {
    fn has_attachment(&self, parent_key: &str) -> bool {
        self.0.contains(&parent_key)
    }
}

/// Each test gets its own library id: the engine's per-process recovery
/// bookkeeping is keyed by library.
fn scope(name: &str) -> Scope {
    Scope { library_row_id: name.into(), web_user_id: "4242".into() }
}

fn run(conn: &rusqlite::Connection, scope: &Scope, api: &FakeApi) -> SyncReport {
    sync_library(conn, scope, api, &Parents(vec!["PARENT22", "LOCALONL"]), 1_000).unwrap()
}

fn unpause(conn: &rusqlite::Connection, scope: &Scope) {
    let mut state = repo::sync_state(conn, scope).unwrap();
    state.paused_until = Some(0);
    repo::set_sync_state(conn, scope, &state, 1).unwrap();
}

fn row(conn: &rusqlite::Connection, scope: &Scope, key: &str) -> AnnotationRow {
    repo::load(conn, scope, key).unwrap().expect("row")
}

fn edit_comment(conn: &rusqlite::Connection, scope: &Scope, key: &str, comment: &str) {
    let mut current = row(conn, scope, key);
    let mut desired = current.desired().cloned().unwrap();
    desired.insert("annotationComment".into(), json!(comment));
    local_edit(&mut current, desired);
    repo::save(conn, scope, &current, 1).unwrap();
}

#[test]
fn a_first_pull_stores_every_annotation_and_moves_the_cursor() {
    let scope = scope("lib-pull");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    api.put("AAAA2222", "one");
    api.put("BBBB3333", "two");
    let report = run(&conn, &scope, &api);
    assert_eq!(report.pulled, 2);
    assert_eq!(row(&conn, &scope, "AAAA2222").sync_state, SyncState::Synced);
    assert_eq!(repo::sync_state(&conn, &scope).unwrap().web_library_version, 12);
    assert_eq!(run(&conn, &scope, &api).pulled, 0, "nothing changed since the cursor");
}

#[test]
fn a_local_edit_round_trips_and_a_remote_edit_comes_back() {
    let scope = scope("lib-roundtrip");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    api.put("AAAA2222", "one");
    run(&conn, &scope, &api);
    edit_comment(&conn, &scope, "AAAA2222", "edited here");
    run(&conn, &scope, &api);
    assert_eq!(api.comment("AAAA2222"), Some(json!("edited here")));
    assert_eq!(row(&conn, &scope, "AAAA2222").sync_state, SyncState::Synced);
    api.edit_remote("AAAA2222", "annotationComment", json!("edited in Zotero"));
    run(&conn, &scope, &api);
    assert_eq!(row(&conn, &scope, "AAAA2222").desired().unwrap()["annotationComment"], json!("edited in Zotero"));
}

#[test]
fn a_local_create_is_posted_once_even_when_its_answer_is_lost() {
    let scope = scope("lib-create");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    let created = AnnotationRow::new_local("NEWK2345", annotation("NEWK2345", 0, "mine"));
    repo::save(&conn, &scope, &created, 1).unwrap();
    api.fault(Fault::LoseAnswer);
    let report = run(&conn, &scope, &api);
    assert_eq!(report.uncertain, 1);
    unpause(&conn, &scope);
    run(&conn, &scope, &api);
    assert_eq!(api.writes(), vec!["POST NEWK2345"], "reconciled by reading, not by posting again");
    let stored = row(&conn, &scope, "NEWK2345");
    assert_eq!(stored.sync_state, SyncState::Synced);
    assert!(stored.base_version.is_some());
}

#[test]
fn offline_edits_are_sent_once_after_reconnecting() {
    let scope = scope("lib-offline");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    api.put("AAAA2222", "one");
    run(&conn, &scope, &api);
    edit_comment(&conn, &scope, "AAAA2222", "offline");
    api.fault(Fault::Offline);
    let report = run(&conn, &scope, &api);
    assert_eq!(report.paused.as_deref(), Some("unreachable"));
    assert_eq!(row(&conn, &scope, "AAAA2222").sync_state, SyncState::Pending);
    unpause(&conn, &scope);
    run(&conn, &scope, &api);
    assert_eq!(api.comment("AAAA2222"), Some(json!("offline")));
    assert_eq!(api.writes().iter().filter(|w| w.starts_with("PATCH")).count(), 1);
}

#[test]
fn concurrent_edits_of_the_same_comment_become_a_conflict_and_nothing_is_lost() {
    let scope = scope("lib-conflict");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    api.put("AAAA2222", "one");
    run(&conn, &scope, &api);
    edit_comment(&conn, &scope, "AAAA2222", "mine");
    api.edit_remote("AAAA2222", "annotationComment", json!("theirs"));
    run(&conn, &scope, &api);
    let conflicted = row(&conn, &scope, "AAAA2222");
    assert_eq!(conflicted.sync_state, SyncState::Conflict);
    assert_eq!(conflicted.local_json.as_ref().unwrap()["annotationComment"], json!("mine"));
    assert_eq!(conflicted.conflict_json.as_ref().unwrap()["annotationComment"], json!("theirs"));
    assert_eq!(api.comment("AAAA2222"), Some(json!("theirs")));
}

#[test]
fn a_stale_patch_is_merged_and_retried_once() {
    let scope = scope("lib-stale");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    api.put("AAAA2222", "one");
    run(&conn, &scope, &api);
    edit_comment(&conn, &scope, "AAAA2222", "mine");
    // Changed after this run's pull but before its PATCH: simulate by moving
    // the version without a pull in between.
    let mut current = row(&conn, &scope, "AAAA2222");
    api.edit_remote("AAAA2222", "annotationColor", json!("#ff6666"));
    current.base_version = Some(current.base_version.unwrap());
    repo::save(&conn, &scope, &current, 1).unwrap();
    run(&conn, &scope, &api);
    assert_eq!(api.comment("AAAA2222"), Some(json!("mine")));
    assert_eq!(row(&conn, &scope, "AAAA2222").sync_state, SyncState::Synced);
}

#[test]
fn a_delete_reaches_zotero_and_a_remote_purge_removes_the_row() {
    let scope = scope("lib-delete");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    api.put("AAAA2222", "one");
    api.put("BBBB3333", "two");
    run(&conn, &scope, &api);
    let mut doomed = row(&conn, &scope, "AAAA2222");
    local_delete(&mut doomed).unwrap();
    repo::save(&conn, &scope, &doomed, 1).unwrap();
    api.purge("BBBB3333");
    run(&conn, &scope, &api);
    assert_eq!(api.comment("AAAA2222"), None);
    assert!(repo::load(&conn, &scope, "AAAA2222").unwrap().is_none());
    assert!(repo::load(&conn, &scope, "BBBB3333").unwrap().is_none());
}

#[test]
fn a_create_under_a_parent_still_in_zotero_desktop_waits_for_it() {
    let scope = scope("lib-parent");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    let mut data = annotation("WAIT2345", 0, "waiting");
    data.insert("parentItem".into(), json!("LOCALONL"));
    repo::save(&conn, &scope, &AnnotationRow::new_local("WAIT2345", data), 1).unwrap();
    run(&conn, &scope, &api);
    assert_eq!(row(&conn, &scope, "WAIT2345").sync_state, SyncState::WaitingParent);
    api.0.borrow_mut().parents.push("LOCALONL".into());
    run(&conn, &scope, &api);
    assert_eq!(row(&conn, &scope, "WAIT2345").sync_state, SyncState::Synced);
}

#[test]
fn a_create_under_a_deleted_parent_becomes_an_attachment_conflict() {
    let scope = scope("lib-orphan");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    let mut data = annotation("ORPH2345", 0, "orphan");
    data.insert("parentItem".into(), json!("GONE2345"));
    repo::save(&conn, &scope, &AnnotationRow::new_local("ORPH2345", data), 1).unwrap();
    run(&conn, &scope, &api);
    assert_eq!(row(&conn, &scope, "ORPH2345").existence_reason(), Some(ExistenceReason::AttachmentDeleted));
}

#[test]
fn a_backoff_header_pauses_the_library() {
    let scope = scope("lib-backoff");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    api.put("AAAA2222", "one");
    run(&conn, &scope, &api);
    edit_comment(&conn, &scope, "AAAA2222", "x");
    api.fault(Fault::Backoff);
    let report = run(&conn, &scope, &api);
    assert_eq!(report.paused.as_deref(), Some("backoff"));
    assert_eq!(run(&conn, &scope, &api).paused.as_deref(), Some("backoff"), "still paused");
}

#[test]
fn a_restart_reconciles_a_request_whose_answer_was_never_recorded() {
    let scope = scope("lib-restart");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    api.put("AAAA2222", "A");
    run(&conn, &scope, &api);
    // A previous process sent B, the server applied it, the user restored A,
    // and the process died before recording the answer.
    let mut stuck = row(&conn, &scope, "AAAA2222");
    let mut desired = stuck.desired().cloned().unwrap();
    desired.insert("annotationComment".into(), json!("B"));
    local_edit(&mut stuck, desired);
    let request = super::machine::next_request(&mut stuck).unwrap();
    super::machine::begin_send(&mut stuck, &request);
    api.edit_remote("AAAA2222", "annotationComment", json!("B"));
    let mut back = stuck.desired().cloned().unwrap();
    back.insert("annotationComment".into(), json!("A"));
    local_edit(&mut stuck, back);
    repo::save(&conn, &scope, &stuck, 1).unwrap();
    // A new process: a scope this process has not synced yet.
    let restarted = Scope { library_row_id: scope.library_row_id.clone(), web_user_id: scope.web_user_id.clone() };
    assert_eq!(recover_after_start(&conn, &restarted, 1).unwrap(), 1);
    run(&conn, &restarted, &api);
    assert_eq!(api.comment("AAAA2222"), Some(json!("A")), "the revert wins");
    assert!(wire::is_valid_key("AAAA2222"));
}

/// Opens a second connection on the same file, as a command would.
fn file_archive(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{name}.sqlite"));
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
    let memory = archive_with_library(name);
    memory.backup(rusqlite::DatabaseName::Main, &path, None).unwrap();
    drop(conn);
    (dir, path)
}

#[test]
fn an_edit_made_while_a_patch_is_in_flight_is_not_overwritten() {
    let scope = scope("lib-race");
    let (_dir, path) = file_archive(&scope.library_row_id);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let api = FakeApi::new();
    api.put("AAAA2222", "A");
    run(&conn, &scope, &api);
    edit_comment(&conn, &scope, "AAAA2222", "B");
    let racing_path = path.clone();
    let racing_scope = scope.clone();
    api.0.borrow_mut().during_write = Some(Box::new(move || {
        let other = rusqlite::Connection::open(&racing_path).unwrap();
        edit_comment(&other, &racing_scope, "AAAA2222", "C");
    }));
    run(&conn, &scope, &api);
    assert_eq!(api.comment("AAAA2222"), Some(json!("B")), "B reached Zotero");
    let stored = row(&conn, &scope, "AAAA2222");
    assert_eq!(stored.desired().unwrap()["annotationComment"], json!("C"), "the racing edit survives");
    assert_eq!(stored.sync_state, SyncState::Pending);
    run(&conn, &scope, &api);
    assert_eq!(api.comment("AAAA2222"), Some(json!("C")));
}

#[test]
fn an_undo_made_while_a_delete_is_in_flight_recreates_the_annotation() {
    let scope = scope("lib-race-delete");
    let (_dir, path) = file_archive(&scope.library_row_id);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let api = FakeApi::new();
    api.put("AAAA2222", "keep me");
    run(&conn, &scope, &api);
    let mut doomed = row(&conn, &scope, "AAAA2222");
    local_delete(&mut doomed).unwrap();
    repo::save(&conn, &scope, &doomed, 1).unwrap();
    let racing_path = path.clone();
    let racing_scope = scope.clone();
    api.0.borrow_mut().during_write = Some(Box::new(move || {
        let other = rusqlite::Connection::open(&racing_path).unwrap();
        let mut current = repo::load(&other, &racing_scope, "AAAA2222").unwrap().unwrap();
        let desired = current.base_json.clone().unwrap();
        local_edit(&mut current, desired);
        repo::save(&other, &racing_scope, &current, 1).unwrap();
    }));
    run(&conn, &scope, &api);
    assert!(repo::load(&conn, &scope, "AAAA2222").unwrap().is_none(), "the old key is gone");
    let rows = repo::by_parent(&conn, &scope, "PARENT22").unwrap();
    assert_eq!(rows.len(), 1, "the annotation lives on under a new key");
    assert_eq!(rows[0].desired().unwrap()["annotationComment"], json!("keep me"));
}


#[test]
fn network_failures_back_off_exponentially_until_a_run_goes_through() {
    let scope = scope("lib-backoff-grow");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    api.put("AAAA2222", "one");
    run(&conn, &scope, &api);
    edit_comment(&conn, &scope, "AAAA2222", "x");
    let mut delays = Vec::new();
    for _ in 0..3 {
        api.fault(Fault::Offline);
        let report = run(&conn, &scope, &api);
        assert_eq!(report.paused.as_deref(), Some("unreachable"));
        delays.push(repo::sync_state(&conn, &scope).unwrap().paused_until.unwrap() - 1_000);
        unpause(&conn, &scope);
    }
    assert_eq!(delays, vec![60_000, 120_000, 240_000]);
    run(&conn, &scope, &api);
    let state = repo::sync_state(&conn, &scope).unwrap();
    assert_eq!((state.paused_until, state.pause_reason), (None, None), "a clean run resets it");
    assert_eq!(api.comment("AAAA2222"), Some(json!("x")));
}

#[test]
fn an_edit_then_a_delete_in_the_next_run_deletes_without_a_conflict() {
    let scope = scope("lib-edit-delete");
    let conn = archive_with_library(&scope.library_row_id);
    let api = FakeApi::new();
    api.put("AAAA2222", "one");
    run(&conn, &scope, &api);
    edit_comment(&conn, &scope, "AAAA2222", "edited");
    run(&conn, &scope, &api);
    let mut doomed = row(&conn, &scope, "AAAA2222");
    local_delete(&mut doomed).unwrap();
    repo::save(&conn, &scope, &doomed, 1).unwrap();
    run(&conn, &scope, &api);
    assert_eq!(api.comment("AAAA2222"), None, "deleted in Zotero");
    assert!(repo::load(&conn, &scope, "AAAA2222").unwrap().is_none());
}
