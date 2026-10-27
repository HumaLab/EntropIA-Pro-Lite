//! Transport tests for `bibliography-derived-v1` (plan §2.5 option (b), §2.9,
//! WU4): the frozen request scope, the capability gate, the channel's own
//! cursor and catch-up, byte-bounded push batches and receipt validation.
//!
//! The two-device acceptance lives in `tests/derived_sync_two_device.rs`; these
//! tests drive the transport units directly so a "response released after an
//! account switch" is deterministic (prepare → switch → install/settle) and no
//! thread or server is needed.

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use rusqlite::{params, Connection};
use serde_json::json;

use super::derived::{
    cache_paid_vector, cached_ocr_pages, cached_vectors, capability_state, clear_account_metadata,
    last_pull_started_at, pull_requested, record_capability, request_incremental_pull,
    DerivedCapability,
};
use super::derived_transport::{
    install_derived_pull_page, next_derived_push_batch, prepare_derived_pull, prepare_derived_push,
    read_scope, run_derived_cycle, settle_derived_push, DerivedDiscard, DerivedPullOutcome,
    DerivedPushOutcome, CONTRACT_TABLE, OCR_TABLE, VECTOR_TABLE,
};
use super::http::{
    PullResponse, PullRow, PushRequest, PushResponse, BIBLIOGRAPHY_DERIVED_V1_CAPABILITY,
};
use super::session::{clear_sync_state, meta_get, meta_set, write_sync_session};
use super::test_support::{new_synced_test_db, MockSyncApi};

const ACCOUNT_A: &str = "account-a";
const ACCOUNT_B: &str = "account-b";
const EPOCH: &str = "epoch-1";
const CONTRACT: &str = "contract-v1";
const OCR_CONTRACT: &str = "ocr-v1:glm:glm-ocr:p1";
const SOURCE_HASH: &str = "3b1e000000000000000000000000000000000000000000000000000000000000";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn login(conn: &Connection, account: &str) {
    write_sync_session(
        conn,
        "https://sync.example.test",
        account,
        "reader@example.test",
        "device-a",
    )
    .expect("write sync session");
    meta_set(conn, "server_epoch", EPOCH).expect("server epoch");
}

/// A supported capability for the epoch the session is in.
fn support_channel(conn: &Connection) {
    record_capability(conn, EPOCH, true).expect("record capability");
    super::derived::record_catchup_done(conn, EPOCH).expect("record catch-up");
}

fn blob(dimensions: i64, value: u8) -> Vec<u8> {
    vec![value; (dimensions * 4) as usize]
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .expect("count rows")
}

fn pending_count(conn: &Connection, account: &str) -> i64 {
    conn.query_row(
        "SELECT (SELECT COUNT(*) FROM sync_derived_vectors
                  WHERE account_id = ?1 AND pushed = 0)
              + (SELECT COUNT(*) FROM sync_derived_ocr_pages
                  WHERE account_id = ?1 AND pushed = 0)",
        [account],
        |row| row.get(0),
    )
    .expect("count pending rows")
}

fn pushed_of(conn: &Connection, table: &str, account: &str) -> i64 {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE account_id = ?1 AND pushed = 1"),
        [account],
        |row| row.get(0),
    )
    .expect("count pushed")
}

fn page(rows: Vec<PullRow>, next_since: i64, has_more: bool) -> PullResponse {
    PullResponse {
        rows,
        next_since,
        has_more,
        schema_tag: String::new(),
        server_epoch: EPOCH.to_string(),
        server_now_ms: 0,
        capabilities: vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()],
    }
}

fn vector_row(contract: &str, input: &str, dimensions: i64, embedding: &[u8]) -> PullRow {
    PullRow {
        table: VECTOR_TABLE.to_string(),
        row_id: format!("{contract}:{input}"),
        server_seq: 1,
        deleted: false,
        changed_at: 0,
        device_id: "device-a".to_string(),
        payload: Some(json!({
            "contract_hash": contract,
            "input_hash": input,
            "dimensions": dimensions,
            "embedding": BASE64_STANDARD.encode(embedding),
            "created_at": 1,
        })),
    }
}

fn ocr_row(source_hash: &str, page_number: i64, outcome: &str, text: &str) -> PullRow {
    PullRow {
        table: OCR_TABLE.to_string(),
        row_id: format!("{source_hash}:{page_number}:{OCR_CONTRACT}"),
        server_seq: 2,
        deleted: false,
        changed_at: 0,
        device_id: "device-a".to_string(),
        payload: Some(json!({
            "source_hash": source_hash,
            "page_number": page_number,
            "ocr_contract": OCR_CONTRACT,
            "outcome": outcome,
            "text_content": text,
            "text_hash": super::derived::sha256_hex(text.as_bytes()),
            "created_at": 1,
        })),
    }
}

fn contract_row() -> PullRow {
    PullRow {
        table: CONTRACT_TABLE.to_string(),
        row_id: CONTRACT.to_string(),
        server_seq: 3,
        deleted: false,
        changed_at: 0,
        device_id: "device-a".to_string(),
        payload: Some(json!({
            "contract_hash": CONTRACT,
            "provider": "openrouter",
            "model": "text-embedding-3-large",
            "dimensions": 4,
            "chunking_contract": "chunks-v1",
        })),
    }
}

fn support_response() -> PushResponse {
    PushResponse {
        results: Vec::new(),
        max_server_seq: 3,
        server_epoch: EPOCH.to_string(),
        server_now_ms: 0,
        capabilities: vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()],
    }
}

// ---------------------------------------------------------------------------
// Capability (§2.5, §2.6)
// ---------------------------------------------------------------------------

