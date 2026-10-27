//! Two-device acceptance for the derived Bibliography cache channel
//! (`bibliography-derived-v1`, plan `odd/plans/plan-sync-bibliografia-derivada.md`
//! §2.5 option (b), WU4).
//!
//! Two independent devices, each with its OWN temp `app_data_dir` and its OWN
//! SQLite archive built from the real schema fixture, run the production
//! `sync::engine::run_cycle` against an in-memory `SyncApi` server (`tests/common`)
//! that implements the client-side wire contract of `src/sync/derived_protocol.md`:
//! capability advertisement on ordinary responses, per-account row channels with
//! their own sequences, idempotent insert, cursor pages and a `max_push_bytes`
//! budget. No network, no real archive, no server binary.
//!
//! The cache-hit assertions call the same readers the paid paths use
//! (`sync::derived::cached_vectors`, `cached_ocr_pages`) before their provider
//! call: a hit there is exactly "no paid request is needed" for that row (S3).

mod common;

use common::{vector_payload, MockServer, TestDevice};
use entropia_desktop_lib::sync::derived::{
    cached_ocr_pages, cached_vectors, last_pull_started_at, request_incremental_pull,
};
use serde_json::json;

const SERVER_URL: &str = "https://sync.example.test";
const ACCOUNT_A: &str = "account-a";
const ACCOUNT_B: &str = "account-b";
const CONTRACT: &str = "5f0c5f0c5f0c5f0c5f0c5f0c5f0c5f0c";
const OCR_CONTRACT: &str = "ocr-v1:glm:glm-ocr:p1";
const SOURCE_HASH: &str = "3b1e3b1e3b1e3b1e3b1e3b1e3b1e3b1e3b1e3b1e3b1e3b1e3b1e3b1e3b1e3b1e";
const DIMENSIONS: i64 = 8;

/// A's device token and B's device token, bound to the account each device is
/// logged into.
fn devices() -> (TestDevice, TestDevice) {
    (
        TestDevice::new("device-a", "token-a"),
        TestDevice::new("device-b", "token-b"),
    )
}

