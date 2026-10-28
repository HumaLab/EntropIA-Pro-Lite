//! Pull-before-pay gate (plan `odd/plans/plan-sync-bibliografia-derivada.md`
//! §2.6, decision D5, spec S5).
//!
//! The cache of [`super::derived`] only saves money when the answer is already
//! local before the device pays. A device that boots with the catalogue and the
//! paid backlog at the same time would otherwise start OCR and embeddings while
//! the `bibliography-derived-v1` channel is still catching up, and pay for what
//! another device of the account already bought. This module is the one
//! question every paid path asks before its FIRST provider call:
//!
//! > the cache answered the misses it could; for what is left, is the cache as
//! > complete as it can get right now, or should the unit wait?
//!
//! The answer is a [`GateVerdict`]: [`GateVerdict::Proceed`] (pay only the
//! misses) or [`GateVerdict::Defer`] (park the unit and re-run it later).
//!
//! States (§2.6), in this order:
//!
//! | State | Condition | Verdict |
//! | --- | --- | --- |
//! | No session | no `account_id` in `sync_meta` | pay: the cache was already read with the effective account (§2.9) and nothing will arrive by itself |
//! | Unknown capability | a session but no verdict for the current epoch (or no epoch yet) | wait, bounded |
//! | Supported, catch-up pending | advertised for this epoch and its since-zero catch-up has not finished | wait, bounded |
//! | Supported and caught up | advertised and the catch-up finished | freshness barrier: see below |
//! | Unsupported (confirmed) | this epoch's record says `advertised: false` | pay as before: a legacy server will never send the cache |
//!
//! **Freshness barrier.** "Caught up" says the epoch's catch-up finished once,
//! not that the local cache holds what another device uploaded since. So before
//! a miss is paid, the last successful incremental pull of the channel must be
//! *later than the unit's admission* (`processing_tasks.created_at`, which
//! follows the Zotero synchronisation that admitted the work). When it is not,
//! the gate requests one incremental pull
//! ([`super::derived::request_incremental_pull`], coalesced: many units, one
//! flag, one pull, and the request bypasses the transport's 60 s incremental
//! spacing at most once) and parks the unit.
//!
//! There is deliberately no age bound on that pull (D5's `F` was dropped):
//! freshness is per admission, not per clock. A Zotero synchronisation admits
//! the whole backlog at once and the device processes it for hours, so an age
//! bound would make every later unit with a miss ask for one more pull and park
//! ~60 s for it — on the device that actually pays, a large throughput loss for
//! no protection S5 asks for.
//!
//! **Bounded wait (D5), per wait episode.** [`DERIVED_WAIT_MAX_MS`] caps each
//! wait from the instant that wait began, and there are two kinds:
//!
//! - **First boot** (unknown capability, catch-up pending) is a property of the
//!   epoch that the whole device shares, so its first instant is recorded in
//!   `sync_meta['derived_wait_verdict']` and every unit of the epoch shares one
//!   deadline — sized for the ~400 MiB catch-up of a library like the plan's
//!   §1.5, not for a short pull. The memory dies with the epoch (and with
//!   logout: `derived::clear_account_metadata` drops the key).
//! - **A stale cache** is a property of ONE unit's admission: the epilogue of a
//!   catalogue synchronisation that may postdate the last pull. Its deadline is
//!   anchored at that unit's own admission, so a backlog that starts hours after
//!   the epoch's first boot still waits for its own pull instead of inheriting
//!   an epoch-wide deadline that expired long ago. Only a unit that has waited
//!   its whole budget for its own pull pays without one.
//!
//! Past an episode's deadline the unit pays without waiting.
//!
//! **Never blocks the user's work.** The wait is never a sleep inside the
//! scheduler: the executors report the wait as `ExecOutput::Deferred` carrying
//! the `resume_at_ms` this module computed and [`DERIVED_WAIT_CODE`], and the
//! supervisor parks the unit in `retry_wait` with
//! [`crate::processing::repository::defer_task`] until then. Nothing is charged
//! to the retry budget and no failure is recorded. The sync engine, not this
//! module, discovers the capability and runs the catch-up: the gate only
//! observes its state and asks for a pull.
//!
//! The cache saves money but never withholds the paid work: any read that
//! cannot answer makes the gate proceed, and an offline device is not visible
//! from here (the engine's last cycle error lives in its own in-memory status),
//! so it waits until the cap and then pays — bounded, as D5 asks.

