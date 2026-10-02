//! The durable queue of copies to Zotero.
//!
//! One row per (source, capture, library). The row is created by the person's
//! request, is the only thing a drain works from, and records where the copy
//! ended up (`item_key`), so a second request is answered from it and a source
//! deleted later leaves no row behind ([`forget_source`]). The table is
//! device-local and created here at runtime: it is not part of the sync set and
//! needs no migration (same approach as `sync_web_pending_blobs`).

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::processing::repository::now_ms;

pub const STATE_QUEUED: &str = "queued";
/// Zotero did not answer: the copy waits for it.
pub const STATE_WAITING: &str = "waiting";
pub const STATE_RUNNING: &str = "running";
/// A new item was created in Zotero.
pub const STATE_COPIED: &str = "copied";
/// The item was already in Zotero and was linked instead of duplicated.
pub const STATE_LINKED: &str = "linked";
pub const STATE_FAILED: &str = "failed";
pub const STATE_CANCELLED: &str = "cancelled";

const DDL: &str = "
CREATE TABLE IF NOT EXISTS navegador_zotero_copies (
  id            TEXT PRIMARY KEY,
  source_id     TEXT NOT NULL,
  capture_id    TEXT NOT NULL DEFAULT '',
  library_type  TEXT NOT NULL CHECK (library_type IN ('user','group')),
  library_id    TEXT NOT NULL,
  library_name  TEXT,
  state         TEXT NOT NULL CHECK (state IN
                  ('queued','waiting','running','copied','linked','failed','cancelled')),
  item_key      TEXT,
  detail_json   TEXT,
  error_code    TEXT,
  error_message TEXT,
  attempts      INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  UNIQUE (source_id, capture_id, library_type, library_id)
);
CREATE INDEX IF NOT EXISTS idx_navegador_zotero_copies_state
  ON navegador_zotero_copies (state, created_at);
";

/// The library a copy is for, as the person chose it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRef {
    /// `user` or `group`.
    pub library_type: String,
    /// `0` for the personal library, the numeric group id otherwise.
    pub library_id: String,
    pub library_name: Option<String>,
}

/// One row of the queue, as the UI reads it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroCopy {
    pub id: String,
    pub source_id: String,
    /// The PDF capture that rides along, if any.
    pub capture_id: Option<String>,
    pub library_type: String,
    pub library_id: String,
    pub library_name: Option<String>,
    pub state: String,
    pub item_key: Option<String>,
    /// What the last run found (see `run`), as JSON.
    pub detail: Option<serde_json::Value>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub attempts: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

fn db_error(error: impl std::fmt::Display) -> String {
    format!("db_error: {error}")
}

fn refuse(code: &str, message: &str) -> String {
    format!("{code}: {message}")
}

pub fn ensure_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(DDL).map_err(db_error)
}

const COLUMNS: &str = "id, source_id, capture_id, library_type, library_id, library_name, state,
    item_key, detail_json, error_code, error_message, attempts, created_at, updated_at";

