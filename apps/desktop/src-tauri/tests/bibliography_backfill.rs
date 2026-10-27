//! T7: backfill of already-paid results into the derived cache (§2.3b of
//! `plan-sync-bibliografia-derivada`, specs S4/S7).
//!
//! What they pin, one test each:
//!
//! - an already-processed archive (unchanged profile, current extraction) hands
//!   its ficha/pasaje vectors and its OCR pages — the `quality='empty'` one as
//!   `outcome='empty'` — to the cache with the owner's session, `origin='local'`
//!   and `pushed=0`, without readmitting a single task (S4);
//! - a file whose catalog `mtime` moved, or whose bytes changed, is skipped page
//!   by page and logged, never guessed;
//! - a second pass adds nothing and opens no local file: the per-account mark is
//!   honored (S4);
//! - A logs out and B logs in: the pass does not run for B, B gets no row at or
//!   under its account, and A's rows stay A's; back as A, the pass does not
//!   re-read the PDFs (S7);
//! - a reader replaying a backfilled OCR answer through `settled_page_row`
//!   publishes the very row the archive stored, and a page whose stored text
//!   would convert again is skipped instead of guessed (L6);
//! - published bibliography rows survive the pass untouched.
//!
//! The synthetic "PDF" is arbitrary bytes on purpose: the pass never parses a
//! file. It hashes the bytes the catalog's extraction identity pins (§2.3b), so
//! a payload that is not a real PDF exercises exactly what this slice does.

use std::path::PathBuf;

use entropia_desktop_lib::bibliography::chunks::chunk_hash;
use entropia_desktop_lib::bibliography::derived_backfill::{run_backfill, BackfillStatus};
use entropia_desktop_lib::bibliography::generation::{
    ensure_staging_generation_for_contract, EmbeddingContractRow,
};
use entropia_desktop_lib::bibliography::processing::{
    extraction_quality, production_page_ocr_contract, settled_page_row, ExtractPageText,
};
use entropia_desktop_lib::bibliography::profile::profile_input_hash;
use entropia_desktop_lib::bibliography::repository as catalog;
use entropia_desktop_lib::processing::eligibility::resolve_effective_embedding_contract;
use entropia_desktop_lib::processing::repository as processing_repository;
use entropia_desktop_lib::sync::schema::ensure_sync_schema;
use entropia_desktop_lib::sync::session::{clear_sync_state, meta_get, meta_set};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// The real application schema (every migration + the programmatic layouts),
/// exported from the store package — the same fixture the sync tests use.
const SCHEMA_FIXTURE: &str = include_str!("fixtures/schema_full.sql");

const ACCOUNT_A: &str = "account-a";
const ACCOUNT_B: &str = "account-b";
/// The catalog mtime the seeded attachment pins: the same value the extraction
/// stores as `source_mtime`, which is how `extraction_matches_source` proves the
/// file is still the one that was read.
const MTIME: i64 = 1_700_000_000;
/// `RAG_CHUNKING_CONTRACT_V1`, the chunking half of the effective contract.
const RAG_CHUNKING_CONTRACT: &str = "rag-chunk-800-100-char-v1";
const NATIVE_TEXT: &str = "The quiet reading room holds every book the city ever loved.";
const OCR_TEXT: &str =
    "Texto reconocido de la pagina dos, con palabras legibles y espacios normales.";
/// The profile text the stored ficha vector was computed for: the cache key has
/// to be the hash a reader rebuilds from the catalog (`profile_input_hash`).
const PROFILE_TEXT: &str = "Titulo: La biblioteca silenciosa\nAutoria: Ada Lovelace";
const CHUNK_TEXT: &str = "Un pasaje legible que el perfil troceo y que se pago una sola vez.";
/// A converted OCR answer that still LOOKS like markup: `settled_page_row` would
/// convert it again (L6), so the backfill must leave it out.
const STORED_TEXT_THAT_CONVERTS_AGAIN: &str = "a <b> c";

fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// One float vector as the publisher stores it: f32 little-endian, exactly
/// `dimensions * 4` bytes.
fn vector_blob(dimensions: i64, seed: f32) -> Vec<u8> {
    let mut blob = Vec::with_capacity((dimensions * 4) as usize);
    for index in 0..dimensions {
        blob.extend_from_slice(&(seed + index as f32 * 0.25).to_le_bytes());
    }
    blob
}

/// Everything one seeded archive exposes to a test. Holding the `TempDir` keeps
/// the archive and its synthetic attachment file alive for the test.
struct Seeded {
    dir: tempfile::TempDir,
    item_id: String,
    attachment_id: String,
    file: PathBuf,
    source_bytes: Vec<u8>,
    contract_hash: String,
    dimensions: i64,
    item_input_hash: String,
    chunk_input_hash: String,
}

impl Seeded {
    fn source_hash(&self) -> String {
        sha256_hex(&self.source_bytes)
    }
}

