//! Running the queue: one copy end to end, and the drain over what waits.
//!
//! One run, in this order, and it stops at the first thing that is not right:
//!
//! 1. claim the row, read the source (and the PDF, hashed again) from our rows;
//! 2. ping Zotero. No answer is not a failure: the row goes to `waiting`;
//! 3. resolve the library the person chose to Zotero's own id for it;
//! 4. look the item up by address in that library. Found: it is linked, never
//!    written again (`linked`), and what would differ is reported ([`plan`]);
//! 5. not found: save it, move the session to the library, attach the PDF in the
//!    same session, then read it back for its key. A write that cannot be read
//!    back fails `readback_miss`, and the retry finds it by address and links it,
//!    so nothing is duplicated.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

use super::plan::{
    attachment_title, lookup_urls, matching_items, owned_from_zotero, plan_merge, webpage_item,
    ExistingItem, Owned, SourceFacts,
};
use super::port::{PortError, Targets, ZoteroPort};
use super::store::{self, ZoteroCopy};
use crate::navegador::sources;
use crate::writing::zotero::{Library, LibraryType};

/// The largest PDF sent to Zotero.
pub const MAX_PDF_BYTES: u64 = 50 * 1024 * 1024;
/// The id the item has inside its save session.
const CONNECTOR_ID: &str = "entropia-web-1";

pub struct RunOptions {
    /// How many times the new item is looked for after it was written.
    pub readback_tries: u32,
    pub readback_delay_ms: u64,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            readback_tries: 5,
            readback_delay_ms: 1500,
        }
    }
}

/// What a drain did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrainReport {
    /// `false` only when a probe found Zotero not answering.
    pub reachable: bool,
    /// The rows the drain went through, in order, as they are now.
    pub copies: Vec<ZoteroCopy>,
}

enum Stop {
    Wait,
    Fail { code: String, message: String },
}

struct Done {
    state: &'static str,
    item_key: String,
    detail: Value,
}

fn fail(code: &str, message: impl Into<String>) -> Stop {
    Stop::Fail {
        code: code.to_string(),
        message: message.into(),
    }
}

/// A refusal from a read (local API). `403` is the API switched off.
fn read_error(error: PortError) -> Stop {
    match error {
        PortError::Unreachable => Stop::Wait,
        PortError::Rejected { status: 403, .. } => {
            fail("zotero_api_disabled", "Zotero's local API is turned off")
        }
        PortError::Rejected { status, detail } => fail(
            "zotero_rejected",
            format!("Zotero answered {status}: {detail}"),
        ),
        PortError::Invalid(detail) => fail("zotero_rejected", detail),
    }
}

fn write_error(error: PortError) -> Stop {
    match error {
        PortError::Unreachable => Stop::Wait,
        PortError::Rejected { status, detail } => fail(
            "zotero_rejected",
            format!("Zotero answered {status}: {detail}"),
        ),
        PortError::Invalid(detail) => fail("zotero_rejected", detail),
    }
}

/// `code: detail` strings of the sources module become a code and a message.
fn from_source_error(error: String) -> Stop {
    match error.split_once(": ") {
        Some((code, message)) => fail(code, message),
        None => fail("db_error", error),
    }
}

struct Pdf {
    sha256: String,
    url: String,
    bytes: Vec<u8>,
}

