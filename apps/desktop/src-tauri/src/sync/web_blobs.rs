//! Files of saved web captures on the sync wire (`web-captures/<source>/<capture>.<ext>`).
//!
//! A `web_captures` row can reference up to two files:
//!
//! - `rel_path`: the HTML snapshot (page) or the PDF. Its digest and size are
//!   the row's own `sha256` / `size_bytes` (`hash_of` is `html` or `pdf`).
//! - `text_rel_path`: text too large for the row (`<capture>.txt`). The schema
//!   keeps no digest for it, so the push computes it and sends the wire-only
//!   keys `text_sha256` / `text_size`; the receiver strips them before the row
//!   is written (like `assets` strips its wire-only keys).
//!
//! Files travel as content-addressed blobs (`/v1/blobs/{sha256}`, the same
//! endpoint as assets). Push uploads every file BEFORE its row. Pull applies
//! the row, queues the downloads in `sync_web_pending_blobs` in the same
//! transaction, and a drain installs each file after verifying size and
//! sha256 (temp file, fsync, atomic rename). A row whose file has not arrived
//! is a normal state the UI reports as "downloading" or "not available".
//!
//! The queue is a parallel table rather than a `(kind, id)` generalisation of
//! the asset one: `sync_pending_blobs` and `sync_blob_index` are keyed by
//! `asset_id` and read by five asset-specific paths, and rebuilding populated
//! production tables for a new key is a risk this feature does not need. No
//! upload index exists either: captures are immutable, so each file is hashed
//! once per push and an mtime cache would only save work that is not repeated.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{Map, Value};

use super::blobs::{download_blob, hash_file, BlobDownloadError, PendingBlob};
use super::http::{PushChange, SyncApi};
use crate::navegador::capture_files::{self, Located};

/// Directory under the data dir that holds every capture file.
pub(crate) const DIR: &str = "web-captures";
/// Download attempts that end in a 404 before the file is given up on.
pub(crate) const MAX_DOWNLOAD_RETRIES: i64 = 5;

/// Which of a capture's two files a queue entry or key refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// The HTML snapshot or the PDF (`rel_path`).
    File,
    /// The long text (`text_rel_path`).
    Text,
}

impl Role {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Role::File => "file",
            Role::Text => "text",
        }
    }

    fn parse(raw: &str) -> Option<Role> {
        match raw {
            "file" => Some(Role::File),
            "text" => Some(Role::Text),
            _ => None,
        }
    }
}

/// One file a capture row references, with what verifies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WebFile {
    pub(crate) role: Role,
    pub(crate) rel_path: String,
    pub(crate) sha256: String,
    pub(crate) size: i64,
}

fn is_digest(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Validates a file key for one capture. The only accepted shape is
/// `web-captures/<web_source_id>/<capture_id>.<ext>` with plain ids, the source
/// matching the row's own `web_source_id`, and an extension that agrees with
/// the file's role and the capture's kind. Anything else (absolute paths,
/// drives, UNC, `..`, backslashes, other folders, extra segments) is refused.
pub(crate) fn validate_rel_key(
    rel_path: &str,
    web_source_id: &str,
    capture_id: &str,
    role: Role,
    kind: Option<&str>,
) -> Result<(), String> {
    if rel_path.contains('\\') || rel_path.contains(':') {
        return Err("the file key is not a plain relative key".to_string());
    }
    let parts: Vec<&str> = rel_path.split('/').collect();
    let [dir, source, name] = parts.as_slice() else {
        return Err("the file key must be web-captures/<source>/<file>".to_string());
    };
    if *dir != DIR {
        return Err("the file key is outside web-captures".to_string());
    }
    if !capture_files::valid_source_id(web_source_id) || !capture_files::valid_source_id(capture_id)
    {
        return Err("the capture or source id is not valid".to_string());
    }
    if *source != web_source_id {
        return Err("the file key names another source".to_string());
    }
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return Err("the file key has no extension".to_string());
    };
    if stem != capture_id {
        return Err("the file key names another capture".to_string());
    }
    let allowed: &[&str] = match (role, kind) {
        (Role::File, Some("page")) => &["html"],
        (Role::File, Some("pdf")) => &["pdf"],
        // A queued entry no longer carries the kind: either extension is fine.
        (Role::File, None) => &["html", "pdf"],
        (Role::Text, _) => &["txt"],
        (Role::File, Some(_)) => return Err("this kind of capture has no file".to_string()),
    };
    if !allowed.contains(&ext) {
        return Err(format!(
            "the file key must end in .{}",
            allowed.join(" or .")
        ));
    }
    Ok(())
}

