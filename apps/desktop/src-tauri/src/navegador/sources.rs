//! Reading and deleting the saved web sources.
//!
//! Rust owns `web_sources` and `web_captures` end to end: [`super::save`]
//! writes them, this module reads and deletes them, and the renderer only asks
//! by command. Reads live here, not in a TypeScript repository over `db_select`,
//! because everything the list shows beyond the rows (is the file still on
//! disk?) needs the file system, a delete has to change the rows and the files
//! together, and one typed set of commands is a smaller surface than SQL built in
//! the renderer.
//!
//! Search is a plain substring match (`LIKE` with the wildcards escaped) over
//! titles, URLs and the text kept in the capture rows. SQLite's `LIKE` folds
//! only ASCII case, so a query is also tried in lower case, upper case and with
//! a capital first letter: that covers `educación`, `EDUCACIÓN` and
//! `Educación`, not every mixed spelling. Text larger than 512 KB lives in a
//! file and is not searched.
//!
//! Deleting a source removes its rows (the captures with them) in one
//! transaction and then its folder. Copies made into a collection are
//! independent and are not touched. A folder that will not go never fails the
//! delete: the rows are already gone, so the startup sweep sees a folder with no
//! source and removes it.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params_from_iter, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;

use super::capture_files::{self, Located};
use super::save::DIR;
use super::text_copy;

/// Sources a list returns when the caller does not say.
pub const DEFAULT_LIMIT: usize = 200;
/// The most a list may return.
pub const MAX_LIMIT: usize = 500;
/// Longest search text used, in characters.
pub const QUERY_MAX_CHARS: usize = 200;
/// Longest text preview of a capture, in characters.
pub const PREVIEW_MAX_CHARS: usize = 2000;

/// Stable codes the UI maps to messages.
pub mod code {
    pub const INVALID_ID: &str = "invalid_id";
    pub const NOT_FOUND: &str = "not_found";
    pub const NOT_A_PDF: &str = "not_a_pdf";
    pub const FILE_MISSING: &str = "file_missing";
    pub const FILE_CHANGED: &str = "file_changed";
    pub const NO_TEXT: &str = "no_text";
    pub const DB_ERROR: &str = "db_error";
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub id: String,
    pub title: Option<String>,
    pub final_url: String,
    pub site_name: Option<String>,
    /// Epoch milliseconds.
    pub updated_at: i64,
    pub capture_count: i64,
    /// Distinct kinds among its captures (`page`, `pdf`, `selection`), sorted.
    pub kinds: Vec<String>,
}

/// One capture of a source, as the detail shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDetail {
    pub id: String,
    pub kind: String,
    pub mime_type: String,
    /// UTC, RFC 3339, exactly as recorded.
    pub accessed_at: String,
    pub final_url: String,
    pub title: Option<String>,
    pub sha256: String,
    /// What `sha256` covers: `html`, `quote` or `pdf`.
    pub hash_of: String,
    pub size_bytes: i64,
    /// The start of the text kept in the row (the quote of a selection), cut to
    /// [`PREVIEW_MAX_CHARS`] characters.
    pub text_preview: Option<String>,
    /// The text was too large for the row and lives in a file.
    pub text_in_file: bool,
    pub quote_prefix: Option<String>,
    pub quote_suffix: Option<String>,
    /// Whether the saved file (HTML snapshot or PDF) is on disk; `None` when
    /// this capture has no file.
    pub file_present: Option<bool>,
    /// The file came from another device and sync is still downloading it.
    pub file_pending: bool,
    /// Epoch milliseconds.
    pub created_at: i64,
}

/// A source with its captures, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDetail {
    pub id: String,
    pub original_url: String,
    pub final_url: String,
    pub canonical_url: Option<String>,
    pub title: Option<String>,
    pub site_name: Option<String>,
    pub first_accessed_at: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub captures: Vec<CaptureDetail>,
}

/// Where a web capture came from, as it is written into the item a copy creates
/// (`items.metadata.__entropia_web_capture`). Built here from the rows, never
/// from anything the renderer sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebCaptureProvenance {
    pub source_id: String,
    pub capture_id: String,
    pub original_url: String,
    pub final_url: String,
    /// The capture's own title, else the source's.
    pub page_title: Option<String>,
    /// UTC, RFC 3339, exactly as recorded.
    pub accessed_at: String,
    /// What was verified when the PDF was saved; for a rendered copy, the
    /// capture's own hash (of the page's HTML, or of the quote).
    pub sha256: String,
    /// `page` or `selection`: only set on a copy rendered from a capture's text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_kind: Option<String>,
    /// `text-pdf`: the copy is a PDF rendered from the capture's text, not the
    /// page itself. Absent on a copy of a saved PDF.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rendering: Option<String>,
}

/// What copying a saved PDF into a collection needs: the file, found on this
/// side, and its provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyTicket {
    /// Absolute path of the saved PDF, for the import to copy from. The copy is
    /// a new file: nothing keeps pointing into the web-captures folder.
    pub path: String,
    pub provenance: WebCaptureProvenance,
}

/// What a delete did besides removing the rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOutcome {
    /// Some files or the folder could not be removed now; the startup sweep
    /// takes them later. The source itself is gone either way.
    pub leftover_files: bool,
}

fn db_error(error: impl ToString) -> String {
    format!("{}: {}", code::DB_ERROR, error.to_string())
}

/// The forms of `text` a case-insensitive search tries: as typed, lower case,
/// upper case and with a capital first letter, without repeats.
pub fn query_variants(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let upper = text.to_uppercase();
    let mut chars = lower.chars();
    let capitalised = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
        None => String::new(),
    };
    let mut variants = vec![text.to_string()];
    for variant in [lower, upper, capitalised] {
        if !variants.contains(&variant) {
            variants.push(variant);
        }
    }
    variants
}

/// `text` with the `LIKE` wildcards and the escape character made literal,
/// wrapped as a "contains" pattern. Pairs with `ESCAPE '\'`.
pub fn contains_pattern(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// `field LIKE ?1 ESCAPE '\' OR field LIKE ?2 ...` over `variants` numbered
/// parameters, for each field, in parentheses.
fn like_any(fields: &[&str], variants: usize) -> String {
    let clauses: Vec<String> = fields
        .iter()
        .flat_map(|field| (1..=variants).map(move |n| format!("{field} LIKE ?{n} ESCAPE '\\'")))
        .collect();
    format!("({})", clauses.join(" OR "))
}

/// The search text as used: trimmed and cut to [`QUERY_MAX_CHARS`] characters;
/// `None` when nothing is left.
fn normalise_query(query: Option<&str>) -> Option<String> {
    let text: String = query?.trim().chars().take(QUERY_MAX_CHARS).collect();
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn invalid_id() -> String {
    format!("{}: the source id is not valid", code::INVALID_ID)
}

/// The saved sources, newest first (`updated_at`), optionally only those whose
/// title, URLs or captured text contain `query`.
pub fn list_sources(
    conn: &Connection,
    query: Option<&str>,
    limit: usize,
) -> Result<Vec<SourceSummary>, String> {
    let limit = match limit {
        0 => DEFAULT_LIMIT,
        n => n.min(MAX_LIMIT),
    };
    let mut values: Vec<rusqlite::types::Value> = Vec::new();
    let mut filter = String::new();
    if let Some(text) = normalise_query(query) {
        let variants = query_variants(&text);
        for variant in &variants {
            values.push(contains_pattern(variant).into());
        }
        filter = format!(
            "WHERE {} OR EXISTS (SELECT 1 FROM web_captures c
                                 WHERE c.web_source_id = s.id AND {})",
            like_any(
                &[
                    "s.title",
                    "s.site_name",
                    "s.original_url",
                    "s.final_url",
                    "s.canonical_url"
                ],
                variants.len(),
            ),
            like_any(&["c.text", "c.title", "c.final_url"], variants.len()),
        );
    }
    let limit_slot = values.len() + 1;
    values.push((limit as i64).into());
    let sql = format!(
        "SELECT s.id, s.title, s.final_url, s.site_name, s.updated_at,
                (SELECT COUNT(*) FROM web_captures c WHERE c.web_source_id = s.id),
                (SELECT group_concat(DISTINCT c.kind) FROM web_captures c
                  WHERE c.web_source_id = s.id)
         FROM web_sources s
         {filter}
         ORDER BY s.updated_at DESC, s.id
         LIMIT ?{limit_slot}"
    );
    let mut statement = conn.prepare(&sql).map_err(db_error)?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            let kinds: Option<String> = row.get(6)?;
            let mut kinds: Vec<String> = kinds
                .map(|joined| joined.split(',').map(str::to_string).collect())
                .unwrap_or_default();
            kinds.sort();
            Ok(SourceSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                final_url: row.get(2)?,
                site_name: row.get(3)?,
                updated_at: row.get(4)?,
                capture_count: row.get(5)?,
                kinds,
            })
        })
        .map_err(db_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
}

