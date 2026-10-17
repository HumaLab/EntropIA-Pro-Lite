//! Caso de prensa del baseline: una página de *El Obrero* (Montevideo,
//! 26/08/1905, dominio público) con el artículo "La Mujer" al final.
//!
//! A diferencia del golden SOIP 1961 (`baseline.rs`), este caso corre en CI
//! sin red ni base privada: el texto OCR está versionado junto al golden.

/// Texto OCR exacto de la página, sin normalizar.
const EL_OBRERO_1905_08_26_P1: &str =
    include_str!("../../tests/fixtures/rag_golden/el_obrero_1905_08_26_p1.txt");

/// Escalares Unicode de la página. Si cambia, el archivo se corrompió.
const EL_OBRERO_CHARS: usize = 14_124;

/// Carácter (base 0, escalares Unicode) donde empieza `## La Mujer`.
const LA_MUJER_TITLE_CHAR: usize = 12_034;

const LA_MUJER_TITLE: &str = "## La Mujer";

/// Posición, en escalares Unicode, de la primera aparición de `needle`.
fn char_position(text: &str, needle: &str) -> Option<usize> {
    text.find(needle)
        .map(|byte_index| text[..byte_index].chars().count())
}

#[test]
fn el_obrero_fixture_keeps_its_exact_length_and_title_offset() {
    assert_eq!(EL_OBRERO_1905_08_26_P1.chars().count(), EL_OBRERO_CHARS);
    assert_eq!(
        char_position(EL_OBRERO_1905_08_26_P1, LA_MUJER_TITLE),
        Some(LA_MUJER_TITLE_CHAR)
    );
    assert_eq!(
        EL_OBRERO_1905_08_26_P1.matches(LA_MUJER_TITLE).count(),
        1,
        "the title marks exactly one section"
    );
}

// ── Caso de prensa offline ───────────────────────────────────────────────────
//
// Corre el pipeline real de recuperación (piernas, RRF, empaquetado) sobre la
// página, con embeddings deterministas en lugar del proveedor: CI no tiene red
// ni clave. Sin rerank (es un proveedor) y sin generación: el informe declara
// las dos etapas como no evaluadas y atribuye hasta el contexto enviado.

use rusqlite::Connection;
use serde::Serialize;

use super::baseline::{
    attribute_failure, evidence_position, Evidence, FailureAttribution, SentSource,
};
use super::params::RagParams;
use super::retrieval::{self, RetrievalQuery, RetrievalUnit};
use crate::nlp::embeddings::{
    CANONICAL_EMBEDDING_CONTRACT_V1, CANONICAL_EMBEDDING_DIMENSIONS, CANONICAL_EMBEDDING_MODEL,
};

const RAG_CHUNKS_MIGRATION_SQL: &str =
    include_str!("../../../../../packages/store/src/migrations/0029_rag_chunks.sql");

const PAGE_COLLECTION: (&str, &str) = ("col-prensa", "Prensa obrera");
const PAGE_ITEM: (&str, &str) = ("item-uy-obr-1905", "[UY-OBR-1905] El Obrero, 1905");
const PAGE_ASSET: &str = "asset-1905-08-26-page-1";
const PAGE_EXTRACTION: &str = "ext-1905-08-26-page-1";

/// Una pregunta del caso de prensa y el fragmento literal que prueba que la
/// evidencia llegó al contexto.
struct PressCase {
    id: &'static str,
    question: &'static str,
    cue: &'static str,
}

/// El primero es el caso fijo del plan; los otros dos cubren los artículos
/// restantes de la página para que una mejora en uno no rompa los demás.
const PRESS_CASES: &[PressCase] = &[
    PressCase {
        id: "el-obrero-la-mujer",
        question: "¿Qué mirada sobre la mujer tiene el periódico 'El Obrero'?",
        cue: LA_MUJER_TITLE,
    },
    PressCase {
        id: "el-obrero-la-teja",
        question: "¿Qué denunciaban los huelguistas de La Teja contra la Empresa del Puerto?",
        cue: "Empresa del Puerto",
    },
    PressCase {
        id: "el-obrero-viciosos",
        question: "¿Por qué son viciosos los trabajadores según El Obrero?",
        cue: "## Viciosos",
    },
];

