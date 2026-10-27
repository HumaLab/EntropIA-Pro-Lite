//! Transport of the derived cache (`bibliography-derived-v1`) — plan
//! `odd/plans/plan-sync-bibliografia-derivada.md` §2.5 option (b), §2.7 and §2.9.
//!
//! The cache tables of [`super::derived`] are the account's paid content; this
//! module is the dedicated channel that moves their rows through EntropIA Cloud.
//! Like writing and research, it is *not* part of the generic oplog: its own
//! outbox (`pushed = 0`), its own `SyncApi` methods with the exact
//! `bibliography-derived-v1` token, its own cursor and its own since-zero
//! catch-up. `SYNCED_TABLES` and `last_pull_seq` are never touched.
//!
//! Wire contract (client side): `src/sync/derived_protocol.md`. The server half
//! is task T10 and does not exist yet, so this module defines the client side
//! and tests it against [`super::http::SyncApi`] doubles.
//!
//! Capability discipline (§2.5, the same one `web-capture-v1` uses):
//!
//! - Only an ORDINARY push/pull response advertises the token, and the verdict
//!   is only trusted for the `server_epoch` that advertised it
//!   ([`super::derived::record_capability`]). Until the verdict is `Supported`,
//!   this module makes no derived request at all: a legacy server never receives
//!   a call it does not understand, and the corpus keeps syncing.
//! - Three states, not a boolean: [`DerivedCapability::Unknown`] (wait, bounded),
//!   [`DerivedCapability::Supported`] and [`DerivedCapability::Unsupported`] (a
//!   legacy server, confirmed — pay as before).
//! - Every epoch needs one since-zero catch-up before its first incremental
//!   pull; a rotated epoch drops the record and re-runs it.
//!
//! Frozen request scope (§2.9). The network always runs outside every
//! transaction and the scope `(account_id, server_url, device_id, server_epoch,
//! session_incarnation)` is frozen when a request is prepared. The installing
//! / settling transaction re-reads it and compares it with the frozen copy *and*
//! with the response's `server_epoch`:
//!
//! - **Pull**: on a match the page installs with `origin = 'remote'` and the
//!   FROZEN `account_id`, in one transaction that also moves the cursor and
//!   records capability-independent bookkeeping; on any mismatch the response is
//!   discarded and neither the cursor, nor the capability record, nor the
//!   catch-up moves. A pull that left with account A can never install rows
//!   under account B.
//! - **Push**: the batches are built from the rows of the session account only,
//!   streamed page by page out of the database (the outbox is never materialized:
//!   a backfill is ~100k vectors) and byte-bounded per request. The response sets
//!   `pushed = 1` only for the sent keys and only while the frozen scope still
//!   matches; after an account switch nothing is marked, and the rows are
//!   (idempotently) re-sent under their own account when it logs in again. A row
//!   larger than the server's `max_push_bytes` travels alone; a `413` for it skips
//!   that row for the rest of the cycle so the rows behind it keep flowing.
//!
//! Pull-before-pay hook (§2.6): [`super::derived::request_incremental_pull`]
//! sets `sync_meta['derived_pull_requested']`. The phase below performs at most
//! one incremental pull per cycle; it always performs it when the flag is set
//! (even within [`MIN_PULL_SPACING_MS`] of the previous one), and it consumes the
//! flag with a successful install (a completed catch-up consumes it too: a
//! since-zero walk is fresher than the page the gate asked for). A request this
//! cycle cannot serve — capability missing, incomplete scope, failed or
//! discarded pull, catch-up over budget — is consumed as well
//! ([`abandon_requested_pull`]): the gate's bounded wait proceeds, and a stuck
//! flag must never keep the engine's interval ticker running a cycle every 60 s.
//!
//! The gate's freshness check reads
//! `sync_meta['derived_last_pull_started_at']`
//! ([`super::derived::last_pull_started_at`]). An incremental pull stamps its own
//! start; a COMPLETED catch-up stamps the instant it started, so the backlog
//! that admitted its work while the catch-up ran is fresh without an extra pull.
//! Many callers coalesce into one pull; the gate only has to observe that
//! timestamp.

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use super::derived::{
    capability_state, catchup_needed, clear_pull_request, current_epoch, last_pull_started_at,
    pull_requested, record_catchup_done, record_pull_started_at, DerivedCapability,
    PULL_CURSOR_KEY,
};
use super::engine::read_schema_tag;
use super::http::{PullResponse, PullRow, PushChange, PushResponse, SyncApi, SyncError};
use super::push::{DEFAULT_MAX_PUSH_BYTES, MAX_PUSH_CHANGES};
use super::session::{meta_get, meta_set, read_session_incarnation};

/// The three wire tables of the channel (`derived_protocol.md` §3).
pub(crate) const VECTOR_TABLE: &str = "derived_vectors";
pub(crate) const OCR_TABLE: &str = "derived_ocr_pages";
pub(crate) const CONTRACT_TABLE: &str = "derived_contracts";
/// Rows requested per pull page (PROTOCOL `limit` range is 1..=1000).
pub(crate) const PAGE_LIMIT: i64 = 500;
/// One cycle may walk the whole account once during catch-up (bounded), and
/// only a few pages for an incremental pull.
const MAX_CATCHUP_PAGES: usize = 64;
const MAX_INCREMENTAL_PAGES: usize = 8;
/// One cycle sends at most this many derived push requests (settled batches and
/// skipped oversized rows both count, so a cycle can never spin on rejected rows).
pub(crate) const MAX_PUSH_BATCHES: usize = 64;
/// The incremental pull is not repeated more often than this unless the
/// pull-before-pay gate asked for it (§2.6).
const MIN_PULL_SPACING_MS: i64 = 60_000;
/// Approximate per-change JSON envelope cost when sizing a batch.
const CHANGE_OVERHEAD_BYTES: usize = 96;
/// Floor for a batch budget, even if the server reports something absurd.
const MIN_MAX_PUSH_BYTES: usize = 512;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Frozen request scope (§2.9)
// ---------------------------------------------------------------------------

/// The identity a derived request belongs to, frozen when the request is
/// prepared from `sync_meta`. Not a guess and not the session at response time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DerivedScope {
    pub(crate) account_id: String,
    pub(crate) server_url: String,
    pub(crate) device_id: String,
    pub(crate) server_epoch: String,
    pub(crate) session_incarnation: Uuid,
}