fn text_field<'a>(payload: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
}

/// The files a capture row references, validated, with their digests. Reads
/// `rel_path` + `sha256` + `size_bytes` and `text_rel_path` + `text_sha256` +
/// `text_size`. Used on both sides of the wire.
pub(crate) fn files_of(
    payload: &Map<String, Value>,
    capture_id: &str,
) -> Result<Vec<WebFile>, String> {
    let source = text_field(payload, "web_source_id").ok_or("the capture has no source")?;
    let kind = text_field(payload, "kind").ok_or("the capture has no kind")?;
    let mut files = Vec::new();

    if let Some(rel) = text_field(payload, "rel_path") {
        validate_rel_key(rel, source, capture_id, Role::File, Some(kind))?;
        let sha256 = text_field(payload, "sha256").ok_or("the file has no digest")?;
        if !is_digest(sha256) {
            return Err("the file digest is not a sha256".to_string());
        }
        let size = payload
            .get("size_bytes")
            .and_then(Value::as_i64)
            .filter(|n| *n >= 0)
            .ok_or("the file has no size")?;
        files.push(WebFile {
            role: Role::File,
            rel_path: rel.to_string(),
            sha256: sha256.to_string(),
            size,
        });
    }
    if let Some(rel) = text_field(payload, "text_rel_path") {
        validate_rel_key(rel, source, capture_id, Role::Text, Some(kind))?;
        let sha256 = text_field(payload, "text_sha256").ok_or("the text file has no digest")?;
        if !is_digest(sha256) {
            return Err("the text digest is not a sha256".to_string());
        }
        let size = payload
            .get("text_size")
            .and_then(Value::as_i64)
            .filter(|n| *n >= 0)
            .ok_or("the text file has no size")?;
        files.push(WebFile {
            role: Role::Text,
            rel_path: rel.to_string(),
            sha256: sha256.to_string(),
            size,
        });
    }
    Ok(files)
}

fn local_path(app_data_dir: &Path, rel_path: &str) -> PathBuf {
    let mut path = app_data_dir.to_path_buf();
    for component in rel_path.split('/') {
        path.push(component);
    }
    path
}

fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file())
}

// ---------------------------------------------------------------------------
// Push
// ---------------------------------------------------------------------------

/// What to do with a `web_captures` upsert after its files were handled.
#[derive(Debug)]
pub(crate) enum WebPushOutcome {
    /// Every file is on the server; the payload carries the wire-only digests.
    Ready,
    /// The row must not be pushed (the caller journals it and purges the oplog).
    Skip(String),
}

/// Makes every file of one `web_captures` upsert available on the server BEFORE
/// the row is sent, and adds `text_sha256` / `text_size` for the long text.
///
/// A key that fails validation, a file that is not a regular file inside the
/// data dir, or bytes that no longer match the recorded digest skip the row: it
/// would reference something no other device could fetch or verify. A missing
/// local file is only acceptable when the server already holds its digest.
/// Network failures are returned as `Err` so the cycle backs off.
pub(crate) async fn prepare_web_capture_push<A: SyncApi>(
    api: &A,
    token: &str,
    app_data_dir: &Path,
    change: &mut PushChange,
) -> Result<WebPushOutcome, String> {
    let Some(Value::Object(payload)) = change.payload.as_mut() else {
        return Ok(WebPushOutcome::Skip(
            "capture upsert has no payload".to_string(),
        ));
    };
    // The text digest does not exist yet: compute it from the file first.
    if let Some(rel) = text_field(payload, "text_rel_path").map(str::to_string) {
        {
            let source = text_field(payload, "web_source_id")
                .unwrap_or_default()
                .to_string();
            let kind = text_field(payload, "kind").unwrap_or_default().to_string();
            if let Err(reason) =
                validate_rel_key(&rel, &source, &change.row_id, Role::Text, Some(&kind))
            {
                return Ok(WebPushOutcome::Skip(reason));
            }
            let path = local_path(app_data_dir, &rel);
            if !is_regular_file(&path) {
                return Ok(WebPushOutcome::Skip(
                    "the text file is missing from this device".to_string(),
                ));
            }
            let (sha256, size) = hash_file(&path)?;
            payload.insert("text_sha256".to_string(), Value::String(sha256));
            payload.insert("text_size".to_string(), Value::from(size));
        }
    }

    let files = match files_of(payload, &change.row_id) {
        Ok(files) => files,
        Err(reason) => return Ok(WebPushOutcome::Skip(reason)),
    };

    for file in &files {
        let path = local_path(app_data_dir, &file.rel_path);
        if is_regular_file(&path) {
            let (sha256, size) = hash_file(&path)?;
            if sha256 != file.sha256 || size != file.size {
                return Ok(WebPushOutcome::Skip(format!(
                    "the {} file no longer matches its recorded digest",
                    file.role.as_str()
                )));
            }
            let present = api
                .blob_head(token, &file.sha256)
                .await
                .map_err(String::from)?;
            if !present {
                api.blob_put_file(token, &file.sha256, &path, file.size)
                    .await
                    .map_err(String::from)?;
            }
        } else if !api
            .blob_head(token, &file.sha256)
            .await
            .map_err(String::from)?
        {
            return Ok(WebPushOutcome::Skip(format!(
                "the {} file is missing and the server does not hold it",
                file.role.as_str()
            )));
        }
    }
    Ok(WebPushOutcome::Ready)
}