/// An archive that already paid for its work before the channel existed: a
/// current extraction with an `ocr` page (text) and a `quality='empty'` one, a
/// ficha vector and a pasaje vector, and — deliberately — a chunk whose
/// `input_hash` is NOT its text hash, which §2.3b says must not be seeded.
fn seeded_archive() -> (Seeded, Connection) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("entropia.sqlite");
    let mut conn = entropia_desktop_lib::db_open_for_tests(&path);
    conn.execute_batch(SCHEMA_FIXTURE).expect("schema fixture");
    ensure_sync_schema(&conn).expect("sync schema");

    // The bytes the extraction was read from. Not a real PDF: the pass hashes
    // them and never parses them.
    let source_bytes = b"%PDF-1.4 synthetic scan 0000000000".to_vec();
    let file = dir.path().join("paper.pdf");
    std::fs::write(&file, &source_bytes).expect("write attachment file");

    let source = catalog::upsert_connection(
        &mut conn,
        catalog::UpsertConnection {
            id: "conn-1".to_string(),
            source_origin: catalog::SourceOrigin::Local,
            source_instance_id: None,
            endpoint: Some("http://synthetic.invalid".to_string()),
            capabilities_json: r#"{"read":true}"#.to_string(),
        },
    )
    .expect("seed connection");
    let library = catalog::upsert_library(
        &mut conn,
        catalog::UpsertLibrary {
            connection_id: source.id,
            library_type: catalog::LibraryType::User,
            library_id: "0".to_string(),
            name: "Personal".to_string(),
            last_modified_version: Some(7),
        },
    )
    .expect("seed library");
    let item = catalog::upsert_item(
        &mut conn,
        &library.id,
        catalog::BibliographicItemInput {
            item_key: "ITEM1".to_string(),
            item_version: Some(3),
            native_json_snapshot: serde_json::json!({ "key": "ITEM1", "itemType": "book" })
                .to_string(),
            csl_json_snapshot: serde_json::json!({ "id": "ITEM1", "type": "book" }).to_string(),
            title: Some("La biblioteca silenciosa".to_string()),
            ..Default::default()
        },
    )
    .expect("seed item");
    let attachment = catalog::upsert_attachment(
        &mut conn,
        &item.id,
        catalog::AttachmentInput {
            attachment_key: "ATT1".to_string(),
            content_type: Some("application/pdf".to_string()),
            link_mode: Some("linked_file".to_string()),
            filename: Some("paper.pdf".to_string()),
            native_path: Some(file.to_string_lossy().into_owned()),
            url: None,
            md5: None,
            mtime: Some(MTIME),
            native_json_snapshot: serde_json::json!({
                "key": "ATT1", "itemType": "attachment", "linkMode": "linked_file",
                "contentType": "application/pdf",
            })
            .to_string(),
            native_version: Some(3),
        },
    )
    .expect("seed attachment");

    // The extraction the pages belong to: current, `rich` native text.
    catalog::upsert_extraction_in_transaction(
        &conn,
        &catalog::ExtractionRow {
            attachment_id: attachment.id.clone(),
            item_id: item.id.clone(),
            page_count: 3,
            method: "native".to_string(),
            text_content: NATIVE_TEXT.to_string(),
            text_hash: sha256_hex(NATIVE_TEXT.as_bytes()),
            text_chars: NATIVE_TEXT.chars().count() as i64,
            quality: extraction_quality(NATIVE_TEXT).to_string(),
            source_mtime: Some(MTIME),
            source_bytes: source_bytes.len() as i64,
        },
        7,
    )
    .expect("plant extraction");
    plant_page(&conn, &attachment.id, 1, "native", NATIVE_TEXT);
    plant_page(&conn, &attachment.id, 2, "ocr", OCR_TEXT);
    plant_page(&conn, &attachment.id, 3, "ocr", "");

    // The embedding space the archive paid under, and the rows it kept.
    let effective = resolve_effective_embedding_contract(&conn).expect("effective contract");
    let dimensions = effective.dimensions as i64;
    let generation = ensure_staging_generation_for_contract(
        &conn,
        &EmbeddingContractRow {
            contract_hash: effective.hash.clone(),
            provider: effective.provider.clone(),
            model: effective.model.clone(),
            dimensions,
            chunking_contract: RAG_CHUNKING_CONTRACT.to_string(),
        },
        7,
    )
    .expect("staging generation");

    let item_input_hash = profile_input_hash(PROFILE_TEXT);
    catalog::upsert_item_embedding_in_transaction(
        &conn,
        &catalog::ItemEmbeddingRow {
            item_id: item.id.clone(),
            generation_id: generation.id.clone(),
            embedding_contract: effective.contract.clone(),
            embedding_model: effective.model.clone(),
            dimensions: dimensions as usize,
            embedding: vector_blob(dimensions, 0.5),
            input_hash: item_input_hash.clone(),
            profile_revision: 1,
        },
        7,
    )
    .expect("plant ficha vector");

    let chunk_input_hash = chunk_hash(CHUNK_TEXT);
    let decoy_text = "Otro pasaje cuyo hash guardado no es el de su texto.";
    // Its vector IS keyed by the hash of its text, but the chunk row kept an older
    // hash: `input_hash = chunks.text_hash` fails, which is exactly the §2.3b
    // condition that keeps it out of the cache.
    let decoy_input_hash = chunk_hash(decoy_text);
    catalog::replace_work_chunks_in_transaction(
        &conn,
        &item.id,
        &[
            catalog::ChunkRow {
                id: "chunk-1".to_string(),
                item_id: item.id.clone(),
                attachment_id: attachment.id.clone(),
                ordinal: 0,
                text_content: CHUNK_TEXT.to_string(),
                text_hash: chunk_input_hash.clone(),
                chunking_contract:
                    entropia_desktop_lib::bibliography::chunks::BIBLIOGRAPHY_CHUNKING_CONTRACT_V2
                        .to_string(),
                spans: vec![(1, 0, CHUNK_TEXT.chars().count() as i64)],
            },
            // §2.3b: `input_hash = chunks.text_hash` is the pasaje condition. This
            // row breaks it, so its vector must stay out of the cache: the reader
            // would never look it up under the hash of its text.
            catalog::ChunkRow {
                id: "chunk-2".to_string(),
                item_id: item.id.clone(),
                attachment_id: attachment.id.clone(),
                ordinal: 1,
                text_content: decoy_text.to_string(),
                text_hash: "legacy-hash-not-the-text-hash".to_string(),
                chunking_contract:
                    entropia_desktop_lib::bibliography::chunks::BIBLIOGRAPHY_CHUNKING_CONTRACT_V2
                        .to_string(),
                spans: vec![(1, 0, 10)],
            },
        ],
        7,
    )
    .expect("plant chunks");
    for (chunk_id, input_hash) in [
        ("chunk-1", chunk_input_hash.clone()),
        ("chunk-2", decoy_input_hash),
    ] {
        catalog::upsert_chunk_embedding_in_transaction(
            &conn,
            &catalog::ChunkEmbeddingRow {
                chunk_id: chunk_id.to_string(),
                generation_id: generation.id.clone(),
                embedding_contract: effective.contract.clone(),
                embedding_model: effective.model.clone(),
                dimensions: dimensions as usize,
                embedding: vector_blob(dimensions, 0.75),
                input_hash,
            },
            7,
        )
        .expect("plant pasaje vector");
    }

    // The owner's session, exactly as the first binding leaves it (§2.9).
    meta_set(&conn, "account_id", ACCOUNT_A).expect("session account");
    meta_set(&conn, "derived_bound_account", ACCOUNT_A).expect("bound account");
    meta_set(&conn, "derived_owner_account", ACCOUNT_A).expect("owner account");

    (
        Seeded {
            dir,
            item_id: item.id,
            attachment_id: attachment.id,
            file,
            source_bytes,
            contract_hash: effective.hash,
            dimensions,
            item_input_hash,
            chunk_input_hash,
        },
        conn,
    )
}