#[test]
fn capability_states_are_three_and_follow_ordinary_responses_per_epoch() {
    let conn = new_synced_test_db();
    assert_eq!(
        capability_state(&conn),
        DerivedCapability::Unknown,
        "no session, no epoch: nothing is known"
    );

    login(&conn, ACCOUNT_A);
    assert_eq!(
        capability_state(&conn),
        DerivedCapability::Unknown,
        "an epoch without a recorded verdict is unknown, not unsupported"
    );

    record_capability(&conn, EPOCH, true).expect("advertised");
    assert_eq!(capability_state(&conn), DerivedCapability::Supported);

    record_capability(&conn, EPOCH, false).expect("not advertised");
    assert_eq!(
        capability_state(&conn),
        DerivedCapability::Unsupported,
        "a legacy server is confirmed, not just unknown"
    );

    record_capability(&conn, "epoch-2", true).expect("other epoch");
    assert_eq!(
        capability_state(&conn),
        DerivedCapability::Unknown,
        "a verdict for another epoch is never trusted"
    );

    // Logout drops the record: the channel keeps its rows but nothing is known
    // about the next session's server until it advertises again.
    record_capability(&conn, EPOCH, true).expect("advertised again");
    cache_paid_vector(&conn, CONTRACT, "input-1", 1, &blob(1, 1)).expect("paid vector");
    clear_sync_state(&conn).expect("logout");
    assert_eq!(capability_state(&conn), DerivedCapability::Unknown);
    assert_eq!(
        count(&conn, "sync_derived_vectors"),
        1,
        "rows survive logout"
    );
    assert!(meta_get(&conn, "derived_capability")
        .expect("meta read")
        .is_none());
}

// ---------------------------------------------------------------------------
// Pull (§2.5, §2.9)
// ---------------------------------------------------------------------------

#[test]
fn pull_installs_rows_of_the_frozen_account_as_remote_and_moves_only_its_cursor() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    record_capability(&conn, EPOCH, true).expect("capability");
    meta_set(&conn, "last_pull_seq", "77").expect("corpus cursor");

    let prepared = prepare_derived_pull(&conn)
        .expect("prepare")
        .expect("a supported channel is prepared");
    assert!(prepared.catchup, "the epoch still needs its catch-up");
    assert_eq!(prepared.since, 0);

    let response = page(
        vec![
            vector_row(CONTRACT, "input-1", 4, &blob(4, 1)),
            ocr_row(SOURCE_HASH, 3, "text", "page three"),
            contract_row(),
        ],
        12,
        false,
    );
    let outcome = install_derived_pull_page(&conn, &prepared, &response).expect("install");
    let DerivedPullOutcome::Installed(install) = outcome else {
        panic!("a matching response must install, got {outcome:?}");
    };
    assert_eq!(install.installed, 3);
    assert_eq!(install.skipped, 0);
    assert!(
        install.catchup_recorded,
        "the terminal page records catch-up"
    );

    let vectors = cached_vectors(&conn, CONTRACT, 4, &["input-1".to_string()]);
    assert_eq!(
        vectors.get("input-1"),
        Some(&blob(4, 1)),
        "the installed vector answers the cache read of its account"
    );
    let pages = cached_ocr_pages(&conn, SOURCE_HASH, OCR_CONTRACT, &[3]);
    assert_eq!(
        pages.get(&3).map(|answer| answer.text.as_str()),
        Some("page three")
    );

    let (account, origin, pushed): (String, String, i64) = conn
        .query_row(
            "SELECT account_id, origin, pushed FROM sync_derived_vectors",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("installed vector");
    assert_eq!(account, ACCOUNT_A, "the frozen account stamps the row");
    assert_eq!(origin, "remote");
    assert_eq!(
        pushed, 1,
        "a row that came from the server is already there"
    );
    assert_eq!(count(&conn, "sync_derived_contracts"), 1);

    let corpus_cursor: String = meta_get(&conn, "last_pull_seq")
        .expect("read")
        .expect("value");
    assert_eq!(corpus_cursor, "77", "last_pull_seq is never moved here");
    let cursor: String = meta_get(&conn, "derived_cursor")
        .expect("read")
        .expect("value");
    assert!(cursor.contains("\"since\":12"), "own cursor: {cursor}");

    // The catch-up ran: the next prepare is incremental from the cursor.
    let incremental = prepare_derived_pull(&conn)
        .expect("prepare")
        .expect("prepared");
    assert!(!incremental.catchup);
    assert_eq!(incremental.since, 12);
}

#[test]
fn pull_skips_rows_that_break_the_wire_contract() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    record_capability(&conn, EPOCH, true).expect("capability");

    let prepared = prepare_derived_pull(&conn)
        .expect("prepare")
        .expect("prepared");

    let mut wrong_width = vector_row(CONTRACT, "wrong-width", 4, &blob(2, 1));
    if let Some(payload) = wrong_width.payload.as_mut() {
        payload["dimensions"] = json!(4);
    }
    let mut non_finite = vector_row(CONTRACT, "non-finite", 4, &blob(4, 1));
    if let Some(payload) = non_finite.payload.as_mut() {
        payload["embedding"] = json!(BASE64_STANDARD.encode([0xff_u8; 16]));
    }
    let mut not_base64 = vector_row(CONTRACT, "not-base64", 4, &blob(4, 1));
    if let Some(payload) = not_base64.payload.as_mut() {
        payload["embedding"] = json!("***not base64***");
    }
    let mut forged_hash = ocr_row(SOURCE_HASH, 4, "text", "paid text");
    if let Some(payload) = forged_hash.payload.as_mut() {
        payload["text_hash"] = json!("00");
    }
    let empty_with_text = ocr_row(SOURCE_HASH, 5, "empty", "text that must not exist");
    let unknown_outcome = ocr_row(SOURCE_HASH, 6, "failed", "");
    let negative_page = ocr_row(SOURCE_HASH, -1, "text", "page");
    let mut zero_dimensions_contract = contract_row();
    if let Some(payload) = zero_dimensions_contract.payload.as_mut() {
        payload["dimensions"] = json!(0);
    }
    let tombstone = PullRow {
        table: VECTOR_TABLE.to_string(),
        row_id: "gone".to_string(),
        server_seq: 4,
        deleted: true,
        changed_at: 0,
        device_id: "device-a".to_string(),
        payload: None,
    };

    let response = page(
        vec![
            wrong_width,
            non_finite,
            not_base64,
            forged_hash,
            empty_with_text,
            unknown_outcome,
            negative_page,
            zero_dimensions_contract,
            tombstone,
        ],
        5,
        false,
    );
    let outcome = install_derived_pull_page(&conn, &prepared, &response).expect("install");
    let DerivedPullOutcome::Installed(install) = outcome else {
        panic!("a matching response installs its usable rows");
    };
    assert_eq!(install.installed, 0, "no unusable row may enter the cache");
    assert_eq!(install.skipped, 9);
    assert_eq!(count(&conn, "sync_derived_vectors"), 0);
    assert_eq!(count(&conn, "sync_derived_ocr_pages"), 0);
    assert_eq!(count(&conn, "sync_derived_contracts"), 0);
    assert!(cached_vectors(&conn, CONTRACT, 4, &["wrong-width".to_string()]).is_empty());
    assert!(cached_ocr_pages(&conn, SOURCE_HASH, OCR_CONTRACT, &[4, 5, 6]).is_empty());
}

