//! Derived-cache reuse for Collections embeddings (plan-sync-bibliografia-derivada
//! §2.8, task T6, spec S6/S8).
//!
//! These tests drive the real claim → execute → publish path of the embeddings
//! executor against a whole-archive schema fixture. The embedder is a counting
//! fake injected at the provider boundary, so "did this device pay?" is observed
//! exactly: every `embed_text` call is recorded, and a run that must not pay
//! leaves the log empty.
//!
//! What they pin:
//!
//! - a seeded cache finishes an `IncompleteChunks` asset with **zero** provider
//!   calls, publishing rows byte-identical to a paid run (S6);
//! - a partial seed pays only the misses;
//! - another contract, another account, or no usable account at all pays;
//! - without a session the archive's bound account answers, so reprocessing
//!   already-cached material is free (S8);
//! - a paid run leaves one cache row per fragment and one for the asset
//!   aggregate, under the effective contract hash;
//! - a cache write that cannot land never costs the publish;
//! - with no derived tables the publish behaves exactly as before.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use entropia_desktop_lib::bibliography::derived_backfill::{run_backfill, BackfillStatus};
use entropia_desktop_lib::processing::eligibility::{
    self, EmbeddingDecision, EmbeddingStaleReason,
};
use entropia_desktop_lib::processing::embedding::{CollectionEmbedder, EmbeddingExecutor};
use entropia_desktop_lib::processing::repository;
use entropia_desktop_lib::processing::scheduler::{
    run_one, ExecCtx, ExecutorRegistry, RunOneOutcome,
};
use entropia_desktop_lib::sync::schema::ensure_sync_schema;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// The real application schema (every migration + the programmatic layouts),
/// exported from the store package. Same fixture the sync tests use.
const SCHEMA_FIXTURE: &str = include_str!("fixtures/schema_full.sql");

const ASSET: &str = "a1";
const ITEM: &str = "i1";
const DIMENSIONS: i64 = 1024;
/// `RAG_CHUNKING_CONTRACT_V1`, the chunking half of the effective contract.
const RAG_CHUNKING_CONTRACT: &str = "rag-chunk-800-100-char-v1";

/// Long enough to plan four RAG chunks (800 chars each), so the run pays for
/// several fragments plus the aggregate.
fn source_text() -> String {
    "fragmento de coleccion con peso semantico distinto ".repeat(62)
}

struct Archive {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Archive {
    fn conn(&self) -> Connection {
        entropia_desktop_lib::db_open_for_tests(&self.path)
    }
}

/// A throwaway archive carrying the real schema, an `IncompleteChunks` asset
/// (a current vector with no chunk set), and optionally the derived tables.
fn archive(with_derived_tables: bool) -> Archive {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("entropia.sqlite");
    let conn = entropia_desktop_lib::db_open_for_tests(&path);
    conn.execute_batch(SCHEMA_FIXTURE).expect("schema fixture");
    if with_derived_tables {
        ensure_sync_schema(&conn).expect("sync schema");
    }
    let effective = eligibility::resolve_effective_embedding_contract(&conn).expect("contract");
    conn.execute(
        "INSERT INTO collections (id, name, created_at, updated_at) VALUES ('c1', 'Coleccion', 1, 1)",
        [],
    )
    .expect("collection");
    conn.execute(
        "INSERT INTO items (id, title, collection_id, created_at, updated_at) VALUES (?1, 'Obra', 'c1', 1, 1)",
        [ITEM],
    )
    .expect("item");
    conn.execute(
        "INSERT INTO assets (id, item_id, path, type, size, created_at) VALUES (?1, ?2, 'a1.pdf', 'pdf', 1, 1)",
        rusqlite::params![ASSET, ITEM],
    )
    .expect("asset");
    conn.execute(
        "INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES ('e1', ?1, ?2, 'ocr', 1)",
        rusqlite::params![ASSET, source_text()],
    )
    .expect("extraction");
    // Current vector, no chunk set: the eligibility rule reports exactly the
    // `IncompleteChunks` case the plan describes for a device that did not pay.
    conn.execute(
        "INSERT INTO vec_assets (asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            ASSET,
            ITEM,
            vec![0_u8; (DIMENSIONS as usize) * 4],
            effective.model,
            effective.contract,
            effective.dimensions as i64,
        ],
    )
    .expect("existing vector");
    Archive { _dir: dir, path }
}