/// Reads the current scope, or `None` when the session is not complete (no
/// account, URL, device, epoch or incarnation). An incomplete session sends and
/// installs nothing.
pub(crate) fn read_scope(conn: &Connection) -> Result<Option<DerivedScope>, String> {
    fn required(conn: &Connection, key: &str) -> Result<Option<String>, String> {
        Ok(meta_get(conn, key)?.filter(|value| !value.trim().is_empty()))
    }
    let Some(account_id) = required(conn, "account_id")? else {
        return Ok(None);
    };
    let Some(server_url) = required(conn, "server_url")? else {
        return Ok(None);
    };
    let Some(device_id) = required(conn, "device_id")? else {
        return Ok(None);
    };
    let Some(server_epoch) = required(conn, "server_epoch")? else {
        return Ok(None);
    };
    let Some(session_incarnation) = read_session_incarnation(conn)? else {
        return Ok(None);
    };
    Ok(Some(DerivedScope {
        account_id,
        server_url,
        device_id,
        server_epoch,
        session_incarnation,
    }))
}

/// The session scope still matching `frozen`, read inside the installing
/// transaction. `Err` carries the reason the request must be discarded.
fn scope_still_matches(conn: &Connection, frozen: &DerivedScope) -> Result<(), DerivedDiscard> {
    let current = read_scope(conn).map_err(|_| DerivedDiscard::SessionChanged)?;
    match current {
        Some(current) if current == *frozen => Ok(()),
        _ => Err(DerivedDiscard::SessionChanged),
    }
}

/// Why a response carried no changes into the local state (§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DerivedDiscard {
    /// Account, server, device, epoch or session incarnation changed while the
    /// request was in flight.
    SessionChanged,
    /// The response came from another server epoch than the frozen one.
    EpochChanged,
    /// The response did not advertise `bibliography-derived-v1` back.
    CapabilityMissing,
}

// ---------------------------------------------------------------------------
// Pull
// ---------------------------------------------------------------------------

/// A pull request frozen before the network runs.
#[derive(Debug, Clone)]
pub(crate) struct PreparedDerivedPull {
    pub(crate) scope: DerivedScope,
    pub(crate) schema_tag: String,
    pub(crate) since: i64,
    pub(crate) limit: i64,
    /// True while the epoch's since-zero catch-up has not completed.
    pub(crate) catchup: bool,
    /// Start time of this request, persisted on a successful incremental pull
    /// as the gate's freshness signal (§2.6).
    pub(crate) started_at_ms: i64,
}

/// What one install produced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DerivedInstall {
    /// Rows that were not in the local cache before this page.
    pub(crate) installed: usize,
    /// Rows rejected by validation; never repaired or published.
    pub(crate) skipped: usize,
    pub(crate) next_since: i64,
    pub(crate) has_more: bool,
    /// True only when this page recorded the epoch's catch-up.
    pub(crate) catchup_recorded: bool,
}

/// Result of applying one fetched page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DerivedPullOutcome {
    Installed(DerivedInstall),
    Discarded(DerivedDiscard),
}

/// The channel's own pull cursor, bound to the session that wrote it (§2.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCursor {
    scope: DerivedScope,
    since: i64,
}

fn read_cursor(conn: &Connection) -> Result<Option<StoredCursor>, String> {
    let raw = meta_get(conn, PULL_CURSOR_KEY)?;
    Ok(raw.and_then(|raw| serde_json::from_str(&raw).ok()))
}

fn write_cursor(conn: &Connection, scope: &DerivedScope, since: i64) -> Result<(), String> {
    let cursor = StoredCursor {
        scope: scope.clone(),
        since,
    };
    let raw = serde_json::to_string(&cursor)
        .map_err(|e| format!("[sync] failed to serialize the derived cursor: {e}"))?;
    meta_set(conn, PULL_CURSOR_KEY, &raw)
}

/// Freezes one pull request: complete session scope, the exact capability
/// supported for the current epoch, and the channel's own cursor (`0` when the
/// stored cursor belongs to another session). Returns `None` when the channel
/// must not call the wire — no session, or an unknown/unsupported capability.
pub(crate) fn prepare_derived_pull(
    conn: &Connection,
) -> Result<Option<PreparedDerivedPull>, String> {
    let Some(scope) = read_scope(conn)? else {
        return Ok(None);
    };
    if capability_state(conn) != DerivedCapability::Supported {
        return Ok(None);
    }
    let catchup = catchup_needed(conn, &scope.server_epoch)?;
    let since = match read_cursor(conn)? {
        Some(cursor) if cursor.scope == scope => cursor.since,
        // A cursor of another session/epoch is stale staging: it is ignored, so
        // the catch-up starts from zero instead of skipping unseen rows.
        _ => 0,
    };
    Ok(Some(PreparedDerivedPull {
        scope,
        schema_tag: read_schema_tag(conn)?,
        since,
        limit: PAGE_LIMIT,
        catchup,
        started_at_ms: now_ms(),
    }))
}

/// Issues exactly one opted-in page request. No database handle crosses this
/// await, and the trait's default implementation fails locally instead of
/// falling back to a capability-free pull.
pub(crate) async fn fetch_derived_pull_page<A: SyncApi>(
    api: &A,
    token: &str,
    prepared: &PreparedDerivedPull,
) -> Result<PullResponse, SyncError> {
    api.pull_with_bibliography_derived_v1(
        token,
        &prepared.schema_tag,
        prepared.since,
        prepared.limit,
    )
    .await
}

