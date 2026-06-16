//! Blob upload side of the sync client (DESIGN §7, PROTOCOL "Blobs" +
//! "Transformación de assets"). Covers, for the PUSH direction only (download
//! lives in a later slice):
//!
//! - `rel_path` derivation from the local absolute `assets.path` (strip the
//!   app-data-dir prefix, normalize separators to `/`, require an `assets/`
//!   prefix). Paths outside the app-data dir are rejected so the caller can skip
//!   the row and journal `apply_error`.
//! - SHA-256 hashing of the local file, cached in `sync_blob_index` and
//!   invalidated by file mtime.
//! - The asset wire transformation: the payload's absolute `path` key is OMITTED
//!   and replaced with `rel_path` + `sha256` + `size` (PROTOCOL).
//!
//! This module's surface is driven by the engine slice (next slice); here it is
//! exercised only by unit tests, so the forward-looking API carries a
//! module-level `allow(dead_code)` (removed once the engine wires it up — same
//! convention as the C1 foundations).
#![allow(dead_code)]

use std::io::Write as _;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// Lowercase hex encoding of raw bytes (mirrors `audio_preview::hex_lower`; kept
/// local to avoid a cross-module dependency on a private helper).
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Why a `rel_path` derivation failed. The caller maps these to a skipped row
/// plus a journaled `apply_error` (DESIGN §7).
#[derive(Debug, PartialEq, Eq)]
pub enum RelPathError {
    /// The local path is not inside the app-data dir (e.g. an external import
    /// that was never copied in). The row must be skipped, not pushed.
    OutsideAppData,
    /// After stripping the prefix the remainder did not begin with `assets/`.
    NotUnderAssets,
    /// The path was empty.
    Empty,
}

impl std::fmt::Display for RelPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelPathError::OutsideAppData => write!(f, "asset path is outside the app-data dir"),
            RelPathError::NotUnderAssets => write!(f, "asset path is not under assets/"),
            RelPathError::Empty => write!(f, "asset path is empty"),
        }
    }
}

/// Derives the wire `rel_path` from a local absolute `assets.path` (PROTOCOL
/// "Transformación de assets"):
///
/// 1. Strip the `app_data_dir` prefix.
/// 2. Normalize separators to `/`.
/// 3. Require the remainder to start with `assets/`.
///
/// Comparison is done on the string form normalized to `/` so a Windows
/// backslash path matches a forward-slash app-data dir. Rows whose path is
/// outside the app-data dir return [`RelPathError::OutsideAppData`] so the
/// caller skips + journals them (DESIGN §7).
pub fn derive_rel_path(abs_path: &str, app_data_dir: &Path) -> Result<String, RelPathError> {
    if abs_path.trim().is_empty() {
        return Err(RelPathError::Empty);
    }

    let normalize = |s: &str| s.replace('\\', "/");
    let path_norm = normalize(abs_path);
    let mut prefix_norm = normalize(&app_data_dir.to_string_lossy());
    if !prefix_norm.ends_with('/') {
        prefix_norm.push('/');
    }

    // Case-insensitive prefix match on Windows (drive letters/paths are
    // case-insensitive there); exact elsewhere.
    let starts_with_prefix = if cfg!(windows) {
        path_norm
            .to_ascii_lowercase()
            .starts_with(&prefix_norm.to_ascii_lowercase())
    } else {
        path_norm.starts_with(&prefix_norm)
    };
    if !starts_with_prefix {
        return Err(RelPathError::OutsideAppData);
    }

    // Slice off the matched prefix length from the ORIGINAL-normalized path so
    // the casing of the remainder (the assets/ subtree) is preserved verbatim.
    let rel = &path_norm[prefix_norm.len()..];
    let rel = rel.trim_start_matches('/');

    if !rel.starts_with("assets/") {
        return Err(RelPathError::NotUnderAssets);
    }

    Ok(rel.to_string())
}

/// Result of resolving a blob's hash/size for push, after consulting (and
/// refreshing) the `sync_blob_index` mtime cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobDigest {
    pub sha256: String,
    pub size: i64,
    /// Whether the cached entry was reused without re-hashing the file.
    pub from_cache: bool,
}