/// The file `key` (a stored relative path) of `source_id`, when it is a regular
/// file on disk. The key must be exactly `web-captures/<source_id>/<name>` with
/// a plain name, so a stored path can never point anywhere else, and the folder
/// must be a real one inside the captures root (never a link).
pub(super) fn capture_file(data_dir: &Path, source_id: &str, key: &str) -> Option<PathBuf> {
    let name = key.strip_prefix(&format!("{DIR}/{source_id}/"))?;
    let plain = !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':']);
    if !plain {
        return None;
    }
    match capture_files::locate_source_dir(data_dir, source_id) {
        Located::Dir(dir) => {
            let file = dir.join(name);
            fs::symlink_metadata(&file)
                .is_ok_and(|meta| meta.file_type().is_file())
                .then_some(file)
        }
        _ => None,
    }
}

fn file_is_present(data_dir: &Path, source_id: &str, key: &str) -> bool {
    capture_file(data_dir, source_id, key).is_some()
}

/// Whether `text` looks like a lower-case hex SHA-256.
fn is_digest(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The source that already holds a PDF capture with this sha256, if any. Anything
/// that is not a digest matches nothing.
pub fn source_of_pdf_sha(conn: &Connection, sha256: &str) -> Result<Option<String>, String> {
    if !is_digest(sha256) {
        return Ok(None);
    }
    conn.query_row(
        "SELECT web_source_id FROM web_captures
         WHERE kind = 'pdf' AND sha256 = ?1
         ORDER BY created_at, id LIMIT 1",
        [sha256],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_error)
}

/// The saved PDF file of capture `capture_id`, resolved on this side: the
/// renderer names a capture and never a path. The stored key goes through the
/// same checks as the file-presence report, so a key that leaves its source's
/// folder, a folder that is a link or a file that is not a regular file is never
/// returned. Reading it does not touch the network.
pub fn pdf_capture_file(
    conn: &Connection,
    data_dir: &Path,
    capture_id: &str,
) -> Result<PathBuf, String> {
    if !capture_files::valid_source_id(capture_id) {
        return Err(invalid_id());
    }
    let row: Option<(String, String, Option<String>)> = conn
        .query_row(
            "SELECT web_source_id, kind, rel_path FROM web_captures WHERE id = ?1",
            [capture_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(db_error)?;
    let Some((source_id, kind, rel_path)) = row else {
        return Err(format!("{}: there is no such capture", code::NOT_FOUND));
    };
    if kind != "pdf" {
        return Err(format!("{}: the capture is not a PDF", code::NOT_A_PDF));
    }
    rel_path
        .and_then(|key| capture_file(data_dir, &source_id, &key))
        .ok_or_else(|| format!("{}: the saved PDF is not on disk", code::FILE_MISSING))
}

/// Everything a copy of the capture `capture_id` needs. A page or a selection is
/// rendered into a PDF from its text ([`text_copy`]); the rest of this comment is
/// about a saved PDF.
///
/// Everything a copy of the PDF capture needs. The file goes through
/// the same checks as [`pdf_capture_file`] and, before a copy is allowed to
/// carry the capture's name, it is hashed again: bytes that no longer match the
/// sha256 recorded when they were verified are refused (`file_changed`), so a
/// copy's provenance never vouches for a file that is not the one that was saved.
pub fn copy_ticket(
    conn: &Connection,
    data_dir: &Path,
    capture_id: &str,
) -> Result<CopyTicket, String> {
    let file = match pdf_capture_file(conn, data_dir, capture_id) {
        Ok(file) => file,
        // A page or a selection has no file: its text is rendered into one.
        Err(error) if error.starts_with(code::NOT_A_PDF) => {
            return text_copy::copy_ticket(conn, data_dir, capture_id)
        }
        Err(error) => return Err(error),
    };
    let (source_id, original_url, final_url, source_title, capture_title, accessed_at, sha256): (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT s.id, s.original_url, s.final_url, s.title, c.title, c.accessed_at, c.sha256
             FROM web_captures c JOIN web_sources s ON s.id = c.web_source_id
             WHERE c.id = ?1",
            [capture_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .map_err(db_error)?;

    let actual = hash_file(&file).map_err(|error| {
        // A file that vanished between the check and the read is missing, not changed.
        if error.kind() == std::io::ErrorKind::NotFound {
            format!("{}: the saved PDF is not on disk", code::FILE_MISSING)
        } else {
            db_error(error)
        }
    })?;
    if actual != sha256 {
        return Err(format!(
            "{}: the saved PDF is not the one that was verified",
            code::FILE_CHANGED
        ));
    }

    let page_title = capture_title
        .filter(|title| !title.trim().is_empty())
        .or(source_title.filter(|title| !title.trim().is_empty()));
    Ok(CopyTicket {
        path: file.to_string_lossy().into_owned(),
        provenance: WebCaptureProvenance {
            source_id,
            capture_id: capture_id.to_string(),
            original_url,
            final_url,
            page_title,
            accessed_at,
            sha256,
            capture_kind: None,
            rendering: None,
        },
    })
}

/// The lower-case hex SHA-256 of a file, read in blocks.
fn hash_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut block = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut block)?;
        if read == 0 {
            break;
        }
        hasher.update(&block[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// A source with its captures; `None` when there is no such source.
pub fn source_detail(
    conn: &Connection,
    data_dir: &Path,
    source_id: &str,
) -> Result<Option<SourceDetail>, String> {
    if !capture_files::valid_source_id(source_id) {
        return Err(invalid_id());
    }
    let head = conn
        .query_row(
            "SELECT original_url, final_url, canonical_url, title, site_name,
                    first_accessed_at, created_at, updated_at
             FROM web_sources WHERE id = ?1",
            [source_id],
            |row| {
                Ok(SourceDetail {
                    id: source_id.to_string(),
                    original_url: row.get(0)?,
                    final_url: row.get(1)?,
                    canonical_url: row.get(2)?,
                    title: row.get(3)?,
                    site_name: row.get(4)?,
                    first_accessed_at: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    captures: Vec::new(),
                })
            },
        )
        .optional()
        .map_err(db_error)?;
    let Some(mut detail) = head else {
        return Ok(None);
    };

    let mut statement = conn
        .prepare(
            "SELECT id, kind, mime_type, accessed_at, final_url, title, sha256, hash_of,
                    size_bytes, substr(text, 1, ?2), text_rel_path IS NOT NULL,
                    quote_prefix, quote_suffix, rel_path, created_at
             FROM web_captures WHERE web_source_id = ?1
             ORDER BY accessed_at DESC, created_at DESC, id",
        )
        .map_err(db_error)?;
    let rows = statement
        .query_map(
            rusqlite::params![source_id, PREVIEW_MAX_CHARS as i64],
            |row| {
                let rel_path: Option<String> = row.get(13)?;
                let id: String = row.get(0)?;
                let file_pending = crate::sync::web_blobs::has_pending_download(conn, &id);
                Ok(CaptureDetail {
                    id,
                    kind: row.get(1)?,
                    mime_type: row.get(2)?,
                    accessed_at: row.get(3)?,
                    final_url: row.get(4)?,
                    title: row.get(5)?,
                    sha256: row.get(6)?,
                    hash_of: row.get(7)?,
                    size_bytes: row.get(8)?,
                    text_preview: row.get(9)?,
                    text_in_file: row.get(10)?,
                    quote_prefix: row.get(11)?,
                    quote_suffix: row.get(12)?,
                    file_present: rel_path
                        .as_deref()
                        .map(|key| file_is_present(data_dir, source_id, key)),
                    file_pending,
                    created_at: row.get(14)?,
                })
            },
        )
        .map_err(db_error)?;
    detail.captures = rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?;
    Ok(Some(detail))
}

/// Delete a source and its captures, then its folder.
pub fn delete_source(
    conn: &Connection,
    data_dir: &Path,
    source_id: &str,
) -> Result<DeleteOutcome, String> {
    if !capture_files::valid_source_id(source_id) {
        return Err(invalid_id());
    }
    {
        // IMMEDIATE, like a save: nothing changes the source under this delete.
        let tx =
            Transaction::new_unchecked(conn, TransactionBehavior::Immediate).map_err(db_error)?;
        let exists = tx
            .query_row(
                "SELECT 1 FROM web_sources WHERE id = ?1",
                [source_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(db_error)?;
        if exists.is_none() {
            return Err(format!("{}: there is no such source", code::NOT_FOUND));
        }
        // The captures go explicitly: the delete does not depend on the
        // connection enforcing foreign keys.
        tx.execute(
            "DELETE FROM web_captures WHERE web_source_id = ?1",
            [source_id],
        )
        .map_err(db_error)?;
        tx.execute("DELETE FROM web_sources WHERE id = ?1", [source_id])
            .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
    }
    // Its copies to Zotero are records of this source; the Zotero items stay. A
    // failure here never fails the delete.
    if let Err(error) = super::zotero_copy::store::forget_source(conn, source_id) {
        eprintln!("[navegador] source {source_id} deleted, its Zotero copy rows stayed: {error}");
    }

    // The rows are gone. The files follow, and nothing here fails the delete:
    // what stays is a folder with no source, which the startup sweep removes.
    let leftover_files = match capture_files::locate_source_dir(data_dir, source_id) {
        Located::Missing => false,
        Located::Dir(dir) => !capture_files::remove_dir_contents(&dir).dir_removed,
        Located::Refused(reason) => {
            eprintln!(
                "[navegador] source {source_id} deleted, its folder was not touched: {reason}"
            );
            true
        }
    };
    Ok(DeleteOutcome { leftover_files })
}

#[cfg(test)]
mod tests {
    use super::text_copy::COPY_DIR;
    use super::*;
    use crate::sync::test_support::new_app_schema_db;

    struct Env {
        data: tempfile::TempDir,
        conn: Connection,
    }

    struct NewSource<'a> {
        id: &'a str,
        title: Option<&'a str>,
        original: &'a str,
        final_url: &'a str,
        canonical: Option<&'a str>,
        updated_at: i64,
    }

    fn source<'a>(id: &'a str, final_url: &'a str, updated_at: i64) -> NewSource<'a> {
        NewSource {
            id,
            title: Some("A title"),
            original: final_url,
            final_url,
            canonical: None,
            updated_at,
        }
    }

    struct NewCapture<'a> {
        id: &'a str,
        source: &'a str,
        kind: &'a str,
        accessed_at: &'a str,
        text: Option<&'a str>,
        title: Option<&'a str>,
        rel_path: Option<&'a str>,
        text_rel_path: Option<&'a str>,
    }

    fn capture<'a>(
        id: &'a str,
        source: &'a str,
        kind: &'a str,
        accessed_at: &'a str,
    ) -> NewCapture<'a> {
        NewCapture {
            id,
            source,
            kind,
            accessed_at,
            text: None,
            title: None,
            rel_path: None,
            text_rel_path: None,
        }
    }

    impl Env {
        fn new() -> Self {
            Self {
                data: tempfile::tempdir().unwrap(),
                conn: new_app_schema_db(),
            }
        }

        fn add_source(&self, s: NewSource<'_>) {
            self.conn
                .execute(
                    "INSERT INTO web_sources
                       (id, original_url, final_url, canonical_url, title,
                        first_accessed_at, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, '2026-09-30T12:00:00Z', 1, ?6)",
                    rusqlite::params![
                        s.id,
                        s.original,
                        s.final_url,
                        s.canonical,
                        s.title,
                        s.updated_at
                    ],
                )
                .unwrap();
        }

        fn add_capture(&self, c: NewCapture<'_>) {
            let hash_of = match c.kind {
                "page" => "html",
                "selection" => "quote",
                _ => "pdf",
            };
            self.conn
                .execute(
                    "INSERT INTO web_captures
                       (id, web_source_id, accessed_at, final_url, kind, mime_type, text,
                        text_rel_path, rel_path, sha256, hash_of, size_bytes, title, created_at)
                     VALUES (?1, ?2, ?3, 'https://e.com/', ?4, 'text/html', ?5, ?6, ?7,
                             'abc', ?8, 10, ?9, 5)",
                    rusqlite::params![
                        c.id,
                        c.source,
                        c.accessed_at,
                        c.kind,
                        c.text,
                        c.text_rel_path,
                        c.rel_path,
                        hash_of,
                        c.title
                    ],
                )
                .unwrap();
        }

        fn file(&self, rel: &str) -> PathBuf {
            let path = self.data.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"x").unwrap();
            path
        }

        fn ids(&self, query: Option<&str>) -> Vec<String> {
            list_sources(&self.conn, query, DEFAULT_LIMIT)
                .unwrap()
                .into_iter()
                .map(|s| s.id)
                .collect()
        }

        fn count(&self, table: &str) -> i64 {
            self.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        }
    }

    // --- query helpers -------------------------------------------------------

    #[test]
    fn a_query_is_tried_as_typed_lower_upper_and_capitalised() {
        let variants = query_variants("educación");
        assert_eq!(variants[0], "educación");
        assert!(variants.contains(&"EDUCACIÓN".to_string()));
        assert!(variants.contains(&"Educación".to_string()));
        // No repeats, and an all-caps query does not multiply.
        let mut unique = variants.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), variants.len());
        assert_eq!(query_variants("123"), vec!["123".to_string()]);
    }

    #[test]
    fn wildcards_and_the_escape_character_are_made_literal() {
        assert_eq!(contains_pattern("100%"), "%100\\%%");
        assert_eq!(contains_pattern("a_b"), "%a\\_b%");
        assert_eq!(contains_pattern("c:\\x"), "%c:\\\\x%");
        assert_eq!(contains_pattern("plain"), "%plain%");
    }

    // --- list ----------------------------------------------------------------

    #[test]
    fn an_empty_archive_lists_nothing() {
        let env = Env::new();
        assert!(env.ids(None).is_empty());
    }

    #[test]
    fn sources_come_newest_first_with_their_capture_count_and_kinds() {
        let env = Env::new();
        env.add_source(source("old", "https://e.com/old", 100));
        env.add_source(source("new", "https://e.com/new", 300));
        env.add_source(source("mid", "https://e.com/mid", 200));
        env.add_capture(capture("c1", "new", "page", "2026-09-30T10:00:00Z"));
        env.add_capture(capture("c2", "new", "selection", "2026-09-30T11:00:00Z"));
        env.add_capture(capture("c3", "new", "selection", "2026-09-30T12:00:00Z"));
        env.add_capture(capture("c4", "mid", "pdf", "2026-09-30T12:00:00Z"));

        let list = list_sources(&env.conn, None, DEFAULT_LIMIT).unwrap();

        let ids: Vec<_> = list.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["new", "mid", "old"]);
        assert_eq!(list[0].capture_count, 3);
        assert_eq!(list[0].kinds, ["page", "selection"]);
        assert_eq!(list[1].kinds, ["pdf"]);
        assert_eq!(list[2].capture_count, 0);
        assert!(list[2].kinds.is_empty());
        assert_eq!(list[0].final_url, "https://e.com/new");
        assert_eq!(list[0].updated_at, 300);
    }

    #[test]
    fn the_limit_cuts_the_list_and_is_itself_bounded() {
        let env = Env::new();
        for n in 0..5 {
            env.add_source(source(&format!("s{n}"), &format!("https://e.com/{n}"), n));
        }
        assert_eq!(list_sources(&env.conn, None, 2).unwrap().len(), 2);
        assert_eq!(list_sources(&env.conn, None, 0).unwrap().len(), 5);
        assert_eq!(list_sources(&env.conn, None, 10_000).unwrap().len(), 5);
    }

    #[test]
    fn a_blank_query_lists_everything() {
        let env = Env::new();
        env.add_source(source("a", "https://e.com/a", 1));
        assert_eq!(env.ids(Some("   ")), ["a"]);
        assert_eq!(env.ids(Some("")), ["a"]);
    }

    #[test]
    fn search_finds_by_title_and_by_every_url() {
        let env = Env::new();
        env.add_source(NewSource {
            title: Some("Historia del trabajo"),
            ..source("by-title", "https://e.com/1", 1)
        });
        env.add_source(NewSource {
            original: "https://original.example/start",
            ..source("by-original", "https://e.com/2", 2)
        });
        env.add_source(source("by-final", "https://final.example/page", 3));
        env.add_source(NewSource {
            canonical: Some("https://canon.example/p"),
            ..source("by-canonical", "https://e.com/4", 4)
        });
        env.add_source(source("none", "https://e.com/5", 5));

        assert_eq!(env.ids(Some("trabajo")), ["by-title"]);
        assert_eq!(env.ids(Some("original.example")), ["by-original"]);
        assert_eq!(env.ids(Some("final.example")), ["by-final"]);
        assert_eq!(env.ids(Some("canon.example")), ["by-canonical"]);
        assert!(env.ids(Some("nowhere-to-be-found")).is_empty());
    }

    #[test]
    fn search_finds_by_the_text_a_capture_kept_and_lists_the_source_once() {
        let env = Env::new();
        env.add_source(source("a", "https://e.com/a", 1));
        env.add_source(source("b", "https://e.com/b", 2));
        env.add_capture(NewCapture {
            text: Some("the unique phrase in a page"),
            ..capture("c1", "a", "page", "2026-09-30T10:00:00Z")
        });
        env.add_capture(NewCapture {
            text: Some("the unique phrase again in a quote"),
            ..capture("c2", "a", "selection", "2026-09-30T11:00:00Z")
        });
        env.add_capture(NewCapture {
            title: Some("A capture title: zeta"),
            ..capture("c3", "b", "page", "2026-09-30T11:00:00Z")
        });

        assert_eq!(env.ids(Some("unique phrase")), ["a"]);
        assert_eq!(env.ids(Some("zeta")), ["b"]);
    }

    #[test]
    fn search_ignores_case_including_accents() {
        let env = Env::new();
        env.add_source(NewSource {
            title: Some("EDUCACIÓN pública"),
            ..source("upper", "https://e.com/1", 1)
        });
        env.add_source(NewSource {
            title: Some("Educación popular"),
            ..source("capital", "https://e.com/2", 2)
        });
        env.add_source(NewSource {
            title: Some("Other"),
            ..source("other", "https://e.com/3", 3)
        });

        let mut found = env.ids(Some("educación"));
        found.sort();
        assert_eq!(found, ["capital", "upper"]);
        assert_eq!(env.ids(Some("OTHER")), ["other"]);
        assert_eq!(env.ids(Some("other")), ["other"]);
    }

    #[test]
    fn percent_underscore_and_backslash_in_a_query_match_themselves() {
        let env = Env::new();
        env.add_source(NewSource {
            title: Some("Up 100% this year"),
            ..source("percent", "https://e.com/1", 1)
        });
        env.add_source(NewSource {
            title: Some("Up 1000 this year"),
            ..source("digits", "https://e.com/2", 2)
        });
        env.add_source(NewSource {
            title: Some("file a_b here"),
            ..source("under", "https://e.com/3", 3)
        });
        env.add_source(NewSource {
            title: Some("file axb here"),
            ..source("any", "https://e.com/4", 4)
        });
        env.add_source(NewSource {
            title: Some("path c:\\temp"),
            ..source("slash", "https://e.com/5", 5)
        });

        assert_eq!(env.ids(Some("100%")), ["percent"]);
        assert_eq!(env.ids(Some("a_b")), ["under"]);
        assert_eq!(env.ids(Some("c:\\t")), ["slash"]);
        assert!(
            env.ids(Some("%")).len() == 1,
            "a bare % matches only a literal %"
        );
        assert!(
            env.ids(Some("_")).len() == 1,
            "a bare _ matches only a literal _"
        );
    }

    #[test]
    fn a_quote_in_a_query_is_data_not_sql() {
        let env = Env::new();
        env.add_source(source("a", "https://e.com/a", 1));
        assert!(env.ids(Some("x'; DROP TABLE web_sources; --")).is_empty());
        assert_eq!(env.count("web_sources"), 1);
    }

    #[test]
    fn a_very_long_query_is_cut_not_refused() {
        let env = Env::new();
        env.add_source(source("a", "https://e.com/a", 1));
        let long = "x".repeat(QUERY_MAX_CHARS * 10);
        assert!(list_sources(&env.conn, Some(&long), DEFAULT_LIMIT)
            .unwrap()
            .is_empty());
    }

    // --- detail --------------------------------------------------------------

    #[test]
    fn the_detail_lists_captures_newest_first_with_what_was_recorded() {
        let env = Env::new();
        env.add_source(NewSource {
            canonical: Some("https://e.com/canon"),
            ..source("s", "https://e.com/s", 7)
        });
        env.add_capture(capture("older", "s", "page", "2026-09-29T08:00:00Z"));
        env.add_capture(NewCapture {
            text: Some("quoted words"),
            ..capture("newer", "s", "selection", "2026-09-30T08:00:00Z")
        });

        let detail = source_detail(&env.conn, env.data.path(), "s")
            .unwrap()
            .unwrap();

        assert_eq!(detail.id, "s");
        assert_eq!(detail.original_url, "https://e.com/s");
        assert_eq!(detail.canonical_url.as_deref(), Some("https://e.com/canon"));
        assert_eq!(detail.first_accessed_at, "2026-09-30T12:00:00Z");
        assert_eq!(detail.updated_at, 7);
        let ids: Vec<_> = detail.captures.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["newer", "older"]);
        let newer = &detail.captures[0];
        assert_eq!(newer.kind, "selection");
        assert_eq!(newer.accessed_at, "2026-09-30T08:00:00Z");
        assert_eq!(newer.hash_of, "quote");
        assert_eq!(newer.sha256, "abc");
        assert_eq!(newer.size_bytes, 10);
        assert_eq!(newer.text_preview.as_deref(), Some("quoted words"));
        assert!(!newer.text_in_file);
    }

    #[test]
    fn the_preview_is_cut_on_characters_and_a_text_file_is_flagged() {
        let env = Env::new();
        env.add_source(source("s", "https://e.com/s", 1));
        let long = "ñ".repeat(PREVIEW_MAX_CHARS + 50);
        env.add_capture(NewCapture {
            text: Some(&long),
            ..capture("long", "s", "page", "2026-09-30T08:00:00Z")
        });
        env.add_capture(NewCapture {
            text_rel_path: Some("web-captures/s/big.txt"),
            ..capture("big", "s", "page", "2026-09-29T08:00:00Z")
        });

        let detail = source_detail(&env.conn, env.data.path(), "s")
            .unwrap()
            .unwrap();

        let long = &detail.captures[0];
        assert_eq!(
            long.text_preview.as_ref().unwrap().chars().count(),
            PREVIEW_MAX_CHARS
        );
        let big = &detail.captures[1];
        assert!(big.text_in_file);
        assert_eq!(big.text_preview, None);
    }

    #[test]
    fn the_quote_context_comes_with_a_selection() {
        let env = Env::new();
        env.add_source(source("s", "https://e.com/s", 1));
        env.add_capture(capture("c", "s", "selection", "2026-09-30T08:00:00Z"));
        env.conn
            .execute(
                "UPDATE web_captures SET quote_prefix = 'before ', quote_suffix = ' after'",
                [],
            )
            .unwrap();

        let detail = source_detail(&env.conn, env.data.path(), "s")
            .unwrap()
            .unwrap();

        assert_eq!(detail.captures[0].quote_prefix.as_deref(), Some("before "));
        assert_eq!(detail.captures[0].quote_suffix.as_deref(), Some(" after"));
    }

    #[test]
    fn file_presence_is_reported_for_what_is_on_disk_and_what_is_not() {
        let env = Env::new();
        env.add_source(source("s", "https://e.com/s", 1));
        env.file("web-captures/s/here.html");
        env.add_capture(NewCapture {
            rel_path: Some("web-captures/s/here.html"),
            ..capture("here", "s", "page", "2026-09-30T03:00:00Z")
        });
        env.add_capture(NewCapture {
            rel_path: Some("web-captures/s/gone.pdf"),
            ..capture("gone", "s", "pdf", "2026-09-30T02:00:00Z")
        });
        env.add_capture(capture("none", "s", "selection", "2026-09-30T01:00:00Z"));

        let detail = source_detail(&env.conn, env.data.path(), "s")
            .unwrap()
            .unwrap();

        let presence: Vec<_> = detail.captures.iter().map(|c| c.file_present).collect();
        assert_eq!(presence, [Some(true), Some(false), None]);
    }

    #[test]
    fn a_file_still_being_downloaded_by_sync_is_reported_as_pending() {
        let env = Env::new();
        crate::sync::schema::ensure_sync_schema(&env.conn).unwrap();
        env.add_source(source("s", "https://e.com/s", 1));
        env.add_capture(NewCapture {
            rel_path: Some("web-captures/s/late.pdf"),
            ..capture("late", "s", "pdf", "2026-09-30T03:00:00Z")
        });
        env.add_capture(NewCapture {
            rel_path: Some("web-captures/s/lost.pdf"),
            ..capture("lost", "s", "pdf", "2026-09-30T02:00:00Z")
        });
        env.conn
            .execute(
                "INSERT INTO sync_web_pending_blobs(capture_id, role, sha256, rel_path, size)
                 VALUES ('late', 'file', 'abc', 'web-captures/s/late.pdf', 10)",
                [],
            )
            .unwrap();

        let detail = source_detail(&env.conn, env.data.path(), "s")
            .unwrap()
            .unwrap();

        let by_id = |id: &str| detail.captures.iter().find(|c| c.id == id).unwrap();
        assert_eq!(by_id("late").file_present, Some(false));
        assert!(by_id("late").file_pending, "queued for download");
        assert!(!by_id("lost").file_pending, "nothing queued: just missing");
    }

    #[test]
    fn pending_is_false_when_the_sync_tables_do_not_exist() {
        let env = Env::new();
        env.add_source(source("s", "https://e.com/s", 1));
        env.add_capture(NewCapture {
            rel_path: Some("web-captures/s/x.pdf"),
            ..capture("x", "s", "pdf", "2026-09-30T03:00:00Z")
        });
        let detail = source_detail(&env.conn, env.data.path(), "s")
            .unwrap()
            .unwrap();
        assert!(!detail.captures[0].file_pending);
    }

    #[test]
    fn a_stored_path_that_leaves_its_folder_never_counts_as_present() {
        let env = Env::new();
        env.add_source(source("s", "https://e.com/s", 1));
        env.add_source(source("other", "https://e.com/o", 2));
        env.file("web-captures/other/theirs.html");
        env.file("outside.html");
        let outside = env.data.path().join("outside.html");
        for (n, rel) in [
            "web-captures/s/../other/theirs.html",
            "web-captures/other/theirs.html",
            "../outside.html",
            "web-captures/s/..\\..\\outside.html",
            "outside.html",
        ]
        .into_iter()
        .enumerate()
        {
            env.add_capture(NewCapture {
                rel_path: Some(rel),
                ..capture(
                    &format!("c{n}"),
                    "s",
                    "page",
                    &format!("2026-09-30T0{n}:00:00Z"),
                )
            });
        }
        let absolute = outside.to_string_lossy().into_owned();
        env.add_capture(NewCapture {
            rel_path: Some(&absolute),
            ..capture("abs", "s", "page", "2026-09-30T09:00:00Z")
        });

        let detail = source_detail(&env.conn, env.data.path(), "s")
            .unwrap()
            .unwrap();

        assert_eq!(detail.captures.len(), 6);
        assert!(detail
            .captures
            .iter()
            .all(|c| c.file_present == Some(false)));
    }

    #[test]
    fn a_missing_source_is_none_and_a_bad_id_is_refused() {
        let env = Env::new();
        assert_eq!(
            source_detail(&env.conn, env.data.path(), "nope").unwrap(),
            None
        );
        for bad in ["", "../x", "a/b", "a b"] {
            let error = source_detail(&env.conn, env.data.path(), bad).unwrap_err();
            assert!(error.starts_with(code::INVALID_ID), "{error}");
        }
    }

    // --- delete --------------------------------------------------------------

    #[test]
    fn deleting_removes_the_rows_the_captures_and_the_folder() {
        let env = Env::new();
        env.add_source(source("gone", "https://e.com/g", 1));
        env.add_source(source("kept", "https://e.com/k", 2));
        env.add_capture(NewCapture {
            rel_path: Some("web-captures/gone/c1.html"),
            ..capture("c1", "gone", "page", "2026-09-30T10:00:00Z")
        });
        env.add_capture(capture("c2", "gone", "selection", "2026-09-30T11:00:00Z"));
        env.add_capture(NewCapture {
            rel_path: Some("web-captures/kept/c3.html"),
            ..capture("c3", "kept", "page", "2026-09-30T12:00:00Z")
        });
        env.file("web-captures/gone/c1.html");
        let kept_file = env.file("web-captures/kept/c3.html");

        let outcome = delete_source(&env.conn, env.data.path(), "gone").unwrap();

        assert_eq!(
            outcome,
            DeleteOutcome {
                leftover_files: false
            }
        );
        assert_eq!(env.ids(None), ["kept"]);
        assert_eq!(env.count("web_captures"), 1);
        assert!(!env.data.path().join("web-captures/gone").exists());
        assert!(kept_file.exists(), "another source's files stay");
    }

    #[test]
    fn the_captures_go_even_when_the_connection_does_not_enforce_foreign_keys() {
        let env = Env::new();
        env.conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        env.add_source(source("gone", "https://e.com/g", 1));
        env.add_capture(capture("c1", "gone", "selection", "2026-09-30T10:00:00Z"));

        delete_source(&env.conn, env.data.path(), "gone").unwrap();

        assert_eq!(env.count("web_captures"), 0);
    }

    #[test]
    fn a_source_with_no_folder_deletes_fine() {
        let env = Env::new();
        env.add_source(source("only-selections", "https://e.com/s", 1));
        env.add_capture(capture(
            "c1",
            "only-selections",
            "selection",
            "2026-09-30T10:00:00Z",
        ));

        let outcome = delete_source(&env.conn, env.data.path(), "only-selections").unwrap();

        assert!(!outcome.leftover_files);
        assert_eq!(env.count("web_sources"), 0);
    }

    #[test]
    fn an_unknown_or_malformed_id_changes_nothing() {
        let env = Env::new();
        env.add_source(source("a", "https://e.com/a", 1));
        let folder = env.file("web-captures/a/x.html");

        let unknown = delete_source(&env.conn, env.data.path(), "nope").unwrap_err();
        assert!(unknown.starts_with(code::NOT_FOUND), "{unknown}");
        for bad in ["", "..", "../a", "a/../a", "a\\b"] {
            let error = delete_source(&env.conn, env.data.path(), bad).unwrap_err();
            assert!(error.starts_with(code::INVALID_ID), "{bad:?}: {error}");
        }
        assert_eq!(env.count("web_sources"), 1);
        assert!(folder.exists());
    }

    #[test]
    fn a_database_failure_leaves_the_files_alone() {
        let env = Env::new();
        env.add_source(source("a", "https://e.com/a", 1));
        let file = env.file("web-captures/a/x.html");
        env.conn.execute_batch("DROP TABLE web_captures").unwrap();

        let error = delete_source(&env.conn, env.data.path(), "a").unwrap_err();

        assert!(error.starts_with(code::DB_ERROR), "{error}");
        assert!(file.exists(), "files go only after the rows are gone");
        assert_eq!(env.count("web_sources"), 1);
    }

    #[test]
    fn a_folder_that_cannot_be_emptied_is_reported_but_never_fails_the_delete() {
        let env = Env::new();
        env.add_source(source("a", "https://e.com/a", 1));
        env.file("web-captures/a/x.html");
        fs::create_dir_all(env.data.path().join("web-captures/a/inner")).unwrap();

        let outcome = delete_source(&env.conn, env.data.path(), "a").unwrap();

        assert!(outcome.leftover_files);
        assert_eq!(env.count("web_sources"), 0);
        assert!(!env.data.path().join("web-captures/a/x.html").exists());
    }

    #[test]
    fn a_leftover_folder_of_a_deleted_source_is_what_the_sweep_removes() {
        let env = Env::new();
        env.add_source(source("a", "https://e.com/a", 1));
        env.file("web-captures/a/x.html");
        // The rows are gone but the folder stayed (what a failed removal leaves).
        env.conn.execute("DELETE FROM web_sources", []).unwrap();

        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2 * 3600);
        let report = super::super::sweep::sweep(
            env.data.path(),
            &env.conn,
            later,
            super::super::sweep::MIN_AGE,
        );

        assert_eq!(report.ghost_dirs, 1);
        assert!(!env.data.path().join("web-captures/a").exists());
    }

    #[test]
    fn a_source_folder_that_is_a_link_is_refused_and_what_it_points_at_survives() {
        let env = Env::new();
        env.add_source(source("a", "https://e.com/a", 1));
        let outside = tempfile::tempdir().unwrap();
        let precious = outside.path().join("precious.html");
        fs::write(&precious, b"x").unwrap();
        fs::create_dir_all(env.data.path().join("web-captures")).unwrap();
        let link = env.data.path().join("web-captures/a");
        let made = {
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_dir(outside.path(), &link).is_ok()
                    || std::process::Command::new("cmd")
                        .args(["/C", "mklink", "/J"])
                        .arg(&link)
                        .arg(outside.path())
                        .output()
                        .map(|out| out.status.success())
                        .unwrap_or(false)
            }
            #[cfg(not(windows))]
            {
                std::os::unix::fs::symlink(outside.path(), &link).is_ok()
            }
        };
        if !made {
            eprintln!("skipped: this machine cannot create a directory link");
            return;
        }

        let outcome = delete_source(&env.conn, env.data.path(), "a").unwrap();

        assert!(outcome.leftover_files);
        assert!(precious.exists(), "a file behind a link was deleted");
        assert_eq!(env.count("web_sources"), 0);
    }

    // --- PDFs: lookup by hash and the file behind a capture -------------------

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    impl Env {
        fn add_pdf(&self, id: &str, source: &str, sha: &str, rel_path: Option<&str>) {
            self.conn
                .execute(
                    "INSERT INTO web_captures
                       (id, web_source_id, accessed_at, final_url, kind, mime_type, rel_path,
                        sha256, hash_of, size_bytes, created_at)
                     VALUES (?1, ?2, '2026-09-30T12:00:00Z', 'https://e.com/f.pdf', 'pdf',
                             'application/pdf', ?3, ?4, 'pdf', 10, 5)",
                    rusqlite::params![id, source, rel_path, sha],
                )
                .unwrap();
        }
    }

    #[test]
    fn a_pdf_hash_finds_the_source_that_already_holds_it() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/one", 1));
        env.add_source(source("s2", "https://e.com/two", 2));
        env.add_pdf("c1", "s1", SHA_A, None);
        env.add_pdf("c2", "s2", SHA_B, None);

        assert_eq!(
            source_of_pdf_sha(&env.conn, SHA_A).unwrap().as_deref(),
            Some("s1")
        );
        assert_eq!(
            source_of_pdf_sha(&env.conn, SHA_B).unwrap().as_deref(),
            Some("s2")
        );
        assert_eq!(source_of_pdf_sha(&env.conn, &"c".repeat(64)).unwrap(), None);
    }

    #[test]
    fn only_a_pdf_capture_counts_for_a_hash_not_a_page_or_a_quote_with_the_same_digest() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/one", 1));
        env.conn
            .execute(
                "INSERT INTO web_captures
                   (id, web_source_id, accessed_at, final_url, kind, mime_type, sha256,
                    hash_of, size_bytes, created_at)
                 VALUES ('p', 's1', '2026-09-30T12:00:00Z', 'https://e.com/', 'page',
                         'text/html', ?1, 'html', 10, 5)",
                [SHA_A],
            )
            .unwrap();

        assert_eq!(source_of_pdf_sha(&env.conn, SHA_A).unwrap(), None);
    }

    #[test]
    fn a_hash_that_is_not_a_digest_matches_nothing() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/one", 1));
        env.add_pdf("c1", "s1", SHA_A, None);

        for bad in ["", "abc", "%", "' OR 1=1 --", &SHA_A.to_uppercase()] {
            assert_eq!(source_of_pdf_sha(&env.conn, bad).unwrap(), None, "{bad}");
        }
    }

    #[test]
    fn a_saved_pdf_is_found_by_its_capture_id_and_never_by_a_path() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/one", 1));
        let file = env.file("web-captures/s1/c1.pdf");
        env.add_pdf("c1", "s1", SHA_A, Some("web-captures/s1/c1.pdf"));

        let found = pdf_capture_file(&env.conn, env.data.path(), "c1").unwrap();

        assert_eq!(found, file);
        for bad in ["", "../c1", "a/b", "c1.pdf", "C:\\x"] {
            let error = pdf_capture_file(&env.conn, env.data.path(), bad).unwrap_err();
            assert!(
                error.starts_with(code::INVALID_ID) || error.starts_with(code::NOT_FOUND),
                "{bad}: {error}"
            );
        }
    }

    #[test]
    fn an_unknown_capture_or_one_that_is_not_a_pdf_is_refused_with_its_own_code() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/one", 1));
        env.file("web-captures/s1/h.html");
        env.add_capture(NewCapture {
            rel_path: Some("web-captures/s1/h.html"),
            ..capture("h", "s1", "page", "2026-09-30T03:00:00Z")
        });

        let unknown = pdf_capture_file(&env.conn, env.data.path(), "nope").unwrap_err();
        let page = pdf_capture_file(&env.conn, env.data.path(), "h").unwrap_err();

        assert!(unknown.starts_with(code::NOT_FOUND), "{unknown}");
        assert!(page.starts_with(code::NOT_A_PDF), "{page}");
    }

    #[test]
    fn a_pdf_whose_file_is_gone_says_so() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/one", 1));
        env.add_pdf("c1", "s1", SHA_A, Some("web-captures/s1/c1.pdf"));
        env.add_pdf("c2", "s1", SHA_B, None);

        for id in ["c1", "c2"] {
            let error = pdf_capture_file(&env.conn, env.data.path(), id).unwrap_err();
            assert!(error.starts_with(code::FILE_MISSING), "{id}: {error}");
        }
    }

    #[test]
    fn a_stored_path_that_leaves_the_sources_folder_is_never_returned() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/one", 1));
        env.add_source(source("s2", "https://e.com/two", 2));
        env.file("web-captures/s2/theirs.pdf");
        env.file("outside.pdf");
        let outside = env.data.path().join("outside.pdf");
        let absolute = outside.to_string_lossy().into_owned();
        let cases = [
            "web-captures/s1/../s2/theirs.pdf",
            "web-captures/s2/theirs.pdf",
            "../outside.pdf",
            "web-captures/s1/..\\..\\outside.pdf",
            "outside.pdf",
            absolute.as_str(),
        ];
        for (n, rel) in cases.into_iter().enumerate() {
            env.add_pdf(&format!("c{n}"), "s1", &format!("{n:0>64}"), Some(rel));
        }

        for n in 0..cases.len() {
            let result = pdf_capture_file(&env.conn, env.data.path(), &format!("c{n}"));
            assert!(
                result
                    .as_ref()
                    .is_err_and(|e| e.starts_with(code::FILE_MISSING)),
                "case {n} returned {result:?}"
            );
        }
    }

    #[test]
    fn a_source_folder_that_is_a_link_is_not_followed_to_find_a_pdf() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/one", 1));
        let elsewhere = tempfile::tempdir().unwrap();
        fs::write(elsewhere.path().join("c1.pdf"), b"x").unwrap();
        fs::create_dir_all(env.data.path().join("web-captures")).unwrap();
        let link = env.data.path().join("web-captures/s1");
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(elsewhere.path(), &link).is_ok();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(elsewhere.path(), &link).is_ok();
        if !linked {
            eprintln!("skipped: this machine cannot create a directory link");
            return;
        }
        env.add_pdf("c1", "s1", SHA_A, Some("web-captures/s1/c1.pdf"));

        let error = pdf_capture_file(&env.conn, env.data.path(), "c1").unwrap_err();

        assert!(error.starts_with(code::FILE_MISSING), "{error}");
    }

    // --- copy ----------------------------------------------------------------

    fn digest(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(bytes))
    }

    impl Env {
        /// A saved PDF whose recorded hash is the real one of `bytes`.
        fn add_real_pdf(&self, id: &str, source: &str, bytes: &[u8], title: Option<&str>) {
            let rel = format!("web-captures/{source}/{id}.pdf");
            let path = self.data.path().join(&rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            self.add_pdf(id, source, &digest(bytes), Some(&rel));
            self.conn
                .execute(
                    "UPDATE web_captures SET title = ?2 WHERE id = ?1",
                    rusqlite::params![id, title],
                )
                .unwrap();
        }
    }

    #[test]
    fn a_copy_ticket_names_the_file_and_carries_the_provenance_of_the_capture() {
        let env = Env::new();
        env.add_source(NewSource {
            title: Some("Source title"),
            original: "https://e.com/article",
            final_url: "https://e.com/article?ref=1",
            ..source("s1", "https://e.com/article", 1)
        });
        let bytes = b"%PDF-1.7 a saved pdf";
        env.add_real_pdf("c1", "s1", bytes, Some("The PDF title"));

        let ticket = copy_ticket(&env.conn, env.data.path(), "c1").unwrap();

        let file = env.data.path().join("web-captures/s1/c1.pdf");
        assert_eq!(PathBuf::from(&ticket.path), file);
        assert_eq!(
            ticket.provenance,
            WebCaptureProvenance {
                source_id: "s1".into(),
                capture_id: "c1".into(),
                original_url: "https://e.com/article".into(),
                final_url: "https://e.com/article?ref=1".into(),
                page_title: Some("The PDF title".into()),
                accessed_at: "2026-09-30T12:00:00Z".into(),
                sha256: digest(bytes),
                capture_kind: None,
                rendering: None,
            }
        );
    }

    #[test]
    fn a_capture_with_no_title_of_its_own_takes_the_title_of_its_source() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/a", 1));
        env.add_real_pdf("c1", "s1", b"%PDF-1.7 one", None);

        let ticket = copy_ticket(&env.conn, env.data.path(), "c1").unwrap();

        assert_eq!(ticket.provenance.page_title.as_deref(), Some("A title"));
    }

    #[test]
    fn a_pdf_that_is_no_longer_the_one_that_was_verified_is_refused() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/a", 1));
        env.add_real_pdf("c1", "s1", b"%PDF-1.7 original", None);
        fs::write(
            env.data.path().join("web-captures/s1/c1.pdf"),
            b"%PDF-1.7 tampered",
        )
        .unwrap();

        let error = copy_ticket(&env.conn, env.data.path(), "c1").unwrap_err();

        assert!(error.starts_with(code::FILE_CHANGED), "{error}");
    }

    #[test]
    fn a_copy_is_refused_for_the_same_reasons_as_viewing_the_pdf() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/a", 1));
        env.file("web-captures/s1/h.html");
        env.add_capture(NewCapture {
            rel_path: Some("web-captures/s1/h.html"),
            ..capture("h", "s1", "page", "2026-09-30T03:00:00Z")
        });
        env.add_pdf("gone", "s1", SHA_A, Some("web-captures/s1/gone.pdf"));

        let cases = [
            ("../c1", code::INVALID_ID),
            ("nope", code::NOT_FOUND),
            ("h", code::NO_TEXT),
            ("gone", code::FILE_MISSING),
        ];
        for (id, expected) in cases {
            let error = copy_ticket(&env.conn, env.data.path(), id).unwrap_err();
            assert!(error.starts_with(expected), "{id}: {error}");
        }
    }

    #[test]
    fn deleting_a_source_leaves_a_copy_made_in_a_collection_alone() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/a", 1));
        let bytes = b"%PDF-1.7 shared bytes";
        env.add_real_pdf("c1", "s1", bytes, None);
        let ticket = copy_ticket(&env.conn, env.data.path(), "c1").unwrap();
        // What the import does: a new file under assets/, never a link or a move.
        let copy = env.data.path().join("assets/col1/item1/uuid_a.pdf");
        fs::create_dir_all(copy.parent().unwrap()).unwrap();
        fs::copy(&ticket.path, &copy).unwrap();

        delete_source(&env.conn, env.data.path(), "s1").unwrap();

        assert!(!env.data.path().join("web-captures/s1").exists());
        assert_eq!(fs::read(&copy).unwrap(), bytes);
        assert_eq!(env.count("web_captures"), 0);
    }

    #[test]
    fn deleting_a_source_drops_its_zotero_copy_rows_and_only_its_own() {
        use super::super::zotero_copy::store::{self, LibraryRef};
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/a", 1));
        env.add_source(source("s2", "https://e.com/b", 2));
        let library = LibraryRef {
            library_type: "user".into(),
            library_id: "0".into(),
            library_name: None,
        };
        store::request(&env.conn, "s1", None, &library).unwrap();
        store::request(&env.conn, "s2", None, &library).unwrap();

        delete_source(&env.conn, env.data.path(), "s1").unwrap();

        let left = store::list(&env.conn, None).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].source_id, "s2");
    }

    // --- copy of a page or a selection, rendered as a PDF ----------------------

    impl Env {
        /// A page or selection capture with the sha256 the caller says it has.
        fn add_text_capture(&self, c: NewCapture<'_>, sha: &str) {
            let id = c.id;
            self.add_capture(c);
            self.conn
                .execute(
                    "UPDATE web_captures SET sha256 = ?2 WHERE id = ?1",
                    rusqlite::params![id, sha],
                )
                .unwrap();
        }

        fn set_context(&self, id: &str, prefix: &str, suffix: &str) {
            self.conn
                .execute(
                    "UPDATE web_captures SET quote_prefix = ?2, quote_suffix = ?3 WHERE id = ?1",
                    rusqlite::params![id, prefix, suffix],
                )
                .unwrap();
        }
    }

    fn pdf_text(path: &str) -> String {
        let bytes = fs::read(path).expect("the rendered pdf exists");
        let text = pdf_extract::extract_text_from_mem(&bytes).expect("a readable text layer");
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn page_env(text: Option<&str>) -> Env {
        let env = Env::new();
        env.add_source(NewSource {
            title: Some("Source title"),
            original: "https://e.com/nota",
            final_url: "https://e.com/nota?ref=1",
            ..source("s1", "https://e.com/nota", 1)
        });
        env.add_text_capture(
            NewCapture {
                text,
                title: Some("La educación en Córdoba"),
                ..capture("p1", "s1", "page", "2026-09-30T03:00:00Z")
            },
            SHA_A,
        );
        env
    }

    #[test]
    fn a_page_copy_renders_its_text_into_a_pdf_with_the_provenance_header() {
        let env = page_env(Some(
            "¿Qué pasó con la educación?\n\nLa ñandú corrió — “rápido”.",
        ));

        let ticket = copy_ticket(&env.conn, env.data.path(), "p1").unwrap();

        let folder = env.data.path().join("web-captures").join(COPY_DIR);
        assert_eq!(PathBuf::from(&ticket.path).parent().unwrap(), folder);
        assert!(ticket.path.ends_with(".pdf"), "{}", ticket.path);
        let text = pdf_text(&ticket.path);
        assert!(text.contains("La educación en Córdoba"), "{text}");
        assert!(text.contains("https://e.com/nota?ref=1"), "{text}");
        assert!(
            text.contains("Consultada (UTC): 2026-09-30T03:00:00Z"),
            "{text}"
        );
        assert!(text.contains("Página"), "{text}");
        assert!(text.contains(SHA_A), "{text}");
        assert!(text.contains("¿Qué pasó con la educación?"), "{text}");
        assert!(text.contains("La ñandú corrió — “rápido”."), "{text}");
        assert_eq!(ticket.provenance.rendering.as_deref(), Some("text-pdf"));
        assert_eq!(ticket.provenance.capture_kind.as_deref(), Some("page"));
        assert_eq!(ticket.provenance.capture_id, "p1");
        assert_eq!(ticket.provenance.sha256, SHA_A);
        assert_eq!(
            ticket.provenance.page_title.as_deref(),
            Some("La educación en Córdoba")
        );
    }

    #[test]
    fn a_page_whose_text_is_in_a_file_is_read_from_the_file() {
        let env = page_env(None);
        let rel = "web-captures/s1/p1.txt";
        let path = env.data.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let big = format!("inicio {} final", "palabra ".repeat(20_000));
        fs::write(&path, &big).unwrap();
        env.conn
            .execute(
                "UPDATE web_captures SET text_rel_path = ?1 WHERE id = 'p1'",
                [rel],
            )
            .unwrap();

        let ticket = copy_ticket(&env.conn, env.data.path(), "p1").unwrap();

        let text = pdf_text(&ticket.path);
        assert!(text.contains("inicio palabra"), "{}", &text[..200]);
        assert!(
            text.ends_with("palabra final"),
            "{}",
            &text[text.len() - 100..]
        );
        let pages = lopdf::Document::load_mem(&fs::read(&ticket.path).unwrap())
            .unwrap()
            .get_pages()
            .len();
        assert!(pages > 5, "{pages} pages");
    }

    #[test]
    fn a_selection_copy_sets_the_quote_between_its_context_and_checks_the_quote_hash() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/nota", 1));
        let quote = "la cita exacta, con acentos: canción";
        env.add_text_capture(
            NewCapture {
                text: Some(quote),
                ..capture("q1", "s1", "selection", "2026-09-30T04:00:00Z")
            },
            &digest(quote.as_bytes()),
        );
        env.set_context("q1", "texto previo ", " texto posterior");

        let ticket = copy_ticket(&env.conn, env.data.path(), "q1").unwrap();

        let text = pdf_text(&ticket.path);
        let before = text.find("texto previo").expect("prefix");
        let at = text.find(quote).expect("the exact quote");
        let after = text.find("texto posterior").expect("suffix");
        assert!(before < at && at < after, "{text}");
        assert!(text.contains("Selección"), "{text}");
        assert_eq!(ticket.provenance.capture_kind.as_deref(), Some("selection"));
        assert_eq!(ticket.provenance.rendering.as_deref(), Some("text-pdf"));
        assert_eq!(ticket.provenance.sha256, digest(quote.as_bytes()));
    }

    #[test]
    fn a_selection_whose_text_no_longer_matches_its_hash_is_refused() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/nota", 1));
        env.add_text_capture(
            NewCapture {
                text: Some("an edited quote"),
                ..capture("q1", "s1", "selection", "2026-09-30T04:00:00Z")
            },
            &digest(b"the quote that was saved"),
        );

        let error = copy_ticket(&env.conn, env.data.path(), "q1").unwrap_err();

        assert!(error.starts_with(code::FILE_CHANGED), "{error}");
        assert!(!env.data.path().join("web-captures").join(COPY_DIR).exists());
    }

    #[test]
    fn a_capture_without_text_cannot_be_copied() {
        let env = page_env(None);
        env.add_text_capture(
            NewCapture {
                text: Some("  \n\t "),
                ..capture("blank", "s1", "page", "2026-09-30T05:00:00Z")
            },
            SHA_B,
        );

        for id in ["p1", "blank"] {
            let error = copy_ticket(&env.conn, env.data.path(), id).unwrap_err();
            assert!(error.starts_with(code::NO_TEXT), "{id}: {error}");
        }
    }

    #[test]
    fn a_page_whose_text_file_is_gone_is_missing_not_empty() {
        let env = page_env(None);
        env.conn
            .execute(
                "UPDATE web_captures SET text_rel_path = 'web-captures/s1/p1.txt' WHERE id = 'p1'",
                [],
            )
            .unwrap();

        let error = copy_ticket(&env.conn, env.data.path(), "p1").unwrap_err();

        assert!(error.starts_with(code::FILE_MISSING), "{error}");
    }

    #[test]
    fn rendered_copies_are_temporary_and_stale_ones_are_purged_on_the_next_copy() {
        let env = page_env(Some("texto de la página que se copia"));
        let folder = env.data.path().join("web-captures").join(COPY_DIR);
        fs::create_dir_all(&folder).unwrap();
        let old = folder.join("old.pdf");
        let fresh = folder.join("fresh.pdf");
        let other = folder.join("notes.txt");
        for file in [&old, &fresh, &other] {
            fs::write(file, b"x").unwrap();
        }
        let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 3600);
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();

        let first = copy_ticket(&env.conn, env.data.path(), "p1").unwrap();
        let second = copy_ticket(&env.conn, env.data.path(), "p1").unwrap();

        assert!(!old.exists(), "a stale copy stays");
        assert!(fresh.exists(), "a fresh copy goes");
        assert!(other.exists(), "only pdfs are purged");
        assert_ne!(first.path, second.path);
        assert!(PathBuf::from(&first.path).exists());
    }

    #[test]
    fn a_short_selection_copy_still_has_enough_text_to_skip_the_ocr() {
        let env = Env::new();
        env.add_source(source("s1", "https://e.com/nota", 1));
        env.add_text_capture(
            NewCapture {
                text: Some("Sí."),
                ..capture("q1", "s1", "selection", "2026-09-30T04:00:00Z")
            },
            &digest("Sí.".as_bytes()),
        );

        let ticket = copy_ticket(&env.conn, env.data.path(), "q1").unwrap();

        let bytes = fs::read(&ticket.path).unwrap();
        let text = pdf_extract::extract_text_from_mem(&bytes).unwrap();
        assert!(crate::ocr::pdf::is_quality_text(&text), "{text}");
    }

    #[test]
    fn only_a_text_copy_says_it_is_a_rendering() {
        let pdf = serde_json::to_value(WebCaptureProvenance {
            source_id: "s".into(),
            capture_id: "c".into(),
            original_url: "o".into(),
            final_url: "f".into(),
            page_title: None,
            accessed_at: "a".into(),
            sha256: "h".into(),
            capture_kind: None,
            rendering: None,
        })
        .unwrap();
        assert!(pdf.get("rendering").is_none());
        assert!(pdf.get("captureKind").is_none());

        let env = page_env(Some("texto suficiente para renderizar"));
        let ticket = copy_ticket(&env.conn, env.data.path(), "p1").unwrap();
        let text = serde_json::to_value(&ticket.provenance).unwrap();
        assert_eq!(text["rendering"], "text-pdf");
        assert_eq!(text["captureKind"], "page");
    }

    // --- shape ---------------------------------------------------------------

    #[test]
    fn the_results_reach_the_ui_in_camel_case() {
        let summary = serde_json::to_value(SourceSummary {
            id: "s".into(),
            title: None,
            final_url: "https://e.com/".into(),
            site_name: None,
            updated_at: 1,
            capture_count: 2,
            kinds: vec!["page".into()],
        })
        .unwrap();
        for key in [
            "id",
            "title",
            "finalUrl",
            "siteName",
            "updatedAt",
            "captureCount",
            "kinds",
        ] {
            assert!(summary.get(key).is_some(), "missing {key}");
        }
        let outcome = serde_json::to_value(DeleteOutcome {
            leftover_files: true,
        })
        .unwrap();
        assert_eq!(outcome["leftoverFiles"], true);
        let ticket = serde_json::to_value(CopyTicket {
            path: "p".into(),
            provenance: WebCaptureProvenance {
                source_id: "s".into(),
                capture_id: "c".into(),
                original_url: "o".into(),
                final_url: "f".into(),
                page_title: None,
                accessed_at: "a".into(),
                sha256: "h".into(),
                capture_kind: None,
                rendering: None,
            },
        })
        .unwrap();
        assert!(ticket.get("path").is_some());
        for key in [
            "sourceId",
            "captureId",
            "originalUrl",
            "finalUrl",
            "pageTitle",
            "accessedAt",
            "sha256",
        ] {
            assert!(ticket["provenance"].get(key).is_some(), "missing {key}");
        }
        let capture = serde_json::to_value(CaptureDetail {
            id: "c".into(),
            kind: "page".into(),
            mime_type: "text/html".into(),
            accessed_at: "x".into(),
            final_url: "u".into(),
            title: None,
            sha256: "h".into(),
            hash_of: "html".into(),
            size_bytes: 1,
            text_preview: None,
            text_in_file: false,
            quote_prefix: None,
            quote_suffix: None,
            file_present: Some(true),
            file_pending: false,
            created_at: 1,
        })
        .unwrap();
        for key in [
            "mimeType",
            "accessedAt",
            "finalUrl",
            "hashOf",
            "sizeBytes",
            "textPreview",
            "textInFile",
            "quotePrefix",
            "quoteSuffix",
            "filePresent",
            "filePending",
            "createdAt",
        ] {
            assert!(capture.get(key).is_some(), "missing {key}");
        }
    }
}
