//! Scale ladder: how the operations that grow with the archive behave at
//! 10k, 50k, 100k and 200k pages, to find where each one stops being usable
//! before a user does (Tropy's users hit that wall past some photo count).
//!
//! Ignored by default. One size per run, so a caller can cap memory and time
//! per step from outside:
//!
//!   DB_SCALE_PAGES=10000 cargo test --profile measure --test db_scale -- --ignored --nocapture
//!
//! Every run builds a throwaway temp database from the real schema fixture —
//! never the user's archive. It prints numbers and asserts nothing about
//! speed. The SQL of the TypeScript store is copied here (source noted at
//! each copy), so a change there must be mirrored to keep this honest.

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};

const SCHEMA_FIXTURE: &str = include_str!("fixtures/schema_full.sql");
const PAGES_PER_ITEM: usize = 5;
const EMBEDDING_DIM: usize = 1024; // nlp/embeddings.rs
const SEED_CHUNK_ITEMS: usize = 2_000;

const WORDS: &[&str] = &[
    "expediente",
    "juzgado",
    "sentencia",
    "acta",
    "legajo",
    "carta",
    "archivo",
    "provincia",
    "municipio",
    "decreto",
    "censo",
    "parroquia",
    "bautismo",
    "matrimonio",
    "defunción",
    "notaría",
    "escritura",
    "testamento",
    "puerto",
    "aduana",
    "ferrocarril",
    "colonia",
    "estancia",
    "cabildo",
];

fn text(seed: usize, words: usize) -> String {
    (0..words)
        .map(|i| WORDS[(seed.wrapping_mul(31).wrapping_add(i * 7)) % WORDS.len()])
        .collect::<Vec<_>>()
        .join(" ")
}

fn embedding(seed: usize) -> Vec<u8> {
    (0..EMBEDDING_DIM)
        .flat_map(|i| (((seed * 7 + i * 13) % 97) as f32 / 97.0 - 0.5).to_le_bytes())
        .collect()
}