/// Embedding determinista de prueba: bolsa de prefijos de palabra (5 chars,
/// minúsculas) en las dimensiones canónicas, normalizada L2. Conserva lo que
/// la pierna semántica necesita para el caso (vocabulario compartido acerca)
/// sin depender de un modelo.
fn press_embedding(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; CANONICAL_EMBEDDING_DIMENSIONS];
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 4)
    {
        let prefix: String = word.to_lowercase().chars().take(5).collect();
        let hash = prefix
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
            });
        vector[(hash % CANONICAL_EMBEDDING_DIMENSIONS as u64) as usize] += 1.0;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        vector.iter_mut().for_each(|value| *value /= norm);
    }
    vector
}

fn press_db() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory DB");
    conn.execute_batch(
        "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL);
         CREATE TABLE items (
             id TEXT PRIMARY KEY, collection_id TEXT, title TEXT NOT NULL, metadata TEXT
         );
         CREATE TABLE assets (
             id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL,
             type TEXT NOT NULL, created_at INTEGER NOT NULL
         );
         CREATE TABLE extractions (
             id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL,
             method TEXT, confidence REAL, created_at INTEGER NOT NULL
         );
         CREATE TABLE transcriptions (
             id TEXT PRIMARY KEY, asset_id TEXT UNIQUE, text_content TEXT NOT NULL,
             language TEXT, duration_ms INTEGER, model TEXT, segments TEXT,
             confidence REAL, created_at INTEGER NOT NULL
         );
         CREATE TABLE vec_assets (
             asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, embedding BLOB NOT NULL,
             embedding_model TEXT NOT NULL DEFAULT 'legacy',
             embedding_contract TEXT NOT NULL DEFAULT 'legacy',
             dimensions INTEGER NOT NULL DEFAULT 0
         );",
    )
    .expect("press schema");
    conn.execute_batch(crate::nlp::fts::FTS_ITEMS_DDL)
        .expect("fts_items");
    conn.execute_batch(RAG_CHUNKS_MIGRATION_SQL)
        .expect("rag_chunks migration");
    conn
}

/// Carga la página como la deja el procesamiento: extracción OCR, índice
/// léxico del ítem, vector del asset y, si `with_chunks`, sus chunks.
fn load_page(conn: &Connection, with_chunks: bool) {
    conn.execute(
        "INSERT INTO collections(id, name) VALUES (?1, ?2)",
        rusqlite::params![PAGE_COLLECTION.0, PAGE_COLLECTION.1],
    )
    .expect("collection");
    conn.execute(
        "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, ?2, ?3, '{}')",
        rusqlite::params![PAGE_ITEM.0, PAGE_COLLECTION.0, PAGE_ITEM.1],
    )
    .expect("item");
    conn.execute(
        "INSERT INTO assets(id, item_id, path, type, created_at)
         VALUES (?1, ?2, '1905-08-26_page_1.pdf', 'pdf', 1)",
        rusqlite::params![PAGE_ASSET, PAGE_ITEM.0],
    )
    .expect("asset");
    conn.execute(
        "INSERT INTO extractions(id, asset_id, text_content, method, confidence, created_at)
         VALUES (?1, ?2, ?3, 'ocr', 0.9, 1)",
        rusqlite::params![PAGE_EXTRACTION, PAGE_ASSET, EL_OBRERO_1905_08_26_P1],
    )
    .expect("extraction");
    crate::nlp::fts::fts_index_item(conn, PAGE_ITEM.0, PAGE_ITEM.1, "", EL_OBRERO_1905_08_26_P1)
        .expect("fts index");
    conn.execute(
        "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model,
                                embedding_contract, dimensions)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            PAGE_ASSET,
            PAGE_ITEM.0,
            crate::nlp::embeddings::floats_to_blob(&press_embedding(EL_OBRERO_1905_08_26_P1)),
            CANONICAL_EMBEDDING_MODEL,
            CANONICAL_EMBEDDING_CONTRACT_V1,
            CANONICAL_EMBEDDING_DIMENSIONS as i64
        ],
    )
    .expect("asset vector");
    if with_chunks {
        crate::nlp::embeddings::backfill_rag_chunks(
            conn,
            &crate::nlp::embeddings::RagChunkSource {
                asset_id: PAGE_ASSET.to_string(),
                item_id: PAGE_ITEM.0.to_string(),
                source_kind: crate::nlp::embeddings::RagChunkSourceKind::Extraction,
                source_id: PAGE_EXTRACTION.to_string(),
                text: EL_OBRERO_1905_08_26_P1.to_string(),
            },
            crate::nlp::embeddings::RagChunkEmbeddingSpec {
                model: CANONICAL_EMBEDDING_MODEL.to_string(),
                contract: CANONICAL_EMBEDDING_CONTRACT_V1.to_string(),
                dimensions: CANONICAL_EMBEDDING_DIMENSIONS,
            },
            |text| Ok(press_embedding(text)),
        )
        .expect("chunk backfill");
    }
}