/// One stored page row with its identity computed exactly as the publisher
/// computes it (`sha256` of the published text).
fn plant_page(conn: &Connection, attachment_id: &str, page_number: i64, method: &str, text: &str) {
    let quality = extraction_quality(text).to_string();
    catalog::upsert_page_text_in_transaction(
        conn,
        &catalog::PageTextRow {
            attachment_id: attachment_id.to_string(),
            page_number,
            method: method.to_string(),
            text_content: text.to_string(),
            text_hash: sha256_hex(text.as_bytes()),
            text_chars: text.chars().count() as i64,
            quality,
        },
        7,
    )
    .expect("plant page row");
}

/// A second attachment of the same work whose paid blank answer survived only as
/// a task checkpoint: the extraction never published a row, so §2.3b reads it
/// from the retained `ocr-page:N` checkpoint. Returns its id and file path.
fn add_retained_empty_answer(
    conn: &mut Connection,
    dir: &std::path::Path,
    item_id: &str,
) -> (String, PathBuf) {
    let bytes = b"%PDF-1.4 synthetic scan of a cancelled extraction".to_vec();
    let file = dir.join("cancelled.pdf");
    std::fs::write(&file, &bytes).expect("write checkpoint attachment file");
    let attachment = catalog::upsert_attachment(
        conn,
        item_id,
        catalog::AttachmentInput {
            attachment_key: "ATT2".to_string(),
            content_type: Some("application/pdf".to_string()),
            link_mode: Some("linked_file".to_string()),
            filename: Some("cancelled.pdf".to_string()),
            native_path: Some(file.to_string_lossy().into_owned()),
            url: None,
            md5: None,
            mtime: Some(MTIME),
            native_json_snapshot: serde_json::json!({
                "key": "ATT2", "itemType": "attachment", "linkMode": "linked_file",
                "contentType": "application/pdf",
            })
            .to_string(),
            native_version: Some(3),
        },
    )
    .expect("seed checkpoint attachment");

    let batch =
        processing_repository::ensure_system_batch(conn, "bibliography").expect("system batch");
    let admitted = processing_repository::admit_subject_or_attach(
        conn,
        &batch,
        "bibliography_extract",
        &processing_repository::TaskSubject {
            domain: "bibliography".to_string(),
            subject_kind: "attachment".to_string(),
            subject_id: attachment.id.clone(),
        },
        0,
        "",
        "",
        None,
    )
    .expect("admit extraction task")
    .expect("no live task owns this attachment")
    .task_id;
    // The attempt ended without publishing anything: `failed` keeps its paid
    // checkpoints (JD8-B-002).
    conn.execute(
        "UPDATE processing_tasks SET state = 'failed' WHERE id = ?1",
        [&admitted],
    )
    .expect("settle task");
    let (fingerprint, contract): (String, String) = conn
        .query_row(
            "SELECT input_fingerprint, contract_hash FROM processing_tasks WHERE id = ?1",
            [&admitted],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("task identity");
    // `ocr-page:1` is the blank answer that cost money; `ocr-page:2` carries text
    // and is NOT a source §2.3b names, so it must stay out of the cache.
    for (unit_key, payload) in [("ocr-page:1", r#""""#), ("ocr-page:2", r#""texto""#)] {
        conn.execute(
            "INSERT INTO processing_checkpoints
               (task_id, unit_key, input_fingerprint, contract_hash, payload, payload_checksum, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'checksum', 7)",
            rusqlite::params![admitted, unit_key, fingerprint, contract, payload],
        )
        .expect("plant checkpoint");
    }
    (attachment.id, file)
}

/// One `sync_derived_vectors` row as a test compares it, ordered by key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedVector {
    contract_hash: String,
    input_hash: String,
    dimensions: i64,
    embedding: Vec<u8>,
    origin: String,
    pushed: i64,
}

fn cache_vector_rows(conn: &Connection, account: &str) -> Vec<CachedVector> {
    let mut statement = conn
        .prepare(
            "SELECT contract_hash, input_hash, dimensions, embedding, origin, pushed
               FROM sync_derived_vectors WHERE account_id = ?1
              ORDER BY input_hash",
        )
        .expect("prepare vector read");
    statement
        .query_map([account], |row| {
            Ok(CachedVector {
                contract_hash: row.get(0)?,
                input_hash: row.get(1)?,
                dimensions: row.get(2)?,
                embedding: row.get(3)?,
                origin: row.get(4)?,
                pushed: row.get(5)?,
            })
        })
        .expect("read cache vectors")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect cache vectors")
}

/// One `sync_derived_ocr_pages` row as a test compares it, ordered by
/// `(file, page)`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedOcrPage {
    source_hash: String,
    page_number: i64,
    ocr_contract: String,
    outcome: String,
    text: String,
    text_hash: String,
    origin: String,
    pushed: i64,
}