use rusqlite::Connection;

use super::derived::{
    capability_state, catchup_needed, current_epoch, first_wait_ms, last_pull_started_at,
    remember_first_wait, request_incremental_pull, session_account, DerivedCapability,
};

/// D5: the most a unit may wait, per epoch, for the derived cache before it
/// pays anyway. Sized for the first boot of a new device, whose catch-up walks
/// the whole channel (~400 MiB in a library like the plan's §1.5), not for a
/// short incremental pull.
///
/// D5's second value, the freshness bound `F` (120 s), was dropped: freshness
/// is decided by the admission alone (see the module docs). The wait keeps this
/// one tunable.
pub const DERIVED_WAIT_MAX_MS: i64 = 15 * 60 * 1000;

/// How long a deferred unit sleeps before asking again while the engine has not
/// answered yet. One engine tick (the interval ticker fires every 60 s), so a
/// unit re-checks roughly when the pull it asked for lands.
pub const DERIVED_WAIT_RECHECK_MS: i64 = 60 * 1000;

/// The durable code a parked unit carries in `processing_tasks.last_error_code`
/// and in the code of its interrupted attempt. It is what the status bar counts
/// as "waiting for the other devices" and what keeps these near-zero attempts
/// out of the ETA average they would otherwise flatten.
pub const DERIVED_WAIT_CODE: &str = "derived_cache_wait";

/// What the gate allows a unit to do with its misses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateVerdict {
    /// Pay for the misses that survived the cache read.
    Proceed(ProceedReason),
    /// Park the unit and re-run it at `resume_at_ms` (never later than the
    /// epoch's D5 deadline).
    Defer {
        resume_at_ms: i64,
        reason: DeferReason,
    },
}

/// Why the gate let the unit pay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProceedReason {
    /// No session: the cache was read with the archive's bound account (§2.9)
    /// and no pull can arrive.
    NoSession,
    /// The server confirmed it does not implement the channel for this epoch.
    Unsupported,
    /// The last pull is later than the admission and younger than `F`.
    Fresh,
    /// This wait episode's bounded wait is over (D5): pay as before.
    WaitCapReached,
    /// A read the answer depends on failed. Fails open: paying once is better
    /// than withholding work the owner asked for.
    Unreadable,
}

/// Why the gate parked the unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferReason {
    /// A session exists and this epoch has no capability verdict yet.
    CapabilityUnknown,
    /// The channel is supported and this epoch's catch-up has not finished.
    CatchupPending,
    /// Caught up, but the cache is not fresh for this unit's admission: one
    /// incremental pull was requested.
    StalePull,
}

impl ProceedReason {
    /// The line the executors log, one short phrase per state.
    pub fn as_str(self) -> &'static str {
        match self {
            ProceedReason::NoSession => "no session",
            ProceedReason::Unsupported => "unsupported",
            ProceedReason::Fresh => "fresh",
            ProceedReason::WaitCapReached => "wait cap reached",
            ProceedReason::Unreadable => "unreadable",
        }
    }
}

impl DeferReason {
    /// The line the executors log, one short phrase per state.
    pub fn as_str(self) -> &'static str {
        match self {
            DeferReason::CapabilityUnknown => "capability unknown",
            DeferReason::CatchupPending => "catch-up pending",
            DeferReason::StalePull => "stale pull",
        }
    }
}