/// One collection; per item 5 image pages, each with an OCR extraction, an
/// embedding, an OCR task (pending) and an embedding task blocked on it — the
/// state right after a user starts "OCR + embeddings" on a big import.
fn seed(conn: &Connection, items: usize) {
    conn.execute_batch(
        "INSERT INTO collections (id, name, created_at, updated_at) VALUES ('c1', 'Fondo', 1, 1);
         INSERT INTO processing_batches (id, request_id, origin, state, desired_state, operations, created_at, updated_at)
         VALUES ('b1', 'r1', 'user', 'running', 'run', '[\"ocr\",\"embedding\"]', 1, 1);",
    )
    .unwrap();
    for start in (0..items).step_by(SEED_CHUNK_ITEMS) {
        let tx = conn.unchecked_transaction().unwrap();
        {
            let mut item = tx.prepare("INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at) VALUES (?1, ?2, 'c1', ?3, ?4, ?4)").unwrap();
            let mut asset = tx.prepare("INSERT INTO assets (id, item_id, path, type, size, sort_index, created_at) VALUES (?1, ?2, ?3, 'image', 1000, ?4, ?5)").unwrap();
            let mut extraction = tx.prepare("INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES (?1, ?2, ?3, 'ocr', ?4)").unwrap();
            let mut vec = tx
                .prepare(
                    "INSERT INTO vec_assets (asset_id, item_id, embedding) VALUES (?1, ?2, ?3)",
                )
                .unwrap();
            let mut task = tx.prepare("INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 1, 1)").unwrap();
            let mut link = tx.prepare("INSERT INTO processing_batch_tasks (batch_id, task_id, kind, asset_id_snapshot, dependency_task_id) VALUES ('b1', ?1, ?2, ?3, ?4)").unwrap();
            let mut fts = tx.prepare("INSERT INTO fts_items (rowid, item_id, title, metadata, extracted_text) SELECT rowid, id, title, metadata, ?2 FROM items WHERE id = ?1").unwrap();
            for i in start..(start + SEED_CHUNK_ITEMS).min(items) {
                let item_id = format!("i{i}");
                let meta = format!(
                    r#"{{"__entropia_file_metadata":{{"originalPath":"C:\\Archivo\\Fondo\\doc{i}.pdf","sizeBytes":{},"modifiedAt":{}}}}}"#,
                    1000 + i,
                    1_700_000_000_000i64 + i as i64
                );
                item.execute(params![item_id, format!("Documento {i}"), meta, i as i64])
                    .unwrap();
                let mut item_text = String::new();
                for p in 0..PAGES_PER_ITEM {
                    let asset_id = format!("a{i}_{p}");
                    let page = text(i + p, 120);
                    asset
                        .execute(params![
                            asset_id,
                            item_id,
                            format!("{asset_id}.png"),
                            p as i64,
                            i as i64
                        ])
                        .unwrap();
                    extraction
                        .execute(params![format!("e{i}_{p}"), asset_id, page, i as i64])
                        .unwrap();
                    vec.execute(params![
                        asset_id,
                        item_id,
                        embedding(i * PAGES_PER_ITEM + p)
                    ])
                    .unwrap();
                    let (ocr, emb) = (format!("ocr-{asset_id}"), format!("emb-{asset_id}"));
                    task.execute(params![ocr, "ocr", asset_id, "pending"])
                        .unwrap();
                    task.execute(params![emb, "embedding", asset_id, "blocked"])
                        .unwrap();
                    link.execute(params![ocr, "ocr", asset_id, None::<String>])
                        .unwrap();
                    link.execute(params![emb, "embedding", asset_id, ocr])
                        .unwrap();
                    item_text.push_str(&page);
                    item_text.push(' ');
                }
                fts.execute(params![item_id, item_text]).unwrap();
            }
        }
        tx.commit().unwrap();
    }
}

fn time<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let out = f();
    (out, t.elapsed())
}

fn secs(d: Duration) -> String {
    format!("{:.2}s", d.as_secs_f64())
}