fn cache_ocr_rows(conn: &Connection, account: &str) -> Vec<CachedOcrPage> {
    let mut statement = conn
        .prepare(
            "SELECT source_hash, page_number, ocr_contract, outcome, text_content,
                    text_hash, origin, pushed
               FROM sync_derived_ocr_pages WHERE account_id = ?1
              ORDER BY source_hash, page_number",
        )
        .expect("prepare OCR read");
    statement
        .query_map([account], |row| {
            Ok(CachedOcrPage {
                source_hash: row.get(0)?,
                page_number: row.get(1)?,
                ocr_contract: row.get(2)?,
                outcome: row.get(3)?,
                text: row.get(4)?,
                text_hash: row.get(5)?,
                origin: row.get(6)?,
                pushed: row.get(7)?,
            })
        })
        .expect("read cache OCR")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect cache OCR")
}

/// Bulk-seeds `count` extra pasaje vectors on top of the fixture's pair, the way a
/// library that already paid leaves them: one transaction so the fixture itself
/// stays fast.
fn bulk_seed_chunk_vectors(conn: &Connection, seeded: &Seeded, count: i64) {
    let (model, contract): (String, String) = conn
        .query_row(
            "SELECT embedding_model, embedding_contract FROM bibliographic_chunk_embeddings
              WHERE chunk_id = 'chunk-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("fixture chunk stamps");
    let generation: String = conn
        .query_row(
            "SELECT id FROM bibliographic_index_generations
              WHERE contract_hash = ?1 ORDER BY created_at, id LIMIT 1",
            [&seeded.contract_hash],
            |row| row.get(0),
        )
        .expect("staging generation");
    conn.execute_batch("BEGIN").expect("begin bulk vector seed");
    {
        let mut chunk = conn
            .prepare(
                "INSERT INTO bibliographic_chunks
                   (id, item_id, attachment_id, ordinal, text_content, text_hash,
                    chunking_contract, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'bibliography-chunk-800-paragraph-v2', 7, 7)",
            )
            .expect("prepare bulk chunk");
        let mut vector = conn
            .prepare(
                "INSERT INTO bibliographic_chunk_embeddings
                   (chunk_id, generation_id, embedding_contract, embedding_model, dimensions,
                    embedding, input_hash, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 7, 7)",
            )
            .expect("prepare bulk vector");
        for index in 0..count {
            let chunk_id = format!("chunk-bulk-{index:05}");
            let text = format!("fragmento sintetico numero {index} con texto estable para trocear");
            let input_hash = chunk_hash(&text);
            chunk
                .execute(rusqlite::params![
                    chunk_id,
                    seeded.item_id,
                    seeded.attachment_id,
                    10 + index,
                    text,
                    input_hash,
                ])
                .expect("bulk chunk row");
            vector
                .execute(rusqlite::params![
                    chunk_id,
                    generation,
                    contract,
                    model,
                    seeded.dimensions,
                    vector_blob(seeded.dimensions, 1.0 + index as f32),
                    input_hash,
                ])
                .expect("bulk chunk vector");
        }
    }
    conn.execute_batch("COMMIT")
        .expect("commit bulk vector seed");
}

/// The durable progress mark of the fixture's account and contract.
fn mark_value(conn: &Connection, seeded: &Seeded) -> Option<String> {
    meta_get(
        conn,
        &format!("derived_backfill:{}:{}", ACCOUNT_A, seeded.contract_hash),
    )
    .expect("read mark")
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .expect("count rows")
}

/// Everything the pass must never touch: the published bibliography rows and the
/// whole processing queue (a readmission would show up here).
fn published_state(conn: &Connection) -> Vec<String> {
    let mut lines = Vec::new();
    for (label, sql) in [
        (
            "extraction",
            "SELECT attachment_id || '|' || method || '|' || text_hash FROM bibliographic_extractions ORDER BY attachment_id",
        ),
        (
            "page",
            "SELECT attachment_id || '|' || page_number || '|' || method || '|' || quality || '|' || text_hash FROM bibliographic_page_texts ORDER BY attachment_id, page_number",
        ),
        (
            "item-embedding",
            "SELECT item_id || '|' || generation_id || '|' || input_hash FROM bibliographic_item_embeddings ORDER BY item_id, generation_id",
        ),
        (
            "chunk-embedding",
            "SELECT chunk_id || '|' || generation_id || '|' || input_hash FROM bibliographic_chunk_embeddings ORDER BY chunk_id",
        ),
        (
            "chunk",
            "SELECT id || '|' || text_hash FROM bibliographic_chunks ORDER BY id",
        ),
        (
            "task",
            "SELECT id || '|' || state || '|' || input_fingerprint FROM processing_tasks ORDER BY id",
        ),
        (
            "checkpoint",
            "SELECT task_id || '|' || unit_key || '|' || payload FROM processing_checkpoints ORDER BY task_id, unit_key",
        ),
        ("attempt", "SELECT CAST(COUNT(*) AS TEXT) FROM processing_attempts"),
    ] {
        let mut statement = conn.prepare(sql).expect("prepare snapshot");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("snapshot rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect snapshot");
        for row in rows {
            lines.push(format!("{label}:{row}"));
        }
    }
    lines
}

/// §2.3b/S4: the archive already paid, the cache gets it for free.
#[test]
fn an_already_processed_archive_seeds_its_vectors_and_ocr_without_readmitting_tasks() {
    let (seeded, mut conn) = seeded_archive();
    let (checkpoint_attachment, checkpoint_file) =
        add_retained_empty_answer(&mut conn, seeded.dir.path(), &seeded.item_id);
    let checkpoint_source = sha256_hex(&std::fs::read(&checkpoint_file).expect("checkpoint file"));
    let published_before = published_state(&conn);
    let tasks_before = count(&conn, "processing_tasks");

    let report = run_backfill(&conn);

    assert_eq!(report.status, BackfillStatus::Completed);
    assert_eq!(report.account.as_deref(), Some(ACCOUNT_A));
    assert_eq!(
        report.contract_hash.as_deref(),
        Some(seeded.contract_hash.as_str())
    );
    assert_eq!(report.vectors, 2, "the ficha and the pasaje vector");
    assert_eq!(report.vectors_skipped, 0);
    assert_eq!(
        report.ocr_pages, 3,
        "the text page, the empty page and the retained blank answer"
    );
    assert_eq!(report.ocr_empty, 2, "both what came back empty");
    assert_eq!(report.ocr_skipped, 0);
    assert_eq!(report.attachments_skipped, 0);
    assert_eq!(report.files_read, 2);
    assert_eq!(report.failures, 0);
    assert!(report.completed());

    // ── vectors: keyed by the hash a reader rebuilds, under the current contract
    let vectors = cache_vector_rows(&conn, ACCOUNT_A);
    let mut expected_vectors = vec![
        CachedVector {
            contract_hash: seeded.contract_hash.clone(),
            input_hash: seeded.chunk_input_hash.clone(),
            dimensions: seeded.dimensions,
            embedding: vector_blob(seeded.dimensions, 0.75),
            origin: "local".to_string(),
            pushed: 0,
        },
        CachedVector {
            contract_hash: seeded.contract_hash.clone(),
            input_hash: seeded.item_input_hash.clone(),
            dimensions: seeded.dimensions,
            embedding: vector_blob(seeded.dimensions, 0.5),
            origin: "local".to_string(),
            pushed: 0,
        },
    ];
    expected_vectors.sort_by(|left, right| left.input_hash.cmp(&right.input_hash));
    assert_eq!(
        vectors, expected_vectors,
        "the unusable pasaje (input_hash != text_hash) is not cached"
    );
    let mirrored: (String, String, i64, String) = conn
        .query_row(
            "SELECT provider, model, dimensions, chunking_contract
               FROM sync_derived_contracts WHERE contract_hash = ?1",
            [&seeded.contract_hash],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("mirrored contract");
    assert_eq!(mirrored.2, seeded.dimensions);
    assert_eq!(mirrored.3, RAG_CHUNKING_CONTRACT);

    // ── OCR: keyed by the sha256 of the file, under the production contract
    let ocr_contract = production_page_ocr_contract();
    let pages = cache_ocr_rows(&conn, ACCOUNT_A);
    let source_hash = seeded.source_hash();
    let mut expected_pages = vec![
        CachedOcrPage {
            source_hash: source_hash.clone(),
            page_number: 2,
            ocr_contract: ocr_contract.clone(),
            outcome: "text".to_string(),
            text: OCR_TEXT.to_string(),
            text_hash: sha256_hex(OCR_TEXT.as_bytes()),
            origin: "local".to_string(),
            pushed: 0,
        },
        CachedOcrPage {
            source_hash: source_hash.clone(),
            page_number: 3,
            ocr_contract: ocr_contract.clone(),
            outcome: "empty".to_string(),
            text: String::new(),
            text_hash: sha256_hex(b""),
            origin: "local".to_string(),
            pushed: 0,
        },
        CachedOcrPage {
            source_hash: checkpoint_source,
            page_number: 1,
            ocr_contract,
            outcome: "empty".to_string(),
            text: String::new(),
            text_hash: sha256_hex(b""),
            origin: "local".to_string(),
            pushed: 0,
        },
    ];
    expected_pages.sort_by(|left, right| {
        left.source_hash
            .cmp(&right.source_hash)
            .then_with(|| left.page_number.cmp(&right.page_number))
    });
    assert_eq!(
        pages, expected_pages,
        "the blank `quality='empty'` row and the retained blank answer travel as 'empty'; \
         the retained answer that carried text is not a source §2.3b names"
    );
    assert!(cache_ocr_rows(&conn, ACCOUNT_B).is_empty());

    // ── no task, no attempt, no checkpoint, and no published row moved
    assert_eq!(
        count(&conn, "processing_tasks"),
        tasks_before,
        "the pass never readmits work"
    );
    assert_eq!(published_state(&conn), published_before);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sync_derived_ocr_pages WHERE account_id = ?1",
            [checkpoint_attachment.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .expect("count rows under the attachment id"),
        0
    );
}

/// §2.3b: the file hash only means something if the file is still the one the
/// current extraction was read from.
#[test]
fn a_changed_file_skips_its_ocr_pages_but_keeps_the_vectors() {
    for change in ["mtime", "bytes", "missing"] {
        let (seeded, conn) = seeded_archive();
        let mut expected_reads = 1;
        match change {
            // A metadata edit moved the catalog mtime.
            "mtime" => {
                conn.execute(
                    "UPDATE zotero_attachments SET mtime = ?1 WHERE id = ?2",
                    rusqlite::params![MTIME + 1, &seeded.attachment_id],
                )
                .expect("move catalog mtime");
            }
            // The file was replaced by one of another size. (The catalog keeps
            // only `mtime` and byte count, so a same-size same-mtime replacement
            // is invisible to every gate that reads this identity, not just to
            // this pass — see the plan's risk list.)
            "bytes" => {
                std::fs::write(
                    &seeded.file,
                    b"%PDF-1.4 another scan, longer than the first one",
                )
                .expect("rewrite");
            }
            // The stored copy is gone: same verdict, nothing is read at all.
            _ => {
                std::fs::remove_file(&seeded.file).expect("remove attachment file");
                expected_reads = 0;
            }
        }

        let report = run_backfill(&conn);

        assert_eq!(report.status, BackfillStatus::Completed, "{change}");
        assert_eq!(report.attachments_skipped, 1, "{change}");
        assert_eq!(report.ocr_pages, 0, "{change}: no page is guessed");
        assert_eq!(
            report.ocr_skipped, 2,
            "{change}: both stored pages are left out"
        );
        assert_eq!(report.files_read, expected_reads, "{change}");
        assert!(
            cache_ocr_rows(&conn, ACCOUNT_A).is_empty(),
            "{change}: nothing may key the pages under a file the extraction no longer matches"
        );
        // The vectors do not depend on the file at all.
        assert_eq!(report.vectors, 2, "{change}");
        assert_eq!(cache_vector_rows(&conn, ACCOUNT_A).len(), 2, "{change}");
    }
}

/// §2.3b: the mark describes local rows, so a second pass costs a metadata read
/// and opens no file.
#[test]
fn a_second_pass_is_honored_by_the_mark_and_reads_no_pdf() {
    let (seeded, conn) = seeded_archive();
    let first = run_backfill(&conn);
    assert_eq!(first.status, BackfillStatus::Completed);
    assert_eq!(first.files_read, 1);
    assert_eq!(first.ocr_pages, 2);
    let vectors = cache_vector_rows(&conn, ACCOUNT_A);
    let pages = cache_ocr_rows(&conn, ACCOUNT_A);

    // Removing the file proves the mark is what answers: a pass that re-read the
    // PDF would report a skip and a read.
    std::fs::remove_file(&seeded.file).expect("remove attachment file");
    let second = run_backfill(&conn);
    assert_eq!(second.status, BackfillStatus::AlreadySeeded);
    assert_eq!(second.files_read, 0, "no PDF is opened again");
    assert_eq!(second.vectors, 0);
    assert_eq!(second.ocr_pages, 0);
    assert_eq!(second.attachments_skipped, 0);
    assert_eq!(second.failures, 0);
    assert_eq!(cache_vector_rows(&conn, ACCOUNT_A), vectors);
    assert_eq!(cache_ocr_rows(&conn, ACCOUNT_A), pages);
}

/// §2.9/S7: logout keeps the rows and the mark; another account neither seeds
/// the archive's past nor inherits it, and the owner's return does not re-read
/// the PDFs.
#[test]
fn another_account_never_seeds_the_owners_archive() {
    let (seeded, conn) = seeded_archive();
    assert_eq!(run_backfill(&conn).status, BackfillStatus::Completed);
    let owners_vectors = cache_vector_rows(&conn, ACCOUNT_A);
    let owners_pages = cache_ocr_rows(&conn, ACCOUNT_A);

    // A logs out: the rows stay, the session and the bound account go away.
    clear_sync_state(&conn).expect("logout");
    let logged_out = run_backfill(&conn);
    assert_eq!(logged_out.status, BackfillStatus::NoSession);
    assert_eq!(logged_out.files_read, 0);
    assert_eq!(
        meta_get(&conn, "derived_owner_account").expect("owner mark"),
        Some(ACCOUNT_A.to_string()),
        "logout never drops the owner mark"
    );

    // B logs in: the session and the bound account move, the owner does not.
    meta_set(&conn, "account_id", ACCOUNT_B).expect("session B");
    meta_set(&conn, "derived_bound_account", ACCOUNT_B).expect("bound B");
    std::fs::remove_file(&seeded.file).expect("remove attachment file");
    let under_b = run_backfill(&conn);
    assert_eq!(under_b.status, BackfillStatus::NotOwner);
    assert_eq!(under_b.account.as_deref(), Some(ACCOUNT_B));
    assert_eq!(under_b.vectors, 0);
    assert_eq!(under_b.ocr_pages, 0);
    assert_eq!(under_b.files_read, 0);
    assert!(cache_vector_rows(&conn, ACCOUNT_B).is_empty());
    assert!(cache_ocr_rows(&conn, ACCOUNT_B).is_empty());
    assert_eq!(cache_vector_rows(&conn, ACCOUNT_A), owners_vectors);
    assert_eq!(cache_ocr_rows(&conn, ACCOUNT_A), owners_pages);

    // A is back: the mark is still there, so the pass does not re-open a file.
    meta_set(&conn, "account_id", ACCOUNT_A).expect("session A");
    meta_set(&conn, "derived_bound_account", ACCOUNT_A).expect("bound A");
    let back = run_backfill(&conn);
    assert_eq!(back.status, BackfillStatus::AlreadySeeded);
    assert_eq!(back.files_read, 0);
    assert_eq!(cache_vector_rows(&conn, ACCOUNT_A), owners_vectors);
    assert_eq!(cache_ocr_rows(&conn, ACCOUNT_A), owners_pages);
}

/// §2.3b: without an owner mark the archive was never bound, so there is nobody
/// whose past this is and nothing is seeded under the session account.
#[test]
fn an_archive_without_an_owner_mark_never_seeds() {
    let (_seeded, conn) = seeded_archive();
    conn.execute(
        "DELETE FROM sync_meta WHERE key = 'derived_owner_account'",
        [],
    )
    .expect("drop owner mark");

    let report = run_backfill(&conn);

    assert_eq!(report.status, BackfillStatus::NotOwner);
    assert_eq!(report.vectors, 0);
    assert_eq!(report.ocr_pages, 0);
    assert_eq!(report.files_read, 0);
    assert!(cache_vector_rows(&conn, ACCOUNT_A).is_empty());
    assert!(cache_ocr_rows(&conn, ACCOUNT_A).is_empty());
}

/// An archive whose bootstrap has not created the derived tables yet answers
/// honestly instead of failing row by row.
#[test]
fn without_the_derived_tables_the_pass_answers_cache_unavailable() {
    let (_seeded, conn) = seeded_archive();
    conn.execute_batch(
        "DROP TABLE sync_derived_vectors;
         DROP TABLE sync_derived_ocr_pages;
         DROP TABLE sync_derived_contracts;",
    )
    .expect("drop derived tables");

    let report = run_backfill(&conn);

    assert_eq!(report.status, BackfillStatus::CacheUnavailable);
    assert_eq!(report.account.as_deref(), Some(ACCOUNT_A));
    assert_eq!(report.files_read, 0);
    assert_eq!(report.vectors, 0);
    assert_eq!(report.ocr_pages, 0);
}

/// L6/§2.4: the stored OCR row is the CONVERTED answer, so only a text the
/// conversion leaves alone may be seeded — a reader replaying it must publish
/// the same row, and one that would convert again is skipped instead of guessed.
#[test]
fn a_backfilled_ocr_answer_replays_to_the_row_the_archive_published() {
    let (seeded, conn) = seeded_archive();
    // A second `ocr` page whose text still looks like markup: this is exactly the
    // shape L6 found (`<p>a &lt;b&gt; c</p>` → `a <b> c`), which a reader would
    // convert a second time into `a c`.
    plant_page(
        &conn,
        &seeded.attachment_id,
        4,
        "ocr",
        STORED_TEXT_THAT_CONVERTS_AGAIN,
    );

    let report = run_backfill(&conn);
    assert_eq!(report.status, BackfillStatus::Completed);
    assert_eq!(
        report.ocr_skipped, 1,
        "the page whose stored text does not replay unchanged is left out"
    );
    assert_eq!(report.ocr_pages, 2);

    // The stored row, straight from the archive.
    let (stored_text, stored_hash, stored_chars, stored_quality, stored_method): (
        String,
        String,
        i64,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT text_content, text_hash, text_chars, quality, method
               FROM bibliographic_page_texts WHERE attachment_id = ?1 AND page_number = 2",
            [&seeded.attachment_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("stored page row");
    let cached_text: String = conn
        .query_row(
            "SELECT text_content FROM sync_derived_ocr_pages
              WHERE account_id = ?1 AND source_hash = ?2 AND page_number = 2",
            rusqlite::params![ACCOUNT_A, &seeded.source_hash()],
            |row| row.get(0),
        )
        .expect("seeded OCR answer");

    // What a reader does (§2.4): settle the cached answer over its OWN native
    // page. The answer is not empty, so the native page cannot leak into it.
    let native = ExtractPageText {
        page_number: 2,
        method: "native".to_string(),
        text_content: "texto nativo de esta pagina".to_string(),
        text_hash: sha256_hex("texto nativo de esta pagina".as_bytes()),
        text_chars: 26,
        quality: "sparse".to_string(),
    };
    let replayed = settled_page_row(native, cached_text);
    assert_eq!(replayed.text_content, stored_text);
    assert_eq!(replayed.text_hash, stored_hash);
    assert_eq!(replayed.text_chars, stored_chars);
    assert_eq!(replayed.quality, stored_quality);
    assert_eq!(replayed.method, stored_method);

    // And the page that would convert again has no cache row at all.
    let skipped: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_derived_ocr_pages
              WHERE account_id = ?1 AND source_hash = ?2 AND page_number = 4",
            rusqlite::params![ACCOUNT_A, &seeded.source_hash()],
            |row| row.get(0),
        )
        .expect("count skipped page");
    assert_eq!(skipped, 0);
}

/// A1: the pass writes in pages, so a library-sized vector set finishes and stays
/// idempotent. No timing assertion: what is pinned is that 2,000+ rows land under
/// the mark and that the mark — not a lucky retry — is what makes the next pass
/// cheap.
#[test]
fn a_large_vector_set_is_seeded_and_the_mark_keeps_the_next_pass_cheap() {
    let (seeded, conn) = seeded_archive();
    bulk_seed_chunk_vectors(&conn, &seeded, 2_000);

    let first = run_backfill(&conn);

    assert_eq!(first.status, BackfillStatus::Completed);
    assert_eq!(
        first.vectors, 2_002,
        "the ficha, the pasaje and the 2,000 bulk vectors"
    );
    assert_eq!(first.vectors_skipped, 0);
    assert_eq!(first.failures, 0);
    assert!(first.completed());
    assert_eq!(cache_vector_rows(&conn, ACCOUNT_A).len(), 2_002);
    assert_eq!(mark_value(&conn, &seeded).as_deref(), Some("done"));

    let second = run_backfill(&conn);
    assert_eq!(second.status, BackfillStatus::AlreadySeeded);
    assert_eq!(second.vectors, 0);
    assert_eq!(second.collection_vectors, 0);
    assert_eq!(second.files_read, 0);
    assert_eq!(cache_vector_rows(&conn, ACCOUNT_A).len(), 2_002);
}

/// A1: a failed write withholds the mark, and the retry completes — the rows the
/// first pass did write are kept (`INSERT OR IGNORE`), never duplicated.
#[test]
fn a_storage_failure_withholds_the_mark_and_the_next_pass_completes() {
    let (seeded, conn) = seeded_archive();
    conn.execute_batch(
        "CREATE TRIGGER block_derived_ocr BEFORE INSERT ON sync_derived_ocr_pages
         BEGIN SELECT RAISE(ABORT, 'blocked by test'); END;",
    )
    .expect("blocking trigger");

    let broken = run_backfill(&conn);

    assert_eq!(broken.status, BackfillStatus::Completed);
    assert!(broken.failures > 0, "the blocked inserts are failures");
    assert!(!broken.completed(), "a failure means the pass is not done");
    assert_eq!(broken.ocr_pages, 0);
    assert_eq!(
        mark_value(&conn, &seeded),
        None,
        "a storage failure must withhold the mark so the next pass retries"
    );
    assert_eq!(
        broken.vectors, 2,
        "the vectors written before the failure are kept"
    );
    assert_eq!(cache_vector_rows(&conn, ACCOUNT_A).len(), 2);
    assert!(cache_ocr_rows(&conn, ACCOUNT_A).is_empty());

    conn.execute_batch("DROP TRIGGER block_derived_ocr;")
        .expect("drop blocking trigger");
    let retried = run_backfill(&conn);

    assert_eq!(retried.status, BackfillStatus::Completed);
    assert_eq!(retried.failures, 0);
    assert!(retried.completed());
    assert_eq!(retried.ocr_pages, 2, "the retry seeds what was blocked");
    assert_eq!(retried.vectors, 0, "the kept vectors are not written twice");
    assert_eq!(cache_vector_rows(&conn, ACCOUNT_A).len(), 2);
    assert_eq!(cache_ocr_rows(&conn, ACCOUNT_A).len(), 2);
    assert_eq!(mark_value(&conn, &seeded).as_deref(), Some("done"));
}

/// The mark is an optimization, not the only thing keeping the cache consistent:
/// erased, a second pass re-reads the sources and still adds nothing.
#[test]
fn a_pass_without_its_mark_adds_nothing_the_second_time() {
    let (seeded, conn) = seeded_archive();
    let first = run_backfill(&conn);
    assert_eq!(first.status, BackfillStatus::Completed);
    let vectors = cache_vector_rows(&conn, ACCOUNT_A);
    let pages = cache_ocr_rows(&conn, ACCOUNT_A);

    conn.execute(
        "DELETE FROM sync_meta WHERE key = ?1",
        [format!(
            "derived_backfill:{ACCOUNT_A}:{}",
            seeded.contract_hash
        )],
    )
    .expect("erase the mark");

    let second = run_backfill(&conn);

    assert_eq!(second.status, BackfillStatus::Completed);
    assert_eq!(second.failures, 0);
    assert_eq!(second.vectors, 0, "INSERT OR IGNORE adds nothing");
    assert_eq!(second.collection_vectors, 0);
    assert_eq!(second.ocr_pages, 0);
    assert_eq!(
        second.files_read, 1,
        "without the mark the pass may look at the file again"
    );
    assert_eq!(cache_vector_rows(&conn, ACCOUNT_A), vectors);
    assert_eq!(cache_ocr_rows(&conn, ACCOUNT_A), pages);
    assert_eq!(mark_value(&conn, &seeded).as_deref(), Some("done"));
}