/// The admission instant of one unit: `processing_tasks.created_at`, the
/// moment the work entered the queue (right after the Zotero synchronisation
/// that created it). `None` when the row cannot be read, which pays.
fn admission_ms(conn: &Connection, task_id: &str) -> Option<i64> {
    conn.query_row(
        "SELECT created_at FROM processing_tasks WHERE id = ?1",
        [task_id],
        |row| row.get::<_, i64>(0),
    )
    .ok()
}

/// Whether the current epoch's cache is fresh for a unit admitted at
/// `admission`: the last successful incremental pull of the epoch started after
/// that instant, so it covers everything the catalogue that admitted the unit
/// could have seen. How long ago that pull ran does not matter — the unit's
/// admission, not the clock, is the boundary (no age bound, see the module
/// docs).
fn cache_is_fresh(conn: &Connection, admission: i64) -> bool {
    last_pull_started_at(conn).is_some_and(|started| started > admission)
}

/// The gate itself: what may this unit do right now, at `now_ms`?
///
/// `now_ms` is a parameter, not a clock read, so the bounded wait is testable
/// without sleeping (the executors pass
/// [`crate::processing::repository::now_ms`]).
pub fn evaluate(conn: &Connection, task_id: &str, now_ms: i64) -> GateVerdict {
    // No session: the cache was read with the bound account and nothing can
    // arrive by itself (§2.6 first row, S8).
    match session_account(conn) {
        Ok(None) => return GateVerdict::Proceed(ProceedReason::NoSession),
        Ok(Some(_)) => {}
        // Unreadable session metadata: pay, never withhold the owner's work.
        Err(_) => return GateVerdict::Proceed(ProceedReason::Unreadable),
    }
    match capability_state(conn) {
        DerivedCapability::Unsupported => return GateVerdict::Proceed(ProceedReason::Unsupported),
        // No verdict for this epoch yet: wait bounded by the epoch's first boot.
        DerivedCapability::Unknown => {
            return defer_for_epoch(conn, now_ms, DeferReason::CapabilityUnknown)
        }
        DerivedCapability::Supported => {}
    }
    let epoch = match current_epoch(conn) {
        Ok(Some(epoch)) => epoch,
        _ => return GateVerdict::Proceed(ProceedReason::Unreadable),
    };
    match catchup_needed(conn, &epoch) {
        // Caught up: the freshness barrier decides, and ITS wait is per unit.
        Ok(false) => {
            let Some(admission) = admission_ms(conn, task_id) else {
                return GateVerdict::Proceed(ProceedReason::Unreadable);
            };
            if cache_is_fresh(conn, admission) {
                return GateVerdict::Proceed(ProceedReason::Fresh);
            }
            // Not fresh for this unit: one coalesced incremental pull, then wait
            // for it — bounded per unit, so work admitted long after the epoch's
            // first boot still gets its own pull (D5's cap is per wait episode,
            // not per epoch: one epoch can hold minutes of first boot and hours
            // of backlog).
            if request_incremental_pull(conn).is_err() {
                return GateVerdict::Proceed(ProceedReason::Unreadable);
            }
            defer_until(now_ms, admission, DeferReason::StalePull)
        }
        // The since-zero catch-up is still running: wait bounded by first boot.
        Ok(true) => defer_for_epoch(conn, now_ms, DeferReason::CatchupPending),
        Err(_) => GateVerdict::Proceed(ProceedReason::Unreadable),
    }
}

/// The first-boot wait: D5's cap anchored to the epoch's FIRST wait, because the
/// thing being waited for (the capability verdict and the since-zero catch-up)
/// is a property of the epoch, and the whole device shares it.
fn defer_for_epoch(conn: &Connection, now_ms: i64, reason: DeferReason) -> GateVerdict {
    let read = || -> Result<GateVerdict, String> {
        let epoch = current_epoch(conn)?.unwrap_or_default();
        let first_wait = match first_wait_ms(conn, &epoch) {
            Some(first) => first,
            None => {
                remember_first_wait(conn, &epoch, now_ms)?;
                now_ms
            }
        };
        Ok(defer_until(now_ms, first_wait, reason))
    };
    match read() {
        Ok(verdict) => verdict,
        // The wait memory cannot be read: pay instead of guessing a deadline.
        Err(_) => GateVerdict::Proceed(ProceedReason::Unreadable),
    }
}