fn load(
    conn: &Connection,
    data_dir: &Path,
    row: &ZoteroCopy,
) -> Result<(SourceFacts, Option<Pdf>), Stop> {
    let source = conn
        .query_row(
            "SELECT s.title, s.final_url, s.canonical_url, s.site_name,
                    COALESCE((SELECT MAX(accessed_at) FROM web_captures WHERE web_source_id = s.id),
                             s.first_accessed_at)
             FROM web_sources s WHERE s.id = ?1",
            [&row.source_id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| fail("db_error", error.to_string()))?;
    let Some((title, final_url, canonical_url, site_name, accessed_at)) = source else {
        return Err(fail("not_found", "the source no longer exists"));
    };
    let mut facts = SourceFacts {
        source_id: row.source_id.clone(),
        title,
        final_url,
        canonical_url,
        site_name,
        accessed_at,
    };
    let Some(capture_id) = row.capture_id.as_deref() else {
        return Ok((facts, None));
    };

    let ticket = sources::copy_ticket(conn, data_dir, capture_id).map_err(from_source_error)?;
    if ticket.provenance.rendering.is_some() {
        return Err(fail("not_a_pdf", "only a saved PDF goes along"));
    }
    let (capture_url, accessed): (String, String) = conn
        .query_row(
            "SELECT final_url, accessed_at FROM web_captures WHERE id = ?1",
            [capture_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|error| fail("db_error", error.to_string()))?;
    let length = std::fs::metadata(&ticket.path)
        .map_err(|_| fail("file_missing", "the saved PDF is not on disk"))?
        .len();
    if length > MAX_PDF_BYTES {
        return Err(fail("file_too_large", "the PDF is too large for Zotero"));
    }
    let bytes = std::fs::read(&ticket.path)
        .map_err(|_| fail("file_missing", "the saved PDF is not on disk"))?;
    facts.accessed_at = accessed;
    Ok((
        facts,
        Some(Pdf {
            sha256: ticket.provenance.sha256,
            url: capture_url,
            bytes,
        }),
    ))
}

fn library_of(row: &ZoteroCopy) -> Result<Library, Stop> {
    let kind = if row.library_type == "group" {
        LibraryType::Group
    } else {
        LibraryType::User
    };
    Library::new(kind, row.library_id.clone()).map_err(|message| fail("invalid_library", message))
}

/// Zotero's own id (`L<libraryID>`) for the library the person chose.
///
/// The personal library is always libraryID 1 in Zotero. A group is matched by
/// its name among the editable libraries the connector lists: two with the same
/// name are never guessed between.
fn resolve_target(
    port: &dyn ZoteroPort,
    row: &ZoteroCopy,
    targets: &Targets,
) -> Result<String, Stop> {
    let unavailable = || {
        fail(
            "library_unavailable",
            "the library is not an editable library of this Zotero",
        )
    };
    if row.library_type != "group" {
        return targets
            .libraries
            .iter()
            .any(|library| library.id == "L1")
            .then(|| "L1".to_string())
            .ok_or_else(unavailable);
    }
    let name = port
        .group_name(&row.library_id)
        .map_err(read_error)?
        .ok_or_else(unavailable)?;
    let mut matches = targets
        .libraries
        .iter()
        .filter(|library| library.id != "L1" && library.name == name);
    match (matches.next(), matches.next()) {
        (Some(only), None) => Ok(only.id.clone()),
        (None, _) => Err(unavailable()),
        _ => Err(fail(
            "ambiguous_library",
            "two libraries in Zotero have that name",
        )),
    }
}

fn find_existing(
    port: &dyn ZoteroPort,
    library: &Library,
    urls: &[String],
) -> Result<Vec<ExistingItem>, Stop> {
    let mut hits: Vec<Value> = Vec::new();
    for url in urls {
        for hit in port.find_items(library, url).map_err(read_error)? {
            let key = hit.get("key").and_then(Value::as_str).unwrap_or_default();
            if !hits
                .iter()
                .any(|seen| seen.get("key").and_then(Value::as_str) == Some(key))
            {
                hits.push(hit);
            }
        }
    }
    Ok(matching_items(&hits, urls))
}

/// What a copy of the same source wrote earlier in this library, if any.
fn previous_written(conn: &Connection, row: &ZoteroCopy) -> Owned {
    let json: Option<String> = conn
        .query_row(
            "SELECT detail_json FROM navegador_zotero_copies
             WHERE source_id = ?1 AND library_type = ?2 AND library_id = ?3
               AND state = 'copied' AND detail_json IS NOT NULL
             ORDER BY created_at LIMIT 1",
            rusqlite::params![row.source_id, row.library_type, row.library_id],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    json.and_then(|json| serde_json::from_str::<Value>(&json).ok())
        .and_then(|detail| serde_json::from_value(detail.get("written")?.clone()).ok())
        .unwrap_or_default()
}

fn names(fields: &[&'static str]) -> Value {
    json!(fields)
}

fn attempt(
    conn: &Connection,
    data_dir: &Path,
    port: &dyn ZoteroPort,
    row: &ZoteroCopy,
    options: &RunOptions,
) -> Result<Done, Stop> {
    let (facts, pdf) = load(conn, data_dir, row)?;
    let library = library_of(row)?;

    port.ping().map_err(|_| Stop::Wait)?;
    let targets = port.targets().map_err(read_error)?;
    let target = resolve_target(port, row, &targets)?;

    let urls = lookup_urls(&facts);
    let item = webpage_item(&facts, CONNECTOR_ID);
    let ours = owned_from_zotero(&item);

    if let Some(existing) = find_existing(port, &library, &urls)?.into_iter().next() {
        let merge = plan_merge(&ours, &existing.owned, &previous_written(conn, row));
        let pdf_note = match &pdf {
            None => "none",
            Some(pdf) => {
                let title = attachment_title(&pdf.sha256);
                let children = port.children(&library, &existing.key).map_err(read_error)?;
                let there = children.iter().any(|child| {
                    child.pointer("/data/title").and_then(Value::as_str) == Some(title.as_str())
                });
                if there {
                    "already_there"
                } else {
                    "parent_exists"
                }
            }
        };
        let pending: Vec<&'static str> = merge.fill.iter().chain(&merge.update).copied().collect();
        return Ok(Done {
            state: store::STATE_LINKED,
            item_key: existing.key,
            detail: json!({
                "existing": true,
                "pdf": pdf_note,
                "pendingFields": names(&pending),
                "keptFields": names(&merge.kept),
            }),
        });
    }

    let session = uuid::Uuid::new_v4().to_string();
    port.save_item(&session, &facts.final_url, item)
        .map_err(write_error)?;
    // Always move: the item lands where the Zotero window happens to be, and the
    // move puts it at the root of the library the person chose.
    port.move_session(&session, &target).map_err(write_error)?;
    let pdf_note = match &pdf {
        None => "none",
        Some(pdf) => {
            let attached = port
                .attach_pdf(
                    &session,
                    CONNECTOR_ID,
                    &attachment_title(&pdf.sha256),
                    &pdf.url,
                    pdf.bytes.clone(),
                )
                .map_err(write_error)?;
            if attached {
                "attached"
            } else {
                "not_attached"
            }
        }
    };

    for tries in 0..options.readback_tries.max(1) {
        if tries > 0 && options.readback_delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(options.readback_delay_ms));
        }
        if let Some(found) = find_existing(port, &library, &urls)?.into_iter().next() {
            return Ok(Done {
                state: store::STATE_COPIED,
                item_key: found.key,
                detail: json!({
                    "existing": false,
                    "pdf": pdf_note,
                    "pendingFields": [],
                    "keptFields": [],
                    "written": ours,
                }),
            });
        }
    }
    Err(fail(
        "readback_miss",
        "Zotero accepted the item but does not return it yet",
    ))
}

/// Runs one copy end to end and returns the row as it ended.
pub fn run_one(
    conn: &Connection,
    data_dir: &Path,
    port: &dyn ZoteroPort,
    id: &str,
    options: &RunOptions,
) -> Result<ZoteroCopy, String> {
    let row = store::claim(conn, id)?;
    match attempt(conn, data_dir, port, &row, options) {
        Ok(done) => store::finish(
            conn,
            id,
            done.state,
            Some(done.item_key.as_str()),
            &done.detail.to_string(),
        ),
        Err(Stop::Wait) => store::wait(conn, id),
        Err(Stop::Fail { code, message }) => store::fail(conn, id, &code, &message),
    }
}

/// Works through the queued and waiting copies, oldest first. Zotero not
/// answering stops it at once and parks the rest as waiting: one probe, not one
/// per row.
pub fn drain(
    conn: &Connection,
    data_dir: &Path,
    port: &dyn ZoteroPort,
    options: &RunOptions,
) -> Result<DrainReport, String> {
    store::recover(conn)?;
    let rows = store::pending(conn)?;
    let mut report = DrainReport {
        reachable: true,
        copies: Vec::new(),
    };
    let mut rows = rows.into_iter();
    while let Some(row) = rows.next() {
        let done = match run_one(conn, data_dir, port, &row.id, options) {
            Ok(done) => done,
            // Cancelled or taken between listing and claiming: not ours any more.
            Err(error) if error.starts_with("invalid_transition") => continue,
            Err(error) => return Err(error),
        };
        let closed = done.state == store::STATE_WAITING;
        report.copies.push(done);
        if closed {
            report.reachable = false;
            for rest in rows.by_ref() {
                if rest.state == store::STATE_QUEUED {
                    report.copies.push(store::wait(conn, &rest.id)?);
                } else {
                    report.copies.push(rest);
                }
            }
            break;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::super::port::{PortError, TargetLibrary, Targets, ZoteroPort};
    use super::super::store::{self, LibraryRef};
    use super::*;
    use crate::sync::test_support::new_app_schema_db;
    use crate::writing::zotero::Library;
    use rusqlite::Connection;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};

    /// session, parent, title, url, bytes
    type Attached = (String, String, String, String, Vec<u8>);

    /// A scripted Zotero. Every call is logged in order.
    struct FakePort {
        calls: RefCell<Vec<String>>,
        ping: Result<(), PortError>,
        targets: Result<Targets, PortError>,
        groups: HashMap<String, Option<String>>,
        /// Answers of successive `find_items` calls; the last one repeats.
        finds: RefCell<VecDeque<Result<Vec<Value>, PortError>>>,
        children: Vec<Value>,
        save: Result<(), PortError>,
        attach: Result<bool, PortError>,
        saved: RefCell<Vec<(String, String, Value)>>,
        moved: RefCell<Vec<(String, String)>>,
        attached: RefCell<Vec<Attached>>,
        searched: RefCell<Vec<String>>,
    }

    fn two_libraries() -> Targets {
        Targets {
            selected_library: "L1".into(),
            libraries: vec![
                TargetLibrary {
                    id: "L1".into(),
                    name: "My Library".into(),
                },
                TargetLibrary {
                    id: "L3".into(),
                    name: "prueba".into(),
                },
            ],
        }
    }

    impl FakePort {
        fn open() -> Self {
            Self {
                calls: RefCell::new(vec![]),
                ping: Ok(()),
                targets: Ok(two_libraries()),
                groups: HashMap::from([("6680944".to_string(), Some("prueba".to_string()))]),
                finds: RefCell::new(VecDeque::from([Ok(vec![])])),
                children: vec![],
                save: Ok(()),
                attach: Ok(true),
                saved: RefCell::new(vec![]),
                moved: RefCell::new(vec![]),
                attached: RefCell::new(vec![]),
                searched: RefCell::new(vec![]),
            }
        }

        /// No match before the write, the new item after it.
        fn creating(self, key: &str) -> Self {
            *self.finds.borrow_mut() = VecDeque::from([Ok(vec![]), Ok(vec![hit(key, "webpage")])]);
            self
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    fn hit(key: &str, kind: &str) -> Value {
        json!({"key": key, "version": 4, "data": {
            "key": key, "itemType": kind, "title": "A title",
            "url": "https://a.test/x", "dateAdded": "2026-10-01T10:00:01Z"}})
    }

    impl ZoteroPort for FakePort {
        fn ping(&self) -> Result<(), PortError> {
            self.calls.borrow_mut().push("ping".into());
            self.ping.clone()
        }
        fn targets(&self) -> Result<Targets, PortError> {
            self.calls.borrow_mut().push("targets".into());
            self.targets.clone()
        }
        fn group_name(&self, group_id: &str) -> Result<Option<String>, PortError> {
            self.calls
                .borrow_mut()
                .push(format!("group_name {group_id}"));
            Ok(self.groups.get(group_id).cloned().flatten())
        }
        fn find_items(&self, library: &Library, url: &str) -> Result<Vec<Value>, PortError> {
            self.calls
                .borrow_mut()
                .push(format!("find {}", library.storage_key()));
            self.searched.borrow_mut().push(url.to_string());
            let mut finds = self.finds.borrow_mut();
            if finds.len() > 1 {
                finds.pop_front().unwrap()
            } else {
                finds.front().cloned().unwrap()
            }
        }
        fn children(&self, _: &Library, key: &str) -> Result<Vec<Value>, PortError> {
            self.calls.borrow_mut().push(format!("children {key}"));
            Ok(self.children.clone())
        }
        fn save_item(&self, session: &str, uri: &str, item: Value) -> Result<(), PortError> {
            self.calls.borrow_mut().push("save".into());
            self.saved
                .borrow_mut()
                .push((session.into(), uri.into(), item));
            self.save.clone()
        }
        fn move_session(&self, session: &str, target: &str) -> Result<(), PortError> {
            self.calls.borrow_mut().push(format!("move {target}"));
            self.moved
                .borrow_mut()
                .push((session.into(), target.into()));
            Ok(())
        }
        fn attach_pdf(
            &self,
            session: &str,
            parent: &str,
            title: &str,
            url: &str,
            bytes: Vec<u8>,
        ) -> Result<bool, PortError> {
            self.calls.borrow_mut().push("attach".into());
            self.attached.borrow_mut().push((
                session.into(),
                parent.into(),
                title.into(),
                url.into(),
                bytes,
            ));
            self.attach.clone()
        }
    }

    struct Env {
        data: tempfile::TempDir,
        conn: Connection,
    }

    const PDF: &[u8] = b"%PDF-1.4 fake";

    fn env() -> Env {
        let data = tempfile::tempdir().unwrap();
        let conn = new_app_schema_db();
        conn.execute(
            "INSERT INTO web_sources (id, original_url, final_url, canonical_url, title, site_name, first_accessed_at, created_at, updated_at)
             VALUES ('src1', 'https://a.test/x', 'https://a.test/x', NULL, 'A title', 'A site', '2026-10-01T10:00:00Z', 1, 1)",
            [],
        )
        .unwrap();
        let dir = data.path().join("web-captures").join("src1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cap-pdf.pdf"), PDF).unwrap();
        let sha = format!("{:x}", Sha256::digest(PDF));
        conn.execute(
            "INSERT INTO web_captures (id, web_source_id, accessed_at, final_url, kind, mime_type, rel_path, sha256, hash_of, size_bytes, title, created_at)
             VALUES ('cap-pdf', 'src1', '2026-10-02T09:30:00Z', 'https://a.test/x.pdf', 'pdf', 'application/pdf', 'web-captures/src1/cap-pdf.pdf', ?1, 'pdf', ?2, NULL, 1)",
            rusqlite::params![sha, PDF.len() as i64],
        )
        .unwrap();
        Env { data, conn }
    }

    fn personal() -> LibraryRef {
        LibraryRef {
            library_type: "user".into(),
            library_id: "0".into(),
            library_name: None,
        }
    }

    fn group() -> LibraryRef {
        LibraryRef {
            library_type: "group".into(),
            library_id: "6680944".into(),
            library_name: Some("prueba".into()),
        }
    }

    fn opts() -> RunOptions {
        RunOptions {
            readback_tries: 3,
            readback_delay_ms: 0,
        }
    }

    fn go(env: &Env, port: &FakePort, id: &str) -> store::ZoteroCopy {
        run_one(&env.conn, env.data.path(), port, id, &opts()).unwrap()
    }

    #[test]
    fn a_closed_zotero_leaves_the_copy_waiting_without_writing() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let mut port = FakePort::open();
        port.ping = Err(PortError::Unreachable);
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_WAITING);
        assert_eq!(port.calls(), vec!["ping"]);
        assert_eq!(done.error_code, None, "waiting is not a failure");
    }

    #[test]
    fn a_page_is_created_in_the_personal_library_and_read_back_for_its_key() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open().creating("NEWKEY22");
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_COPIED);
        assert_eq!(done.item_key.as_deref(), Some("NEWKEY22"));
        assert_eq!(
            port.calls(),
            vec!["ping", "targets", "find 0", "save", "move L1", "find 0"]
        );
        let (session, uri, item) = port.saved.borrow()[0].clone();
        assert_eq!(uri, "https://a.test/x");
        assert_eq!(item["itemType"], "webpage");
        assert_eq!(item["title"], "A title");
        assert_eq!(item["websiteTitle"], "A site");
        assert_eq!(port.moved.borrow()[0], (session, "L1".to_string()));
        let detail = done.detail.unwrap();
        assert_eq!(detail["existing"], false);
        assert_eq!(detail["pdf"], "none");
    }

    #[test]
    fn a_group_is_found_by_name_among_the_editable_libraries() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &group()).unwrap();
        let port = FakePort::open().creating("GRPKEY22");
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_COPIED);
        assert!(port.calls().contains(&"group_name 6680944".to_string()));
        assert_eq!(port.moved.borrow()[0].1, "L3");
    }

    #[test]
    fn a_group_zotero_cannot_write_to_fails_with_a_code_the_ui_explains() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &group()).unwrap();
        let mut port = FakePort::open();
        port.targets = Ok(Targets {
            selected_library: "L1".into(),
            libraries: vec![TargetLibrary {
                id: "L1".into(),
                name: "My Library".into(),
            }],
        });
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_FAILED);
        assert_eq!(done.error_code.as_deref(), Some("library_unavailable"));
        assert!(!port.calls().contains(&"save".to_string()));
    }

    #[test]
    fn two_libraries_with_the_same_name_are_never_guessed() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &group()).unwrap();
        let mut port = FakePort::open();
        port.targets = Ok(Targets {
            selected_library: "L1".into(),
            libraries: vec![
                TargetLibrary {
                    id: "L1".into(),
                    name: "My Library".into(),
                },
                TargetLibrary {
                    id: "L3".into(),
                    name: "prueba".into(),
                },
                TargetLibrary {
                    id: "L4".into(),
                    name: "prueba".into(),
                },
            ],
        });
        let done = go(&env, &port, &row.id);
        assert_eq!(done.error_code.as_deref(), Some("ambiguous_library"));
        assert!(!port.calls().contains(&"save".to_string()));
    }

    #[test]
    fn a_disabled_local_api_is_reported_as_such() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open();
        *port.finds.borrow_mut() = VecDeque::from([Err(PortError::Rejected {
            status: 403,
            detail: String::new(),
        })]);
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_FAILED);
        assert_eq!(done.error_code.as_deref(), Some("zotero_api_disabled"));
        assert!(!port.calls().contains(&"save".to_string()));
    }

    #[test]
    fn an_item_already_there_is_linked_and_never_written_again() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open();
        *port.finds.borrow_mut() = VecDeque::from([Ok(vec![hit("OLDKEY22", "webpage")])]);
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_LINKED);
        assert_eq!(done.item_key.as_deref(), Some("OLDKEY22"));
        assert!(port.saved.borrow().is_empty());
        assert!(port.moved.borrow().is_empty());
        let detail = done.detail.unwrap();
        assert_eq!(detail["existing"], true);
    }

    #[test]
    fn what_differs_in_an_existing_item_is_reported_and_never_overwritten() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open();
        let mut existing = hit("OLDKEY22", "webpage");
        existing["data"]["title"] = json!("My own title");
        *port.finds.borrow_mut() = VecDeque::from([Ok(vec![existing])]);
        let done = go(&env, &port, &row.id);
        let detail = done.detail.unwrap();
        assert_eq!(detail["keptFields"], json!(["title"]));
        assert_eq!(
            detail["pendingFields"],
            json!(["accessDate", "websiteTitle"])
        );
        assert!(port.saved.borrow().is_empty());
    }

    #[test]
    fn a_pdf_rides_along_in_the_same_session_as_a_child() {
        let env = env();
        let row = store::request(&env.conn, "src1", Some("cap-pdf"), &personal()).unwrap();
        let port = FakePort::open().creating("PDFKEY22");
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_COPIED);
        let (session, parent, title, url, bytes) = port.attached.borrow()[0].clone();
        assert_eq!(session, port.saved.borrow()[0].0);
        assert_eq!(parent, port.saved.borrow()[0].2["id"].as_str().unwrap());
        assert!(title.starts_with("Captura web ") && title.ends_with(".pdf"));
        assert_eq!(url, "https://a.test/x.pdf");
        assert_eq!(bytes, PDF);
        // The order matters: the item is in the right library before the file goes.
        let calls = port.calls();
        let at = |name: &str| calls.iter().position(|c| c.starts_with(name)).unwrap();
        assert!(at("save") < at("move") && at("move") < at("attach"));
        assert_eq!(done.detail.unwrap()["pdf"], "attached");
        // The item records when the PDF was saved, not when the source was first seen.
        assert_eq!(
            port.saved.borrow()[0].2["accessDate"],
            "2026-10-02T09:30:00Z"
        );
    }

    #[test]
    fn a_library_without_file_storage_keeps_the_item_and_says_the_pdf_is_missing() {
        let env = env();
        let row = store::request(&env.conn, "src1", Some("cap-pdf"), &personal()).unwrap();
        let mut port = FakePort::open().creating("PDFKEY22");
        port.attach = Ok(false);
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_COPIED);
        assert_eq!(done.detail.unwrap()["pdf"], "not_attached");
    }

    #[test]
    fn a_pdf_for_an_item_that_already_exists_is_not_forced_in() {
        let env = env();
        let row = store::request(&env.conn, "src1", Some("cap-pdf"), &personal()).unwrap();
        let port = FakePort::open();
        *port.finds.borrow_mut() = VecDeque::from([Ok(vec![hit("OLDKEY22", "webpage")])]);
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_LINKED);
        assert!(port.attached.borrow().is_empty());
        assert_eq!(done.detail.unwrap()["pdf"], "parent_exists");
    }

    #[test]
    fn a_pdf_already_under_the_existing_item_is_recognised() {
        let env = env();
        let row = store::request(&env.conn, "src1", Some("cap-pdf"), &personal()).unwrap();
        let mut port = FakePort::open();
        let sha = format!("{:x}", Sha256::digest(PDF));
        port.children = vec![json!({"data": {"itemType": "attachment",
            "title": format!("Captura web {}.pdf", &sha[..8])}})];
        *port.finds.borrow_mut() = VecDeque::from([Ok(vec![hit("OLDKEY22", "webpage")])]);
        let done = go(&env, &port, &row.id);
        assert_eq!(done.detail.unwrap()["pdf"], "already_there");
    }

    #[test]
    fn a_changed_pdf_file_is_not_sent() {
        let env = env();
        let row = store::request(&env.conn, "src1", Some("cap-pdf"), &personal()).unwrap();
        std::fs::write(
            env.data.path().join("web-captures/src1/cap-pdf.pdf"),
            b"tampered",
        )
        .unwrap();
        let port = FakePort::open().creating("K");
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_FAILED);
        assert_eq!(done.error_code.as_deref(), Some("file_changed"));
        assert!(!port.calls().contains(&"save".to_string()));
    }

    #[test]
    fn a_source_deleted_while_queued_fails_instead_of_copying() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        env.conn.execute("DELETE FROM web_sources", []).unwrap();
        let port = FakePort::open();
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_FAILED);
        assert_eq!(done.error_code.as_deref(), Some("not_found"));
    }

    #[test]
    fn zotero_dying_mid_write_sends_the_copy_back_to_waiting() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let mut port = FakePort::open();
        port.save = Err(PortError::Unreachable);
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_WAITING);
    }

    #[test]
    fn a_rejected_write_fails_with_what_zotero_said() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let mut port = FakePort::open();
        port.save = Err(PortError::Rejected {
            status: 500,
            detail: "boom".into(),
        });
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_FAILED);
        assert_eq!(done.error_code.as_deref(), Some("zotero_rejected"));
        assert!(done.error_message.unwrap().contains("500"));
    }

    #[test]
    fn a_write_that_cannot_be_read_back_fails_but_a_retry_links_instead_of_duplicating() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open(); // never finds it
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_FAILED);
        assert_eq!(done.error_code.as_deref(), Some("readback_miss"));
        assert_eq!(port.saved.borrow().len(), 1);

        // The write did land: the retry finds it by address and links it.
        let retry = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let again = FakePort::open();
        *again.finds.borrow_mut() = VecDeque::from([Ok(vec![hit("LANDED22", "webpage")])]);
        let done = go(&env, &again, &retry.id);
        assert_eq!(done.state, store::STATE_LINKED);
        assert!(again.saved.borrow().is_empty());
    }

    #[test]
    fn the_drain_works_oldest_first_and_reports_what_it_did() {
        let env = env();
        let a = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let b = store::request(&env.conn, "src1", None, &group()).unwrap();
        env.conn
            .execute(
                "UPDATE navegador_zotero_copies SET created_at = 1 WHERE id = ?1",
                [&b.id],
            )
            .unwrap();
        env.conn
            .execute(
                "UPDATE navegador_zotero_copies SET created_at = 2 WHERE id = ?1",
                [&a.id],
            )
            .unwrap();
        let port = FakePort::open().creating("K1111111");
        let report = drain(&env.conn, env.data.path(), &port, &opts()).unwrap();
        assert!(report.reachable);
        let ids: Vec<&str> = report.copies.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec![b.id.as_str(), a.id.as_str()]);
    }

    #[test]
    fn the_drain_stops_at_the_first_closed_zotero_and_parks_the_rest() {
        let env = env();
        let a = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let b = store::request(&env.conn, "src1", None, &group()).unwrap();
        let mut port = FakePort::open();
        port.ping = Err(PortError::Unreachable);
        let report = drain(&env.conn, env.data.path(), &port, &opts()).unwrap();
        assert!(!report.reachable);
        assert_eq!(port.calls(), vec!["ping"], "one probe, not one per row");
        for id in [&a.id, &b.id] {
            assert_eq!(
                store::get(&env.conn, id).unwrap().unwrap().state,
                store::STATE_WAITING
            );
        }
    }

    #[test]
    fn a_drain_with_nothing_queued_does_not_even_probe_zotero() {
        let env = env();
        let port = FakePort::open();
        let report = drain(&env.conn, env.data.path(), &port, &opts()).unwrap();
        assert!(report.copies.is_empty());
        assert!(port.calls().is_empty());
    }

    #[test]
    fn a_drain_recovers_rows_a_crash_left_running() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        store::claim(&env.conn, &row.id).unwrap();
        let port = FakePort::open().creating("K2222222");
        let report = drain(&env.conn, env.data.path(), &port, &opts()).unwrap();
        assert_eq!(report.copies[0].state, store::STATE_COPIED);
    }

    #[test]
    fn a_cancelled_copy_is_not_run() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        store::cancel(&env.conn, &row.id).unwrap();
        let port = FakePort::open();
        let report = drain(&env.conn, env.data.path(), &port, &opts()).unwrap();
        assert!(report.copies.is_empty());
    }

    #[test]
    fn the_address_searched_is_the_final_one() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open().creating("K");
        go(&env, &port, &row.id);
        assert_eq!(port.searched.borrow()[0], "https://a.test/x");
    }
}
