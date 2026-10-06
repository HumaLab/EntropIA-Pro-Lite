//! Bounded W-ENGINE activation of the writing sync inside the real sync cycle.
//!
//! This is the orchestrator the engine invokes once per cycle, after the corpus
//! pull and before the cycle is recorded as successful. It composes the already
//! verified inactive modules — `writing_pull` (prepare/fetch/stage),
//! `writing_receive` (prepare/download/settle), `writing_push`
//! (prepare/send/settle) and the W-GUARD2 dirty barrier — under the PROTOCOL
//! activation rules ("Negociación de capacidades de sync" and the writing
//! aggregate section):
//!
//! - Nothing writing-related runs without a complete session that carries a
//!   session incarnation. A stale session never sends or settles anything.
//! - The exact `writing-envelope-v1` capability is only ever trusted from an
//!   ordinary successful response (no opt-in header) recorded for the CURRENT
//!   server epoch, or from a record that already holds it. Opted-in requests
//!   run only after that record exists.
//! - After the capability is recorded, the epoch's since-zero writing catch-up
//!   runs to completion BEFORE incremental writing pulls. A staging cursor that
//!   belongs to a dead session/epoch is discarded so the catch-up restarts from
//!   `since=0`; old rows are never staged under a stale scope.
//! - Writing rows are staged durably (the `writing_receive:` queue) before any
//!   download or apply; blobs download outside every database transaction;
//!   settlement applies and deletes atomically. Corpus rows that ride along
//!   with writing pages go through the existing corpus page machinery and the
//!   shared `last_pull_seq` cursor is never advanced from this phase.
//! - [`crate::writing::sync_transport::pending_local_writes`] defers automatic
//!   application for any document with pending journal or outbox work, leaving
//!   its queue entry retained.
//! - A validated `lww_lost` push result settles its divergent edit through
//!   [`crate::writing::sync_transport::apply_lww_lost_winner`]: the losing
//!   generation becomes exactly one visible conflict copy (enqueued to sync
//!   back) before the winner lands, a pending recovery journal still defers
//!   instead of being overwritten, and a newer local generation is never
//!   cleared by an older result.
//! - Existing local manuscripts are seeded into the outbox
//!   ([`crate::writing::sync_capture::seed_outbox`]) once per cycle, only after
//!   the epoch's catch-up is recorded (the same gate as pushes): remote
//!   tombstones apply first, so a deleted document is never resurrected.
//!   Seeding is idempotent (acknowledged documents and existing entries are
//!   skipped) and a seeding failure stays pending without aborting the phase.
//! - Pushes run only after the epoch's catch-up is recorded, through
//!   prepare/send/settle with the prepared session binding; a logout or session
//!   change between send and settle aborts settlement and keeps the outbox.
//! - Every failure stays pending: outbox entries and queue rows are never
//!   dropped, and the corpus cycle result is never changed by this phase.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::Connection;
use uuid::Uuid;

use super::apply::{apply_page, ApplyContext};
use super::engine::read_schema_tag;
use super::http::{HealthLimits, HealthResponse, PullRow, SyncApi};
use super::session::{ensure_session_incarnation, meta_delete, meta_get, meta_get_i64};
use super::writing_blobs::ensure_writing_blobs_installed;
use super::writing_pull::pull_writing_page;
use super::writing_push::{
    prepare_writing_push, send_prepared_writing_push, settle_writing_push, WritingPushOutcome,
    WritingPushPreparation,
};
use super::writing_receive::{
    download_prepared_writing_receive, prepare_writing_receive, queued_writing_receives,
    settle_writing_receive,
};
use crate::writing::sync_capture::{
    outbox_entries, record_capability, seed_outbox, supports_writing, OutboxAcknowledgment,
    ENVELOPE_TABLE, PULL_CURSOR_KEY,
};
use crate::writing::sync_envelope::WritingEnvelopeV1;
use crate::writing::sync_transport::{
    apply_lww_lost_winner, catchup_needed, pending_local_writes, LwwLostSettlement,
    PullApplyOutcome, PulledWritingRow,
};

