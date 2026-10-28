//! Backfill of already-paid Bibliografía and Colecciones work into the derived
//! cache (§2.3b of `plan-sync-bibliografia-derivada`).
//!
//! §2.3 writes the cache when a device pays for something. That leaves the
//! archive the user already had out of it: a work whose profile did not change
//! is never re-admitted, a current extraction ends in the `already_current`
//! shortcut without a single page row, and a Collection is `Fresh` so nothing
//! ever re-demands it. Without a seed, nothing would export the OCR and vectors
//! that were paid before this channel existed and the next device would pay for
//! all of them again (S4). This module copies what is already stored into the
//! cache: pure local reads plus `origin='local'`, `pushed=0` rows and a
//! per-account/per-contract progress mark. No provider, no queue: the pass never
//! admits, claims, retries or settles a task.
//!
//! # Attribution — the owner only (§2.3b, §2.9)
//!
//! The source tables carry no `account_id` and survive logout, so seeding them
//! under whatever account happens to be logged in would hand one account's
//! private OCR to the next (S7). The pass therefore runs only while the session
//! `account_id` equals `sync_meta['derived_owner_account']` — the first account
//! ever bound to this archive — and stamps its rows with that account. Under any
//! other account it does nothing at all: that account only caches what it pays
//! from its own binding on. The progress mark
//! (`sync_meta['derived_backfill:<account>:<contract_hash>']`) describes local
//! rows, so logout keeps it: a re-login of the owner never re-reads the PDFs.
//!
//! # Sources (§2.3b)
//!
//! - `bibliographic_item_embeddings` and `bibliographic_chunk_embeddings`, keyed
//!   by the generation's `contract_hash` and the row's own `input_hash`. Chunk
//!   rows only where `input_hash = bibliographic_chunks.text_hash` (the
//!   `reusable_chunk_embeddings` condition): that hash is `sha256` of the exact
//!   text sent to the embedder, which is the key the reader looks up.
//! - **Colecciones** (§2.8): every asset whose `embedding_decision` is `Fresh`
//!   under the current effective contract. Its `rag_chunks` rows are seeded as
//!   fragments keyed by `sha256(text_content)`, and `vec_assets` as the asset
//!   aggregate keyed by `sha256` of the full source text — both built with the
//!   executor's own text construction (`processing::embedding::snapshot_sources`
//!   and `processing::embedding::aggregate_request_text`), so the keys are
//!   exactly the ones a T6 run looks up.
//! - `bibliographic_page_texts` with `method='ocr'`: the file hash is not
//!   stored, so the pass re-reads the local file, proves it is still the one the
//!   current extraction was read from (`extraction_matches_source`), and keys
//!   the answers under `sha256` of those bytes — the same key §2.3 uses. A
//!   missing file or a changed catalog `mtime`/byte count skips the attachment
//!   and logs it.
//! - retained `ocr-page:N` checkpoints of `failed`/`cancelled`
//!   `bibliography_extract` tasks whose payload is the empty answer: a blank
//!   page the owner paid for and no published row remembers. They are trusted
//!   only when the task's pinned file identity names the current catalog `mtime`
//!   — and, when an extraction exists, only if that extraction is current too.
//!
//! # The OCR answer a reader replays (L6)
//!
//! The cache stores the provider's RAW answer and the reader runs
//! `settled_page_row` on it (T3/T5). A page row in the archive, however, holds
//! the CONVERTED text: `ocr_markup_to_text` already ran on the answer, and it is
//! not idempotent (L6: `"<p>a &lt;b&gt; c</p>"` → `"a <b> c"` → `"a c"`). Storing
//! the row's text blindly would convert it a second time on every reader and
//! publish different text than this device. So a text page is seeded only when
//! the conversion is the identity on the stored text
//! (`ocr_markup_to_text(text) == text`): then a reader's replay reproduces the
//! stored row byte for byte, which is asserted by a test. A page whose text
//! would convert again is skipped and logged instead of guessed — an encoding
//! that survives `ocr_markup_to_text` for every reachable text does not exist
//! (`push_collapsed` rewrites the double spaces a Markdown table cell leaves),
//! and paying one page once more is the same bounded loss §2.3b already accepts
//! for answers that left no evidence.
//!
//! Empty answers (`''`, the `quality='empty'` row `settled_page_row` publishes
//! when OCR owns a garbled native page, and the retained empty checkpoints) are
//! seeded as `outcome='empty'`: a reader replays them against its own native
//! page exactly as it replays a fresh blank answer, so nobody pays for the page
//! again.
//!
//! # Cost: pages of reads, short write transactions (A1)
//!
//! The owner's archive holds on the order of 10^5 pasaje vectors. One autocommit
//! per row would mean one `fsync` per row under `synchronous=FULL` — minutes of
//! write lock competing with the scheduler — so the pass reads its sources in
//! keyset pages (500 rows at a time) and commits each page in ONE short
//! transaction. No write transaction is ever held across a PDF read, and
//! no cursor stays open while a write runs. Every write is still `INSERT OR
//! IGNORE`, so an interrupted pass resumes without duplicating a row, and the
//! mark is written only when no row failed.
//!
//! # Running it
//!
//! [`run_backfill`] is the whole pass: synchronous, idempotent and resumable. It
//! is cheap when there is nothing to do — a metadata read and the mark.
//! [`request_backfill`] is the background trigger: one low-priority thread with
//! its own connection, so startup, the UI and the batch scheduler never wait for
//! it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::bibliography::attachment::{
    attachment_ref_for, resolve_attachment_file, AttachmentResolution,
};
use crate::bibliography::processing::{production_page_ocr_contract, ZOTERO_DATA_DIR_SETTING_KEY};
use crate::bibliography::repository::extraction_matches_source;
use crate::ocr::markup::ocr_markup_to_text;
use crate::processing::eligibility::{
    embedding_decision, resolve_effective_embedding_contract, EffectiveEmbeddingContract,
    EmbeddingDecision,
};
use crate::processing::embedding::{aggregate_request_text, snapshot_sources};
use crate::processing::repository::attachment_extraction_identity_prefix;
use crate::sync::derived::{insert_ocr_page, insert_vector, BACKFILL_PREFIX};
use crate::sync::session::{meta_get, meta_set};