#[test]
fn catchup_runs_once_per_epoch_and_a_new_epoch_runs_it_again_from_zero() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    support_channel(&conn);

    let prepared = prepare_derived_pull(&conn)
        .expect("prepare")
        .expect("prepared");
    assert!(!prepared.catchup, "the epoch's catch-up is recorded");
    let response = page(vec![], 4, false);
    install_derived_pull_page(&conn, &prepared, &response).expect("install");
    let after = prepare_derived_pull(&conn)
        .expect("prepare")
        .expect("prepared");
    assert!(!after.catchup);
    assert_eq!(after.since, 4, "incremental pulls continue from the cursor");

    // A rotated epoch: the verdict is unknown until it advertises, and the
    // catch-up runs again from zero — the old epoch's cursor never skips rows.
    meta_set(&conn, "server_epoch", "epoch-2").expect("new epoch");
    record_capability(&conn, "epoch-2", true).expect("capability");
    let rotated = prepare_derived_pull(&conn)
        .expect("prepare")
        .expect("prepared");
    assert!(rotated.catchup, "a new epoch re-runs the catch-up");
    assert_eq!(rotated.since, 0);
    assert_eq!(
        last_pull_started_at(&conn),
        None,
        "the previous epoch's pull start is session metadata, gone with it"
    );
}

#[test]
fn a_pull_released_after_an_account_switch_installs_nothing_for_the_new_account() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    record_capability(&conn, EPOCH, true).expect("capability");

    // The request leaves with A's session; the answer arrives after the switch.
    let prepared_a = prepare_derived_pull(&conn)
        .expect("prepare")
        .expect("prepared");
    let mut response_a = page(
        vec![vector_row(CONTRACT, "input-1", 4, &blob(4, 1))],
        999,
        false,
    );
    response_a.capabilities = vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()];

    clear_sync_state(&conn).expect("logout");
    login(&conn, ACCOUNT_B);
    meta_set(&conn, "server_epoch", EPOCH).expect("B epoch");
    record_capability(&conn, EPOCH, true).expect("B capability");

    // B already has its own catch-up and cursor: A's answer must not touch them.
    let prepared_b = prepare_derived_pull(&conn)
        .expect("prepare")
        .expect("prepared");
    let install_b =
        install_derived_pull_page(&conn, &prepared_b, &page(vec![], 40, false)).expect("install B");
    assert!(matches!(install_b, DerivedPullOutcome::Installed(_)));
    request_incremental_pull(&conn).expect("B gate request");

    let outcome = install_derived_pull_page(&conn, &prepared_a, &response_a).expect("install A");
    assert_eq!(
        outcome,
        DerivedPullOutcome::Discarded(DerivedDiscard::SessionChanged)
    );
    assert_eq!(
        count(&conn, "sync_derived_vectors"),
        0,
        "A's row was not installed"
    );
    assert_eq!(
        capability_state(&conn),
        DerivedCapability::Supported,
        "B's capability is untouched"
    );
    let cursor: String = meta_get(&conn, "derived_cursor")
        .expect("read")
        .expect("value");
    assert!(
        cursor.contains("\"since\":40"),
        "B's own cursor did not move: {cursor}"
    );
    assert_eq!(
        meta_get(&conn, "derived_catchup_epoch").expect("read"),
        Some(EPOCH.to_string()),
        "B's catch-up record is untouched"
    );
    assert_eq!(last_pull_started_at(&conn), None);
    assert!(
        pull_requested(&conn).expect("read"),
        "a discarded response consumes no gate request"
    );
    assert!(
        cached_vectors(&conn, CONTRACT, 4, &["input-1".to_string()]).is_empty(),
        "B cannot read A's rows"
    );
}

#[test]
fn a_pull_page_from_another_epoch_or_without_the_capability_is_discarded() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    support_channel(&conn);
    let prepared = prepare_derived_pull(&conn)
        .expect("prepare")
        .expect("prepared");

    let mut other_epoch = page(
        vec![vector_row(CONTRACT, "input-1", 4, &blob(4, 1))],
        1,
        true,
    );
    other_epoch.server_epoch = "epoch-2".to_string();
    assert_eq!(
        install_derived_pull_page(&conn, &prepared, &other_epoch).expect("install"),
        DerivedPullOutcome::Discarded(DerivedDiscard::EpochChanged)
    );

    let mut silent = page(
        vec![vector_row(CONTRACT, "input-1", 4, &blob(4, 1))],
        1,
        true,
    );
    silent.capabilities.clear();
    assert_eq!(
        install_derived_pull_page(&conn, &prepared, &silent).expect("install"),
        DerivedPullOutcome::Discarded(DerivedDiscard::CapabilityMissing)
    );

    assert_eq!(count(&conn, "sync_derived_vectors"), 0);
    assert!(meta_get(&conn, "derived_cursor").expect("read").is_none());
}

