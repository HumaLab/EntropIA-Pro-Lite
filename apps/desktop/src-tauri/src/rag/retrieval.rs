//! Recuperación híbrida (vector + FTS5) sobre la base documental
//! (extracciones OCR + transcripciones).
//!
//! Todo el pipeline opera sobre una `&Connection` para que el comando Tauri
//! pueda envolverlo en `spawn_blocking`. El embedding de la pregunta llega
//! como parámetro — este módulo nunca toca la red, lo que mantiene cada
//! función testeable contra una base en memoria. Los parámetros de
//! recuperación (candidatos por pierna, RRF, presupuestos de snippet y
//! contexto) llegan vía [`RagParams`].

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;

use super::params::RagParams;
use super::RagSource;
use crate::nlp::vector::{cosine_distance, decode_embedding_blob};

/// Longitud mínima (en chars) de un término de la pregunta para anclar snippets.
const MIN_TERM_CHARS: usize = 4;

// El tope de candidatos RRF vive ahora en `params::DEFAULT_FUSION_CANDIDATE_LIMIT`
// (setting `rag_fusion_candidate_limit`): era la única perilla de recuperación
// que el usuario no podía tocar.

/// Metadatos y texto combinado necesarios para construir la cita de un asset.
///
/// `text_content` es el MISMO texto combinado que produce
/// `nlp::text_provider::get_asset_text`: extracción más antigua primero,
/// después la transcripción, unidas con un espacio cuando hay ambas.
/// `transcription_offset_chars` marca el char (en el texto combinado) donde
/// arranca la porción de transcripción — `None` si el asset no tiene
/// transcripción con texto.
#[derive(Debug, Clone)]
pub(crate) struct SourceRecord {
    pub asset_id: String,
    pub item_id: String,
    pub item_title: String,
    pub collection_id: String,
    pub collection_name: String,
    pub text_content: String,
    pub segments_json: Option<String>,
    pub transcription_offset_chars: Option<usize>,
}

/// Owned RRF result. Keeping the full record here lets the SQLite lock be
/// released before optional reranking and final source construction.
#[derive(Debug, Clone)]
pub(crate) struct RrfCandidate {
    pub(crate) record: SourceRecord,
    pub(crate) score: f64,
}

#[derive(Debug, Deserialize)]
struct TranscriptSegment {
    start: f64,
    end: f64,
    text: String,
}

/// Recuperación híbrida hasta la lista RRF de candidatos. Los registros son
/// owned para que cualquier reranking ocurra después de liberar SQLite.
pub(crate) fn hybrid_retrieve_candidates(
    conn: &Connection,
    question: &str,
    query_embedding: Option<&[f32]>,
    params: &RagParams,
) -> Result<Vec<RrfCandidate>, String> {
    let vector = match query_embedding {
        Some(embedding) => vector_leg(
            conn,
            embedding,
            params.candidates_per_leg,
            params.min_similarity,
        )?,
        None => Vec::new(),
    };
    let lexical = lexical_leg(conn, question, params.candidates_per_leg)?;
    let fused = rrf_fuse(
        &[vector, lexical],
        params.fusion_candidate_limit,
        params.rrf_k as f64,
    );

    // Solo materializamos los `rerank_depth` mejores. `load_source_record` lee
    // el texto COMPLETO del asset (extracción + transcripción, megabytes en
    // audios largos) y nada aguas abajo mira más allá de esa profundidad: el
    // reranker recorta ahí y `build_sources` toma `top_k <= rerank_depth`.
    // Cargar los 40 fusionados para citar 6 era I/O y trabajo UTF-8 tirado (F3).
    //
    // Iteramos la lista fusionada hasta JUNTAR esa cantidad, en vez de cortarla
    // de antemano: un asset borrado en carrera devuelve `None` y el siguiente
    // candidato ocupa su lugar, igual que antes.
    let wanted = params.rerank_depth.max(params.top_k).max(1);
    let mut candidates = Vec::with_capacity(wanted.min(fused.len()));
    for (asset_id, score) in fused {
        if candidates.len() >= wanted {
            break;
        }
        if let Some(record) = load_source_record(conn, &asset_id)? {
            candidates.push(RrfCandidate { record, score });
        }
    }

    Ok(candidates)
}

#[cfg(test)]
fn hybrid_retrieve(
    conn: &Connection,
    question: &str,
    query_embedding: Option<&[f32]>,
    params: &RagParams,
) -> Result<Vec<RagSource>, String> {
    let candidates = hybrid_retrieve_candidates(conn, question, query_embedding, params)?;
    Ok(build_sources(
        candidates,
        question,
        params.top_k,
        params.snippet_max_chars,
        params.context_max_chars,
    ))
}

