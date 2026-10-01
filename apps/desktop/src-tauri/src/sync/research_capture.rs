//! Outbox capture and capability state for the inactive research sync
//! transport.
//!
//! `research_envelopes` is a virtual wire table: the aggregate is built from
//! `estado.sqlite` at push time (PROTOCOL: one opaque snapshot per terminal
//! job), so the generic SQL-trigger capture of `sync/capture.rs` must not (and
//! cannot) run for it. Instead the future engine calls
//! [`enqueue_terminal_job`] when a job closes, and
//! [`seed_terminal_outbox`] backfills the terminal jobs once the epoch's
//! catch-up gate is recorded. The entries live in `sync_meta` under
//! `research_outbox:<job_id>` — the same durable key-value the sync engine
//! already owns, so no new migration is needed and the generic oplog/coalesce
//! path is never polluted with rows it cannot read.
//!
//! Every entry is generation-bound: each captured value carries a fresh UUID
//! generation, and the future push acknowledges the exact captured generation
//! through [`acknowledge_outbox_entry`] (compare-and-delete). A stale
//! acknowledgment can never remove newer work; replaying one is a no-op.
//!
//! Nothing here talks to the network. Capture is a no-op when the sync schema
//! is absent or capture is disabled, and an active job is never enqueued —
//! the seed only reads terminal jobs and [`enqueue_terminal_job`] verifies
//! terminality against the read-only research state database. Remote tombstone
//! receive behavior lives in `research_transport`; the only delete work here is
//! the LOCAL delete-intent lifecycle: [`begin_delete_intent`] durably records
//! the user's delete request, [`settle_delete_intent`] turns a successful
//! engine delete into one idempotent `D` outbox tombstone, and
//! [`abort_delete_intent`] removes the intent after a failed delete so a
//! failed delete request never leaves an enqueued delete.

// Inactive slice: the future engine calls these helpers after catch-up.
#![allow(dead_code)]

use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use uuid::Uuid;

use super::research_envelope::{
    require_terminal_research_job, terminal_research_job_ids, ResearchError, ResearchResult,
};

/// Literal key prefix of the per-job outbox. Queries match it with `substr`,
/// never `LIKE`: the underscores would act as wildcards.
pub(crate) const OUTBOX_PREFIX: &str = "research_outbox:";
/// Durable staging of pulled `research_envelopes` rows awaiting application to
/// the separate research state database. Owned by `research_receive`; the
/// prefix lives here so account cleanup can clear every account-bound key from
/// one place.
pub(crate) const RECEIVE_PREFIX: &str = "research_receive:";
/// One durable delete intent per job, created before the destructive engine
/// delete runs and removed by settle/abort.
pub(crate) const DELETE_INTENT_PREFIX: &str = "research_delete_intent:";
pub(crate) const CAPABILITY_KEY: &str = "research_capability";
/// The research-only pull staging cursor (bound to the session that advanced
/// it). Deliberately distinct from the shared `last_pull_seq` and from the
/// writing cursor: a research page never moves corpus history.
pub(crate) const PULL_CURSOR_KEY: &str = "research_pull_cursor";
/// The one-time since-zero research catch-up marker per server epoch. An epoch
/// change (restore or `bump-epoch`) makes catch-up needed again.
pub(crate) const CATCHUP_KEY: &str = "research_catchup_epoch";
/// The negotiated wire table (PROTOCOL allowlist + capability opt-in).
pub(crate) const ENVELOPE_TABLE: &str = "research_envelopes";
/// The exact capability name negotiated with the server.
pub(crate) const RESEARCH_CAPABILITY: &str = "research-envelope-v1";

/// The identity a future push response must present to acknowledge one exact
/// captured outbox generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutboxAcknowledgment {
    pub(crate) job_id: String,
    pub(crate) generation: String,
    captured_value: String,
}

/// One pending terminal job snapshot, coalesced per job (last capture wins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutboxEntry {
    pub(crate) job_id: String,
    pub(crate) op: char,
    pub(crate) changed_at: i64,
    pub(crate) generation: String,
    pub(crate) acknowledgment: OutboxAcknowledgment,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredOutboxEntry {
    op: String,
    changed_at: i64,
    generation: String,
}

/// Clears every account-bound metadata entry owned by research sync.
///
/// The caller supplies the account-reset transaction. Prefixes are compared as
/// literal substrings because SQL `LIKE` would treat their underscores as
/// wildcards and could delete unrelated metadata.
pub(crate) fn clear_account_metadata(conn: &Connection) -> ResearchResult<()> {
    conn.execute(
        "DELETE FROM sync_meta
          WHERE key = ?1
             OR key = ?2
             OR key = ?3
             OR substr(key, 1, length(?4)) = ?4
             OR substr(key, 1, length(?5)) = ?5
             OR substr(key, 1, length(?6)) = ?6",
        params![
            CAPABILITY_KEY,
            PULL_CURSOR_KEY,
            CATCHUP_KEY,
            OUTBOX_PREFIX,
            RECEIVE_PREFIX,
            DELETE_INTENT_PREFIX
        ],
    )
    .map(|_| ())
    .map_err(|error| ResearchError::sql("Failed to clear research sync metadata", error))
}