/// Returns the file's mtime in ms since the Unix epoch, or `0` if unavailable.
fn file_mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Hashes `path` with SHA-256 and returns the lowercase hex digest + byte size.
/// Streams the file in chunks so large blobs do not load fully into memory.
pub fn hash_file(path: &Path) -> Result<(String, i64), String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("[sync] failed to open blob {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total: i64 = 0;
    loop {
        let read = std::io::Read::read(&mut file, &mut buf)
            .map_err(|e| format!("[sync] failed to read blob {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        total += read as i64;
    }
    Ok((hex_lower(&hasher.finalize()), total))
}

/// Resolves the SHA-256 + size for an asset's file, using the `sync_blob_index`
/// mtime cache (DESIGN §7). When the cached `file_mtime_ms` matches the file's
/// current mtime the cached hash is trusted; otherwise the file is re-hashed and
/// the cache row is upserted with `uploaded` left untouched on a cache hit and
/// reset to `0` on a re-hash (a changed file means the old blob is stale).
pub fn resolve_blob_digest(
    conn: &Connection,
    asset_id: &str,
    abs_path: &Path,
) -> Result<BlobDigest, String> {
    let meta = std::fs::metadata(abs_path)
        .map_err(|e| format!("[sync] failed to stat blob {}: {e}", abs_path.display()))?;
    let mtime = file_mtime_ms(&meta);

    let cached: Option<(String, i64, i64)> = conn
        .query_row(
            "SELECT sha256, size, file_mtime_ms FROM sync_blob_index WHERE asset_id = ?1",
            [asset_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();

    if let Some((sha256, size, cached_mtime)) = cached {
        if cached_mtime == mtime {
            return Ok(BlobDigest {
                sha256,
                size,
                from_cache: true,
            });
        }
    }

    // Cache miss or stale mtime: re-hash and refresh the cache. A re-hash means
    // the file changed under us, so the previously-uploaded blob no longer
    // matches — reset uploaded=0 to force a fresh HEAD/PUT (DESIGN §7).
    let (sha256, size) = hash_file(abs_path)?;
    conn.execute(
        "INSERT INTO sync_blob_index(asset_id, sha256, size, file_mtime_ms, uploaded)
         VALUES (?1, ?2, ?3, ?4, 0)
         ON CONFLICT(asset_id) DO UPDATE SET
           sha256 = excluded.sha256,
           size = excluded.size,
           file_mtime_ms = excluded.file_mtime_ms,
           uploaded = 0",
        rusqlite::params![asset_id, sha256, size, mtime],
    )
    .map_err(|e| format!("[sync] failed to update blob index for {asset_id}: {e}"))?;

    Ok(BlobDigest {
        sha256,
        size,
        from_cache: false,
    })
}

/// Reads the `uploaded` flag for an asset from `sync_blob_index` (DESIGN §6.3).
/// `uploaded=1` is only trusted after a HEAD re-confirm before a row push.
#[allow(dead_code)]
pub fn blob_uploaded(conn: &Connection, asset_id: &str) -> Result<bool, String> {
    let flag: Option<i64> = conn
        .query_row(
            "SELECT uploaded FROM sync_blob_index WHERE asset_id = ?1",
            [asset_id],
            |row| row.get(0),
        )
        .ok();
    Ok(flag == Some(1))
}

/// Marks an asset's blob as uploaded (`uploaded=1`) after a confirmed PUT/HEAD.
#[allow(dead_code)]
pub fn mark_blob_uploaded(conn: &Connection, asset_id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sync_blob_index SET uploaded = 1 WHERE asset_id = ?1",
        [asset_id],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to mark blob uploaded for {asset_id}: {e}"))
}

/// Resets an asset's blob `uploaded` flag to `0` (DESIGN §6.3): used when a HEAD
/// returns 404 for a blob we believed was uploaded (server restore), forcing a
/// re-PUT.
#[allow(dead_code)]
pub fn reset_blob_uploaded(conn: &Connection, asset_id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sync_blob_index SET uploaded = 0 WHERE asset_id = ?1",
        [asset_id],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to reset blob uploaded for {asset_id}: {e}"))
}

/// Transforms a raw `assets` row payload (read from the local table) into the
/// wire shape (PROTOCOL "Transformación de assets"): the absolute `path` key is
/// REMOVED and replaced with `rel_path` + `sha256` + `size`. All other keys
/// (id, item_id, type, sort_index, created_at, …) are preserved.
///
/// `size` always reflects the actual file size from the hash pass, overriding
/// any stale `assets.size` column value.
pub fn asset_payload_to_wire(
    mut payload: serde_json::Value,
    rel_path: &str,
    sha256: &str,
    size: i64,
) -> serde_json::Value {
    if let Some(obj) = payload.as_object_mut() {
        obj.remove("path");
        obj.insert(
            "rel_path".to_string(),
            serde_json::Value::String(rel_path.to_string()),
        );
        obj.insert(
            "sha256".to_string(),
            serde_json::Value::String(sha256.to_string()),
        );
        obj.insert(
            "size".to_string(),
            serde_json::Value::Number(serde_json::Number::from(size)),
        );
    }
    payload
}

// ---------------------------------------------------------------------------
// Blob upload orchestration (DESIGN §7, PROTOCOL "Transformación de assets" —
// PUSH direction). The engine calls [`prepare_asset_push`] for each dirty asset
// `upsert` change BEFORE the row goes into a push batch: blob first, row after,
// so no device ever sees a row whose blob is absent.
// ---------------------------------------------------------------------------

/// What to do with an asset `upsert` change after the blob step (DESIGN §7).
#[derive(Debug)]
pub enum AssetPushOutcome {
    /// The blob is present on the server (HEAD/PUT confirmed); the change's
    /// payload was rewritten to the wire shape (`rel_path`/`sha256`/`size`).
    Ready,
    /// The row must NOT be pushed (local file missing and not confirmed
    /// uploaded, or path outside the app-data dir). The caller journals
    /// `apply_error`, purges the oplog entry, and skips the row.
    Skip(String),
}

/// Prepares one asset `upsert` [`PushChange`](crate::sync::http::PushChange) for
/// the wire (DESIGN §7, PROTOCOL "Transformación de assets"). Mutates `change`
/// in place. Steps:
///
/// 1. Read the absolute `path` from the raw payload; derive `rel_path` (a path
///    outside the app-data dir ⇒ [`AssetPushOutcome::Skip`]).
/// 2. If the local file exists: hash (mtime cache), HEAD the server, PUT if
///    missing, mark `uploaded=1`, then rewrite the payload to the wire shape.
/// 3. If the local file is MISSING: only push when `uploaded=1` is confirmed by
///    a fresh HEAD (a HEAD 404 resets `uploaded=0`); otherwise skip (the row
///    would reference a blob no device can fetch).
///
/// Non-asset changes and `delete` ops must NOT be passed here.
pub async fn prepare_asset_push<A: crate::sync::http::SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    app_data_dir: &Path,
    change: &mut crate::sync::http::PushChange,
) -> Result<AssetPushOutcome, String> {
    let Some(payload) = change.payload.as_ref() else {
        return Ok(AssetPushOutcome::Skip(
            "asset upsert has no payload".to_string(),
        ));
    };
    let asset_id = change.row_id.clone();
    let abs_path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let rel_path = match derive_rel_path(&abs_path, app_data_dir) {
        Ok(rel) => rel,
        Err(err) => return Ok(AssetPushOutcome::Skip(err.to_string())),
    };

    let local_path = Path::new(&abs_path);
    if local_path.is_file() {
        // File present: hash (cache), ensure the blob is on the server.
        let digest = resolve_blob_digest(conn, &asset_id, local_path)?;
        ensure_blob_on_server(api, token, conn, &asset_id, &digest, local_path).await?;
        let wire = asset_payload_to_wire(
            change.payload.take().unwrap_or(serde_json::Value::Null),
            &rel_path,
            &digest.sha256,
            digest.size,
        );
        change.payload = Some(wire);
        Ok(AssetPushOutcome::Ready)
    } else {
        // File missing: only safe to push if a prior upload is HEAD-confirmed.
        let cached: Option<(String, i64, i64)> = conn
            .query_row(
                "SELECT sha256, size, uploaded FROM sync_blob_index WHERE asset_id = ?1",
                [&asset_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        let Some((sha256, size, uploaded)) = cached else {
            return Ok(AssetPushOutcome::Skip(
                "asset file missing and never hashed".to_string(),
            ));
        };
        if uploaded != 1 {
            return Ok(AssetPushOutcome::Skip(
                "asset file missing and blob not uploaded".to_string(),
            ));
        }
        // Confirm with HEAD (uploaded=1 is only trusted post-HEAD, DESIGN §6.3).
        if api.blob_head(token, &sha256).await.map_err(String::from)? {
            let wire = asset_payload_to_wire(
                change.payload.take().unwrap_or(serde_json::Value::Null),
                &rel_path,
                &sha256,
                size,
            );
            change.payload = Some(wire);
            Ok(AssetPushOutcome::Ready)
        } else {
            // Server lost the blob and we have no local file to re-upload.
            reset_blob_uploaded(conn, &asset_id)?;
            Ok(AssetPushOutcome::Skip(
                "asset file missing and server blob gone (HEAD 404)".to_string(),
            ))
        }
    }
}

/// HEAD → PUT the blob for `digest` if the server does not already have it, then
/// mark `uploaded=1` (DESIGN §7). `uploaded=1` is always re-confirmed by the
/// HEAD here, so a server restore that dropped the blob triggers a fresh PUT.
async fn ensure_blob_on_server<A: crate::sync::http::SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    asset_id: &str,
    digest: &BlobDigest,
    local_path: &Path,
) -> Result<(), String> {
    let exists = api
        .blob_head(token, &digest.sha256)
        .await
        .map_err(String::from)?;
    if !exists {
        let bytes = std::fs::read(local_path)
            .map_err(|e| format!("[sync] failed to read blob {}: {e}", local_path.display()))?;
        api.blob_put(token, &digest.sha256, bytes)
            .await
            .map_err(String::from)?;
    }
    mark_blob_uploaded(conn, asset_id)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Blob download (DESIGN §7, PROTOCOL "Blobs" — PULL direction, C3)
// ---------------------------------------------------------------------------

/// Max download attempts before a 404 is journaled `blob_missing` and the entry
/// is dropped from the queue (PROTOCOL: bounded retries on 404).
pub const MAX_BLOB_DOWNLOAD_RETRIES: i64 = 5;

/// A queued blob download read from `sync_pending_blobs`.
#[derive(Debug, Clone)]
pub struct PendingBlob {
    pub asset_id: String,
    pub sha256: String,
    pub rel_path: String,
    pub size: i64,
    pub retry_count: i64,
}

/// Reads all queued blob downloads ordered by fewest retries first (so a
/// perpetually-failing blob does not starve the rest).
pub fn read_pending_blobs(conn: &Connection) -> Result<Vec<PendingBlob>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT asset_id, sha256, rel_path, size, retry_count
             FROM sync_pending_blobs ORDER BY retry_count ASC, asset_id ASC",
        )
        .map_err(|e| format!("[sync] failed to prepare pending blobs read: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(PendingBlob {
                asset_id: row.get(0)?,
                sha256: row.get(1)?,
                rel_path: row.get(2)?,
                size: row.get(3)?,
                retry_count: row.get(4)?,
            })
        })
        .map_err(|e| format!("[sync] failed to query pending blobs: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("[sync] failed to read pending blob: {e}"))?);
    }
    Ok(out)
}

fn delete_pending_blob(conn: &Connection, asset_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM sync_pending_blobs WHERE asset_id = ?1",
        [asset_id],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to delete pending blob {asset_id}: {e}"))
}

fn record_blob_failure(conn: &Connection, asset_id: &str, error: &str) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    conn.execute(
        "UPDATE sync_pending_blobs
         SET retry_count = retry_count + 1, last_error = ?2, last_attempt_at = ?3
         WHERE asset_id = ?1",
        rusqlite::params![asset_id, error, now],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to record blob failure for {asset_id}: {e}"))
}

/// The error of a single blob download attempt — distinguishes a not-found (404,
/// retry-then-journal) from a mismatch (journal + keep queued) from a transport
/// error (keep queued).
#[derive(Debug)]
pub enum BlobDownloadError {
    /// Server returned 404 — the blob is not (yet) on the server.
    NotFound,
    /// Downloaded bytes did not match the declared sha256/size.
    HashMismatch(String),
    /// Transport / IO failure during the stream.
    Transport(String),
}

/// Streams one blob from the server into `{final}.part`, feeding a hasher and a
/// byte counter; aborts if the stream exceeds the declared `size`. On a verified
/// match it fsyncs and atomically renames to the final path (DESIGN §7). The
/// final path is `{app_data_dir}/{rel_path}` (rel_path already validated at apply
/// time). The temp file is always removed on any failure.
pub async fn download_blob<A: crate::sync::http::SyncApi>(
    api: &A,
    token: &str,
    blob: &PendingBlob,
    app_data_dir: &Path,
) -> Result<(), BlobDownloadError> {
    let final_path = blob_local_path(app_data_dir, &blob.rel_path);
    let temp_path = part_path(&final_path);

    // Ensure the parent directory exists (rel_path is validated to live under
    // assets/; ensure_within_dir already ran at apply time).
    if let Some(parent) = final_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| BlobDownloadError::Transport(format!("mkdir failed: {e}")))?;
    }

    let response = match api.blob_get(token, &blob.sha256).await {
        Ok(resp) => resp,
        Err(crate::sync::http::SyncError::Api { status: 404, .. }) => {
            return Err(BlobDownloadError::NotFound);
        }
        Err(other) => return Err(BlobDownloadError::Transport(other.to_string())),
    };

    let result = stream_to_temp(response, &temp_path, blob.size).await;
    let (sha256, total) = match result {
        Ok(pair) => pair,
        Err(err) => {
            let _ = std::fs::remove_file(&temp_path);
            return Err(err);
        }
    };

    if sha256 != blob.sha256 || total != blob.size {
        let _ = std::fs::remove_file(&temp_path);
        return Err(BlobDownloadError::HashMismatch(format!(
            "expected {}/{} got {}/{}",
            blob.sha256, blob.size, sha256, total
        )));
    }

    // fsync the temp file, then atomically rename into place.
    fsync_and_rename(&temp_path, &final_path)
        .map_err(|e| BlobDownloadError::Transport(format!("rename failed: {e}")))?;
    Ok(())
}

