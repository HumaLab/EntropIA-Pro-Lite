//! Bounded Investigations research sync activation inside the real sync cycle.
//!
//! This is the orchestrator the engine invokes once per cycle, after the
//! writing phase and before the cycle is recorded as successful. It composes
//! the already verified research modules — `research_pull` (prepare/fetch/
//! stage), `research_receive` (prepare/settle) and `research_blobs` (report
//! blob install) — under the PROTOCOL activation rules ("Negociación de
//! capacidades de sync" and the research aggregate section), mirroring the
//! `writing_cycle` architecture:
//!
//! - When `<app_data_dir>/research/estado.sqlite` does not exist the phase is a
//!   complete no-op: the Investigations engine has no state here.
//! - Nothing research-related runs without a complete session that carries a
//!   session incarnation. A stale session never sends or settles anything.
//! - The exact `research-envelope-v1` capability is only ever trusted from an
//!   ordinary successful response (no opt-in header) recorded for the CURRENT
//!   server epoch, or from a record that already holds it. Opted-in requests
//!   run only after that record exists; a legacy server is quietly skipped.
//! - After the capability is recorded, the epoch's since-zero research catch-up
//!   runs to completion BEFORE incremental research pulls. A staging cursor
//!   that belongs to a dead session/epoch is discarded once per cycle so the
//!   catch-up restarts from `since=0`; old rows are never staged under a stale
//!   scope.
//! - Research rows are staged durably (the `research_receive:` queue) before
//!   any download or apply; report blobs install outside every database
//!   transaction BEFORE the settlement applies the projection (a tombstone
//!   makes no blob call); Deferred/Unsupported rows keep their queue entry
//!   with a bounded reason. Corpus rows that ride along with research pages go
//!   through the existing corpus page machinery and the shared `last_pull_seq`
//!   cursor is never advanced from this phase.
//! - Settlement opens a fresh writable connection to `estado.sqlite` per row
//!   and holds no state connection across an await.
//! - Terminal jobs (including jobs edited after closure) seed into the outbox
//!   ([`seed_terminal_outbox`]) only after the epoch's catch-up is recorded:
//!   remote tombstones apply first, so a deleted job is never resurrected.
//!   Seeding is idempotent and only ever queues terminal jobs.
//! - Pushes run only after the epoch's catch-up is recorded (IS5b activation),
//!   through the same prepare/send/settle shape as `writing_cycle`: one bounded
//!   `research-envelope-v1` request per draft, the report blob uploaded
//!   (HEAD→PUT, re-proof) BEFORE its row (a tombstone has no blob), the clock
//!   offset applied to `changed_at` exactly once, and every response validated
//!   (epoch, exact capability, exactly one matching result, positive server
//!   sequence, known status). Only the exact captured outbox generation is
//!   ever acknowledged; a validated `lww_lost` installs the winner's report
//!   blob before applying it through the receive and conflict machinery and
//!   clears only its adjudicated generation. Pending/invalid/missing-file
//!   outbox entries are retained verbatim and surfaced as pending notes.
//! - Every failure stays pending: outbox entries and queue rows are never
//!   dropped, and the corpus cycle result is never changed by this phase.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use uuid::Uuid;

use super::apply::{apply_page, ApplyContext};
use super::engine::read_schema_tag;
use super::http::{
    HealthLimits, HealthResponse, PullRow, PushRequest, PushResponse, PushResult, SyncApi,
    SyncError,
};
use super::research_blobs::{
    ensure_research_report_installed, ensure_research_report_uploaded, ResearchBlobPendingKind,
};
use super::research_capture::{
    catchup_needed, record_capability, seed_terminal_outbox, supports_research,
    OutboxAcknowledgment, ENVELOPE_TABLE, PULL_CURSOR_KEY,
};
use super::research_envelope::{open_research_state_read_only, ResearchEnvelopeV1, ResearchError};
use super::research_pull::pull_research_page;
use super::research_receive::{
    defer_research_receive, prepare_research_receive, queued_research_receives,
    settle_research_receive,
};
use super::research_transport::{
    apply_lww_lost_winner, build_push_changes, settle_applied_push_draft, LwwLostSettlement,
    PullApplyOutcome, PulledResearchRow, PushDraftSettlement, ResearchChangeDraft,
};
use super::session::{ensure_session_incarnation, meta_delete, meta_get, meta_get_i64};

/// One ordinary pull page is enough to sample `capabilities` (PROTOCOL: every
/// successful push/pull response advertises them, opt-in or not).
const DISCOVERY_PAGE_LIMIT: i64 = 1;
/// Bounded research page size (PROTOCOL `limit` ceiling is 1000).
const RESEARCH_PAGE_LIMIT: i64 = 500;
/// Per-cycle budgets: catch-up may walk the whole account once; incremental
/// pulls and receive settlements stay small so one cycle never runs unbounded.
const MAX_CATCHUP_PAGES: usize = 64;
const MAX_INCREMENTAL_PAGES: usize = 32;
const MAX_RECEIVE_PER_CYCLE: usize = 128;
/// Push attempts per cycle: one draft per request, retried next cycle when the
/// budget runs out. Unsettled entries keep their exact outbox rows.
const MAX_PUSH_ATTEMPTS: usize = 16;
/// The `research_pull` contract code for a staging cursor that belongs to
/// another account/epoch/session. Compared literally because `research_pull`
/// keeps its constants private.
const PULL_STALE_STAGING: &str = "research_pull_stale_staging";

/// What capability discovery observed this cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResearchDiscovery {
    /// `<app_data_dir>/research/estado.sqlite` is absent: no research state,
    /// nothing research-related ran.
    NoState,
    /// No complete session identity (or a deferred incarnation mint): nothing
    /// research-related ran.
    NoSession,
    /// The exact capability was already recorded for the current epoch.
    AlreadyKnown,
    /// An ordinary response advertised the capability; it is now recorded.
    Recorded,
    /// An ordinary response did not advertise the capability (legacy server).
    NotAdvertised,
    /// The ordinary response came from another epoch; nothing was recorded.
    StaleSample,
    /// The discovery request or its recording failed; retried next cycle.
    Failed,
}