/// Admits the automatic embedding repair the way the queue does.
fn admit_embedding(conn: &Connection) {
    let batch = repository::ensure_system_batch(conn, "repair").expect("system batch");
    let effective = eligibility::resolve_effective_embedding_contract(conn).expect("contract");
    let fingerprint = eligibility::embedding_input_fingerprint(conn, ASSET).expect("fingerprint");
    repository::admit_or_attach(
        conn,
        &batch,
        "embedding",
        ASSET,
        0,
        &fingerprint,
        &effective.hash,
        None,
    )
    .expect("admit embedding");
}

/// Records every text handed to the provider. The vector is a pure function of
/// that text, which makes a cache hit's published bytes checkable against the
/// vector the paying device computed.
#[derive(Default)]
struct CountingEmbedder {
    calls: Mutex<Vec<String>>,
}

impl CountingEmbedder {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }

    fn vector_for(text: &str) -> Vec<f32> {
        let seed = text.bytes().map(|byte| byte as u32).sum::<u32>() % 97 + 1;
        vec![seed as f32 / 100.0; DIMENSIONS as usize]
    }
}

impl CollectionEmbedder for CountingEmbedder {
    fn embed_text(&self, text: &str) -> Result<Vec<f32>, String> {
        self.calls.lock().expect("calls").push(text.to_string());
        Ok(Self::vector_for(text))
    }
}

fn run_embedding(
    archive: &Archive,
    conn: &Connection,
    embedder: Arc<CountingEmbedder>,
) -> RunOneOutcome {
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(EmbeddingExecutor::with_embedder(
        archive.path.clone(),
        embedder,
    )));
    run_one(
        conn,
        &ExecCtx {
            db_path: archive.path.clone(),
        },
        &registry,
        "embedding-session",
        repository::now_ms(),
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("embedding run")
}

fn succeeded(outcome: &RunOneOutcome) {
    assert!(
        matches!(outcome, RunOneOutcome::Succeeded { .. }),
        "expected a successful publish, got {outcome:?}"
    );
}

/// One published chunk row: `(id, text, embedding, model, contract, dimensions,
/// source_text_hash, chunking_contract)`.
type ChunkRow = (String, String, Vec<u8>, String, String, i64, String, String);

/// Everything a published run leaves in the two canonical tables. Timestamps
/// are excluded on purpose: only content and stamps may be compared.
#[derive(Debug, PartialEq, Eq)]
struct Published {
    vector: (Vec<u8>, String, String, i64),
    chunks: Vec<ChunkRow>,
}

fn published(conn: &Connection) -> Published {
    let vector = conn
        .query_row(
            "SELECT embedding, embedding_model, embedding_contract, dimensions
               FROM vec_assets WHERE asset_id = ?1",
            [ASSET],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("published vector");
    let mut statement = conn
        .prepare(
            "SELECT id, text_content, embedding, embedding_model, embedding_contract,
                    dimensions, source_text_hash, chunking_contract
               FROM rag_chunks WHERE asset_id = ?1 ORDER BY id",
        )
        .expect("chunks query");
    let chunks = statement
        .query_map([ASSET], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })
        .expect("chunks map")
        .collect::<Result<Vec<_>, _>>()
        .expect("chunks collect");
    Published { vector, chunks }
}

fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// `(input_hash, embedding)` of every cached vector, whatever its account.
fn cached_vectors(conn: &Connection) -> Vec<(String, Vec<u8>)> {
    let mut statement = conn
        .prepare("SELECT input_hash, embedding FROM sync_derived_vectors ORDER BY input_hash")
        .expect("cache query");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("cache map")
        .collect::<Result<Vec<_>, _>>()
        .expect("cache collect")
}