// ---------------------------------------------------------------------------
// Push (§2.5, §2.9)
// ---------------------------------------------------------------------------

#[test]
fn push_prepares_only_the_session_accounts_unanswered_rows() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    support_channel(&conn);
    conn.execute(
        "INSERT INTO bibliographic_embedding_contracts
           (contract_hash, provider, model, dimensions, chunking_contract, created_at)
         VALUES (?1, 'openrouter', 'text-embedding-3-large', 4, 'chunks-v1', 1)",
        [CONTRACT],
    )
    .expect("source contract");
    cache_paid_vector(&conn, CONTRACT, "input-a", 4, &blob(4, 1)).expect("A vector");
    cache_paid_vector(&conn, CONTRACT, "input-a2", 4, &blob(4, 2)).expect("A vector");
    conn.execute(
        "INSERT INTO sync_derived_vectors
           (account_id, contract_hash, input_hash, dimensions, embedding, origin, pushed, created_at)
         VALUES (?1, ?2, 'input-b', 4, ?3, 'local', 0, 1)",
        params![ACCOUNT_B, CONTRACT, blob(4, 3)],
    )
    .expect("B row");
    super::derived::insert_ocr_page(
        &conn,
        ACCOUNT_B,
        SOURCE_HASH,
        1,
        OCR_CONTRACT,
        "text",
        "B page",
        "local",
    )
    .expect("B page");

    let mut prepared = prepare_derived_push(&conn, 8 * 1024 * 1024)
        .expect("prepare")
        .expect("prepared");
    let mut sent: Vec<(String, String)> = Vec::new();
    while let Some(batch) = next_derived_push_batch(&conn, &mut prepared).expect("batch") {
        sent.extend(
            batch
                .changes
                .iter()
                .map(|change| (change.table.clone(), change.row_id.clone())),
        );
    }
    assert!(
        sent.iter()
            .all(|(table, row_id)| table == CONTRACT_TABLE || !row_id.contains("input-b")),
        "a row of another account is never sent: {sent:?}"
    );
    assert_eq!(
        sent.iter()
            .filter(|(table, _)| table == VECTOR_TABLE)
            .count(),
        2
    );
    assert!(
        sent.iter().any(|(table, _)| table == CONTRACT_TABLE),
        "the vector's contract travels with it"
    );
    assert_eq!(
        pushed_of(&conn, "sync_derived_vectors", ACCOUNT_A),
        0,
        "preparing marks nothing"
    );
    assert_eq!(
        count(&conn, "sync_derived_ocr_pages"),
        1,
        "B's page excluded"
    );
}

#[test]
fn push_splits_into_byte_bounded_batches_that_cover_every_row_once() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    support_channel(&conn);
    for index in 0..12 {
        cache_paid_vector(
            &conn,
            CONTRACT,
            &format!("input-{index}"),
            4,
            &blob(4, index as u8),
        )
        .expect("paid vector");
    }

    let limit = 1_200usize;
    let mut prepared = prepare_derived_push(&conn, limit)
        .expect("prepare")
        .expect("prepared");

    let mut seen = std::collections::HashSet::new();
    let mut batches = 0usize;
    while let Some(batch) = next_derived_push_batch(&conn, &mut prepared).expect("batch") {
        batches += 1;
        let bytes = serde_json::to_vec(&PushRequest {
            changes: batch.changes.clone(),
        })
        .expect("serialize batch")
        .len();
        assert!(
            bytes <= limit,
            "batch of {bytes} bytes exceeds the {limit}-byte budget"
        );
        for key in &batch.vectors {
            assert!(
                seen.insert(key.input_hash.clone()),
                "a row is sent twice: {}",
                key.input_hash
            );
        }
    }
    assert!(batches > 1, "the outbox must be split");
    assert_eq!(seen.len(), 12, "every pending vector travels exactly once");
}

#[test]
fn push_settles_only_the_sent_keys_and_only_while_the_scope_still_matches() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    support_channel(&conn);
    for index in 0..6 {
        cache_paid_vector(
            &conn,
            CONTRACT,
            &format!("input-{index}"),
            4,
            &blob(4, index as u8),
        )
        .expect("paid vector");
    }
    let mut prepared = prepare_derived_push(&conn, 1_400)
        .expect("prepare")
        .expect("prepared");
    let first = next_derived_push_batch(&conn, &mut prepared)
        .expect("first batch")
        .expect("a first batch");

    let outcome =
        settle_derived_push(&conn, &prepared, &first, &support_response()).expect("settle");
    let DerivedPushOutcome::Settled(settlement) = outcome else {
        panic!("a matching response settles");
    };
    assert_eq!(settlement.vectors_marked, first.vectors.len());
    for key in &first.vectors {
        let pushed: i64 = conn
            .query_row(
                "SELECT pushed FROM sync_derived_vectors
                  WHERE account_id = ?1 AND contract_hash = ?2 AND input_hash = ?3",
                params![ACCOUNT_A, key.contract_hash, key.input_hash],
                |row| row.get(0),
            )
            .expect("row");
        assert_eq!(pushed, 1);
    }
    let second = next_derived_push_batch(&conn, &mut prepared)
        .expect("second batch")
        .expect("a second batch");
    for key in &second.vectors {
        let pushed: i64 = conn
            .query_row(
                "SELECT pushed FROM sync_derived_vectors
                  WHERE account_id = ?1 AND contract_hash = ?2 AND input_hash = ?3",
                params![ACCOUNT_A, key.contract_hash, key.input_hash],
                |row| row.get(0),
            )
            .expect("row");
        assert_eq!(pushed, 0, "an unsent key is never marked");
    }

    // The second batch's response arrives after an account switch: nothing is
    // marked under B and A's rows stay pending for A's next session.
    clear_sync_state(&conn).expect("logout");
    login(&conn, ACCOUNT_B);
    let outcome =
        settle_derived_push(&conn, &prepared, &second, &support_response()).expect("settle");
    assert_eq!(
        outcome,
        DerivedPushOutcome::Discarded(DerivedDiscard::SessionChanged)
    );
    assert_eq!(
        pushed_of(&conn, "sync_derived_vectors", ACCOUNT_A),
        0,
        "the logout reset A's flags and B's discarded response marked none of them again"
    );
    assert_eq!(
        pushed_of(&conn, "sync_derived_vectors", ACCOUNT_B),
        0,
        "B's rows are untouched"
    );
    assert_eq!(
        count(&conn, "sync_derived_vectors"),
        6,
        "the rows themselves survive the account change"
    );
}

