//! Engine tests (DESIGN §3.1, §11, PROTOCOL flow): the gate, single-flight
//! coalescing, fresh schema_tag reads, the state-machine error mapping, the
//! status payload shape, and an end-to-end cycle against the in-memory mock.
//!
//! These exercise the engine's PURE pieces and the `run_cycle` orchestration
//! without an `AppHandle` (the cycle takes a logging sink, not a handle) and
//! without the OS keyring (the token is a plain argument), so they run in CI with
//! no server and no credential store.

use super::*;
use crate::sync::capture::ensure_capture;
use crate::sync::http::{PullResponse, PullRow};
use crate::sync::session::{meta_get_i64, meta_set};
use crate::sync::test_support::{new_synced_test_db, MockSyncApi};

use rusqlite::Connection;
use serde_json::json;

const TEST_ACCOUNT: &str = "mock-account";

/// A captured, logged-in session DB ready for the engine: triggers installed,
/// capture on, session meta populated. Capture/session opens the engine gate.
fn engine_session_db() -> Connection {
    let conn = new_synced_test_db();
    ensure_capture(&conn).expect("ensure capture");
    crate::sync::test_support::set_session_with_capture(&conn);
    meta_set(&conn, "account_id", TEST_ACCOUNT).unwrap();
    meta_set(&conn, "server_url", "https://sync.test").unwrap();
    // Pretend a previous account was already seeded so the cycle does not run the
    // up-front full pull (tests that want seeding clear this).
    meta_set(&conn, "seeded_account", TEST_ACCOUNT).unwrap();
    conn
}

fn tmp_app_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("assets")).expect("assets dir");
    dir
}

/// A no-op warn sink for the cycle.
fn no_warn() -> impl Fn(String) + Sync {
    |_| {}
}

fn seed_collection(conn: &Connection) {
    conn.execute(
        "INSERT INTO collections(id,name,created_at,updated_at) VALUES('c1','C',1,1)",
        [],
    )
    .expect("seed collection");
    conn.execute_batch("DELETE FROM sync_oplog;")
        .expect("clear oplog after seed");
}

// --------------------------------------------------------------------------
// Gate (DESIGN §3.1)
// --------------------------------------------------------------------------

#[test]
fn gate_closed_without_capture_or_session() {
    // Fresh sync schema, NO triggers_version and NO device_id → gate closed.
    let conn = new_synced_test_db();
    assert!(
        !is_gated_open(&conn),
        "gate must be closed before capture/session"
    );

    // Capture only (triggers_version set by ensure_capture) but no session.
    ensure_capture(&conn).expect("ensure capture");
    assert!(
        !is_gated_open(&conn),
        "capture alone must not open the gate (no session)"
    );

    // Session only, no capture.
    let conn2 = new_synced_test_db();
    meta_set(&conn2, "device_id", "dev-1").unwrap();
    assert!(
        !is_gated_open(&conn2),
        "session alone must not open the gate (no capture)"
    );
}

#[test]
fn gate_open_with_capture_and_session() {
    let conn = engine_session_db();
    assert!(is_gated_open(&conn), "capture + session opens the gate");
}

#[tokio::test]
async fn no_cycle_runs_before_gate_opens() {
    // A run_cycle against a DB with NO session must fail fast (no account_id) and
    // never push — proving the engine has nothing to do before the gate opens.
    // (The engine loop itself short-circuits on the gate; here we assert the cycle
    // refuses to proceed without a configured session.)
    let conn = new_synced_test_db();
    ensure_capture(&conn).unwrap();
    assert!(!is_gated_open(&conn));

    let api = MockSyncApi::default();
    let dir = tmp_app_dir();
    let warn = no_warn();
    let result = run_cycle(&api, "tok", &conn, dir.path(), &warn).await;
    assert!(result.is_err(), "cycle must not proceed without a session");
    assert_eq!(
        api.pushed_count(),
        0,
        "nothing pushed before the gate opens"
    );
}