/// `sync_meta` key of the FIRST account ever bound to this archive (§2.9).
/// Written by `sync::derived::bind_account`; read here, never written.
const OWNER_ACCOUNT_KEY: &str = "derived_owner_account";
/// `sync_meta` key of the active session's account (§2.9).
const SESSION_ACCOUNT_KEY: &str = "account_id";
/// The value the mark carries once a pass finished: any other value (or none)
/// means the pass still owes rows.
const MARK_DONE: &str = "done";

/// Source rows one keyset page reads and one write transaction commits. 500
/// vectors of 1024 f32 are about 2 MB: small enough to hold, and short enough
/// that the write lock is taken per page instead of per row.
const VECTOR_PAGE: i64 = 500;
/// Asset rows one page of the Colecciones pass names. The page is only a list of
/// ids and their aggregate vectors; fragments are read per asset.
const ASSET_PAGE: i64 = 64;

/// Why [`run_backfill`] did or did not seed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillStatus {
    /// The pass ran to the end under the owner's session and set its mark
    /// (`failures` still reports rows a storage error kept out).
    Completed,
    /// A previous pass under this account and contract already seeded every
    /// source; nothing was read, no PDF was opened.
    AlreadySeeded,
    /// No account is logged in. The pass needs the owner's own session: with
    /// no session there is no way to prove the archive's content belongs to the
    /// account that would be stamped on the rows (§2.3b/S7).
    NoSession,
    /// The archive has no `derived_owner_account` mark, or the session account
    /// is not it. Nothing was written, under any account.
    NotOwner,
    /// The effective embedding contract cannot be named, so no mark and no key
    /// can be produced (`resolve_effective_embedding_contract` failed).
    NoContract,
    /// The derived tables do not exist on this connection yet.
    CacheUnavailable,
}

/// What one [`run_backfill`] pass observed. Counts are ROWS INSERTED: a key
/// that was already cached is not counted again, which is what makes a second
/// pass report zeros.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillReport {
    pub status: BackfillStatus,
    /// The session account the pass ran for, when one was known.
    pub account: Option<String>,
    /// The effective embedding contract hash the rows were keyed by.
    pub contract_hash: Option<String>,
    /// Bibliografía embedding rows seeded (ficha + pasaje).
    pub vectors: usize,
    /// Colecciones rows seeded (fragments + the asset aggregate).
    pub collection_vectors: usize,
    /// Colecciones assets visited with a `Fresh` decision and handed to the
    /// write path. Rows already cached are not counted in
    /// [`Self::collection_vectors`], so this stays non-zero on a repeat pass.
    pub collection_assets: usize,
    /// Stored vector rows that are unusable (wrong width, non-finite value,
    /// blank key) and were therefore not copied.
    pub vectors_skipped: usize,
    /// OCR answers seeded, text and empty alike.
    pub ocr_pages: usize,
    /// The subset of [`Self::ocr_pages`] whose outcome is `empty`.
    pub ocr_empty: usize,
    /// OCR pages that could not be represented for a reader (a stored text the
    /// conversion would change, a stale file identity) and were left out.
    pub ocr_skipped: usize,
    /// Attachments whose local file could not be read or proved current, so no
    /// page of them was seeded.
    pub attachments_skipped: usize,
    /// How many local files this pass read: `0` on a marked archive.
    pub files_read: usize,
    /// Rows a storage error kept out. A non-zero count withholds the mark so a
    /// later pass retries; the rows already seeded are kept (`INSERT OR IGNORE`).
    pub failures: usize,
}

impl BackfillReport {
    fn new(status: BackfillStatus) -> Self {
        Self {
            status,
            account: None,
            contract_hash: None,
            vectors: 0,
            collection_vectors: 0,
            collection_assets: 0,
            vectors_skipped: 0,
            ocr_pages: 0,
            ocr_empty: 0,
            ocr_skipped: 0,
            attachments_skipped: 0,
            files_read: 0,
            failures: 0,
        }
    }

    fn for_account(status: BackfillStatus, account: &str) -> Self {
        let mut report = Self::new(status);
        report.account = Some(account.to_string());
        report
    }