/// Streams a reqwest response body into `temp_path`, returning the streamed
/// content's `(sha256, byte_count)`. Aborts (error) when the stream exceeds
/// `declared_size` bytes (PROTOCOL: abort over declared size).
async fn stream_to_temp(
    response: reqwest::Response,
    temp_path: &Path,
    declared_size: i64,
) -> Result<(String, i64), BlobDownloadError> {
    let mut file = std::fs::File::create(temp_path)
        .map_err(|e| BlobDownloadError::Transport(format!("create temp failed: {e}")))?;
    let mut hasher = Sha256::new();
    let mut total: i64 = 0;
    let mut response = response;
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|e| BlobDownloadError::Transport(format!("stream read failed: {e}")))?;
        let Some(chunk) = chunk else { break };
        total += chunk.len() as i64;
        if total > declared_size {
            return Err(BlobDownloadError::HashMismatch(format!(
                "stream exceeded declared size {declared_size}"
            )));
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .map_err(|e| BlobDownloadError::Transport(format!("temp write failed: {e}")))?;
    }
    Ok((hex_lower(&hasher.finalize()), total))
}

/// fsyncs `temp` and renames it to `final_path`. On Windows a rename over an
/// existing file fails, so the destination is removed first (a successful verify
/// means the bytes are identical anyway).
fn fsync_and_rename(temp: &Path, final_path: &Path) -> std::io::Result<()> {
    {
        let file = std::fs::OpenOptions::new().write(true).open(temp)?;
        file.sync_all()?;
    }
    if final_path.exists() {
        let _ = std::fs::remove_file(final_path);
    }
    std::fs::rename(temp, final_path)
}