/// Records one pending terminal job snapshot. A no-op when sync capture is off
/// (`capture_enabled`/`applying` gates, or no sync schema at all). When capture
/// runs, the job must exist and be terminal in the read-only research state
/// database: an active job is never enqueued.
pub(crate) fn enqueue_terminal_job(
    conn: &Connection,
    state: &Connection,
    job_id: &str,
) -> ResearchResult<()> {
    if !capture_active(conn)? {
        return Ok(());
    }
    require_terminal_research_job(state, job_id)?;
    write_outbox_entry(conn, job_id, 'U', now_ms())
}

/// Coalesces one pending outbox entry (last capture wins), minting a fresh
/// generation bound to this exact captured value.
fn write_outbox_entry(
    conn: &Connection,
    job_id: &str,
    op: char,
    changed_at: i64,
) -> ResearchResult<()> {
    let value = new_outbox_value(op, changed_at);
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![format!("{OUTBOX_PREFIX}{job_id}"), value],
    )
    .map(|_| ())
    .map_err(|error| ResearchError::sql("Failed to enqueue research outbox entry", error))
}

fn new_outbox_value(op: char, changed_at: i64) -> String {
    serde_json::json!({
        "op": op.to_string(),
        "changed_at": changed_at,
        "generation": Uuid::new_v4().to_string(),
    })
    .to_string()
}

/// Enqueues every terminal research job the server has never acknowledged
/// (`sync_row_versions` has no `(research_envelopes, job_id)` entry). Reads the
/// job ids from the read-only research state database; active jobs never
/// appear. Idempotent: re-running adds nothing new, never replaces pending
/// work, and never resurrects an acknowledged job.
///
/// The caller owns the catch-up gate: the future engine runs this only after
/// the epoch's since-zero research catch-up is recorded (the same gate as
/// pushes), so remote tombstones apply before a deleted job is re-seeded.
pub(crate) fn seed_terminal_outbox(conn: &Connection, state: &Connection) -> ResearchResult<usize> {
    if !capture_active(conn)? {
        return Ok(0);
    }
    let job_ids = terminal_research_job_ids(state)?;
    let mut seeded = 0;
    for job_id in job_ids {
        let acknowledged: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sync_row_versions
                     WHERE table_name = ?1 AND row_id = ?2)",
                params![ENVELOPE_TABLE, job_id],
                |row| row.get(0),
            )
            .map_err(|error| {
                ResearchError::sql("Failed to inspect research row versions", error)
            })?;
        if acknowledged {
            continue;
        }
        seeded += conn
            .execute(
                "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO NOTHING",
                params![
                    format!("{OUTBOX_PREFIX}{job_id}"),
                    new_outbox_value('U', now_ms()),
                ],
            )
            .map_err(|error| ResearchError::sql("Failed to seed research outbox", error))?;
    }
    Ok(seeded)
}

/// Pending entries in stable (changed_at, job_id) order. Keys are parsed by
/// literal prefix and values fail closed: a malformed entry is a real
/// sync-state error, never silently skipped.
pub(crate) fn outbox_entries(conn: &Connection) -> ResearchResult<Vec<OutboxEntry>> {
    let mut stmt = conn
        .prepare(
            "SELECT key, value
               FROM sync_meta
              WHERE substr(key, 1, length(?1)) = ?1
              ORDER BY key",
        )
        .map_err(|error| ResearchError::sql("Failed to read research outbox", error))?;
    let rows = stmt
        .query_map(params![OUTBOX_PREFIX], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| ResearchError::sql("Failed to read research outbox", error))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| ResearchError::sql("Failed to read research outbox", error))?;

    let mut entries: Vec<OutboxEntry> = rows
        .into_iter()
        .map(|(key, raw)| parse_outbox_entry(&key, raw))
        .collect::<ResearchResult<_>>()?;
    entries.sort_by(|a, b| (a.changed_at, &a.job_id).cmp(&(b.changed_at, &b.job_id)));
    Ok(entries)
}