#[test]
fn push_marks_nothing_when_the_response_lacks_the_capability_or_the_epoch() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    support_channel(&conn);
    cache_paid_vector(&conn, CONTRACT, "input-1", 4, &blob(4, 1)).expect("paid vector");
    let mut prepared = prepare_derived_push(&conn, 8 * 1024 * 1024)
        .expect("prepare")
        .expect("prepared");
    let batch = next_derived_push_batch(&conn, &mut prepared)
        .expect("batch")
        .expect("a batch");

    let mut silent = support_response();
    silent.capabilities.clear();
    assert_eq!(
        settle_derived_push(&conn, &prepared, &batch, &silent).expect("settle"),
        DerivedPushOutcome::Discarded(DerivedDiscard::CapabilityMissing)
    );

    let mut other_epoch = support_response();
    other_epoch.server_epoch = "epoch-2".to_string();
    assert_eq!(
        settle_derived_push(&conn, &prepared, &batch, &other_epoch).expect("settle"),
        DerivedPushOutcome::Discarded(DerivedDiscard::EpochChanged)
    );
    assert_eq!(pushed_of(&conn, "sync_derived_vectors", ACCOUNT_A), 0);
}

// ---------------------------------------------------------------------------
// Cycle gate (§2.5, §2.6, §2.9)
// ---------------------------------------------------------------------------

fn warn_sink() -> impl Fn(String) + Sync {
    |_message: String| {}
}

#[tokio::test]
async fn a_legacy_server_receives_no_derived_call_and_the_channel_keeps_its_rows() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    record_capability(&conn, EPOCH, false).expect("legacy verdict");
    cache_paid_vector(&conn, CONTRACT, "input-1", 4, &blob(4, 1)).expect("paid vector");

    let api = MockSyncApi::default();
    let warn = warn_sink();
    let outcome = run_derived_cycle(&api, "token-a", &conn, 8 * 1024 * 1024, &warn).await;

    assert_eq!(outcome.capability, Some(DerivedCapability::Unsupported));
    assert_eq!(*api.derived_push_calls.lock().unwrap(), 0);
    assert_eq!(*api.derived_pull_calls.lock().unwrap(), 0);
    assert_eq!(
        pushed_of(&conn, "sync_derived_vectors", ACCOUNT_A),
        0,
        "the outbox waits for a capable epoch"
    );
}

#[tokio::test]
async fn an_unknown_capability_never_calls_the_wire() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    let api = MockSyncApi::default();
    let warn = warn_sink();
    let outcome = run_derived_cycle(&api, "token-a", &conn, 8 * 1024 * 1024, &warn).await;
    assert_eq!(outcome.capability, Some(DerivedCapability::Unknown));
    assert_eq!(*api.derived_push_calls.lock().unwrap(), 0);
    assert_eq!(*api.derived_pull_calls.lock().unwrap(), 0);
}

#[tokio::test]
async fn a_supported_server_catches_up_then_pushes_the_outbox() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    record_capability(&conn, EPOCH, true).expect("capability");
    let mut api = MockSyncApi::default();
    api.server_epoch = EPOCH.to_string();
    *api.server_capabilities.lock().unwrap() = vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()];
    api.queue_pull_page(PullResponse {
        rows: vec![vector_row(CONTRACT, "input-remote", 4, &blob(4, 9))],
        next_since: 9,
        has_more: false,
        schema_tag: String::new(),
        server_epoch: EPOCH.to_string(),
        server_now_ms: 0,
        capabilities: vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()],
    });
    cache_paid_vector(&conn, CONTRACT, "input-local", 4, &blob(4, 1)).expect("paid vector");

    let warn = warn_sink();
    let outcome = run_derived_cycle(&api, "token-a", &conn, 8 * 1024 * 1024, &warn).await;
    assert!(
        outcome.catchup_recorded,
        "the first supported cycle catches up"
    );
    assert_eq!(outcome.rows_installed, 1);
    assert_eq!(*api.derived_pull_calls.lock().unwrap(), 1);
    assert_eq!(*api.derived_push_calls.lock().unwrap(), 1);
    assert_eq!(
        outcome.rows_pushed, 1,
        "the local vector is now acknowledged"
    );
    let pushed: i64 = conn
        .query_row(
            "SELECT pushed FROM sync_derived_vectors
              WHERE account_id = ?1 AND input_hash = 'input-local'",
            [ACCOUNT_A],
            |row| row.get(0),
        )
        .expect("local vector");
    assert_eq!(pushed, 1);
    assert_eq!(
        cached_vectors(&conn, CONTRACT, 4, &["input-remote".to_string()]).get("input-remote"),
        Some(&blob(4, 9))
    );
}