/// The local absolute path for a blob from its validated `rel_path`.
fn blob_local_path(app_data_dir: &Path, rel_path: &str) -> PathBuf {
    let mut path = app_data_dir.to_path_buf();
    for component in rel_path.split('/') {
        path.push(component);
    }
    path
}

/// The `.part` temp companion of a final blob path.
fn part_path(final_path: &Path) -> PathBuf {
    let mut os = final_path.as_os_str().to_os_string();
    os.push(".part");
    PathBuf::from(os)
}

/// Drains the `sync_pending_blobs` queue once (DESIGN §7, PROTOCOL step 7). For
/// each queued blob: download → verify → rename. Successes are removed from the
/// queue. A `HashMismatch` journals `blob_hash_mismatch` and keeps the entry
/// queued with bumped backoff. A `NotFound` bumps the retry count and, once
/// `MAX_BLOB_DOWNLOAD_RETRIES` is reached, journals `blob_missing` and drops it.
/// Transport errors keep the entry queued (transient). Returns the number of
/// blobs successfully downloaded.
pub async fn drain_pending_blobs<A: crate::sync::http::SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    app_data_dir: &Path,
) -> Result<usize, String> {
    let pending = read_pending_blobs(conn)?;
    let mut downloaded = 0usize;
    for blob in pending {
        match download_blob(api, token, &blob, app_data_dir).await {
            Ok(()) => {
                delete_pending_blob(conn, &blob.asset_id)?;
                downloaded += 1;
            }
            Err(BlobDownloadError::NotFound) => {
                if blob.retry_count + 1 >= MAX_BLOB_DOWNLOAD_RETRIES {
                    journal_blob_conflict(
                        conn,
                        &blob.asset_id,
                        "blob_missing",
                        "server returned 404 after bounded retries",
                    )?;
                    delete_pending_blob(conn, &blob.asset_id)?;
                } else {
                    record_blob_failure(conn, &blob.asset_id, "not_found")?;
                }
            }
            Err(BlobDownloadError::HashMismatch(detail)) => {
                journal_blob_conflict(conn, &blob.asset_id, "blob_hash_mismatch", &detail)?;
                record_blob_failure(conn, &blob.asset_id, &detail)?;
            }
            Err(BlobDownloadError::Transport(detail)) => {
                record_blob_failure(conn, &blob.asset_id, &detail)?;
            }
        }
    }
    Ok(downloaded)
}

