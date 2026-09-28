//! Outbox capture and capability state for the inactive writing sync
//! transport.
//!
//! `writing_envelopes` is a virtual wire table: the aggregate is built from the
//! canonical writing tables at push time, so the generic SQL-trigger capture of
//! `sync/capture.rs` must not (and cannot) run for it. Instead, the repository
//! write paths call [`enqueue_document`] directly, and the entries live in
//! `sync_meta` under `writing_outbox:<doc_id>` — the same durable key-value the
//! sync engine already owns, so no new migration is needed and the generic
//! oplog/coalesce path is never polluted with rows it cannot read.
//!
//! Nothing here talks to the network. The future transport consumes
//! [`outbox_entries`], builds envelopes through `sync_envelope` +
//! `sync_files`, and acknowledges the exact captured generation through
//! [`acknowledge_outbox_entry`]; capability discovery gates everything through
//! [`record_capability`] / [`supports_writing`].

use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use uuid::Uuid;

use super::repository::{now_ms, WritingError, WritingResult};

pub(super) const OUTBOX_PREFIX: &str = "writing_outbox:";
const CAPABILITY_KEY: &str = "writing_capability";
pub(super) const MANIFEST_PREFIX: &str = "writing_manifest:";
pub(super) const PENDING_ASSETS_PREFIX: &str = "writing_pending_assets:";
pub(crate) const RECEIVE_PREFIX: &str = "writing_receive:";
pub(super) const CATCHUP_EPOCH_KEY: &str = "writing_catchup_epoch";
pub(crate) const ENVELOPE_TABLE: &str = "writing_envelopes";
/// The writing-only pull staging cursor owned by `sync::writing_pull`. It is
/// bound to the account/epoch/session that wrote it and must die with the
/// account's writing metadata, like every other whole key above.
pub(crate) const PULL_CURSOR_KEY: &str = "writing_pull_cursor";

/// The identity a future push response must present to acknowledge one exact
/// captured outbox generation. Legacy entries have no generation and use the
/// exact stored value as their conservative compare-and-delete identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutboxAcknowledgment {
    pub(crate) document_id: String,
    pub(crate) generation: Option<String>,
    captured_value: String,
}

/// One pending document change, coalesced per document (last capture wins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutboxEntry {
    pub(crate) document_id: String,
    pub(crate) op: char,
    pub(crate) changed_at: i64,
    pub(crate) acknowledgment: OutboxAcknowledgment,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredOutboxEntry {
    op: String,
    changed_at: i64,
    #[serde(default)]
    generation: Option<String>,
}

/// Clears every account-bound metadata entry owned by writing sync.
///
/// The caller supplies the account-reset transaction. Prefixes are compared as
/// literal substrings because SQL `LIKE` would treat their underscores as
/// wildcards and could delete unrelated metadata.
pub(crate) fn clear_account_metadata(conn: &Connection) -> WritingResult<()> {
    conn.execute(
        "DELETE FROM sync_meta
          WHERE key = ?1
             OR key = ?2
             OR key = ?3
             OR substr(key, 1, length(?4)) = ?4
             OR substr(key, 1, length(?5)) = ?5
             OR substr(key, 1, length(?6)) = ?6
             OR substr(key, 1, length(?7)) = ?7",
        params![
            CAPABILITY_KEY,
            CATCHUP_EPOCH_KEY,
            PULL_CURSOR_KEY,
            OUTBOX_PREFIX,
            MANIFEST_PREFIX,
            PENDING_ASSETS_PREFIX,
            RECEIVE_PREFIX,
        ],
    )
    .map(|_| ())
    .map_err(|error| WritingError::sql("Failed to clear writing sync metadata", error))
}

/// Records one pending document change. A no-op when sync capture is off
/// (`capture_enabled`/`applying` gates, or no sync schema at all), exactly like
/// the corpus triggers: without a session the document simply stays local
/// until backfill runs.
pub(crate) fn enqueue_document(conn: &Connection, document_id: &str) -> WritingResult<()> {
    enqueue_document_at(conn, document_id, now_ms())
}

fn enqueue_document_at(conn: &Connection, document_id: &str, changed_at: i64) -> WritingResult<()> {
    if !capture_active(conn)? {
        return Ok(());
    }
    let value = new_outbox_value(changed_at);
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![format!("{OUTBOX_PREFIX}{document_id}"), value],
    )
    .map(|_| ())
    .map_err(|error| WritingError::sql("Failed to enqueue writing document", error))
}

