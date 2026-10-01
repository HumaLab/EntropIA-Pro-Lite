//! Saving what the browser captured: files first, then one database
//! transaction, so there is never a row without its file.
//!
//! The renderer never holds the HTML of a page or the path of a quarantined PDF,
//! and it is not trusted to say what they hash to. It asks to save a draft or a
//! download by id; this module looks the id up in [`Holds`] (what the capture
//! and the download code handed over), re-checks sizes and hashes, and writes.
//!
//! Layout: `<data>/web-captures/<source_id>/<capture_id>.<ext>`, stored as the
//! relative key `web-captures/<source_id>/<capture_id>.<ext>` so the archive
//! stays portable. Captures are immutable: this module only ever inserts
//! `web_captures` rows. `web_sources` rows are found by `final_url` or created,
//! and a later capture refreshes the source's title and `updated_at`.
//!
//! Order of a save:
//! 1. validate the capture (sizes, hash, kind);
//! 2. write each file under a temporary name in its final directory, fsync it
//!    and rename it into place;
//! 3. insert the source (or update it) and the capture in one transaction;
//! 4. if anything after step 2 fails, remove the files written and the source
//!    directory if it was ours, so a failed save leaves nothing behind.
//!
//! A PDF is copied out of quarantine (hashing while it copies, which re-checks
//! the sha256) and the quarantined file is deleted only after the commit. That
//! is a move whose failure never loses the PDF: until the rows exist, the
//! download is still in quarantine and can be saved again.

use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::capture::{self, CaptureDraft, CaptureKind};
use super::download;

/// Text up to this many bytes stays in the row; more goes to a file.
pub const TEXT_IN_ROW_MAX_BYTES: usize = 512 * 1024;

/// Directory under the data directory that holds every saved capture.
pub const DIR: &str = "web-captures";

/// What produced a page or selection capture. Bumped when the capture script
/// changes what it extracts.
pub const EXTRACTOR_VERSION: &str = "navegador-capture-1";

/// Drafts kept on this side waiting for a decision. The oldest goes first: a
/// draft is up to 10 MB of HTML, and the person only ever looks at the latest.
pub const MAX_HELD_DRAFTS: usize = 4;

/// Verified PDFs in quarantine that can still be saved.
pub const MAX_HELD_PDFS: usize = 50;

const COPY_BLOCK: usize = 64 * 1024;

/// Stable codes the UI maps to messages.
pub mod code {
    pub const UNKNOWN_DRAFT: &str = "unknown_draft";
    pub const UNKNOWN_DOWNLOAD: &str = "unknown_download";
    pub const INVALID_CAPTURE: &str = "invalid_capture";
    pub const HASH_MISMATCH: &str = "hash_mismatch";
    pub const FILE_MISSING: &str = "file_missing";
    pub const IO_ERROR: &str = "io_error";
    pub const DB_ERROR: &str = "db_error";
}

/// Why a save did not happen. `code` is stable; `detail` is for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveError {
    pub code: &'static str,
    pub detail: Option<String>,
}

impl SaveError {
    fn new(code: &'static str) -> Self {
        Self { code, detail: None }
    }

    fn with_detail(code: &'static str, detail: impl ToString) -> Self {
        Self {
            code,
            detail: Some(detail.to_string()),
        }
    }
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{}: {detail}", self.code),
            None => f.write_str(self.code),
        }
    }
}

/// The ids of what was saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub source_id: String,
    pub capture_id: String,
}

/// Where a save goes: the data directory, an open archive connection and the
/// clock (injected so tests are exact).
pub struct Target<'a> {
    pub data_dir: &'a Path,
    pub conn: &'a Connection,
    pub now_ms: i64,
}

/// A verified PDF waiting in quarantine as `<id>.pdf`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyPdf {
    pub id: String,
    /// Where the file came from.
    pub url: String,
    /// Sanitized name, for display and as the title of last resort.
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    /// UTC, RFC 3339, from this process's clock when the download started.
    pub accessed_at: String,
    /// Title of the page the download started from.
    pub page_title: Option<String>,
}

/// What this side keeps for the renderer: capture drafts (with their HTML) and
/// verified PDFs. Saves are serialised, so two clicks never save twice.
#[derive(Default)]
pub struct Holds {
    drafts: Mutex<VecDeque<CaptureDraft>>,
    pdfs: Mutex<VecDeque<ReadyPdf>>,
    saving: Mutex<()>,
}

impl Holds {
    /// Keep `draft` until it is saved or discarded, dropping the oldest ones
    /// beyond [`MAX_HELD_DRAFTS`].
    pub fn hold_draft(&self, draft: CaptureDraft) {
        let mut drafts = self.drafts.lock().unwrap_or_else(|e| e.into_inner());
        drafts.push_back(draft);
        while drafts.len() > MAX_HELD_DRAFTS {
            drafts.pop_front();
        }
    }

    /// Forget a draft the person dismissed.
    pub fn discard_draft(&self, id: &str) {
        let mut drafts = self.drafts.lock().unwrap_or_else(|e| e.into_inner());
        drafts.retain(|draft| draft.id != id);
    }

