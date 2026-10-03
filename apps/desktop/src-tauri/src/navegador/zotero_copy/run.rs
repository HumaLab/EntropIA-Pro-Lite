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
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

use super::plan::{
    attachment_title, lookup_urls, matching_items, owned_from_zotero, plan_merge, webpage_item,
    ExistingItem, Owned, SourceFacts,
};
use super::port::{PortError, Targets, ZoteroPort};
use super::store::{self, ZoteroCopy};
use super::web::{
    attach_pdf_via_web, complete_item, md5_hex, Completion, Failure, PdfFile, PdfOutcome, WebError,
    WebLibrary, WebPort,
};
use crate::navegador::sources;
use crate::settings::{get_setting, ZOTERO_API_KEY};
use crate::writing::zotero::{Library, LibraryType};
use crate::zotero_web::{stored_credentials, KeyCheck};

/// The largest PDF sent to Zotero.
pub const MAX_PDF_BYTES: u64 = 50 * 1024 * 1024;
/// The id the item has inside its save session.
const CONNECTOR_ID: &str = "entropia-web-1";

pub struct RunOptions {
    /// How many times the new item is looked for after it was written.
    pub readback_tries: u32,
    pub readback_delay_ms: u64,
    /// The Web API, when this run may use it to complete an item that already
    /// exists. `None` keeps the connector-only behaviour.
    pub web: Option<Arc<dyn WebPort>>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            readback_tries: 5,
            readback_delay_ms: 1500,
            web: None,
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

/// The source as Zotero would see it. With a capture, the item records when that
/// PDF was saved rather than the latest access.
fn load_source(
    conn: &Connection,
    source_id: &str,
    capture_id: Option<&str>,
) -> Result<SourceFacts, Stop> {
    let source = conn
        .query_row(
            "SELECT s.title, s.final_url, s.canonical_url, s.site_name,
                    COALESCE((SELECT MAX(accessed_at) FROM web_captures WHERE web_source_id = s.id),
                             s.first_accessed_at)
             FROM web_sources s WHERE s.id = ?1",
            [source_id],
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
    let Some((title, final_url, canonical_url, site_name, mut accessed_at)) = source else {
        return Err(fail("not_found", "the source no longer exists"));
    };
    if let Some(capture) = capture_id {
        accessed_at = conn
            .query_row(
                "SELECT accessed_at FROM web_captures WHERE id = ?1 AND web_source_id = ?2",
                rusqlite::params![capture, source_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|error| fail("db_error", error.to_string()))?
            .ok_or_else(|| fail("not_found", "there is no such capture"))?;
    }
    Ok(SourceFacts {
        source_id: source_id.to_string(),
        title,
        final_url,
        canonical_url,
        site_name,
        accessed_at,
    })
}

/// Reads one saved PDF capture: its ticket (hashed again, so a file that is not
/// the one that was saved is refused), then its bytes.
fn read_pdf(conn: &Connection, data_dir: &Path, capture_id: &str) -> Result<Pdf, Stop> {
    let ticket = sources::copy_ticket(conn, data_dir, capture_id).map_err(from_source_error)?;
    if ticket.provenance.rendering.is_some() {
        return Err(fail("not_a_pdf", "only a saved PDF goes along"));
    }
    let capture_url: String = conn
        .query_row(
            "SELECT final_url FROM web_captures WHERE id = ?1",
            [capture_id],
            |r| r.get(0),
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
    Ok(Pdf {
        sha256: ticket.provenance.sha256,
        url: capture_url,
        bytes,
    })
}

/// The PDF a copy of the source itself takes along: its latest saved PDF capture
/// that is still on disk, still matches its hash and fits. One that does not is
/// skipped for the one before it: a page copy never fails for a PDF nobody asked
/// for by name. Returns the capture's id and the time it was saved too.
fn latest_pdf(
    conn: &Connection,
    data_dir: &Path,
    source_id: &str,
) -> Option<(String, String, Pdf)> {
    let mut statement = conn
        .prepare(
            "SELECT id, accessed_at FROM web_captures
             WHERE web_source_id = ?1 AND kind = 'pdf'
             ORDER BY accessed_at DESC, created_at DESC, id DESC",
        )
        .ok()?;
    let captures: Vec<(String, String)> = statement
        .query_map([source_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .ok()?
        .filter_map(Result::ok)
        .collect();
    captures.into_iter().find_map(|(id, saved_at)| {
        let pdf = read_pdf(conn, data_dir, &id).ok()?;
        Some((id, saved_at, pdf))
    })
}

fn load(
    conn: &Connection,
    data_dir: &Path,
    row: &ZoteroCopy,
) -> Result<(SourceFacts, Option<Pdf>), Stop> {
    let facts = load_source(conn, &row.source_id, row.capture_id.as_deref())?;
    // A PDF capture named by the person must be right or the copy fails; for the
    // source itself the latest saved PDF goes along when there is one.
    let pdf = match row.capture_id.as_deref() {
        Some(capture) => Some(read_pdf(conn, data_dir, capture)?),
        None => latest_pdf(conn, data_dir, &row.source_id).map(|(_, _, pdf)| pdf),
    };
    Ok((facts, pdf))
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
fn previous_written(
    conn: &Connection,
    source_id: &str,
    library_type: &str,
    library_id: &str,
) -> Owned {
    let json: Option<String> = conn
        .query_row(
            "SELECT detail_json FROM navegador_zotero_copies
             WHERE source_id = ?1 AND library_type = ?2 AND library_id = ?3
               AND state = 'copied' AND detail_json IS NOT NULL
             ORDER BY created_at LIMIT 1",
            rusqlite::params![source_id, library_type, library_id],
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

/// What the Web API did for an existing item.
struct WebOutcome {
    /// `completed`, `nothing_missing`, `conflict`, `no_key`, `invalid_key`,
    /// `no_write`, `other_account`, `account_unknown`, `not_synced_yet` (the item
    /// is only in the local Zotero so far) or `failed`.
    state: &'static str,
    completed: Vec<&'static str>,
    /// What became of the PDF: `attached`, `already_there`, `quota` or `failed`.
    /// `None` when there was none to attach or the access was not granted.
    pdf: Option<&'static str>,
    /// Where a `failed` state stopped (`patch:500`), key-free.
    reason: Option<String>,
    /// Where a failed PDF stopped (`upload:500`).
    pdf_reason: Option<String>,
}

impl WebOutcome {
    fn of(state: &'static str) -> Self {
        Self {
            state,
            completed: Vec::new(),
            pdf: None,
            reason: None,
            pdf_reason: None,
        }
    }

    fn failed(failure: &Failure) -> Self {
        Self {
            reason: Some(failure.code()),
            ..Self::of("failed")
        }
    }
}

/// What a copy knows about the item that is already in Zotero.
#[derive(Clone, Copy)]
struct ExistingCopy<'a> {
    row: &'a ZoteroCopy,
    existing: &'a ExistingItem,
    ours: &'a Owned,
    pdf: Option<&'a Pdf>,
    /// What the local API shows of the PDF (`already_there`, `parent_exists`...).
    pdf_note: &'a str,
}

/// Fills in the fields an existing item lacks, when a stored key is still valid
/// and may write to that library. Never fails the copy: whatever goes wrong, the
/// item stays linked and the outcome says what happened.
fn complete_through_web(
    conn: &Connection,
    port: &dyn ZoteroPort,
    web: &dyn WebPort,
    job: &ExistingCopy,
) -> WebOutcome {
    let ExistingCopy {
        row,
        existing,
        ours,
        pdf,
        pdf_note,
    } = *job;
    let Some(credentials) = stored_credentials(conn) else {
        return WebOutcome::of("no_key");
    };
    let info = match web.key_info(&credentials.key) {
        Ok(KeyCheck::Valid(info)) => info,
        Ok(KeyCheck::InvalidKey) => return WebOutcome::of("invalid_key"),
        Ok(KeyCheck::Unreachable) => {
            return WebOutcome::failed(&Failure::of("key_info", &WebError::Unreachable))
        }
        Err(error) => return WebOutcome::failed(&Failure::of("key_info", &error)),
    };
    // The personal library is the account the key belongs to.
    let (library, writable) = if row.library_type == "group" {
        (
            WebLibrary::Group(row.library_id.clone()),
            info.can_write_group(&row.library_id),
        )
    } else if row.library_id == "0" || row.library_id == credentials.user_id.to_string() {
        (WebLibrary::User(credentials.user_id), info.can_write_user())
    } else {
        return WebOutcome::of("no_write");
    };
    if !writable {
        return WebOutcome::of("no_write");
    }
    if let WebLibrary::User(key_account) = &library {
        // The personal library is whichever account the open Zotero is signed in
        // to. A key of another account would write to someone else's library.
        match port.local_user_id() {
            Ok(Some(open)) if open == *key_account => {}
            Ok(Some(_)) => return WebOutcome::of("other_account"),
            Ok(None) | Err(_) => return WebOutcome::of("account_unknown"),
        }
    }
    let mut outcome = match complete_item(web, &credentials.key, &library, &existing.key, ours) {
        Ok(Completion::Completed(fields)) => WebOutcome {
            state: "completed",
            completed: fields,
            ..WebOutcome::of("completed")
        },
        Ok(Completion::NothingMissing) => WebOutcome::of("nothing_missing"),
        Ok(Completion::Conflict) => WebOutcome::of("conflict"),
        // An item just created through the connector exists only in the local
        // Zotero until it syncs: the server does not know it yet.
        Err(failure) if failure.phase == "read_item" && failure.is_not_found() => {
            return WebOutcome::of("not_synced_yet")
        }
        Err(failure) => WebOutcome::failed(&failure),
    };
    // Zotero's own children (read locally) already show this PDF: nothing to do.
    if let Some(pdf) = pdf.filter(|_| pdf_note != "already_there") {
        let short: String = pdf.sha256.chars().take(8).collect();
        let file = PdfFile {
            title: attachment_title(&pdf.sha256),
            filename: format!("web-capture-{short}.pdf"),
            md5: md5_hex(&pdf.bytes),
            size: pdf.bytes.len() as u64,
            mtime_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as u64)
                .unwrap_or(0),
            bytes: pdf.bytes.clone(),
        };
        outcome.pdf = Some(
            match attach_pdf_via_web(web, &credentials.key, &library, &existing.key, &file) {
                PdfOutcome::Attached => "attached",
                PdfOutcome::AlreadyThere => "already_there",
                PdfOutcome::Quota => "quota",
                PdfOutcome::Failed(failure) => {
                    outcome.pdf_reason = Some(failure.code());
                    "failed"
                }
            },
        );
    }
    outcome
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
        let (mut pending, kept, pdf_note) = existing_report(
            conn,
            port,
            &library,
            &existing,
            &ours,
            pdf.as_ref().map(|pdf| pdf.sha256.as_str()),
            (&row.source_id, &row.library_type, &row.library_id),
        )?;
        let mut detail = json!({
            "existing": true,
            "pdf": pdf_note,
            "keptFields": names(&kept),
        });
        if let Some(web) = options.web.as_deref() {
            let outcome = complete_through_web(
                conn,
                port,
                web,
                &ExistingCopy {
                    row,
                    existing: &existing,
                    ours: &ours,
                    pdf: pdf.as_ref(),
                    pdf_note,
                },
            );
            pending.retain(|field| !outcome.completed.contains(field));
            detail["web"] = json!({
                "state": outcome.state,
                "completed": names(&outcome.completed),
            });
            if let Some(reason) = &outcome.reason {
                detail["web"]["reason"] = json!(reason);
            }
            if let Some(reason) = &outcome.pdf_reason {
                detail["web"]["pdfReason"] = json!(reason);
            }
            if let Some(result) = outcome.pdf {
                detail["web"]["pdf"] = json!(result);
                // Attached, or found already there: the PDF is in Zotero now.
                if matches!(result, "attached" | "already_there") {
                    detail["pdf"] = json!(result);
                }
            }
        }
        detail["pendingFields"] = names(&pending);
        return Ok(Done {
            state: store::STATE_LINKED,
            item_key: existing.key,
            detail,
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

/// What an item that is already in Zotero would need: the fields a copy could
/// fill or update, the ones the person edited (kept), and what a PDF could do.
fn existing_report(
    conn: &Connection,
    port: &dyn ZoteroPort,
    library: &Library,
    existing: &ExistingItem,
    ours: &Owned,
    pdf_sha: Option<&str>,
    written_for: (&str, &str, &str),
) -> Result<(Vec<&'static str>, Vec<&'static str>, &'static str), Stop> {
    let written = previous_written(conn, written_for.0, written_for.1, written_for.2);
    let merge = plan_merge(ours, &existing.owned, &written);
    let pdf_note = match pdf_sha {
        None => "none",
        Some(sha) => {
            let title = attachment_title(sha);
            let children = port.children(library, &existing.key).map_err(read_error)?;
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
    let pending = merge.fill.iter().chain(&merge.update).copied().collect();
    Ok((pending, merge.kept, pdf_note))
}

/// One library Zotero offers for writing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveLibrary {
    pub library_type: String,
    pub library_id: String,
    /// `None` for the personal library (the UI has its own word for it).
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryList {
    /// `false` when Zotero did not answer: the list is then empty and the UI
    /// falls back to the libraries the archive knows.
    pub reachable: bool,
    pub libraries: Vec<LiveLibrary>,
}

/// The libraries Zotero itself offers for writing: the personal one first, then
/// the groups whose name is among the editable targets of the connector.
pub fn live_libraries(port: &dyn ZoteroPort) -> Result<LibraryList, String> {
    let unreachable = LibraryList {
        reachable: false,
        libraries: Vec::new(),
    };
    if port.ping().is_err() {
        return Ok(unreachable);
    }
    let Ok(targets) = port.targets() else {
        return Ok(unreachable);
    };
    let mut libraries = vec![LiveLibrary {
        library_type: "user".into(),
        library_id: "0".into(),
        name: None,
    }];
    for (id, name) in port.groups().unwrap_or_default() {
        if targets
            .libraries
            .iter()
            .any(|target| target.id != "L1" && target.name == name)
        {
            libraries.push(LiveLibrary {
                library_type: "group".into(),
                library_id: id,
                name: Some(name),
            });
        }
    }
    Ok(LibraryList {
        reachable: true,
        libraries,
    })
}

/// Whether a source is already in a library, before anything is copied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyStatus {
    /// `absent`, `present` or `unreachable` (Zotero closed and no record).
    pub state: String,
    /// `zotero` (found by address) or `record` (our copy record, Zotero closed).
    pub source: String,
    pub item_key: Option<String>,
    pub pdf: String,
    pub pending_fields: Vec<String>,
    pub kept_fields: Vec<String>,
    /// The saved PDF that goes along with this copy: the named capture, or the
    /// source's latest one. `None`: no PDF goes along.
    pub pdf_capture: Option<PdfCaptureInfo>,
    /// The item is there, a Web API key is stored, and the item lacks something
    /// a copy could fill in or attach.
    pub can_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfCaptureInfo {
    pub id: String,
    /// When the PDF was saved (UTC, RFC 3339).
    pub saved_at: String,
}

fn status(state: &str, source: &str) -> CopyStatus {
    CopyStatus {
        state: state.into(),
        source: source.into(),
        item_key: None,
        pdf: "none".into(),
        pending_fields: Vec::new(),
        kept_fields: Vec::new(),
        pdf_capture: None,
        can_complete: false,
    }
}

/// Checks, without writing to Zotero, whether the source is already in the
/// library: by address when Zotero answers, else from our own record. A record
/// of an item Zotero no longer has is dropped so the source can be copied again.
pub fn check_status(
    conn: &Connection,
    data_dir: &Path,
    port: &dyn ZoteroPort,
    source_id: &str,
    capture_id: Option<&str>,
    library: &store::LibraryRef,
) -> Result<CopyStatus, String> {
    let (lib_type, lib_id) = (library.library_type.trim(), library.library_id.trim());
    let kind = match lib_type {
        "group" => LibraryType::Group,
        "user" => LibraryType::User,
        _ => return Err("invalid_library: the library is not a Zotero library".into()),
    };
    let zotero_library =
        Library::new(kind, lib_id).map_err(|message| format!("invalid_library: {message}"))?;
    let to_text = |stop: Stop| match stop {
        Stop::Wait => "zotero_unavailable: Zotero did not answer".to_string(),
        Stop::Fail { code, message } => format!("{code}: {message}"),
    };
    let facts = load_source(conn, source_id, capture_id).map_err(to_text)?;
    let recorded = store::recorded_item(conn, source_id, lib_type, lib_id)?;
    // The PDF that would go along: the named capture, or the source's latest.
    let picked: Option<(PdfCaptureInfo, String)> = match capture_id {
        Some(capture) => conn
            .query_row(
                "SELECT accessed_at, sha256 FROM web_captures WHERE id = ?1",
                [capture],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| format!("db_error: {error}"))?
            .map(|(saved_at, sha)| {
                (
                    PdfCaptureInfo {
                        id: capture.to_string(),
                        saved_at,
                    },
                    sha,
                )
            }),
        None => latest_pdf(conn, data_dir, source_id)
            .map(|(id, saved_at, pdf)| (PdfCaptureInfo { id, saved_at }, pdf.sha256)),
    };
    let sha: Option<String> = picked.as_ref().map(|(_, sha)| sha.clone());
    let pdf_capture: Option<PdfCaptureInfo> = picked.map(|(info, _)| info);
    let tagged = |mut got: CopyStatus| {
        got.pdf_capture = pdf_capture.clone();
        got
    };
    let from_record = |recorded: Option<String>| match recorded {
        Some(key) => CopyStatus {
            item_key: Some(key),
            pdf: if pdf_capture.is_some() {
                "unknown"
            } else {
                "none"
            }
            .into(),
            ..status("present", "record")
        },
        None => status("unreachable", "none"),
    };

    if port.ping().is_err() {
        return Ok(tagged(from_record(recorded)));
    }
    let urls = lookup_urls(&facts);
    let found = match find_existing(port, &zotero_library, &urls) {
        Ok(found) => found,
        Err(Stop::Wait) => return Ok(tagged(from_record(recorded))),
        Err(stop) => return Err(to_text(stop)),
    };
    let Some(existing) = found.into_iter().next() else {
        if recorded.is_some() {
            store::forget_result(conn, source_id, lib_type, lib_id)?;
        }
        return Ok(tagged(status("absent", "none")));
    };
    let ours = owned_from_zotero(&webpage_item(&facts, CONNECTOR_ID));
    let (pending, kept, pdf) = existing_report(
        conn,
        port,
        &zotero_library,
        &existing,
        &ours,
        sha.as_deref(),
        (source_id, lib_type, lib_id),
    )
    .map_err(to_text)?;
    let has_key = get_setting(conn, ZOTERO_API_KEY).is_some_and(|key| !key.trim().is_empty());
    let can_complete = has_key && (!pending.is_empty() || pdf == "parent_exists");
    Ok(tagged(CopyStatus {
        item_key: Some(existing.key),
        pdf: pdf.into(),
        pending_fields: pending.into_iter().map(String::from).collect(),
        kept_fields: kept.into_iter().map(String::from).collect(),
        can_complete,
        ..status("present", "zotero")
    }))
}

static DRAINING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The one slot for a drain: the view polls and a button may press at the same
/// time, and two drains would race for the same rows.
pub struct DrainGuard;

impl DrainGuard {
    pub fn try_acquire() -> Option<Self> {
        use std::sync::atomic::Ordering;
        DRAINING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for DrainGuard {
    fn drop(&mut self) {
        DRAINING.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// [`drain`], unless another drain is already running (`None`).
pub fn drain_exclusive(
    conn: &Connection,
    data_dir: &Path,
    port: &dyn ZoteroPort,
    options: &RunOptions,
) -> Result<Option<DrainReport>, String> {
    let Some(_slot) = DrainGuard::try_acquire() else {
        return Ok(None);
    };
    drain(conn, data_dir, port, options).map(Some)
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
        /// The account the open Zotero belongs to (7 matches the test key).
        user_id: Result<Option<u64>, PortError>,
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
                user_id: Ok(Some(7)),
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
        fn groups(&self) -> Result<Vec<(String, String)>, PortError> {
            self.calls.borrow_mut().push("groups".into());
            let mut all: Vec<(String, String)> = self
                .groups
                .iter()
                .filter_map(|(id, name)| Some((id.clone(), name.clone()?)))
                .collect();
            all.sort();
            Ok(all)
        }
        fn local_user_id(&self) -> Result<Option<u64>, PortError> {
            self.calls.borrow_mut().push("local_user_id".into());
            self.user_id.clone()
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
            ..RunOptions::default()
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
        // The source's saved PDF goes along, attached before the read-back.
        assert_eq!(
            port.calls(),
            vec!["ping", "targets", "find 0", "save", "move L1", "attach", "find 0"]
        );
        let (session, uri, item) = port.saved.borrow()[0].clone();
        assert_eq!(uri, "https://a.test/x");
        assert_eq!(item["itemType"], "webpage");
        assert_eq!(item["title"], "A title");
        assert_eq!(item["websiteTitle"], "A site");
        assert_eq!(port.moved.borrow()[0], (session, "L1".to_string()));
        let detail = done.detail.unwrap();
        assert_eq!(detail["existing"], false);
        assert_eq!(detail["pdf"], "attached");
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
    fn only_one_drain_runs_at_a_time_and_the_guard_lets_go() {
        let first = DrainGuard::try_acquire().expect("nothing else is draining");
        assert!(
            DrainGuard::try_acquire().is_none(),
            "a second drain must not start"
        );
        drop(first);
        assert!(
            DrainGuard::try_acquire().is_some(),
            "dropping the guard frees the slot"
        );
    }

    #[test]
    fn an_exclusive_drain_that_finds_the_slot_taken_does_nothing() {
        let env = env();
        store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open();
        let held = DrainGuard::try_acquire().unwrap();
        let report = drain_exclusive(&env.conn, env.data.path(), &port, &opts()).unwrap();
        assert!(report.is_none());
        assert!(port.calls().is_empty());
        drop(held);
        let report = drain_exclusive(&env.conn, env.data.path(), &port, &opts()).unwrap();
        assert!(report.is_some());
    }

    #[test]
    fn the_address_searched_is_the_final_one() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open().creating("K");
        go(&env, &port, &row.id);
        assert_eq!(port.searched.borrow()[0], "https://a.test/x");
    }

    // --- live library list and the "already in Zotero" check -------------------

    #[test]
    fn libraries_come_from_zotero_personal_first_and_only_the_editable_groups() {
        let mut port = FakePort::open();
        port.groups
            .insert("111".into(), Some("not editable here".into()));
        let list = live_libraries(&port).unwrap();
        assert!(list.reachable);
        let got: Vec<(&str, &str, Option<&str>)> = list
            .libraries
            .iter()
            .map(|l| {
                (
                    l.library_type.as_str(),
                    l.library_id.as_str(),
                    l.name.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![("user", "0", None), ("group", "6680944", Some("prueba"))]
        );
    }

    #[test]
    fn a_closed_zotero_lists_nothing_and_says_unreachable() {
        let mut port = FakePort::open();
        port.ping = Err(PortError::Unreachable);
        let list = live_libraries(&port).unwrap();
        assert!(!list.reachable);
        assert!(list.libraries.is_empty());
    }

    #[test]
    fn without_the_group_list_the_personal_library_is_still_offered() {
        let mut port = FakePort::open();
        port.targets = Err(PortError::Unreachable);
        let list = live_libraries(&port).unwrap();
        assert!(!list.reachable);
    }

    fn status(env: &Env, port: &FakePort, capture: Option<&str>) -> CopyStatus {
        check_status(
            &env.conn,
            env.data.path(),
            port,
            "src1",
            capture,
            &personal(),
        )
        .unwrap()
    }

    #[test]
    fn a_source_not_in_zotero_is_absent_and_nothing_is_written() {
        let env = env();
        let port = FakePort::open();
        let got = status(&env, &port, None);
        assert_eq!(got.state, "absent");
        assert_eq!(got.item_key, None);
        assert!(port.saved.borrow().is_empty());
    }

    #[test]
    fn a_source_found_by_address_is_present_with_what_would_differ() {
        let env = env();
        let port = FakePort::open();
        let mut existing = hit("OLDKEY22", "webpage");
        existing["data"]["title"] = json!("My own title");
        *port.finds.borrow_mut() = VecDeque::from([Ok(vec![existing])]);
        let got = status(&env, &port, None);
        assert_eq!(got.state, "present");
        assert_eq!(got.source, "zotero");
        assert_eq!(got.item_key.as_deref(), Some("OLDKEY22"));
        assert_eq!(got.kept_fields, vec!["title"]);
        assert_eq!(got.pending_fields, vec!["accessDate", "websiteTitle"]);
        // The source has a saved PDF that would go along; the item lacks it.
        assert_eq!(got.pdf, "parent_exists");
    }

    #[test]
    fn a_pdf_that_cannot_join_an_existing_page_is_said_up_front() {
        let env = env();
        let port = FakePort::open();
        *port.finds.borrow_mut() = VecDeque::from([Ok(vec![hit("OLDKEY22", "webpage")])]);
        assert_eq!(status(&env, &port, Some("cap-pdf")).pdf, "parent_exists");
    }

    #[test]
    fn a_pdf_already_attached_is_recognised_up_front() {
        let env = env();
        let mut port = FakePort::open();
        let sha = format!("{:x}", Sha256::digest(PDF));
        port.children = vec![json!({"data": {"title": format!("Captura web {}.pdf", &sha[..8])}})];
        *port.finds.borrow_mut() = VecDeque::from([Ok(vec![hit("OLDKEY22", "webpage")])]);
        assert_eq!(status(&env, &port, Some("cap-pdf")).pdf, "already_there");
    }

    #[test]
    fn a_closed_zotero_answers_from_our_record_when_there_is_one() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        store::claim(&env.conn, &row.id).unwrap();
        store::finish(
            &env.conn,
            &row.id,
            store::STATE_COPIED,
            Some("RECKEY22"),
            "{}",
        )
        .unwrap();
        let mut port = FakePort::open();
        port.ping = Err(PortError::Unreachable);
        let got = status(&env, &port, None);
        assert_eq!(
            (got.state.as_str(), got.source.as_str()),
            ("present", "record")
        );
        assert_eq!(got.item_key.as_deref(), Some("RECKEY22"));
    }

    #[test]
    fn a_closed_zotero_without_a_record_is_unreachable() {
        let env = env();
        let mut port = FakePort::open();
        port.ping = Err(PortError::Unreachable);
        assert_eq!(status(&env, &port, None).state, "unreachable");
    }

    #[test]
    fn a_record_of_an_item_the_person_deleted_is_dropped_so_it_can_be_copied_again() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        store::claim(&env.conn, &row.id).unwrap();
        store::finish(
            &env.conn,
            &row.id,
            store::STATE_COPIED,
            Some("GONEKEY2"),
            "{}",
        )
        .unwrap();
        let port = FakePort::open();
        assert_eq!(status(&env, &port, None).state, "absent");
        let again = store::request(&env.conn, "src1", None, &personal()).unwrap();
        assert_eq!(again.state, store::STATE_QUEUED);
        assert_eq!(again.item_key, None);
    }

    #[test]
    fn a_disabled_local_api_is_an_error_not_a_guess() {
        let env = env();
        let port = FakePort::open();
        *port.finds.borrow_mut() = VecDeque::from([Err(PortError::Rejected {
            status: 403,
            detail: String::new(),
        })]);
        let error =
            check_status(&env.conn, env.data.path(), &port, "src1", None, &personal()).unwrap_err();
        assert!(error.starts_with("zotero_api_disabled"), "{error}");
    }

    #[test]
    fn the_status_of_a_missing_source_or_bad_library_is_refused() {
        let env = env();
        let port = FakePort::open();
        let gone =
            check_status(&env.conn, env.data.path(), &port, "nope", None, &personal()).unwrap_err();
        assert!(gone.starts_with("not_found"), "{gone}");
        let bad = LibraryRef {
            library_type: "team".into(),
            library_id: "1".into(),
            library_name: None,
        };
        let error =
            check_status(&env.conn, env.data.path(), &port, "src1", None, &bad).unwrap_err();
        assert!(error.starts_with("invalid_library"), "{error}");
    }

    // --- Completing an existing item through the Web API ------------------------

    use super::super::web::{
        md5_hex, Authorization, Patched, PdfFile, UploadTicket, WebError, WebItem, WebLibrary,
        WebPort,
    };
    use crate::zotero_web::{GroupAccess, KeyCheck, KeyInfo};
    use std::sync::{Arc, Mutex};

    const WEB_KEY: &str = "AbCdEf0123456789abcdEFgh";

    struct FakeWeb {
        key: Result<KeyCheck, WebError>,
        items: Mutex<Vec<Result<WebItem, WebError>>>,
        patches: Mutex<Vec<Result<Patched, WebError>>>,
        calls: Mutex<Vec<String>>,
        files: Mutex<FileScript>,
    }

    /// What the file-upload side answers; the default is a flow that works.
    struct FileScript {
        children: Vec<Value>,
        auth: Result<Authorization, WebError>,
        upload: Result<(), WebError>,
    }

    impl FakeWeb {
        fn script_children(&self, children: Vec<Value>) {
            self.files.lock().unwrap().children = children;
        }
        fn script_auth(&self, auth: Result<Authorization, WebError>) {
            self.files.lock().unwrap().auth = auth;
        }
        fn script_upload(&self, upload: Result<(), WebError>) {
            self.files.lock().unwrap().upload = upload;
        }
        fn log(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    fn key_info(personal_write: bool, group_write: bool) -> KeyCheck {
        KeyCheck::Valid(KeyInfo {
            user_id: 7,
            username: "agus".into(),
            personal_library: true,
            personal_write,
            groups: vec![GroupAccess {
                id: "6680944".into(),
                library: true,
                write: group_write,
                name: None,
            }],
        })
    }

    fn web_with(
        key: Result<KeyCheck, WebError>,
        items: Vec<Result<WebItem, WebError>>,
        patches: Vec<Result<Patched, WebError>>,
    ) -> Arc<FakeWeb> {
        Arc::new(FakeWeb {
            key,
            items: Mutex::new(items),
            patches: Mutex::new(patches),
            calls: Mutex::new(vec![]),
            files: Mutex::new(FileScript {
                children: vec![],
                auth: Ok(Authorization::Upload(UploadTicket {
                    url: "https://storage.test/u".into(),
                    content_type: "multipart/form-data; boundary=x".into(),
                    prefix: "--x\r\n".into(),
                    suffix: "\r\n--x--".into(),
                    upload_key: "UPKEY".into(),
                })),
                upload: Ok(()),
            }),
        })
    }

    /// The existing item as the Web API returns it.
    fn web_item(extra: Value) -> Result<WebItem, WebError> {
        let mut data = hit("OLDKEY22", "webpage")["data"].clone();
        for (name, value) in extra.as_object().unwrap() {
            data[name] = value.clone();
        }
        Ok(WebItem { version: 4, data })
    }

    impl WebPort for FakeWeb {
        fn key_info(&self, _: &str) -> Result<KeyCheck, WebError> {
            self.calls.lock().unwrap().push("key_info".into());
            self.key.clone()
        }
        fn get_item(&self, _: &str, library: &WebLibrary, item: &str) -> Result<WebItem, WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("get {item} {library:?}"));
            self.items.lock().unwrap().remove(0)
        }
        fn patch_item(
            &self,
            _: &str,
            _: &WebLibrary,
            item: &str,
            version: u64,
            body: &Value,
        ) -> Result<Patched, WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("patch {item} v{version} {body}"));
            self.patches.lock().unwrap().remove(0)
        }
        fn children(&self, _: &str, _: &WebLibrary, parent: &str) -> Result<Vec<Value>, WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("children {parent}"));
            Ok(self.files.lock().unwrap().children.clone())
        }
        fn create_attachment(
            &self,
            _: &str,
            _: &WebLibrary,
            body: &Value,
            _: &str,
        ) -> Result<(String, u64), WebError> {
            self.calls.lock().unwrap().push(format!("create {body}"));
            Ok(("ATTKEY12".into(), 31))
        }
        fn authorize_upload(
            &self,
            _: &str,
            _: &WebLibrary,
            item: &str,
            file: &PdfFile,
        ) -> Result<Authorization, WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("authorize {item} {}", file.md5));
            self.files.lock().unwrap().auth.clone()
        }
        fn upload_file(&self, _: &UploadTicket, bytes: &[u8]) -> Result<(), WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("upload {}", bytes.len()));
            self.files.lock().unwrap().upload.clone()
        }
        fn register_upload(
            &self,
            _: &str,
            _: &WebLibrary,
            item: &str,
            upload_key: &str,
        ) -> Result<(), WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("register {item} {upload_key}"));
            Ok(())
        }
        fn delete_item(&self, _: &str, _: &WebLibrary, item: &str, v: u64) -> Result<(), WebError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("delete {item} v{v}"));
            Ok(())
        }
    }

    fn store_web_key(env: &Env) {
        env.conn
            .execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT OR REPLACE INTO app_settings (key, value) VALUES ('zotero_api_key', '{WEB_KEY}');
                 INSERT OR REPLACE INTO app_settings (key, value) VALUES ('zotero_user_id', '7');"
            ))
            .unwrap();
    }

    fn existing_port() -> FakePort {
        let port = FakePort::open();
        *port.finds.borrow_mut() = VecDeque::from([Ok(vec![hit("OLDKEY22", "webpage")])]);
        port
    }

    fn go_web(env: &Env, port: &FakePort, id: &str, web: &Arc<FakeWeb>) -> store::ZoteroCopy {
        let options = RunOptions {
            web: Some(web.clone()),
            ..opts()
        };
        run_one(&env.conn, env.data.path(), port, id, &options).unwrap()
    }

    #[test]
    fn a_verified_writing_key_completes_the_missing_fields_of_an_existing_item() {
        let env = env();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let web = web_with(
            Ok(key_info(true, false)),
            vec![web_item(json!({}))],
            vec![Ok(Patched::Done)],
        );
        let port = existing_port();
        let done = go_web(&env, &port, &row.id, &web);
        assert_eq!(done.state, store::STATE_LINKED);
        let detail = done.detail.unwrap();
        assert_eq!(detail["web"]["state"], "completed");
        assert_eq!(
            detail["web"]["completed"],
            json!(["accessDate", "websiteTitle"])
        );
        assert_eq!(detail["pendingFields"], json!([]));
        let calls = web.calls.lock().unwrap().clone();
        assert_eq!(calls[0], "key_info");
        assert_eq!(calls[1], "get OLDKEY22 User(7)");
        assert!(calls[2].starts_with("patch OLDKEY22 v4 "), "{}", calls[2]);
        // The connector stays out of it: nothing is created or moved.
        assert!(port.saved.borrow().is_empty() && port.moved.borrow().is_empty());
    }

    #[test]
    fn without_a_stored_key_the_copy_behaves_as_before() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let web = web_with(Ok(key_info(true, true)), vec![], vec![]);
        let done = go_web(&env, &existing_port(), &row.id, &web);
        assert_eq!(done.state, store::STATE_LINKED);
        let detail = done.detail.unwrap();
        assert_eq!(detail["web"]["state"], "no_key");
        assert_eq!(
            detail["pendingFields"],
            json!(["accessDate", "websiteTitle"])
        );
        assert!(web.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_key_that_cannot_write_to_that_library_is_left_alone() {
        let env = env();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let web = web_with(Ok(key_info(false, true)), vec![], vec![]);
        let done = go_web(&env, &existing_port(), &row.id, &web);
        let detail = done.detail.unwrap();
        assert_eq!(detail["web"]["state"], "no_write");
        assert_eq!(*web.calls.lock().unwrap(), vec!["key_info"]);
    }

    #[test]
    fn a_key_zotero_no_longer_recognises_is_reported_as_invalid() {
        let env = env();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let web = web_with(Ok(KeyCheck::InvalidKey), vec![], vec![]);
        let done = go_web(&env, &existing_port(), &row.id, &web);
        assert_eq!(done.detail.unwrap()["web"]["state"], "invalid_key");
    }

    #[test]
    fn a_group_item_is_addressed_by_its_group_id() {
        let env = env();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", None, &group()).unwrap();
        let web = web_with(
            Ok(key_info(false, true)),
            vec![web_item(json!({}))],
            vec![Ok(Patched::Done)],
        );
        let done = go_web(&env, &existing_port(), &row.id, &web);
        assert_eq!(done.detail.unwrap()["web"]["state"], "completed");
        assert_eq!(
            web.calls.lock().unwrap()[1],
            "get OLDKEY22 Group(\"6680944\")"
        );
    }

    #[test]
    fn nothing_missing_says_so_and_writes_nothing() {
        let env = env();
        // No saved PDF: only the fields are in play.
        env.conn.execute("DELETE FROM web_captures", []).unwrap();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let full = json!({"accessDate": "2020-01-01T00:00:00Z", "websiteTitle": "Mine"});
        let web = web_with(Ok(key_info(true, true)), vec![web_item(full)], vec![]);
        let done = go_web(&env, &existing_port(), &row.id, &web);
        assert_eq!(done.detail.unwrap()["web"]["state"], "nothing_missing");
        assert_eq!(web.calls.lock().unwrap().len(), 2, "key_info and get only");
    }

    #[test]
    fn a_conflict_after_the_retry_still_links_the_item() {
        let env = env();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let web = web_with(
            Ok(key_info(true, true)),
            vec![web_item(json!({})), web_item(json!({}))],
            vec![Ok(Patched::Stale), Ok(Patched::Stale)],
        );
        let done = go_web(&env, &existing_port(), &row.id, &web);
        assert_eq!(done.state, store::STATE_LINKED);
        let detail = done.detail.unwrap();
        assert_eq!(detail["web"]["state"], "conflict");
        assert_eq!(
            detail["pendingFields"],
            json!(["accessDate", "websiteTitle"])
        );
    }

    #[test]
    fn a_web_api_failure_never_fails_the_copy() {
        let failures = [
            web_with(Err(WebError::Unreachable), vec![], vec![]),
            web_with(
                Ok(key_info(true, true)),
                vec![web_item(json!({}))],
                vec![Err(WebError::Rejected(400))],
            ),
        ];
        for failing in failures {
            let env = env();
            store_web_key(&env);
            let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
            let done = go_web(&env, &existing_port(), &row.id, &failing);
            assert_eq!(done.state, store::STATE_LINKED);
            let detail = done.detail.unwrap();
            assert_eq!(detail["web"]["state"], "failed");
            assert!(!detail.to_string().contains(WEB_KEY));
        }
    }

    #[test]
    fn a_key_of_another_account_never_writes_to_the_personal_library() {
        let env = env();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let web = web_with(Ok(key_info(true, true)), vec![], vec![]);
        let mut port = existing_port();
        port.user_id = Ok(Some(99));
        let done = go_web(&env, &port, &row.id, &web);
        assert_eq!(done.state, store::STATE_LINKED);
        assert_eq!(done.detail.unwrap()["web"]["state"], "other_account");
        assert_eq!(*web.calls.lock().unwrap(), vec!["key_info"]);
    }

    #[test]
    fn an_account_that_cannot_be_read_is_never_guessed() {
        for unreadable in [Ok(None), Err(PortError::Unreachable)] {
            let env = env();
            store_web_key(&env);
            let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
            let web = web_with(Ok(key_info(true, true)), vec![], vec![]);
            let mut port = existing_port();
            port.user_id = unreadable;
            let done = go_web(&env, &port, &row.id, &web);
            assert_eq!(done.detail.unwrap()["web"]["state"], "account_unknown");
            assert_eq!(*web.calls.lock().unwrap(), vec!["key_info"]);
        }
    }

    #[test]
    fn the_matching_account_is_read_before_the_write_and_a_group_needs_no_check() {
        let env = env();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let web = web_with(
            Ok(key_info(true, true)),
            vec![web_item(json!({}))],
            vec![Ok(Patched::Done)],
        );
        let port = existing_port();
        let done = go_web(&env, &port, &row.id, &web);
        assert_eq!(done.detail.unwrap()["web"]["state"], "completed");
        assert!(port.calls().contains(&"local_user_id".to_string()));

        // A group is addressed by its own id: the open account is not consulted,
        // even when it would not match.
        let group_row = store::request(&env.conn, "src1", None, &group()).unwrap();
        let web = web_with(
            Ok(key_info(false, true)),
            vec![web_item(json!({}))],
            vec![Ok(Patched::Done)],
        );
        let mut port = existing_port();
        port.user_id = Ok(Some(99));
        let done = go_web(&env, &port, &group_row.id, &web);
        assert_eq!(done.detail.unwrap()["web"]["state"], "completed");
        assert!(!port.calls().contains(&"local_user_id".to_string()));
    }

    // --- The PDF of an existing item through the Web API ------------------------

    /// A writing key, the matching account, and an item already in Zotero, with
    /// the captured PDF of the source.
    fn pdf_copy(web: &Arc<FakeWeb>, port: &FakePort) -> (Env, store::ZoteroCopy) {
        let env = env();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", Some("cap-pdf"), &personal()).unwrap();
        let done = go_web(&env, port, &row.id, web);
        (env, done)
    }

    fn pdf_web() -> Arc<FakeWeb> {
        // The fields are all there already: only the PDF is left to do.
        let full = json!({"accessDate": "2020-01-01T00:00:00Z", "websiteTitle": "Mine"});
        web_with(Ok(key_info(true, true)), vec![web_item(full)], vec![])
    }

    #[test]
    fn the_captured_pdf_is_attached_to_the_existing_item_through_the_web_api() {
        let web = pdf_web();
        let (_env, done) = pdf_copy(&web, &existing_port());
        assert_eq!(done.state, store::STATE_LINKED);
        let detail = done.detail.unwrap();
        assert_eq!(detail["pdf"], "attached");
        assert_eq!(detail["web"]["pdf"], "attached");
        let calls = web.log();
        let md5 = md5_hex(PDF);
        let at = |prefix: &str| calls.iter().position(|c| c.starts_with(prefix)).unwrap();
        assert!(at("children OLDKEY22") < at("create"));
        assert!(at("create") < at("authorize ATTKEY12"));
        assert!(calls.contains(&format!("authorize ATTKEY12 {md5}")));
        assert!(at("authorize") < at("upload") && at("upload") < at("register"));
        assert_eq!(
            calls.iter().find(|c| c.starts_with("upload")).unwrap(),
            &format!("upload {}", PDF.len())
        );
        let create = calls.iter().find(|c| c.starts_with("create")).unwrap();
        assert!(create.contains("\"parentItem\":\"OLDKEY22\""), "{create}");
        assert!(create.contains("Captura web "), "{create}");
    }

    #[test]
    fn a_pdf_the_item_already_has_is_not_attached_twice() {
        let web = pdf_web();
        web.script_children(vec![json!({
            "itemType": "attachment", "md5": md5_hex(PDF), "filename": "old.pdf"
        })]);
        let (_env, done) = pdf_copy(&web, &existing_port());
        let detail = done.detail.unwrap();
        assert_eq!(detail["web"]["pdf"], "already_there");
        assert_eq!(detail["pdf"], "already_there");
        assert!(!web.log().iter().any(|c| c.starts_with("create")));
    }

    #[test]
    fn a_pdf_zotero_already_stores_is_linked_without_an_upload() {
        let web = pdf_web();
        web.script_auth(Ok(Authorization::Exists));
        let (_env, done) = pdf_copy(&web, &existing_port());
        assert_eq!(done.detail.unwrap()["web"]["pdf"], "attached");
        assert!(!web.log().iter().any(|c| c.starts_with("upload")));
    }

    #[test]
    fn a_full_quota_is_reported_and_never_fails_the_copy() {
        let web = pdf_web();
        web.script_auth(Err(WebError::Rejected(413)));
        let (_env, done) = pdf_copy(&web, &existing_port());
        assert_eq!(done.state, store::STATE_LINKED);
        let detail = done.detail.unwrap();
        assert_eq!(detail["web"]["pdf"], "quota");
        assert_eq!(detail["pdf"], "parent_exists", "the PDF is still not there");
        assert!(web.log().contains(&"delete ATTKEY12 v31".to_string()));
    }

    #[test]
    fn any_other_upload_failure_is_a_failed_pdf_not_a_failed_copy() {
        let web = pdf_web();
        web.script_upload(Err(WebError::Rejected(500)));
        let (_env, done) = pdf_copy(&web, &existing_port());
        assert_eq!(done.state, store::STATE_LINKED);
        let detail = done.detail.unwrap();
        assert_eq!(detail["web"]["pdf"], "failed");
        assert!(!detail.to_string().contains(WEB_KEY));
    }

    #[test]
    fn a_source_without_a_pdf_never_asks_for_children() {
        let env = env();
        env.conn.execute("DELETE FROM web_captures", []).unwrap();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let web = pdf_web();
        let done = go_web(&env, &existing_port(), &row.id, &web);
        assert!(done.detail.unwrap()["web"].get("pdf").is_none());
        assert!(!web.log().iter().any(|c| c.starts_with("children")));
    }

    #[test]
    fn the_guard_applies_to_the_pdf_too() {
        let web = pdf_web();
        let mut port = existing_port();
        port.user_id = Ok(Some(99));
        let (_env, done) = pdf_copy(&web, &port);
        let detail = done.detail.unwrap();
        assert_eq!(detail["web"]["state"], "other_account");
        assert!(detail["web"].get("pdf").is_none());
        assert_eq!(web.log(), vec!["key_info"]);
    }

    #[test]
    fn a_pdf_zotero_shows_locally_is_not_looked_up_again() {
        let web = pdf_web();
        let mut port = existing_port();
        let sha = format!("{:x}", Sha256::digest(PDF));
        port.children = vec![json!({"data": {"title": attachment_title(&sha)}})];
        let (_env, done) = pdf_copy(&web, &port);
        let detail = done.detail.unwrap();
        assert_eq!(detail["pdf"], "already_there");
        assert!(detail["web"].get("pdf").is_none());
        assert!(!web.log().iter().any(|c| c.starts_with("children")));
    }

    // --- Diagnostics, items not yet on the server, and the source-level PDF -----

    /// A second PDF capture of `src1`. With no bytes the row exists but the file
    /// does not.
    fn add_pdf_capture(env: &Env, id: &str, accessed_at: &str, bytes: Option<&[u8]>) -> String {
        let content = bytes.unwrap_or(b"%PDF-1.4 missing");
        let sha = format!("{:x}", Sha256::digest(content));
        let rel = format!("web-captures/src1/{id}.pdf");
        if let Some(bytes) = bytes {
            std::fs::write(env.data.path().join(&rel), bytes).unwrap();
        }
        env.conn
            .execute(
                "INSERT INTO web_captures (id, web_source_id, accessed_at, final_url, kind, mime_type, rel_path, sha256, hash_of, size_bytes, title, created_at)
                 VALUES (?1, 'src1', ?2, 'https://a.test/other.pdf', 'pdf', 'application/pdf', ?3, ?4, 'pdf', ?5, NULL, 2)",
                rusqlite::params![id, accessed_at, rel, sha, content.len() as i64],
            )
            .unwrap();
        sha
    }

    #[test]
    fn a_failure_records_where_it_happened_and_never_the_key() {
        let cases: Vec<(Arc<FakeWeb>, &str)> = vec![
            (
                web_with(Err(WebError::Unreachable), vec![], vec![]),
                "key_info:network",
            ),
            (
                web_with(Err(WebError::Rejected(500)), vec![], vec![]),
                "key_info:500",
            ),
            (
                web_with(Ok(KeyCheck::Unreachable), vec![], vec![]),
                "key_info:network",
            ),
            (
                web_with(
                    Ok(key_info(true, true)),
                    vec![Err(WebError::Rejected(500))],
                    vec![],
                ),
                "read_item:500",
            ),
            (
                web_with(
                    Ok(key_info(true, true)),
                    vec![web_item(json!({}))],
                    vec![Err(WebError::Rejected(400))],
                ),
                "patch:400",
            ),
        ];
        for (web, expected) in cases {
            let env = env();
            store_web_key(&env);
            let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
            let done = go_web(&env, &existing_port(), &row.id, &web);
            let detail = done.detail.unwrap();
            assert_eq!(detail["web"]["state"], "failed", "{expected}");
            assert_eq!(detail["web"]["reason"], expected);
            assert!(!detail.to_string().contains(WEB_KEY));
        }
    }

    #[test]
    fn a_pdf_failure_records_its_phase_too() {
        let web = pdf_web();
        web.script_upload(Err(WebError::Rejected(500)));
        let (_env, done) = pdf_copy(&web, &existing_port());
        let detail = done.detail.unwrap();
        assert_eq!(detail["web"]["pdf"], "failed");
        assert_eq!(detail["web"]["pdfReason"], "upload:500");
    }

    #[test]
    fn an_item_the_server_does_not_know_yet_is_not_synced_and_nothing_else_is_tried() {
        let env = env();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", Some("cap-pdf"), &personal()).unwrap();
        let web = web_with(
            Ok(key_info(true, true)),
            vec![Err(WebError::Rejected(404))],
            vec![],
        );
        let done = go_web(&env, &existing_port(), &row.id, &web);
        assert_eq!(done.state, store::STATE_LINKED);
        let detail = done.detail.unwrap();
        assert_eq!(detail["web"]["state"], "not_synced_yet");
        assert!(detail["web"].get("reason").is_none());
        assert!(detail["web"].get("pdf").is_none(), "the PDF is not tried");
        assert!(!web.log().iter().any(|c| c.starts_with("children")));
    }

    #[test]
    fn copying_the_source_takes_its_saved_pdf_along_to_a_new_item() {
        let env = env();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open().creating("NEWKEY22");
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_COPIED);
        assert_eq!(done.detail.unwrap()["pdf"], "attached");
        assert_eq!(port.attached.borrow()[0].4, PDF);
    }

    #[test]
    fn copying_the_source_attaches_its_pdf_to_an_existing_item_through_the_web_api() {
        let env = env();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let web = pdf_web();
        let done = go_web(&env, &existing_port(), &row.id, &web);
        let detail = done.detail.unwrap();
        assert_eq!(detail["web"]["pdf"], "attached");
        assert!(web
            .log()
            .contains(&format!("authorize ATTKEY12 {}", md5_hex(PDF))));
    }

    #[test]
    fn with_several_pdf_captures_the_latest_one_goes() {
        let env = env();
        let newer: &[u8] = b"%PDF-1.4 the newer one";
        add_pdf_capture(&env, "cap-new", "2026-10-03T08:00:00Z", Some(newer));
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open().creating("NEWKEY22");
        go(&env, &port, &row.id);
        assert_eq!(port.attached.borrow()[0].4, newer);
        assert_eq!(port.attached.borrow()[0].3, "https://a.test/other.pdf");
    }

    #[test]
    fn a_newer_capture_whose_file_is_gone_falls_back_to_the_next_one() {
        let env = env();
        add_pdf_capture(&env, "cap-gone", "2026-10-03T08:00:00Z", None);
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open().creating("NEWKEY22");
        let done = go(&env, &port, &row.id);
        assert_eq!(
            done.state,
            store::STATE_COPIED,
            "the page copy never fails for it"
        );
        assert_eq!(port.attached.borrow()[0].4, PDF);
    }

    #[test]
    fn a_source_without_a_pdf_capture_copies_only_the_page() {
        let env = env();
        env.conn.execute("DELETE FROM web_captures", []).unwrap();
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let port = FakePort::open().creating("NEWKEY22");
        let done = go(&env, &port, &row.id);
        assert_eq!(done.state, store::STATE_COPIED);
        assert!(port.attached.borrow().is_empty());
        assert_eq!(done.detail.unwrap()["pdf"], "none");
    }

    #[test]
    fn the_status_says_which_pdf_would_go_along() {
        let env = env();
        let port = FakePort::open();
        // From the source: the latest verified PDF capture.
        let got =
            check_status(&env.conn, env.data.path(), &port, "src1", None, &personal()).unwrap();
        let pdf = got.pdf_capture.unwrap();
        assert_eq!(pdf.id, "cap-pdf");
        assert_eq!(pdf.saved_at, "2026-10-02T09:30:00Z");
        add_pdf_capture(
            &env,
            "cap-new",
            "2026-10-03T08:00:00Z",
            Some(b"%PDF-1.4 newer"),
        );
        let got =
            check_status(&env.conn, env.data.path(), &port, "src1", None, &personal()).unwrap();
        assert_eq!(got.pdf_capture.unwrap().id, "cap-new");
        // From a capture: that capture.
        let got = check_status(
            &env.conn,
            env.data.path(),
            &port,
            "src1",
            Some("cap-pdf"),
            &personal(),
        )
        .unwrap();
        assert_eq!(got.pdf_capture.unwrap().id, "cap-pdf");
        // With none saved: none, in every answer.
        env.conn.execute("DELETE FROM web_captures", []).unwrap();
        let got =
            check_status(&env.conn, env.data.path(), &port, "src1", None, &personal()).unwrap();
        assert!(got.pdf_capture.is_none());
    }

    #[test]
    fn the_status_of_a_present_item_sees_the_source_level_pdf_as_missing_there() {
        let env = env();
        let port = existing_port();
        let got =
            check_status(&env.conn, env.data.path(), &port, "src1", None, &personal()).unwrap();
        assert_eq!(got.state, "present");
        assert_eq!(got.pdf, "parent_exists");
    }

    #[test]
    fn completing_is_offered_only_with_a_key_and_something_to_complete() {
        let env = env();
        let port = existing_port();
        let got =
            check_status(&env.conn, env.data.path(), &port, "src1", None, &personal()).unwrap();
        assert!(!got.can_complete, "no key stored");
        store_web_key(&env);
        let got =
            check_status(&env.conn, env.data.path(), &port, "src1", None, &personal()).unwrap();
        assert!(got.can_complete);
        // Nothing left to fill and no PDF: nothing to complete.
        env.conn.execute("DELETE FROM web_captures", []).unwrap();
        let full = FakePort::open();
        let mut data = hit("OLDKEY22", "webpage");
        data["data"]["accessDate"] = json!("2026-10-01T10:00:00Z");
        data["data"]["websiteTitle"] = json!("A site");
        *full.finds.borrow_mut() = VecDeque::from([Ok(vec![data])]);
        let got =
            check_status(&env.conn, env.data.path(), &full, "src1", None, &personal()).unwrap();
        assert!(!got.can_complete);
    }

    #[test]
    fn a_newly_created_item_never_touches_the_web_api() {
        let env = env();
        store_web_key(&env);
        let row = store::request(&env.conn, "src1", None, &personal()).unwrap();
        let web = web_with(Ok(key_info(true, true)), vec![], vec![]);
        let done = go_web(&env, &FakePort::open().creating("NEWKEY22"), &row.id, &web);
        assert_eq!(done.state, store::STATE_COPIED);
        assert!(web.calls.lock().unwrap().is_empty());
    }
}