#[cfg(test)]
pub(crate) fn enqueue_document_at_for_test(
    conn: &Connection,
    document_id: &str,
    changed_at: i64,
) -> WritingResult<()> {
    enqueue_document_at(conn, document_id, changed_at)
}

fn new_outbox_value(changed_at: i64) -> String {
    serde_json::json!({
        "op": "U",
        "changed_at": changed_at,
        "generation": Uuid::new_v4().to_string(),
    })
    .to_string()
}

/// Enqueues every document the server has never acknowledged
/// (`sync_row_versions` has no `(writing_envelopes, id)` entry). Idempotent:
/// re-running adds nothing new, never replaces pending work, and gives each
/// newly seeded document its own generation.
pub(crate) fn seed_outbox(conn: &Connection) -> WritingResult<usize> {
    let document_ids = {
        let mut stmt = conn
            .prepare(
                "SELECT d.id
                   FROM writing_documents d
                  WHERE NOT EXISTS (
                        SELECT 1 FROM sync_row_versions v
                         WHERE v.table_name = ?1 AND v.row_id = d.id)
                    AND NOT EXISTS (
                        SELECT 1 FROM sync_meta m
                         WHERE m.key = ?2 || d.id)
                  ORDER BY d.id",
            )
            .map_err(|error| WritingError::sql("Failed to inspect writing outbox seed", error))?;
        let rows = stmt
            .query_map(params![ENVELOPE_TABLE, OUTBOX_PREFIX], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| WritingError::sql("Failed to inspect writing outbox seed", error))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| WritingError::sql("Failed to inspect writing outbox seed", error))?;
        rows
    };

    let mut seeded = 0;
    for document_id in document_ids {
        seeded += conn
            .execute(
                "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO NOTHING",
                params![
                    format!("{OUTBOX_PREFIX}{document_id}"),
                    new_outbox_value(now_ms()),
                ],
            )
            .map_err(|error| WritingError::sql("Failed to seed writing outbox", error))?;
    }
    Ok(seeded)
}

/// Pending entries in stable (changed_at, document_id) order.
pub(crate) fn outbox_entries(conn: &Connection) -> WritingResult<Vec<OutboxEntry>> {
    let mut stmt = conn
        .prepare(
            "SELECT key, value
               FROM sync_meta
              WHERE substr(key, 1, length(?1)) = ?1
              ORDER BY key",
        )
        .map_err(|error| WritingError::sql("Failed to read writing outbox", error))?;
    let rows = stmt
        .query_map(params![OUTBOX_PREFIX], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| WritingError::sql("Failed to read writing outbox", error))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| WritingError::sql("Failed to read writing outbox", error))?;

    let mut entries: Vec<OutboxEntry> = rows
        .into_iter()
        .map(|(key, raw)| parse_outbox_entry(&key, raw))
        .collect::<WritingResult<_>>()?;
    entries.sort_by(|a, b| (a.changed_at, &a.document_id).cmp(&(b.changed_at, &b.document_id)));
    Ok(entries)
}

fn parse_outbox_entry(key: &str, raw: String) -> WritingResult<OutboxEntry> {
    let document_id = key
        .strip_prefix(OUTBOX_PREFIX)
        .filter(|document_id| !document_id.is_empty())
        .ok_or_else(|| corrupt_outbox(format!("invalid outbox key {key:?}")))?
        .to_string();
    let stored: StoredOutboxEntry = serde_json::from_str(&raw)
        .map_err(|error| corrupt_outbox(format!("cannot parse outbox entry {key:?}: {error}")))?;
    let op = match stored.op.as_str() {
        "U" => 'U',
        other => {
            return Err(corrupt_outbox(format!(
                "outbox entry {key:?} has unsupported op {other:?}"
            )))
        }
    };
    if stored.changed_at < 0 {
        return Err(corrupt_outbox(format!(
            "outbox entry {key:?} has negative changed_at {}",
            stored.changed_at
        )));
    }
    if let Some(generation) = &stored.generation {
        let parsed = Uuid::parse_str(generation).map_err(|error| {
            corrupt_outbox(format!(
                "outbox entry {key:?} has invalid generation: {error}"
            ))
        })?;
        if parsed.to_string() != *generation {
            return Err(corrupt_outbox(format!(
                "outbox entry {key:?} has a non-canonical generation"
            )));
        }
    }

    Ok(OutboxEntry {
        document_id: document_id.clone(),
        op,
        changed_at: stored.changed_at,
        acknowledgment: OutboxAcknowledgment {
            document_id,
            generation: stored.generation,
            captured_value: raw,
        },
    })
}