/// Posición de la evidencia en una pierna o en la fusión.
#[derive(Debug, Serialize)]
struct StagePosition {
    stage: &'static str,
    records: usize,
    evidence_position: Option<usize>,
}

/// Informe de una pregunta: el `ranking_diagnostico` del Agente más la
/// atribución y las métricas por pasaje.
#[derive(Debug, Serialize)]
struct PressCaseReport {
    case_id: &'static str,
    unit: &'static str,
    stages: Vec<StagePosition>,
    /// Etapas que este baseline no corre y que, por eso, no se miden.
    degradation: Vec<&'static str>,
    sources_sent: usize,
    context_chars: usize,
    /// Posición (base 1) de la primera fuente enviada que lleva la evidencia.
    passage_position: Option<usize>,
    passage_recall_at_k: f64,
    passage_reciprocal_rank: f64,
    attribution: FailureAttribution,
}

fn unit_name(unit: RetrievalUnit) -> &'static str {
    match unit {
        RetrievalUnit::Asset => "asset",
        RetrievalUnit::Chunk => "chunk",
    }
}

/// `true` si el registro de una pierna cubre la evidencia: un asset la
/// contiene entera; un chunk, solo si su rango se cruza con el de la cita.
fn record_covers(
    conn: &Connection,
    unit: RetrievalUnit,
    record_id: &str,
    cue_range: (usize, usize),
) -> bool {
    match unit {
        RetrievalUnit::Asset => record_id == PAGE_ASSET,
        RetrievalUnit::Chunk => conn
            .query_row(
                "SELECT asset_id, start_char, end_char FROM rag_chunks WHERE id = ?1",
                [record_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)? as usize,
                        row.get::<_, i64>(2)? as usize,
                    ))
                },
            )
            .map(|(asset_id, start, end)| {
                asset_id == PAGE_ASSET && start < cue_range.1 && end > cue_range.0
            })
            .unwrap_or(false),
    }
}