    /// Drafts held right now.
    pub fn held_drafts(&self) -> usize {
        self.drafts.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Keep a verified PDF's facts so it can be saved by id.
    pub fn hold_pdf(&self, pdf: ReadyPdf) {
        let mut pdfs = self.pdfs.lock().unwrap_or_else(|e| e.into_inner());
        pdfs.retain(|held| held.id != pdf.id);
        pdfs.push_back(pdf);
        while pdfs.len() > MAX_HELD_PDFS {
            pdfs.pop_front();
        }
    }

    /// Forget a PDF (its quarantine file went away or was saved).
    pub fn discard_pdf(&self, id: &str) {
        let mut pdfs = self.pdfs.lock().unwrap_or_else(|e| e.into_inner());
        pdfs.retain(|held| held.id != id);
    }

    /// Save the held draft `id`. A draft that fails to save stays held, so the
    /// person can try again; one that saves is gone.
    pub fn save_draft(&self, id: &str, target: &Target<'_>) -> Result<Saved, SaveError> {
        let _serial = self.saving.lock().unwrap_or_else(|e| e.into_inner());
        let draft = {
            let drafts = self.drafts.lock().unwrap_or_else(|e| e.into_inner());
            drafts.iter().find(|draft| draft.id == id).cloned()
        }
        .ok_or_else(|| SaveError::new(code::UNKNOWN_DRAFT))?;
        let saved = save_draft(target, &draft)?;
        self.discard_draft(id);
        Ok(saved)
    }

    /// Save the verified PDF `id` from `quarantine`.
    pub fn save_pdf(
        &self,
        id: &str,
        target: &Target<'_>,
        quarantine: &Path,
    ) -> Result<Saved, SaveError> {
        let _serial = self.saving.lock().unwrap_or_else(|e| e.into_inner());
        let pdf = {
            let pdfs = self.pdfs.lock().unwrap_or_else(|e| e.into_inner());
            pdfs.iter().find(|held| held.id == id).cloned()
        }
        .ok_or_else(|| SaveError::new(code::UNKNOWN_DOWNLOAD))?;
        let saved = save_pdf(target, &pdf, quarantine)?;
        self.discard_pdf(id);
        Ok(saved)
    }
}

/// The holds shared by the capture commands, the download handler and the save
/// commands. Created on first use, like the viewer's state.
pub fn holds<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::State<'_, Holds> {
    use tauri::Manager;
    // `manage` keeps the first value when one exists.
    app.manage(Holds::default());
    app.state::<Holds>()
}

/// Save a page or selection capture.
pub fn save_draft(target: &Target<'_>, draft: &CaptureDraft) -> Result<Saved, SaveError> {
    let plan = plan_draft(draft)?;
    commit(target, plan)
}

/// Save a verified PDF out of `quarantine`.
pub fn save_pdf(
    target: &Target<'_>,
    pdf: &ReadyPdf,
    quarantine: &Path,
) -> Result<Saved, SaveError> {
    if !download::valid_id(&pdf.id) {
        return Err(SaveError::with_detail(
            code::INVALID_CAPTURE,
            "the download id is not valid",
        ));
    }
    check_url(&pdf.url)?;
    let title = pdf
        .page_title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| pdf.file_name.clone());
    let file = download::pdf_path(quarantine, &pdf.id);
    let plan = Plan {
        source: SourceInfo {
            final_url: pdf.url.clone(),
            canonical_url: None,
            title: Some(title.clone()),
            site_name: None,
            first_accessed_at: pdf.accessed_at.clone(),
        },
        capture: CaptureInfo {
            kind: "pdf",
            mime_type: "application/pdf",
            accessed_at: pdf.accessed_at.clone(),
            title: Some(title),
            text: None,
            quote_prefix: None,
            quote_suffix: None,
            sha256: pdf.sha256.clone(),
            hash_of: "pdf",
            size_bytes: pdf.size,
            extractor_version: None,
        },
        payloads: vec![Payload {
            slot: Slot::Main,
            ext: "pdf",
            body: Body::Copy {
                from: file.clone(),
                sha256: pdf.sha256.clone(),
                size: pdf.size,
            },
        }],
        consumed: Some(file),
    };
    commit(target, plan)
}

/// Where a payload's relative key is stored.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// `rel_path`: the HTML snapshot or the PDF.
    Main,
    /// `text_rel_path`: text too large for the row.
    Text,
}

enum Body {
    Bytes(Vec<u8>),
    /// A file to copy, whose hash and size must be what is expected.
    Copy {
        from: PathBuf,
        sha256: String,
        size: u64,
    },
}

struct Payload {
    slot: Slot,
    ext: &'static str,
    body: Body,
}

struct SourceInfo {
    final_url: String,
    canonical_url: Option<String>,
    title: Option<String>,
    site_name: Option<String>,
    first_accessed_at: String,
}

struct CaptureInfo {
    kind: &'static str,
    mime_type: &'static str,
    accessed_at: String,
    title: Option<String>,
    /// Text for the row; `None` when it goes to a file or there is none.
    text: Option<String>,
    quote_prefix: Option<String>,
    quote_suffix: Option<String>,
    sha256: String,
    hash_of: &'static str,
    size_bytes: u64,
    extractor_version: Option<&'static str>,
}

/// Everything a save will do, validated, before anything is written.
struct Plan {
    source: SourceInfo,
    capture: CaptureInfo,
    payloads: Vec<Payload>,
    /// A file to delete once the rows exist (the quarantined PDF).
    consumed: Option<PathBuf>,
}

fn check_url(url: &str) -> Result<(), SaveError> {
    match tauri::Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https" | "blob") => Ok(()),
        _ => Err(SaveError::with_detail(
            code::INVALID_CAPTURE,
            "the address is not a web address",
        )),
    }
}

fn invalid(why: &str) -> SaveError {
    SaveError::with_detail(code::INVALID_CAPTURE, why)
}