#[tokio::test]
async fn cached_results_of_a_reach_b_and_b_answers_from_them() {
    let server = MockServer::new();
    let (mut a, mut b) = devices();

    // A pays for one vector and two OCR pages (one empty) and syncs.
    a.login(&server, SERVER_URL, ACCOUNT_A);
    a.seed_paid_vector(CONTRACT, "sha256-of-the-profile", DIMENSIONS, 0x3f);
    a.seed_paid_ocr_page(SOURCE_HASH, 1, OCR_CONTRACT, "text", "page one, paid");
    a.seed_paid_ocr_page(SOURCE_HASH, 2, OCR_CONTRACT, "empty", "");
    a.sync(&server).await;

    assert_eq!(
        a.pushed_vectors(ACCOUNT_A),
        1,
        "A's paid vector reached the channel"
    );
    assert_eq!(
        a.pending_rows(ACCOUNT_A),
        0,
        "A acknowledged everything it owns"
    );
    assert_eq!(
        server.derived_keys(ACCOUNT_A).len(),
        4,
        "one contract, one vector, two OCR pages"
    );
    assert!(a
        .meta("derived_capability")
        .expect("capability recorded")
        .contains("\"advertised\":true"));

    // B boots with the same account: one ordinary response advertises the
    // capability, the epoch's catch-up pulls the whole channel, and the cache
    // answers exactly what A paid for — with no provider call in this path.
    b.login(&server, SERVER_URL, ACCOUNT_A);
    b.sync(&server).await;

    let vector = cached_vectors(
        &b.conn,
        CONTRACT,
        DIMENSIONS as usize,
        &["sha256-of-the-profile".to_string()],
    );
    assert_eq!(
        vector.get("sha256-of-the-profile"),
        Some(&vec![0x3fu8; (DIMENSIONS * 4) as usize]),
        "B answers the vector from the cache instead of paying for it"
    );
    let pages = cached_ocr_pages(&b.conn, SOURCE_HASH, OCR_CONTRACT, &[1, 2]);
    assert_eq!(
        pages.get(&1).map(|answer| answer.text.as_str()),
        Some("page one, paid")
    );
    assert_eq!(pages.get(&2).map(|answer| answer.text.as_str()), Some(""));
    assert_eq!(
        pages.get(&2).map(|answer| answer.outcome.as_str()),
        Some("empty"),
        "an empty paid answer travels too: the second device must not pay again"
    );

    let (account, origin, pushed): (String, String, i64) = b
        .conn
        .query_row(
            "SELECT account_id, origin, pushed FROM sync_derived_vectors",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("installed vector");
    assert_eq!(account, ACCOUNT_A);
    assert_eq!(origin, "remote");
    assert_eq!(pushed, 1, "a received row is already on the server");
    assert_eq!(
        server.derived_pushes.lock().unwrap().len(),
        1,
        "B had nothing to push: it only received"
    );

    // A third device behaves like B (S3: "un tercer dispositivo, C").
    let mut c = TestDevice::new("device-c", "token-c");
    c.login(&server, SERVER_URL, ACCOUNT_A);
    c.sync(&server).await;
    assert_eq!(
        cached_vectors(
            &c.conn,
            CONTRACT,
            DIMENSIONS as usize,
            &["sha256-of-the-profile".to_string()]
        )
        .get("sha256-of-the-profile"),
        Some(&vec![0x3fu8; (DIMENSIONS * 4) as usize])
    );
}

#[tokio::test]
async fn a_legacy_server_never_receives_a_derived_call_and_the_corpus_still_syncs() {
    let server = MockServer::legacy();
    let (mut a, mut b) = devices();

    a.login(&server, SERVER_URL, ACCOUNT_A);
    a.seed_collection("collection-1");
    a.seed_paid_vector(CONTRACT, "sha256-of-the-profile", DIMENSIONS, 0x11);
    a.sync(&server).await;

    assert!(
        server.derived_pushes.lock().unwrap().is_empty()
            && server.derived_pull_since.lock().unwrap().is_empty(),
        "a server without the capability receives no derived request at all"
    );
    assert_eq!(
        a.pending_rows(ACCOUNT_A),
        1,
        "the outbox waits for a capable epoch"
    );
    assert!(a
        .meta("derived_capability")
        .expect("capability recorded")
        .contains("\"advertised\":false"));

    b.login(&server, SERVER_URL, ACCOUNT_A);
    b.sync(&server).await;

    assert_eq!(
        b.scalar("SELECT COUNT(*) FROM collections WHERE id = 'collection-1'"),
        1,
        "the corpus keeps syncing on a legacy server"
    );
    assert_eq!(
        b.scalar("SELECT COUNT(*) FROM sync_derived_vectors"),
        0,
        "nothing derived travelled"
    );
    assert!(cached_vectors(
        &b.conn,
        CONTRACT,
        DIMENSIONS as usize,
        &["sha256-of-the-profile".to_string()]
    )
    .is_empty());
    assert!(server.derived_pushes.lock().unwrap().is_empty());
    assert!(server.derived_pull_since.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_account_change_never_pushes_the_previous_accounts_rows_including_offline_ones() {
    let server = MockServer::new();
    let (mut device, _b) = devices();

    // A works, logs out, and the device keeps processing offline: those rows
    // carry the archive's bound account (A), never '' and never the next one.
    device.login(&server, SERVER_URL, ACCOUNT_A);
    device.seed_paid_vector(CONTRACT, "input-a", DIMENSIONS, 0x21);
    device.logout();
    device.seed_paid_vector(CONTRACT, "offline-after-logout", DIMENSIONS, 0x22);
    device.seed_paid_ocr_page(SOURCE_HASH, 1, OCR_CONTRACT, "text", "offline page");

    // B logs into the same archive: it must not push nor read a single row of A.
    device.login(&server, SERVER_URL, ACCOUNT_B);
    device.sync(&server).await;

    assert!(
        server.derived_pushes.lock().unwrap().is_empty(),
        "B pushes nothing: the pending rows are A's"
    );
    assert!(
        cached_vectors(
            &device.conn,
            CONTRACT,
            DIMENSIONS as usize,
            &["input-a".to_string()]
        )
        .is_empty(),
        "B cannot read A's rows"
    );
    assert_eq!(
        device.pending_rows(ACCOUNT_A),
        3,
        "A's rows stay inert, awaiting A"
    );
    assert_eq!(device.pending_rows(ACCOUNT_B), 0);

    // A comes back: its rows are still its own and are pushed now.
    device.login(&server, SERVER_URL, ACCOUNT_A);
    device.sync(&server).await;

    let keys = server.derived_keys(ACCOUNT_A);
    assert!(
        keys.iter().any(|(table, row_id)| {
            table == "derived_vectors" && row_id.contains("offline-after-logout")
        }),
        "the offline row is pushed under A: {keys:?}"
    );
    assert_eq!(device.pending_rows(ACCOUNT_A), 0);
    assert_eq!(
        server.derived_keys(ACCOUNT_B).len(),
        0,
        "no row ever landed under B"
    );
}

#[tokio::test]
async fn catchup_runs_once_per_epoch_and_the_gate_request_forces_an_incremental_pull() {
    let server = MockServer::new();
    let (mut a, mut b) = devices();

    a.login(&server, SERVER_URL, ACCOUNT_A);
    a.seed_paid_vector(CONTRACT, "first", DIMENSIONS, 0x31);
    a.sync(&server).await;

    let pulls_before_b = server.derived_pull_since.lock().unwrap().len();
    b.login(&server, SERVER_URL, ACCOUNT_A);
    b.sync(&server).await;
    assert_eq!(
        server.derived_pull_since.lock().unwrap()[pulls_before_b..].to_vec(),
        vec![0],
        "B's first supported cycle is a since-zero catch-up"
    );
    assert!(b
        .meta("derived_catchup_epoch")
        .is_some_and(|epoch| !epoch.is_empty()));

    // A uploads after B's catch-up; the gate requests a pull and it arrives
    // before the next cycle would have pulled it by itself.
    a.seed_paid_vector(CONTRACT, "second", DIMENSIONS, 0x32);
    a.sync(&server).await;
    let pulls_before_request = server.derived_pull_since.lock().unwrap().len();
    request_incremental_pull(&b.conn).expect("gate request");
    b.sync(&server).await;

    let since = server.derived_pull_since.lock().unwrap().clone();
    let new_pulls = &since[pulls_before_request..];
    assert_eq!(
        new_pulls.len(),
        1,
        "one requested incremental pull: {new_pulls:?}"
    );
    assert!(
        new_pulls[0] > 0,
        "the incremental pull resumes from the cursor"
    );
    assert!(
        cached_vectors(
            &b.conn,
            CONTRACT,
            DIMENSIONS as usize,
            &["second".to_string()]
        )
        .contains_key("second"),
        "the freshness pull brought A's new row before B would have paid for it"
    );
    assert!(
        b.meta("derived_pull_requested").is_none(),
        "the request is consumed"
    );
    assert!(last_pull_started_at(&b.conn).is_some());

    // A rotated epoch re-runs the catch-up from zero.
    let pulls_before_rotation = server.derived_pull_since.lock().unwrap().len();
    server.set_epoch("epoch-2");
    b.sync(&server).await;
    let since = server.derived_pull_since.lock().unwrap().clone();
    assert_eq!(
        &since[pulls_before_rotation..],
        &[0],
        "a new epoch pulls the channel from zero again: {since:?}"
    );
}

#[tokio::test]
async fn push_batches_respect_the_byte_budget_and_malformed_rows_are_skipped() {
    let mut server = MockServer::new();
    server.max_push_bytes = 900;
    let (mut a, mut b) = devices();

    a.login(&server, SERVER_URL, ACCOUNT_A);
    for index in 0..10 {
        a.seed_paid_vector(CONTRACT, &format!("input-{index}"), DIMENSIONS, index as u8);
    }
    a.sync(&server).await;

    let batches = server.derived_pushes.lock().unwrap().clone();
    assert!(
        batches.len() > 1,
        "the outbox is split by bytes: {}",
        batches.len()
    );
    for request in &batches {
        let bytes = serde_json::to_vec(request).expect("serialize batch").len();
        assert!(
            bytes <= 900,
            "a derived batch of {bytes} bytes exceeded the 900-byte budget"
        );
    }
    let sent: Vec<String> = batches
        .iter()
        .flat_map(|request| request.changes.iter())
        .filter(|change| change.table == "derived_vectors")
        .map(|change| change.row_id.clone())
        .collect();
    let mut unique = sent.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 10, "every row travelled exactly once");
    assert_eq!(sent.len(), 10);
    assert_eq!(a.pending_rows(ACCOUNT_A), 0);

    // A row that breaks the wire contract must be skipped on receipt, not
    // repaired and not published: B keeps the ten valid rows only.
    // Dimensions say 12 floats, the blob carries 8: a width a reader must reject.
    let mut bad_width = vector_payload(CONTRACT, "bad:width", DIMENSIONS, 1.0);
    bad_width["dimensions"] = json!(DIMENSIONS + 4);
    server.inject_derived_row(ACCOUNT_A, "derived_vectors", "bad:width", bad_width);
    server.inject_derived_row(
        ACCOUNT_A,
        "derived_vectors",
        "bad:base64",
        json!({
            "contract_hash": CONTRACT,
            "input_hash": "bad:base64",
            "dimensions": DIMENSIONS,
            "embedding": "*** not base64 ***",
        }),
    );

    b.login(&server, SERVER_URL, ACCOUNT_A);
    b.sync(&server).await;
    assert_eq!(
        b.scalar("SELECT COUNT(*) FROM sync_derived_vectors"),
        10,
        "the two malformed rows were skipped"
    );
    assert!(!cached_vectors(
        &b.conn,
        CONTRACT,
        DIMENSIONS as usize,
        &["bad:width".to_string()]
    )
    .contains_key("bad:width"));
    for index in 0..10 {
        assert!(
            cached_vectors(
                &b.conn,
                CONTRACT,
                DIMENSIONS as usize,
                &[format!("input-{index}")]
            )
            .contains_key(&format!("input-{index}")),
            "input-{index} reached B"
        );
    }
}

#[tokio::test]
async fn a_capability_discovered_on_a_later_epoch_pushes_what_waited() {
    let server = MockServer::new();
    let (mut a, _b) = devices();
    // Start on a legacy server: the paid row waits.
    let legacy = MockServer::legacy();
    a.login(&legacy, SERVER_URL, ACCOUNT_A);
    a.seed_paid_vector(CONTRACT, "waited", DIMENSIONS, 0x41);
    a.sync(&legacy).await;
    assert_eq!(a.pending_rows(ACCOUNT_A), 1);
    assert!(legacy.derived_pushes.lock().unwrap().is_empty());

    // The server is upgraded, with a new epoch: the ordinary response advertises
    // the token, so the outbox goes out idempotently and the catch-up runs.
    a.login(&server, SERVER_URL, ACCOUNT_A);
    a.sync(&server).await;
    assert_eq!(a.pending_rows(ACCOUNT_A), 0);
    assert_eq!(
        server.derived_keys(ACCOUNT_A).len(),
        2,
        "contract + vector reached the upgraded server"
    );
    assert_eq!(
        server.derived_pull_since.lock().unwrap().as_slice(),
        &[0],
        "the upgraded epoch also catches up"
    );
    assert_eq!(
        server.derived_pushes.lock().unwrap()[0].changes[0].table,
        "derived_contracts"
    );
}