    /// True when the pass finished (marked or already marked) without a storage
    /// failure.
    pub fn completed(&self) -> bool {
        matches!(
            self.status,
            BackfillStatus::Completed | BackfillStatus::AlreadySeeded
        ) && self.failures == 0
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A `sync_meta` account value: present and not blank, exactly as the sync side
/// reads its marks (`''` never counts as an account).
fn meta_account(conn: &Connection, key: &str) -> Option<String> {
    meta_get(conn, key)
        .ok()
        .flatten()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// True when the connection carries the three derived tables. Mirrors the
/// private guard of `sync::derived`: the pass reports
/// [`BackfillStatus::CacheUnavailable`] once instead of failing row by row on an
/// archive whose bootstrap has not created them yet.
fn derived_tables_present(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
          WHERE type = 'table'
            AND name IN ('sync_derived_vectors', 'sync_derived_ocr_pages',
                         'sync_derived_contracts')",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count == 3)
    .unwrap_or(false)
}

/// Mirrors the Bibliography's immutable contract row into
/// `sync_derived_contracts` (§2.2), so the vectors seeded below travel with the
/// provider, model, width and chunking contract that name their space. The
/// statement is the one `sync::derived::cache_paid_vector` runs — unavailable
/// here because that helper also picks the account itself and this pass must
/// stamp the owner even if the session changes underneath it.
fn mirror_contract(conn: &Connection, contract_hash: &str, report: &mut BackfillReport) {
    if let Err(error) = conn.execute(
        "INSERT OR IGNORE INTO sync_derived_contracts
           (contract_hash, provider, model, dimensions, chunking_contract)
         SELECT contract_hash, provider, model, dimensions, chunking_contract
           FROM bibliographic_embedding_contracts WHERE contract_hash = ?1",
        [contract_hash],
    ) {
        log::warn!(
            target: "bibliography",
            "derived backfill could not mirror contract {contract_hash}: {error}"
        );
        report.failures += 1;
    }
}

/// Mirrors the effective contract for the Colecciones side (§2.8), from the
/// settings the pass already resolved. `processing::embedding` writes the same
/// row when it pays, so a Collection can be the first thing to name a space.
fn mirror_collection_contract(
    conn: &Connection,
    effective: &EffectiveEmbeddingContract,
    report: &mut BackfillReport,
) {
    if let Err(error) = conn.execute(
        "INSERT OR IGNORE INTO sync_derived_contracts
           (contract_hash, provider, model, dimensions, chunking_contract)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            effective.hash,
            effective.provider,
            effective.model,
            effective.dimensions as i64,
            crate::nlp::embeddings::RAG_CHUNKING_CONTRACT_V1,
        ],
    ) {
        log::warn!(
            target: "bibliography",
            "derived backfill could not mirror collection contract {}: {error}",
            effective.hash
        );
        report.failures += 1;
    }
}

/// A stored vector is copyable only when a reader could publish it unchanged:
/// the width the row declares, and finite f32 values (the same validation the
/// publisher applies and the cache reader repeats).
fn vector_is_usable(dimensions: i64, embedding: &[u8]) -> bool {
    if dimensions <= 0 || embedding.is_empty() || embedding.len() as i64 != dimensions * 4 {
        return false;
    }
    embedding
        .chunks_exact(4)
        .all(|value| f32::from_le_bytes([value[0], value[1], value[2], value[3]]).is_finite())
}

/// One vector on its way to the cache.
struct VectorRow {
    input_hash: String,
    dimensions: i64,
    embedding: Vec<u8>,
}

/// Writes one page of rows in ONE short transaction (§2.3b, A1): the write lock
/// is held for a page of inserts, never for a read or a PDF. Returns how many
/// rows were actually inserted (a key that was already cached is the same
/// content under the same account, so `INSERT OR IGNORE` keeps the first
/// writer's bytes and nothing is counted).
fn write_vector_page(
    conn: &Connection,
    account: &str,
    contract_hash: &str,
    rows: &[VectorRow],
    report: &mut BackfillReport,
) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let tx = match conn.unchecked_transaction() {
        Ok(tx) => tx,
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill could not open a vector write batch: {error}"
            );
            report.failures += 1;
            return 0;
        }
    };
    let mut inserted = 0;
    for row in rows {
        match insert_vector(
            &tx,
            account,
            contract_hash,
            &row.input_hash,
            row.dimensions,
            &row.embedding,
            "local",
        ) {
            Ok(()) if tx.changes() == 1 => inserted += 1,
            Ok(()) => {}
            Err(error) => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill skipped a vector of contract {contract_hash}: {error}"
                );
                report.failures += 1;
            }
        }
    }
    match tx.commit() {
        Ok(()) => inserted,
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill could not commit a vector batch: {error}"
            );
            report.failures += 1;
            0
        }
    }
}

// ── Bibliografía vectors (§2.3b) ────────────────────────────────────────────

const ITEM_VECTORS_PAGE_SQL: &str = "SELECT e.rowid, e.input_hash, e.dimensions, e.embedding
      FROM bibliographic_item_embeddings e
      JOIN bibliographic_index_generations g ON g.id = e.generation_id
     WHERE g.contract_hash = ?1 AND e.rowid > ?2
     ORDER BY e.rowid
     LIMIT ?3";

const CHUNK_VECTORS_PAGE_SQL: &str = "SELECT e.rowid, e.input_hash, e.dimensions, e.embedding
      FROM bibliographic_chunk_embeddings e
      JOIN bibliographic_index_generations g ON g.id = e.generation_id
      JOIN bibliographic_chunks c ON c.id = e.chunk_id
     WHERE g.contract_hash = ?1 AND e.rowid > ?2
       AND c.text_hash <> ''
       AND e.input_hash = c.text_hash
     ORDER BY e.rowid
     LIMIT ?3";

/// One keyset page of a stored vector source: the last rowid read and the rows a
/// reader could publish. `None` when the source has no more rows.
///
/// The statement is dropped before the caller opens its write transaction, so no
/// read cursor stays open while the page is written.
fn read_vector_page(
    conn: &Connection,
    contract_hash: &str,
    after_rowid: i64,
    sql: &str,
    report: &mut BackfillReport,
) -> Result<Option<(i64, Vec<VectorRow>)>, String> {
    let mut statement = conn
        .prepare(sql)
        .map_err(|error| format!("vector read failed: {error}"))?;
    let rows = statement
        .query_map(
            rusqlite::params![contract_hash, after_rowid, VECTOR_PAGE],
            |row| {
                // The rowid advances the keyset even when the payload cannot be
                // decoded, so a corrupt row can never trap the scan on one page.
                let rowid = row.get::<_, i64>(0)?;
                let payload = row.get::<_, String>(1).and_then(|input_hash| {
                    Ok((input_hash, row.get::<_, i64>(2)?, row.get::<_, Vec<u8>>(3)?))
                });
                Ok((rowid, payload))
            },
        )
        .map_err(|error| format!("vector read failed: {error}"))?;
    let mut last_rowid = after_rowid;
    let mut usable = Vec::new();
    let mut any = false;
    for row in rows {
        any = true;
        let (rowid, payload) = match row {
            Ok(row) => row,
            Err(error) => {
                log::warn!(target: "bibliography", "derived backfill vector row skipped: {error}");
                report.vectors_skipped += 1;
                continue;
            }
        };
        last_rowid = last_rowid.max(rowid);
        let (input_hash, dimensions, embedding) = match payload {
            Ok(payload) => payload,
            Err(error) => {
                log::warn!(target: "bibliography", "derived backfill vector row skipped: {error}");
                report.vectors_skipped += 1;
                continue;
            }
        };
        if input_hash.trim().is_empty() || !vector_is_usable(dimensions, &embedding) {
            report.vectors_skipped += 1;
            continue;
        }
        usable.push(VectorRow {
            input_hash,
            dimensions,
            embedding,
        });
    }
    Ok(any.then_some((last_rowid, usable)))
}