/// Observable result of one research phase. Counters exist for tests and
/// diagnostics; `pending` carries the retained-work notes the engine logs.
#[derive(Debug, Default)]
pub(crate) struct ResearchCycleOutcome {
    pub(crate) discovery: Option<ResearchDiscovery>,
    /// True when the epoch still needed its since-zero catch-up at phase start.
    pub(crate) catchup_needed_at_start: bool,
    /// True when this phase recorded the epoch's research catch-up.
    pub(crate) catchup_recorded: bool,
    pub(crate) pages_staged: usize,
    pub(crate) research_rows_staged: usize,
    pub(crate) corpus_rows_routed: usize,
    /// Terminal jobs seeded into the outbox this cycle (idempotent).
    pub(crate) jobs_seeded: usize,
    pub(crate) receives_settled: usize,
    pub(crate) receives_deferred: usize,
    /// Push drafts acknowledged after `applied`/`lww_won`, or settled by an
    /// accepted `lww_lost` winner outcome.
    pub(crate) pushes_settled: usize,
    /// Push attempts and retained outbox entries that stayed pending.
    pub(crate) pushes_pending: usize,
    /// Local losers preserved in `sync_conflicts` by accepted `lww_lost`
    /// settlements this cycle.
    pub(crate) conflicts_preserved: usize,
    pub(crate) pending: Vec<String>,
}

/// The persisted identity one research phase is bound to. Every field comes
/// from `sync_meta`; the incarnation is the logout/login identity, never
/// guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PhaseScope {
    account_id: String,
    server_url: String,
    device_id: String,
    server_epoch: String,
    session_incarnation: Uuid,
}

/// Runs one bounded research phase. Never returns a transport failure to the
/// corpus cycle: everything it cannot finish stays pending for the next run.
/// No-op when the research state database does not exist.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_research_cycle<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    app_data_dir: &Path,
    research_state_path: &Path,
    artifacts_root: &Path,
    health: &HealthResponse,
    warn: &(dyn Fn(String) + Sync),
) -> ResearchCycleOutcome {
    let mut outcome = ResearchCycleOutcome::default();

    // 0. No Investigations state on this machine: the phase is a complete
    // no-op and never touches the wire.
    if !research_state_path.exists() {
        outcome.discovery = Some(ResearchDiscovery::NoState);
        return outcome;
    }

    // 0b. Session scope with incarnation. Missing identity disables the phase.
    let scope = match read_phase_scope(conn) {
        Ok(Some(scope)) => scope,
        Ok(None) => {
            outcome.discovery = Some(ResearchDiscovery::NoSession);
            return outcome;
        }
        Err(error) => {
            note(
                &mut outcome,
                warn,
                format!("research sync deferred: {error}"),
            );
            return outcome;
        }
    };

    // 0c. A rotated server epoch reconciles on the corpus path first; the
    // research phase waits for the persisted epoch to match this cycle's health.
    if health.epoch != scope.server_epoch {
        note(
            &mut outcome,
            warn,
            "research sync deferred: health epoch does not match the session epoch".to_string(),
        );
        return outcome;
    }

    // 1. Exact capability for the current epoch, else discover from one
    // ordinary response without opt-in (PROTOCOL "Negociación de capacidades").
    let mut supported = match supports_research(conn, &scope.server_epoch) {
        Ok(supported) => {
            if supported {
                outcome.discovery = Some(ResearchDiscovery::AlreadyKnown);
            }
            supported
        }
        Err(error) => {
            note(
                &mut outcome,
                warn,
                format!("research sync deferred: {}", error.message),
            );
            return outcome;
        }
    };
    if !supported {
        supported =
            discover_capability(api, token, conn, app_data_dir, &scope, &mut outcome, warn).await;
    }
    if !supported {
        // Legacy server: no opted-in call ever runs and the corpus cycle keeps
        // its exact behavior.
        return outcome;
    }

    // 2. Since-zero research catch-up before incremental research pulls.
    outcome.catchup_needed_at_start = match catchup_needed(conn, &scope.server_epoch) {
        Ok(needed) => needed,
        Err(error) => {
            note(
                &mut outcome,
                warn,
                format!("research sync deferred: {}", error.message),
            );
            return outcome;
        }
    };
    let catchup = outcome.catchup_needed_at_start;
    let mut receive_budget = MAX_RECEIVE_PER_CYCLE;
    pull_pages(
        api,
        token,
        conn,
        app_data_dir,
        research_state_path,
        artifacts_root,
        &health.limits,
        &scope,
        &mut outcome,
        warn,
        catchup,
        &mut receive_budget,
    )
    .await;

    // 3. Seed terminal jobs into the outbox, then push — both only after the
    // epoch's catch-up is recorded (the same gate as pushes), so remote
    // tombstones apply first and a deleted job is never resurrected.
    // `seed_terminal_outbox` is idempotent and only queues terminal jobs; a
    // seeding failure stays pending and never aborts the corpus-safe phase.
    let catchup_done = match catchup_needed(conn, &scope.server_epoch) {
        Ok(needed) => !needed,
        Err(error) => {
            note(
                &mut outcome,
                warn,
                format!("research sync deferred: {}", error.message),
            );
            false
        }
    };
    if catchup_done {
        seed_outbox(conn, research_state_path, &mut outcome, warn);
        push_loop(
            api,
            token,
            conn,
            research_state_path,
            artifacts_root,
            health,
            &scope,
            &mut outcome,
            warn,
        )
        .await;
    }

    outcome
}

// ---------------------------------------------------------------------------
// Capability discovery
// ---------------------------------------------------------------------------