#[tokio::test]
async fn the_gate_request_forces_one_incremental_pull_and_is_consumed() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    support_channel(&conn);
    // A pull that just happened: without a request the phase does not repeat it.
    let recent = now_ms() - 1_000;
    super::derived::record_pull_started_at(&conn, EPOCH, recent).expect("last pull");

    let mut api = MockSyncApi::default();
    api.server_epoch = EPOCH.to_string();
    *api.server_capabilities.lock().unwrap() = vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()];
    api.queue_pull_page(PullResponse {
        rows: vec![],
        next_since: 0,
        has_more: false,
        schema_tag: String::new(),
        server_epoch: EPOCH.to_string(),
        server_now_ms: 0,
        capabilities: vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()],
    });
    let warn = warn_sink();

    let idle = run_derived_cycle(&api, "token-a", &conn, 8 * 1024 * 1024, &warn).await;
    assert!(!idle.pulled_incrementally, "a recent pull is not repeated");
    assert_eq!(*api.derived_pull_calls.lock().unwrap(), 0);

    request_incremental_pull(&conn).expect("request");
    let forced = run_derived_cycle(&api, "token-a", &conn, 8 * 1024 * 1024, &warn).await;
    assert!(forced.pull_requested);
    assert!(forced.pulled_incrementally, "the request is honored");
    assert_eq!(*api.derived_pull_calls.lock().unwrap(), 1);
    assert!(
        !pull_requested(&conn).expect("read"),
        "a successful install consumes the request"
    );
    assert!(
        last_pull_started_at(&conn).expect("freshness signal") > recent,
        "the freshness signal advances with the pull"
    );
}

/// A requested pull can be satisfied by the epoch's catch-up: a since-zero walk
/// of the whole channel is strictly fresher than the incremental page the gate
/// asked for, so the completed catch-up consumes the request instead of leaving
/// it latched for a second pull.
#[tokio::test]
async fn a_completed_catchup_consumes_the_gate_request() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    record_capability(&conn, EPOCH, true).expect("capability");
    request_incremental_pull(&conn).expect("gate request");

    let mut api = MockSyncApi::default();
    api.server_epoch = EPOCH.to_string();
    *api.server_capabilities.lock().unwrap() = vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()];
    api.queue_pull_page(PullResponse {
        rows: vec![],
        next_since: 3,
        has_more: false,
        schema_tag: String::new(),
        server_epoch: EPOCH.to_string(),
        server_now_ms: 0,
        capabilities: vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()],
    });

    let warn = warn_sink();
    let outcome = run_derived_cycle(&api, "token-a", &conn, 8 * 1024 * 1024, &warn).await;

    assert!(outcome.catchup_recorded);
    assert!(
        !pull_requested(&conn).expect("read"),
        "the completed catch-up consumed the gate request"
    );
    assert_eq!(
        *api.derived_pull_calls.lock().unwrap(),
        1,
        "one catch-up and no redundant incremental pull in the same cycle"
    );
}

/// One row bigger than the server's `max_push_bytes` must not wedge the outbox:
/// it travels alone, a `413` skips it for the rest of the cycle, and the rows
/// after it still go out.
#[tokio::test]
async fn an_oversized_row_is_sent_alone_and_skipped_when_the_server_rejects_it() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    support_channel(&conn);
    conn.execute(
        "INSERT INTO bibliographic_embedding_contracts
           (contract_hash, provider, model, dimensions, chunking_contract, created_at)
         VALUES (?1, 'openrouter', 'text-embedding-3-large', 4, 'chunks-v1', 1)",
        [CONTRACT],
    )
    .expect("source contract");
    // Sorted first by input hash, so the run meets it before the small rows.
    cache_paid_vector(&conn, CONTRACT, "aaa-oversize", 4096, &blob(4096, 3)).expect("big vector");
    for index in 0..3 {
        cache_paid_vector(
            &conn,
            CONTRACT,
            &format!("bbb-{index}"),
            4,
            &blob(4, index as u8),
        )
        .expect("small vector");
    }

    let mut api = MockSyncApi::default();
    api.server_epoch = EPOCH.to_string();
    *api.server_capabilities.lock().unwrap() = vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()];
    *api.derived_push_max_bytes.lock().unwrap() = Some(4 * 1024);

    let warnings = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = {
        let warnings = warnings.clone();
        move |message: String| warnings.lock().unwrap().push(message)
    };
    let outcome = run_derived_cycle(&api, "token-a", &conn, 4 * 1024, &sink).await;

    let pushed = |input: &str| -> i64 {
        conn.query_row(
            "SELECT pushed FROM sync_derived_vectors
              WHERE account_id = ?1 AND contract_hash = ?2 AND input_hash = ?3",
            params![ACCOUNT_A, CONTRACT, input],
            |row| row.get(0),
        )
        .expect("row")
    };
    assert_eq!(pushed("aaa-oversize"), 0, "the rejected row stays pending");
    for index in 0..3 {
        assert_eq!(
            pushed(&format!("bbb-{index}")),
            1,
            "the row after the rejected one still went out"
        );
    }
    assert_eq!(
        outcome.rows_pushed, 3,
        "only the three small rows were acknowledged this cycle"
    );
    assert!(
        api.derived_pushed
            .lock()
            .unwrap()
            .iter()
            .any(|change| change.table == super::derived_transport::CONTRACT_TABLE),
        "the contract row rides with the next vector after the rejected batch"
    );
    let warnings = warnings.lock().unwrap();
    let skips: Vec<&String> = warnings
        .iter()
        .filter(|line| line.contains("skipped") && line.contains("max_push_bytes"))
        .collect();
    assert_eq!(
        skips.len(),
        1,
        "one summary line about the skipped row: {warnings:?}"
    );
}