/// Copies one stored vector source (§2.3b), a page of rows at a time. The key is
/// already the reader's: a ficha row keeps the profile `input_hash` it was
/// computed for, and a pasaje row is kept only when its `input_hash` is the
/// chunk's own text hash (`sha256` of the exact text sent to the embedder).
fn seed_vector_source(
    conn: &Connection,
    account: &str,
    contract_hash: &str,
    sql: &str,
    report: &mut BackfillReport,
) {
    let mut after_rowid = 0_i64;
    loop {
        match read_vector_page(conn, contract_hash, after_rowid, sql, report) {
            Ok(Some((last_rowid, rows))) => {
                after_rowid = last_rowid;
                report.vectors += write_vector_page(conn, account, contract_hash, &rows, report);
            }
            Ok(None) => return,
            Err(error) => {
                log::warn!(target: "bibliography", "derived backfill vector read skipped: {error}");
                report.failures += 1;
                return;
            }
        }
    }
}

fn seed_vectors(
    conn: &Connection,
    account: &str,
    effective: &EffectiveEmbeddingContract,
    report: &mut BackfillReport,
) {
    mirror_contract(conn, &effective.hash, report);
    for sql in [ITEM_VECTORS_PAGE_SQL, CHUNK_VECTORS_PAGE_SQL] {
        seed_vector_source(conn, account, &effective.hash, sql, report);
    }
}

// ── Colecciones (§2.8, §2.3b) ───────────────────────────────────────────────

const ASSETS_PAGE_SQL: &str =
    "SELECT asset_id, embedding, dimensions, embedding_model, embedding_contract
       FROM vec_assets WHERE asset_id > ?1 ORDER BY asset_id LIMIT ?2";

/// One already-embedded asset's aggregate row, as `vec_assets` stores it.
struct StoredAsset {
    asset_id: String,
    embedding: Vec<u8>,
    dimensions: i64,
    embedding_model: String,
    embedding_contract: String,
}

/// One keyset page of embedded assets. `None` when there is no next page. The
/// statement is dropped before any per-asset read or write begins.
fn read_asset_page(
    conn: &Connection,
    after_asset: &str,
    report: &mut BackfillReport,
) -> Result<Option<(String, Vec<StoredAsset>)>, String> {
    let mut statement = conn
        .prepare(ASSETS_PAGE_SQL)
        .map_err(|error| format!("collection asset read failed: {error}"))?;
    let rows = statement
        .query_map(rusqlite::params![after_asset, ASSET_PAGE], |row| {
            // The id advances the keyset even when the payload cannot be decoded.
            let asset_id = row.get::<_, String>(0)?;
            let payload = row.get::<_, Vec<u8>>(1).and_then(|embedding| {
                Ok((
                    embedding,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            });
            Ok((asset_id, payload))
        })
        .map_err(|error| format!("collection asset read failed: {error}"))?;
    let mut last = after_asset.to_string();
    let mut assets = Vec::new();
    let mut any = false;
    for row in rows {
        any = true;
        let (asset_id, payload) = match row {
            Ok(row) => row,
            Err(error) => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill collection asset row skipped: {error}"
                );
                report.failures += 1;
                continue;
            }
        };
        last = asset_id.clone();
        let (embedding, dimensions, embedding_model, embedding_contract) = match payload {
            Ok(payload) => payload,
            Err(error) => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill collection asset {asset_id} skipped: {error}"
                );
                report.failures += 1;
                continue;
            }
        };
        assets.push(StoredAsset {
            asset_id,
            embedding,
            dimensions,
            embedding_model,
            embedding_contract,
        });
    }
    Ok(any.then_some((last, assets)))
}

/// The stored fragments of one asset, keyed exactly as the T6 executor hashed
/// them: `sha256` of each `rag_chunks.text_content`, the string handed to the
/// embedder (§2.8, assumption S-e).
fn read_collection_fragments(
    conn: &Connection,
    asset_id: &str,
    effective: &EffectiveEmbeddingContract,
    report: &mut BackfillReport,
) -> Option<Vec<VectorRow>> {
    let mut statement = match conn.prepare(
        "SELECT text_content, embedding, dimensions, embedding_model, embedding_contract
           FROM rag_chunks WHERE asset_id = ?1 ORDER BY chunk_ordinal, id",
    ) {
        Ok(statement) => statement,
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill could not read the fragments of {asset_id}: {error}"
            );
            report.failures += 1;
            return None;
        }
    };
    let rows = match statement.query_map([asset_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill could not read the fragments of {asset_id}: {error}"
            );
            report.failures += 1;
            return None;
        }
    };
    let mut fragments = Vec::new();
    for row in rows {
        let (text, embedding, dimensions, model, contract) = match row {
            Ok(row) => row,
            Err(error) => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill skipped a fragment of {asset_id}: {error}"
                );
                report.failures += 1;
                continue;
            }
        };
        // Only the space the archive currently names: a row of another contract,
        // model or width answers a different question (§2.4).
        if model != effective.model
            || contract != effective.contract
            || dimensions != effective.dimensions as i64
            || !vector_is_usable(dimensions, &embedding)
        {
            report.vectors_skipped += 1;
            continue;
        }
        if text.trim().is_empty() {
            report.vectors_skipped += 1;
            continue;
        }
        fragments.push(VectorRow {
            input_hash: sha256_hex(text.as_bytes()),
            dimensions,
            embedding,
        });
    }
    Some(fragments)
}