// packages/store/src/repos/item.repo.ts — getCorpusStats (Inicio).
const CORPUS_STATS_SQL: &str = "
          WITH
          -- One pass over the text and one over the viewable files: each file
          -- is classified once and the counts are sums of its flags. The
          -- earlier shape re-ran the viewable_assets CTE for every count and
          -- read every extraction three times, 3.4 s at 200k pages. No
          -- semicolons in this statement, comments included: db_select
          -- rejects any.
          extraction_text AS (
            SELECT e.asset_id,
                   MAX(e.method = 'native' AND TRIM(e.text_content) <> '') AS has_native,
                   -- OCR-derived text: any extraction method other than 'native'.
                   MAX(e.method <> 'native' AND TRIM(e.text_content) <> '') AS has_ocr
              FROM extractions e
             WHERE e.text_content IS NOT NULL
             GROUP BY e.asset_id
          ),
          transcription_text AS (
            SELECT t.asset_id, 1 AS has_stt
              FROM transcriptions t
             WHERE t.text_content IS NOT NULL AND TRIM(t.text_content) <> ''
             GROUP BY t.asset_id
          ),
          viewable_assets AS (
            SELECT a.id, a.type AS type,
                   COALESCE(et.has_native, 0) AS has_native,
                   COALESCE(et.has_ocr, 0) AS has_ocr,
                   COALESCE(tt.has_stt, 0) AS has_stt,
                   EXISTS (SELECT 1 FROM vec_assets v WHERE v.asset_id = a.id) AS has_vec
              FROM assets a
              LEFT JOIN extraction_text et ON et.asset_id = a.id
              LEFT JOIN transcription_text tt ON tt.asset_id = a.id
             WHERE NOT EXISTS (
               SELECT 1 FROM assets child WHERE child.parent_asset_id = a.id
             )
          ),
          classified AS (
            SELECT va.*,
                   -- OCR universe: a viewable IMAGE, or a viewable PDF page with
                   -- no non-empty native text layer (a scanned page). A page not
                   -- yet checked for a native layer counts as scanned.
                   (va.type = 'image' OR (va.type = 'pdf' AND NOT va.has_native)) AS in_ocr_universe,
                   -- STT universe: a viewable AUDIO file.
                   (va.type = 'audio') AS in_stt_universe,
                   -- Texto: any non-empty text, native, OCR or transcription alike.
                   (va.has_native OR va.has_ocr OR va.has_stt) AS has_text
              FROM viewable_assets va
          ),
          totals AS (
            SELECT COALESCE(SUM(in_ocr_universe AND has_ocr), 0) AS ocr_count,
                   COALESCE(SUM(in_ocr_universe), 0) AS ocr_universe_count,
                   COALESCE(SUM(in_stt_universe AND has_stt), 0) AS stt_count,
                   COALESCE(SUM(in_stt_universe), 0) AS stt_universe_count,
                   COALESCE(SUM(has_text), 0) AS text_count,
                   COUNT(*) AS text_universe_count,
                   -- Embeddings: a vector without text must never count.
                   COALESCE(SUM(has_text AND has_vec), 0) AS embed_count
              FROM classified
          )
          SELECT
            (SELECT COUNT(*) FROM collections) AS collections_count,
            (SELECT COUNT(*) FROM items) AS items_count,
            totals.*,
            (SELECT COUNT(*)
               FROM processing_tasks pt
              WHERE pt.kind = 'ocr'
                AND pt.state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled')
            ) AS pending_ocr_count,
            (SELECT COUNT(*)
               FROM processing_tasks pt
              WHERE pt.kind = 'embedding'
                AND pt.state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled')
            ) AS pending_embed_count
            FROM totals
        ";

// packages/store/src/repos/item.repo.ts — findImportedFromSource, run once
// per imported file.
const IMPORT_DUPLICATE_SQL: &str = "
  SELECT id FROM items
   WHERE collection_id = ?1
     AND lower(json_extract(metadata, '$.__entropia_file_metadata.originalPath')) = lower(?2)
     AND json_extract(metadata, '$.__entropia_file_metadata.sizeBytes') = ?3
     AND json_extract(metadata, '$.__entropia_file_metadata.modifiedAt') IS ?4
   LIMIT 1";

// packages/store/src/repos/fts.repo.ts — rebuildIndex, which item and
// collection deletes used to run.
const FTS_REBUILD_SQL: &str = "
  INSERT INTO fts_items(fts_items) VALUES ('delete-all');
  INSERT INTO fts_items(rowid, item_id, title, metadata, extracted_text)
  SELECT i.rowid, i.id, i.title, COALESCE(i.metadata, ''),
    COALESCE((
      SELECT GROUP_CONCAT(text_part, ' ') FROM (
        SELECT text_part FROM (
          SELECT COALESCE(e.text_content, '') AS text_part, 0 AS source_order,
                 COALESCE(a.sort_index, 0) AS sort_index, e.created_at AS created_at
            FROM extractions e JOIN assets a ON a.id = e.asset_id WHERE a.item_id = i.id
          UNION ALL
          SELECT COALESCE(t.text_content, '') AS text_part, 1 AS source_order,
                 COALESCE(a.sort_index, 0) AS sort_index, t.created_at AS created_at
            FROM transcriptions t JOIN assets a ON a.id = t.asset_id WHERE a.item_id = i.id
        ) ordered_text
        ORDER BY source_order ASC, sort_index ASC, created_at ASC
      )
    ), '')
  FROM items i;";