/// Samples `capabilities` from ONE ordinary pull (never an opted-in request)
/// and records the exact token against the response's server epoch. Rows
/// returned by that ordinary response ride through the corpus path untouched by
/// any cursor write. Returns true only when the capability is supported
/// afterwards.
async fn discover_capability<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    app_data_dir: &Path,
    scope: &PhaseScope,
    outcome: &mut ResearchCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
) -> bool {
    let schema_tag = match read_schema_tag(conn) {
        Ok(tag) => tag,
        Err(error) => {
            note(
                outcome,
                warn,
                format!("research capability discovery deferred: {error}"),
            );
            outcome.discovery = Some(ResearchDiscovery::Failed);
            return false;
        }
    };
    let since = match meta_get_i64(conn, "last_pull_seq") {
        Ok(since) => since,
        Err(error) => {
            note(
                outcome,
                warn,
                format!("research capability discovery deferred: {error}"),
            );
            outcome.discovery = Some(ResearchDiscovery::Failed);
            return false;
        }
    };

    // Ordinary request: NO capability header, so it can only ever discover the
    // capability, never receive or send research rows.
    let response = match api
        .pull(token, &schema_tag, since, DISCOVERY_PAGE_LIMIT)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            note(
                outcome,
                warn,
                format!("research capability discovery deferred: {error}"),
            );
            outcome.discovery = Some(ResearchDiscovery::Failed);
            return false;
        }
    };
    if response.server_epoch != scope.server_epoch {
        // A sample from another epoch never enables the transport.
        outcome.discovery = Some(ResearchDiscovery::StaleSample);
        return false;
    }

    let advertised = response.supports_research_envelope_v1();
    route_corpus_rows(conn, &response.rows, app_data_dir, outcome, warn);
    match record_capability(conn, &response.server_epoch, advertised) {
        Ok(()) => {
            outcome.discovery = Some(if advertised {
                ResearchDiscovery::Recorded
            } else {
                ResearchDiscovery::NotAdvertised
            });
            advertised
        }
        Err(error) => {
            note(
                outcome,
                warn,
                format!("research capability discovery deferred: {}", error.message),
            );
            outcome.discovery = Some(ResearchDiscovery::Failed);
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Pull pages (catch-up / incremental) + receive settlement
// ---------------------------------------------------------------------------

/// One bounded pull loop. `catchup` runs the epoch's since-zero catch-up (pages
/// until `catchup_recorded`, retrying next cycle while rows stay deferred);
/// otherwise it runs incremental research pulls for this cycle.
#[allow(clippy::too_many_arguments)]
async fn pull_pages<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    app_data_dir: &Path,
    research_state_path: &Path,
    artifacts_root: &Path,
    limits: &HealthLimits,
    scope: &PhaseScope,
    outcome: &mut ResearchCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
    catchup: bool,
    receive_budget: &mut usize,
) {
    let budget = if catchup {
        MAX_CATCHUP_PAGES
    } else {
        MAX_INCREMENTAL_PAGES
    };
    let mut attempts = 0usize;
    let mut cleared_stale_cursor = false;

    loop {
        attempts += 1;
        if attempts > budget {
            note(
                outcome,
                warn,
                format!(
                    "research pull budget ({budget} pages) reached this cycle; remaining rows stay pending"
                ),
            );
            return;
        }
        if !session_unchanged(conn, scope) {
            note(
                outcome,
                warn,
                "sync session changed; staged research rows stay pending".to_string(),
            );
            return;
        }

        let page = match pull_research_page(conn, api, token, RESEARCH_PAGE_LIMIT).await {
            Ok(page) => page,
            Err(pending) => {
                if !session_unchanged(conn, scope) {
                    note(
                        outcome,
                        warn,
                        "sync session changed; staged research rows stay pending".to_string(),
                    );
                    return;
                }
                if pending.code == PULL_STALE_STAGING && !cleared_stale_cursor {
                    // The stored staging cursor belongs to a dead session or a
                    // rotated epoch. Discarding it restarts this catch-up from
                    // `since=0`; the module never stages old rows under it.
                    if let Err(error) = meta_delete(conn, PULL_CURSOR_KEY) {
                        note(
                            outcome,
                            warn,
                            format!("research pull stays pending: {error}"),
                        );
                        return;
                    }
                    cleared_stale_cursor = true;
                    continue;
                }
                note(
                    outcome,
                    warn,
                    format!(
                        "research pull stays pending ({}): {}",
                        pending.code, pending.message
                    ),
                );
                return;
            }
        };

        outcome.pages_staged += 1;
        outcome.research_rows_staged += page.staged_job_ids.len();
        if page.catchup_recorded {
            outcome.catchup_recorded = true;
        }
        route_corpus_rows(conn, &page.corpus_rows, app_data_dir, outcome, warn);
        let settled = settle_queue(
            api,
            token,
            conn,
            research_state_path,
            artifacts_root,
            limits,
            scope,
            outcome,
            warn,
            receive_budget,
        )
        .await;

        if catchup {
            if page.catchup_recorded {
                return;
            }
            // A final page that reports no more rows but left deferred rows in
            // the queue keeps catch-up pending: the next cycle retries, and the
            // page after a full drain is the one that records it.
            if !page.has_more && settled == 0 {
                return;
            }
        } else if !page.has_more {
            return;
        }
    }
}

/// Routes corpus rows that arrived alongside research rows through the EXISTING
/// corpus page machinery. The shared `last_pull_seq` cursor is re-persisted at
/// its current value (never advanced from this phase), so corpus rows are
/// neither discarded nor able to skip unseen corpus history.
fn route_corpus_rows(
    conn: &Connection,
    rows: &[PullRow],
    app_data_dir: &Path,
    outcome: &mut ResearchCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
) {
    let corpus: Vec<PullRow> = rows
        .iter()
        .filter(|row| row.table != ENVELOPE_TABLE)
        .cloned()
        .collect();
    if corpus.is_empty() {
        return;
    }
    let shared_cursor = match meta_get_i64(conn, "last_pull_seq") {
        Ok(cursor) => cursor,
        Err(error) => {
            note(
                outcome,
                warn,
                format!("corpus rows from the research phase stay pending: {error}"),
            );
            return;
        }
    };
    let mut ctx = ApplyContext::new(app_data_dir);
    match apply_page(conn, &mut ctx, &corpus, shared_cursor) {
        Ok(_) => outcome.corpus_rows_routed += corpus.len(),
        Err(error) => note(
            outcome,
            warn,
            format!("corpus rows from the research phase stay pending: {error}"),
        ),
    }
}

/// Drains the durable `research_receive:` queue: prepare, report blob install
/// OUTSIDE any transaction (a tombstone makes no blob call), then settle into
/// a fresh writable `estado.sqlite` connection. Returns the settled count;
/// every deferral increments `outcome.receives_deferred` and leaves the queue
/// entry retained with a bounded reason.
#[allow(clippy::too_many_arguments)]
async fn settle_queue<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    research_state_path: &Path,
    artifacts_root: &Path,
    limits: &HealthLimits,
    scope: &PhaseScope,
    outcome: &mut ResearchCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
    receive_budget: &mut usize,
) -> usize {
    let queued = match queued_research_receives(conn) {
        Ok(queued) => queued,
        Err(error) => {
            note(
                outcome,
                warn,
                format!("research receive queue unreadable: {}", error.message),
            );
            return 0;
        }
    };

    let mut settled = 0usize;
    for entry in queued {
        if *receive_budget == 0 {
            note(
                outcome,
                warn,
                format!(
                    "research receive budget ({MAX_RECEIVE_PER_CYCLE} rows) reached this cycle; remaining rows stay pending"
                ),
            );
            break;
        }
        if !session_unchanged(conn, scope) {
            note(
                outcome,
                warn,
                "sync session changed; queued research rows stay pending".to_string(),
            );
            break;
        }
        *receive_budget -= 1;

        let prepared = match prepare_research_receive(conn, &entry.job_id) {
            Ok(Some(prepared)) => prepared,
            Ok(None) => continue,
            Err(pending) => {
                note(
                    outcome,
                    warn,
                    format!(
                        "research receive for {} stays pending ({}): {}",
                        entry.job_id, pending.code, pending.message
                    ),
                );
                outcome.receives_deferred += 1;
                continue;
            }
        };

        // Report blob BEFORE apply: the settle installs the projection and
        // would drop an unverified declared report file. On a pending blob the
        // row is retained with the bounded reason and retried next cycle.
        if let Err(reason) =
            ensure_row_report_installed(api, token, artifacts_root, &prepared.row, limits).await
        {
            note(
                outcome,
                warn,
                format!(
                    "research receive for {} stays pending: {reason}",
                    prepared.job_id
                ),
            );
            if let Err(pending) = defer_research_receive(conn, &prepared, &reason) {
                note(
                    outcome,
                    warn,
                    format!(
                        "research receive for {} could not retain its deferral ({}): {}",
                        prepared.job_id, pending.code, pending.message
                    ),
                );
            }
            outcome.receives_deferred += 1;
            continue;
        }

        // A FRESH writable state connection per settlement, dropped before the
        // next await: no state connection is ever held across one.
        let settlement = {
            let state = match rusqlite::Connection::open(research_state_path) {
                Ok(state) => state,
                Err(error) => {
                    note(
                        outcome,
                        warn,
                        format!(
                            "research receive for {} stays pending: cannot open the research state: {error}",
                            prepared.job_id
                        ),
                    );
                    outcome.receives_deferred += 1;
                    continue;
                }
            };
            settle_research_receive(conn, &state, artifacts_root, &prepared)
        };

        match settlement {
            Ok(PullApplyOutcome::Applied { .. })
            | Ok(PullApplyOutcome::NoOp)
            | Ok(PullApplyOutcome::Stale)
            | Ok(PullApplyOutcome::OwnChangeObserved) => {
                settled += 1;
                outcome.receives_settled += 1;
            }
            Ok(PullApplyOutcome::Deferred { reason })
            | Ok(PullApplyOutcome::Unsupported { reason }) => {
                note(
                    outcome,
                    warn,
                    format!(
                        "research receive for {} retained: {reason}",
                        prepared.job_id
                    ),
                );
                outcome.receives_deferred += 1;
            }
            Err(pending) => {
                note(
                    outcome,
                    warn,
                    format!(
                        "research receive for {} stays pending ({}): {}",
                        prepared.job_id, pending.code, pending.message
                    ),
                );
                outcome.receives_deferred += 1;
            }
        }
    }
    settled
}