/// One ordinary pull page is enough to sample `capabilities` (PROTOCOL: every
/// successful push/pull response advertises them, opt-in or not).
const DISCOVERY_PAGE_LIMIT: i64 = 1;
/// Bounded writing page size (PROTOCOL `limit` ceiling is 1000).
const WRITING_PAGE_LIMIT: i64 = 500;
/// Per-cycle budgets: catch-up may walk the whole account once; incremental
/// pulls and pushes stay small so one cycle can never run unbounded.
const MAX_CATCHUP_PAGES: usize = 64;
const MAX_INCREMENTAL_PAGES: usize = 32;
const MAX_PUSH_ATTEMPTS: usize = 16;
/// The `writing_pull` contract code for a staging cursor that belongs to
/// another account/epoch/session. It is a stable protocol code (the modules
/// share it with `writing_receive`); compared literally because `writing_pull`
/// keeps its constants private.
const PULL_STALE_STAGING: &str = "writing_pull_stale_staging";

/// What capability discovery observed this cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WritingDiscovery {
    /// No complete session identity (or a deferred incarnation mint): nothing
    /// writing-related ran.
    NoSession,
    /// The exact capability was already recorded for the current epoch.
    AlreadyKnown,
    /// An ordinary response advertised the capability; it is now recorded.
    Recorded,
    /// An ordinary response did not advertise the capability.
    NotAdvertised,
    /// The ordinary response came from another epoch; nothing was recorded.
    StaleSample,
    /// The discovery request or its recording failed; retried next cycle.
    Failed,
}

/// Observable result of one writing phase. Counters exist for tests and
/// diagnostics; `pending` carries the retained-work notes the engine logs.
#[derive(Debug, Default)]
pub(crate) struct WritingCycleOutcome {
    pub(crate) discovery: Option<WritingDiscovery>,
    /// True when the epoch still needed its since-zero catch-up at phase start.
    pub(crate) catchup_needed_at_start: bool,
    /// True when this phase recorded the epoch's writing catch-up.
    pub(crate) catchup_recorded: bool,
    pub(crate) pages_staged: usize,
    pub(crate) writing_rows_staged: usize,
    pub(crate) corpus_rows_routed: usize,
    /// Local manuscripts seeded into the outbox this cycle (only documents the
    /// server has never acknowledged; seeding is idempotent).
    pub(crate) documents_seeded: usize,
    pub(crate) receives_settled: usize,
    pub(crate) receives_deferred: usize,
    pub(crate) pushes_settled: usize,
    pub(crate) pushes_pending: usize,
    /// Visible conflict copies preserved for adjudicated divergent edits.
    pub(crate) conflict_copies_preserved: usize,
    pub(crate) pending: Vec<String>,
}

/// The persisted identity one writing phase is bound to. Every field comes from
/// `sync_meta`; the incarnation is the logout/login identity, never guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PhaseScope {
    account_id: String,
    server_url: String,
    device_id: String,
    server_epoch: String,
    session_incarnation: Uuid,
}