/// Journals a blob conflict (`blob_missing` / `blob_hash_mismatch`) for the
/// status UI (DESIGN §6 schema). Idempotent per `(asset_id, reason)`.
fn journal_blob_conflict(
    conn: &Connection,
    asset_id: &str,
    reason: &str,
    detail: &str,
) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let conflict_id = format!("{reason}-assets-{asset_id}");
    conn.execute(
        "INSERT INTO sync_conflicts(id, table_name, row_id, reason, loser_payload, winner_summary, created_at, acknowledged)
         VALUES (?1, 'assets', ?2, ?3, NULL, ?4, ?5, 0)
         ON CONFLICT(id) DO UPDATE SET winner_summary = excluded.winner_summary, created_at = excluded.created_at",
        rusqlite::params![conflict_id, asset_id, reason, detail, now],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to journal {reason}: {e}"))
}

/// Removes orphaned `*.part` temp files under `{app_data_dir}/assets/` left by an
/// interrupted download (DESIGN §7 — `*.part` huérfanos se borran al arrancar).
/// Walks the assets subtree; best-effort (IO errors are ignored per file).
pub fn cleanup_orphan_parts(app_data_dir: &Path) -> Result<usize, String> {
    let assets_dir = app_data_dir.join("assets");
    if !assets_dir.exists() {
        return Ok(0);
    }
    let mut removed = 0usize;
    let mut stack = vec![assets_dir];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("part")
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Resets `uploaded=0` for every asset the local device OWNS (i.e. has a row in
/// `sync_blob_index`), forcing a re-HEAD / re-PUT on the next push (DESIGN §7).
/// Backing the `sync_reverify_blobs` command — HEAD answers from the filesystem,
/// so this repopulates a restored server. Returns the number of rows reset.
pub fn reverify_all_blobs(conn: &Connection) -> Result<usize, String> {
    conn.execute("UPDATE sync_blob_index SET uploaded = 0", [])
        .map_err(|e| format!("[sync] failed to reverify blobs: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_support::new_synced_test_db;
    use std::path::PathBuf;

    fn app_dir() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Users\ana\AppData\Roaming\com.entropia.lite")
        } else {
            PathBuf::from("/home/ana/.local/share/com.entropia.lite")
        }
    }

    fn abs(rel: &str) -> String {
        let mut p = app_dir().to_string_lossy().to_string();
        if cfg!(windows) {
            p.push('\\');
            p.push_str(&rel.replace('/', "\\"));
        } else {
            p.push('/');
            p.push_str(rel);
        }
        p
    }

    #[test]
    fn derive_rel_path_strips_prefix_and_normalizes() {
        let p = abs("assets/col-1/item-1/uuid_foto.png");
        let rel = derive_rel_path(&p, &app_dir()).expect("derive");
        assert_eq!(rel, "assets/col-1/item-1/uuid_foto.png");
    }

    #[test]
    fn derive_rel_path_handles_unicode_names() {
        let p = abs("assets/col-1/item-1/uuid_documentó_ñ.png");
        let rel = derive_rel_path(&p, &app_dir()).expect("derive");
        assert_eq!(rel, "assets/col-1/item-1/uuid_documentó_ñ.png");
    }

    #[test]
    fn derive_rel_path_rejects_outside_app_data() {
        let outside = if cfg!(windows) {
            r"D:\elsewhere\assets\x.png"
        } else {
            "/tmp/elsewhere/assets/x.png"
        };
        assert_eq!(
            derive_rel_path(outside, &app_dir()),
            Err(RelPathError::OutsideAppData)
        );
    }

    #[test]
    fn derive_rel_path_rejects_non_assets_subtree() {
        // Inside the app dir but not under assets/ (e.g. logs/).
        let p = abs("logs/entropia.log");
        assert_eq!(
            derive_rel_path(&p, &app_dir()),
            Err(RelPathError::NotUnderAssets)
        );
    }

    #[test]
    fn derive_rel_path_rejects_empty() {
        assert_eq!(derive_rel_path("", &app_dir()), Err(RelPathError::Empty));
        assert_eq!(derive_rel_path("   ", &app_dir()), Err(RelPathError::Empty));
    }

    #[test]
    fn hash_file_matches_known_sha256() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("blob.bin");
        std::fs::write(&file, b"hello").expect("write");
        let (sha, size) = hash_file(&file).expect("hash");
        // sha256("hello")
        assert_eq!(
            sha,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(size, 5);
    }

    #[test]
    fn resolve_blob_digest_uses_cache_then_invalidates_on_mtime() {
        let conn = new_synced_test_db();
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.bin");
        std::fs::write(&file, b"hello").expect("write");

        // First call: cache miss → hashes and stores uploaded=0.
        let first = resolve_blob_digest(&conn, "asset-1", &file).expect("first");
        assert!(!first.from_cache);
        assert_eq!(
            first.sha256,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );

        // Pretend the blob got uploaded.
        mark_blob_uploaded(&conn, "asset-1").expect("mark uploaded");
        assert!(blob_uploaded(&conn, "asset-1").unwrap());

        // Second call without touching the file: cache HIT (mtime unchanged),
        // uploaded flag preserved.
        let second = resolve_blob_digest(&conn, "asset-1", &file).expect("second");
        assert!(second.from_cache, "unchanged mtime should hit cache");
        assert_eq!(second.sha256, first.sha256);
        assert!(
            blob_uploaded(&conn, "asset-1").unwrap(),
            "cache hit must not reset uploaded"
        );

        // Change the file content AND its mtime: cache must invalidate, re-hash,
        // and reset uploaded=0.
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(&file, b"world!!").expect("rewrite");
        // Force a newer mtime explicitly so the test is robust on coarse clocks.
        let new_mtime = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        filetime_set(&file, new_mtime);

        let third = resolve_blob_digest(&conn, "asset-1", &file).expect("third");
        assert!(!third.from_cache, "changed mtime must miss cache");
        assert_ne!(third.sha256, first.sha256, "content changed → new hash");
        assert!(
            !blob_uploaded(&conn, "asset-1").unwrap(),
            "re-hash must reset uploaded"
        );
    }

    /// Sets a file's mtime using only std (no `filetime` crate): re-open and use
    /// `set_modified` (stable since Rust 1.75 via `File::set_modified`).
    fn filetime_set(path: &Path, when: std::time::SystemTime) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open for mtime");
        file.set_modified(when).expect("set mtime");
    }

    #[test]
    fn asset_payload_to_wire_omits_path_and_adds_integrity() {
        let payload = serde_json::json!({
            "id": "a1",
            "item_id": "i1",
            "path": "/abs/local/assets/c/i/x.png",
            "type": "image",
            "size": 0,
            "sort_index": 2,
            "created_at": 123
        });
        let wire = asset_payload_to_wire(payload, "assets/c/i/x.png", "deadbeef", 456);
        let obj = wire.as_object().expect("object");
        assert!(!obj.contains_key("path"), "absolute path must be omitted");
        assert_eq!(obj["rel_path"], "assets/c/i/x.png");
        assert_eq!(obj["sha256"], "deadbeef");
        assert_eq!(obj["size"], 456, "size reflects the real file size");
        // Untouched keys survive.
        assert_eq!(obj["id"], "a1");
        assert_eq!(obj["item_id"], "i1");
        assert_eq!(obj["type"], "image");
        assert_eq!(obj["sort_index"], 2);
    }

    #[test]
    fn reset_blob_uploaded_clears_flag() {
        let conn = new_synced_test_db();
        conn.execute(
            "INSERT INTO sync_blob_index(asset_id,sha256,size,file_mtime_ms,uploaded)
             VALUES('a1','h',1,1,1)",
            [],
        )
        .expect("seed");
        assert!(blob_uploaded(&conn, "a1").unwrap());
        reset_blob_uploaded(&conn, "a1").expect("reset");
        assert!(!blob_uploaded(&conn, "a1").unwrap());
    }

    // ---- blob download (C3) ----

    use crate::sync::test_support::MockSyncApi;

    /// sha256("hello") and its bytes, used across the download tests.
    const HELLO_SHA: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    fn tmp_app_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("assets")).expect("assets dir");
        dir
    }

    fn enqueue(conn: &Connection, asset_id: &str, sha: &str, rel_path: &str, size: i64) {
        conn.execute(
            "INSERT INTO sync_pending_blobs(asset_id,sha256,rel_path,size) VALUES(?1,?2,?3,?4)",
            rusqlite::params![asset_id, sha, rel_path, size],
        )
        .expect("enqueue pending blob");
    }

    #[tokio::test]
    async fn drain_downloads_verified_blob_and_clears_queue() {
        let conn = new_synced_test_db();
        let dir = tmp_app_dir();
        let api = MockSyncApi::default();
        api.put_blob_bytes(HELLO_SHA, b"hello".to_vec());
        enqueue(&conn, "a1", HELLO_SHA, "assets/c1/i1/blob.bin", 5);

        let downloaded = drain_pending_blobs(&api, "tok", &conn, dir.path())
            .await
            .expect("drain");
        assert_eq!(downloaded, 1);

        // Final file present, verified, .part gone, queue empty.
        let final_path = dir.path().join("assets/c1/i1/blob.bin");
        assert!(final_path.exists(), "verified blob written");
        assert_eq!(std::fs::read(&final_path).unwrap(), b"hello");
        assert!(!part_path(&final_path).exists(), "temp removed");
        let queued: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_pending_blobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(queued, 0);
    }

    #[tokio::test]
    async fn drain_rejects_wrong_bytes_no_final_file_journal_and_still_queued() {
        let conn = new_synced_test_db();
        let dir = tmp_app_dir();
        let api = MockSyncApi::default();
        // Server serves DIFFERENT bytes than the declared sha256.
        api.put_blob_bytes(HELLO_SHA, b"WRONG".to_vec());
        enqueue(&conn, "a1", HELLO_SHA, "assets/c1/i1/blob.bin", 5);

        let downloaded = drain_pending_blobs(&api, "tok", &conn, dir.path())
            .await
            .expect("drain");
        assert_eq!(downloaded, 0);

        let final_path = dir.path().join("assets/c1/i1/blob.bin");
        assert!(!final_path.exists(), "no final file on mismatch");
        assert!(!part_path(&final_path).exists(), "temp cleaned up");
        // Journaled blob_hash_mismatch, still queued with bumped retry.
        let mismatch: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_conflicts WHERE reason='blob_hash_mismatch' AND row_id='a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mismatch, 1);
        let (count, retry): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(MAX(retry_count),0) FROM sync_pending_blobs WHERE asset_id='a1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "still queued");
        assert_eq!(retry, 1, "retry bumped");
    }

    #[tokio::test]
    async fn drain_aborts_oversize_stream_and_journals() {
        let conn = new_synced_test_db();
        let dir = tmp_app_dir();
        let api = MockSyncApi::default();
        // Declared size is smaller than the actual bytes → abort over declared.
        api.put_blob_bytes(HELLO_SHA, b"hello".to_vec());
        enqueue(&conn, "a1", HELLO_SHA, "assets/c1/i1/blob.bin", 2);

        let downloaded = drain_pending_blobs(&api, "tok", &conn, dir.path())
            .await
            .expect("drain");
        assert_eq!(downloaded, 0);
        let final_path = dir.path().join("assets/c1/i1/blob.bin");
        assert!(!final_path.exists());
        assert!(!part_path(&final_path).exists());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sync_conflicts WHERE reason='blob_hash_mismatch' AND row_id='a1'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn drain_404_journals_blob_missing_after_bounded_retries() {
        let conn = new_synced_test_db();
        let dir = tmp_app_dir();
        let api = MockSyncApi::default();
        // No bytes stored → blob_get returns 404. Pre-set retry to the threshold-1
        // so a single drain crosses MAX_BLOB_DOWNLOAD_RETRIES.
        enqueue(&conn, "a1", HELLO_SHA, "assets/c1/i1/blob.bin", 5);
        conn.execute(
            "UPDATE sync_pending_blobs SET retry_count = ?1 WHERE asset_id='a1'",
            [MAX_BLOB_DOWNLOAD_RETRIES - 1],
        )
        .unwrap();

        let downloaded = drain_pending_blobs(&api, "tok", &conn, dir.path())
            .await
            .expect("drain");
        assert_eq!(downloaded, 0);
        // Journaled blob_missing and removed from the queue.
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sync_conflicts WHERE reason='blob_missing' AND row_id='a1'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM sync_pending_blobs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "dropped after bounded retries"
        );
    }

    #[tokio::test]
    async fn drain_404_keeps_queued_below_retry_threshold() {
        let conn = new_synced_test_db();
        let dir = tmp_app_dir();
        let api = MockSyncApi::default();
        enqueue(&conn, "a1", HELLO_SHA, "assets/c1/i1/blob.bin", 5);

        drain_pending_blobs(&api, "tok", &conn, dir.path())
            .await
            .expect("drain");
        let (count, retry): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(MAX(retry_count),0) FROM sync_pending_blobs",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "still queued below threshold");
        assert_eq!(retry, 1);
    }

    #[test]
    fn cleanup_orphan_parts_removes_only_part_files() {
        let dir = tmp_app_dir();
        let nested = dir.path().join("assets/c1/i1");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("good.png"), b"keep").unwrap();
        std::fs::write(nested.join("good.png.part"), b"junk").unwrap();
        std::fs::write(nested.join("other.bin.part"), b"junk").unwrap();

        let removed = cleanup_orphan_parts(dir.path()).expect("cleanup");
        assert_eq!(removed, 2, "two .part files removed");
        assert!(nested.join("good.png").exists(), "real file kept");
        assert!(!nested.join("good.png.part").exists());
        assert!(!nested.join("other.bin.part").exists());
    }

    #[test]
    fn reverify_all_blobs_resets_uploaded_flags() {
        let conn = new_synced_test_db();
        conn.execute_batch(
            "INSERT INTO sync_blob_index(asset_id,sha256,size,file_mtime_ms,uploaded) VALUES('a1','h1',1,1,1);
             INSERT INTO sync_blob_index(asset_id,sha256,size,file_mtime_ms,uploaded) VALUES('a2','h2',1,1,1);",
        )
        .unwrap();
        let reset = reverify_all_blobs(&conn).expect("reverify");
        assert_eq!(reset, 2);
        let any_uploaded: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_blob_index WHERE uploaded=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(any_uploaded, 0, "all flags reset, hashes retained");
    }
}