// ---------------------------------------------------------------------------
// Pull
// ---------------------------------------------------------------------------

/// Validates an inbound `web_captures` payload and strips its wire-only keys so
/// the row can be written to the real table. Returns the files to fetch.
pub(crate) fn rewrite_inbound(
    payload: &mut Map<String, Value>,
    capture_id: &str,
) -> Result<Vec<WebFile>, String> {
    let files = files_of(payload, capture_id)?;
    payload.remove("text_sha256");
    payload.remove("text_size");
    Ok(files)
}

/// Queues the downloads of the files that are not already on disk with the
/// expected size. Runs inside the apply transaction.
pub(crate) fn enqueue_downloads(
    conn: &Connection,
    capture_id: &str,
    files: &[WebFile],
    app_data_dir: &Path,
) -> Result<(), String> {
    for file in files {
        let path = local_path(app_data_dir, &file.rel_path);
        let present = std::fs::symlink_metadata(&path)
            .is_ok_and(|meta| meta.file_type().is_file() && meta.len() as i64 == file.size);
        if present {
            continue;
        }
        conn.execute(
            "INSERT INTO sync_web_pending_blobs(capture_id, role, sha256, rel_path, size)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(capture_id, role) DO UPDATE SET
               sha256 = excluded.sha256, rel_path = excluded.rel_path, size = excluded.size",
            params![
                capture_id,
                file.role.as_str(),
                file.sha256,
                file.rel_path,
                file.size
            ],
        )
        .map_err(|e| format!("[sync] failed to queue web capture file for {capture_id}: {e}"))?;
    }
    Ok(())
}

/// True while a file of this capture is still queued for download.
pub(crate) fn has_pending_download(conn: &Connection, capture_id: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sync_web_pending_blobs WHERE capture_id = ?1 LIMIT 1",
        [capture_id],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
    .unwrap_or(false)
}

struct Pending {
    capture_id: String,
    role: Role,
    sha256: String,
    rel_path: String,
    size: i64,
    retry_count: i64,
}

fn read_pending(conn: &Connection) -> Result<Vec<Pending>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT capture_id, role, sha256, rel_path, size, retry_count
             FROM sync_web_pending_blobs ORDER BY retry_count ASC, capture_id ASC, role ASC",
        )
        .map_err(|e| format!("[sync] failed to read the web file queue: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|e| format!("[sync] failed to read the web file queue: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        let (capture_id, role, sha256, rel_path, size, retry_count) =
            row.map_err(|e| format!("[sync] failed to read the web file queue: {e}"))?;
        let Some(role) = Role::parse(&role) else {
            continue;
        };
        out.push(Pending {
            capture_id,
            role,
            sha256,
            rel_path,
            size,
            retry_count,
        });
    }
    Ok(out)
}