/// The owner's backfill is about a hundred thousand vectors (~400 MiB): the push
/// run must stream it. Nothing is materialized up front, every request respects
/// the byte budget, and a large outbox drains across several cycles without
/// losing or duplicating a row.
#[tokio::test]
async fn a_large_outbox_streams_across_batches_and_cycles_without_losing_a_row() {
    const ROWS: usize = 5_000;
    const BUDGET: i64 = 3_000;
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    support_channel(&conn);
    conn.execute(
        "INSERT INTO bibliographic_embedding_contracts
           (contract_hash, provider, model, dimensions, chunking_contract, created_at)
         VALUES (?1, 'openrouter', 'text-embedding-3-large', 4, 'chunks-v1', 1)",
        [CONTRACT],
    )
    .expect("source contract");
    for index in 0..ROWS {
        cache_paid_vector(
            &conn,
            CONTRACT,
            &format!("input-{index:05}"),
            4,
            &blob(4, (index % 251) as u8),
        )
        .expect("paid vector");
    }

    let mut api = MockSyncApi::default();
    api.server_epoch = EPOCH.to_string();
    *api.server_capabilities.lock().unwrap() = vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()];
    let warn = warn_sink();

    let mut cycles = 0usize;
    while pending_count(&conn, ACCOUNT_A) > 0 && cycles < 16 {
        run_derived_cycle(&api, "token-a", &conn, BUDGET, &warn).await;
        cycles += 1;
    }

    assert_eq!(
        pending_count(&conn, ACCOUNT_A),
        0,
        "the whole outbox drained in {cycles} cycles"
    );
    assert!(cycles > 1, "5,000 rows do not fit in one cycle: {cycles}");
    assert_eq!(
        count(&conn, "sync_derived_vectors") as usize,
        ROWS,
        "every row is in the cache"
    );
    assert_eq!(
        pushed_of(&conn, "sync_derived_vectors", ACCOUNT_A) as usize,
        ROWS,
        "every key was acknowledged exactly once"
    );

    let requests = api.derived_push_requests.lock().unwrap();
    assert!(
        requests.len() > 64,
        "the outbox needed more batches than one cycle's budget: {}",
        requests.len()
    );
    let mut keys = std::collections::HashSet::new();
    for request in requests.iter() {
        let bytes = serde_json::to_vec(request)
            .expect("serialize request")
            .len();
        assert!(
            bytes <= BUDGET as usize,
            "a request of {bytes} bytes exceeded the {BUDGET}-byte budget"
        );
        assert!(
            request.changes.len() <= 32,
            "a request held {} changes for a {BUDGET}-byte budget",
            request.changes.len()
        );
        for change in &request.changes {
            if change.table == VECTOR_TABLE {
                keys.insert(change.row_id.clone());
            }
        }
    }
    assert_eq!(
        keys.len(),
        ROWS,
        "each vector was sent exactly once across all requests"
    );
}

/// A login mints a fresh session incarnation, so a pull that left with the
/// previous session of the SAME account is stale identity too (§2.9).
#[test]
fn a_pull_released_after_a_same_account_relogin_is_discarded() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    record_capability(&conn, EPOCH, true).expect("capability");

    let prepared = prepare_derived_pull(&conn)
        .expect("prepare")
        .expect("prepared");
    let response = page(
        vec![vector_row(CONTRACT, "input-1", 4, &blob(4, 1))],
        1,
        false,
    );

    let before = meta_get(&conn, "sync_session_incarnation").expect("read");
    clear_sync_state(&conn).expect("logout");
    login(&conn, ACCOUNT_A);
    let after = meta_get(&conn, "sync_session_incarnation").expect("read");
    assert_ne!(
        before, after,
        "a relogin of the same account mints a new session incarnation"
    );

    let outcome = install_derived_pull_page(&conn, &prepared, &response).expect("install");
    assert_eq!(
        outcome,
        DerivedPullOutcome::Discarded(DerivedDiscard::SessionChanged)
    );
    assert_eq!(count(&conn, "sync_derived_vectors"), 0);
    assert!(meta_get(&conn, "derived_cursor").expect("read").is_none());
    assert!(
        cached_vectors(&conn, CONTRACT, 4, &["input-1".to_string()]).is_empty(),
        "the stale response installed nothing the account can read"
    );
}

/// Unreadable channel records never turn into trust: a corrupt capability
/// verdict reads as unknown (no call), a corrupt cursor fails closed to a
/// since-zero pull, and a corrupt freshness stamp reads as "no pull yet".
#[test]
fn malformed_capability_or_cursor_records_fail_closed() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);

    meta_set(&conn, "derived_capability", "{not json").expect("corrupt verdict");
    assert_eq!(capability_state(&conn), DerivedCapability::Unknown);
    meta_set(
        &conn,
        "derived_capability",
        &format!("{{\"epoch\":\"{EPOCH}\",\"advertised\":\"true\"}}"),
    )
    .expect("wrong type");
    assert_eq!(
        capability_state(&conn),
        DerivedCapability::Unknown,
        "a non-boolean verdict is unknown, never supported"
    );
    meta_set(
        &conn,
        "derived_capability",
        &format!("{{\"epoch\":\"{EPOCH}\",\"advertised\":true}}"),
    )
    .expect("valid verdict");
    assert_eq!(capability_state(&conn), DerivedCapability::Supported);

    meta_set(&conn, "derived_cursor", "not a cursor").expect("corrupt cursor");
    let prepared = prepare_derived_pull(&conn)
        .expect("prepare")
        .expect("prepared");
    assert_eq!(
        prepared.since, 0,
        "an unreadable cursor restarts from zero instead of skipping rows"
    );

    meta_set(&conn, "derived_last_pull_started_at", "garbage").expect("corrupt stamp");
    assert_eq!(
        last_pull_started_at(&conn),
        None,
        "a corrupt freshness stamp is no evidence of a pull"
    );
}