/// Installs the declared `report.md` of one wire row OUTSIDE every database
/// transaction — a queued receive row or a validated `lww_lost` winner.
/// Tombstones and rows without a readable report manifest make no blob call:
/// their classification belongs to the settle/apply. Returns the bounded
/// reason the row must be retained with when the blob cannot be installed.
async fn ensure_row_report_installed<A: SyncApi>(
    api: &A,
    token: &str,
    artifacts_root: &Path,
    row: &PulledResearchRow,
    limits: &HealthLimits,
) -> Result<(), String> {
    if row.deleted {
        // A tombstone deletes; it never downloads.
        return Ok(());
    }
    let Some(payload) = &row.payload else {
        // No payload: the settle classifies the row as Unsupported and retains.
        return Ok(());
    };
    let Ok(envelope) = ResearchEnvelopeV1::from_json(&payload.to_string()) else {
        // Malformed envelope: the settle classifies and retains it.
        return Ok(());
    };
    if envelope.id != row.job_id {
        // Id mismatch: the settle rejects the row without any download.
        return Ok(());
    }
    let Some(manifest) = envelope.report_file.as_ref() else {
        return Ok(());
    };
    ensure_research_report_installed(api, token, artifacts_root, &row.job_id, manifest, limits)
        .await
        .map(|_| ())
        .map_err(|pending| format!("({:?}) {}", pending.kind, pending.message))
}

/// Seeds terminal jobs into the research outbox behind the recorded catch-up
/// gate. The seed reads the research state through a read-only connection.
fn seed_outbox(
    conn: &Connection,
    research_state_path: &Path,
    outcome: &mut ResearchCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
) {
    let state = match open_research_state_read_only(research_state_path) {
        Ok(state) => state,
        Err(error) => {
            note(
                outcome,
                warn,
                format!("research outbox seeding deferred: {}", error.message),
            );
            return;
        }
    };
    match seed_terminal_outbox(conn, &state) {
        Ok(seeded) => outcome.jobs_seeded = seeded,
        Err(error) => note(
            outcome,
            warn,
            format!("research outbox seeding deferred: {}", error.message),
        ),
    }
}

// ---------------------------------------------------------------------------
// Push (prepare / send / settle)
// ---------------------------------------------------------------------------

/// Typed reasons one research push stays pending. Everything here keeps the
/// exact outbox generation: no failure ever drops local work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResearchPushPendingKind {
    MissingSession,
    EpochMismatch,
    CapabilityUnavailable,
    CatchupIncomplete,
    InvalidLimits,
    RequestTooLarge,
    InvalidDraft,
    /// Outbox entries whose snapshot or report-file proof cannot be completed
    /// right now (pending/invalid/missing-file): retained verbatim.
    UnprovableOutbox,
    Blob(ResearchBlobPendingKind),
    Network,
    Unauthorized,
    AccessDenied,
    RemoteRejected,
    MalformedResponse,
    SessionChanged,
    LocalState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResearchPushPending {
    kind: ResearchPushPendingKind,
    job_ids: Vec<String>,
    message: String,
}

impl ResearchPushPending {
    fn new(kind: ResearchPushPendingKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            job_ids: Vec::new(),
            message: message.into(),
        }
    }

    fn for_job(kind: ResearchPushPendingKind, job_id: &str, message: impl Into<String>) -> Self {
        Self {
            kind,
            job_ids: vec![job_id.to_string()],
            message: message.into(),
        }
    }

    fn for_jobs(
        kind: ResearchPushPendingKind,
        job_ids: Vec<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            job_ids,
            message: message.into(),
        }
    }

    /// One bounded note body: the typed kind, the affected jobs (when the
    /// failure names any), and the human reason.
    fn describe(&self) -> String {
        if self.job_ids.is_empty() {
            format!("({:?}): {}", self.kind, self.message)
        } else {
            format!(
                "({:?}) for {}: {}",
                self.kind,
                self.job_ids.join(", "),
                self.message
            )
        }
    }
}

/// Preparation either yields one bounded single-job request, finds no work, or
/// explains why existing outbox work must remain pending.
// Short-lived, one per cycle: boxing the large variant buys nothing.
#[allow(clippy::large_enum_variant)]
enum ResearchPushPreparation {
    Ready(PreparedResearchPush),
    Idle,
    Pending(ResearchPushPending),
}

/// Immutable state carried across the async network boundary. Every identity
/// field comes from the persisted session, never from a settlement-time guess.
struct PreparedResearchPush {
    job_id: String,
    binding: PhaseScope,
    schema_tag: String,
    limits: HealthLimits,
    artifacts_root: PathBuf,
    request: PushRequest,
    draft: ResearchChangeDraft,
}