fn cache_accounts(conn: &Connection) -> Vec<(String, String, String, i64, String, i64)> {
    let mut statement = conn
        .prepare(
            "SELECT account_id, contract_hash, input_hash, dimensions, origin, pushed
               FROM sync_derived_vectors ORDER BY input_hash",
        )
        .expect("cache query");
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .expect("cache map")
        .collect::<Result<Vec<_>, _>>()
        .expect("cache collect")
}

/// Seeds a cache row exactly as a device of the same account would have left
/// it: remote origin (it arrived through Cloud), already pushed.
fn seed_vectors(conn: &Connection, account: &str, contract: &str, rows: &[(String, Vec<u8>)]) {
    for (input_hash, embedding) in rows {
        conn.execute(
            "INSERT INTO sync_derived_vectors
               (account_id, contract_hash, input_hash, dimensions, embedding, origin, pushed, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'remote', 1, 1)",
            rusqlite::params![account, contract, input_hash, DIMENSIONS, embedding],
        )
        .expect("seed derived vector");
    }
}

/// The state of an archive that synced against a server which does not
/// implement `bibliography-derived-v1`: the epoch is known and the channel is
/// confirmed absent, so the pull-before-pay gate pays exactly as before (§2.6).
/// These suites are about what the cache answers, never about the wait.
fn set_legacy_channel(conn: &Connection) {
    set_meta(conn, "server_epoch", "epoch-legacy");
    set_meta(
        conn,
        "derived_capability",
        "{\"epoch\":\"epoch-legacy\",\"advertised\":false}",
    );
}

fn set_meta(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO sync_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .expect("sync_meta");
}

/// Pays once in a throwaway archive and returns what it paid for: the exact
/// texts sent to the provider (fragments in plan order, then the aggregate),
/// and the cache rows the paid run is expected to leave.
fn paid_baseline() -> (Vec<String>, Vec<(String, Vec<u8>)>) {
    let archive = archive(true);
    let conn = archive.conn();
    admit_embedding(&conn);
    let embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(&archive, &conn, embedder.clone()));
    let calls = embedder.calls();
    assert!(calls.len() >= 2, "fragments plus the aggregate");
    let rows = cached_vectors(&conn);
    (calls, rows)
}

/// A device of the same account: same asset, same vector row (so eligibility
/// still says `IncompleteChunks`), and the seed rows in its own cache.
fn seeded_device(
    account: Option<&str>,
    bound_account: Option<&str>,
    contract: &str,
    rows: &[(String, Vec<u8>)],
) -> (Archive, Connection) {
    let archive = archive(true);
    let conn = archive.conn();
    if let Some(account) = account {
        set_meta(&conn, "account_id", account);
        set_legacy_channel(&conn);
    }
    if let Some(bound) = bound_account {
        set_meta(&conn, "derived_bound_account", bound);
    }
    let owner = account.or(bound_account).unwrap_or("");
    seed_vectors(&conn, owner, contract, rows);
    admit_embedding(&conn);
    (archive, conn)
}

fn effective_contract(conn: &Connection) -> eligibility::EffectiveEmbeddingContract {
    eligibility::resolve_effective_embedding_contract(conn).expect("contract")
}

#[test]
fn the_asset_starts_as_incomplete_chunks() {
    let archive = archive(true);
    let conn = archive.conn();
    assert_eq!(
        eligibility::embedding_decision(&conn, ASSET).expect("decision"),
        EmbeddingDecision::Eligible {
            reason: EmbeddingStaleReason::IncompleteChunks
        }
    );
}

#[test]
fn the_seeded_cache_finishes_incomplete_chunks_without_calling_the_embedder() {
    let (calls, rows) = paid_baseline();
    assert_eq!(rows.len(), calls.len(), "one cache row per paid vector");
    // A paid run in a fresh archive, for the published rows to compare against.
    let paid_archive = archive(true);
    let paid_conn = paid_archive.conn();
    admit_embedding(&paid_conn);
    let paid_embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(
        &paid_archive,
        &paid_conn,
        paid_embedder.clone(),
    ));
    assert_eq!(paid_embedder.calls(), calls, "baseline pays every vector");
    let paid = published(&paid_conn);

    let contract = effective_contract(&paid_conn).hash;
    let (reader, reader_conn) = seeded_device(Some("acct-A"), None, &contract, &rows);
    assert_eq!(
        eligibility::embedding_decision(&reader_conn, ASSET).expect("decision"),
        EmbeddingDecision::Eligible {
            reason: EmbeddingStaleReason::IncompleteChunks
        },
        "B still owes the chunk set; the cache is what makes it free"
    );
    let embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(&reader, &reader_conn, embedder.clone()));
    assert!(
        embedder.calls().is_empty(),
        "a seeded cache must not call the provider: {:?}",
        embedder.calls()
    );
    assert_eq!(
        published(&reader_conn),
        paid,
        "the cached publish must be byte-identical to a computed one"
    );
}