fn corrupt_outbox(message: impl Into<String>) -> WritingError {
    WritingError::new("writing_outbox_corrupt", message)
}

/// Removes an entry only when it is still byte-for-byte the generation that a
/// push draft captured. Replaying an acknowledgment is a harmless no-op, and a
/// stale acknowledgment can never remove newer work. Exact raw comparison also
/// gives legacy pre-generation entries a conservative CAS identity.
pub(crate) fn acknowledge_outbox_entry(
    conn: &Connection,
    acknowledgment: &OutboxAcknowledgment,
) -> WritingResult<bool> {
    conn.execute(
        "DELETE FROM sync_meta WHERE key = ?1 AND value = ?2",
        params![
            format!("{OUTBOX_PREFIX}{}", acknowledgment.document_id),
            acknowledgment.captured_value,
        ],
    )
    .map(|deleted| deleted != 0)
    .map_err(|error| WritingError::sql("Failed to acknowledge writing outbox entry", error))
}

/// Unconditionally discards pending work because an authoritative remote
/// replacement or tombstone won. The caller must run this in the same
/// savepoint as that authoritative mutation. Network push acknowledgments must
/// use [`acknowledge_outbox_entry`] instead.
pub(crate) fn discard_outbox_for_authoritative_receive(
    conn: &Connection,
    document_id: &str,
) -> WritingResult<()> {
    conn.execute(
        "DELETE FROM sync_meta WHERE key = ?1",
        params![format!("{OUTBOX_PREFIX}{document_id}")],
    )
    .map(|_| ())
    .map_err(|error| WritingError::sql("Failed to discard superseded writing outbox entry", error))
}

/// Records whether the server advertised `writing-envelope-v1` during the
/// successful sync response of `server_epoch`. An epoch change (restore or
/// `bump-epoch`) invalidates the record until discovery runs again.
pub(crate) fn record_capability(
    conn: &Connection,
    server_epoch: &str,
    advertised: bool,
) -> WritingResult<()> {
    let value = serde_json::json!({ "epoch": server_epoch, "advertised": advertised }).to_string();
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![CAPABILITY_KEY, value],
    )
    .map(|_| ())
    .map_err(|error| WritingError::sql("Failed to record writing capability", error))
}

/// True only when discovery saw the capability advertised at exactly this
/// server epoch. Discovery from a stale epoch never enables the transport.
pub(crate) fn supports_writing(conn: &Connection, server_epoch: &str) -> WritingResult<bool> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![CAPABILITY_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| WritingError::sql("Failed to read writing capability", error))?;
    let Some(raw) = raw else {
        return Ok(false);
    };
    let parsed: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        WritingError::new(
            "writing_outbox_corrupt",
            format!("cannot parse capability record: {error}"),
        )
    })?;
    Ok(parsed
        .get("advertised")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        && parsed.get("epoch").and_then(serde_json::Value::as_str) == Some(server_epoch))
}

/// The corpus capture gates, plus "sync schema exists at all": a database with
/// only the writing migrations must never fail a save because of capture
/// bookkeeping. Once `sync_meta` exists, read failures are real sync-state
/// errors and must abort the surrounding canonical mutation.
fn capture_active(conn: &Connection) -> WritingResult<bool> {
    let sync_meta_exists = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sync_meta'
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| WritingError::sql("Failed to inspect writing sync schema", error))?
        != 0;
    if !sync_meta_exists {
        return Ok(false);
    }

    conn.query_row(
        "SELECT CASE
                 WHEN COALESCE((SELECT value FROM sync_meta WHERE key = 'applying'), '0') = '1'
                   THEN 0
                 WHEN COALESCE((SELECT value FROM sync_meta WHERE key = 'capture_enabled'), '0') = '1'
                   THEN 1
                 ELSE 0
               END",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|active| active == 1)
    .map_err(|error| WritingError::sql("Failed to read writing capture gates", error))
}