/// Pierna vectorial: kNN por similitud coseno sobre `vec_assets`, restringida
/// a assets con texto (extracción O transcripción no vacía, mismo idioma que
/// `summarize_asset_embedding_coverage`). Devuelve asset_ids ordenados (mejor
/// primero). Embeddings con dimensión distinta a la del query se saltean en
/// runtime, lo que permite incluir tanto embeddings canónicos como legacy
/// siempre que sean dimensionalmente compatibles.
/// `min_similarity > 0.0` descarta candidatos con similitud menor ANTES del
/// ranking; `0.0` deshabilita el filtro (las similitudes negativas se quedan).
pub(crate) fn vector_leg(
    conn: &Connection,
    query_embedding: &[f32],
    limit: usize,
    min_similarity: f64,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT v.asset_id, v.embedding
             FROM vec_assets v
             WHERE (
                   EXISTS(SELECT 1 FROM extractions e
                          WHERE e.asset_id = v.asset_id
                            AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0)
                   OR EXISTS(SELECT 1 FROM transcriptions t
                             WHERE t.asset_id = v.asset_id
                               AND LENGTH(TRIM(COALESCE(t.text_content, ''))) > 0)
               )",
        )
        .map_err(|e| format!("Failed to prepare RAG vector query: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| format!("Failed to run RAG vector query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read RAG vector rows: {e}"))?;

    let mut scored: Vec<(String, f64)> = rows
        .into_iter()
        .filter_map(|(asset_id, blob)| {
            let embedding = decode_embedding_blob(&blob).ok()?;
            if embedding.len() != query_embedding.len() {
                return None;
            }
            let distance = cosine_distance(query_embedding, &embedding)?;
            let similarity = 1.0 - distance;
            if min_similarity > 0.0 && similarity < min_similarity {
                return None;
            }
            Some((asset_id, similarity))
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored
        .into_iter()
        .take(limit)
        .map(|(asset_id, _)| asset_id)
        .collect())
}

/// Pierna léxica: BM25 a nivel ítem vía FTS5, aplanada a los assets con texto
/// (extracción O transcripción no vacía) de cada ítem preservando el orden de
/// relevancia. El rank léxico de un asset es su posición en esta lista
/// aplanada.
pub(crate) fn lexical_leg(
    conn: &Connection,
    question: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    // El input es una pregunta en lenguaje natural, no una lista de keywords:
    // con el AND implícito de FTS5 cada `que`/`el`/`sobre` sería obligatorio y
    // esta pierna devolvía cero filas para cualquier pregunta real (F1).
    let items = crate::nlp::fts::fts_search_with_mode(
        conn,
        question,
        None,
        crate::nlp::fts::FtsMatchMode::Any,
    )?;

    let mut stmt = conn
        .prepare(
            "SELECT a.id
             FROM assets a
             WHERE a.item_id = ?1
               AND (EXISTS(SELECT 1 FROM extractions e
                           WHERE e.asset_id = a.id
                             AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0)
                    OR EXISTS(SELECT 1 FROM transcriptions t
                              WHERE t.asset_id = a.id
                                AND LENGTH(TRIM(COALESCE(t.text_content, ''))) > 0))
             ORDER BY a.created_at ASC, a.id ASC
             LIMIT ?2",
        )
        .map_err(|e| format!("Failed to prepare RAG lexical asset query: {e}"))?;

    // Assets por ítem, en orden de relevancia BM25 del ítem. El `LIMIT` acota
    // cada ítem al presupuesto total: ningún ítem necesita más que eso.
    let mut per_item: Vec<Vec<String>> = Vec::new();
    for item in items.iter().take(limit) {
        let ids = stmt
            .query_map(rusqlite::params![item.item_id, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("Failed to run RAG lexical asset query: {e}"))?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|e| format!("Failed to read RAG lexical asset rows: {e}"))?;

        if !ids.is_empty() {
            per_item.push(ids);
        }
    }

    // Intercalado round-robin en vez de drenar ítem por ítem: un solo ítem con
    // muchos assets consumía TODO el presupuesto de la pierna y descartaba a
    // los demás ítems que también matcheaban (F7). La primera ronda sigue
    // siendo el mejor asset de cada ítem en orden de relevancia, así que el
    // ranking de cabeza no cambia.
    let mut assets = Vec::new();
    let mut depth = 0usize;
    loop {
        let mut advanced = false;
        for item_assets in &per_item {
            if let Some(asset_id) = item_assets.get(depth) {
                assets.push(asset_id.clone());
                advanced = true;
                if assets.len() >= limit {
                    return Ok(assets);
                }
            }
        }
        if !advanced {
            return Ok(assets);
        }
        depth += 1;
    }
}

/// Reciprocal Rank Fusion: score(asset) = Σ sobre piernas de 1/(rrf_k + rank),
/// con rank arrancando en 1. Orden descendente por score; los empates se
/// resuelven determinísticamente por asset_id ascendente.
pub(crate) fn rrf_fuse(legs: &[Vec<String>], limit: usize, rrf_k: f64) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    for leg in legs {
        for (rank0, asset_id) in leg.iter().enumerate() {
            let rank = (rank0 + 1) as f64;
            *scores.entry(asset_id.clone()).or_default() += 1.0 / (rrf_k + rank);
        }
    }

    let mut fused: Vec<(String, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    fused.truncate(limit);
    fused
}

/// Carga los metadatos de citación y el texto combinado de un asset. `None`
/// si el asset ya no existe (carrera con un borrado, por ejemplo).
///
/// El texto combinado replica la semántica de `get_asset_text`: texto de la
/// extracción más antigua primero, después el de la transcripción más
/// antigua, unidos con un solo espacio cuando hay ambos. El offset de la
/// porción de transcripción queda registrado para gatear los timestamps.
pub(crate) fn load_source_record(
    conn: &Connection,
    asset_id: &str,
) -> Result<Option<SourceRecord>, String> {
    let metadata = conn
        .query_row(
            "SELECT a.id, a.item_id, i.title,
                    COALESCE(i.collection_id, ''), COALESCE(c.name, '')
             FROM assets a
             JOIN items i ON i.id = a.item_id
             LEFT JOIN collections c ON c.id = i.collection_id
             WHERE a.id = ?1
             LIMIT 1",
            rusqlite::params![asset_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("Failed to load RAG source metadata for asset '{asset_id}': {e}"))?;

    let Some((asset_id, item_id, item_title, collection_id, collection_name)) = metadata else {
        return Ok(None);
    };

    // Extracción más antigua (misma convención que get_asset_text).
    let extraction_text: String = conn
        .query_row(
            "SELECT COALESCE(text_content, '')
             FROM extractions
             WHERE asset_id = ?1
             ORDER BY created_at ASC
             LIMIT 1",
            rusqlite::params![asset_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to load RAG extraction text for asset '{asset_id}': {e}"))?
        .unwrap_or_default();

    // Transcripción más antigua: texto + segments para los timestamps.
    let (transcription_text, segments_json): (String, Option<String>) = conn
        .query_row(
            "SELECT COALESCE(text_content, ''), segments
             FROM transcriptions
             WHERE asset_id = ?1
             ORDER BY created_at ASC
             LIMIT 1",
            rusqlite::params![asset_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("Failed to load RAG transcription for asset '{asset_id}': {e}"))?
        .unwrap_or((String::new(), None));

    // Texto combinado con la MISMA semántica de get_asset_text: partes no
    // vacías unidas con un espacio, extracción primero.
    let mut text_content = extraction_text;
    let mut transcription_offset_chars = None;
    if !transcription_text.is_empty() {
        if !text_content.is_empty() {
            text_content.push(' ');
        }
        transcription_offset_chars = Some(text_content.chars().count());
        text_content.push_str(&transcription_text);
    }

    Ok(Some(SourceRecord {
        asset_id,
        item_id,
        item_title,
        collection_id,
        collection_name,
        text_content,
        segments_json,
        transcription_offset_chars,
    }))
}

/// Convierte los registros fusionados (en orden) en fuentes citables con
/// snippet y timestamps, frenando cuando el contexto total supera el tope.
/// Si el PRIMER snippet ya supera el tope, se trunca en vez de descartarse:
/// con registros disponibles nunca se devuelven cero fuentes.
pub(crate) fn build_sources(
    candidates: Vec<RrfCandidate>,
    question: &str,
    top_k: usize,
    snippet_max_chars: usize,
    context_max_chars: usize,
) -> Vec<RagSource> {
    let terms = extract_query_terms(question);
    let mut sources: Vec<RagSource> = Vec::new();
    let mut total_chars = 0usize;

    for RrfCandidate { record, score } in candidates.into_iter().take(top_k) {
        let (mut snippet, window_start) =
            snippet_window(&record.text_content, &terms, snippet_max_chars);
        let snippet_chars = snippet.chars().count();
        if total_chars + snippet_chars > context_max_chars {
            if !sources.is_empty() {
                // La lista de fuentes debe reflejar exactamente lo que entra al prompt.
                break;
            }
            // Garantía defensiva: el primer registro nunca deja la respuesta
            // sin fuentes — se trunca (char-safe) al presupuesto de contexto.
            snippet = snippet.chars().take(context_max_chars).collect();
        }
        total_chars += snippet.chars().count();

        let timestamps = resolve_source_timestamps(&record, &terms, window_start);

        sources.push(RagSource {
            index: (sources.len() + 1) as u32,
            asset_id: record.asset_id,
            item_id: record.item_id,
            item_title: record.item_title,
            collection_id: record.collection_id,
            collection_name: record.collection_name,
            snippet,
            score,
            start_seconds: timestamps.map(|(start, _)| start),
            end_seconds: timestamps.map(|(_, end)| end),
        });
    }

    sources
}

/// Términos de la pregunta para anclar snippets: split por whitespace, se
/// recorta puntuación en los bordes, lowercase, se conservan términos de
/// 4+ chars, ordenados del más largo al más corto (sin duplicados).
pub(crate) fn extract_query_terms(question: &str) -> Vec<String> {
    let mut terms: Vec<String> = question
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| word.chars().count() >= MIN_TERM_CHARS)
        .collect();

    // Más largo primero: el término más específico ancla la ventana.
    terms.sort_by_key(|term| std::cmp::Reverse(term.chars().count()));

    let mut seen = HashSet::new();
    terms.retain(|term| seen.insert(term.clone()));
    terms
}

/// Ventana de snippet centrada en la primera ocurrencia (case-insensitive)
/// del término más largo encontrado; si ningún término aparece, arranca al
/// inicio del texto. Opera SIEMPRE sobre chars (texto Unicode en español).
///
/// Devuelve `(snippet, índice_de_char_donde_arranca_la_ventana)`.
pub(crate) fn snippet_window(text: &str, terms: &[String], max_chars: usize) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return (text.to_string(), 0);
    }

    // Lowercase 1:1 por char para mantener alineados los índices (suficiente
    // para español: Á→á, Ñ→ñ son todos mapeos de un char).
    let lowered: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();

    let match_pos = terms.iter().find_map(|term| find_chars(&lowered, term));

    let start = match match_pos {
        Some(pos) => pos
            .saturating_sub(max_chars / 2)
            .min(chars.len().saturating_sub(max_chars)),
        None => 0,
    };

    let snippet: String = chars[start..].iter().take(max_chars).collect();
    (snippet, start)
}

/// Busca `needle` (ya en lowercase) dentro de `haystack` (chars en lowercase).
/// Devuelve el índice de char del primer match.
fn find_chars(haystack: &[char], needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle.as_slice())
}

/// Timestamps de un registro con texto combinado: SOLO se resuelven cuando el
/// ancla del snippet (el término matcheado o, sin match, el inicio de la
/// ventana) cae dentro de la porción de transcripción. La posición se traduce
/// a chars relativos a la transcripción antes de la resolución de segmentos
/// existente. Snippets anclados en la extracción → `None`.
fn resolve_source_timestamps(
    record: &SourceRecord,
    terms: &[String],
    window_start: usize,
) -> Option<(f64, f64)> {
    let offset = record.transcription_offset_chars?;
    let anchor = find_term_anchor(&record.text_content, terms).unwrap_or(window_start);
    if anchor < offset {
        return None;
    }
    resolve_timestamps(record.segments_json.as_deref(), terms, anchor - offset)
}

/// Posición (en chars) de la primera ocurrencia del término más largo que
/// aparece en el texto, con el mismo lowering 1:1 por char de `snippet_window`
/// (los términos llegan ordenados del más largo al más corto).
fn find_term_anchor(text: &str, terms: &[String]) -> Option<usize> {
    let lowered: Vec<char> = text
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    terms.iter().find_map(|term| find_chars(&lowered, term))
}

/// Resuelve timestamps desde el JSON de `transcriptions.segments`:
/// 1. Primer segmento cuyo texto contiene un término buscado (case-insensitive).
/// 2. Si no, el segmento que solapa el inicio de la ventana por longitud
///    acumulada de texto.
/// 3. `None` si segments es NULL, JSON inválido o vacío.
pub(crate) fn resolve_timestamps(
    segments_json: Option<&str>,
    terms: &[String],
    window_start_char: usize,
) -> Option<(f64, f64)> {
    let raw = segments_json?.trim();
    if raw.is_empty() {
        return None;
    }
    let segments: Vec<TranscriptSegment> = serde_json::from_str(raw).ok()?;
    if segments.is_empty() {
        return None;
    }

    for segment in &segments {
        let lowered = segment.text.to_lowercase();
        if terms.iter().any(|term| lowered.contains(term.as_str())) {
            return Some((segment.start, segment.end));
        }
    }

    let mut cumulative = 0usize;
    for segment in &segments {
        let len = segment.text.chars().count();
        if window_start_char < cumulative + len {
            return Some((segment.start, segment.end));
        }
        cumulative += len;
    }

    None
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlp::embeddings::{
        CANONICAL_EMBEDDING_CONTRACT_V1, CANONICAL_EMBEDDING_DIMENSIONS, CANONICAL_EMBEDDING_MODEL,
    };
    use rusqlite::params;

    fn floats_to_blob(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    fn setup_rag_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory DB failed");
        conn.execute_batch(
            r#"
            CREATE TABLE collections (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL
            );

            CREATE TABLE items (
                id TEXT PRIMARY KEY,
                collection_id TEXT,
                title TEXT NOT NULL,
                metadata TEXT
            );

            CREATE TABLE assets (
                id TEXT PRIMARY KEY,
                item_id TEXT NOT NULL,
                path TEXT NOT NULL,
                type TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE transcriptions (
                id TEXT PRIMARY KEY,
                asset_id TEXT UNIQUE,
                text_content TEXT NOT NULL,
                language TEXT,
                duration_ms INTEGER,
                model TEXT,
                segments TEXT,
                confidence REAL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE extractions (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL,
                text_content TEXT NOT NULL,
                method TEXT,
                confidence REAL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE vec_assets (
                asset_id TEXT PRIMARY KEY,
                item_id TEXT NOT NULL,
                embedding BLOB NOT NULL,
                embedding_model TEXT NOT NULL DEFAULT 'legacy',
                embedding_contract TEXT NOT NULL DEFAULT 'legacy',
                dimensions INTEGER NOT NULL DEFAULT 0
            );

            CREATE VIRTUAL TABLE fts_items USING fts5(
                item_id UNINDEXED,
                title,
                metadata,
                extracted_text,
                tokenize = 'unicode61 remove_diacritics 1',
                content = ''
            );
            "#,
        )
        .expect("RAG test schema creation failed");
        conn
    }

    fn insert_doc(
        conn: &Connection,
        collection: (&str, &str),
        item: (&str, &str),
        asset_id: &str,
        text: &str,
        segments: Option<&str>,
        embedding: Option<&[f32]>,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO collections(id, name) VALUES (?1, ?2)",
            params![collection.0, collection.1],
        )
        .expect("collection insert");
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, ?2, ?3, '{}')",
            params![item.0, collection.0, item.1],
        )
        .expect("item insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, 'audio.mp3', 'audio', 1)",
            params![asset_id, item.0],
        )
        .expect("asset insert");
        conn.execute(
            "INSERT INTO transcriptions(id, asset_id, text_content, model, segments, created_at)
             VALUES (?1, ?2, ?3, 'base', ?4, 1)",
            params![format!("tr-{asset_id}"), asset_id, text, segments],
        )
        .expect("transcription insert");
        if let Some(embedding) = embedding {
            conn.execute(
                "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    asset_id,
                    item.0,
                    floats_to_blob(embedding),
                    CANONICAL_EMBEDDING_MODEL,
                    CANONICAL_EMBEDDING_CONTRACT_V1,
                    CANONICAL_EMBEDDING_DIMENSIONS as i64
                ],
            )
            .expect("embedding insert");
        }
        crate::nlp::fts::fts_index_item(conn, item.0, item.1, "", text).expect("fts index");
    }

    fn insert_extraction(conn: &Connection, asset_id: &str, text: &str, created_at: i64) {
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, method, confidence, created_at)
             VALUES (?1, ?2, ?3, 'ocr', 0.9, ?4)",
            params![
                format!("ext-{asset_id}-{created_at}"),
                asset_id,
                text,
                created_at
            ],
        )
        .expect("extraction insert");
    }

    /// Documento SOLO-OCR: colección + ítem + asset + extracción (sin
    /// transcripción), indexado en FTS y con embedding opcional.
    fn insert_ocr_doc(
        conn: &Connection,
        collection: (&str, &str),
        item: (&str, &str),
        asset_id: &str,
        text: &str,
        embedding: Option<&[f32]>,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO collections(id, name) VALUES (?1, ?2)",
            params![collection.0, collection.1],
        )
        .expect("collection insert");
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, ?2, ?3, '{}')",
            params![item.0, collection.0, item.1],
        )
        .expect("item insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, 'scan.png', 'image', 1)",
            params![asset_id, item.0],
        )
        .expect("asset insert");
        insert_extraction(conn, asset_id, text, 1);
        if let Some(embedding) = embedding {
            conn.execute(
                "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    asset_id,
                    item.0,
                    floats_to_blob(embedding),
                    CANONICAL_EMBEDDING_MODEL,
                    CANONICAL_EMBEDDING_CONTRACT_V1,
                    CANONICAL_EMBEDDING_DIMENSIONS as i64
                ],
            )
            .expect("embedding insert");
        }
        crate::nlp::fts::fts_index_item(conn, item.0, item.1, "", text).expect("fts index");
    }

    fn record(asset_id: &str, text: &str) -> SourceRecord {
        SourceRecord {
            asset_id: asset_id.to_string(),
            item_id: format!("item-{asset_id}"),
            item_title: "Título".to_string(),
            collection_id: "col".to_string(),
            collection_name: "Colección".to_string(),
            text_content: text.to_string(),
            segments_json: None,
            transcription_offset_chars: None,
        }
    }

    fn candidate(asset_id: &str, text: &str, score: f64) -> RrfCandidate {
        RrfCandidate {
            record: record(asset_id, text),
            score,
        }
    }

    // ── RRF fusion ───────────────────────────────────────────────────────────

    #[test]
    fn rrf_fuse_asset_in_both_legs_beats_single_leg() {
        let legs = vec![
            vec!["a".to_string(), "b".to_string()],
            vec!["a".to_string(), "c".to_string()],
        ];
        let fused = rrf_fuse(&legs, 10, 60.0);
        assert_eq!(fused[0].0, "a");
        let expected = 2.0 / 61.0;
        assert!((fused[0].1 - expected).abs() < 1e-12);
        // b y c quedan detrás con un solo aporte de rank 2.
        assert!(fused[1].1 < fused[0].1);
    }

    #[test]
    fn rrf_fuse_orders_by_rank_within_leg() {
        let legs = vec![vec!["a".to_string(), "b".to_string(), "c".to_string()]];
        let fused = rrf_fuse(&legs, 10, 60.0);
        let ids: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        assert!(fused[0].1 > fused[1].1 && fused[1].1 > fused[2].1);
    }

    #[test]
    fn rrf_fuse_breaks_ties_deterministically_by_asset_id() {
        // Mismo rank (1) en piernas distintas → mismo score → orden por id.
        let legs = vec![vec!["zeta".to_string()], vec!["alfa".to_string()]];
        let fused = rrf_fuse(&legs, 10, 60.0);
        assert_eq!(fused[0].0, "alfa");
        assert_eq!(fused[1].0, "zeta");
        assert!((fused[0].1 - fused[1].1).abs() < 1e-12);
    }

    #[test]
    fn rrf_fuse_truncates_to_limit() {
        let legs = vec![vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ]];
        let fused = rrf_fuse(&legs, 2, 60.0);
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].0, "a");
    }

    // ── Query terms ──────────────────────────────────────────────────────────

    #[test]
    fn extract_query_terms_filters_lowercases_and_sorts_longest_first() {
        let terms = extract_query_terms("¿Cuándo llegó Belgrano al Cabildo de Buenos Aires?");
        assert!(terms.contains(&"belgrano".to_string()));
        assert!(terms.contains(&"cabildo".to_string()));
        assert!(terms.contains(&"cuándo".to_string()));
        assert!(!terms.iter().any(|t| t == "de" || t == "al"));

        let lengths: Vec<usize> = terms.iter().map(|t| t.chars().count()).collect();
        let mut sorted = lengths.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(lengths, sorted, "longest terms must come first");
    }

    #[test]
    fn extract_query_terms_dedupes_repeated_words() {
        let terms = extract_query_terms("cabildo CABILDO cabildo,");
        assert_eq!(terms, vec!["cabildo".to_string()]);
    }

    // ── Snippet window ───────────────────────────────────────────────────────

    #[test]
    fn snippet_window_centers_on_term_found_mid_text() {
        let pre = "relleno ".repeat(100); // 800 chars
        let post = " cola".repeat(100);
        let text = format!("{pre}cabildo{post}");
        let terms = vec!["cabildo".to_string()];
        let (snippet, start) = snippet_window(&text, &terms, 200);
        assert!(snippet.to_lowercase().contains("cabildo"));
        assert!(start > 0, "window should not start at text begin");
        assert!(snippet.chars().count() <= 200);
    }

    #[test]
    fn snippet_window_falls_back_to_text_start_when_term_missing() {
        let text = "x".repeat(500);
        let (snippet, start) = snippet_window(&text, &["cabildo".to_string()], 100);
        assert_eq!(start, 0);
        assert_eq!(snippet.chars().count(), 100);
    }

    #[test]
    fn snippet_window_short_text_returned_whole() {
        let text = "texto corto con ñandú";
        let (snippet, start) = snippet_window(text, &[], 100);
        assert_eq!(snippet, text);
        assert_eq!(start, 0);
    }

    #[test]
    fn snippet_window_is_multibyte_safe() {
        let text = format!("{}TÉRMINO{}🦉🦉🦉", "ñ".repeat(50), "á".repeat(400));
        let terms = vec!["término".to_string()];
        let (snippet, _start) = snippet_window(&text, &terms, 80);
        assert!(snippet.chars().count() <= 80);
        assert!(snippet.to_lowercase().contains("término"));
    }

    #[test]
    fn snippet_window_term_near_end_clamps_window() {
        let text = format!("{}objetivo🦉", "relleno ".repeat(50));
        let terms = vec!["objetivo".to_string()];
        let (snippet, start) = snippet_window(&text, &terms, 100);
        assert!(snippet.to_lowercase().contains("objetivo"));
        assert_eq!(snippet.chars().count(), 100);
        assert_eq!(start, text.chars().count() - 100);
    }

    // ── Segment timestamps ───────────────────────────────────────────────────

    #[test]
    fn resolve_timestamps_finds_segment_containing_term() {
        let segments = r#"[{"start":0.0,"end":2.0,"text":"hola mundo"},{"start":2.0,"end":5.5,"text":"el Cabildo abierto"}]"#;
        let result = resolve_timestamps(Some(segments), &["cabildo".to_string()], 0);
        assert_eq!(result, Some((2.0, 5.5)));
    }

    #[test]
    fn resolve_timestamps_falls_back_to_cumulative_window_overlap() {
        let segments = r#"[{"start":0.0,"end":2.0,"text":"0123456789"},{"start":2.0,"end":4.0,"text":"abcdefghij"}]"#;
        // La ventana arranca en el char 12 → cae dentro del segundo segmento.
        let result = resolve_timestamps(Some(segments), &["zzzz".to_string()], 12);
        assert_eq!(result, Some((2.0, 4.0)));
    }

    #[test]
    fn resolve_timestamps_none_on_null_or_garbage() {
        assert_eq!(resolve_timestamps(None, &[], 0), None);
        assert_eq!(resolve_timestamps(Some("not json"), &[], 0), None);
        assert_eq!(resolve_timestamps(Some("[]"), &[], 0), None);
        assert_eq!(resolve_timestamps(Some("   "), &[], 0), None);
        assert_eq!(resolve_timestamps(Some(r#"[{"foo": 1}]"#), &[], 0), None);
    }

    #[test]
    fn resolve_timestamps_none_when_window_beyond_segments() {
        let segments = r#"[{"start":0.0,"end":2.0,"text":"corto"}]"#;
        let result = resolve_timestamps(Some(segments), &["zzzz".to_string()], 999);
        assert_eq!(result, None);
    }

    // ── build_sources ────────────────────────────────────────────────────────

    #[test]
    fn build_sources_stops_when_context_budget_is_exceeded() {
        let candidates = vec![
            candidate("a", &"a".repeat(40), 0.9),
            candidate("b", &"b".repeat(40), 0.8),
            candidate("c", &"c".repeat(40), 0.7),
        ];
        // 40 + 40 = 80 entra; sumar el tercero (120) supera 90 → corta.
        let sources = build_sources(candidates, "pregunta", 3, 100, 90);
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].index, 1);
        assert_eq!(sources[1].index, 2);
    }

    #[test]
    fn build_sources_truncates_first_snippet_larger_than_context_budget() {
        // El snippet del mejor registro (60 chars, multibyte) excede el
        // presupuesto total (25): se trunca char-safe en vez de devolver cero
        // fuentes, y el siguiente registro ya no entra.
        let candidates = vec![
            candidate("a", &"ñ".repeat(60), 0.9),
            candidate("b", &"b".repeat(10), 0.8),
        ];
        let sources = build_sources(candidates, "pregunta", 2, 100, 25);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].asset_id, "a");
        assert_eq!(sources[0].snippet.chars().count(), 25);
        assert_eq!(sources[0].snippet, "ñ".repeat(25));
    }

    #[test]
    fn build_sources_caps_each_snippet() {
        let candidates = vec![candidate("a", &"palabra ".repeat(100), 1.0)];
        let sources = build_sources(candidates, "pregunta", 1, 50, 1000);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].snippet.chars().count(), 50);
    }

    #[test]
    fn build_sources_enforces_final_top_k_and_preserves_ranked_scores() {
        let candidates = vec![
            candidate("reranked-first", "first", 0.97),
            candidate("reranked-second", "second", 0.42),
            candidate("excluded", "third", 0.1),
        ];

        let sources = build_sources(candidates, "pregunta", 2, 100, 1000);

        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].index, 1);
        assert_eq!(sources[0].asset_id, "reranked-first");
        assert_eq!(sources[0].score, 0.97);
        assert_eq!(sources[1].index, 2);
        assert_eq!(sources[1].asset_id, "reranked-second");
        assert_eq!(sources[1].score, 0.42);
    }

    // ── Retrieval integration (in-memory DB) ─────────────────────────────────

    #[test]
    fn hybrid_retrieve_fuses_vector_and_lexical_legs() {
        let conn = setup_rag_db();
        let query_embedding = [1.0_f32, 0.0, 0.0];

        // En ambas piernas: texto con el término (x2) + embedding cercano.
        insert_doc(
            &conn,
            ("col-1", "Archivo General"),
            ("item-both", "Acta del Cabildo"),
            "asset-both",
            "El cabildo abierto convocó al cabildo en mayo",
            Some(r#"[{"start":1.5,"end":4.0,"text":"El cabildo abierto"}]"#),
            Some(&[0.9, 0.1, 0.0]),
        );

        // Solo vectorial: máxima similitud pero sin el término.
        insert_doc(
            &conn,
            ("col-1", "Archivo General"),
            ("item-vec", "Memoria oral"),
            "asset-vec",
            "Una memoria sobre la vida cotidiana en la aldea",
            None,
            Some(&[1.0, 0.0, 0.0]),
        );

        // Solo léxica: contiene el término una vez, sin embedding.
        insert_doc(
            &conn,
            ("col-2", "Hemeroteca"),
            ("item-fts", "Crónica de mayo"),
            "asset-fts",
            "La crónica menciona el cabildo una vez",
            None,
            None,
        );

        // Dimensión incompatible: la pierna vectorial debe saltearlo.
        insert_doc(
            &conn,
            ("col-2", "Hemeroteca"),
            ("item-bad", "Vector corrupto"),
            "asset-bad",
            "Texto sin relación alguna",
            None,
            Some(&[0.5, 0.5]),
        );

        let sources = hybrid_retrieve(
            &conn,
            "cabildo",
            Some(&query_embedding),
            &RagParams::default(),
        )
        .expect("hybrid retrieval should succeed");

        let ids: Vec<&str> = sources.iter().map(|s| s.asset_id.as_str()).collect();
        assert_eq!(ids, vec!["asset-both", "asset-vec", "asset-fts"]);

        // Índices 1-based contiguos y scores RRF descendentes.
        assert_eq!(sources[0].index, 1);
        assert_eq!(sources[1].index, 2);
        assert_eq!(sources[2].index, 3);
        assert!(sources[0].score > sources[1].score);
        assert!(sources[1].score > sources[2].score);

        // Metadatos de citación.
        assert_eq!(sources[0].item_id, "item-both");
        assert_eq!(sources[0].item_title, "Acta del Cabildo");
        assert_eq!(sources[0].collection_id, "col-1");
        assert_eq!(sources[0].collection_name, "Archivo General");
        assert!(sources[0].snippet.contains("cabildo"));

        // Timestamps desde segments; None cuando no hay segments.
        assert_eq!(sources[0].start_seconds, Some(1.5));
        assert_eq!(sources[0].end_seconds, Some(4.0));
        assert_eq!(sources[1].start_seconds, None);
        assert_eq!(sources[1].end_seconds, None);
    }

    #[test]
    fn hybrid_retrieve_without_embedding_degrades_to_fts_only() {
        let conn = setup_rag_db();
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-1", "Acta"),
            "asset-fts",
            "El cabildo sesionó en pleno",
            None,
            None,
        );

        let sources = hybrid_retrieve(&conn, "cabildo", None, &RagParams::default())
            .expect("fts-only retrieval should work");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].asset_id, "asset-fts");
    }

    #[test]
    fn hybrid_retrieve_empty_db_returns_no_sources() {
        let conn = setup_rag_db();
        let sources = hybrid_retrieve(&conn, "cabildo", Some(&[1.0, 0.0]), &RagParams::default())
            .expect("empty retrieval should succeed");
        assert!(sources.is_empty());
    }

    #[test]
    fn hybrid_candidates_are_capped_independently_of_final_top_k() {
        let conn = setup_rag_db();
        let mut params = RagParams {
            top_k: 2,
            candidates_per_leg: 50,
            ..RagParams::default()
        };
        params.context_max_chars = 60_000;

        for index in 0..45 {
            insert_doc(
                &conn,
                ("col-1", "Archivo"),
                (&format!("item-{index:02}"), "Acta"),
                &format!("asset-{index:02}"),
                "cabildo documento",
                None,
                Some(&[1.0, 0.0, 0.0]),
            );
        }

        let candidates =
            hybrid_retrieve_candidates(&conn, "cabildo", Some(&[1.0, 0.0, 0.0]), &params)
                .expect("hybrid candidate retrieval should succeed");

        // El tope de materialización es `rerank_depth`, no el tope de fusión:
        // cargar el texto completo de los 40 fusionados para citar 2 era I/O
        // tirada (F3). Sigue siendo independiente de `top_k`, que es lo que
        // este test protege.
        assert_eq!(candidates.len(), params.rerank_depth);
        assert!(candidates.len() > params.top_k);
    }

    /// F3: subir `rerank_depth` materializa más candidatos, y bajarlo menos —
    /// el tope de fusión sigue acotando por arriba.
    #[test]
    fn materialized_candidates_follow_rerank_depth_not_the_fusion_limit() {
        let conn = setup_rag_db();
        for index in 0..45 {
            insert_doc(
                &conn,
                ("col-1", "Archivo"),
                (&format!("item-{index:02}"), "Acta"),
                &format!("asset-{index:02}"),
                "cabildo documento",
                None,
                Some(&[1.0, 0.0, 0.0]),
            );
        }

        let shallow = RagParams {
            top_k: 2,
            candidates_per_leg: 50,
            rerank_depth: 5,
            context_max_chars: 60_000,
            ..RagParams::default()
        };
        let deep = RagParams {
            rerank_depth: 30,
            ..shallow
        };

        let shallow_candidates =
            hybrid_retrieve_candidates(&conn, "cabildo", Some(&[1.0, 0.0, 0.0]), &shallow)
                .expect("shallow retrieval should succeed");
        let deep_candidates =
            hybrid_retrieve_candidates(&conn, "cabildo", Some(&[1.0, 0.0, 0.0]), &deep)
                .expect("deep retrieval should succeed");

        assert_eq!(shallow_candidates.len(), 5);
        assert_eq!(deep_candidates.len(), 30);
    }

    /// La profundidad nunca puede dejar menos candidatos que fuentes finales,
    /// incluso si un `RagParams` construido a mano la pone por debajo.
    #[test]
    fn materialized_candidates_never_drop_below_top_k() {
        let conn = setup_rag_db();
        for index in 0..10 {
            insert_doc(
                &conn,
                ("col-1", "Archivo"),
                (&format!("item-{index:02}"), "Acta"),
                &format!("asset-{index:02}"),
                "cabildo documento",
                None,
                Some(&[1.0, 0.0, 0.0]),
            );
        }

        let params = RagParams {
            top_k: 6,
            rerank_depth: 1,
            candidates_per_leg: 50,
            context_max_chars: 60_000,
            ..RagParams::default()
        };

        let candidates =
            hybrid_retrieve_candidates(&conn, "cabildo", Some(&[1.0, 0.0, 0.0]), &params)
                .expect("retrieval should succeed");

        assert_eq!(candidates.len(), 6);
    }

    #[test]
    fn vector_leg_only_considers_assets_with_text() {
        let conn = setup_rag_db();
        // Asset con embedding pero sin extracción NI transcripción → fuera de
        // la base RAG.
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding) VALUES ('a1', 'i1', ?1)",
            params![floats_to_blob(&[1.0, 0.0])],
        )
        .expect("embedding insert");

        let ranked = vector_leg(&conn, &[1.0, 0.0], 10, 0.0).expect("vector leg should succeed");
        assert!(ranked.is_empty());
    }

    #[test]
    fn vector_leg_includes_legacy_embeddings_with_compatible_dimensions() {
        let conn = setup_rag_db();
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-current", "Actual"),
            "asset-current",
            "texto actual",
            None,
            Some(&[1.0, 0.0]),
        );
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-legacy", "Legacy"),
            "asset-legacy",
            "texto legacy",
            None,
            None,
        );
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding) VALUES ('asset-legacy', 'item-legacy', ?1)",
            params![floats_to_blob(&[1.0, 0.0])],
        )
        .expect("legacy embedding insert");

        let ranked = vector_leg(&conn, &[1.0, 0.0], 10, 0.0).expect("vector leg should succeed");
        assert_eq!(ranked.len(), 2);
        assert!(ranked.contains(&"asset-current".to_string()));
        assert!(ranked.contains(&"asset-legacy".to_string()));
    }

    #[test]
    fn vector_leg_skips_legacy_embeddings_with_incompatible_dimensions() {
        let conn = setup_rag_db();
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-current", "Actual"),
            "asset-current",
            "texto actual",
            None,
            Some(&[1.0, 0.0, 0.0]),
        );
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-legacy", "Legacy"),
            "asset-legacy",
            "texto legacy",
            None,
            None,
        );
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding) VALUES ('asset-legacy', 'item-legacy', ?1)",
            params![floats_to_blob(&[1.0, 0.0])],
        )
        .expect("legacy embedding insert (2-dim vs 3-dim query)");

        let ranked =
            vector_leg(&conn, &[1.0, 0.0, 0.0], 10, 0.0).expect("vector leg should succeed");
        assert_eq!(ranked, vec!["asset-current".to_string()]);
    }

    // ── OCR en retrieval (Cambio 1) ──────────────────────────────────────────

    #[test]
    fn vector_leg_includes_ocr_only_assets() {
        let conn = setup_rag_db();
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-ocr", "Panfleto digitalizado"),
            "asset-ocr",
            "El panfleto convoca al cabildo abierto",
            Some(&[1.0, 0.0, 0.0]),
        );

        let ranked =
            vector_leg(&conn, &[1.0, 0.0, 0.0], 10, 0.0).expect("vector leg should succeed");
        assert_eq!(ranked, vec!["asset-ocr".to_string()]);
    }

    #[test]
    fn lexical_leg_includes_ocr_only_assets() {
        let conn = setup_rag_db();
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-ocr", "Panfleto digitalizado"),
            "asset-ocr",
            "El panfleto convoca al cabildo abierto",
            None,
        );

        let ranked = lexical_leg(&conn, "cabildo", 10).expect("lexical leg should succeed");
        assert_eq!(ranked, vec!["asset-ocr".to_string()]);
    }

    /// F7 regression: un ítem con muchos assets no debe monopolizar la pierna.
    ///
    /// Drenando ítem por ítem, el ítem gordo llenaba el presupuesto entero y el
    /// segundo ítem —que también matchea— quedaba afuera.
    #[test]
    fn lexical_leg_does_not_let_one_multi_asset_item_starve_the_others() {
        let conn = setup_rag_db();

        // Ítem gordo: 8 assets, todos con texto que matchea.
        conn.execute(
            "INSERT OR IGNORE INTO collections(id, name) VALUES ('col-1', 'Archivo')",
            [],
        )
        .expect("collection insert");
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES ('item-fat', 'col-1', 'Expediente largo', '{}')",
            [],
        )
        .expect("fat item insert");
        for index in 0..8 {
            let asset_id = format!("asset-fat-{index}");
            conn.execute(
                "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, 'item-fat', 'scan.png', 'image', ?2)",
                params![asset_id, index as i64],
            )
            .expect("fat asset insert");
            insert_extraction(&conn, &asset_id, "acta del cabildo", 1);
        }
        crate::nlp::fts::fts_index_item(
            &conn,
            "item-fat",
            "Expediente largo",
            "",
            "acta del cabildo",
        )
        .expect("fts index fat");

        // Ítem flaco: un solo asset, también matchea.
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-thin", "Panfleto"),
            "asset-thin",
            "el cabildo abierto",
            None,
        );

        // Presupuesto 4: alcanza de sobra para incluir al ítem flaco.
        let ranked = lexical_leg(&conn, "cabildo", 4).expect("lexical leg should succeed");

        assert_eq!(ranked.len(), 4, "debe llenar el presupuesto: {ranked:?}");
        assert!(
            ranked.contains(&"asset-thin".to_string()),
            "el ítem de un solo asset fue starvado por el ítem gordo: {ranked:?}"
        );
    }

    #[test]
    fn hybrid_retrieve_ocr_only_asset_yields_snippet_without_timestamps() {
        let conn = setup_rag_db();
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-ocr", "Panfleto digitalizado"),
            "asset-ocr",
            "El panfleto convoca al cabildo abierto en la plaza",
            Some(&[1.0, 0.0, 0.0]),
        );

        let sources = hybrid_retrieve(
            &conn,
            "cabildo",
            Some(&[1.0, 0.0, 0.0]),
            &RagParams::default(),
        )
        .expect("OCR-only retrieval should succeed");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].asset_id, "asset-ocr");
        assert!(sources[0].snippet.contains("cabildo"));
        assert_eq!(sources[0].start_seconds, None);
        assert_eq!(sources[0].end_seconds, None);
    }

    #[test]
    fn vector_leg_min_similarity_filters_candidates_below_threshold() {
        let conn = setup_rag_db();
        // Similitud 1.0 contra el query [1, 0, 0].
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-near", "Cercano"),
            "asset-near",
            "texto cercano",
            None,
            Some(&[1.0, 0.0, 0.0]),
        );
        // Ortogonal: similitud 0.0.
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-far", "Lejano"),
            "asset-far",
            "texto lejano",
            None,
            Some(&[0.0, 1.0, 0.0]),
        );
        // Opuesto: similitud -1.0.
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-anti", "Opuesto"),
            "asset-anti",
            "texto opuesto",
            None,
            Some(&[-1.0, 0.0, 0.0]),
        );

        let query = [1.0_f32, 0.0, 0.0];

        // Umbral 0.5: solo sobrevive el cercano.
        let ranked = vector_leg(&conn, &query, 10, 0.5).expect("vector leg should succeed");
        assert_eq!(ranked, vec!["asset-near".to_string()]);

        // Umbral 0.0 = deshabilitado: quedan TODOS, incluso similitud negativa.
        let ranked = vector_leg(&conn, &query, 10, 0.0).expect("vector leg should succeed");
        assert_eq!(
            ranked,
            vec![
                "asset-near".to_string(),
                "asset-far".to_string(),
                "asset-anti".to_string()
            ]
        );
    }

    // ── Texto combinado extracción + transcripción ───────────────────────────

    #[test]
    fn load_source_record_combines_extraction_then_transcription() {
        let conn = setup_rag_db();
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-both", "Acta mixta"),
            "asset-both",
            "audio transcripto",
            None,
            None,
        );
        // Dos extracciones: la convención lee la MÁS ANTIGUA.
        insert_extraction(&conn, "asset-both", "ocr más nuevo", 50);
        insert_extraction(&conn, "asset-both", "ocr original", 10);

        let record = load_source_record(&conn, "asset-both")
            .expect("load should succeed")
            .expect("record should exist");

        // Misma semántica que get_asset_text: extracción + espacio + transcripción.
        assert_eq!(record.text_content, "ocr original audio transcripto");
        assert_eq!(
            record.text_content,
            crate::nlp::text_provider::get_asset_text(&conn, "asset-both")
                .expect("get_asset_text should succeed"),
            "combined text must match get_asset_text semantics"
        );
        // La transcripción arranca después de "ocr original " (13 chars).
        assert_eq!(record.transcription_offset_chars, Some(13));
        assert_eq!(record.item_id, "item-both");
        assert_eq!(record.collection_name, "Archivo");
    }

    #[test]
    fn load_source_record_ocr_only_has_no_transcription_offset() {
        let conn = setup_rag_db();
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-ocr", "Panfleto"),
            "asset-ocr",
            "texto del panfleto",
            None,
        );

        let record = load_source_record(&conn, "asset-ocr")
            .expect("load should succeed")
            .expect("record should exist");
        assert_eq!(record.text_content, "texto del panfleto");
        assert_eq!(record.transcription_offset_chars, None);
        assert_eq!(record.segments_json, None);
    }

    #[test]
    fn load_source_record_transcription_only_offset_is_zero() {
        let conn = setup_rag_db();
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-audio", "Entrevista"),
            "asset-audio",
            "solo transcripción",
            Some(r#"[{"start":0.0,"end":2.0,"text":"solo transcripción"}]"#),
            None,
        );

        let record = load_source_record(&conn, "asset-audio")
            .expect("load should succeed")
            .expect("record should exist");
        assert_eq!(record.text_content, "solo transcripción");
        assert_eq!(record.transcription_offset_chars, Some(0));
        assert!(record.segments_json.is_some());
    }

    #[test]
    fn build_sources_term_in_transcription_part_yields_timestamps() {
        let conn = setup_rag_db();
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-both", "Acta mixta"),
            "asset-both",
            "el cabildo abierto de mayo",
            Some(r#"[{"start":2.0,"end":5.5,"text":"el cabildo abierto de mayo"}]"#),
            None,
        );
        insert_extraction(&conn, "asset-both", "panfleto repartido en la plaza", 1);

        let record = load_source_record(&conn, "asset-both")
            .expect("load should succeed")
            .expect("record should exist");
        let sources = build_sources(
            vec![RrfCandidate { record, score: 1.0 }],
            "cabildo",
            1,
            1600,
            10_000,
        );

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].start_seconds, Some(2.0));
        assert_eq!(sources[0].end_seconds, Some(5.5));
    }

    #[test]
    fn build_sources_term_in_extraction_part_yields_no_timestamps() {
        let conn = setup_rag_db();
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-both", "Acta mixta"),
            "asset-both",
            "una memoria sobre la vida en la aldea",
            Some(r#"[{"start":0.0,"end":9.0,"text":"una memoria sobre la vida en la aldea"}]"#),
            None,
        );
        insert_extraction(
            &conn,
            "asset-both",
            "el cabildo abierto convocado en mayo",
            1,
        );

        let record = load_source_record(&conn, "asset-both")
            .expect("load should succeed")
            .expect("record should exist");
        let sources = build_sources(
            vec![RrfCandidate { record, score: 1.0 }],
            "cabildo",
            1,
            1600,
            10_000,
        );

        assert_eq!(sources.len(), 1);
        assert!(sources[0].snippet.contains("cabildo"));
        assert_eq!(sources[0].start_seconds, None);
        assert_eq!(sources[0].end_seconds, None);
    }
}