fn evaluate_press_case(
    conn: &Connection,
    case: &PressCase,
    unit: RetrievalUnit,
    params: &RagParams,
) -> PressCaseReport {
    let cue_start = char_position(EL_OBRERO_1905_08_26_P1, case.cue).expect("cue is in the page");
    let cue_range = (cue_start, cue_start + case.cue.chars().count());
    let embedding = press_embedding(case.question);

    let (vector, lexical) = match unit {
        RetrievalUnit::Asset => (
            retrieval::vector_leg(
                conn,
                &embedding,
                params.candidates_per_leg,
                params.min_similarity,
            ),
            retrieval::lexical_leg(conn, case.question, params.candidates_per_leg),
        ),
        RetrievalUnit::Chunk => (
            retrieval::chunk_vector_leg(
                conn,
                &embedding,
                params.candidates_per_leg,
                params.min_similarity,
            ),
            retrieval::chunk_lexical_leg(conn, case.question, params.candidates_per_leg),
        ),
    };
    let legs = [
        ("vector", vector.expect("vector leg")),
        ("lexical", lexical.expect("lexical leg")),
    ];
    let fused = retrieval::rrf_fuse(
        &legs.iter().map(|(_, ids)| ids.clone()).collect::<Vec<_>>(),
        params.fusion_candidate_limit,
        params.rrf_k as f64,
    );
    let covers = |id: &String| record_covers(conn, unit, id, cue_range);
    let mut stages: Vec<StagePosition> = legs
        .iter()
        .map(|(stage, ids)| StagePosition {
            stage,
            records: ids.len(),
            evidence_position: evidence_position(ids, covers),
        })
        .collect();
    let fused_ids: Vec<String> = fused.into_iter().map(|(id, _)| id).collect();
    stages.push(StagePosition {
        stage: "fused",
        records: fused_ids.len(),
        evidence_position: evidence_position(&fused_ids, covers),
    });

    let query = RetrievalQuery {
        text: case.question,
        embedding: Some(&embedding),
    };
    let candidates = retrieval::hybrid_retrieve_candidates(conn, &[query], params, unit)
        .expect("hybrid retrieval");
    let sources = retrieval::pack_sources(
        conn,
        candidates,
        case.question,
        unit,
        params.top_k,
        params.snippet_max_chars,
        params.context_max_chars,
    )
    .expect("pack sources");

    let evidence = Evidence {
        asset_id: PAGE_ASSET,
        cue: case.cue,
    };
    let sent: Vec<SentSource<'_>> = sources
        .iter()
        .map(|source| SentSource {
            index: source.index,
            asset_id: &source.asset_id,
            text: &source.snippet,
        })
        .collect();
    let passage_position = evidence_position(&sent, |source| {
        source.asset_id == evidence.asset_id && source.text.contains(evidence.cue)
    });
    PressCaseReport {
        case_id: case.id,
        unit: unit_name(unit),
        stages,
        degradation: vec!["sin rerank", "sin generación"],
        sources_sent: sources.len(),
        context_chars: sources
            .iter()
            .map(|source| source.snippet.chars().count())
            .sum(),
        passage_position,
        passage_recall_at_k: passage_position
            .filter(|position| *position <= params.top_k)
            .map_or(0.0, |_| 1.0),
        passage_reciprocal_rank: passage_position.map_or(0.0, |position| 1.0 / position as f64),
        attribution: attribute_failure(&evidence, true, &sent, None),
    }
}

fn press_reports(unit: RetrievalUnit) -> Vec<PressCaseReport> {
    let conn = press_db();
    load_page(&conn, true);
    let params = RagParams::default();
    let reports: Vec<PressCaseReport> = PRESS_CASES
        .iter()
        .map(|case| evaluate_press_case(&conn, case, unit, &params))
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&reports).expect("press report serializes")
    );
    reports
}

#[test]
fn press_baseline_attributes_the_reference_case_to_passage_selection_in_asset_mode() {
    let reports = press_reports(RetrievalUnit::Asset);
    let reference = &reports[0];
    assert_eq!(reference.case_id, "el-obrero-la-mujer");
    // La página se recupera primera en las dos piernas y en la fusión…
    assert!(reference
        .stages
        .iter()
        .all(|stage| stage.evidence_position == Some(1)));
    // …pero el recorte que llega al LLM no contiene el artículo.
    assert_eq!(reference.attribution, FailureAttribution::PassageSelection);
    assert_eq!(reference.passage_position, None);
    assert_eq!(reference.passage_recall_at_k, 0.0);
}

#[test]
fn press_baseline_reports_every_case_with_its_attribution_and_stage_positions() {
    for unit in [RetrievalUnit::Asset, RetrievalUnit::Chunk] {
        let reports = press_reports(unit);
        assert_eq!(reports.len(), PRESS_CASES.len());
        for report in &reports {
            assert_eq!(report.unit, unit_name(unit));
            assert_eq!(
                report
                    .stages
                    .iter()
                    .map(|stage| stage.stage)
                    .collect::<Vec<_>>(),
                ["vector", "lexical", "fused"]
            );
            assert!(report.sources_sent > 0, "{} sent nothing", report.case_id);
        }
    }
}