fn parse_outbox_entry(key: &str, raw: String) -> ResearchResult<OutboxEntry> {
    let job_id = key
        .strip_prefix(OUTBOX_PREFIX)
        .filter(|job_id| !job_id.is_empty())
        .ok_or_else(|| corrupt_outbox(format!("invalid outbox key {key:?}")))?
        .to_string();
    let stored: StoredOutboxEntry = serde_json::from_str(&raw)
        .map_err(|error| corrupt_outbox(format!("cannot parse outbox entry {key:?}: {error}")))?;
    let op = match stored.op.as_str() {
        "U" => 'U',
        "D" => 'D',
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
    let parsed = Uuid::parse_str(&stored.generation).map_err(|error| {
        corrupt_outbox(format!(
            "outbox entry {key:?} has invalid generation: {error}"
        ))
    })?;
    if parsed.to_string() != stored.generation {
        return Err(corrupt_outbox(format!(
            "outbox entry {key:?} has a non-canonical generation"
        )));
    }

    Ok(OutboxEntry {
        job_id: job_id.clone(),
        op,
        changed_at: stored.changed_at,
        generation: stored.generation.clone(),
        acknowledgment: OutboxAcknowledgment {
            job_id,
            generation: stored.generation,
            captured_value: raw,
        },
    })
}

fn corrupt_outbox(message: impl Into<String>) -> ResearchError {
    ResearchError::new("research_outbox_corrupt", message)
}

/// True when one job still has a pending outbox entry (`U` or `D`). The
/// receive side defers automatic application while local work is pending: an
/// outbox entry is only ever removed by its exact generation acknowledgment,
/// never by a wire timestamp.
pub(crate) fn has_pending_outbox_entry(conn: &Connection, job_id: &str) -> ResearchResult<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_meta WHERE key = ?1)",
        params![format!("{OUTBOX_PREFIX}{job_id}")],
        |row| row.get::<_, i64>(0),
    )
    .map(|present| present != 0)
    .map_err(|error| ResearchError::sql("Failed to inspect research outbox", error))
}

// ───────────────────── local delete-intent lifecycle ─────────────────────

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDeleteIntent {
    op: String,
    job_id: String,
    began_at: i64,
}

/// Durably records the user's intent to delete one local research job before
/// the destructive engine delete runs. No-op when capture is off or the id is
/// not an existing research job (a non-research job is never captured as a
/// research tombstone). Idempotent: the first intent survives until settle or
/// abort removes it.
pub(crate) fn begin_delete_intent(
    conn: &Connection,
    state: &Connection,
    job_id: &str,
) -> ResearchResult<()> {
    if !capture_active(conn)? {
        return Ok(());
    }
    if job_id.is_empty() {
        return Err(corrupt_intent("delete intent requires a job id"));
    }
    if !is_research_job(state, job_id)? {
        return Ok(());
    }
    let value = serde_json::json!({
        "op": "D",
        "job_id": job_id,
        "began_at": now_ms(),
    })
    .to_string();
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO NOTHING",
        params![format!("{DELETE_INTENT_PREFIX}{job_id}"), value],
    )
    .map(|_| ())
    .map_err(|error| ResearchError::sql("Failed to record research delete intent", error))
}

/// Removes the delete intent of one job. A failed delete request must leave no
/// intent and no enqueued delete, so this is always safe and idempotent.
pub(crate) fn abort_delete_intent(conn: &Connection, job_id: &str) -> ResearchResult<()> {
    conn.execute(
        "DELETE FROM sync_meta WHERE key = ?1",
        params![format!("{DELETE_INTENT_PREFIX}{job_id}")],
    )
    .map(|_| ())
    .map_err(|error| ResearchError::sql("Failed to abort research delete intent", error))
}

/// Settles one recorded delete intent. A successful engine delete (the job is
/// gone from the research state database) leaves ONE idempotent `D` outbox
/// tombstone; a delete that did not happen only removes the intent — a failed
/// delete request never enqueues a delete. The tombstone and the intent removal
/// commit together, so a crash in between replays as the same idempotent
/// tombstone. Without a recorded intent this is a no-op: nothing was begun.
pub(crate) fn settle_delete_intent(
    conn: &Connection,
    state: &Connection,
    job_id: &str,
) -> ResearchResult<()> {
    if !capture_active(conn)? {
        return Ok(());
    }
    let key = format!("{DELETE_INTENT_PREFIX}{job_id}");
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| ResearchError::sql("Failed to read research delete intent", error))?;
    let Some(raw) = raw else {
        return Ok(());
    };
    let intent: StoredDeleteIntent = serde_json::from_str(&raw)
        .map_err(|error| corrupt_intent(format!("cannot parse delete intent {key:?}: {error}")))?;
    if intent.op != "D" || intent.job_id != job_id {
        return Err(corrupt_intent(format!(
            "delete intent {key:?} does not match its key"
        )));
    }
    if research_job_present(state, job_id)? {
        // The engine delete did not happen (or a new job reused the id):
        // never enqueue a delete for a failed delete request.
        return abort_delete_intent(conn, job_id);
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| ResearchError::sql("Failed to start research delete settlement", error))?;
    write_outbox_entry(&tx, job_id, 'D', now_ms())?;
    tx.execute("DELETE FROM sync_meta WHERE key = ?1", params![key])
        .map(|_| ())
        .map_err(|error| ResearchError::sql("Failed to clear research delete intent", error))?;
    tx.commit()
        .map_err(|error| ResearchError::sql("Failed to commit research delete settlement", error))
}