/// D5's cap for one wait episode: `anchor + DERIVED_WAIT_MAX_MS`, where `anchor`
/// starts the episode. Past it the unit pays instead of waiting.
fn defer_until(now_ms: i64, anchor: i64, reason: DeferReason) -> GateVerdict {
    let deadline = anchor.saturating_add(DERIVED_WAIT_MAX_MS);
    if now_ms >= deadline {
        return GateVerdict::Proceed(ProceedReason::WaitCapReached);
    }
    GateVerdict::Defer {
        resume_at_ms: now_ms.saturating_add(DERIVED_WAIT_RECHECK_MS).min(deadline),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::derived::{
        clear_account_metadata, pull_requested, record_capability, record_catchup_done,
        record_pull_started_at,
    };
    use crate::sync::session::{meta_get, meta_set};
    use crate::sync::test_support::new_synced_test_db;

    const EPOCH: &str = "epoch-1";
    const ACCOUNT: &str = "account-a";
    const TASK: &str = "task-1";

    /// One logged-in archive with one admitted unit. The unit's admission time
    /// is the only thing the freshness barrier needs from it.
    fn archive(admitted_at: i64) -> Connection {
        let conn = new_synced_test_db();
        meta_set(&conn, "account_id", ACCOUNT).expect("session");
        admit(&conn, admitted_at);
        conn
    }

    fn admit(conn: &Connection, created_at: i64) {
        admit_as(conn, TASK, created_at);
    }

    fn admit_as(conn: &Connection, task_id: &str, created_at: i64) {
        conn.execute(
            "INSERT INTO processing_tasks
               (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, created_at, updated_at)
             VALUES (?1, 'bibliography_extract', ?1, 'bibliography', 'attachment', ?1, 'running', ?2, ?2)",
            rusqlite::params![task_id, created_at],
        )
        .expect("admit task");
    }

    fn epoch(conn: &Connection) {
        meta_set(conn, "server_epoch", EPOCH).expect("epoch");
    }

    /// The device reached the state §2.6 calls "supported and caught up": the
    /// epoch is known, the token was advertised and the catch-up finished.
    fn supported_and_caught_up(conn: &Connection) {
        epoch(conn);
        record_capability(conn, EPOCH, true).expect("capability");
        record_catchup_done(conn, EPOCH).expect("catch-up");
    }

    fn deferral(verdict: GateVerdict) -> (i64, DeferReason) {
        match verdict {
            GateVerdict::Defer {
                resume_at_ms,
                reason,
            } => (resume_at_ms, reason),
            other => panic!("expected a deferral, got {other:?}"),
        }
    }

    fn proceed(verdict: GateVerdict) -> ProceedReason {
        match verdict {
            GateVerdict::Proceed(reason) => reason,
            other => panic!("expected a proceed, got {other:?}"),
        }
    }

    /// §2.6 / S5 first boot of B: a session with no capability verdict yet
    /// waits, bounded by D5, and then pays. The bound is per epoch and is
    /// remembered, so the last unit of the epoch pays as soon as the deadline
    /// passes instead of starting a new wait.
    #[test]
    fn an_unknown_capability_with_a_session_waits_until_the_cap_and_then_pays() {
        let conn = archive(1_000);
        assert_eq!(
            deferral(evaluate(&conn, TASK, 1_000)),
            (
                1_000 + DERIVED_WAIT_RECHECK_MS,
                DeferReason::CapabilityUnknown
            )
        );
        // A re-check inside the wait keeps the same epoch deadline: it does not
        // start a new 15-minute budget.
        assert_eq!(
            deferral(evaluate(&conn, TASK, 1_000 + DERIVED_WAIT_RECHECK_MS)),
            (
                1_000 + 2 * DERIVED_WAIT_RECHECK_MS,
                DeferReason::CapabilityUnknown
            )
        );
        assert_eq!(
            deferral(evaluate(
                &conn,
                TASK,
                1_000 + DERIVED_WAIT_MAX_MS - DERIVED_WAIT_RECHECK_MS / 2
            ))
            .0,
            1_000 + DERIVED_WAIT_MAX_MS,
            "the last re-check sleeps exactly to the deadline"
        );
        assert_eq!(
            proceed(evaluate(&conn, TASK, 1_000 + DERIVED_WAIT_MAX_MS)),
            ProceedReason::WaitCapReached,
            "past D5 the unit pays instead of waiting"
        );
        assert_eq!(
            proceed(evaluate(&conn, TASK, 1_000 + DERIVED_WAIT_MAX_MS + 1)),
            ProceedReason::WaitCapReached,
            "the verdict is remembered for the rest of the epoch"
        );
    }

    /// S8 and §2.6 first row: no session never waits — the cache was already
    /// read with the archive's bound account, and no pull can arrive.
    #[test]
    fn without_a_session_the_gate_never_waits() {
        let conn = new_synced_test_db();
        admit(&conn, 1_000);
        assert_eq!(
            proceed(evaluate(&conn, TASK, 1_000)),
            ProceedReason::NoSession
        );
        assert_eq!(
            meta_get(&conn, "derived_wait_verdict").expect("wait memory"),
            None,
            "no wait is even recorded"
        );
        assert!(!pull_requested(&conn).expect("pull flag"));
    }

    /// §2.6: a confirmed absence (this epoch advertised `false`) pays as today.
    #[test]
    fn a_confirmed_absence_pays_without_waiting() {
        let conn = archive(1_000);
        epoch(&conn);
        record_capability(&conn, EPOCH, false).expect("legacy verdict");
        assert_eq!(
            proceed(evaluate(&conn, TASK, 1_000)),
            ProceedReason::Unsupported
        );
        assert_eq!(
            meta_get(&conn, "derived_wait_verdict").expect("wait memory"),
            None
        );
        assert!(!pull_requested(&conn).expect("pull flag"));
    }

    /// §2.6: supported with the catch-up still pending waits; the moment the
    /// catch-up lands and the pull is fresh for the admission, the unit pays.
    #[test]
    fn a_supported_epoch_waits_for_its_catchup_and_then_for_freshness() {
        let conn = archive(1_000);
        supported_and_caught_up(&conn);
        record_catchup_done(&conn, EPOCH).expect("catch-up");
        // Undo the catch-up: the state the row describes is "pending".
        conn.execute(
            "DELETE FROM sync_meta WHERE key = 'derived_catchup_epoch'",
            [],
        )
        .expect("forget catch-up");
        assert_eq!(
            deferral(evaluate(&conn, TASK, 1_000)).1,
            DeferReason::CatchupPending
        );
        // Caught up, but the last pull started BEFORE the unit was admitted:
        // the cache may be missing what another device uploaded meanwhile.
        supported_and_caught_up(&conn);
        record_pull_started_at(&conn, EPOCH, 900).expect("pull");
        assert_eq!(
            deferral(evaluate(&conn, TASK, 1_000)).1,
            DeferReason::StalePull,
            "a pull older than the admission does not prove freshness"
        );
        assert!(
            pull_requested(&conn).expect("pull flag"),
            "the gate asks the engine for one incremental pull"
        );
    }

    /// §2.6 freshness barrier: a pull that started after the admission is fresh
    /// for that unit and is never repeated, however long ago it ran — the unit
    /// just pays its misses. There is no age bound (D5's `F` was dropped).
    #[test]
    fn a_pull_after_the_admission_is_fresh_and_never_repeated() {
        let conn = archive(1_000);
        supported_and_caught_up(&conn);
        record_pull_started_at(&conn, EPOCH, 1_500).expect("pull");
        for now in [
            1_500,
            1_500 + 1,
            1_500 + 120_000,
            1_500 + 6 * 60 * 60 * 1000,
        ] {
            assert_eq!(
                proceed(evaluate(&conn, TASK, now)),
                ProceedReason::Fresh,
                "a pull after the admission is fresh at {now}"
            );
            assert!(
                !pull_requested(&conn).expect("pull flag"),
                "a fresh cache is never pulled again for this unit"
            );
        }
    }

    /// §2.6 / S5: a Zotero synchronisation admits the whole backlog at once and
    /// the device then processes it for hours. ONE pull after that admission
    /// keeps every unit fresh for the rest of the backlog — no unit asks for a
    /// pull again merely because time passed. Only work admitted AFTER the pull
    /// (a later catalogue) waits for the next one.
    #[test]
    fn a_backlog_admitted_at_once_is_not_throttled_by_the_pull_s_age() {
        let conn = archive(1_000);
        supported_and_caught_up(&conn);
        let admitted_at = 1_000;
        for index in 0..1_050 {
            admit_as(&conn, &format!("backlog-{index}"), admitted_at);
        }
        // The engine answers the gate's first request: one pull, at 2_000.
        record_pull_started_at(&conn, EPOCH, 2_000).expect("pull");

        // Hours of processing later every unit of the wave is still fresh.
        for index in [0_usize, 1, 500, 1_049] {
            let now = 2_000 + 4 * 60 * 60 * 1000 + index as i64;
            assert_eq!(
                proceed(evaluate(&conn, &format!("backlog-{index}"), now)),
                ProceedReason::Fresh,
                "unit {index} must not wait again four hours after the pull"
            );
            assert!(
                !pull_requested(&conn).expect("pull flag"),
                "no unit of the wave asks for another pull"
            );
        }

        // A later catalogue admission is a different question: its work may
        // postdate the pull, so it waits — within its OWN budget, counted from
        // its admission — for a pull that started after it.
        admit_as(&conn, "task-late", 3_000);
        assert_eq!(
            deferral(evaluate(&conn, "task-late", 3_001)).1,
            DeferReason::StalePull
        );
        assert!(pull_requested(&conn).expect("pull flag"));
        // Four hours later that unit's own wait is long over and it pays.
        assert_eq!(
            proceed(evaluate(&conn, "task-late", 4 * 60 * 60 * 1000)),
            ProceedReason::WaitCapReached
        );
    }

    /// D1 (verifier): the cap is per wait episode, not per epoch. A stale unit
    /// admitted two hours after the epoch's first boot — long past that boot's
    /// deadline — still waits for ITS pull, requests it, and is not handed
    /// `WaitCapReached` on the epoch's account.
    #[test]
    fn a_stale_cache_waits_again_for_work_admitted_after_the_epochs_first_wait() {
        let conn = archive(1_000);
        supported_and_caught_up(&conn);
        record_pull_started_at(&conn, EPOCH, 900).expect("pull older than the admission");
        // The epoch's first boot: its own wait starts now and expires at 901_000.
        assert_eq!(
            deferral(evaluate(&conn, TASK, 1_000)).1,
            DeferReason::StalePull
        );
        let later_admission = 1_000 + 2 * 60 * 60 * 1000;
        admit_as(&conn, "task-hours-later", later_admission);
        assert!(
            later_admission > 1_000 + DERIVED_WAIT_MAX_MS,
            "the epoch's first deadline is long gone"
        );

        let (resume, reason) = deferral(evaluate(&conn, "task-hours-later", later_admission));
        assert_eq!(
            reason,
            DeferReason::StalePull,
            "work admitted hours later still waits for its own pull"
        );
        assert_eq!(
            resume,
            later_admission + DERIVED_WAIT_RECHECK_MS,
            "the wait is anchored at this unit's admission"
        );
        assert!(
            pull_requested(&conn).expect("pull flag"),
            "and it asked the engine for one"
        );
    }

    /// D1: one stale unit pays after its OWN bounded wait — its budget starts at
    /// its admission, and the last re-check sleeps exactly to that deadline.
    #[test]
    fn a_stale_unit_proceeds_after_its_own_bounded_wait() {
        let conn = archive(1_000);
        supported_and_caught_up(&conn);
        record_pull_started_at(&conn, EPOCH, 900).expect("pull older than the admission");
        assert_eq!(
            deferral(evaluate(&conn, TASK, 1_000)).1,
            DeferReason::StalePull
        );
        let deadline = 1_000 + DERIVED_WAIT_MAX_MS;
        let (resume, reason) = deferral(evaluate(
            &conn,
            TASK,
            deadline - DERIVED_WAIT_RECHECK_MS / 2,
        ));
        assert_eq!(reason, DeferReason::StalePull);
        assert_eq!(resume, deadline, "the last re-check sleeps to the deadline");
        assert_eq!(
            proceed(evaluate(&conn, TASK, deadline)),
            ProceedReason::WaitCapReached,
            "after its own wait the unit pays"
        );
        // A pull that lands in the meantime still makes it fresh instead.
        assert_eq!(
            proceed(evaluate(&conn, TASK, deadline + 1)),
            ProceedReason::WaitCapReached,
            "the unit does not get a second wait for the same admission"
        );
    }

    /// D5's memory is per epoch: a rotated epoch starts its own bounded wait,
    /// and logout drops the memory with the rest of the channel metadata.
    #[test]
    fn the_wait_memory_belongs_to_its_epoch_and_logout_drops_it() {
        let conn = archive(1_000);
        deferral(evaluate(&conn, TASK, 1_000));
        assert_eq!(
            proceed(evaluate(&conn, TASK, 1_000 + DERIVED_WAIT_MAX_MS)),
            ProceedReason::WaitCapReached
        );
        // A new epoch is a new catch-up and a new wait: nothing carried over.
        meta_set(&conn, "server_epoch", "epoch-2").expect("rotation");
        assert_eq!(
            deferral(evaluate(&conn, TASK, 1_000 + DERIVED_WAIT_MAX_MS + 1)).0,
            1_000 + DERIVED_WAIT_MAX_MS + 1 + DERIVED_WAIT_RECHECK_MS
        );
        // Logout clears the channel metadata (derived::clear_account_metadata),
        // so the next login starts a fresh bounded wait instead of inheriting
        // the previous account's deadline.
        clear_account_metadata(&conn).expect("logout");
        assert_eq!(
            meta_get(&conn, "derived_wait_verdict").expect("wait memory"),
            None
        );
    }

    /// A unit whose row cannot be read pays: the gate never withholds work it
    /// cannot reason about.
    #[test]
    fn a_unit_without_an_admission_time_pays() {
        let conn = archive(1_000);
        supported_and_caught_up(&conn);
        record_pull_started_at(&conn, EPOCH, 1_500).expect("pull");
        assert_eq!(
            proceed(evaluate(&conn, "task-missing", 1_500)),
            ProceedReason::Unreadable
        );
    }

    /// Several units of one epoch share one deadline and one pull request
    /// (the transport coalesces the flag into a single incremental pull).
    #[test]
    fn several_units_share_one_deadline_and_one_pull_request() {
        let conn = archive(1_000);
        supported_and_caught_up(&conn);
        record_pull_started_at(&conn, EPOCH, 900).expect("pull");
        assert_eq!(
            deferral(evaluate(&conn, TASK, 1_000)).0,
            1_000 + DERIVED_WAIT_RECHECK_MS
        );
        admit_as(&conn, "task-2", 1_100);
        assert_eq!(
            deferral(evaluate(&conn, "task-2", 1_010)).0,
            1_010 + DERIVED_WAIT_RECHECK_MS,
            "a later unit does not start a new budget"
        );
        assert!(pull_requested(&conn).expect("pull flag"));
    }
}