/// Text goes to the row up to [`TEXT_IN_ROW_MAX_BYTES`] and to a file beyond.
fn place_text(text: &str, payloads: &mut Vec<Payload>) -> Option<String> {
    if text.len() <= TEXT_IN_ROW_MAX_BYTES {
        return Some(text.to_string());
    }
    payloads.push(Payload {
        slot: Slot::Text,
        ext: "txt",
        body: Body::Bytes(text.as_bytes().to_vec()),
    });
    None
}

fn plan_draft(draft: &CaptureDraft) -> Result<Plan, SaveError> {
    check_url(&draft.final_url)?;
    if draft.text.len() > capture::TEXT_MAX_BYTES {
        return Err(invalid("the text is too large"));
    }

    let mut payloads = Vec::new();
    let (kind, mime_type, hashed, hash_of) = match draft.kind {
        CaptureKind::Page => {
            let html = draft
                .html
                .as_deref()
                .filter(|html| !html.is_empty())
                .ok_or_else(|| invalid("the page has no snapshot"))?;
            if html.len() > capture::HTML_MAX_BYTES {
                return Err(invalid("the snapshot is too large"));
            }
            ("page", "text/html", html.as_bytes(), "html")
        }
        CaptureKind::Selection => {
            let quote = draft
                .quote
                .as_deref()
                .filter(|quote| !quote.trim().is_empty())
                .ok_or_else(|| invalid("the selection is empty"))?;
            if quote.len() > capture::TEXT_MAX_BYTES {
                return Err(invalid("the selection is too large"));
            }
            ("selection", "text/plain", quote.as_bytes(), "quote")
        }
    };

    // What was hashed is hashed again here: the draft's own claim is not enough.
    if capture::sha256_hex(hashed) != draft.sha256 {
        return Err(SaveError::with_detail(
            code::HASH_MISMATCH,
            "the capture does not match its hash",
        ));
    }

    let (row_text, quote_prefix, quote_suffix) = match draft.kind {
        CaptureKind::Page => {
            payloads.push(Payload {
                slot: Slot::Main,
                ext: "html",
                body: Body::Bytes(hashed.to_vec()),
            });
            (place_text(&draft.text, &mut payloads), None, None)
        }
        CaptureKind::Selection => (
            place_text(&draft.text, &mut payloads),
            draft.quote_prefix.clone(),
            draft.quote_suffix.clone(),
        ),
    };

    Ok(Plan {
        source: SourceInfo {
            final_url: draft.final_url.clone(),
            canonical_url: draft.canonical_url.clone(),
            title: draft.title.clone(),
            site_name: draft.site_name.clone(),
            first_accessed_at: draft.accessed_at.clone(),
        },
        capture: CaptureInfo {
            kind,
            mime_type,
            accessed_at: draft.accessed_at.clone(),
            title: draft.title.clone(),
            text: row_text,
            quote_prefix,
            quote_suffix,
            sha256: draft.sha256.clone(),
            hash_of,
            size_bytes: hashed.len() as u64,
            extractor_version: Some(EXTRACTOR_VERSION),
        },
        payloads,
        consumed: None,
    })
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The relative key stored in the archive: forward slashes on every platform.
fn rel_key(source_id: &str, capture_id: &str, ext: &str) -> String {
    format!("{DIR}/{source_id}/{capture_id}.{ext}")
}

fn db_error(error: impl ToString) -> SaveError {
    SaveError::with_detail(code::DB_ERROR, error)
}

fn io_error(error: impl ToString) -> SaveError {
    SaveError::with_detail(code::IO_ERROR, error)
}

fn find_source(conn: &Connection, final_url: &str) -> Result<Option<String>, SaveError> {
    conn.query_row(
        "SELECT id FROM web_sources WHERE final_url = ?1 ORDER BY created_at, id LIMIT 1",
        [final_url],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_error)
}

/// Files this save wrote, so a failure can take them back.
struct Written {
    files: Vec<PathBuf>,
    tmp: Option<PathBuf>,
    dir: PathBuf,
    /// The directory did not exist before this save.
    dir_is_new: bool,
}

impl Written {
    fn undo(&mut self) {
        if let Some(tmp) = self.tmp.take() {
            let _ = fs::remove_file(tmp);
        }
        for file in self.files.drain(..) {
            let _ = fs::remove_file(file);
        }
        if self.dir_is_new {
            // Only an empty directory goes: `remove_dir` refuses anything else.
            let _ = fs::remove_dir(&self.dir);
        }
    }
}

fn commit(target: &Target<'_>, plan: Plan) -> Result<Saved, SaveError> {
    let source_id = find_source(target.conn, &plan.source.final_url)?.unwrap_or_else(new_id);
    let capture_id = new_id();

    let dir = target.data_dir.join(DIR).join(&source_id);
    let mut written = Written {
        files: Vec::new(),
        tmp: None,
        dir_is_new: !dir.exists(),
        dir: dir.clone(),
    };
    fs::create_dir_all(&dir).map_err(io_error)?;

    let mut rel_path = None;
    let mut text_rel_path = None;
    for payload in &plan.payloads {
        let name = format!("{capture_id}.{}", payload.ext);
        if let Err(error) = write_file(&dir, &name, &payload.body, &mut written) {
            written.undo();
            return Err(error);
        }
        let key = rel_key(&source_id, &capture_id, payload.ext);
        match payload.slot {
            Slot::Main => rel_path = Some(key),
            Slot::Text => text_rel_path = Some(key),
        }
    }

    match insert_rows(
        target,
        &plan,
        &source_id,
        &capture_id,
        rel_path,
        text_rel_path,
    ) {
        Ok(()) => {
            if let Some(consumed) = &plan.consumed {
                // The rows exist; the quarantined copy has done its job.
                let _ = fs::remove_file(consumed);
            }
            Ok(Saved {
                source_id,
                capture_id,
            })
        }
        Err(error) => {
            written.undo();
            Err(error)
        }
    }
}

/// Write `body` as `<dir>/<name>`: under a temporary name first, flushed to
/// disk, then renamed into place, so a crash leaves at worst a `.tmp` file.
fn write_file(dir: &Path, name: &str, body: &Body, written: &mut Written) -> Result<(), SaveError> {
    let tmp = dir.join(format!(".{name}.tmp"));
    written.tmp = Some(tmp.clone());
    let mut file = fs::File::create(&tmp).map_err(io_error)?;
    match body {
        Body::Bytes(bytes) => file.write_all(bytes).map_err(io_error)?,
        Body::Copy { from, sha256, size } => copy_verified(from, &mut file, sha256, *size)?,
    }
    file.sync_all().map_err(io_error)?;
    drop(file);
    let finished = dir.join(name);
    fs::rename(&tmp, &finished).map_err(io_error)?;
    written.tmp = None;
    written.files.push(finished);
    Ok(())
}

/// Copy `from` into `to` while hashing it; the copy is only good if it is the
/// expected size and the expected sha256.
fn copy_verified(
    from: &Path,
    to: &mut fs::File,
    expected_sha256: &str,
    expected_size: u64,
) -> Result<(), SaveError> {
    let mut source = match fs::File::open(from) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(SaveError::new(code::FILE_MISSING))
        }
        Err(error) => return Err(io_error(error)),
    };
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut block = vec![0u8; COPY_BLOCK];
    loop {
        let read = source.read(&mut block).map_err(io_error)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        // A file that keeps growing is not the one that was verified.
        if total > expected_size {
            break;
        }
        hasher.update(&block[..read]);
        to.write_all(&block[..read]).map_err(io_error)?;
    }
    if total != expected_size || format!("{:x}", hasher.finalize()) != expected_sha256 {
        return Err(SaveError::with_detail(
            code::HASH_MISMATCH,
            "the file is not the one that was verified",
        ));
    }
    Ok(())
}