// --------------------------------------------------------------------------
// Fresh schema_tag (DESIGN §3.1)
// --------------------------------------------------------------------------

#[test]
fn read_schema_tag_returns_empty_when_no_migrations() {
    // The fixture concatenates DDL but inserts no _migrations rows.
    let conn = engine_session_db();
    assert_eq!(read_schema_tag(&conn).unwrap(), "");
}

#[test]
fn read_schema_tag_tracks_migrations_head_freshly() {
    let conn = engine_session_db();
    conn.execute(
        "INSERT INTO _migrations(name, applied_at) VALUES('0022_topics', 1)",
        [],
    )
    .unwrap();
    assert_eq!(read_schema_tag(&conn).unwrap(), "0022_topics");

    // Simulate a migration applied while the engine runs: the head moves and a
    // FRESH read reflects it (the engine reads per cycle, never caches).
    conn.execute(
        "INSERT INTO _migrations(name, applied_at) VALUES('0023_sync_ids', 2)",
        [],
    )
    .unwrap();
    assert_eq!(
        read_schema_tag(&conn).unwrap(),
        "0023_sync_ids",
        "schema_tag must reflect the new head on a fresh read"
    );
}

// --------------------------------------------------------------------------
// State-machine error mapping (DESIGN §11)
// --------------------------------------------------------------------------

#[test]
fn classify_network_error_is_offline() {
    match classify_error(SyncError::Network("dns".into())) {
        CycleError::Offline(_) => {}
        other => panic!("network error must map to Offline, got {other:?}"),
    }
}

#[test]
fn classify_426_507_clock_skew_are_fatal_with_specific_messages() {
    let m426 = match classify_error(SyncError::Api {
        status: 426,
        code: "schema_upgrade_required".into(),
        message: "x".into(),
    }) {
        CycleError::Fatal { message } => message,
        other => panic!("426 must be Fatal, got {other:?}"),
    };
    assert!(m426.contains("Actualizá"), "426 → update message: {m426}");

    let m507 = match classify_error(SyncError::Api {
        status: 507,
        code: "insufficient_storage".into(),
        message: "x".into(),
    }) {
        CycleError::Fatal { message } => message,
        other => panic!("507 must be Fatal, got {other:?}"),
    };
    assert!(m507.contains("lleno"), "507 → storage full message: {m507}");

    let skew = match classify_error(SyncError::Api {
        status: 400,
        code: "clock_skew".into(),
        message: "x".into(),
    }) {
        CycleError::Fatal { message } => message,
        other => panic!("clock_skew must be Fatal, got {other:?}"),
    };
    assert!(skew.contains("reloj"), "clock_skew → clock message: {skew}");
}

#[test]
fn classify_account_suspended_and_subscription_expired_are_fatal_with_specific_messages() {
    let suspended = match classify_error(SyncError::Api {
        status: 403,
        code: "account_suspended".into(),
        message: "suspended".into(),
    }) {
        CycleError::Fatal { message } => message,
        other => panic!("account_suspended must be Fatal, got {other:?}"),
    };
    assert!(
        suspended.contains("suspendida"),
        "account_suspended → suspended message: {suspended}"
    );

    let expired = match classify_error(SyncError::Api {
        status: 403,
        code: "subscription_expired".into(),
        message: "expired".into(),
    }) {
        CycleError::Fatal { message } => message,
        other => panic!("subscription_expired must be Fatal, got {other:?}"),
    };
    assert!(
        expired.contains("venció"),
        "subscription_expired → expired message: {expired}"
    );
}

// --------------------------------------------------------------------------
// Status payload shape (DESIGN §11)
// --------------------------------------------------------------------------