#[test]
fn clear_account_metadata_drops_every_channel_key_and_keeps_the_rows() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    support_channel(&conn);
    cache_paid_vector(&conn, CONTRACT, "input-1", 4, &blob(4, 1)).expect("paid vector");
    request_incremental_pull(&conn).expect("request");
    clear_account_metadata(&conn).expect("logout");

    assert_eq!(count(&conn, "sync_derived_vectors"), 1);
    assert_eq!(pushed_of(&conn, "sync_derived_vectors", ACCOUNT_A), 0);
    for key in [
        "derived_capability",
        "derived_catchup_epoch",
        "derived_cursor",
        "derived_last_pull_started_at",
        "derived_wait_verdict",
        "derived_pull_requested",
    ] {
        assert!(
            meta_get(&conn, key).expect("read").is_none(),
            "{key} must be dropped on logout"
        );
    }
}

#[test]
fn the_scope_requires_a_complete_session() {
    let conn = new_synced_test_db();
    assert!(
        read_scope(&conn).expect("read scope").is_none(),
        "no session, no scope"
    );
    login(&conn, ACCOUNT_A);
    assert!(read_scope(&conn).expect("read scope").is_some());
    super::session::meta_delete(&conn, "sync_session_incarnation").expect("drop incarnation");
    assert!(
        read_scope(&conn).expect("read scope").is_none(),
        "no incarnation, no scope"
    );
}

// ── T9 fixes: unserved requests are consumed, catch-up stamps freshness ────

/// D3: a request the phase cannot serve must be consumed. An unsupported epoch
/// (no capability) never serves one, so without this the flag would stay set and
/// the engine's interval ticker would run a full sync cycle every 60 s forever.
#[tokio::test]
async fn an_unsupported_channel_consumes_the_gate_request() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    record_capability(&conn, EPOCH, false).expect("legacy verdict");
    request_incremental_pull(&conn).expect("gate request");

    let api = MockSyncApi::default();
    let warn = warn_sink();
    let outcome = run_derived_cycle(&api, "token-a", &conn, 8 * 1024 * 1024, &warn).await;

    assert_eq!(outcome.capability, Some(DerivedCapability::Unsupported));
    assert!(!outcome.pulled_incrementally);
    assert_eq!(*api.derived_pull_calls.lock().unwrap(), 0);
    assert!(
        !pull_requested(&conn).expect("read"),
        "the request is consumed: the confirm is absence, not a pending pull"
    );
    assert!(
        !super::engine::tick_should_run(&conn, std::time::Duration::from_secs(1)),
        "the ticker no longer bypasses the auto-sync interval"
    );
}

/// D3: a failing pull also consumes the request — the gate's bounded wait
/// proceeds, and the flag cannot spin the engine every 60 s.
#[tokio::test]
async fn a_failing_pull_consumes_the_gate_request() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    support_channel(&conn);
    request_incremental_pull(&conn).expect("gate request");

    let api = MockSyncApi::default();
    // The next derived pull answers 409 cursor_ahead: the phase cannot install it.
    api.set_cursor_ahead(1);
    let warn = warn_sink();
    let outcome = run_derived_cycle(&api, "token-a", &conn, 8 * 1024 * 1024, &warn).await;

    assert!(outcome.pull_requested, "the gate had asked for it");
    assert_eq!(
        *api.derived_pull_calls.lock().unwrap(),
        1,
        "it tried exactly once"
    );
    assert!(
        !pull_requested(&conn).expect("read"),
        "the failed attempt consumed the request"
    );
    assert!(
        !super::engine::tick_should_run(&conn, std::time::Duration::from_secs(1)),
        "the ticker goes back to the configured interval"
    );
}

/// Advisory: a completed catch-up stamps the freshness signal with the instant
/// it STARTED, so the backlog that admitted its work while the catch-up ran is
/// fresh afterwards — no extra pull for material the catch-up already carried.
#[tokio::test]
async fn a_completed_catchup_stamps_the_freshness_signal_with_its_own_start() {
    let conn = new_synced_test_db();
    login(&conn, ACCOUNT_A);
    record_capability(&conn, EPOCH, true).expect("capability");
    let admitted_at = now_ms() - 60_000;
    conn.execute(
        "INSERT INTO processing_tasks
           (id, kind, asset_id_snapshot, domain, subject_kind, subject_id, state, created_at, updated_at)
         VALUES ('task-before-catchup', 'bibliography_extract', 'att-1', 'bibliography',
                 'attachment', 'att-1', 'pending', ?1, ?1)",
        [admitted_at],
    )
    .expect("admitted before the catch-up");

    let mut api = MockSyncApi::default();
    api.server_epoch = EPOCH.to_string();
    *api.server_capabilities.lock().unwrap() = vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()];
    api.queue_pull_page(PullResponse {
        rows: vec![],
        next_since: 3,
        has_more: false,
        schema_tag: String::new(),
        server_epoch: EPOCH.to_string(),
        server_now_ms: 0,
        capabilities: vec![BIBLIOGRAPHY_DERIVED_V1_CAPABILITY.to_string()],
    });

    let warn = warn_sink();
    let outcome = run_derived_cycle(&api, "token-a", &conn, 8 * 1024 * 1024, &warn).await;
    assert!(outcome.catchup_recorded);

    let stamped = last_pull_started_at(&conn).expect("the catch-up stamped the signal");
    assert!(stamped > admitted_at, "stamped with its own start");
    let gate_verdict = super::derived_gate::evaluate(&conn, "task-before-catchup", now_ms());
    assert_eq!(
        gate_verdict,
        super::derived_gate::GateVerdict::Proceed(super::derived_gate::ProceedReason::Fresh),
        "the catch-up covers work admitted before it began"
    );
    assert!(
        !pull_requested(&conn).expect("read"),
        "so the gate asks for nothing"
    );
}