#[test]
fn a_partial_seed_pays_only_the_missing_fragments() {
    let (calls, rows) = paid_baseline();
    // calls = fragments..., aggregate. Drop the second fragment's row.
    let missing_index = 1;
    let rows: Vec<(String, Vec<u8>)> = rows
        .into_iter()
        .filter(|(hash, _)| *hash != sha256_hex(&calls[missing_index]))
        .collect();
    let contract = {
        let archive = archive(true);
        effective_contract(&archive.conn()).hash
    };
    let (reader, reader_conn) = seeded_device(Some("acct-A"), None, &contract, &rows);
    let embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(&reader, &reader_conn, embedder.clone()));
    assert_eq!(
        embedder.calls(),
        vec![calls[missing_index].clone()],
        "only the unseeded fragment is paid: its aggregate row is a hit too"
    );
}

#[test]
fn another_contract_pays_again() {
    let (calls, rows) = paid_baseline();
    let (reader, reader_conn) = seeded_device(Some("acct-A"), None, "another-contract", &rows);
    let embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(&reader, &reader_conn, embedder.clone()));
    assert_eq!(
        embedder.calls(),
        calls,
        "rows under another contract are a different space"
    );
}

#[test]
fn another_accounts_rows_are_never_used() {
    let (calls, rows) = paid_baseline();
    let archive = archive(true);
    let conn = archive.conn();
    set_meta(&conn, "account_id", "acct-A");
    set_legacy_channel(&conn);
    let contract = effective_contract(&conn).hash;
    seed_vectors(&conn, "acct-other", &contract, &rows);
    admit_embedding(&conn);
    let embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(&archive, &conn, embedder.clone()));
    assert_eq!(
        embedder.calls(),
        calls,
        "a row of another account is inert: the device pays its own material"
    );
}

#[test]
fn without_a_session_the_bound_accounts_rows_are_free() {
    let (calls, rows) = paid_baseline();
    let contract = {
        let archive = archive(true);
        effective_contract(&archive.conn()).hash
    };
    let (reader, reader_conn) = seeded_device(None, Some("acct-owner"), &contract, &rows);
    let embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(&reader, &reader_conn, embedder.clone()));
    assert!(
        embedder.calls().is_empty(),
        "S8: no session must not re-pay the archive owner's material: {:?}",
        embedder.calls()
    );
    assert_eq!(rows.len(), calls.len());
}