/// Seeds one `Fresh` asset's fragments and its aggregate (§2.8, §2.3b). Only an
/// asset the archive itself confirms complete under the current effective
/// contract is seeded: anything else is a result a later run still has to
/// compute, and seeding half of it would only fake a partial saving.
fn seed_collection_asset(
    conn: &Connection,
    account: &str,
    effective: &EffectiveEmbeddingContract,
    asset: &StoredAsset,
    report: &mut BackfillReport,
) {
    match embedding_decision(conn, &asset.asset_id) {
        Ok(EmbeddingDecision::Fresh) => {}
        Ok(_) => return,
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill skipped collection {}: {error}",
                asset.asset_id
            );
            report.failures += 1;
            return;
        }
    }
    if asset.embedding_model != effective.model
        || asset.embedding_contract != effective.contract
        || asset.dimensions != effective.dimensions as i64
    {
        return;
    }
    // A corrupt aggregate names a space nothing can be published from: leave the
    // asset out whole instead of faking a partial saving.
    if !vector_is_usable(asset.dimensions, &asset.embedding) {
        report.vectors_skipped += 1;
        return;
    }
    let sources = match snapshot_sources(conn, &asset.asset_id, "") {
        Ok(sources) => sources,
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill skipped collection {}: {error}",
                asset.asset_id
            );
            report.failures += 1;
            return;
        }
    };
    if sources.is_empty() {
        return;
    }
    let Some(fragments) = read_collection_fragments(conn, &asset.asset_id, effective, report)
    else {
        return;
    };
    // The aggregate answers the asset's full source text, joined exactly as the
    // executor joins it (§2.8, assumption S-e).
    let aggregate_text = aggregate_request_text(
        sources
            .iter()
            .map(|(_, _, text)| text.as_str())
            .collect::<Vec<_>>(),
    );
    let mut rows = fragments;
    rows.push(VectorRow {
        input_hash: sha256_hex(aggregate_text.as_bytes()),
        dimensions: asset.dimensions,
        embedding: asset.embedding.clone(),
    });
    mirror_collection_contract(conn, effective, report);
    let inserted = write_vector_page(conn, account, &effective.hash, &rows, report);
    report.collection_vectors += inserted;
    report.collection_assets += 1;
}

/// Seeds every `Fresh` Colecciones asset (§2.8): its fragments and its aggregate
/// under `(effective contract hash, sha256 of the exact text)`. Assets are
/// visited a keyset page at a time, so the pass never holds a scan open across
/// the transactions it writes.
fn seed_collections(
    conn: &Connection,
    account: &str,
    effective: &EffectiveEmbeddingContract,
    report: &mut BackfillReport,
) {
    let mut after_asset = String::new();
    loop {
        let page = match read_asset_page(conn, &after_asset, report) {
            Ok(page) => page,
            Err(error) => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill collection read skipped: {error}"
                );
                report.failures += 1;
                return;
            }
        };
        let Some((last_asset, assets)) = page else {
            return;
        };
        after_asset = last_asset;
        for asset in &assets {
            seed_collection_asset(conn, account, effective, asset, report);
        }
    }
}

// ── OCR pages (§2.3b) ───────────────────────────────────────────────────────

/// One attachment's OCR evidence: which local sources name it, so the pass can
/// visit it once and read a single attachment's pages at a time. The page texts
/// themselves are never held for the whole archive — an archive can carry tens
/// of thousands of OCR pages.
#[derive(Default)]
struct AttachmentOcrWork {
    /// The attachment has published `method='ocr'` rows.
    has_published_pages: bool,
    /// Pages whose paid blank answer survives only as a task checkpoint.
    empty_checkpoint_pages: BTreeSet<i64>,
}

/// The attachments with published OCR rows. Ids only: their pages are read one
/// attachment at a time by [`published_pages_for`].
const PUBLISHED_OCR_ATTACHMENTS_SQL: &str =
    "SELECT DISTINCT attachment_id FROM bibliographic_page_texts WHERE method = 'ocr'";

/// Retained blank answers (§2.3b): a `failed`/`cancelled` extraction keeps its
/// `ocr-page:N` checkpoints, and an empty payload is the JSON empty string
/// `PageOcrOutcome::Text("")` serializes to — a page the owner paid for that no
/// published row remembers. The join on the task's own fingerprint and contract
/// is the one `adopt_reprocess_checkpoints` uses, so a checkpoint of a different
/// file or plan is never mistaken for this one.
const RETAINED_EMPTY_ANSWERS_SQL: &str =
    "SELECT t.subject_id, t.input_fingerprint, a.mtime, c.unit_key
      FROM processing_tasks t
      JOIN zotero_attachments a ON a.id = t.subject_id
      JOIN processing_checkpoints c ON c.task_id = t.id
     WHERE t.domain = 'bibliography'
       AND t.subject_kind = 'attachment'
       AND t.kind = 'bibliography_extract'
       AND t.state IN ('failed', 'cancelled')
       AND c.unit_key LIKE 'ocr-page:%'
       AND c.payload = '\"\"'
       AND c.input_fingerprint = t.input_fingerprint
       AND c.contract_hash = t.contract_hash";

/// `ocr-page:12` → `12`, anything else → `None`.
fn checkpoint_page(unit_key: &str) -> Option<i64> {
    unit_key
        .strip_prefix("ocr-page:")
        .and_then(|page| page.parse::<i64>().ok())
        .filter(|page| *page >= 1)
}