/// Installs one fetched page in a single transaction, or discards it (§2.9).
///
/// The transaction re-reads the session scope and compares it with the frozen
/// scope and with the response's `server_epoch` before touching anything, so a
/// response that left under another account/session can install no row, move no
/// cursor, record no catch-up and clear no gate request. Rows that fail wire
/// validation are skipped (never repaired): a corrupt vector or a forged OCR
/// answer can never enter the cache and be published later.
pub(crate) fn install_derived_pull_page(
    conn: &Connection,
    prepared: &PreparedDerivedPull,
    response: &PullResponse,
) -> Result<DerivedPullOutcome, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("[sync] failed to begin the derived pull transaction: {e}"))?;
    if let Err(reason) = scope_still_matches(&tx, &prepared.scope) {
        return Ok(DerivedPullOutcome::Discarded(reason));
    }
    if response.server_epoch != prepared.scope.server_epoch {
        return Ok(DerivedPullOutcome::Discarded(DerivedDiscard::EpochChanged));
    }
    if !response.supports_bibliography_derived_v1() {
        return Ok(DerivedPullOutcome::Discarded(
            DerivedDiscard::CapabilityMissing,
        ));
    }

    let mut install = DerivedInstall {
        next_since: prepared.since.max(response.next_since),
        has_more: response.has_more,
        ..DerivedInstall::default()
    };
    for row in &response.rows {
        match install_wire_row(&tx, &prepared.scope.account_id, row)? {
            RowEffect::Inserted => install.installed += 1,
            RowEffect::Skipped => install.skipped += 1,
            RowEffect::UnknownTable => {}
        }
    }

    write_cursor(&tx, &prepared.scope, install.next_since)?;
    if prepared.catchup {
        if !response.has_more {
            record_catchup_done(&tx, &prepared.scope.server_epoch)?;
            install.catchup_recorded = true;
            // A completed catch-up walked the whole channel from zero: it is
            // strictly fresher than the incremental page the pull-before-pay
            // gate asked for, so it consumes the request (many requests, one
            // pull). An interrupted catch-up keeps it.
            clear_pull_request(&tx)?;
        }
    } else {
        // An incremental pull stamps the start of that pull (§2.6). A completed
        // catch-up gets its own stamp from the cycle, which knows when the walk
        // began (the install only sees one page).
        record_pull_started_at(&tx, &prepared.scope.server_epoch, prepared.started_at_ms)?;
        clear_pull_request(&tx)?;
    }
    tx.commit()
        .map_err(|e| format!("[sync] failed to commit the derived pull: {e}"))?;
    Ok(DerivedPullOutcome::Installed(install))
}

/// What one wire row did to the local cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowEffect {
    Inserted,
    Skipped,
    UnknownTable,
}

fn install_wire_row(
    conn: &Connection,
    account_id: &str,
    row: &PullRow,
) -> Result<RowEffect, String> {
    match row.table.as_str() {
        VECTOR_TABLE => install_vector_row(conn, account_id, row),
        OCR_TABLE => install_ocr_row(conn, account_id, row),
        CONTRACT_TABLE => install_contract_row(conn, row),
        _ => Ok(RowEffect::UnknownTable),
    }
}

fn payload_field<'a>(payload: &'a Value, key: &str) -> Option<&'a Value> {
    payload.get(key).filter(|value| !value.is_null())
}

fn payload_str<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    payload_field(payload, key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn payload_i64(payload: &Value, key: &str) -> Option<i64> {
    payload_field(payload, key).and_then(Value::as_i64)
}

/// A vector is usable only when its width matches and every `f32` is finite —
/// the same rule the publisher applies, because these rows are untrusted input.
fn valid_vector_bytes(embedding: &[u8], dimensions: i64) -> bool {
    if dimensions <= 0 {
        return false;
    }
    let Some(expected) = dimensions.checked_mul(4) else {
        return false;
    };
    if embedding.len() as i64 != expected {
        return false;
    }
    embedding
        .chunks_exact(4)
        .all(|value| f32::from_le_bytes([value[0], value[1], value[2], value[3]]).is_finite())
}

fn install_vector_row(
    conn: &Connection,
    account_id: &str,
    row: &PullRow,
) -> Result<RowEffect, String> {
    // This channel is append-only: a tombstone (or a missing payload) is not a
    // row it can install.
    let Some(payload) = row.payload.as_ref() else {
        return Ok(RowEffect::Skipped);
    };
    let Some(contract_hash) = payload_str(payload, "contract_hash") else {
        return Ok(RowEffect::Skipped);
    };
    let Some(input_hash) = payload_str(payload, "input_hash") else {
        return Ok(RowEffect::Skipped);
    };
    let Some(dimensions) = payload_i64(payload, "dimensions") else {
        return Ok(RowEffect::Skipped);
    };
    let Some(encoded) = payload_str(payload, "embedding") else {
        return Ok(RowEffect::Skipped);
    };
    let Ok(embedding) = BASE64_STANDARD.decode(encoded) else {
        return Ok(RowEffect::Skipped);
    };
    if !valid_vector_bytes(&embedding, dimensions) {
        return Ok(RowEffect::Skipped);
    }
    let created_at = payload_i64(payload, "created_at").unwrap_or_else(now_ms);
    let changed = conn
        .execute(
            "INSERT OR IGNORE INTO sync_derived_vectors
               (account_id, contract_hash, input_hash, dimensions, embedding, origin, pushed, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'remote', 1, ?6)",
            params![
                account_id,
                contract_hash,
                input_hash,
                dimensions,
                embedding,
                created_at
            ],
        )
        .map_err(|e| format!("[sync] failed to install a derived vector: {e}"))?;
    Ok(if changed == 1 {
        RowEffect::Inserted
    } else {
        RowEffect::Skipped
    })
}

fn install_ocr_row(
    conn: &Connection,
    account_id: &str,
    row: &PullRow,
) -> Result<RowEffect, String> {
    let Some(payload) = row.payload.as_ref() else {
        return Ok(RowEffect::Skipped);
    };
    let Some(source_hash) = payload_str(payload, "source_hash") else {
        return Ok(RowEffect::Skipped);
    };
    let Some(page_number) = payload_i64(payload, "page_number") else {
        return Ok(RowEffect::Skipped);
    };
    let Some(ocr_contract) = payload_str(payload, "ocr_contract") else {
        return Ok(RowEffect::Skipped);
    };
    let Some(outcome) = payload_str(payload, "outcome") else {
        return Ok(RowEffect::Skipped);
    };
    // An empty answer is a valid `text_content` (`outcome='empty'`), so the
    // field is read without the non-empty filter the identity fields use.
    let Some(text) = payload_field(payload, "text_content").and_then(Value::as_str) else {
        return Ok(RowEffect::Skipped);
    };
    let Some(text_hash) = payload_str(payload, "text_hash") else {
        return Ok(RowEffect::Skipped);
    };
    if page_number < 0 || text_hash != super::derived::sha256_hex(text.as_bytes()) {
        return Ok(RowEffect::Skipped);
    }
    match outcome {
        "text" => {}
        "empty" if text.is_empty() => {}
        _ => return Ok(RowEffect::Skipped),
    }
    let created_at = payload_i64(payload, "created_at").unwrap_or_else(now_ms);
    let changed = conn
        .execute(
            "INSERT OR IGNORE INTO sync_derived_ocr_pages
               (account_id, source_hash, page_number, ocr_contract, outcome, text_content,
                text_hash, origin, pushed, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'remote', 1, ?8)",
            params![
                account_id,
                source_hash,
                page_number,
                ocr_contract,
                outcome,
                text,
                text_hash,
                created_at
            ],
        )
        .map_err(|e| format!("[sync] failed to install a derived OCR page: {e}"))?;
    Ok(if changed == 1 {
        RowEffect::Inserted
    } else {
        RowEffect::Skipped
    })
}

fn install_contract_row(conn: &Connection, row: &PullRow) -> Result<RowEffect, String> {
    let Some(payload) = row.payload.as_ref() else {
        return Ok(RowEffect::Skipped);
    };
    let Some(contract_hash) = payload_str(payload, "contract_hash") else {
        return Ok(RowEffect::Skipped);
    };
    let Some(provider) = payload_str(payload, "provider") else {
        return Ok(RowEffect::Skipped);
    };
    let Some(model) = payload_str(payload, "model") else {
        return Ok(RowEffect::Skipped);
    };
    let Some(dimensions) = payload_i64(payload, "dimensions") else {
        return Ok(RowEffect::Skipped);
    };
    let Some(chunking_contract) = payload_str(payload, "chunking_contract") else {
        return Ok(RowEffect::Skipped);
    };
    if dimensions <= 0 {
        return Ok(RowEffect::Skipped);
    }
    let changed = conn
        .execute(
            "INSERT OR IGNORE INTO sync_derived_contracts
               (contract_hash, provider, model, dimensions, chunking_contract)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                contract_hash,
                provider,
                model,
                dimensions,
                chunking_contract
            ],
        )
        .map_err(|e| format!("[sync] failed to install a derived contract: {e}"))?;
    Ok(if changed == 1 {
        RowEffect::Inserted
    } else {
        RowEffect::Skipped
    })
}