struct CompletedResearchPush {
    binding: PhaseScope,
    draft: ResearchChangeDraft,
    result: ValidatedPushResult,
}

#[derive(Debug)]
enum ValidatedPushResult {
    /// A well-formed `applied`/`lww_won` result. Settlement is delegated to
    /// [`settle_applied_push_draft`] with the exact captured generation.
    Accepted { result: PushResult },
    /// A well-formed `lww_lost` result whose winner binds this row and sequence.
    Conflict { winner: PulledResearchRow },
}

/// What settling one prepared push produced.
enum ResearchPushOutcome {
    Acknowledged,
    /// The captured generation was already settled earlier: reported, and
    /// never able to clear a newer generation.
    AlreadySettled,
    NewerGenerationPending,
    StaleResponse {
        response_server_seq: i64,
        recorded_server_seq: i64,
    },
    ConflictPending {
        winner: PulledResearchRow,
        adjudicated: OutboxAcknowledgment,
    },
}

/// Captures one coherent outbox snapshot and preflights the exact JSON request
/// body against the server-advertised byte limit. Reads only: building a push
/// never acknowledges, clears or rewrites an outbox entry, and the transport's
/// own savepoints own every write-side decision.
fn prepare_research_push(
    conn: &Connection,
    research_state_path: &Path,
    artifacts_root: &Path,
    health: &HealthResponse,
) -> ResearchPushPreparation {
    match prepare_research_push_inner(conn, research_state_path, artifacts_root, health) {
        Ok(Some(prepared)) => ResearchPushPreparation::Ready(prepared),
        Ok(None) => ResearchPushPreparation::Idle,
        Err(pending) => ResearchPushPreparation::Pending(pending),
    }
}

fn prepare_research_push_inner(
    conn: &Connection,
    research_state_path: &Path,
    artifacts_root: &Path,
    health: &HealthResponse,
) -> Result<Option<PreparedResearchPush>, ResearchPushPending> {
    if health.epoch.is_empty() {
        return Err(ResearchPushPending::new(
            ResearchPushPendingKind::EpochMismatch,
            "health response did not identify a server epoch",
        ));
    }
    let binding = read_phase_scope(conn)
        .map_err(|error| ResearchPushPending::new(ResearchPushPendingKind::LocalState, error))?
        .ok_or_else(|| {
            ResearchPushPending::new(
                ResearchPushPendingKind::MissingSession,
                "sync session metadata is incomplete",
            )
        })?;
    if binding.server_epoch != health.epoch {
        return Err(ResearchPushPending::new(
            ResearchPushPendingKind::EpochMismatch,
            "persisted session epoch does not match the current health response",
        ));
    }
    require_research_gate(conn, &binding.server_epoch)?;

    let max_push_bytes = usize::try_from(health.limits.max_push_bytes)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            ResearchPushPending::new(
                ResearchPushPendingKind::InvalidLimits,
                "server did not advertise a positive max_push_bytes limit",
            )
        })?;
    let schema_tag = read_schema_tag(conn).map_err(|message| {
        ResearchPushPending::new(ResearchPushPendingKind::LocalState, message)
    })?;
    // Applied to `changed_at` EXACTLY once, here at request build: drafts carry
    // capture-time values and settlement never re-reads the offset.
    let clock_offset = super::push::clock_offset(conn).map_err(|message| {
        ResearchPushPending::new(ResearchPushPendingKind::LocalState, message)
    })?;
    let build = build_push_changes(conn, research_state_path, artifacts_root).map_err(|error| {
        ResearchPushPending::new(
            ResearchPushPendingKind::LocalState,
            format!("{}: {}", error.code, error.message),
        )
    })?;
    let Some(draft) = build.ready.into_iter().next() else {
        if build.pending.is_empty() {
            return Ok(None);
        }
        // Pending/invalid/missing-file entries keep their EXACT outbox rows:
        // they are listed here, never dropped, and retried next cycle.
        return Err(ResearchPushPending::for_jobs(
            ResearchPushPendingKind::UnprovableOutbox,
            build
                .pending
                .iter()
                .map(|entry| entry.job_id.clone())
                .collect(),
            "research outbox entries have unprovable snapshots or report files; they stay pending",
        ));
    };

    // One bounded request per draft. `D` tombstones serialize with no payload;
    // `U` drafts carry the canonical envelope of their terminal job.
    let mut change = draft.to_push_change();
    change.changed_at = change.changed_at.saturating_add(clock_offset);
    let request = PushRequest {
        changes: vec![change],
    };
    let serialized_request_bytes = serde_json::to_vec(&request)
        .map_err(|error| {
            ResearchPushPending::for_job(
                ResearchPushPendingKind::InvalidDraft,
                &draft.job_id,
                format!("cannot serialize research push request: {error}"),
            )
        })?
        .len();
    if serialized_request_bytes > max_push_bytes {
        return Err(ResearchPushPending::for_job(
            ResearchPushPendingKind::RequestTooLarge,
            &draft.job_id,
            format!(
                "research push request is {serialized_request_bytes} bytes; advertised limit is \
                 {max_push_bytes} bytes"
            ),
        ));
    }
    if draft.op != 'D' {
        let payload = draft.payload.as_ref().ok_or_else(|| {
            ResearchPushPending::for_job(
                ResearchPushPendingKind::InvalidDraft,
                &draft.job_id,
                "research upsert draft has no payload",
            )
        })?;
        let envelope = ResearchEnvelopeV1::from_json(&payload.to_string()).map_err(|error| {
            ResearchPushPending::for_job(
                ResearchPushPendingKind::InvalidDraft,
                &draft.job_id,
                error.message,
            )
        })?;
        if envelope.id != draft.job_id {
            return Err(ResearchPushPending::for_job(
                ResearchPushPendingKind::InvalidDraft,
                &draft.job_id,
                "research envelope id does not match its outbox job",
            ));
        }
    }

    Ok(Some(PreparedResearchPush {
        job_id: draft.job_id.clone(),
        binding,
        schema_tag,
        limits: health.limits.clone(),
        artifacts_root: artifacts_root.to_path_buf(),
        request,
        draft,
    }))
}