fn insert_rows(
    target: &Target<'_>,
    plan: &Plan,
    source_id: &str,
    capture_id: &str,
    rel_path: Option<String>,
    text_rel_path: Option<String>,
) -> Result<(), SaveError> {
    // IMMEDIATE: take the write lock now, so the source looked up cannot change
    // under this transaction.
    let tx = Transaction::new_unchecked(target.conn, TransactionBehavior::Immediate)
        .map_err(db_error)?;

    match find_source(&tx, &plan.source.final_url)? {
        Some(existing) if existing == source_id => {
            tx.execute(
                "UPDATE web_sources SET title = COALESCE(?1, title), updated_at = ?2
                 WHERE id = ?3",
                params![plan.source.title, target.now_ms, source_id],
            )
            .map_err(db_error)?;
        }
        // Another source took this address after the files were placed.
        Some(_) => {
            return Err(SaveError::with_detail(
                code::DB_ERROR,
                "the source changed while saving; try again",
            ))
        }
        None => {
            tx.execute(
                "INSERT INTO web_sources
                   (id, original_url, final_url, canonical_url, title, site_name,
                    first_accessed_at, created_at, updated_at)
                 VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    source_id,
                    plan.source.final_url,
                    plan.source.canonical_url,
                    plan.source.title,
                    plan.source.site_name,
                    plan.source.first_accessed_at,
                    target.now_ms,
                ],
            )
            .map_err(db_error)?;
        }
    }

    let capture = &plan.capture;
    tx.execute(
        "INSERT INTO web_captures
           (id, web_source_id, accessed_at, final_url, kind, mime_type, text,
            text_rel_path, quote_prefix, quote_suffix, rel_path, sha256, hash_of,
            size_bytes, extractor_version, title, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            capture_id,
            source_id,
            capture.accessed_at,
            plan.source.final_url,
            capture.kind,
            capture.mime_type,
            capture.text,
            text_rel_path,
            capture.quote_prefix,
            capture.quote_suffix,
            rel_path,
            capture.sha256,
            capture.hash_of,
            capture.size_bytes as i64,
            capture.extractor_version,
            capture.title,
            target.now_ms,
        ],
    )
    .map_err(db_error)?;
    tx.commit().map_err(db_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_support::new_app_schema_db;

    const AT: &str = "2026-09-30T12:00:00Z";
    const NOW: i64 = 1_790_000_000_000;

    struct Env {
        data: tempfile::TempDir,
        quarantine: tempfile::TempDir,
        conn: Connection,
    }

    impl Env {
        fn new() -> Self {
            Self {
                data: tempfile::tempdir().unwrap(),
                quarantine: tempfile::tempdir().unwrap(),
                conn: new_app_schema_db(),
            }
        }

        fn target(&self) -> Target<'_> {
            Target {
                data_dir: self.data.path(),
                conn: &self.conn,
                now_ms: NOW,
            }
        }

        fn files(&self) -> Vec<String> {
            fn walk(dir: &Path, out: &mut Vec<String>, root: &Path) {
                for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        walk(&path, out, root);
                    } else {
                        let rel = path.strip_prefix(root).unwrap();
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
            let mut out = Vec::new();
            walk(self.data.path(), &mut out, self.data.path());
            out.sort();
            out
        }

        fn count(&self, table: &str) -> i64 {
            self.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        }

        /// A verified PDF in quarantine, as the download code leaves it.
        fn quarantined_pdf(&self, id: &str, bytes: &[u8]) -> ReadyPdf {
            fs::write(download::pdf_path(self.quarantine.path(), id), bytes).unwrap();
            ReadyPdf {
                id: id.to_string(),
                url: "https://example.com/paper.pdf".into(),
                file_name: "paper.pdf".into(),
                size: bytes.len() as u64,
                sha256: capture::sha256_hex(bytes),
                accessed_at: AT.into(),
                page_title: Some("The paper".into()),
            }
        }
    }

    fn page_draft(url: &str, html: &str, text: &str) -> CaptureDraft {
        CaptureDraft {
            id: uuid::Uuid::new_v4().to_string(),
            kind: CaptureKind::Page,
            final_url: url.into(),
            title: Some("A page".into()),
            canonical_url: Some("https://example.com/canonical".into()),
            site_name: Some("Example".into()),
            lang: Some("en".into()),
            text: text.into(),
            quote: None,
            quote_prefix: None,
            quote_suffix: None,
            html: Some(html.into()),
            html_bytes: html.len(),
            hash_of: "html",
            sha256: capture::sha256_hex(html.as_bytes()),
            truncated: false,
            accessed_at: AT.into(),
        }
    }

    fn selection_draft(url: &str, quote: &str) -> CaptureDraft {
        CaptureDraft {
            id: uuid::Uuid::new_v4().to_string(),
            kind: CaptureKind::Selection,
            final_url: url.into(),
            title: Some("A page".into()),
            canonical_url: None,
            site_name: None,
            lang: None,
            text: quote.into(),
            quote: Some(quote.into()),
            quote_prefix: Some("before ".into()),
            quote_suffix: Some(" after".into()),
            html: None,
            html_bytes: 0,
            hash_of: "quote",
            sha256: capture::sha256_hex(quote.as_bytes()),
            truncated: false,
            accessed_at: AT.into(),
        }
    }

    type SourceRow = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        i64,
        i64,
    );

    type CaptureRow = (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        String,
        i64,
    );

    fn capture_row(env: &Env, id: &str) -> CaptureRow {
        env.conn
            .query_row(
                "SELECT web_source_id, kind, mime_type, text, text_rel_path, rel_path,
                        sha256, hash_of, size_bytes
                 FROM web_captures WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )
            .unwrap()
    }

    #[test]
    fn a_page_is_saved_as_a_file_plus_a_source_and_a_capture_row() {
        let env = Env::new();
        let html = "<html><body>hola</body></html>";
        let draft = page_draft("https://example.com/a", html, "hola");

        let saved = save_draft(&env.target(), &draft).unwrap();

        let rel = format!("web-captures/{}/{}.html", saved.source_id, saved.capture_id);
        assert_eq!(env.files(), vec![rel.clone()]);
        assert_eq!(
            fs::read_to_string(env.data.path().join(&rel)).unwrap(),
            html
        );

        let (source, kind, mime, text, text_rel, rel_path, sha, hash_of, size) =
            capture_row(&env, &saved.capture_id);
        assert_eq!(source, saved.source_id);
        assert_eq!(kind, "page");
        assert_eq!(mime, "text/html");
        assert_eq!(text.as_deref(), Some("hola"));
        assert_eq!(text_rel, None);
        assert_eq!(rel_path.as_deref(), Some(rel.as_str()));
        assert_eq!(sha, capture::sha256_hex(html.as_bytes()));
        assert_eq!(hash_of, "html");
        assert_eq!(size, html.len() as i64);

        let (original, final_url, canonical, title, site, first, created, updated): SourceRow = env
            .conn
            .query_row(
                "SELECT original_url, final_url, canonical_url, title, site_name,
                        first_accessed_at, created_at, updated_at
                 FROM web_sources WHERE id = ?1",
                [&saved.source_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(original, "https://example.com/a");
        assert_eq!(final_url, "https://example.com/a");
        assert_eq!(canonical.as_deref(), Some("https://example.com/canonical"));
        assert_eq!(title.as_deref(), Some("A page"));
        assert_eq!(site.as_deref(), Some("Example"));
        assert_eq!(first, AT);
        assert_eq!((created, updated), (NOW, NOW));
    }

    #[test]
    fn a_selection_keeps_the_quote_in_the_row_and_writes_no_file() {
        let env = Env::new();
        let draft = selection_draft("https://example.com/a", "the quote");

        let saved = save_draft(&env.target(), &draft).unwrap();

        assert!(env.files().is_empty());
        let (_, kind, mime, text, text_rel, rel_path, sha, hash_of, size) =
            capture_row(&env, &saved.capture_id);
        assert_eq!(kind, "selection");
        assert_eq!(mime, "text/plain");
        assert_eq!(text.as_deref(), Some("the quote"));
        assert_eq!((text_rel, rel_path), (None, None));
        assert_eq!(sha, capture::sha256_hex(b"the quote"));
        assert_eq!(hash_of, "quote");
        assert_eq!(size, 9);
        let (prefix, suffix): (Option<String>, Option<String>) = env
            .conn
            .query_row(
                "SELECT quote_prefix, quote_suffix FROM web_captures WHERE id = ?1",
                [&saved.capture_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(prefix.as_deref(), Some("before "));
        assert_eq!(suffix.as_deref(), Some(" after"));
    }

    #[test]
    fn a_verified_pdf_is_moved_out_of_quarantine_with_its_hash_checked() {
        let env = Env::new();
        let bytes = b"%PDF-1.7 a small pdf";
        let pdf = env.quarantined_pdf("dl-1", bytes);

        let saved = save_pdf(&env.target(), &pdf, env.quarantine.path()).unwrap();

        let rel = format!("web-captures/{}/{}.pdf", saved.source_id, saved.capture_id);
        assert_eq!(env.files(), vec![rel.clone()]);
        assert_eq!(fs::read(env.data.path().join(&rel)).unwrap(), bytes);
        assert!(
            !download::pdf_path(env.quarantine.path(), "dl-1").exists(),
            "the quarantined copy is gone once the rows exist"
        );

        let (_, kind, mime, text, _, rel_path, sha, hash_of, size) =
            capture_row(&env, &saved.capture_id);
        assert_eq!(kind, "pdf");
        assert_eq!(mime, "application/pdf");
        assert_eq!(text, None);
        assert_eq!(rel_path.as_deref(), Some(rel.as_str()));
        assert_eq!(sha, capture::sha256_hex(bytes));
        assert_eq!(hash_of, "pdf");
        assert_eq!(size, bytes.len() as i64);
        let title: Option<String> = env
            .conn
            .query_row(
                "SELECT title FROM web_sources WHERE id = ?1",
                [&saved.source_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("The paper"));
    }

    #[test]
    fn a_pdf_changed_after_verification_is_refused_and_stays_in_quarantine() {
        let env = Env::new();
        let pdf = env.quarantined_pdf("dl-1", b"%PDF-1.7 original");
        // Same size, different bytes: only the hash can tell.
        fs::write(
            download::pdf_path(env.quarantine.path(), "dl-1"),
            b"%PDF-1.7 tampere",
        )
        .unwrap();

        let error = save_pdf(&env.target(), &pdf, env.quarantine.path()).unwrap_err();

        assert_eq!(error.code, code::HASH_MISMATCH);
        assert!(env.files().is_empty(), "no destination file survives");
        assert_eq!(env.count("web_sources") + env.count("web_captures"), 0);
        assert!(download::pdf_path(env.quarantine.path(), "dl-1").exists());
    }

    #[test]
    fn a_pdf_missing_from_quarantine_is_reported() {
        let env = Env::new();
        let pdf = env.quarantined_pdf("dl-1", b"%PDF-1.7 x");
        fs::remove_file(download::pdf_path(env.quarantine.path(), "dl-1")).unwrap();

        let error = save_pdf(&env.target(), &pdf, env.quarantine.path()).unwrap_err();

        assert_eq!(error.code, code::FILE_MISSING);
        assert_eq!(env.count("web_captures"), 0);
    }

    #[test]
    fn a_pdf_id_cannot_climb_out_of_quarantine() {
        let env = Env::new();
        let mut pdf = env.quarantined_pdf("dl-1", b"%PDF-1.7 x");
        pdf.id = "../dl-1".into();

        let error = save_pdf(&env.target(), &pdf, env.quarantine.path()).unwrap_err();

        assert_eq!(error.code, code::INVALID_CAPTURE);
    }

    #[test]
    fn captures_of_one_final_url_share_one_source_and_refresh_its_title() {
        let env = Env::new();
        let mut first = page_draft("https://example.com/a", "<p>1</p>", "1");
        first.title = Some("Old title".into());
        let mut second = page_draft("https://example.com/a", "<p>2</p>", "2");
        second.title = Some("New title".into());
        let other = page_draft("https://example.com/b", "<p>3</p>", "3");

        let one = save_draft(&env.target(), &first).unwrap();
        let later = Target {
            now_ms: NOW + 5_000,
            ..env.target()
        };
        let two = save_draft(&later, &second).unwrap();
        let three = save_draft(&env.target(), &other).unwrap();

        assert_eq!(one.source_id, two.source_id);
        assert_ne!(one.source_id, three.source_id);
        assert_ne!(one.capture_id, two.capture_id);
        assert_eq!(env.count("web_sources"), 2);
        assert_eq!(env.count("web_captures"), 3);

        let (title, first_at, created, updated): (String, String, i64, i64) = env
            .conn
            .query_row(
                "SELECT title, first_accessed_at, created_at, updated_at
                 FROM web_sources WHERE id = ?1",
                [&one.source_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(title, "New title");
        assert_eq!(first_at, AT);
        assert_eq!((created, updated), (NOW, NOW + 5_000));
        // Both captures of the first source live in its directory.
        let prefix = format!("web-captures/{}/", one.source_id);
        assert_eq!(
            env.files()
                .iter()
                .filter(|f| f.starts_with(&prefix))
                .count(),
            2
        );
    }

    #[test]
    fn a_capture_without_a_title_keeps_the_sources_title() {
        let env = Env::new();
        let first = page_draft("https://example.com/a", "<p>1</p>", "1");
        let mut second = page_draft("https://example.com/a", "<p>2</p>", "2");
        second.title = None;

        let one = save_draft(&env.target(), &first).unwrap();
        save_draft(&env.target(), &second).unwrap();

        let title: Option<String> = env
            .conn
            .query_row(
                "SELECT title FROM web_sources WHERE id = ?1",
                [&one.source_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("A page"));
    }

    #[test]
    fn text_up_to_512_kb_stays_in_the_row_and_more_goes_to_a_file() {
        let env = Env::new();
        let exact = "a".repeat(TEXT_IN_ROW_MAX_BYTES);
        let kept = save_draft(
            &env.target(),
            &page_draft("https://example.com/a", "<p>x</p>", &exact),
        )
        .unwrap();
        let (_, _, _, text, text_rel, _, _, _, _) = capture_row(&env, &kept.capture_id);
        assert_eq!(text.as_deref(), Some(exact.as_str()));
        assert_eq!(text_rel, None);

        // Bytes, not characters: 'é' is two bytes.
        let wide = "é".repeat(TEXT_IN_ROW_MAX_BYTES / 2 + 1);
        let big = save_draft(
            &env.target(),
            &page_draft("https://example.com/b", "<p>y</p>", &wide),
        )
        .unwrap();
        let (_, _, _, text, text_rel, rel_path, _, _, _) = capture_row(&env, &big.capture_id);
        assert_eq!(text, None);
        let expected = format!("web-captures/{}/{}.txt", big.source_id, big.capture_id);
        assert_eq!(text_rel.as_deref(), Some(expected.as_str()));
        assert!(rel_path.unwrap().ends_with(".html"));
        assert_eq!(
            fs::read_to_string(env.data.path().join(&expected)).unwrap(),
            wide
        );
    }

    #[test]
    fn a_huge_quote_goes_to_a_text_file_too() {
        let env = Env::new();
        let quote = "q".repeat(TEXT_IN_ROW_MAX_BYTES + 10);

        let saved = save_draft(
            &env.target(),
            &selection_draft("https://example.com/a", &quote),
        )
        .unwrap();

        let (_, _, _, text, text_rel, rel_path, _, _, size) = capture_row(&env, &saved.capture_id);
        assert_eq!(text, None);
        assert_eq!(rel_path, None);
        assert!(text_rel.unwrap().ends_with(".txt"));
        assert_eq!(size, quote.len() as i64);
    }

    #[test]
    fn a_database_failure_removes_every_file_and_leaves_no_rows() {
        let env = Env::new();
        env.conn
            .execute_batch(
                "CREATE TRIGGER refuse_captures BEFORE INSERT ON web_captures
                 BEGIN SELECT RAISE(ABORT, 'disk full, say'); END;",
            )
            .unwrap();
        let wide = "é".repeat(TEXT_IN_ROW_MAX_BYTES / 2 + 1);
        let draft = page_draft("https://example.com/a", "<p>x</p>", &wide);

        let error = save_draft(&env.target(), &draft).unwrap_err();

        assert_eq!(error.code, code::DB_ERROR);
        assert!(error.detail.unwrap().contains("disk full"));
        assert!(env.files().is_empty(), "left behind: {:?}", env.files());
        assert!(
            !env.data
                .path()
                .join(DIR)
                .read_dir()
                .is_ok_and(|mut d| d.next().is_some()),
            "the source directory is removed too"
        );
        assert_eq!(env.count("web_sources") + env.count("web_captures"), 0);
    }

    #[test]
    fn a_database_failure_keeps_the_files_of_an_earlier_capture_of_the_source() {
        let env = Env::new();
        let first = save_draft(
            &env.target(),
            &page_draft("https://example.com/a", "<p>1</p>", "1"),
        )
        .unwrap();
        env.conn
            .execute_batch(
                "CREATE TRIGGER refuse_captures BEFORE INSERT ON web_captures
                 BEGIN SELECT RAISE(ABORT, 'nope'); END;",
            )
            .unwrap();

        let error = save_draft(
            &env.target(),
            &page_draft("https://example.com/a", "<p>2</p>", "2"),
        )
        .unwrap_err();

        assert_eq!(error.code, code::DB_ERROR);
        assert_eq!(
            env.files(),
            vec![format!(
                "web-captures/{}/{}.html",
                first.source_id, first.capture_id
            )]
        );
        assert_eq!(env.count("web_captures"), 1);
    }

    #[test]
    fn a_pdf_stays_in_quarantine_when_the_database_refuses_it() {
        let env = Env::new();
        let pdf = env.quarantined_pdf("dl-1", b"%PDF-1.7 keep me");
        env.conn
            .execute_batch(
                "CREATE TRIGGER refuse_captures BEFORE INSERT ON web_captures
                 BEGIN SELECT RAISE(ABORT, 'nope'); END;",
            )
            .unwrap();

        let error = save_pdf(&env.target(), &pdf, env.quarantine.path()).unwrap_err();

        assert_eq!(error.code, code::DB_ERROR);
        assert!(env.files().is_empty());
        assert!(download::pdf_path(env.quarantine.path(), "dl-1").exists());
    }

    #[test]
    fn a_draft_whose_hash_is_not_its_content_is_refused_before_anything_is_written() {
        let env = Env::new();
        let mut draft = page_draft("https://example.com/a", "<p>x</p>", "x");
        draft.sha256 = capture::sha256_hex(b"something else");

        let error = save_draft(&env.target(), &draft).unwrap_err();

        assert_eq!(error.code, code::HASH_MISMATCH);
        assert!(env.files().is_empty());
        assert_eq!(env.count("web_sources"), 0);
    }

    #[test]
    fn a_draft_that_cannot_be_a_capture_is_refused() {
        let env = Env::new();
        let mut no_html = page_draft("https://example.com/a", "<p>x</p>", "x");
        no_html.html = None;
        let mut no_quote = selection_draft("https://example.com/a", "q");
        no_quote.quote = None;
        let mut bad_url = page_draft("file:///etc/passwd", "<p>x</p>", "x");
        bad_url.final_url = "file:///etc/passwd".into();
        let mut huge_html = page_draft("https://example.com/a", "<p>x</p>", "x");
        huge_html.html = Some("h".repeat(capture::HTML_MAX_BYTES + 1));

        for draft in [no_html, no_quote, bad_url, huge_html] {
            let error = save_draft(&env.target(), &draft).unwrap_err();
            assert_eq!(error.code, code::INVALID_CAPTURE, "{:?}", draft.kind);
        }
        assert!(env.files().is_empty());
        assert_eq!(env.count("web_sources"), 0);
    }

    #[test]
    fn saving_an_unknown_draft_or_download_is_refused() {
        let env = Env::new();
        let holds = Holds::default();

        let draft = holds.save_draft("nope", &env.target()).unwrap_err();
        let download = holds
            .save_pdf("nope", &env.target(), env.quarantine.path())
            .unwrap_err();

        assert_eq!(draft.code, code::UNKNOWN_DRAFT);
        assert_eq!(download.code, code::UNKNOWN_DOWNLOAD);
    }

    #[test]
    fn a_saved_draft_is_gone_and_a_failed_one_can_be_tried_again() {
        let env = Env::new();
        let holds = Holds::default();
        let draft = page_draft("https://example.com/a", "<p>x</p>", "x");
        let id = draft.id.clone();
        holds.hold_draft(draft);

        env.conn
            .execute_batch(
                "CREATE TRIGGER refuse_captures BEFORE INSERT ON web_captures
                 BEGIN SELECT RAISE(ABORT, 'nope'); END;",
            )
            .unwrap();
        assert_eq!(
            holds.save_draft(&id, &env.target()).unwrap_err().code,
            code::DB_ERROR
        );
        assert_eq!(holds.held_drafts(), 1, "still held after the failure");

        env.conn
            .execute_batch("DROP TRIGGER refuse_captures;")
            .unwrap();
        holds.save_draft(&id, &env.target()).unwrap();
        assert_eq!(holds.held_drafts(), 0);
        assert_eq!(
            holds.save_draft(&id, &env.target()).unwrap_err().code,
            code::UNKNOWN_DRAFT,
            "a second click cannot save it twice"
        );
        assert_eq!(env.count("web_captures"), 1);
    }

    #[test]
    fn a_saved_pdf_cannot_be_saved_twice() {
        let env = Env::new();
        let holds = Holds::default();
        holds.hold_pdf(env.quarantined_pdf("dl-1", b"%PDF-1.7 once"));

        holds
            .save_pdf("dl-1", &env.target(), env.quarantine.path())
            .unwrap();
        let again = holds
            .save_pdf("dl-1", &env.target(), env.quarantine.path())
            .unwrap_err();

        assert_eq!(again.code, code::UNKNOWN_DOWNLOAD);
        assert_eq!(env.count("web_captures"), 1);
    }

    #[test]
    fn only_the_latest_drafts_are_held_and_a_dismissed_one_is_dropped() {
        let holds = Holds::default();
        let drafts: Vec<CaptureDraft> = (0..MAX_HELD_DRAFTS + 2)
            .map(|n| page_draft(&format!("https://example.com/{n}"), "<p/>", "t"))
            .collect();
        let ids: Vec<String> = drafts.iter().map(|d| d.id.clone()).collect();
        for draft in drafts {
            holds.hold_draft(draft);
        }
        assert_eq!(holds.held_drafts(), MAX_HELD_DRAFTS);

        let env = Env::new();
        // The oldest two were evicted.
        assert_eq!(
            holds.save_draft(&ids[0], &env.target()).unwrap_err().code,
            code::UNKNOWN_DRAFT
        );
        holds.discard_draft(&ids[5]);
        assert_eq!(holds.held_drafts(), MAX_HELD_DRAFTS - 1);
        assert_eq!(
            holds.save_draft(&ids[5], &env.target()).unwrap_err().code,
            code::UNKNOWN_DRAFT
        );
    }

    #[test]
    fn stored_paths_are_relative_with_forward_slashes() {
        let env = Env::new();
        let saved = save_draft(
            &env.target(),
            &page_draft("https://example.com/a", "<p>x</p>", "x"),
        )
        .unwrap();

        let (_, _, _, _, _, rel_path, _, _, _) = capture_row(&env, &saved.capture_id);
        let rel = rel_path.unwrap();
        assert!(!Path::new(&rel).is_absolute());
        assert!(!rel.contains('\\') && !rel.contains(".."));
        assert!(rel.starts_with("web-captures/"));
    }

    #[test]
    fn no_temporary_file_is_left_after_a_save() {
        let env = Env::new();
        save_draft(
            &env.target(),
            &page_draft("https://example.com/a", "<p>x</p>", &"é".repeat(300_000)),
        )
        .unwrap();

        assert!(env.files().iter().all(|f| !f.ends_with(".tmp")));
        assert_eq!(env.files().len(), 2);
    }

    #[test]
    fn the_result_reaches_the_ui_in_camel_case() {
        let json = serde_json::to_value(Saved {
            source_id: "s".into(),
            capture_id: "c".into(),
        })
        .unwrap();
        assert_eq!(json["sourceId"], "s");
        assert_eq!(json["captureId"], "c");
    }
}