/// Every attachment with OCR evidence, from both local sources.
fn gather_ocr_work(
    conn: &Connection,
    report: &mut BackfillReport,
) -> BTreeMap<String, AttachmentOcrWork> {
    let mut work: BTreeMap<String, AttachmentOcrWork> = BTreeMap::new();

    let mut published = match conn.prepare(PUBLISHED_OCR_ATTACHMENTS_SQL) {
        Ok(statement) => statement,
        Err(error) => {
            log::warn!(target: "bibliography", "derived backfill OCR read skipped: {error}");
            report.failures += 1;
            return work;
        }
    };
    let rows = match published.query_map([], |row| row.get::<_, String>(0)) {
        Ok(rows) => rows,
        Err(error) => {
            log::warn!(target: "bibliography", "derived backfill OCR read skipped: {error}");
            report.failures += 1;
            return work;
        }
    };
    for row in rows {
        match row {
            Ok(attachment_id) => {
                work.entry(attachment_id).or_default().has_published_pages = true;
            }
            Err(error) => {
                log::warn!(target: "bibliography", "derived backfill OCR row skipped: {error}");
                report.failures += 1;
            }
        }
    }

    let mut retained = match conn.prepare(RETAINED_EMPTY_ANSWERS_SQL) {
        Ok(statement) => statement,
        Err(error) => {
            log::warn!(target: "bibliography", "derived backfill checkpoint read skipped: {error}");
            report.failures += 1;
            return work;
        }
    };
    let rows = match retained.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, String>(3)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(error) => {
            log::warn!(target: "bibliography", "derived backfill checkpoint read skipped: {error}");
            report.failures += 1;
            return work;
        }
    };
    for row in rows {
        let (attachment_id, fingerprint, mtime, unit_key) = match row {
            Ok(row) => row,
            Err(error) => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill checkpoint row skipped: {error}"
                );
                report.failures += 1;
                continue;
            }
        };
        let Some(page_number) = checkpoint_page(&unit_key) else {
            continue;
        };
        // The answer belongs to the file the task pinned. A metadata edit between
        // attempts changes the catalog mtime and the prefix stops matching, so a
        // blank answer paid for a previous file is never installed for this one.
        if !fingerprint.starts_with(&attachment_extraction_identity_prefix(
            &attachment_id,
            mtime,
        )) {
            continue;
        }
        work.entry(attachment_id)
            .or_default()
            .empty_checkpoint_pages
            .insert(page_number);
    }

    work
}

/// One attachment's published `ocr` page rows: page → (stored text, stored text
/// hash). Read per attachment so the pass never holds the archive's whole OCR
/// corpus in memory.
fn published_pages_for(
    conn: &Connection,
    attachment_id: &str,
    report: &mut BackfillReport,
) -> Vec<(i64, String, String)> {
    let mut statement = match conn.prepare(
        "SELECT page_number, text_content, text_hash
           FROM bibliographic_page_texts
          WHERE attachment_id = ?1 AND method = 'ocr'
          ORDER BY page_number",
    ) {
        Ok(statement) => statement,
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill could not read the OCR pages of {attachment_id}: {error}"
            );
            report.failures += 1;
            return Vec::new();
        }
    };
    let rows = match statement.query_map([attachment_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill could not read the OCR pages of {attachment_id}: {error}"
            );
            report.failures += 1;
            return Vec::new();
        }
    };
    rows.filter_map(|row| match row {
        Ok(row) => Some(row),
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill skipped an OCR row of {attachment_id}: {error}"
            );
            report.failures += 1;
            None
        }
    })
    .collect()
}

/// True when the catalog holds an extraction row for the attachment. Read as a
/// bare `COUNT`: the pass never needs the text, and loading a book-sized
/// `text_content` just to learn the row exists would cost more than the whole
/// seed of its pages.
fn extraction_exists(conn: &Connection, attachment_id: &str, report: &mut BackfillReport) -> bool {
    match conn.query_row(
        "SELECT COUNT(*) FROM bibliographic_extractions WHERE attachment_id = ?1",
        [attachment_id],
        |row| row.get::<_, i64>(0),
    ) {
        Ok(count) => count > 0,
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill could not read the extraction of {attachment_id}: {error}"
            );
            report.failures += 1;
            false
        }
    }
}

/// One answer to seed: the page, what the provider answered and its text.
struct OcrAnswer {
    page_number: i64,
    outcome: &'static str,
    text: String,
}

/// The cache identity of one attachment's OCR answers: the account and file
/// every row is keyed by, under the contract the production page path stamps
/// today.
struct OcrSeed<'a> {
    account: &'a str,
    source_hash: &'a str,
    ocr_contract: &'a str,
    answers: Vec<OcrAnswer>,
}

impl<'a> OcrSeed<'a> {
    fn new(account: &'a str, source_hash: &'a str, ocr_contract: &'a str) -> Self {
        OcrSeed {
            account,
            source_hash,
            ocr_contract,
            answers: Vec::new(),
        }
    }

    /// Queues one published `method='ocr'` row.
    ///
    /// The row's text is the CONVERTED answer, so it can only be stored as the
    /// cache's raw answer when `ocr_markup_to_text` leaves it alone: a reader
    /// then replays `settled_page_row` and publishes exactly this row (same
    /// text, hash, quality and length). Anything else is skipped and logged —
    /// see the module docs for why no escaping trick covers every reachable
    /// text.
    fn push_published(
        &mut self,
        page_number: i64,
        text: &str,
        stored_hash: &str,
        report: &mut BackfillReport,
    ) {
        if text.trim().is_empty() {
            // The blank answer: `settled_page_row` publishes an empty `ocr` row
            // only when OCR owns a garbled native page, and a reader replays this
            // against its own native page exactly like a fresh blank answer.
            self.push_empty(page_number);
            return;
        }
        if ocr_markup_to_text(text) != text || stored_hash != sha256_hex(text.as_bytes()) {
            log::warn!(
                target: "bibliography",
                "derived backfill left OCR page {page_number} of file {} out: its stored text does not replay unchanged",
                self.source_hash
            );
            report.ocr_skipped += 1;
            return;
        }
        self.answers.push(OcrAnswer {
            page_number,
            outcome: "text",
            text: text.to_string(),
        });
    }