/// Uploads the proven report blob (HEAD → PUT with a re-proof) BEFORE the row,
/// then sends the exact opted-in envelope request. No database handle is
/// accepted, so this async phase cannot hold a SQLite lock or acknowledge
/// local work. The caller must still bind both `api` and `token` to the
/// prepared session: the incarnation only rejects a stale settlement.
async fn send_prepared_research_push<A: SyncApi>(
    api: &A,
    token: &str,
    prepared: PreparedResearchPush,
) -> Result<CompletedResearchPush, ResearchPushPending> {
    let PreparedResearchPush {
        job_id,
        binding,
        schema_tag,
        limits,
        artifacts_root,
        request,
        draft,
    } = prepared;

    // Report blob BEFORE the row. A `D` tombstone carries no upload plan and
    // never touches a blob endpoint.
    if let Some(plan) = &draft.report_upload {
        ensure_research_report_uploaded(api, token, &artifacts_root, plan, &limits)
            .await
            .map_err(|pending| {
                ResearchPushPending::for_job(
                    ResearchPushPendingKind::Blob(pending.kind),
                    &job_id,
                    pending.message,
                )
            })?;
    }

    let response = api
        .push_with_research_envelope_v1(token, &schema_tag, request)
        .await
        .map_err(|error| pending_from_sync(&job_id, error))?;
    let result = validate_push_response(response, &job_id, &binding.server_epoch)?;
    Ok(CompletedResearchPush {
        binding,
        draft,
        result,
    })
}

/// Rechecks the persisted account/server/device/epoch/incarnation scope and the
/// push gate, then settles the validated response. `applied`/`lww_won` delegate
/// to [`settle_applied_push_draft`] (only the exact captured generation is ever
/// acknowledged); `lww_lost` returns its winner for the async settle that
/// installs the winner's report blob and routes it through the receive and
/// conflict machinery.
fn settle_research_push(
    conn: &Connection,
    completed: CompletedResearchPush,
) -> Result<ResearchPushOutcome, ResearchPushPending> {
    let job_id = completed.draft.job_id.clone();
    let current = read_phase_scope(conn)
        .map_err(|error| {
            ResearchPushPending::for_job(ResearchPushPendingKind::LocalState, &job_id, error)
        })?
        .ok_or_else(|| {
            ResearchPushPending::for_job(
                ResearchPushPendingKind::MissingSession,
                &job_id,
                "sync session metadata is incomplete",
            )
        })?;
    if current != completed.binding {
        return Err(ResearchPushPending::for_job(
            ResearchPushPendingKind::SessionChanged,
            &job_id,
            "sync session changed after the research request was prepared",
        ));
    }
    require_research_gate(conn, &completed.binding.server_epoch)?;

    match completed.result {
        ValidatedPushResult::Accepted { result } => {
            let settlement = settle_applied_push_draft(conn, &completed.draft, &result)
                .map_err(|error| pending_from_research(&job_id, error))?;
            Ok(match settlement {
                PushDraftSettlement::Acknowledged => ResearchPushOutcome::Acknowledged,
                PushDraftSettlement::NewerGenerationPending => {
                    ResearchPushOutcome::NewerGenerationPending
                }
                PushDraftSettlement::AlreadySettled => ResearchPushOutcome::AlreadySettled,
                PushDraftSettlement::StaleServerSequence {
                    recorded_server_seq,
                    response_server_seq,
                } => ResearchPushOutcome::StaleResponse {
                    response_server_seq,
                    recorded_server_seq,
                },
            })
        }
        ValidatedPushResult::Conflict { winner } => Ok(ResearchPushOutcome::ConflictPending {
            winner,
            adjudicated: completed.draft.acknowledgment.clone(),
        }),
    }
}

/// Validates one opted-in research push response before anything settles:
/// epoch, exact capability advertisement, exactly one result bound to the
/// requested row, a valid positive server sequence, and a known status.
fn validate_push_response(
    response: PushResponse,
    job_id: &str,
    expected_epoch: &str,
) -> Result<ValidatedPushResult, ResearchPushPending> {
    if response.server_epoch != expected_epoch {
        return Err(ResearchPushPending::for_job(
            ResearchPushPendingKind::EpochMismatch,
            job_id,
            "research push response epoch does not match the prepared epoch",
        ));
    }
    if !response.supports_research_envelope_v1() {
        return Err(ResearchPushPending::for_job(
            ResearchPushPendingKind::CapabilityUnavailable,
            job_id,
            "research push response omitted research-envelope-v1 capability",
        ));
    }
    if response.results.len() != 1 {
        return Err(ResearchPushPending::for_job(
            ResearchPushPendingKind::MalformedResponse,
            job_id,
            format!(
                "research push expected exactly one result, received {}",
                response.results.len()
            ),
        ));
    }
    let result = response.results.into_iter().next().ok_or_else(|| {
        ResearchPushPending::for_job(
            ResearchPushPendingKind::MalformedResponse,
            job_id,
            "research push result is absent",
        )
    })?;
    if result.table != ENVELOPE_TABLE || result.row_id != job_id {
        return Err(ResearchPushPending::for_job(
            ResearchPushPendingKind::MalformedResponse,
            job_id,
            "research push result does not match the requested table and row",
        ));
    }
    if result.server_seq <= 0 || response.max_server_seq < result.server_seq {
        return Err(ResearchPushPending::for_job(
            ResearchPushPendingKind::MalformedResponse,
            job_id,
            "research push result has an invalid server sequence",
        ));
    }

    match result.status.as_str() {
        "applied" | "lww_won" => {
            if result.winner.is_some() {
                return Err(ResearchPushPending::for_job(
                    ResearchPushPendingKind::MalformedResponse,
                    job_id,
                    "successful research push result unexpectedly included a winner",
                ));
            }
            Ok(ValidatedPushResult::Accepted { result })
        }
        "lww_lost" => {
            let winner = result.winner.ok_or_else(|| {
                ResearchPushPending::for_job(
                    ResearchPushPendingKind::MalformedResponse,
                    job_id,
                    "lww_lost research result omitted its winner",
                )
            })?;
            Ok(ValidatedPushResult::Conflict {
                winner: validate_winner(winner, job_id, result.server_seq)?,
            })
        }
        _ => Err(ResearchPushPending::for_job(
            ResearchPushPendingKind::MalformedResponse,
            job_id,
            format!(
                "research push returned unsupported status {:?}",
                result.status
            ),
        )),
    }
}