#[test]
fn build_status_reports_pending_blobs_and_conflicts() {
    let conn = engine_session_db();
    seed_collection(&conn);
    // Two dirty rows for the same id coalesce to ONE pending; a third distinct id
    // adds another → pending = 2.
    conn.execute_batch(
        "INSERT INTO sync_oplog(table_name,row_id,op,changed_at) VALUES('items','i1','I',1);
         INSERT INTO sync_oplog(table_name,row_id,op,changed_at) VALUES('items','i1','U',2);
         INSERT INTO sync_oplog(table_name,row_id,op,changed_at) VALUES('items','i2','I',3);
         INSERT INTO sync_pending_blobs(asset_id,sha256,rel_path,size) VALUES('a1','h','assets/x',1);
         INSERT INTO sync_conflicts(id,table_name,row_id,reason,created_at,acknowledged)
           VALUES('cf1','items','i1','lww_lost',1,0);
         INSERT INTO sync_conflicts(id,table_name,row_id,reason,created_at,acknowledged)
           VALUES('cf2','items','i2','lww_lost',1,1);
         -- Two own blobs not yet uploaded (100 + 200) and one already uploaded (999):
         -- the preflight estimate must sum only the un-uploaded sizes.
         INSERT INTO sync_blob_index(asset_id,sha256,size,file_mtime_ms,uploaded)
           VALUES('b1','h1',100,1,0);
         INSERT INTO sync_blob_index(asset_id,sha256,size,file_mtime_ms,uploaded)
           VALUES('b2','h2',200,1,0);
         INSERT INTO sync_blob_index(asset_id,sha256,size,file_mtime_ms,uploaded)
           VALUES('b3','h3',999,1,1);",
    )
    .unwrap();
    meta_set(&conn, "last_sync_at", "1700000000000").unwrap();

    let status = build_status(&conn, SyncState::Idle, None);
    assert_eq!(status.state, SyncState::Idle);
    assert_eq!(status.pending, 2, "coalesced distinct (table,row) count");
    assert_eq!(status.blobs_pending, 1);
    assert_eq!(
        status.pending_blob_bytes, 300,
        "only un-uploaded blob sizes (100 + 200) feed the preflight estimate"
    );
    assert_eq!(status.conflicts, 1, "only unacknowledged conflicts counted");
    assert_eq!(status.last_sync_at, Some(1_700_000_000_000));
    assert!(!status.clock_warning, "no offset stored → no warning");

    // Serialized payload shape: lowercase state, omitted message when None.
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["state"], "idle");
    assert!(json.get("message").is_none(), "message omitted when None");
    assert!(json.get("blobs_pending").is_some());
    assert!(json.get("pending_blob_bytes").is_some());
}

#[test]
fn build_status_flags_clock_warning_over_5_min() {
    let conn = engine_session_db();
    meta_set(&conn, "clock_offset_ms", &(6 * 60 * 1000).to_string()).unwrap();
    let status = build_status(&conn, SyncState::Idle, None);
    assert!(
        status.clock_warning,
        "offset > 5 min raises the clock warning"
    );
}

#[test]
fn disabled_status_serializes_lowercase() {
    let json = serde_json::to_value(SyncStatus::disabled()).unwrap();
    assert_eq!(json["state"], "disabled");
}

// --------------------------------------------------------------------------
// Single-flight coalescing (DESIGN §3.1)
// --------------------------------------------------------------------------

#[tokio::test]
async fn two_concurrent_sync_now_coalesce_to_one_pending() {
    // The capacity-1 channel models the single-flight contract: with a run
    // "in flight" (the receiver not yet draining), a burst of SyncNow leaves at
    // most ONE queued — the rest are dropped as redundant.
    let (sender, mut receiver) = mpsc::channel::<SyncRequest>(1);
    let engine = SyncEngine {
        sender: sender.clone(),
        status: Arc::new(Mutex::new(SyncStatus::disabled())),
    };

    // Fire several requests back to back without draining.
    for _ in 0..5 {
        engine.request(SyncRequest::SyncNow);
    }

    // Exactly one is queued (capacity 1); the rest coalesced away.
    let mut received = 0;
    while receiver.try_recv().is_ok() {
        received += 1;
    }
    assert_eq!(
        received, 1,
        "a burst of SyncNow coalesces to one pending run"
    );
}