/// Runs one bounded writing phase. Never returns a transport failure to the
/// corpus cycle: everything it cannot finish stays pending for the next run.
pub(crate) async fn run_writing_cycle<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    data_root: &Path,
    health: &HealthResponse,
    warn: &(dyn Fn(String) + Sync),
) -> WritingCycleOutcome {
    let mut outcome = WritingCycleOutcome::default();

    // 0. Session scope with incarnation. Missing identity disables the phase.
    let scope = match read_phase_scope(conn) {
        Ok(Some(scope)) => scope,
        Ok(None) => {
            outcome.discovery = Some(WritingDiscovery::NoSession);
            return outcome;
        }
        Err(error) => {
            note(
                &mut outcome,
                warn,
                format!("writing sync deferred: {error}"),
            );
            return outcome;
        }
    };

    // 0b. A rotated server epoch reconciles on the corpus path first; the
    // writing phase waits for the persisted epoch to match this cycle's health.
    if health.epoch != scope.server_epoch {
        note(
            &mut outcome,
            warn,
            "writing sync deferred: health epoch does not match the session epoch".to_string(),
        );
        return outcome;
    }

    // 1. Exact capability for the current epoch, else discover from one
    // ordinary response without opt-in (PROTOCOL "Negociación de capacidades").
    let mut supported = match supports_writing(conn, &scope.server_epoch) {
        Ok(supported) => {
            if supported {
                outcome.discovery = Some(WritingDiscovery::AlreadyKnown);
            }
            supported
        }
        Err(error) => {
            note(
                &mut outcome,
                warn,
                format!("writing sync deferred: {}", error.message),
            );
            return outcome;
        }
    };
    if !supported {
        supported =
            discover_capability(api, token, conn, data_root, &scope, &mut outcome, warn).await;
    }
    if !supported {
        // Legacy server: no opted-in call ever runs and the corpus cycle keeps
        // its exact behavior.
        return outcome;
    }

    // 1b. Shared-document list before pulling, so a document shared since the
    // last cycle applies in this one (`writing::sync_shared`). A server without
    // shares answers an error and the stored list stays as it was.
    if let Ok(shares) = api.list_writing_shares(token).await {
        let rows: Vec<(String, String)> = shares
            .iter()
            .map(|share| {
                let value = serde_json::to_string(share).unwrap_or_default();
                (share.document_id.clone(), value)
            })
            .collect();
        if let Err(error) = crate::writing::sync_shared::replace_shared_documents(conn, &rows) {
            note(
                &mut outcome,
                warn,
                format!("writing share list not stored: {}", error.message),
            );
        }
    }

    // 2. Since-zero writing catch-up before incremental writing pulls.
    outcome.catchup_needed_at_start = match catchup_needed(conn, &scope.server_epoch) {
        Ok(needed) => needed,
        Err(error) => {
            note(
                &mut outcome,
                warn,
                format!("writing sync deferred: {}", error.message),
            );
            return outcome;
        }
    };
    let catchup = outcome.catchup_needed_at_start;
    pull_pages(
        api,
        token,
        conn,
        data_root,
        &health.limits,
        &scope,
        &mut outcome,
        warn,
        catchup,
    )
    .await;

    // 3. Seed existing local manuscripts, then push — both only after the
    // epoch's catch-up is recorded. Seeding behind that gate (the cycle that
    // records it AND every later cycle) lets remote tombstones apply first, so
    // a deleted document is never resurrected into the outbox. `seed_outbox` is
    // idempotent: acknowledged documents and documents that already hold an
    // entry are skipped. A seeding failure stays pending and never aborts the
    // corpus-safe phase.
    let catchup_done = match catchup_needed(conn, &scope.server_epoch) {
        Ok(needed) => !needed,
        Err(error) => {
            note(
                &mut outcome,
                warn,
                format!("writing sync deferred: {}", error.message),
            );
            false
        }
    };
    if catchup_done {
        match seed_outbox(conn) {
            Ok(seeded) => outcome.documents_seeded = seeded,
            Err(error) => note(
                &mut outcome,
                warn,
                format!("writing outbox seeding deferred: {}", error.message),
            ),
        }
        push_loop(
            api,
            token,
            conn,
            data_root,
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

/// Samples `capabilities` from ONE ordinary pull (never an opted-in request) and
/// records the exact token against the response's server epoch. Rows returned
/// by that ordinary response ride through the corpus path untouched by any
/// cursor write. Returns true only when the capability is supported afterwards.
async fn discover_capability<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    data_root: &Path,
    scope: &PhaseScope,
    outcome: &mut WritingCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
) -> bool {
    let schema_tag = match read_schema_tag(conn) {
        Ok(tag) => tag,
        Err(error) => {
            note(
                outcome,
                warn,
                format!("writing capability discovery deferred: {error}"),
            );
            outcome.discovery = Some(WritingDiscovery::Failed);
            return false;
        }
    };
    let since = match meta_get_i64(conn, "last_pull_seq") {
        Ok(since) => since,
        Err(error) => {
            note(
                outcome,
                warn,
                format!("writing capability discovery deferred: {error}"),
            );
            outcome.discovery = Some(WritingDiscovery::Failed);
            return false;
        }
    };

    // Ordinary request: NO capability header, so it can only ever discover the
    // capability, never receive or send writing rows.
    let response = match api
        .pull(token, &schema_tag, since, DISCOVERY_PAGE_LIMIT)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            note(
                outcome,
                warn,
                format!("writing capability discovery deferred: {error}"),
            );
            outcome.discovery = Some(WritingDiscovery::Failed);
            return false;
        }
    };
    if response.server_epoch != scope.server_epoch {
        // A sample from another epoch never enables the transport.
        outcome.discovery = Some(WritingDiscovery::StaleSample);
        return false;
    }

    let advertised = response.supports_writing_envelope_v1();
    route_corpus_rows(conn, &response.rows, data_root, outcome, warn);
    match record_capability(conn, &response.server_epoch, advertised) {
        Ok(()) => {
            outcome.discovery = Some(if advertised {
                WritingDiscovery::Recorded
            } else {
                WritingDiscovery::NotAdvertised
            });
            advertised
        }
        Err(error) => {
            note(
                outcome,
                warn,
                format!("writing capability discovery deferred: {}", error.message),
            );
            outcome.discovery = Some(WritingDiscovery::Failed);
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Pull pages (catch-up / incremental) + receive settlement
// ---------------------------------------------------------------------------

/// One bounded pull loop. `catchup` runs the epoch's since-zero catch-up (pages
/// until `catchup_recorded`, retrying next cycle while rows stay deferred);
/// otherwise it runs incremental writing pulls for this cycle.
#[allow(clippy::too_many_arguments)]
async fn pull_pages<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    data_root: &Path,
    limits: &HealthLimits,
    scope: &PhaseScope,
    outcome: &mut WritingCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
    catchup: bool,
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
                    "writing pull budget ({budget} pages) reached this cycle; remaining rows stay pending"
                ),
            );
            return;
        }
        if !session_unchanged(conn, scope) {
            note(
                outcome,
                warn,
                "sync session changed; staged writing rows stay pending".to_string(),
            );
            return;
        }

        let page = match pull_writing_page(conn, api, token, WRITING_PAGE_LIMIT).await {
            Ok(page) => page,
            Err(pending) => {
                if !session_unchanged(conn, scope) {
                    note(
                        outcome,
                        warn,
                        "sync session changed; staged writing rows stay pending".to_string(),
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
                            format!("writing pull stays pending: {error}"),
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
                        "writing pull stays pending ({}): {}",
                        pending.code, pending.message
                    ),
                );
                return;
            }
        };

        outcome.pages_staged += 1;
        outcome.writing_rows_staged += page.staged_document_ids.len();
        if page.catchup_recorded {
            outcome.catchup_recorded = true;
        }
        route_corpus_rows(conn, &page.corpus_rows, data_root, outcome, warn);
        let settled = settle_queue(api, token, conn, data_root, limits, scope, outcome, warn).await;

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