/// Binds one `lww_lost` winner to the requested research row and adjudicated
/// sequence: the table and row, the exact server sequence, and a payload that
/// is either a tombstone or a well-formed envelope with the row's own id.
fn validate_winner(
    winner: PullRow,
    job_id: &str,
    result_server_seq: i64,
) -> Result<PulledResearchRow, ResearchPushPending> {
    if winner.table != ENVELOPE_TABLE
        || winner.row_id != job_id
        || winner.server_seq <= 0
        || winner.server_seq != result_server_seq
        || winner.changed_at < 0
        || winner.device_id.is_empty()
    {
        return Err(ResearchPushPending::for_job(
            ResearchPushPendingKind::MalformedResponse,
            job_id,
            "lww_lost winner does not bind the requested research row and sequence",
        ));
    }
    if winner.deleted {
        if winner.payload.is_some() {
            return Err(ResearchPushPending::for_job(
                ResearchPushPendingKind::MalformedResponse,
                job_id,
                "deleted lww_lost winner unexpectedly included a payload",
            ));
        }
    } else {
        let payload = winner.payload.as_ref().ok_or_else(|| {
            ResearchPushPending::for_job(
                ResearchPushPendingKind::MalformedResponse,
                job_id,
                "lww_lost winner omitted its research envelope",
            )
        })?;
        let envelope = ResearchEnvelopeV1::from_json(&payload.to_string()).map_err(|error| {
            ResearchPushPending::for_job(
                ResearchPushPendingKind::MalformedResponse,
                job_id,
                format!("lww_lost winner envelope is invalid: {}", error.message),
            )
        })?;
        if envelope.id != job_id {
            return Err(ResearchPushPending::for_job(
                ResearchPushPendingKind::MalformedResponse,
                job_id,
                "lww_lost winner envelope id does not match the requested row",
            ));
        }
    }
    PulledResearchRow::from_wire_row(&winner).map_err(|error| {
        ResearchPushPending::for_job(
            ResearchPushPendingKind::MalformedResponse,
            job_id,
            format!("lww_lost winner is not a research row: {}", error.message),
        )
    })
}