#[test]
fn a_paid_run_records_every_fragment_and_the_aggregate() {
    let archive = archive(true);
    let conn = archive.conn();
    set_meta(&conn, "account_id", "acct-A");
    set_legacy_channel(&conn);
    admit_embedding(&conn);
    let embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(&archive, &conn, embedder.clone()));
    let calls = embedder.calls();
    // Plan assumption S-e: the executor hands the embedder exactly the planned
    // fragment text, and the aggregate exactly the asset's full source text. No
    // instruction, task prefix, or wrapper is added, so the same
    // `(contract, sha256(text))` names the same vector in both domains.
    let (fragments, aggregate) = calls.split_at(calls.len() - 1);
    assert!(
        source_text().starts_with(&fragments[0]),
        "the first fragment starts the source text: no prefix"
    );
    assert!(
        source_text().ends_with(fragments.last().expect("fragments")),
        "the last fragment ends the source text: no suffix"
    );
    for fragment in fragments {
        assert!(
            source_text().contains(fragment.as_str()),
            "every fragment is a slice of the source text itself (the plan's chunks overlap)"
        );
    }
    assert_eq!(
        aggregate,
        [source_text()],
        "the aggregate embeds the full source text, verbatim"
    );
    let contract = effective_contract(&conn).hash;
    let stored = cache_accounts(&conn);
    let expected: HashSet<String> = calls.iter().map(|text| sha256_hex(text)).collect();
    assert_eq!(expected.len(), calls.len(), "distinct fragment texts");
    assert_eq!(
        stored
            .iter()
            .map(|row| row.2.clone())
            .collect::<HashSet<_>>(),
        expected,
        "one row per fragment's exact text and one for the aggregate"
    );
    for (account, row_contract, _, dimensions, origin, pushed) in &stored {
        assert_eq!(account, "acct-A", "stamped with the write account");
        assert_eq!(
            row_contract, &contract,
            "keyed by the effective contract hash"
        );
        assert_eq!(*dimensions, DIMENSIONS);
        assert_eq!(origin, "local");
        assert_eq!(*pushed, 0);
    }
    // The Bibliography's contract mirror never ran here (no
    // `bibliographic_embedding_contracts` row exists for Collections), so the
    // Collections writer must have mirrored the effective contract itself.
    let mirrored: (String, String, i64, String) = conn
        .query_row(
            "SELECT provider, model, dimensions, chunking_contract
               FROM sync_derived_contracts WHERE contract_hash = ?1",
            [&contract],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("mirrored contract row");
    let effective = effective_contract(&conn);
    assert_eq!(
        mirrored,
        (
            effective.provider,
            effective.model,
            effective.dimensions as i64,
            RAG_CHUNKING_CONTRACT.to_string()
        )
    );
}

#[test]
fn a_failing_cache_write_does_not_fail_the_publish() {
    let archive = archive(false);
    let conn = archive.conn();
    // The table exists (the writers see the full set) but its shape is stale,
    // so every insert fails. The publish must still land.
    conn.execute_batch(
        "CREATE TABLE sync_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE sync_derived_contracts (contract_hash TEXT PRIMARY KEY, provider TEXT NOT NULL, model TEXT NOT NULL, dimensions INTEGER NOT NULL, chunking_contract TEXT NOT NULL);
         CREATE TABLE sync_derived_ocr_pages (account_id TEXT NOT NULL);
         CREATE TABLE sync_derived_vectors (account_id TEXT NOT NULL, contract_hash TEXT NOT NULL, input_hash TEXT NOT NULL);",
    )
    .expect("stale cache tables");
    set_meta(&conn, "account_id", "acct-A");
    set_legacy_channel(&conn);
    admit_embedding(&conn);
    let embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(&archive, &conn, embedder.clone()));
    assert_eq!(embedder.calls().len(), published(&conn).chunks.len() + 1);
    let cached: i64 = conn
        .query_row("SELECT COUNT(*) FROM sync_derived_vectors", [], |row| {
            row.get(0)
        })
        .expect("cache count");
    assert_eq!(
        cached, 0,
        "the bad write is dropped, not retried into a lie"
    );
    // The write path was reached (the contract mirrored) and only the vector
    // insert failed; the publish still landed.
    let mirrored: i64 = conn
        .query_row("SELECT COUNT(*) FROM sync_derived_contracts", [], |row| {
            row.get(0)
        })
        .expect("contract mirror count");
    assert_eq!(mirrored, 1);
}

#[test]
fn without_the_derived_tables_the_publish_is_unchanged() {
    let baseline = archive(true);
    let baseline_conn = baseline.conn();
    admit_embedding(&baseline_conn);
    let baseline_embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(
        &baseline,
        &baseline_conn,
        baseline_embedder.clone(),
    ));

    let plain = archive(false);
    let plain_conn = plain.conn();
    admit_embedding(&plain_conn);
    let plain_embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(&plain, &plain_conn, plain_embedder.clone()));

    assert_eq!(
        plain_embedder.calls(),
        baseline_embedder.calls(),
        "same paid work as before the cache existed"
    );
    assert_eq!(
        published(&plain_conn),
        published(&baseline_conn),
        "published rows are identical with and without the derived tables"
    );
    let derived_tables: i64 = plain_conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'sync_derived_%'",
            [],
            |row| row.get(0),
        )
        .expect("derived table count");
    assert_eq!(derived_tables, 0);
}