    /// Queues a page whose paid answer was blank: `outcome='empty'`, no text.
    fn push_empty(&mut self, page_number: i64) {
        self.answers.push(OcrAnswer {
            page_number,
            outcome: "empty",
            text: String::new(),
        });
    }

    /// Writes every queued answer of this attachment in ONE short transaction
    /// (A1). Counts only rows actually inserted, so a repeated pass reports what
    /// it added (nothing), not what it re-visited.
    fn commit(self, conn: &Connection, report: &mut BackfillReport) {
        if self.answers.is_empty() {
            return;
        }
        let tx = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(error) => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill could not open an OCR write batch: {error}"
                );
                report.failures += 1;
                return;
            }
        };
        let mut pages = 0;
        let mut empty = 0;
        for answer in &self.answers {
            match insert_ocr_page(
                &tx,
                self.account,
                self.source_hash,
                answer.page_number,
                self.ocr_contract,
                answer.outcome,
                &answer.text,
                "local",
            ) {
                Ok(()) if tx.changes() == 1 => {
                    pages += 1;
                    if answer.outcome == "empty" {
                        empty += 1;
                    }
                }
                Ok(()) => {}
                Err(error) => {
                    log::warn!(
                        target: "bibliography",
                        "derived backfill skipped OCR page {}: {error}",
                        answer.page_number
                    );
                    report.failures += 1;
                }
            }
        }
        match tx.commit() {
            Ok(()) => {
                report.ocr_pages += pages;
                report.ocr_empty += empty;
            }
            Err(error) => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill could not commit an OCR batch: {error}"
                );
                report.failures += 1;
            }
        }
    }
}

/// Seeds the OCR evidence of the archive (§2.3b): the published `ocr` pages of
/// every attachment whose file still matches its extraction, plus the blank
/// answers retained only as task checkpoints.
///
/// Each attachment is: read its pages, resolve and read its file (no transaction
/// open), then commit all of its answers in one short transaction.
fn seed_ocr(conn: &Connection, account: &str, report: &mut BackfillReport) {
    let work = gather_ocr_work(conn, report);
    if work.is_empty() {
        return;
    }
    let ocr_contract = production_page_ocr_contract();
    let data_dir = crate::settings::get_setting(conn, ZOTERO_DATA_DIR_SETTING_KEY);
    for (attachment_id, work) in work {
        // The pages of THIS attachment only: an archive can hold tens of
        // thousands of OCR pages, and nothing here may scale with the whole
        // corpus in memory.
        let published = if work.has_published_pages {
            published_pages_for(conn, &attachment_id, report)
        } else {
            Vec::new()
        };
        let evidence_pages = published.len() + work.empty_checkpoint_pages.len();
        let attachment = match attachment_ref_for(conn, &attachment_id) {
            Ok(Some(attachment)) => attachment,
            Ok(None) => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill skipped OCR of {attachment_id}: the catalog no longer holds it"
                );
                report.attachments_skipped += 1;
                report.ocr_skipped += evidence_pages;
                continue;
            }
            Err(error) => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill skipped OCR of {attachment_id}: {error}"
                );
                report.failures += 1;
                continue;
            }
        };
        let path = match resolve_attachment_file(&attachment, data_dir.as_deref()) {
            AttachmentResolution::File(path) => path,
            AttachmentResolution::Unavailable { reason, detail } => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill skipped OCR of {attachment_id}: {reason}: {detail}"
                );
                report.attachments_skipped += 1;
                report.ocr_skipped += evidence_pages;
                continue;
            }
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!(
                    target: "bibliography",
                    "derived backfill skipped OCR of {attachment_id}: {} could not be read: {error}",
                    path.display()
                );
                report.attachments_skipped += 1;
                report.ocr_skipped += evidence_pages;
                continue;
            }
        };
        report.files_read += 1;
        let file_bytes = bytes.len() as i64;
        // §2.3b: the hash is only the file's if the file is still the one the
        // current extraction was read from. `mtime` and byte count are all the
        // catalog preserved, and they are what the admission gate trusts too.
        let matches_source =
            match extraction_matches_source(conn, &attachment_id, attachment.mtime, file_bytes) {
                Ok(matches) => matches,
                Err(error) => {
                    log::warn!(
                        target: "bibliography",
                        "derived backfill skipped OCR of {attachment_id}: {}: {}",
                        error.code,
                        error.message
                    );
                    report.failures += 1;
                    continue;
                }
            };
        // A published row proves the extraction; a checkpoint needs no
        // extraction at all (a failed first run never published one), so it is
        // trusted whenever no extraction disagrees with the file.
        let seed_checkpoints = matches_source || !extraction_exists(conn, &attachment_id, report);
        if !matches_source && !seed_checkpoints {
            log::warn!(
                target: "bibliography",
                "derived backfill skipped OCR of {attachment_id}: the file no longer matches its extraction"
            );
            report.attachments_skipped += 1;
            report.ocr_skipped += evidence_pages;
            continue;
        }
        let source_hash = sha256_hex(&bytes);
        let mut seed = OcrSeed::new(account, &source_hash, &ocr_contract);
        if matches_source {
            for (page_number, text, stored_hash) in &published {
                seed.push_published(*page_number, text, stored_hash, report);
            }
        } else {
            log::warn!(
                target: "bibliography",
                "derived backfill left {} OCR page(s) of {attachment_id} out: the file no longer matches its extraction",
                published.len()
            );
            report.ocr_skipped += published.len();
        }
        for page_number in &work.empty_checkpoint_pages {
            seed.push_empty(*page_number);
        }
        seed.commit(conn, report);
    }
}