// ---------------------------------------------------------------------------
// Push
// ---------------------------------------------------------------------------

/// One vector key that travelled in a batch; settlement marks exactly these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SentVector {
    pub(crate) contract_hash: String,
    pub(crate) input_hash: String,
}

/// One OCR page key that travelled in a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SentOcrPage {
    pub(crate) source_hash: String,
    pub(crate) page_number: i64,
    pub(crate) ocr_contract: String,
}

/// One byte-bounded push request and the local keys it carries.
#[derive(Debug, Clone)]
pub(crate) struct DerivedPushBatch {
    pub(crate) changes: Vec<PushChange>,
    pub(crate) vectors: Vec<SentVector>,
    pub(crate) ocr_pages: Vec<SentOcrPage>,
}

impl DerivedPushBatch {
    fn new() -> Self {
        Self {
            changes: Vec::new(),
            vectors: Vec::new(),
            ocr_pages: Vec::new(),
        }
    }

    fn push(&mut self, unit: WireUnit) {
        self.changes.push(unit.change);
        if let Some(vector) = unit.vector {
            self.vectors.push(vector);
        }
        if let Some(page) = unit.ocr_page {
            self.ocr_pages.push(page);
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DerivedPushSettlement {
    pub(crate) vectors_marked: usize,
    pub(crate) ocr_pages_marked: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DerivedPushOutcome {
    Settled(DerivedPushSettlement),
    Discarded(DerivedDiscard),
}

/// Rows read from the database per keyset page while walking the outbox. A push
/// run never materializes the whole outbox — the owner's backfill is about a
/// hundred thousand vectors (~400 MiB) — so one page plus one batch is the whole
/// memory bound of a run.
pub(crate) const PUSH_PAGE_ROWS: usize = 256;

/// Where one wire unit sits in its table's primary key, so the keyset walk can
/// resume exactly after it without an `OFFSET` scan.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UnitKey {
    Contract(String),
    Vector {
        contract_hash: String,
        input_hash: String,
    },
    Ocr {
        source_hash: String,
        page_number: i64,
        ocr_contract: String,
    },
}

struct WireUnit {
    change: PushChange,
    key: UnitKey,
    vector: Option<SentVector>,
    ocr_page: Option<SentOcrPage>,
}

impl WireUnit {
    fn bytes(&self) -> usize {
        serde_json::to_vec(&self.change)
            .map(|bytes| bytes.len())
            .unwrap_or(CHANGE_OVERHEAD_BYTES)
            + CHANGE_OVERHEAD_BYTES
    }

    fn contract_hash(&self) -> Option<&str> {
        self.vector
            .as_ref()
            .map(|vector| vector.contract_hash.as_str())
    }
}

/// One table's keyset walk, buffered for at most [`PUSH_PAGE_ROWS`] rows.
#[derive(Default)]
struct VectorPager {
    buffered: std::collections::VecDeque<WireUnit>,
    /// Last row consumed from the database; the next page starts after it.
    after: Option<(String, String)>,
    exhausted: bool,
}

#[derive(Default)]
struct OcrPager {
    buffered: std::collections::VecDeque<WireUnit>,
    after: Option<(String, i64, String)>,
    exhausted: bool,
}

/// The streaming outbox of one push run. Nothing here holds the whole outbox:
/// `prepare_derived_push` only validates the gate, and every
/// [`next_derived_push_batch`] reads at most one page of rows and returns one
/// byte-bounded request. `pushed` is untouched until a response settles against
/// the frozen scope (§2.9).
pub(crate) struct PreparedDerivedPush {
    pub(crate) scope: DerivedScope,
    pub(crate) schema_tag: String,
    /// Byte budget one request may use (the server's `max_push_bytes`).
    limit: usize,
    vectors: VectorPager,
    ocr: OcrPager,
    /// Contract hashes already placed in a request of this run: a contract row
    /// rides with the first vector of its contract in each run (they are
    /// immutable and small, and the server's insert is idempotent).
    contracts_included: std::collections::HashSet<String>,
    /// A unit read ahead whose batch was full; consumed before any new page.
    carry: Option<WireUnit>,
}

/// Freezes the push gate (complete session, exact capability supported for the
/// epoch, that epoch's catch-up already recorded — the same gate writing uses)
/// without reading a single cache row. `None` means "nothing to do", never
/// "call the wire without a capability".
pub(crate) fn prepare_derived_push(
    conn: &Connection,
    max_push_bytes: usize,
) -> Result<Option<PreparedDerivedPush>, String> {
    let Some(scope) = read_scope(conn)? else {
        return Ok(None);
    };
    if capability_state(conn) != DerivedCapability::Supported
        || catchup_needed(conn, &scope.server_epoch)?
    {
        return Ok(None);
    }
    if !has_pending_rows(conn, &scope.account_id)? {
        return Ok(None);
    }
    Ok(Some(PreparedDerivedPush {
        schema_tag: read_schema_tag(conn)?,
        limit: max_push_bytes.max(MIN_MAX_PUSH_BYTES),
        scope,
        vectors: VectorPager::default(),
        ocr: OcrPager::default(),
        contracts_included: std::collections::HashSet::new(),
        carry: None,
    }))
}

fn has_pending_rows(conn: &Connection, account_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_derived_vectors WHERE account_id = ?1 AND pushed = 0)
             OR EXISTS(SELECT 1 FROM sync_derived_ocr_pages WHERE account_id = ?1 AND pushed = 0)",
        [account_id],
        |row| row.get(0),
    )
    .map_err(|e| format!("[sync] failed to read the derived outbox depth: {e}"))
}

/// Builds the next byte-bounded request from the streaming outbox, or `None`
/// when every pending row of the account has been handed to a batch.
///
/// The batch respects `MAX_PUSH_CHANGES` and the run's byte budget. A single
/// row larger than the budget still travels alone (the server may reject it;
/// [`push_outbox`] then skips it instead of wedging the outbox). Contract rows
/// are prepended to the batch that carries the first vector of their contract in
/// this run, and `changed_at` for them is the client clock at read time (the
/// server ignores it: the channel is append-only, `derived_protocol.md` §3).
pub(crate) fn next_derived_push_batch(
    conn: &Connection,
    prepared: &mut PreparedDerivedPush,
) -> Result<Option<DerivedPushBatch>, String> {
    let mut batch = DerivedPushBatch::new();
    let mut bytes = CHANGE_OVERHEAD_BYTES;
    while batch.changes.len() < MAX_PUSH_CHANGES {
        let Some(unit) = take_unit(conn, prepared)? else {
            break;
        };
        let mut extras = Vec::new();
        let mut extra_bytes = 0usize;
        if let Some(contract_hash) = unit.contract_hash() {
            if !prepared.contracts_included.contains(contract_hash) {
                if let Some(contract) = read_contract_unit(conn, contract_hash)? {
                    extra_bytes += contract.bytes();
                    extras.push(contract);
                }
            }
        }
        if !batch.changes.is_empty() && bytes + unit.bytes() + extra_bytes > prepared.limit {
            // The batch is full: keep this unit for the next request instead of
            // dropping it (and without re-reading its page).
            prepared.carry = Some(unit);
            break;
        }
        for extra in extras {
            if let UnitKey::Contract(hash) = &extra.key {
                prepared.contracts_included.insert(hash.clone());
            }
            bytes += extra.bytes();
            batch.push(extra);
        }
        bytes += unit.bytes();
        batch.push(unit);
    }
    Ok(if batch.changes.is_empty() {
        None
    } else {
        Some(batch)
    })
}

/// The next unit of the run: the carried one first, then the vector page, then
/// the OCR page. Pages are read only when the buffers are empty.
fn take_unit(
    conn: &Connection,
    prepared: &mut PreparedDerivedPush,
) -> Result<Option<WireUnit>, String> {
    if let Some(unit) = prepared.carry.take() {
        return Ok(Some(unit));
    }
    if let Some(unit) = pop_vector(prepared) {
        return Ok(Some(unit));
    }
    if !prepared.vectors.exhausted {
        let page = read_vector_page(
            conn,
            &prepared.scope.account_id,
            prepared.vectors.after.as_ref(),
            PUSH_PAGE_ROWS,
        )?;
        prepared.vectors.exhausted = page.len() < PUSH_PAGE_ROWS;
        prepared.vectors.buffered.extend(page);
        if let Some(unit) = pop_vector(prepared) {
            return Ok(Some(unit));
        }
    }
    if let Some(unit) = pop_ocr(prepared) {
        return Ok(Some(unit));
    }
    if !prepared.ocr.exhausted {
        let page = read_ocr_page(
            conn,
            &prepared.scope.account_id,
            prepared.ocr.after.as_ref(),
            PUSH_PAGE_ROWS,
        )?;
        prepared.ocr.exhausted = page.len() < PUSH_PAGE_ROWS;
        prepared.ocr.buffered.extend(page);
        if let Some(unit) = pop_ocr(prepared) {
            return Ok(Some(unit));
        }
    }
    Ok(None)
}

fn pop_vector(prepared: &mut PreparedDerivedPush) -> Option<WireUnit> {
    let unit = prepared.vectors.buffered.pop_front()?;
    if let UnitKey::Vector {
        contract_hash,
        input_hash,
    } = &unit.key
    {
        prepared.vectors.after = Some((contract_hash.clone(), input_hash.clone()));
    }
    Some(unit)
}

fn pop_ocr(prepared: &mut PreparedDerivedPush) -> Option<WireUnit> {
    let unit = prepared.ocr.buffered.pop_front()?;
    if let UnitKey::Ocr {
        source_hash,
        page_number,
        ocr_contract,
    } = &unit.key
    {
        prepared.ocr.after = Some((source_hash.clone(), *page_number, ocr_contract.clone()));
    }
    Some(unit)
}

/// One keyset page of pending vectors, ordered by primary key. The cursor is a
/// NULL pair on the first page, so a row with an empty `contract_hash` can never
/// be skipped by a sentinel.
fn read_vector_page(
    conn: &Connection,
    account_id: &str,
    after: Option<&(String, String)>,
    page: usize,
) -> Result<Vec<WireUnit>, String> {
    let mut statement = conn
        .prepare(
            "SELECT contract_hash, input_hash, dimensions, embedding, created_at
               FROM sync_derived_vectors
              WHERE account_id = ?1 AND pushed = 0
                AND (?2 IS NULL OR contract_hash > ?2
                     OR (contract_hash = ?2 AND input_hash > ?3))
              ORDER BY contract_hash, input_hash
              LIMIT ?4",
        )
        .map_err(|e| format!("[sync] failed to read pending derived vectors: {e}"))?;
    let rows = statement
        .query_map(
            params![
                account_id,
                after.map(|cursor| cursor.0.as_str()),
                after.map(|cursor| cursor.1.as_str()),
                page as i64
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .map_err(|e| format!("[sync] failed to read pending derived vectors: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("[sync] failed to read pending derived vectors: {e}"))?;
    Ok(rows
        .into_iter()
        .map(
            |(contract_hash, input_hash, dimensions, embedding, created_at)| {
                let row_id = format!("{contract_hash}:{input_hash}");
                WireUnit {
                    change: wire_change(
                        VECTOR_TABLE,
                        &row_id,
                        json!({
                            "contract_hash": contract_hash,
                            "input_hash": input_hash,
                            "dimensions": dimensions,
                            "embedding": BASE64_STANDARD.encode(&embedding),
                            "created_at": created_at,
                        }),
                        created_at,
                    ),
                    key: UnitKey::Vector {
                        contract_hash: contract_hash.clone(),
                        input_hash: input_hash.clone(),
                    },
                    vector: Some(SentVector {
                        contract_hash,
                        input_hash,
                    }),
                    ocr_page: None,
                }
            },
        )
        .collect())
}

fn read_ocr_page(
    conn: &Connection,
    account_id: &str,
    after: Option<&(String, i64, String)>,
    page: usize,
) -> Result<Vec<WireUnit>, String> {
    let mut statement = conn
        .prepare(
            "SELECT source_hash, page_number, ocr_contract, outcome, text_content, text_hash, created_at
               FROM sync_derived_ocr_pages
              WHERE account_id = ?1 AND pushed = 0
                AND (?2 IS NULL OR source_hash > ?2
                     OR (source_hash = ?2 AND (page_number > ?3
                         OR (page_number = ?3 AND ocr_contract > ?4))))
              ORDER BY source_hash, page_number, ocr_contract
              LIMIT ?5",
        )
        .map_err(|e| format!("[sync] failed to read pending derived OCR pages: {e}"))?;
    let rows = statement
        .query_map(
            params![
                account_id,
                after.map(|cursor| cursor.0.as_str()),
                after.map(|cursor| cursor.1),
                after.map(|cursor| cursor.2.as_str()),
                page as i64
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|e| format!("[sync] failed to read pending derived OCR pages: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("[sync] failed to read pending derived OCR pages: {e}"))?;
    Ok(rows
        .into_iter()
        .map(
            |(source_hash, page_number, ocr_contract, outcome, text, text_hash, created_at)| {
                let row_id = format!("{source_hash}:{page_number}:{ocr_contract}");
                WireUnit {
                    change: wire_change(
                        OCR_TABLE,
                        &row_id,
                        json!({
                            "source_hash": source_hash,
                            "page_number": page_number,
                            "ocr_contract": ocr_contract,
                            "outcome": outcome,
                            "text_content": text,
                            "text_hash": text_hash,
                            "created_at": created_at,
                        }),
                        created_at,
                    ),
                    key: UnitKey::Ocr {
                        source_hash: source_hash.clone(),
                        page_number,
                        ocr_contract: ocr_contract.clone(),
                    },
                    vector: None,
                    ocr_page: Some(SentOcrPage {
                        source_hash,
                        page_number,
                        ocr_contract,
                    }),
                }
            },
        )
        .collect())
}

/// The immutable contract row a vector belongs to, read on demand (never all of
/// them up front). A vector whose contract was never mirrored simply travels
/// without it: the reader validates the vector by its own contract anyway.
fn read_contract_unit(conn: &Connection, contract_hash: &str) -> Result<Option<WireUnit>, String> {
    let row = conn
        .query_row(
            "SELECT provider, model, dimensions, chunking_contract
               FROM sync_derived_contracts WHERE contract_hash = ?1",
            [contract_hash],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("[sync] failed to read derived contract {contract_hash}: {e}"))?;
    Ok(row.map(
        |(provider, model, dimensions, chunking_contract)| WireUnit {
            change: wire_change(
                CONTRACT_TABLE,
                contract_hash,
                json!({
                    "contract_hash": contract_hash,
                    "provider": provider,
                    "model": model,
                    "dimensions": dimensions,
                    "chunking_contract": chunking_contract,
                }),
                now_ms(),
            ),
            key: UnitKey::Contract(contract_hash.to_string()),
            vector: None,
            ocr_page: None,
        },
    ))
}

fn wire_change(table: &str, row_id: &str, payload: Value, changed_at: i64) -> PushChange {
    PushChange {
        table: table.to_string(),
        row_id: row_id.to_string(),
        op: "upsert".to_string(),
        changed_at,
        base_seq: 0,
        payload: Some(payload),
    }
}

/// Sends one frozen batch with the exact capability opt-in.
pub(crate) async fn send_derived_push_batch<A: SyncApi>(
    api: &A,
    token: &str,
    prepared: &PreparedDerivedPush,
    batch: &DerivedPushBatch,
) -> Result<PushResponse, SyncError> {
    api.push_with_bibliography_derived_v1(
        token,
        &prepared.schema_tag,
        super::http::PushRequest {
            changes: batch.changes.clone(),
        },
    )
    .await
}

/// Settles one batch's response (§2.9): the response's epoch must equal the
/// frozen one, the exact capability must come back, and the session scope must
/// still match. Only then are the SENT keys of this batch marked `pushed = 1`,
/// under the frozen account. A response that arrives after a logout / account
/// change marks nothing.
pub(crate) fn settle_derived_push(
    conn: &Connection,
    prepared: &PreparedDerivedPush,
    batch: &DerivedPushBatch,
    response: &PushResponse,
) -> Result<DerivedPushOutcome, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("[sync] failed to begin the derived push transaction: {e}"))?;
    if let Err(reason) = scope_still_matches(&tx, &prepared.scope) {
        return Ok(DerivedPushOutcome::Discarded(reason));
    }
    if response.server_epoch != prepared.scope.server_epoch {
        return Ok(DerivedPushOutcome::Discarded(DerivedDiscard::EpochChanged));
    }
    if !response.supports_bibliography_derived_v1() {
        return Ok(DerivedPushOutcome::Discarded(
            DerivedDiscard::CapabilityMissing,
        ));
    }
    let mut settlement = DerivedPushSettlement::default();
    for key in &batch.vectors {
        settlement.vectors_marked += tx
            .execute(
                "UPDATE sync_derived_vectors SET pushed = 1
                  WHERE account_id = ?1 AND contract_hash = ?2 AND input_hash = ?3 AND pushed = 0",
                params![prepared.scope.account_id, key.contract_hash, key.input_hash],
            )
            .map_err(|e| format!("[sync] failed to mark a derived vector as pushed: {e}"))?;
    }
    for key in &batch.ocr_pages {
        settlement.ocr_pages_marked += tx
            .execute(
                "UPDATE sync_derived_ocr_pages SET pushed = 1
                  WHERE account_id = ?1 AND source_hash = ?2 AND page_number = ?3
                    AND ocr_contract = ?4 AND pushed = 0",
                params![
                    prepared.scope.account_id,
                    key.source_hash,
                    key.page_number,
                    key.ocr_contract
                ],
            )
            .map_err(|e| format!("[sync] failed to mark a derived OCR page as pushed: {e}"))?;
    }
    tx.commit()
        .map_err(|e| format!("[sync] failed to commit the derived push: {e}"))?;
    Ok(DerivedPushOutcome::Settled(settlement))
}

// ---------------------------------------------------------------------------
// Cycle phase
// ---------------------------------------------------------------------------

/// Observable result of one derived phase (tests and diagnostics).
#[derive(Debug, Default)]
pub(crate) struct DerivedCycleOutcome {
    pub(crate) capability: Option<DerivedCapability>,
    /// True when the epoch still needed its since-zero catch-up at phase start.
    pub(crate) catchup_pending_at_start: bool,
    pub(crate) catchup_recorded: bool,
    pub(crate) pull_pages: usize,
    pub(crate) rows_installed: usize,
    pub(crate) rows_skipped: usize,
    pub(crate) push_batches: usize,
    pub(crate) rows_pushed: usize,
    /// Rows the server rejected as larger than `max_push_bytes`; they stay
    /// pending for the next cycle and never block the rows behind them.
    pub(crate) skipped_oversize_rows: usize,
    /// Set when a response was thrown away because the session moved (§2.9).
    pub(crate) discarded: Option<DerivedDiscard>,
    /// True when the gate had asked for an incremental pull this cycle.
    pub(crate) pull_requested: bool,
    /// True when this phase performed the incremental pull (requested, or the
    /// previous one is older than [`MIN_PULL_SPACING_MS`]).
    pub(crate) pulled_incrementally: bool,
    /// Pending cache rows at the end of the phase (outbox depth).
    pub(crate) pending_rows: i64,
}

/// How one pull page ended.
enum PageStep {
    Installed { has_more: bool },
    Discarded,
    Failed,
}

fn pending_rows(conn: &Connection, account_id: &str) -> i64 {
    let query = "SELECT
            (SELECT COUNT(*) FROM sync_derived_vectors WHERE account_id = ?1 AND pushed = 0)
          + (SELECT COUNT(*) FROM sync_derived_ocr_pages WHERE account_id = ?1 AND pushed = 0)";
    conn.query_row(query, [account_id], |row| row.get(0))
        .unwrap_or(0)
}

fn note(warn: &(dyn Fn(String) + Sync), message: String) {
    warn(format!("derived sync: {message}"));
}

/// Runs the channel inside the sync cycle, after the ordinary push/pull (and
/// after the writing/research phases). Never changes the cycle's result: every
/// failure stays pending for the next cycle and is reported through `warn`.
///
/// Order: capability gate → since-zero catch-up (once per epoch) → incremental
/// pull (when requested, or at most once per [`MIN_PULL_SPACING_MS`]) → outbox
/// push in byte-bounded batches. Without a session, without a capability
/// verdict, or with an unsupported (legacy) server it returns without a single
/// derived request.
pub(crate) async fn run_derived_cycle<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    max_push_bytes: i64,
    warn: &(dyn Fn(String) + Sync),
) -> DerivedCycleOutcome {
    let mut outcome = DerivedCycleOutcome {
        capability: Some(capability_state(conn)),
        ..DerivedCycleOutcome::default()
    };
    // The gate's request, read once at the start: every path reports it, and
    // every path that cannot serve it consumes it (see `abandon_requested_pull`).
    outcome.pull_requested = pull_requested(conn).unwrap_or(false);
    if outcome.capability != Some(DerivedCapability::Supported) {
        abandon_requested_pull(conn, outcome.pull_requested, warn);
        return outcome;
    }
    let Ok(Some(scope)) = read_scope(conn) else {
        abandon_requested_pull(conn, outcome.pull_requested, warn);
        return outcome;
    };
    let Ok(Some(epoch)) = current_epoch(conn) else {
        abandon_requested_pull(conn, outcome.pull_requested, warn);
        return outcome;
    };
    outcome.catchup_pending_at_start = match catchup_needed(conn, &epoch) {
        Ok(needed) => needed,
        Err(error) => {
            note(warn, format!("catch-up state unreadable: {error}"));
            abandon_requested_pull(conn, outcome.pull_requested, warn);
            return outcome;
        }
    };

    if outcome.catchup_pending_at_start {
        // The instant the catch-up started: a completed since-zero walk is the
        // epoch's freshness signal, and a unit admitted before it began is
        // covered by it (no extra pull for the backlog that admitted the work
        // while the catch-up ran).
        let catchup_started_at = now_ms();
        let mut finished = false;
        for _ in 0..MAX_CATCHUP_PAGES {
            match pull_one_page(api, token, conn, &mut outcome, warn).await {
                PageStep::Installed {
                    has_more: false, ..
                } => {
                    finished = true;
                    break;
                }
                PageStep::Installed { .. } => continue,
                PageStep::Discarded | PageStep::Failed => {
                    abandon_requested_pull(conn, outcome.pull_requested, warn);
                    return outcome;
                }
            }
        }
        outcome.catchup_recorded = !catchup_needed(conn, &epoch).unwrap_or(false);
        if !finished || !outcome.catchup_recorded {
            note(
                warn,
                format!(
                    "catch-up budget ({MAX_CATCHUP_PAGES} pages) reached this cycle; the rest stays pending"
                ),
            );
            abandon_requested_pull(conn, outcome.pull_requested, warn);
            return outcome;
        }
        if let Err(error) = record_pull_started_at(conn, &epoch, catchup_started_at) {
            note(
                warn,
                format!("catch-up freshness signal unrecorded: {error}"),
            );
        }
    }

    // A catch-up is already a since-zero pull of the whole channel: it is not
    // repeated by an incremental one in the same cycle.
    let recent = last_pull_started_at(conn)
        .map(|started| now_ms().saturating_sub(started) < MIN_PULL_SPACING_MS)
        .unwrap_or(false);
    if !outcome.catchup_recorded && (outcome.pull_requested || !recent) {
        outcome.pulled_incrementally = true;
        for _ in 0..MAX_INCREMENTAL_PAGES {
            match pull_one_page(api, token, conn, &mut outcome, warn).await {
                PageStep::Installed { has_more: true, .. } => continue,
                PageStep::Installed { .. } => break,
                PageStep::Discarded | PageStep::Failed => {
                    abandon_requested_pull(conn, outcome.pull_requested, warn);
                    return outcome;
                }
            }
        }
    }

    push_outbox(api, token, conn, max_push_bytes, &mut outcome, warn).await;
    outcome.pending_rows = pending_rows(conn, &scope.account_id);
    outcome
}

/// Consumes a gate request this cycle cannot serve (§2.6): the capability is not
/// advertised, the session scope is incomplete, the catch-up could not finish or
/// the pull failed/was discarded. The install paths consume it themselves on
/// success; every other exit must consume it too, or a stuck flag would make the
/// engine's interval ticker run a full sync cycle every 60 s forever
/// (`engine::tick_should_run` reads exactly this flag). The gate's bounded wait
/// re-checks its unit and simply proceeds when the cache does not arrive, so
/// nothing is lost by giving the request up.
///
/// A request that arrives DURING the cycle is left pending: it is read once at
/// the start, so the bypass can fire at most once per request.
fn abandon_requested_pull(conn: &Connection, requested: bool, warn: &(dyn Fn(String) + Sync)) {
    if !requested {
        return;
    }
    if let Err(error) = clear_pull_request(conn) {
        note(warn, format!("gate pull request stays pending: {error}"));
    }
}

/// Prepares, fetches and installs exactly one page. Returns [`PageStep::Failed`]
/// after logging; no state was touched in that case.
async fn pull_one_page<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    outcome: &mut DerivedCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
) -> PageStep {
    let prepared = match prepare_derived_pull(conn) {
        Ok(Some(prepared)) => prepared,
        Ok(None) => return PageStep::Failed,
        Err(error) => {
            note(warn, format!("pull stays pending: {error}"));
            return PageStep::Failed;
        }
    };
    let response = match fetch_derived_pull_page(api, token, &prepared).await {
        Ok(response) => response,
        Err(error) => {
            note(warn, format!("pull stays pending: {error}"));
            return PageStep::Failed;
        }
    };
    match install_derived_pull_page(conn, &prepared, &response) {
        Ok(DerivedPullOutcome::Installed(install)) => {
            outcome.pull_pages += 1;
            outcome.rows_installed += install.installed;
            outcome.rows_skipped += install.skipped;
            PageStep::Installed {
                has_more: install.has_more,
            }
        }
        Ok(DerivedPullOutcome::Discarded(reason)) => {
            note(
                warn,
                format!("pull response discarded: {reason:?}; nothing was installed"),
            );
            outcome.discarded = Some(reason);
            PageStep::Discarded
        }
        Err(error) => {
            note(warn, format!("pull install failed: {error}"));
            PageStep::Failed
        }
    }
}

/// Streams the outbox one request at a time, settling each response before
/// building the next request from the next page. A failure, a discard or the
/// request budget leaves the remaining rows `pushed = 0` for the next cycle.
async fn push_outbox<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    max_push_bytes: i64,
    outcome: &mut DerivedCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
) {
    let budget = usize::try_from(max_push_bytes)
        .ok()
        .filter(|bytes| *bytes > 0)
        .unwrap_or(DEFAULT_MAX_PUSH_BYTES as usize);
    let mut prepared = match prepare_derived_push(conn, budget) {
        Ok(Some(prepared)) => prepared,
        Ok(None) => return,
        Err(error) => {
            note(warn, format!("push stays pending: {error}"));
            return;
        }
    };
    let account_id = prepared.scope.account_id.clone();
    let mut skipped_oversize = 0usize;
    let mut requests = 0usize;
    while requests < MAX_PUSH_BATCHES {
        let batch = match next_derived_push_batch(conn, &mut prepared) {
            Ok(Some(batch)) => batch,
            Ok(None) => break,
            Err(error) => {
                note(warn, format!("push stays pending: {error}"));
                return;
            }
        };
        let cache_rows = batch.vectors.len() + batch.ocr_pages.len();
        requests += 1;
        match send_derived_push_batch(api, token, &prepared, &batch).await {
            Ok(response) => match settle_derived_push(conn, &prepared, &batch, &response) {
                Ok(DerivedPushOutcome::Settled(settlement)) => {
                    outcome.push_batches += 1;
                    outcome.rows_pushed += settlement.vectors_marked + settlement.ocr_pages_marked;
                }
                Ok(DerivedPushOutcome::Discarded(reason)) => {
                    note(
                        warn,
                        format!("push response discarded: {reason:?}; nothing was marked"),
                    );
                    outcome.discarded = Some(reason);
                    return;
                }
                Err(error) => {
                    note(warn, format!("push settlement failed: {error}"));
                    return;
                }
            },
            // One row larger than the budget travels alone. If the server still
            // rejects it, skipping it for the rest of this cycle keeps the rows
            // behind it moving; it stays `pushed = 0` and is retried once per
            // cycle, with a single summary line per cycle (§2.5 byte limits).
            Err(SyncError::Api { status: 413, .. }) if cache_rows == 1 => {
                skipped_oversize += 1;
                outcome.skipped_oversize_rows += 1;
                if let Some(vector) = batch.vectors.first() {
                    // The rejected request never stored the contract row either.
                    prepared.contracts_included.remove(&vector.contract_hash);
                }
                continue;
            }
            Err(error) => {
                note(warn, format!("push stays pending: {error}"));
                return;
            }
        }
    }
    if skipped_oversize > 0 {
        note(
            warn,
            format!(
                "{skipped_oversize} derived row(s) exceed the server's max_push_bytes and were skipped this cycle"
            ),
        );
    }
    match has_pending_rows(conn, &account_id) {
        Ok(true) if requests >= MAX_PUSH_BATCHES => note(
            warn,
            format!(
                "push batch budget ({MAX_PUSH_BATCHES}) reached this cycle; the rest stays pending"
            ),
        ),
        Ok(_) => {}
        Err(error) => note(warn, format!("push depth unreadable: {error}")),
    }
}