/// T7 (§2.3b, §2.8): an archive that already embedded a Collection before the
/// channel existed exports its fragments and aggregate, and the keys it seeds are
/// exactly the ones a paid run asked for — a device that only has the cache then
/// finishes with zero `embed_text` calls.
#[test]
fn the_backfill_seeds_a_fresh_asset_so_a_fresh_device_pays_nothing() {
    // The owner paid before the derived cache existed: publish with no tables.
    let owner = archive(false);
    let owner_conn = owner.conn();
    admit_embedding(&owner_conn);
    let owner_embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(&owner, &owner_conn, owner_embedder.clone()));
    let paid_calls = owner_embedder.calls();
    let paid_rows = published(&owner_conn);
    assert!(paid_calls.len() >= 2, "fragments plus the aggregate");

    // The upgrade creates the tables; the owner's session seeds what it already
    // paid for.
    ensure_sync_schema(&owner_conn).expect("sync schema");
    set_meta(&owner_conn, "account_id", "acct-A");
    set_meta(&owner_conn, "derived_bound_account", "acct-A");
    set_meta(&owner_conn, "derived_owner_account", "acct-A");
    let report = run_backfill(&owner_conn);

    assert_eq!(report.status, BackfillStatus::Completed);
    assert_eq!(report.account.as_deref(), Some("acct-A"));
    assert_eq!(
        report.collection_assets, 1,
        "the asset is Fresh under the effective contract"
    );
    assert_eq!(
        report.collection_vectors,
        paid_calls.len(),
        "one fragment per paid chunk plus the aggregate"
    );
    assert_eq!(report.vectors, 0, "no bibliography vectors in this archive");
    assert_eq!(report.failures, 0);
    assert!(report.completed());

    // The seeded keys are the exact ones the paid run hashed.
    let mut seeded_keys: Vec<String> = cached_vectors(&owner_conn)
        .into_iter()
        .map(|(input_hash, _)| input_hash)
        .collect();
    let mut paid_keys: Vec<String> = paid_calls.iter().map(|text| sha256_hex(text)).collect();
    seeded_keys.sort();
    paid_keys.sort();
    assert_eq!(seeded_keys, paid_keys);

    // A fresh device of the same account: the catalog and the cache, no local
    // chunk set. It owes the chunk set and pays nothing for it.
    owner_conn
        .execute("DELETE FROM rag_chunks", [])
        .expect("fresh device has no local chunk set");
    assert_eq!(
        eligibility::embedding_decision(&owner_conn, ASSET).expect("decision"),
        EmbeddingDecision::Eligible {
            reason: EmbeddingStaleReason::IncompleteChunks
        }
    );
    admit_embedding(&owner_conn);
    let reader_embedder = Arc::new(CountingEmbedder::default());
    succeeded(&run_embedding(&owner, &owner_conn, reader_embedder.clone()));
    assert!(
        reader_embedder.calls().is_empty(),
        "the seeded cache must not call the provider: {:?}",
        reader_embedder.calls()
    );
    assert_eq!(
        published(&owner_conn),
        paid_rows,
        "the cached publish is byte-identical to the paid one"
    );
}

/// T7 (§2.3b): only an asset the archive confirms `Fresh` under the current
/// effective contract is seeded — anything else is a result a later run still has
/// to compute.
#[test]
fn the_backfill_does_not_seed_an_asset_that_is_not_fresh() {
    let owner = archive(false);
    let owner_conn = owner.conn();
    admit_embedding(&owner_conn);
    succeeded(&run_embedding(
        &owner,
        &owner_conn,
        Arc::new(CountingEmbedder::default()),
    ));
    // A chunk set that no longer covers its source: `IncompleteChunks`, not
    // `Fresh`, so part of the work is still owed and must not be pretended.
    owner_conn
        .execute(
            "DELETE FROM rag_chunks WHERE id = (SELECT id FROM rag_chunks WHERE asset_id = ?1 LIMIT 1)",
            [ASSET],
        )
        .expect("drop one stored fragment");
    assert!(matches!(
        eligibility::embedding_decision(&owner_conn, ASSET).expect("decision"),
        EmbeddingDecision::Eligible { .. }
    ));

    ensure_sync_schema(&owner_conn).expect("sync schema");
    set_meta(&owner_conn, "account_id", "acct-A");
    set_meta(&owner_conn, "derived_bound_account", "acct-A");
    set_meta(&owner_conn, "derived_owner_account", "acct-A");
    let report = run_backfill(&owner_conn);

    assert_eq!(report.status, BackfillStatus::Completed);
    assert_eq!(report.collection_assets, 0);
    assert_eq!(report.collection_vectors, 0);
    assert!(cached_vectors(&owner_conn).is_empty());
}