/// Routes corpus rows that arrived alongside writing rows through the EXISTING
/// corpus page machinery. The shared `last_pull_seq` cursor is re-persisted at
/// its current value (never advanced from this phase), so corpus rows are
/// neither discarded nor able to skip unseen corpus history.
fn route_corpus_rows(
    conn: &Connection,
    rows: &[PullRow],
    app_data_dir: &Path,
    outcome: &mut WritingCycleOutcome,
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
                format!("corpus rows from the writing phase stay pending: {error}"),
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
            format!("corpus rows from the writing phase stay pending: {error}"),
        ),
    }
}

/// Drains the durable `writing_receive:` queue: dirty-barrier check, prepare,
/// blob download OUTSIDE any transaction, then atomic apply + delete. Returns
/// the settled count; every deferral increments `outcome.receives_deferred` and
/// leaves the queue entry retained.
#[allow(clippy::too_many_arguments)]
async fn settle_queue<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    data_root: &Path,
    limits: &HealthLimits,
    scope: &PhaseScope,
    outcome: &mut WritingCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
) -> usize {
    let queued = match queued_writing_receives(conn) {
        Ok(queued) => queued,
        Err(error) => {
            note(
                outcome,
                warn,
                format!("writing receive queue unreadable: {}", error.message),
            );
            return 0;
        }
    };

    let mut settled = 0usize;
    for entry in queued {
        if !session_unchanged(conn, scope) {
            note(
                outcome,
                warn,
                "sync session changed; queued writing rows stay pending".to_string(),
            );
            break;
        }

        // W-GUARD2 durable dirty barrier: a document with pending journal
        // deltas or an unacknowledged outbox entry keeps its queue entry.
        match pending_local_writes(conn, &entry.document_id) {
            Ok(pending) if pending.blocks_automatic_apply() => {
                outcome.receives_deferred += 1;
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                note(
                    outcome,
                    warn,
                    format!(
                        "writing receive for {} deferred: {}",
                        entry.document_id, error.message
                    ),
                );
                outcome.receives_deferred += 1;
                continue;
            }
        }

        let prepared = match prepare_writing_receive(conn, &entry.document_id) {
            Ok(Some(prepared)) => prepared,
            Ok(None) => continue,
            Err(pending) => {
                note(
                    outcome,
                    warn,
                    format!(
                        "writing receive for {} stays pending ({}): {}",
                        entry.document_id, pending.code, pending.message
                    ),
                );
                outcome.receives_deferred += 1;
                continue;
            }
        };

        // Downloads run before any settlement transaction (module contract).
        if let Err(error) =
            download_prepared_writing_receive(api, token, data_root, &prepared, limits).await
        {
            note(
                outcome,
                warn,
                format!(
                    "writing receive for {} stays pending ({:?}): {}",
                    entry.document_id, error.kind, error.message
                ),
            );
            outcome.receives_deferred += 1;
            continue;
        }

        match settle_writing_receive(conn, &prepared, data_root) {
            Ok(PullApplyOutcome::Applied { .. })
            | Ok(PullApplyOutcome::NoOp)
            | Ok(PullApplyOutcome::Stale)
            | Ok(PullApplyOutcome::OwnChangeObserved) => {
                settled += 1;
                outcome.receives_settled += 1;
            }
            Ok(PullApplyOutcome::Deferred { .. }) | Ok(PullApplyOutcome::Unsupported { .. }) => {
                outcome.receives_deferred += 1;
            }
            Err(pending) => {
                note(
                    outcome,
                    warn,
                    format!(
                        "writing receive for {} stays pending ({}): {}",
                        entry.document_id, pending.code, pending.message
                    ),
                );
                outcome.receives_deferred += 1;
            }
        }
    }
    settled
}

