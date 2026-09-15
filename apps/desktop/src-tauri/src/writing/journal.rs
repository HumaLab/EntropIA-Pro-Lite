//! Durable recovery journal for in-progress manuscript edits (§16.1).
//!
//! Spike S6 settled the shape: a SQLite table holding **deltas**, on the
//! connection settings this repository already uses. A ~1 KB delta costs a p95
//! of 1.17 ms with `synchronous=FULL`; the whole manuscript costs 97.5 ms. The
//! journal sustains roughly 2,800 durable appends per second, so the interval
//! between persistences is a product decision (§16.1 recommends <= 500 ms with
//! a declared loss window of <= ~560 ms), not a performance limit.
//!
//! `delta_json` is opaque here. The editor produces ProseMirror steps, this
//! module makes the sequence durable, and recovery hands it back in order.
//!
//! Appending is **not** a canonical save (§16.2). It returns a sequence number,
//! never a revision, so a caller cannot mistake one for the other.

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::repository::{now_ms, require_schema, WritingError, WritingResult, DOCUMENT_NOT_FOUND};

/// One durable delta.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct JournalEntry {
    pub seq: i64,
    pub base_revision: i64,
    pub schema_version: i64,
    pub delta_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AppendJournal {
    pub document_id: String,
    /// The canonical revision this delta applies on top of.
    pub base_revision: i64,
    pub schema_version: i64,
    pub delta_json: String,
}

/// The checksum is computed here, never accepted from the caller: a checksum
/// supplied alongside the data it guards proves nothing about the trip.
fn checksum_of(base_revision: i64, schema_version: i64, delta_json: &str) -> String {
    let digest =
        Sha256::digest(format!("{base_revision}\u{0}{schema_version}\u{0}{delta_json}").as_bytes());
    format!("{digest:x}")
}

/// Appends one delta and returns its sequence number.
pub fn append(conn: &Connection, input: AppendJournal) -> WritingResult<i64> {
    require_schema(conn)?;
    let next: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM writing_journal WHERE document_id = ?1",
            rusqlite::params![input.document_id],
            |row| row.get(0),
        )
        .map_err(|e| WritingError::sql("Failed to pick the next sequence", e))?;

    let checksum = checksum_of(input.base_revision, input.schema_version, &input.delta_json);
    conn.execute(
        "INSERT INTO writing_journal
           (document_id, seq, base_revision, schema_version, delta_json, checksum, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            input.document_id,
            next,
            input.base_revision,
            input.schema_version,
            input.delta_json,
            checksum,
            now_ms()
        ],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            WritingError::new(
                DOCUMENT_NOT_FOUND,
                format!("no document with id {}", input.document_id),
            )
        }
        other => WritingError::sql("Failed to append journal entry", other),
    })?;
    Ok(next)
}

/// Every entry recorded on top of `from_revision` or later, in sequence order,
/// each paired with whether its stored checksum still matches its content.
pub fn read_pending(
    conn: &Connection,
    document_id: &str,
    from_revision: i64,
) -> WritingResult<Vec<(JournalEntry, bool)>> {
    require_schema(conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT seq, base_revision, schema_version, delta_json, checksum, created_at
               FROM writing_journal
              WHERE document_id = ?1 AND base_revision >= ?2
              ORDER BY seq ASC",
        )
        .map_err(|e| WritingError::sql("Failed to read the journal", e))?;

    let rows = stmt
        .query_map(rusqlite::params![document_id, from_revision], |row| {
            let stored: String = row.get(4)?;
            let entry = JournalEntry {
                seq: row.get(0)?,
                base_revision: row.get(1)?,
                schema_version: row.get(2)?,
                delta_json: row.get(3)?,
                created_at: row.get(5)?,
            };
            let intact =
                checksum_of(entry.base_revision, entry.schema_version, &entry.delta_json) == stored;
            Ok((entry, intact))
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| WritingError::sql("Failed to read the journal", e))?;
    Ok(rows)
}

/// Drops entries already folded into a confirmed canonical revision.
///
/// §16.1 is explicit that a journal protecting a sequence must survive until a
/// canonical version containing it exists, so this only removes entries whose
/// `base_revision` is strictly below `confirmed_revision`. An entry recorded on
/// top of that very revision is still the only durable copy of its delta.
pub fn prune_confirmed(
    conn: &Connection,
    document_id: &str,
    confirmed_revision: i64,
) -> WritingResult<usize> {
    require_schema(conn)?;
    conn.execute(
        "DELETE FROM writing_journal WHERE document_id = ?1 AND base_revision < ?2",
        rusqlite::params![document_id, confirmed_revision],
    )
    .map_err(|e| WritingError::sql("Failed to prune the journal", e))
}

/// Removes every entry for a document, used when the human explicitly discards
/// a recovery offer (§16.3). Returns how many went, so the caller can report it.
pub fn discard_all(conn: &Connection, document_id: &str) -> WritingResult<usize> {
    require_schema(conn)?;
    conn.execute(
        "DELETE FROM writing_journal WHERE document_id = ?1",
        rusqlite::params![document_id],
    )
    .map_err(|e| WritingError::sql("Failed to discard the journal", e))
}