fn read(row: &Row<'_>) -> rusqlite::Result<ZoteroCopy> {
    let capture: String = row.get(2)?;
    let detail: Option<String> = row.get(8)?;
    Ok(ZoteroCopy {
        id: row.get(0)?,
        source_id: row.get(1)?,
        capture_id: (!capture.is_empty()).then_some(capture),
        library_type: row.get(3)?,
        library_id: row.get(4)?,
        library_name: row.get(5)?,
        state: row.get(6)?,
        item_key: row.get(7)?,
        detail: detail.and_then(|json| serde_json::from_str(&json).ok()),
        error_code: row.get(9)?,
        error_message: row.get(10)?,
        attempts: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<ZoteroCopy>, String> {
    ensure_table(conn)?;
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM navegador_zotero_copies WHERE id = ?1"),
        [id],
        read,
    )
    .optional()
    .map_err(db_error)
}

fn require(conn: &Connection, id: &str) -> Result<ZoteroCopy, String> {
    get(conn, id)?.ok_or_else(|| refuse("not_found", "there is no such copy"))
}

/// The copies, newest first, of one source or of all of them.
pub fn list(conn: &Connection, source_id: Option<&str>) -> Result<Vec<ZoteroCopy>, String> {
    ensure_table(conn)?;
    let mut statement = conn
        .prepare(&format!(
            "SELECT {COLUMNS} FROM navegador_zotero_copies
             WHERE (?1 IS NULL OR source_id = ?1)
             ORDER BY created_at DESC, id"
        ))
        .map_err(db_error)?;
    let rows = statement
        .query_map([source_id], read)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(rows)
}

/// What a drain works on: queued and waiting copies, oldest first.
pub fn pending(conn: &Connection) -> Result<Vec<ZoteroCopy>, String> {
    ensure_table(conn)?;
    let mut statement = conn
        .prepare(&format!(
            "SELECT {COLUMNS} FROM navegador_zotero_copies
             WHERE state IN ('queued','waiting') ORDER BY created_at, id"
        ))
        .map_err(db_error)?;
    let rows = statement
        .query_map([], read)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(rows)
}

/// Asks for one copy. The same source, capture and library answer with the row
/// that exists; a failed or cancelled one is queued again, a finished one is not.
pub fn request(
    conn: &Connection,
    source_id: &str,
    capture_id: Option<&str>,
    library: &LibraryRef,
) -> Result<ZoteroCopy, String> {
    let library_type = library.library_type.trim();
    let library_id = library.library_id.trim();
    if !matches!(library_type, "user" | "group")
        || library_id.is_empty()
        || !library_id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(refuse(
            "invalid_library",
            "the library is not a Zotero library",
        ));
    }
    let library_name = library
        .library_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());

    let known: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM web_sources WHERE id = ?1",
            [source_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    if known.is_none() {
        return Err(refuse("not_found", "there is no such source"));
    }
    if let Some(capture) = capture_id {
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT web_source_id, kind FROM web_captures WHERE id = ?1",
                [capture],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        match row {
            Some((owner, kind)) if owner == source_id => {
                if kind != "pdf" {
                    return Err(refuse("not_a_pdf", "only a saved PDF goes along"));
                }
            }
            _ => return Err(refuse("not_found", "there is no such capture")),
        }
    }

    ensure_table(conn)?;
    let capture_key = capture_id.unwrap_or("");
    let existing: Option<ZoteroCopy> = conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM navegador_zotero_copies
                 WHERE source_id = ?1 AND capture_id = ?2 AND library_type = ?3 AND library_id = ?4"
            ),
            params![source_id, capture_key, library_type, library_id],
            read,
        )
        .optional()
        .map_err(db_error)?;
    if let Some(row) = existing {
        if row.state == STATE_FAILED || row.state == STATE_CANCELLED {
            conn.execute(
                "UPDATE navegador_zotero_copies
                 SET state = 'queued', error_code = NULL, error_message = NULL,
                     library_name = COALESCE(?2, library_name), updated_at = ?3
                 WHERE id = ?1",
                params![row.id, library_name, now_ms()],
            )
            .map_err(db_error)?;
            return require(conn, &row.id);
        }
        return Ok(row);
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = now_ms();
    conn.execute(
        "INSERT INTO navegador_zotero_copies
           (id, source_id, capture_id, library_type, library_id, library_name, state,
            attempts, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', 0, ?7, ?7)",
        params![
            id,
            source_id,
            capture_key,
            library_type,
            library_id,
            library_name,
            now
        ],
    )
    .map_err(db_error)?;
    require(conn, &id)
}

fn invalid_transition(from: &str, to: &str) -> String {
    refuse(
        "invalid_transition",
        &format!("a copy cannot go from {from} to {to}"),
    )
}

/// `queued` or `waiting` to `running`, counting the attempt.
pub fn claim(conn: &Connection, id: &str) -> Result<ZoteroCopy, String> {
    let row = require(conn, id)?;
    if row.state != STATE_QUEUED && row.state != STATE_WAITING {
        return Err(invalid_transition(&row.state, STATE_RUNNING));
    }
    conn.execute(
        "UPDATE navegador_zotero_copies
         SET state = 'running', attempts = attempts + 1,
             error_code = NULL, error_message = NULL, updated_at = ?2
         WHERE id = ?1",
        params![id, now_ms()],
    )
    .map_err(db_error)?;
    require(conn, id)
}

/// Back to the queue because Zotero did not answer: not a failure.
pub fn wait(conn: &Connection, id: &str) -> Result<ZoteroCopy, String> {
    let row = require(conn, id)?;
    if row.state != STATE_RUNNING && row.state != STATE_QUEUED {
        return Err(invalid_transition(&row.state, STATE_WAITING));
    }
    conn.execute(
        "UPDATE navegador_zotero_copies SET state = 'waiting', updated_at = ?2 WHERE id = ?1",
        params![id, now_ms()],
    )
    .map_err(db_error)?;
    require(conn, id)
}

/// `running` to `copied` or `linked`, with the Zotero item key and what the run found.
pub fn finish(
    conn: &Connection,
    id: &str,
    state: &str,
    item_key: Option<&str>,
    detail_json: &str,
) -> Result<ZoteroCopy, String> {
    if state != STATE_COPIED && state != STATE_LINKED {
        return Err(invalid_transition(STATE_RUNNING, state));
    }
    let row = require(conn, id)?;
    if row.state != STATE_RUNNING {
        return Err(invalid_transition(&row.state, state));
    }
    conn.execute(
        "UPDATE navegador_zotero_copies
         SET state = ?2, item_key = ?3, detail_json = ?4,
             error_code = NULL, error_message = NULL, updated_at = ?5
         WHERE id = ?1",
        params![id, state, item_key, detail_json, now_ms()],
    )
    .map_err(db_error)?;
    require(conn, id)
}

pub fn fail(conn: &Connection, id: &str, code: &str, message: &str) -> Result<ZoteroCopy, String> {
    let row = require(conn, id)?;
    if !matches!(
        row.state.as_str(),
        STATE_RUNNING | STATE_QUEUED | STATE_WAITING
    ) {
        return Err(invalid_transition(&row.state, STATE_FAILED));
    }
    conn.execute(
        "UPDATE navegador_zotero_copies
         SET state = 'failed', error_code = ?2, error_message = ?3, updated_at = ?4
         WHERE id = ?1",
        params![id, code, message, now_ms()],
    )
    .map_err(db_error)?;
    require(conn, id)
}

/// Only a copy that has not started and not finished can be cancelled.
pub fn cancel(conn: &Connection, id: &str) -> Result<ZoteroCopy, String> {
    let row = require(conn, id)?;
    if row.state != STATE_QUEUED && row.state != STATE_WAITING {
        return Err(invalid_transition(&row.state, STATE_CANCELLED));
    }
    conn.execute(
        "UPDATE navegador_zotero_copies SET state = 'cancelled', updated_at = ?2 WHERE id = ?1",
        params![id, now_ms()],
    )
    .map_err(db_error)?;
    require(conn, id)
}

/// Crash recovery: a `running` row is parked back in the queue. Safe because a
/// run looks the item up by address before it writes.
pub fn recover(conn: &Connection) -> Result<usize, String> {
    ensure_table(conn)?;
    conn.execute(
        "UPDATE navegador_zotero_copies SET state = 'queued', updated_at = ?1
         WHERE state = 'running'",
        [now_ms()],
    )
    .map_err(db_error)
}

/// The item key a finished copy of this source recorded for this library.
pub fn recorded_item(
    conn: &Connection,
    source_id: &str,
    library_type: &str,
    library_id: &str,
) -> Result<Option<String>, String> {
    ensure_table(conn)?;
    conn.query_row(
        "SELECT item_key FROM navegador_zotero_copies
         WHERE source_id = ?1 AND library_type = ?2 AND library_id = ?3
           AND state IN ('copied','linked') AND item_key IS NOT NULL
         ORDER BY created_at LIMIT 1",
        params![source_id, library_type, library_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_error)
}

/// Drops the result of finished copies whose Zotero item is gone, so a new
/// request queues them again instead of answering with a stale item key.
pub fn forget_result(
    conn: &Connection,
    source_id: &str,
    library_type: &str,
    library_id: &str,
) -> Result<(), String> {
    ensure_table(conn)?;
    conn.execute(
        "UPDATE navegador_zotero_copies
         SET state = 'queued', item_key = NULL, detail_json = NULL, updated_at = ?4
         WHERE source_id = ?1 AND library_type = ?2 AND library_id = ?3
           AND state IN ('copied','linked')",
        params![source_id, library_type, library_id, now_ms()],
    )
    .map_err(db_error)?;
    Ok(())
}

/// A deleted source takes its rows with it. Never fails for a missing table.
pub fn forget_source(conn: &Connection, source_id: &str) -> Result<(), String> {
    let present: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'navegador_zotero_copies'",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if present == 0 {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM navegador_zotero_copies WHERE source_id = ?1",
        [source_id],
    )
    .map_err(db_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::test_support::new_app_schema_db;
    use rusqlite::Connection;

    fn db() -> Connection {
        let conn = new_app_schema_db();
        conn.execute(
            "INSERT INTO web_sources (id, original_url, final_url, title, first_accessed_at, created_at, updated_at)
             VALUES ('src1', 'https://a.test/x', 'https://a.test/x', 'T', '2026-10-01T10:00:00Z', 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO web_captures (id, web_source_id, accessed_at, final_url, kind, mime_type, sha256, hash_of, size_bytes, created_at)
             VALUES ('cap-pdf', 'src1', '2026-10-01T10:00:00Z', 'https://a.test/x.pdf', 'pdf', 'application/pdf', 'aa', 'pdf', 3, 1),
                    ('cap-page', 'src1', '2026-10-01T10:00:00Z', 'https://a.test/x', 'page', 'text/html', 'bb', 'html', 3, 1)",
            [],
        )
        .unwrap();
        conn
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

    #[test]
    fn a_request_is_queued_once_per_source_capture_and_library() {
        let conn = db();
        let first = request(&conn, "src1", None, &personal()).unwrap();
        let again = request(&conn, "src1", None, &personal()).unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(first.state, STATE_QUEUED);
        let other_library = request(&conn, "src1", None, &group()).unwrap();
        assert_ne!(first.id, other_library.id);
        let with_pdf = request(&conn, "src1", Some("cap-pdf"), &personal()).unwrap();
        assert_ne!(first.id, with_pdf.id);
        assert_eq!(list(&conn, Some("src1")).unwrap().len(), 3);
    }

    #[test]
    fn only_a_pdf_capture_of_that_source_can_ride_along() {
        let conn = db();
        let page = request(&conn, "src1", Some("cap-page"), &personal()).unwrap_err();
        assert!(page.starts_with("not_a_pdf"), "{page}");
        let ghost = request(&conn, "src1", Some("nope"), &personal()).unwrap_err();
        assert!(ghost.starts_with("not_found"), "{ghost}");
        let no_source = request(&conn, "ghost", None, &personal()).unwrap_err();
        assert!(no_source.starts_with("not_found"), "{no_source}");
    }

    #[test]
    fn a_blank_library_is_refused() {
        let conn = db();
        let blank = LibraryRef {
            library_type: "group".into(),
            library_id: "  ".into(),
            library_name: None,
        };
        assert!(request(&conn, "src1", None, &blank)
            .unwrap_err()
            .starts_with("invalid_library"));
        let odd = LibraryRef {
            library_type: "team".into(),
            library_id: "1".into(),
            library_name: None,
        };
        assert!(request(&conn, "src1", None, &odd)
            .unwrap_err()
            .starts_with("invalid_library"));
    }

    #[test]
    fn a_finished_copy_is_never_requeued_but_a_failed_one_is() {
        let conn = db();
        let row = request(&conn, "src1", None, &personal()).unwrap();
        claim(&conn, &row.id).unwrap();
        finish(&conn, &row.id, STATE_COPIED, Some("ABCD2345"), "{}").unwrap();
        let again = request(&conn, "src1", None, &personal()).unwrap();
        assert_eq!(again.state, STATE_COPIED);
        assert_eq!(again.item_key.as_deref(), Some("ABCD2345"));

        let other = request(&conn, "src1", None, &group()).unwrap();
        claim(&conn, &other.id).unwrap();
        fail(&conn, &other.id, "zotero_rejected", "no").unwrap();
        let retried = request(&conn, "src1", None, &group()).unwrap();
        assert_eq!(retried.id, other.id);
        assert_eq!(retried.state, STATE_QUEUED);
        assert_eq!(retried.error_code, None);
    }

    #[test]
    fn claim_counts_an_attempt_and_waiting_rows_can_be_claimed() {
        let conn = db();
        let row = request(&conn, "src1", None, &personal()).unwrap();
        let running = claim(&conn, &row.id).unwrap();
        assert_eq!(running.state, STATE_RUNNING);
        assert_eq!(running.attempts, 1);
        assert!(
            claim(&conn, &row.id).is_err(),
            "a running row is not claimable"
        );
        wait(&conn, &row.id).unwrap();
        let waiting = get(&conn, &row.id).unwrap().unwrap();
        assert_eq!(waiting.state, STATE_WAITING);
        let again = claim(&conn, &row.id).unwrap();
        assert_eq!(again.attempts, 2);
    }

    #[test]
    fn cancel_only_takes_rows_that_have_not_started_or_finished() {
        let conn = db();
        let row = request(&conn, "src1", None, &personal()).unwrap();
        assert_eq!(cancel(&conn, &row.id).unwrap().state, STATE_CANCELLED);
        assert!(cancel(&conn, &row.id).is_err());
        let again = request(&conn, "src1", None, &personal()).unwrap();
        assert_eq!(
            again.state, STATE_QUEUED,
            "a cancelled copy can be asked again"
        );
    }

    #[test]
    fn recovery_parks_running_rows_back_in_the_queue() {
        let conn = db();
        let row = request(&conn, "src1", None, &personal()).unwrap();
        claim(&conn, &row.id).unwrap();
        assert_eq!(recover(&conn).unwrap(), 1);
        assert_eq!(get(&conn, &row.id).unwrap().unwrap().state, STATE_QUEUED);
    }

    #[test]
    fn pending_rows_come_oldest_first() {
        let conn = db();
        let a = request(&conn, "src1", None, &personal()).unwrap();
        let b = request(&conn, "src1", None, &group()).unwrap();
        conn.execute(
            "UPDATE navegador_zotero_copies SET created_at = 5 WHERE id = ?1",
            [&a.id],
        )
        .unwrap();
        conn.execute(
            "UPDATE navegador_zotero_copies SET created_at = 1 WHERE id = ?1",
            [&b.id],
        )
        .unwrap();
        let ids: Vec<String> = pending(&conn).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![b.id, a.id]);
    }

    #[test]
    fn forgetting_a_source_drops_its_rows_and_works_without_the_table() {
        let conn = db();
        request(&conn, "src1", None, &personal()).unwrap();
        forget_source(&conn, "src1").unwrap();
        assert!(list(&conn, None).unwrap().is_empty());
        let bare = new_app_schema_db();
        forget_source(&bare, "src1").unwrap();
    }

    #[test]
    fn rows_reach_the_ui_in_camel_case() {
        let conn = db();
        let row = request(&conn, "src1", Some("cap-pdf"), &group()).unwrap();
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["sourceId"], "src1");
        assert_eq!(json["captureId"], "cap-pdf");
        assert_eq!(json["libraryType"], "group");
        assert_eq!(json["libraryName"], "prueba");
        assert_eq!(json["state"], "queued");
    }
}