#[test]
#[ignore = "scale: run on demand, see module docs"]
fn scale_ladder_step() {
    let pages: usize = std::env::var("DB_SCALE_PAGES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    let items = pages / PAGES_PER_ITEM;
    let tag = format!("[scale {pages}]");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("entropia.sqlite");
    let conn = entropia_desktop_lib::db_open_for_tests(&path);
    conn.execute_batch(SCHEMA_FIXTURE)
        .expect("apply schema fixture");

    let ((), seeded) = time(|| seed(&conn, items));
    let mb = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0) as f64 / 1_048_576.0;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    println!(
        "{tag} seeded {items} documents / {pages} pages in {}, file {:.0} MB",
        secs(seeded),
        mb(&path)
    );

    // Inicio: runs on every open of the home screen.
    let (_, d) = time(|| {
        conn.query_row(CORPUS_STATS_SQL, [], |r| r.get::<_, i64>(1))
            .unwrap()
    });
    println!("{tag} Inicio corpus stats: {}", secs(d));

    // Import: the duplicate check for a file not yet in the collection.
    let mut stmt = conn.prepare(IMPORT_DUPLICATE_SQL).unwrap();
    let (_, d) = time(|| {
        for n in 0..20 {
            let hit: Option<String> = stmt
                .query_row(
                    params!["c1", format!("C:\\Nuevo\\doc{n}.pdf"), 5, 1i64],
                    |r| r.get(0),
                )
                .ok();
            assert!(hit.is_none());
        }
    });
    let per_file = d / 20;
    println!(
        "{tag} import duplicate check: {:.0}ms per file -> 1000 more files spend {} just checking",
        per_file.as_secs_f64() * 1000.0,
        secs(per_file * 1000)
    );

    // Assistant: the RAG vector search over every embedding.
    let query: Vec<f32> = embedding(7)
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let (hits, d) =
        time(|| entropia_desktop_lib::rag_vector_leg_for_tests(&conn, &query, 20).unwrap());
    assert_eq!(hits.len(), 20);
    println!(
        "{tag} assistant vector search: {} (loads ~{:.0} MB of embeddings per question)",
        secs(d),
        (pages * EMBEDDING_DIM * 4) as f64 / 1_048_576.0
    );

    // Processing queue, per page: one claim, then the page's OCR ending,
    // which settles its blocked embedding (0038 trigger) — both inside the
    // write lock. The full settle scan now only runs once, at recovery.
    use entropia_desktop_lib::processing::repository as queue;
    let (claimed, d) = time(|| queue::claim_next(&conn, "scale", &["ocr"], 1));
    let claimed = claimed.expect("claim").map(|t| t.task_id);
    println!("{tag} queue claim: {} (claimed {claimed:?})", secs(d));
    let (_, d) = time(|| {
        conn.execute(
            "UPDATE processing_tasks SET state = 'succeeded' WHERE id = 'ocr-a0_0'",
            [],
        )
        .unwrap()
    });
    let unblocked: String = conn
        .query_row(
            "SELECT state FROM processing_tasks WHERE id = 'emb-a0_0'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        unblocked, "pending",
        "the trigger must unblock the dependent"
    );
    println!(
        "{tag} queue finish one page (trigger settles its dependent): {}",
        secs(d)
    );
    let (_, d) = time(|| queue::settle_blocked_dependents(&conn).unwrap());
    println!("{tag} recovery-only full settle scan: {}", secs(d));

    // Delete: the item and collection deletes now drop the item's own FTS
    // row by rowid; the full rebuild they used to run is kept for comparison.
    let (_, d) = time(|| {
        conn.execute(
            "DELETE FROM fts_items WHERE rowid IN (SELECT rowid FROM items WHERE id = 'i5')",
            [],
        )
        .unwrap()
    });
    println!("{tag} delete one document (its own FTS row): {}", secs(d));
    let (_, d) = time(|| conn.execute_batch(FTS_REBUILD_SQL).unwrap());
    println!(
        "{tag} full FTS rebuild (what a delete used to cost): {}",
        secs(d)
    );
}