/// Bounded outbox push. Only runs with a recorded catch-up (enforced by
/// [`require_research_gate`] as well): one draft per request, the report blob
/// uploaded before its row, settlement only while the prepared session is
/// still current. Failures keep their exact outbox rows and stop the loop for
/// this cycle; the corpus cycle result is never changed.
#[allow(clippy::too_many_arguments)]
async fn push_loop<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    research_state_path: &Path,
    artifacts_root: &Path,
    health: &HealthResponse,
    scope: &PhaseScope,
    outcome: &mut ResearchCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
) {
    let mut attempted: HashSet<String> = HashSet::new();
    loop {
        if attempted.len() >= MAX_PUSH_ATTEMPTS {
            outcome.pushes_pending += 1;
            note(
                outcome,
                warn,
                format!(
                    "research push budget ({MAX_PUSH_ATTEMPTS} jobs) reached this cycle; the rest \
                     stays pending"
                ),
            );
            return;
        }
        if !session_unchanged(conn, scope) {
            outcome.pushes_pending += 1;
            note(
                outcome,
                warn,
                "sync session changed; research outbox stays pending".to_string(),
            );
            return;
        }

        match prepare_research_push(conn, research_state_path, artifacts_root, health) {
            ResearchPushPreparation::Idle => return,
            ResearchPushPreparation::Pending(pending) => {
                // Jobs already attempted this cycle were counted at their own
                // outcome; only fresh retained work adds to the counter.
                let fresh = pending
                    .job_ids
                    .iter()
                    .filter(|job_id| !attempted.contains(job_id.as_str()))
                    .count();
                outcome.pushes_pending += if pending.job_ids.is_empty() { 1 } else { fresh };
                note(
                    outcome,
                    warn,
                    format!("research push stays pending {}", pending.describe()),
                );
                return;
            }
            ResearchPushPreparation::Ready(prepared) => {
                let job_id = prepared.job_id.clone();
                if !attempted.insert(job_id.clone()) {
                    // A new outbox generation replaced the job mid-cycle: its
                    // fresh draft waits for the next cycle instead of looping.
                    return;
                }
                match send_prepared_research_push(api, token, prepared).await {
                    Err(pending) => {
                        outcome.pushes_pending += 1;
                        note(
                            outcome,
                            warn,
                            format!("research push stays pending {}", pending.describe()),
                        );
                        return;
                    }
                    Ok(completed) => {
                        if !session_unchanged(conn, scope) {
                            // Never settle a prepared push under a different
                            // account/epoch/incarnation: the outbox entry stays.
                            outcome.pushes_pending += 1;
                            note(
                                outcome,
                                warn,
                                "sync session changed before settlement; research push stays pending"
                                    .to_string(),
                            );
                            return;
                        }
                        match settle_research_push(conn, completed) {
                            Ok(ResearchPushOutcome::Acknowledged) => {
                                outcome.pushes_settled += 1;
                            }
                            Ok(ResearchPushOutcome::AlreadySettled) => {
                                outcome.pushes_settled += 1;
                                warn(format!(
                                    "research push for {job_id} was already settled; nothing was cleared"
                                ));
                            }
                            Ok(ResearchPushOutcome::NewerGenerationPending) => {
                                outcome.pushes_pending += 1;
                                note(
                                    outcome,
                                    warn,
                                    format!(
                                        "research push for {job_id} stays pending: a newer local \
                                         generation replaced the adjudicated push and keeps its \
                                         outbox entry"
                                    ),
                                );
                            }
                            Ok(ResearchPushOutcome::StaleResponse {
                                response_server_seq,
                                recorded_server_seq,
                            }) => {
                                outcome.pushes_pending += 1;
                                note(
                                    outcome,
                                    warn,
                                    format!(
                                        "research push for {job_id} stays pending: stale response \
                                         seq {response_server_seq} against recorded \
                                         {recorded_server_seq}"
                                    ),
                                );
                            }
                            Ok(ResearchPushOutcome::ConflictPending {
                                winner,
                                adjudicated,
                            }) => {
                                settle_lww_lost(
                                    api,
                                    token,
                                    conn,
                                    research_state_path,
                                    artifacts_root,
                                    &health.limits,
                                    scope,
                                    &job_id,
                                    winner,
                                    adjudicated,
                                    outcome,
                                    warn,
                                )
                                .await;
                            }
                            Err(pending) => {
                                outcome.pushes_pending += 1;
                                note(
                                    outcome,
                                    warn,
                                    format!("research push stays pending {}", pending.describe()),
                                );
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Settles one validated `lww_lost` push result: the server adjudicated the
/// pushed generation as a loser against its winner. The winner's report blob
/// installs OUTSIDE every transaction before the receive/conflict machinery
/// applies it (a tombstone winner makes no blob call); the losing local
/// envelope survives in `sync_conflicts`, and only the exact adjudicated
/// generation is cleared — and only after an accepted `Applied`/`NoOp`.
/// Deferred/Unsupported/Stale/OwnChangeObserved outcomes retain it.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
async fn settle_lww_lost<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    research_state_path: &Path,
    artifacts_root: &Path,
    limits: &HealthLimits,
    scope: &PhaseScope,
    job_id: &str,
    winner: PulledResearchRow,
    adjudicated: OutboxAcknowledgment,
    outcome: &mut ResearchCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
) {
    if !session_unchanged(conn, scope) {
        outcome.pushes_pending += 1;
        note(
            outcome,
            warn,
            format!("research conflict for {job_id} stays pending: sync session changed"),
        );
        return;
    }

    if let Err(reason) =
        ensure_row_report_installed(api, token, artifacts_root, &winner, limits).await
    {
        outcome.pushes_pending += 1;
        note(
            outcome,
            warn,
            format!("research conflict for {job_id} stays pending: {reason}"),
        );
        return;
    }

    if !session_unchanged(conn, scope) {
        outcome.pushes_pending += 1;
        note(
            outcome,
            warn,
            format!("research conflict for {job_id} stays pending: sync session changed"),
        );
        return;
    }

    // A FRESH writable research state connection per settlement, dropped
    // before any further await (none follows): no state connection is ever
    // held across one.
    let settlement = {
        let state = match rusqlite::Connection::open(research_state_path) {
            Ok(state) => state,
            Err(error) => {
                outcome.pushes_pending += 1;
                note(
                    outcome,
                    warn,
                    format!(
                        "research conflict for {job_id} stays pending: cannot open the research \
                         state: {error}"
                    ),
                );
                return;
            }
        };
        apply_lww_lost_winner(conn, &state, artifacts_root, &winner, Some(&adjudicated))
    };

    match settlement {
        Ok(LwwLostSettlement::NewerGenerationPending) => {
            outcome.pushes_pending += 1;
            note(
                outcome,
                warn,
                format!(
                    "research push for {job_id} stays pending: a newer local generation replaced \
                     the adjudicated push and keeps its outbox entry"
                ),
            );
        }
        Ok(LwwLostSettlement::Routed(PullApplyOutcome::Applied { conflict_id, .. })) => {
            outcome.pushes_settled += 1;
            if let Some(conflict_id) = conflict_id {
                outcome.conflicts_preserved += 1;
                warn(format!(
                    "research conflict copy {conflict_id} preserved for {job_id}"
                ));
            }
        }
        Ok(LwwLostSettlement::Routed(PullApplyOutcome::NoOp)) => {
            outcome.pushes_settled += 1;
        }
        Ok(LwwLostSettlement::Routed(PullApplyOutcome::Stale))
        | Ok(LwwLostSettlement::Routed(PullApplyOutcome::OwnChangeObserved)) => {
            outcome.pushes_pending += 1;
            note(
                outcome,
                warn,
                format!(
                    "research push for {job_id} stays pending: the adjudicated winner is already \
                     superseded locally"
                ),
            );
        }
        Ok(LwwLostSettlement::Routed(PullApplyOutcome::Deferred { reason }))
        | Ok(LwwLostSettlement::Routed(PullApplyOutcome::Unsupported { reason })) => {
            outcome.pushes_pending += 1;
            note(
                outcome,
                warn,
                format!("research conflict for {job_id} stays pending: {reason}"),
            );
        }
        Err(error) => {
            outcome.pushes_pending += 1;
            note(
                outcome,
                warn,
                format!(
                    "research conflict for {job_id} stays pending: {}",
                    error.message
                ),
            );
        }
    }
}

/// The push gate: the EXACT `research-envelope-v1` capability must already be
/// recorded for the epoch and the epoch's since-zero catch-up must be
/// complete. A legacy server or an unfinished catch-up never sends anything.
fn require_research_gate(conn: &Connection, server_epoch: &str) -> Result<(), ResearchPushPending> {
    let supported = supports_research(conn, server_epoch).map_err(|error| {
        ResearchPushPending::new(
            ResearchPushPendingKind::LocalState,
            format!("{}: {}", error.code, error.message),
        )
    })?;
    if !supported {
        return Err(ResearchPushPending::new(
            ResearchPushPendingKind::CapabilityUnavailable,
            "research-envelope-v1 was not advertised for the current server epoch",
        ));
    }
    if catchup_needed(conn, server_epoch).map_err(|error| {
        ResearchPushPending::new(
            ResearchPushPendingKind::LocalState,
            format!("{}: {}", error.code, error.message),
        )
    })? {
        return Err(ResearchPushPending::new(
            ResearchPushPendingKind::CatchupIncomplete,
            "full research catch-up is incomplete for the current server epoch",
        ));
    }
    Ok(())
}

fn pending_from_research(job_id: &str, error: ResearchError) -> ResearchPushPending {
    ResearchPushPending::for_job(
        ResearchPushPendingKind::LocalState,
        job_id,
        format!("{}: {}", error.code, error.message),
    )
}

fn pending_from_sync(job_id: &str, error: SyncError) -> ResearchPushPending {
    let kind = match &error {
        SyncError::Network(_) => ResearchPushPendingKind::Network,
        SyncError::Api { status: 401, .. } => ResearchPushPendingKind::Unauthorized,
        SyncError::Api { status: 403, .. } => ResearchPushPendingKind::AccessDenied,
        SyncError::Decode(_) => ResearchPushPendingKind::MalformedResponse,
        SyncError::InvalidUrl(_) | SyncError::Api { .. } => ResearchPushPendingKind::RemoteRejected,
    };
    ResearchPushPending::for_job(kind, job_id, error.to_string())
}

// ---------------------------------------------------------------------------
// Session scope
// ---------------------------------------------------------------------------

/// Reads the persisted session identity including the login incarnation.
/// A pre-upgrade session without an incarnation gets exactly one minted here
/// (see [`ensure_session_incarnation`]); anything else missing means "no
/// research phase this cycle".
fn read_phase_scope(conn: &Connection) -> Result<Option<PhaseScope>, String> {
    let account_id = meta_get(conn, "account_id")?;
    let server_url = meta_get(conn, "server_url")?;
    let device_id = meta_get(conn, "device_id")?;
    let server_epoch = meta_get(conn, "server_epoch")?;
    let session_incarnation = ensure_session_incarnation(conn)?;
    Ok(
        match (
            account_id,
            server_url,
            device_id,
            server_epoch,
            session_incarnation,
        ) {
            (
                Some(account_id),
                Some(server_url),
                Some(device_id),
                Some(server_epoch),
                Some(session_incarnation),
            ) => Some(PhaseScope {
                account_id,
                server_url,
                device_id,
                server_epoch,
                session_incarnation,
            }),
            _ => None,
        },
    )
}

/// True while the persisted identity is still the exact one this phase started
/// with. A logout or re-login mid-cycle stops every send and settlement.
fn session_unchanged(conn: &Connection, scope: &PhaseScope) -> bool {
    matches!(read_phase_scope(conn), Ok(Some(current)) if &current == scope)
}

fn note(outcome: &mut ResearchCycleOutcome, warn: &(dyn Fn(String) + Sync), message: String) {
    warn(message.clone());
    outcome.pending.push(message);
}