// ---------------------------------------------------------------------------
// Push (prepare / send / settle)
// ---------------------------------------------------------------------------

/// Bounded outbox push. Only runs with a recorded catch-up (enforced by
/// `writing_push`'s gate as well): one document per request, blobs uploaded
/// before the row, settlement only while the prepared session is still current.
#[allow(clippy::too_many_arguments)]
async fn push_loop<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    data_root: &Path,
    health: &HealthResponse,
    scope: &PhaseScope,
    outcome: &mut WritingCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
) {
    let mut attempted: HashSet<String> = HashSet::new();
    loop {
        if attempted.len() >= MAX_PUSH_ATTEMPTS {
            note(
                outcome,
                warn,
                format!(
                    "writing push budget ({MAX_PUSH_ATTEMPTS} documents) reached this cycle; the rest stays pending"
                ),
            );
            return;
        }
        if !session_unchanged(conn, scope) {
            note(
                outcome,
                warn,
                "sync session changed; writing outbox stays pending".to_string(),
            );
            return;
        }

        // The exact pending outbox generations BEFORE this iteration's
        // preparation: an `lww_lost` result may settle only the generation its
        // draft was built from. Anything that replaced it mid-flight is newer
        // local work and keeps its outbox entry for its own adjudication.
        let mut adjudications: HashMap<String, OutboxAcknowledgment> = HashMap::new();
        match outbox_entries(conn) {
            Ok(entries) => {
                for entry in entries {
                    adjudications.insert(entry.document_id, entry.acknowledgment);
                }
            }
            Err(error) => {
                outcome.pushes_pending += 1;
                note(
                    outcome,
                    warn,
                    format!("writing outbox unreadable: {}", error.message),
                );
                return;
            }
        }

        match prepare_writing_push(conn, data_root, health) {
            WritingPushPreparation::Idle => return,
            WritingPushPreparation::Pending(pending) => {
                outcome.pushes_pending += 1;
                note(
                    outcome,
                    warn,
                    format!(
                        "writing push stays pending ({:?}): {}",
                        pending.kind, pending.message
                    ),
                );
                return;
            }
            WritingPushPreparation::Ready(prepared) => {
                let adjudicated_generation = adjudications.get(&prepared.document_id).cloned();
                if !attempted.insert(prepared.document_id.clone()) {
                    // A new outbox generation replaced the document mid-cycle:
                    // its fresh draft waits for the next cycle instead of
                    // looping here.
                    return;
                }
                match send_prepared_writing_push(api, token, prepared).await {
                    Err(pending) => {
                        outcome.pushes_pending += 1;
                        note(
                            outcome,
                            warn,
                            format!(
                                "writing push stays pending ({:?}): {}",
                                pending.kind, pending.message
                            ),
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
                                "sync session changed before settlement; writing push stays pending"
                                    .to_string(),
                            );
                            return;
                        }
                        match settle_writing_push(conn, completed) {
                            Ok(WritingPushOutcome::Acknowledged { .. })
                            | Ok(WritingPushOutcome::AlreadySettled { .. }) => {
                                outcome.pushes_settled += 1;
                            }
                            Ok(WritingPushOutcome::NewerGenerationPending { .. }) => {
                                outcome.pushes_pending += 1;
                            }
                            Ok(WritingPushOutcome::StaleResponse {
                                document_id,
                                response_server_seq,
                                recorded_server_seq,
                            }) => {
                                outcome.pushes_pending += 1;
                                note(
                                    outcome,
                                    warn,
                                    format!(
                                        "writing push for {document_id} stays pending: stale response \
                                         seq {response_server_seq} against recorded {recorded_server_seq}"
                                    ),
                                );
                            }
                            Ok(WritingPushOutcome::ConflictPending {
                                document_id,
                                server_seq: _,
                                winner,
                            }) => {
                                settle_lww_lost(
                                    api,
                                    token,
                                    conn,
                                    data_root,
                                    &health.limits,
                                    scope,
                                    &document_id,
                                    winner,
                                    adjudicated_generation,
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
                                    format!(
                                        "writing push stays pending ({:?}): {}",
                                        pending.kind, pending.message
                                    ),
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

// ---------------------------------------------------------------------------
// Adjudicated divergent edits (validated lww_lost winners)
// ---------------------------------------------------------------------------

/// Settles one validated `lww_lost` push result: the server adjudicated the
/// local pending edit as a loser against `winner`. The adjudicated generation
/// survives as exactly one visible conflict copy that syncs back, and the
/// winner lands through the guarded preserve-loser-then-receive path. Anything
/// that cannot settle now stays pending: a pending recovery journal always
/// defers instead of overwriting unsaved work, and a newer local generation
/// keeps its outbox entry for its own adjudication.
#[allow(clippy::too_many_arguments)]
async fn settle_lww_lost<A: SyncApi>(
    api: &A,
    token: &str,
    conn: &Connection,
    data_root: &Path,
    limits: &HealthLimits,
    scope: &PhaseScope,
    document_id: &str,
    winner: PulledWritingRow,
    adjudicated_generation: Option<OutboxAcknowledgment>,
    outcome: &mut WritingCycleOutcome,
    warn: &(dyn Fn(String) + Sync),
) {
    // W-GUARD2 unsaved-work protection: unrecovered journal deltas defer the
    // whole settlement — the winner never overwrites a pending recovery state.
    match pending_local_writes(conn, document_id) {
        Ok(pending) if pending.pending_journal_entries > 0 => {
            outcome.pushes_pending += 1;
            note(
                outcome,
                warn,
                format!(
                    "writing conflict for {document_id} stays pending: {} unrecovered journal \
                     entries must be applied first",
                    pending.pending_journal_entries
                ),
            );
            return;
        }
        Ok(_) => {}
        Err(error) => {
            outcome.pushes_pending += 1;
            note(
                outcome,
                warn,
                format!(
                    "writing conflict for {document_id} stays pending: {}",
                    error.message
                ),
            );
            return;
        }
    }

    // The winner's blobs install OUTSIDE every transaction before the guarded
    // receive can verify them (the pull queue may already be past this row).
    if let Some(payload) = &winner.payload {
        if let Ok(envelope) = WritingEnvelopeV1::from_json(&payload.to_string()) {
            if let Err(error) =
                ensure_writing_blobs_installed(api, token, data_root, &envelope, limits).await
            {
                outcome.pushes_pending += 1;
                note(
                    outcome,
                    warn,
                    format!(
                        "writing conflict for {document_id} stays pending ({:?}): {}",
                        error.kind, error.message
                    ),
                );
                return;
            }
        }
    }

    match apply_lww_lost_winner(
        conn,
        &scope.device_id,
        &winner,
        adjudicated_generation.as_ref(),
        data_root,
    ) {
        Ok(LwwLostSettlement::NewerGenerationPending) => {
            outcome.pushes_pending += 1;
            note(
                outcome,
                warn,
                format!(
                    "writing push for {document_id} stays pending: a newer local generation \
                     replaced the adjudicated push and keeps its outbox entry"
                ),
            );
        }
        Ok(LwwLostSettlement::Routed(PullApplyOutcome::Applied {
            conflict_document_id,
            ..
        })) => {
            outcome.pushes_settled += 1;
            if let Some(conflict_document_id) = conflict_document_id {
                outcome.conflict_copies_preserved += 1;
                warn(format!(
                    "writing conflict copy {conflict_document_id} preserved for {document_id}"
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
                    "writing push for {document_id} stays pending: the adjudicated winner is \
                     already superseded locally"
                ),
            );
        }
        Ok(LwwLostSettlement::Routed(PullApplyOutcome::Deferred { reason }))
        | Ok(LwwLostSettlement::Routed(PullApplyOutcome::Unsupported { reason })) => {
            outcome.pushes_pending += 1;
            note(
                outcome,
                warn,
                format!("writing conflict for {document_id} stays pending: {reason}"),
            );
        }
        Err(error) => {
            outcome.pushes_pending += 1;
            note(
                outcome,
                warn,
                format!(
                    "writing conflict for {document_id} stays pending: {}",
                    error.message
                ),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Session scope
// ---------------------------------------------------------------------------

/// Reads the persisted session identity including the login incarnation.
/// A pre-upgrade session without an incarnation gets exactly one minted here
/// (see [`ensure_session_incarnation`]); anything else missing means "no
/// writing phase this cycle".
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

fn note(outcome: &mut WritingCycleOutcome, warn: &(dyn Fn(String) + Sync), message: String) {
    warn(message.clone());
    outcome.pending.push(message);
}