// ── T9: pull before pay (§2.6, D5) ─────────────────────────────────────────
//
// The Collections executor asks the gate after its cache read and only when a
// fragment (or the aggregate) is still missing, so a fully cached asset never
// waits and a parked one re-runs by itself without burning the retry budget.

fn run_embedding_at(
    archive: &Archive,
    conn: &Connection,
    embedder: Arc<CountingEmbedder>,
    now_ms: i64,
) -> RunOneOutcome {
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(EmbeddingExecutor::with_embedder(
        archive.path.clone(),
        embedder,
    )));
    run_one(
        conn,
        &ExecCtx {
            db_path: archive.path.clone(),
        },
        &registry,
        "embedding-session",
        now_ms,
        &|_, _| {},
        &|_, _, _, _| {},
    )
    .expect("embedding run")
}

/// S5: a session whose channel verdict is still unknown parks the asset before
/// its first paid `embed_text`; the channel's answer (here: a legacy server)
/// releases it, and the fragments are then paid exactly once.
#[test]
fn a_session_with_an_unknown_channel_parks_the_first_paid_fragment() {
    let archive = archive(true);
    let conn = archive.conn();
    set_meta(&conn, "account_id", "acct-A");
    admit_embedding(&conn);
    let embedder = Arc::new(CountingEmbedder::default());

    let task_id = match run_embedding(&archive, &conn, embedder.clone()) {
        RunOneOutcome::Deferred { task_id } => task_id,
        other => panic!("the first paid fragment must wait, got {other:?}"),
    };
    assert!(
        embedder.calls().is_empty(),
        "nothing paid before the account's cache is as complete as it can get"
    );
    let (state, code, retry_count): (String, Option<String>, i64) = conn
        .query_row(
            "SELECT state, last_error_code, retry_count FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("parked task");
    assert_eq!(state, "retry_wait");
    assert_eq!(code.as_deref(), Some("derived_cache_wait"));
    assert_eq!(retry_count, 0, "the wait costs no retry budget");
    let wake: i64 = conn
        .query_row(
            "SELECT next_retry_at FROM processing_tasks WHERE id = ?1",
            [&task_id],
            |row| row.get(0),
        )
        .expect("wake time");

    set_legacy_channel(&conn);
    succeeded(&run_embedding_at(
        &archive,
        &conn,
        embedder.clone(),
        wake + 1,
    ));
    assert_eq!(
        embedder.calls().len(),
        published(&conn).chunks.len() + 1,
        "every fragment plus the aggregate is paid exactly once after the wait"
    );
}

/// §2.6: an asset the cache fully answers never consults the wait, even with a
/// session whose channel is still unknown.
#[test]
fn a_fully_cached_asset_never_waits() {
    let (calls, rows) = paid_baseline();
    let archive = archive(true);
    let conn = archive.conn();
    set_meta(&conn, "account_id", "acct-A");
    let contract = effective_contract(&conn).hash;
    seed_vectors(&conn, "acct-A", &contract, &rows);
    admit_embedding(&conn);
    let embedder = Arc::new(CountingEmbedder::default());

    succeeded(&run_embedding(&archive, &conn, embedder.clone()));

    assert!(
        embedder.calls().is_empty(),
        "the cache answers every miss, so there is nothing to wait for: {:?}",
        embedder.calls()
    );
    assert_eq!(calls.len(), rows.len(), "the paid baseline seeded it all");
}