/// Seeds everything the archive already paid for into the derived cache (§2.3b)
/// and returns what it observed.
///
/// Local and free: it reads catalog rows and files and writes only
/// `sync_derived_*` rows and its own `sync_meta` mark — never a task, an
/// attempt, a checkpoint or a provider call. Safe to call repeatedly: a finished
/// pass reports [`BackfillStatus::AlreadySeeded`] after a metadata read, and a
/// pass interrupted halfway resumes because every write is `INSERT OR IGNORE`.
pub fn run_backfill(conn: &Connection) -> BackfillReport {
    let Some(session) = meta_account(conn, SESSION_ACCOUNT_KEY) else {
        return BackfillReport::new(BackfillStatus::NoSession);
    };
    let Some(owner) = meta_account(conn, OWNER_ACCOUNT_KEY) else {
        // No account was ever bound to this archive: there is no owner whose
        // rows these are, and seeding under the session account would hand one
        // account's paid OCR to another (§2.3b/S7).
        return BackfillReport::for_account(BackfillStatus::NotOwner, &session);
    };
    if owner != session {
        return BackfillReport::for_account(BackfillStatus::NotOwner, &session);
    }
    if !derived_tables_present(conn) {
        return BackfillReport::for_account(BackfillStatus::CacheUnavailable, &owner);
    }
    let effective = match resolve_effective_embedding_contract(conn) {
        Ok(effective) => effective,
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill cannot name the effective embedding contract: {error}"
            );
            return BackfillReport::for_account(BackfillStatus::NoContract, &owner);
        }
    };
    let mark_key = format!("{BACKFILL_PREFIX}{owner}:{}", effective.hash);
    match meta_get(conn, &mark_key) {
        Ok(Some(value)) if value.trim() == MARK_DONE => {
            let mut report = BackfillReport::for_account(BackfillStatus::AlreadySeeded, &owner);
            report.contract_hash = Some(effective.hash);
            return report;
        }
        Ok(_) => {}
        Err(error) => {
            log::warn!(
                target: "bibliography",
                "derived backfill could not read its mark {mark_key}: {error}"
            );
        }
    }

    let mut report = BackfillReport::for_account(BackfillStatus::Completed, &owner);
    report.contract_hash = Some(effective.hash.clone());
    seed_vectors(conn, &owner, &effective, &mut report);
    seed_collections(conn, &owner, &effective, &mut report);
    seed_ocr(conn, &owner, &mut report);
    if report.failures == 0 {
        if let Err(error) = meta_set(conn, &mark_key, MARK_DONE) {
            log::warn!(
                target: "bibliography",
                "derived backfill kept its work but could not set {mark_key}: {error}"
            );
            report.failures += 1;
        }
    } else {
        log::warn!(
            target: "bibliography",
            "derived backfill left its mark unset after {} storage failure(s): the next pass retries them",
            report.failures
        );
    }
    log::info!(
        target: "bibliography",
        "derived backfill for contract {}: {} bibliografía vector(s), {} colección vector(s) over {} asset(s) ({} row(s) skipped), {} OCR page(s) ({} empty, {} skipped), {} attachment(s) skipped, {} file(s) read",
        effective.hash.chars().take(12).collect::<String>(),
        report.vectors,
        report.collection_vectors,
        report.collection_assets,
        report.vectors_skipped,
        report.ocr_pages,
        report.ocr_empty,
        report.ocr_skipped,
        report.attachments_skipped,
        report.files_read,
    );
    report
}

/// True while a [`request_backfill`] thread is alive. Only guards concurrent
/// requests: the durable mark, not this flag, decides whether a pass has work
/// left.
static BACKFILL_RUNNING: AtomicBool = AtomicBool::new(false);
/// A request that arrived while a pass was alive. Startup and a login binding
/// can overlap, and a pass that started before the session existed answers
/// [`BackfillStatus::NoSession`]; without this flag that binding's request would
/// be dropped and the archive would only seed on the next start or login.
static BACKFILL_QUEUED: AtomicBool = AtomicBool::new(false);

/// Low-priority trigger: runs [`run_backfill`] on a background thread with its
/// own archive connection and returns immediately, so startup and the UI never
/// wait for the PDF reads.
///
/// A request that arrives while a pass is alive is coalesced into one more pass
/// on the same thread instead of being dropped. A pass with nothing to do costs
/// a metadata read and the mark, so extra requests are harmless; the thread
/// never runs away: it stops as soon as no request is pending. A request that
/// finds the archive busy (another writer holds the connection's lock) simply
/// logs and returns; the mark is still unset, so the next request retries.
pub fn request_backfill(db_path: std::path::PathBuf) {
    if BACKFILL_RUNNING.swap(true, Ordering::SeqCst) {
        BACKFILL_QUEUED.store(true, Ordering::SeqCst);
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("derived-backfill".to_string())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let conn = match crate::db::open::open_archive_connection(Path::new(&db_path)) {
                    Ok(conn) => conn,
                    Err(error) => {
                        log::warn!(
                            target: "bibliography",
                            "derived backfill could not open the archive: {error}"
                        );
                        return;
                    }
                };
                loop {
                    let report = run_backfill(&conn);
                    log::info!(
                        target: "bibliography",
                        "derived backfill finished: {:?}",
                        report.status
                    );
                    if !BACKFILL_QUEUED.swap(false, Ordering::SeqCst) {
                        break;
                    }
                }
            }));
            BACKFILL_RUNNING.store(false, Ordering::SeqCst);
            if result.is_err() {
                log::warn!(
                    target: "bibliography",
                    "derived backfill panicked; the archive keeps its mark unset"
                );
            }
        });
    if let Err(error) = spawned {
        BACKFILL_RUNNING.store(false, Ordering::SeqCst);
        log::warn!(
            target: "bibliography",
            "derived backfill thread could not start: {error}"
        );
    }
}