#[tokio::test]
async fn request_on_closed_channel_is_a_noop() {
    let (sender, receiver) = mpsc::channel::<SyncRequest>(1);
    let engine = SyncEngine {
        sender,
        status: Arc::new(Mutex::new(SyncStatus::disabled())),
    };
    drop(receiver); // Engine task gone.
                    // Must not panic.
    engine.request(SyncRequest::SyncNow);
    engine.shutdown(); // Also a no-op on a closed channel.
}

#[tokio::test]
async fn shutdown_enqueues_a_shutdown_request() {
    let (sender, mut receiver) = mpsc::channel::<SyncRequest>(1);
    let engine = SyncEngine {
        sender,
        status: Arc::new(Mutex::new(SyncStatus::disabled())),
    };
    engine.shutdown();
    assert!(
        matches!(receiver.try_recv(), Ok(SyncRequest::Shutdown)),
        "shutdown() must enqueue a Shutdown request"
    );
}

// --------------------------------------------------------------------------
// End-to-end cycle against the mock (PROTOCOL flow)
// --------------------------------------------------------------------------

#[tokio::test]
async fn cycle_pushes_dirty_rows_and_advances_versions() {
    let conn = engine_session_db();
    seed_collection(&conn);
    // A dirty item to push.
    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i1','A','c1',1,1)",
        [],
    )
    .unwrap();
    assert!(snapshot_oplog(&conn).unwrap() > 0, "row is dirty");

    let api = MockSyncApi::default();
    let dir = tmp_app_dir();
    let warn = no_warn();
    run_cycle(&api, "tok", &conn, dir.path(), &warn)
        .await
        .expect("cycle ok");

    // The item was pushed and its oplog purged; a row version recorded.
    assert!(api.pushed_count() >= 1, "dirty row pushed");
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM sync_oplog", [], |r| r.get(0))
        .unwrap();
    assert_eq!(remaining, 0, "pushed oplog purged after a successful cycle");
    let versioned: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_row_versions WHERE table_name='items' AND row_id='i1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(versioned, 1, "row version recorded from the push result");

    // last_sync_at stamped.
    assert!(
        meta_get_i64(&conn, "last_sync_at").unwrap() > 0,
        "last_sync_at set"
    );
}