fn is_research_job(state: &Connection, job_id: &str) -> ResearchResult<bool> {
    state
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs WHERE id = ?1 AND modo = ?2)",
            params![job_id, super::research_envelope::RESEARCH_JOB_MODO],
            |row| row.get::<_, i64>(0),
        )
        .map(|present| present != 0)
        .map_err(|error| ResearchError::sql("Failed to inspect research job", error))
}

fn research_job_present(state: &Connection, job_id: &str) -> ResearchResult<bool> {
    state
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs WHERE id = ?1)",
            params![job_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|present| present != 0)
        .map_err(|error| ResearchError::sql("Failed to inspect research job", error))
}

fn corrupt_intent(message: impl Into<String>) -> ResearchError {
    ResearchError::new("research_delete_intent_corrupt", message)
}

/// Removes an entry only when it is still byte-for-byte the generation a push
/// draft captured. Replaying an acknowledgment is a harmless no-op, and a
/// stale acknowledgment can never remove newer work.
pub(crate) fn acknowledge_outbox_entry(
    conn: &Connection,
    acknowledgment: &OutboxAcknowledgment,
) -> ResearchResult<bool> {
    conn.execute(
        "DELETE FROM sync_meta WHERE key = ?1 AND value = ?2",
        params![
            format!("{OUTBOX_PREFIX}{}", acknowledgment.job_id),
            acknowledgment.captured_value,
        ],
    )
    .map(|deleted| deleted != 0)
    .map_err(|error| ResearchError::sql("Failed to acknowledge research outbox entry", error))
}

/// Records whether the server advertised `research-envelope-v1` during a
/// successful sync response of `server_epoch`. An epoch change (restore or
/// `bump-epoch`) invalidates the record until discovery runs again.
pub(crate) fn record_capability(
    conn: &Connection,
    server_epoch: &str,
    advertised: bool,
) -> ResearchResult<()> {
    let value = serde_json::json!({ "epoch": server_epoch, "advertised": advertised }).to_string();
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![CAPABILITY_KEY, value],
    )
    .map(|_| ())
    .map_err(|error| ResearchError::sql("Failed to record research capability", error))
}

/// True until this server epoch's one-time full research catch-up has run.
/// A client that pulled past hidden research rows cannot go incremental until
/// it has seen the whole account once (PROTOCOL activation rule, mirrored from
/// the writing catch-up).
pub(crate) fn catchup_needed(conn: &Connection, server_epoch: &str) -> ResearchResult<bool> {
    let recorded: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![CATCHUP_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| ResearchError::sql("Failed to read research catch-up state", error))?;
    Ok(recorded.as_deref() != Some(server_epoch))
}

/// Marks this server epoch's research catch-up complete.
pub(crate) fn record_catchup_done(conn: &Connection, server_epoch: &str) -> ResearchResult<()> {
    conn.execute(
        "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![CATCHUP_KEY, server_epoch],
    )
    .map(|_| ())
    .map_err(|error| ResearchError::sql("Failed to record research catch-up", error))
}

/// True only when discovery saw the capability advertised at exactly this
/// server epoch. Discovery from a stale epoch never enables the transport,
/// and a malformed record fails closed.
pub(crate) fn supports_research(conn: &Connection, server_epoch: &str) -> ResearchResult<bool> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            params![CAPABILITY_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| ResearchError::sql("Failed to read research capability", error))?;
    let Some(raw) = raw else {
        return Ok(false);
    };
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|error| corrupt_outbox(format!("cannot parse capability record: {error}")))?;
    Ok(parsed
        .get("advertised")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        && parsed.get("epoch").and_then(serde_json::Value::as_str) == Some(server_epoch))
}

/// The corpus capture gates, plus "sync schema exists at all": a database with
/// no sync schema must never fail a research close because of capture
/// bookkeeping. Once `sync_meta` exists, read failures are real sync-state
/// errors and must abort the surrounding call.
fn capture_active(conn: &Connection) -> ResearchResult<bool> {
    let sync_meta_exists = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sync_meta'
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| ResearchError::sql("Failed to inspect research sync schema", error))?
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
    .map_err(|error| ResearchError::sql("Failed to read research capture gates", error))
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