fn delete_pending(conn: &Connection, capture_id: &str, role: Role) -> Result<(), String> {
    conn.execute(
        "DELETE FROM sync_web_pending_blobs WHERE capture_id = ?1 AND role = ?2",
        params![capture_id, role.as_str()],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to drop a web file from the queue: {e}"))
}

fn record_failure(
    conn: &Connection,
    capture_id: &str,
    role: Role,
    error: &str,
) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    conn.execute(
        "UPDATE sync_web_pending_blobs
         SET retry_count = retry_count + 1, last_error = ?3, last_attempt_at = ?4
         WHERE capture_id = ?1 AND role = ?2",
        params![capture_id, role.as_str(), error, now],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to record a web file failure: {e}"))
}

fn journal(
    conn: &Connection,
    capture_id: &str,
    role: Role,
    reason: &str,
    detail: &str,
) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO sync_conflicts(id, table_name, row_id, reason, loser_payload, winner_summary, created_at, acknowledged)
         VALUES (?1, 'web_captures', ?2, ?3, NULL, ?4, ?5, 0)
         ON CONFLICT(id) DO UPDATE SET winner_summary = excluded.winner_summary, created_at = excluded.created_at",
        params![
            format!("{reason}-web_captures-{capture_id}-{}", role.as_str()),
            capture_id,
            reason,
            detail,
            now
        ],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to journal {reason}: {e}"))
}

/// Downloads the queued capture files once. Each file is verified (size and
/// sha256) and installed atomically at its own key. Entries whose capture row
/// is gone are dropped; a folder that is not a plain folder inside
/// `web-captures/` is never written to; a 404 is retried and, after
/// [`MAX_DOWNLOAD_RETRIES`], journaled `blob_missing` and dropped; a mismatch
/// journals `blob_hash_mismatch` and stays queued; transport errors stay
/// queued. Returns the number of files installed.
pub(crate) async fn drain_pending_web_blobs<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    app_data_dir: &Path,
) -> Result<usize, String> {
    let mut installed = 0usize;
    for entry in read_pending(conn)? {
        let row_exists = conn
            .query_row(
                "SELECT web_source_id FROM web_captures WHERE id = ?1",
                [&entry.capture_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("[sync] failed to look up capture {}: {e}", entry.capture_id))?;
        let Some(source_id) = row_exists else {
            delete_pending(conn, &entry.capture_id, entry.role)?;
            continue;
        };
        // The key was validated when queued; re-check against the row's source
        // so a damaged queue row cannot point anywhere else.
        if validate_rel_key(
            &entry.rel_path,
            &source_id,
            &entry.capture_id,
            entry.role,
            None,
        )
        .is_err()
        {
            delete_pending(conn, &entry.capture_id, entry.role)?;
            continue;
        }
        match capture_files::locate_source_dir(app_data_dir, &source_id) {
            Located::Dir(_) => {}
            Located::Missing => {
                let dir = capture_files::root(app_data_dir).join(&source_id);
                if std::fs::create_dir_all(&dir).is_err()
                    || !matches!(
                        capture_files::locate_source_dir(app_data_dir, &source_id),
                        Located::Dir(_)
                    )
                {
                    record_failure(
                        conn,
                        &entry.capture_id,
                        entry.role,
                        "cannot create the folder",
                    )?;
                    continue;
                }
            }
            Located::Refused(reason) => {
                record_failure(conn, &entry.capture_id, entry.role, reason)?;
                continue;
            }
        }

        let blob = PendingBlob {
            asset_id: format!("{}:{}", entry.capture_id, entry.role.as_str()),
            sha256: entry.sha256.clone(),
            rel_path: entry.rel_path.clone(),
            size: entry.size,
            retry_count: entry.retry_count,
        };
        match download_blob(api, token, &blob, app_data_dir).await {
            Ok(()) => {
                delete_pending(conn, &entry.capture_id, entry.role)?;
                installed += 1;
            }
            Err(BlobDownloadError::NotFound) => {
                if entry.retry_count + 1 >= MAX_DOWNLOAD_RETRIES {
                    journal(
                        conn,
                        &entry.capture_id,
                        entry.role,
                        "blob_missing",
                        "server returned 404 after bounded retries",
                    )?;
                    delete_pending(conn, &entry.capture_id, entry.role)?;
                } else {
                    record_failure(conn, &entry.capture_id, entry.role, "not_found")?;
                }
            }
            Err(BlobDownloadError::HashMismatch(detail)) => {
                journal(
                    conn,
                    &entry.capture_id,
                    entry.role,
                    "blob_hash_mismatch",
                    &detail,
                )?;
                record_failure(conn, &entry.capture_id, entry.role, &detail)?;
            }
            Err(BlobDownloadError::Transport(detail)) => {
                record_failure(conn, &entry.capture_id, entry.role, &detail)?;
            }
        }
    }
    Ok(installed)
}