#[tokio::test]
async fn cycle_applies_pulled_rows() {
    let conn = engine_session_db();
    // No local dirty rows; a remote collection arrives via a pull page.
    let api = MockSyncApi::default();
    api.queue_pull_page(PullResponse {
        rows: vec![PullRow {
            table: "collections".to_string(),
            row_id: "cr1".to_string(),
            server_seq: 10,
            deleted: false,
            changed_at: 1,
            device_id: "remote".to_string(),
            payload: Some(json!({"id":"cr1","name":"Remote","created_at":1,"updated_at":1})),
        }],
        next_since: 10,
        has_more: false,
        schema_tag: String::new(),
        server_epoch: "mock-epoch".to_string(),
        server_now_ms: 1_700_000_000_000,
    });

    let dir = tmp_app_dir();
    let warn = no_warn();
    run_cycle(&api, "tok", &conn, dir.path(), &warn)
        .await
        .expect("cycle ok");

    let applied: i64 = conn
        .query_row("SELECT COUNT(*) FROM collections WHERE id='cr1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(applied, 1, "remote collection applied locally");
    assert_eq!(
        meta_get_i64(&conn, "last_pull_seq").unwrap(),
        10,
        "cursor advanced"
    );
}

#[tokio::test]
async fn cycle_runs_seeding_for_a_new_account() {
    let conn = engine_session_db();
    // Force the "new account" path: no seeded_account yet.
    conn.execute("DELETE FROM sync_meta WHERE key='seeded_account'", [])
        .unwrap();
    // Pre-existing local row the server has never seen → must be seeded.
    seed_collection(&conn);
    conn.execute(
        "INSERT INTO items(id,title,collection_id,created_at,updated_at) VALUES('i9','Old','c1',1,1)",
        [],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM sync_oplog;").unwrap(); // simulate pre-trigger rows

    let api = MockSyncApi::default();
    let dir = tmp_app_dir();
    let warn = no_warn();
    run_cycle(&api, "tok", &conn, dir.path(), &warn)
        .await
        .expect("cycle ok");

    // seeded_account marked, and the pre-existing rows got pushed (seeded → oplog
    // → push), so the server saw them.
    assert_eq!(
        meta_get(&conn, "seeded_account").unwrap().as_deref(),
        Some(TEST_ACCOUNT),
        "account marked seeded"
    );
    assert!(
        api.pushed_count() >= 1,
        "seeded pre-existing rows were pushed"
    );
}

#[tokio::test]
async fn network_error_in_cycle_surfaces_as_offline() {
    // A mock that fails health with a network error → the cycle must classify it
    // as Offline (backoff), not Fatal.
    struct OfflineApi;
    impl SyncApi for OfflineApi {
        async fn register(
            &self,
            _r: crate::sync::http::RegisterRequest,
        ) -> Result<crate::sync::http::RegisterResponse, SyncError> {
            unreachable!()
        }
        async fn login(
            &self,
            _r: crate::sync::http::LoginRequest,
        ) -> Result<crate::sync::http::LoginResponse, SyncError> {
            unreachable!()
        }
        async fn logout(&self, _t: &str) -> Result<(), SyncError> {
            unreachable!()
        }
        async fn devices(&self, _t: &str) -> Result<crate::sync::http::DevicesResponse, SyncError> {
            unreachable!()
        }
        async fn revoke(&self, _t: &str, _d: &str) -> Result<(), SyncError> {
            unreachable!()
        }
        async fn delete_account(
            &self,
            _t: &str,
            _r: crate::sync::http::DeleteAccountRequest,
        ) -> Result<(), SyncError> {
            unreachable!()
        }
        async fn usage(&self, _t: &str) -> Result<crate::sync::http::UsageResponse, SyncError> {
            unreachable!()
        }
        async fn list_plans(
            &self,
            _t: &str,
        ) -> Result<Vec<crate::sync::http::PlanCatalogItem>, SyncError> {
            unreachable!()
        }
        async fn request_plan_change(
            &self,
            _t: &str,
            _requested_plan_id: &str,
            _note: Option<&str>,
        ) -> Result<crate::sync::http::PlanChangeRequestResponse, SyncError> {
            unreachable!()
        }
        async fn list_notifications(
            &self,
            _t: &str,
            _since: Option<&str>,
            _limit: Option<i64>,
        ) -> Result<Vec<crate::sync::http::NotificationItem>, SyncError> {
            unreachable!()
        }
        async fn mark_notification_read(&self, _t: &str, _id: &str) -> Result<(), SyncError> {
            unreachable!()
        }
        async fn health(&self) -> Result<crate::sync::http::HealthResponse, SyncError> {
            Err(SyncError::Network("connection refused".into()))
        }
        async fn push(
            &self,
            _t: &str,
            _s: &str,
            _r: crate::sync::http::PushRequest,
        ) -> Result<crate::sync::http::PushResponse, SyncError> {
            unreachable!()
        }
        async fn pull(
            &self,
            _t: &str,
            _s: &str,
            _since: i64,
            _l: i64,
        ) -> Result<PullResponse, SyncError> {
            unreachable!()
        }
        async fn blob_head(&self, _t: &str, _s: &str) -> Result<bool, SyncError> {
            unreachable!()
        }
        async fn blob_put(&self, _t: &str, _s: &str, _b: Vec<u8>) -> Result<(), SyncError> {
            unreachable!()
        }
        async fn blob_get(&self, _t: &str, _s: &str) -> Result<reqwest::Response, SyncError> {
            unreachable!()
        }
    }

    let conn = engine_session_db();
    let dir = tmp_app_dir();
    let warn = no_warn();
    let result = run_cycle(&OfflineApi, "tok", &conn, dir.path(), &warn).await;
    match result {
        Err(CycleError::Offline(_)) => {}
        other => panic!("network failure must surface as Offline, got {other:?}"),
    }
}
