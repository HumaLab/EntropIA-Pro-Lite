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
fn capture_file(data_dir: &Path, source_id: &str, key: &str) -> Option<PathBuf> {
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
                Ok(CaptureDetail {
                    id: row.get(0)?,
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
            "createdAt",
        ] {
            assert!(capture.get(key).is_some(), "missing {key}");
        }
    }
}
