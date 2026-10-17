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
use super::{RagDegradation, RagSource, RagSourceProvenance};
use crate::nlp::vector::cosine_distance_to_blob;
use crate::processing::eligibility::EffectiveEmbeddingContract;

/// Longitud mínima (en chars) de un término de la pregunta para anclar snippets.
const MIN_TERM_CHARS: usize = 4;

/// Pasajes (chunks recuperados) que una misma página puede aportar. Uno solo
/// perdía otros artículos relevantes de una página de diario; sin tope, una
/// página larga podía ocupar todas las fuentes. Los adyacentes se unen en un
/// solo pasaje al empaquetar (`pack_sources`).
const MAX_PASSAGES_PER_ASSET: usize = 3;

/// Ítems cuyas páginas lee la pierna léxica por asset para ordenarlas por sus
/// propias coincidencias (ver `lexical_leg_filtered`). Leer páginas cuesta
/// por página, y un diario puede tener cientos por ítem: sin tope, cada
/// pregunta leía el texto de las páginas de hasta `rag_candidates_per_leg`
/// (24) ítems. Ocho cubre los `top_k` ítems que pueden llegar al prompt solo
/// por el rango de esta pierna (6 por defecto) con margen; si el usuario
/// sube `top_k`, el tope sube con él (`top_k.max(8)`). Los ítems siguientes
/// siguen entrando, con sus páginas en el orden histórico: el tope no quita
/// recall, solo deja de leer texto.
const LEXICAL_SCAN_MIN_ITEMS: usize = 8;

// El tope de candidatos RRF vive ahora en `params::DEFAULT_FUSION_CANDIDATE_LIMIT`
// (setting `rag_fusion_candidate_limit`): era la única perilla de recuperación
// que el usuario no podía tocar.

/// Unidad de recuperación (`rag_retrieval_unit`).
///
/// `Chunk` (el default) es híbrida POR ASSET: el asset con chunks en el
/// espacio de embedding efectivo se recupera por pasaje; el que no los tiene
/// (procesado antes de los chunks, o sin proveedor de embeddings) sigue
/// entrando entero, por `vec_assets` y `fts_items`, como en `Asset`. `Asset`
/// queda como modo explícito: la página entera, recortada por ventana.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetrievalUnit {
    Asset,
    Chunk,
}

impl RetrievalUnit {
    pub(crate) fn from_setting(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            Some("asset") => Ok(Self::Asset),
            None | Some("chunk") => Ok(Self::Chunk),
            Some(other) => Err(format!(
                "Unidad de recuperación RAG no soportada: {other}. Usá 'asset' o 'chunk'."
            )),
        }
    }
}

impl Default for RetrievalUnit {
    fn default() -> Self {
        Self::Chunk
    }
}

/// Metadatos y texto combinado necesarios para construir la cita de un asset.
///
/// `text_content` es el MISMO texto combinado que produce
/// `nlp::text_provider::get_asset_text`: extracción más antigua primero,
/// después la transcripción, unidas con un espacio cuando hay ambas.
/// `transcription_offset_chars` marca el char (en el texto combinado) donde
/// arranca la porción de transcripción — `None` si el asset no tiene
/// transcripción con texto.
#[derive(Debug, Clone)]
pub(crate) struct ChunkRecordProvenance {
    pub source_kind: String,
    pub source_id: String,
    pub chunk_ordinal: i64,
}

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
    pub source_start_char: usize,
    pub chunk: Option<ChunkRecordProvenance>,
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct RetrievalQuery<'a> {
    pub(crate) text: &'a str,
    pub(crate) embedding: Option<&'a [f32]>,
}

/// Registro que propone una pierna: una página entera o un chunk. Las piernas
/// de las dos unidades se fusionan juntas, así que el tipo viaja con el id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum RecordKey {
    Asset(String),
    Chunk(String),
}

/// De dónde sale una pierna. El diagnóstico del baseline la informa por
/// separado; la fusión no la distingue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegKind {
    AssetVector,
    AssetLexical,
    ChunkVector,
    ChunkLexical,
}

#[derive(Debug, Clone)]
pub(crate) struct RetrievalLeg {
    // Lo lee el diagnóstico por pierna del baseline (solo en tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) kind: LegKind,
    pub(crate) records: Vec<RecordKey>,
}

/// Qué páginas puede proponer una pierna por asset.
#[derive(Clone, Copy)]
enum AssetFilter<'a> {
    All,
    /// Solo las que NO tienen chunks en el espacio efectivo: las que sí los
    /// tienen ya entran por las piernas de chunks.
    WithoutChunks(&'a EffectiveEmbeddingContract),
}

/// Pregunta, por asset, si ya tiene chunks en el espacio efectivo.
struct ChunkCoverage<'c, 'a> {
    stmt: Option<(rusqlite::Statement<'c>, &'a EffectiveEmbeddingContract)>,
}

impl<'c, 'a> ChunkCoverage<'c, 'a> {
    fn new(conn: &'c Connection, filter: AssetFilter<'a>) -> Result<Self, String> {
        let stmt = match filter {
            AssetFilter::All => None,
            AssetFilter::WithoutChunks(effective) => Some((
                conn.prepare(
                    "SELECT EXISTS(SELECT 1 FROM rag_chunks
                                   WHERE asset_id = ?1
                                     AND embedding_model = ?2
                                     AND embedding_contract = ?3
                                     AND dimensions = ?4)",
                )
                .map_err(|e| format!("Failed to prepare RAG chunk coverage check: {e}"))?,
                effective,
            )),
        };
        Ok(Self { stmt })
    }

    /// `true` si la pierna por asset puede proponer esta página.
    fn admits(&mut self, asset_id: &str) -> Result<bool, String> {
        let Some((stmt, effective)) = self.stmt.as_mut() else {
            return Ok(true);
        };
        let covered: bool = stmt
            .query_row(
                rusqlite::params![
                    asset_id,
                    effective.model,
                    effective.contract,
                    effective.dimensions as i64
                ],
                |row| row.get(0),
            )
            .map_err(|e| format!("Failed to check RAG chunk coverage of {asset_id}: {e}"))?;
        Ok(!covered)
    }
}

/// Las piernas de una recuperación, en el orden en que se fusionan.
///
/// En `Chunk` la decisión es POR ASSET y vive acá, no en el comando, para
/// que la compartan el Chat (`rag_ask`) y Escritura
/// (`writing_corpus_retrieve`): las piernas de chunks proponen pasajes y las
/// de asset solo proponen páginas sin chunks en el espacio efectivo. Así una
/// página nunca entra dos veces y la que no tiene chunks no se pierde.
#[cfg(test)]
pub(crate) fn retrieval_legs(
    conn: &Connection,
    queries: &[RetrievalQuery<'_>],
    params: &RagParams,
    unit: RetrievalUnit,
) -> Result<Vec<RetrievalLeg>, String> {
    Ok(legs_reporting(conn, queries, params, unit)?.0)
}

/// Código de [`RagDegradation`]: sin espacio de embedding resoluble no hay
/// pasajes indexados que cuenten, y todo el corpus se recupera por página.
pub(crate) const DEGRADATION_WHOLE_PAGES_ONLY: &str = "whole_pages_only";

fn legs_reporting(
    conn: &Connection,
    queries: &[RetrievalQuery<'_>],
    params: &RagParams,
    unit: RetrievalUnit,
) -> Result<(Vec<RetrievalLeg>, Vec<RagDegradation>), String> {
    let mut degradation = Vec::new();
    // Sin espacio efectivo resoluble (proveedor desconocido) no hay chunks que
    // cuenten como indexados: se recupera como en `Asset`, igual que la pierna
    // vectorial cae cuando la consulta no se puede embeber. No en silencio: va
    // al log y a `degradation`, que la respuesta lleva a la interfaz.
    let effective = match unit {
        RetrievalUnit::Asset => None,
        RetrievalUnit::Chunk => {
            match crate::processing::eligibility::resolve_effective_embedding_contract(conn) {
                Ok(effective) => Some(effective),
                Err(error) => {
                    log::warn!(
                        target: "rag",
                        "Recuperación por pasajes deshabilitada (se usan páginas enteras): {error}"
                    );
                    degradation.push(RagDegradation {
                        code: DEGRADATION_WHOLE_PAGES_ONLY.to_string(),
                        detail: error,
                    });
                    None
                }
            }
        }
    };
    let filter = effective
        .as_ref()
        .map_or(AssetFilter::All, AssetFilter::WithoutChunks);
    let limit = params.candidates_per_leg;
    let assets = |ids: Vec<String>| ids.into_iter().map(RecordKey::Asset).collect();
    let chunks = |ids: Vec<String>| ids.into_iter().map(RecordKey::Chunk).collect();

    let mut legs = Vec::with_capacity(queries.len().saturating_mul(4));
    for query in queries {
        if let Some(effective) = effective.as_ref() {
            if let Some(embedding) = query.embedding {
                legs.push(RetrievalLeg {
                    kind: LegKind::ChunkVector,
                    records: chunks(chunk_vector_leg_in(
                        conn,
                        embedding,
                        limit,
                        params.min_similarity,
                        effective,
                    )?),
                });
            }
            legs.push(RetrievalLeg {
                kind: LegKind::ChunkLexical,
                records: chunks(chunk_lexical_leg_in(conn, query.text, limit, effective)?),
            });
        }
        if let Some(embedding) = query.embedding {
            legs.push(RetrievalLeg {
                kind: LegKind::AssetVector,
                records: assets(vector_leg_filtered(
                    conn,
                    embedding,
                    limit,
                    params.min_similarity,
                    filter,
                )?),
            });
        }
        legs.push(RetrievalLeg {
            kind: LegKind::AssetLexical,
            records: assets(lexical_leg_filtered(
                conn,
                query.text,
                limit,
                filter,
                params.top_k.max(LEXICAL_SCAN_MIN_ITEMS),
            )?),
        });
    }
    Ok((legs, degradation))
}

/// Candidatos de una recuperación y lo que tuvo que resignar para obtenerlos.
#[derive(Debug, Default)]
pub(crate) struct HybridRetrieval {
    pub(crate) candidates: Vec<RrfCandidate>,
    pub(crate) degradation: Vec<RagDegradation>,
}

/// Recuperación híbrida hasta la lista RRF de candidatos. Los registros son
/// owned para que cualquier reranking ocurra después de liberar SQLite.
pub(crate) fn hybrid_retrieve_candidates(
    conn: &Connection,
    queries: &[RetrievalQuery<'_>],
    params: &RagParams,
    unit: RetrievalUnit,
) -> Result<Vec<RrfCandidate>, String> {
    Ok(retrieve_reporting(conn, queries, params, unit)?.candidates)
}

/// [`hybrid_retrieve_candidates`] más las degradaciones declaradas, para el
/// Chat, que las devuelve en la respuesta.
pub(crate) fn retrieve_reporting(
    conn: &Connection,
    queries: &[RetrievalQuery<'_>],
    params: &RagParams,
    unit: RetrievalUnit,
) -> Result<HybridRetrieval, String> {
    let (legs, degradation) = legs_reporting(conn, queries, params, unit)?;
    let legs: Vec<Vec<RecordKey>> = legs.into_iter().map(|leg| leg.records).collect();
    let fused = rrf_fuse(&legs, params.fusion_candidate_limit, params.rrf_k as f64);

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
    let mut passages_per_asset: HashMap<String, usize> = HashMap::with_capacity(wanted);
    for (key, score) in fused {
        if candidates.len() >= wanted {
            break;
        }
        let record = match &key {
            RecordKey::Asset(asset_id) => load_source_record(conn, asset_id)?,
            RecordKey::Chunk(chunk_id) => {
                load_source_record_for_unit(conn, RetrievalUnit::Chunk, chunk_id)?
            }
        };
        if let Some(record) = record {
            if record.chunk.is_some() {
                let passages = passages_per_asset
                    .entry(record.asset_id.clone())
                    .or_default();
                if *passages >= MAX_PASSAGES_PER_ASSET {
                    continue;
                }
                *passages += 1;
            }
            candidates.push(RrfCandidate { record, score });
        }
    }

    Ok(HybridRetrieval {
        candidates,
        degradation,
    })
}

/// Cobertura de pasajes del corpus en ESTE dispositivo: páginas con texto y,
/// de ellas, cuántas tienen chunks en el espacio de embedding efectivo. Las
/// que no, se recuperan enteras; `rag_chunks` no se sincroniza, así que cada
/// dispositivo tiene su propia cobertura.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RagChunkCoverage {
    pub assets_with_text: i64,
    pub assets_with_chunks: i64,
}

pub(crate) fn chunk_coverage(conn: &Connection) -> Result<RagChunkCoverage, String> {
    // Sin espacio efectivo resoluble ningún chunk cuenta (ver `retrieval_legs`).
    let effective = crate::processing::eligibility::resolve_effective_embedding_contract(conn).ok();
    conn.query_row(
        "WITH with_text AS (
             SELECT a.id
             FROM assets a
             WHERE EXISTS(SELECT 1 FROM extractions e
                          WHERE e.asset_id = a.id
                            AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0)
                OR EXISTS(SELECT 1 FROM transcriptions t
                          WHERE t.asset_id = a.id
                            AND LENGTH(TRIM(COALESCE(t.text_content, ''))) > 0)
         )
         SELECT
             (SELECT COUNT(*) FROM with_text),
             (SELECT COUNT(*) FROM with_text w
              WHERE EXISTS(SELECT 1 FROM rag_chunks rc
                           WHERE rc.asset_id = w.id
                             AND rc.embedding_model = ?1
                             AND rc.embedding_contract = ?2
                             AND rc.dimensions = ?3))",
        rusqlite::params![
            effective.as_ref().map(|contract| contract.model.as_str()),
            effective
                .as_ref()
                .map(|contract| contract.contract.as_str()),
            effective
                .as_ref()
                .map(|contract| contract.dimensions as i64)
        ],
        |row| {
            Ok(RagChunkCoverage {
                assets_with_text: row.get(0)?,
                assets_with_chunks: row.get(1)?,
            })
        },
    )
    .map_err(|e| format!("Failed to read RAG chunk coverage: {e}"))
}

#[cfg(test)]
fn hybrid_retrieve(
    conn: &Connection,
    question: &str,
    query_embedding: Option<&[f32]>,
    params: &RagParams,
) -> Result<Vec<RagSource>, String> {
    let query = RetrievalQuery {
        text: question,
        embedding: query_embedding,
    };
    let candidates = hybrid_retrieve_candidates(conn, &[query], params, RetrievalUnit::Asset)?;
    Ok(build_sources(
        candidates,
        question,
        params.top_k,
        params.snippet_max_chars,
        params.context_max_chars,
    ))
}

/// How many times `limit` the partial selection ranks before applying the
/// text filter. Four keeps the sorted window small (96 rows for the default
/// `rag_candidates_per_leg` of 24) while the historical full sort remains as
/// the fallback, so this is pure work saved on the common path.
const VECTOR_LEG_SELECTION_FACTOR: usize = 4;

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
    vector_leg_filtered(
        conn,
        query_embedding,
        limit,
        min_similarity,
        AssetFilter::All,
    )
}

fn vector_leg_filtered(
    conn: &Connection,
    query_embedding: &[f32],
    limit: usize,
    min_similarity: f64,
    filter: AssetFilter<'_>,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT v.asset_id, v.embedding FROM vec_assets v")
        .map_err(|e| format!("Failed to prepare RAG vector query: {e}"))?;
    let scored = score_embeddings(
        &mut stmt,
        [],
        query_embedding,
        min_similarity,
        "RAG vector query",
    )?;

    // The text requirement is checked on the ranked candidates, best first,
    // until `limit` pass. Filtering in the scan read every page's text on
    // every question; the result is the same ids in the same order.
    let mut has_text = conn
        .prepare(
            "SELECT EXISTS(SELECT 1 FROM extractions e
                            WHERE e.asset_id = ?1
                              AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0)
                 OR EXISTS(SELECT 1 FROM transcriptions t
                            WHERE t.asset_id = ?1
                              AND LENGTH(TRIM(COALESCE(t.text_content, ''))) > 0)",
        )
        .map_err(|e| format!("Failed to prepare RAG vector text check: {e}"))?;
    let mut coverage = ChunkCoverage::new(conn, filter)?;
    rank_assets_with_text(scored, limit, |asset_id| {
        let with_text: bool = has_text
            .query_row([asset_id], |row| row.get(0))
            .map_err(|e| format!("Failed to check RAG vector text of {asset_id}: {e}"))?;
        Ok(with_text && coverage.admits(asset_id)?)
    })
}

/// Walks `candidates` in ranking order and collects the ids that have text
/// until `limit` are found.
fn collect_with_text<'a>(
    candidates: impl Iterator<Item = &'a (String, f64)>,
    limit: usize,
    with_text: &mut impl FnMut(&str) -> Result<bool, String>,
) -> Result<Vec<String>, String> {
    let mut ranked = Vec::with_capacity(limit);
    for (asset_id, _) in candidates {
        if ranked.len() == limit {
            break;
        }
        if with_text(asset_id)? {
            ranked.push(asset_id.clone());
        }
    }
    Ok(ranked)
}

/// Ranks `scored` (scan order is the tie-break) and returns the first `limit`
/// ids that have text.
///
/// The historical implementation stable-sorted every score on every question.
/// A fixed bounded heap before the text filter is NOT equivalent: the walk has
/// to keep going until `limit` candidates survive, and with more than the
/// window size of high-scoring textless vectors a window alone would silently
/// drop valid candidates. So only the best
/// `limit * VECTOR_LEG_SELECTION_FACTOR` candidates are selected and sorted,
/// and the historical full sort is the fallback when the window yields fewer
/// than `limit` ids with text. The original scan index is the explicit
/// secondary key, so the unstable partition cannot reorder score ties away
/// from the stable sort's order.
fn rank_assets_with_text<F>(
    mut scored: Vec<(String, f64)>,
    limit: usize,
    mut with_text: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(&str) -> Result<bool, String>,
{
    if limit == 0 || scored.is_empty() {
        return Ok(Vec::new());
    }

    let budget = limit.saturating_mul(VECTOR_LEG_SELECTION_FACTOR);
    if scored.len() > budget {
        let mut order: Vec<usize> = (0..scored.len()).collect();
        let by_score = |left: &usize, right: &usize| {
            scored[*right]
                .1
                .partial_cmp(&scored[*left].1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.cmp(right))
        };
        order.select_nth_unstable_by(budget - 1, by_score);
        order[..budget].sort_by(by_score);
        let ranked = collect_with_text(
            order[..budget].iter().map(|&index| &scored[index]),
            limit,
            &mut with_text,
        )?;
        if ranked.len() == limit {
            return Ok(ranked);
        }
        // Fewer than `limit` assets inside the window have text: fall through
        // to the historical full sort and walk, which still finds the
        // remaining candidates past the window (in the same order).
    }

    scored.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    collect_with_text(scored.iter(), limit, &mut with_text)
}

/// Streams `(id, embedding)` rows and returns each id with its cosine
/// similarity to `query`, reading every blob in place. Collecting the rows
/// first held all the embeddings in memory at once: 781 MB per question at
/// 200k pages. Rows of another dimension are skipped, and `min_similarity >
/// 0.0` drops weaker rows before any ranking.
fn score_embeddings<P: rusqlite::Params>(
    stmt: &mut rusqlite::Statement<'_>,
    params: P,
    query: &[f32],
    min_similarity: f64,
    what: &str,
) -> Result<Vec<(String, f64)>, String> {
    let mut rows = stmt
        .query(params)
        .map_err(|e| format!("Failed to run {what}: {e}"))?;
    let mut scored = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|e| format!("Failed to read {what} rows: {e}"))?
    {
        let blob = row
            .get_ref(1)
            .map_err(|e| format!("Failed to read {what} rows: {e}"))?
            .as_blob()
            .map_err(|e| format!("Failed to read {what} rows: {e}"))?;
        let Some(distance) = cosine_distance_to_blob(query, blob) else {
            continue;
        };
        let similarity = 1.0 - distance;
        // A blob holding NaN or infinity scores NaN: it would rank first and
        // break the total order the ranking's sort and selection rely on.
        if !similarity.is_finite() {
            continue;
        }
        if min_similarity > 0.0 && similarity < min_similarity {
            continue;
        }
        let id: String = row
            .get(0)
            .map_err(|e| format!("Failed to read {what} rows: {e}"))?;
        scored.push((id, similarity));
    }
    Ok(scored)
}

/// Pierna vectorial por chunk. Solo puntúa chunks del espacio de embedding
/// EFECTIVO (`resolve_effective_embedding_contract`), el mismo con el que el
/// procesamiento los estampa y con el que se embebe la consulta: filtrar por
/// el contrato canónico dejaba sin pierna semántica a quien usa el proveedor
/// local o un modelo API propio.
#[cfg(test)]
pub(crate) fn chunk_vector_leg(
    conn: &Connection,
    query_embedding: &[f32],
    limit: usize,
    min_similarity: f64,
) -> Result<Vec<String>, String> {
    let effective = crate::processing::eligibility::resolve_effective_embedding_contract(conn)?;
    chunk_vector_leg_in(conn, query_embedding, limit, min_similarity, &effective)
}

fn chunk_vector_leg_in(
    conn: &Connection,
    query_embedding: &[f32],
    limit: usize,
    min_similarity: f64,
    effective: &EffectiveEmbeddingContract,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, embedding
             FROM rag_chunks
             WHERE embedding_model = ?1
               AND embedding_contract = ?2
               AND dimensions = ?3",
        )
        .map_err(|error| format!("Failed to prepare RAG chunk vector query: {error}"))?;
    let mut scored = score_embeddings(
        &mut stmt,
        rusqlite::params![
            effective.model,
            effective.contract,
            query_embedding.len() as i64
        ],
        query_embedding,
        min_similarity,
        "RAG chunk vector query",
    )?;
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    Ok(scored
        .into_iter()
        .take(limit)
        .map(|(chunk_id, _)| chunk_id)
        .collect())
}

/// Pierna léxica: BM25 a nivel ítem vía FTS5, aplanada a los assets con texto
/// (extracción O transcripción no vacía) de cada ítem preservando el orden de
/// relevancia. El rank léxico de un asset es su posición en esta lista
/// aplanada.
#[cfg(test)]
pub(crate) fn lexical_leg(
    conn: &Connection,
    question: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    lexical_leg_filtered(
        conn,
        question,
        limit,
        AssetFilter::All,
        LEXICAL_SCAN_MIN_ITEMS,
    )
}

fn lexical_leg_filtered(
    conn: &Connection,
    question: &str,
    limit: usize,
    filter: AssetFilter<'_>,
    scan_items: usize,
) -> Result<Vec<String>, String> {
    // El input es una pregunta en lenguaje natural, no una lista de keywords:
    // con el AND implícito de FTS5 cada `que`/`el`/`sobre` sería obligatorio y
    // esta pierna devolvía cero filas para cualquier pregunta real (F1).
    let mut items = crate::nlp::fts::fts_search_with_mode(
        conn,
        question,
        None,
        crate::nlp::fts::FtsMatchMode::Any,
    )?;

    // Con lugar libre, los ítems que solo contienen variantes cercanas de los
    // términos (lecturas erróneas del OCR, otras grafías de un nombre raro) van
    // DESPUÉS de todos los exactos. Consultados aparte a propósito: una
    // variante es rara por naturaleza y, con BM25, superaría a la palabra real
    // si fueran en la misma consulta. Si algo falla, la pierna queda exacta.
    if items.len() < limit {
        let terms = crate::nlp::fts::any_mode_terms(question);
        if let Some(expr) = crate::nlp::fuzzy::variants_only_match(conn, &terms) {
            if let Ok(extra) = crate::nlp::fts::fts_search_match(conn, &expr, None) {
                let seen: HashSet<String> = items.iter().map(|row| row.item_id.clone()).collect();
                items.extend(extra.into_iter().filter(|row| !seen.contains(&row.item_id)));
            }
        }
    }

    // Dentro del ítem, cada página se ordena por SUS coincidencias: cuántos
    // términos de la pregunta contiene (sin stopwords, como el recorte),
    // plegados en Rust igual que FTS (minúscula Unicode, sin diacríticos):
    // `lower()` de SQLite solo baja el ASCII y "ÓRGANO" no encontraba
    // "órgano". El BM25 de `fts_items` es por ítem, y ordenar sus páginas por
    // fecha de creación ponía primero la tapa de un diario aunque el tema
    // estuviera en la página tres (S2). Si ninguna página contiene un término
    // (el ítem coincidió por título, metadatos o por una variante del OCR),
    // se conservan todas en el orden histórico. Solo se lee el texto de los
    // primeros `scan_items` ítems (`LEXICAL_SCAN_MIN_ITEMS`).
    let terms: Vec<Vec<char>> = extract_query_terms(question)
        .iter()
        .map(|term| fold(term))
        .collect();
    const PAGES_WITH_TEXT: &str = "FROM assets a
         WHERE a.item_id = ?1
           AND (EXISTS(SELECT 1 FROM extractions e
                       WHERE e.asset_id = a.id
                         AND LENGTH(TRIM(COALESCE(e.text_content, ''))) > 0)
                OR EXISTS(SELECT 1 FROM transcriptions t
                          WHERE t.asset_id = a.id
                            AND LENGTH(TRIM(COALESCE(t.text_content, ''))) > 0))
         ORDER BY a.created_at ASC, a.id ASC";
    let mut ids_stmt = conn
        .prepare(&format!("SELECT a.id {PAGES_WITH_TEXT}"))
        .map_err(|e| format!("Failed to prepare RAG lexical asset query: {e}"))?;
    let mut text_stmt = conn
        .prepare(&format!(
            "SELECT a.id,
                    COALESCE((SELECT group_concat(e.text_content, ' ')
                              FROM extractions e WHERE e.asset_id = a.id), '')
                    || ' ' ||
                    COALESCE((SELECT group_concat(t.text_content, ' ')
                              FROM transcriptions t WHERE t.asset_id = a.id), '')
             {PAGES_WITH_TEXT}"
        ))
        .map_err(|e| format!("Failed to prepare RAG lexical page text query: {e}"))?;

    // Assets por ítem, en orden de relevancia BM25 del ítem. Cada ítem se acota
    // al presupuesto total DESPUÉS del filtro de cobertura: si el tope cortara
    // antes, las páginas sin chunks de un ítem grande cuyas primeras páginas sí
    // los tienen nunca llegarían a esta pierna.
    let mut coverage = ChunkCoverage::new(conn, filter)?;
    let mut per_item: Vec<Vec<String>> = Vec::new();
    for (rank, item) in items.iter().take(limit).enumerate() {
        let ids: Vec<String> = if rank < scan_items && !terms.is_empty() {
            let pages = text_stmt
                .query_map(rusqlite::params![item.item_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| format!("Failed to run RAG lexical page text query: {e}"))?
                .collect::<Result<Vec<(String, String)>, _>>()
                .map_err(|e| format!("Failed to read RAG lexical page text rows: {e}"))?;
            let mut scored: Vec<(String, usize)> = pages
                .into_iter()
                .map(|(asset_id, text)| {
                    let folded = fold(&text);
                    let hits = terms
                        .iter()
                        .filter(|term| !word_start_matches(&folded, term).is_empty())
                        .count();
                    (asset_id, hits)
                })
                .collect();
            // Estable: a igual puntaje queda el orden de creación.
            scored.sort_by(|left, right| right.1.cmp(&left.1));
            let any_hit = scored.first().is_some_and(|(_, hits)| *hits > 0);
            let mut admitted = Vec::new();
            for (asset_id, hits) in scored {
                if admitted.len() == limit {
                    break;
                }
                if (!any_hit || hits > 0) && coverage.admits(&asset_id)? {
                    admitted.push(asset_id);
                }
            }
            admitted
        } else {
            // Sin `LIMIT` en SQL: las filas se leen de a una y se corta al
            // llegar al tope de páginas admitidas.
            let mut rows = ids_stmt
                .query(rusqlite::params![item.item_id])
                .map_err(|e| format!("Failed to run RAG lexical asset query: {e}"))?;
            let mut admitted = Vec::new();
            while admitted.len() < limit {
                let Some(row) = rows
                    .next()
                    .map_err(|e| format!("Failed to read RAG lexical asset rows: {e}"))?
                else {
                    break;
                };
                let asset_id: String = row
                    .get(0)
                    .map_err(|e| format!("Failed to read RAG lexical asset rows: {e}"))?;
                if coverage.admits(&asset_id)? {
                    admitted.push(asset_id);
                }
            }
            admitted
        };

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

#[cfg(test)]
pub(crate) fn chunk_lexical_leg(
    conn: &Connection,
    question: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    let effective = crate::processing::eligibility::resolve_effective_embedding_contract(conn)?;
    chunk_lexical_leg_in(conn, question, limit, &effective)
}

/// Pierna léxica por chunk, restringida a los chunks del espacio efectivo:
/// "tener chunks" significa lo mismo para las dos piernas de chunks y para
/// el filtro de las piernas por asset.
fn chunk_lexical_leg_in(
    conn: &Connection,
    question: &str,
    limit: usize,
    effective: &EffectiveEmbeddingContract,
) -> Result<Vec<String>, String> {
    let fts_query = crate::nlp::fts::sanitize_fts5_query_with_mode(
        question,
        crate::nlp::fts::FtsMatchMode::Any,
    );
    if fts_query.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT rag_chunks_fts.chunk_id
             FROM rag_chunks_fts
             JOIN rag_chunks rc ON rc.id = rag_chunks_fts.chunk_id
             WHERE rag_chunks_fts MATCH ?1
               AND rc.embedding_model = ?3
               AND rc.embedding_contract = ?4
               AND rc.dimensions = ?5
             ORDER BY bm25(rag_chunks_fts), rag_chunks_fts.chunk_id
             LIMIT ?2",
        )
        .map_err(|error| format!("Failed to prepare RAG chunk lexical query: {error}"))?;
    let run = |stmt: &mut rusqlite::Statement<'_>, expr: &str| {
        stmt.query_map(
            rusqlite::params![
                expr,
                limit as i64,
                effective.model,
                effective.contract,
                effective.dimensions as i64
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Failed to run RAG chunk lexical query: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Failed to read RAG chunk lexical rows: {error}"))
    };
    let mut chunks = run(&mut stmt, &fts_query)?;

    // Igual que en `lexical_leg`: las variantes, aparte y al final.
    if chunks.len() < limit {
        let terms = crate::nlp::fts::any_mode_terms(question);
        if let Some(expr) = crate::nlp::fuzzy::variants_only_match(conn, &terms) {
            if let Ok(extra) = run(&mut stmt, &expr) {
                let seen: HashSet<String> = chunks.iter().cloned().collect();
                chunks.extend(
                    extra
                        .into_iter()
                        .filter(|chunk| !seen.contains(chunk))
                        .take(limit - chunks.len()),
                );
            }
        }
    }
    Ok(chunks)
}

/// Reciprocal Rank Fusion: score(asset) = Σ sobre piernas de 1/(rrf_k + rank),
/// con rank arrancando en 1. Orden descendente por score; los empates se
/// resuelven determinísticamente por asset_id ascendente.
pub(crate) fn rrf_fuse<K>(legs: &[Vec<K>], limit: usize, rrf_k: f64) -> Vec<(K, f64)>
where
    K: Clone + Eq + std::hash::Hash + Ord,
{
    let mut scores: HashMap<K, f64> = HashMap::new();
    for leg in legs {
        for (rank0, asset_id) in leg.iter().enumerate() {
            let rank = (rank0 + 1) as f64;
            *scores.entry(asset_id.clone()).or_default() += 1.0 / (rrf_k + rank);
        }
    }

    let mut fused: Vec<(K, f64)> = scores.into_iter().collect();
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
        source_start_char: 0,
        chunk: None,
    }))
}

pub(crate) fn load_source_record_for_unit(
    conn: &Connection,
    unit: RetrievalUnit,
    record_id: &str,
) -> Result<Option<SourceRecord>, String> {
    if unit == RetrievalUnit::Asset {
        return load_source_record(conn, record_id);
    }

    conn.query_row(
        "SELECT rc.asset_id, rc.item_id, i.title,
                COALESCE(i.collection_id, ''), COALESCE(c.name, ''),
                rc.text_content, rc.start_char, rc.source_kind, t.segments,
                rc.id, rc.source_id, rc.chunk_ordinal, rc.end_char
         FROM rag_chunks rc
         JOIN items i ON i.id = rc.item_id
         LEFT JOIN collections c ON c.id = i.collection_id
         LEFT JOIN transcriptions t
           ON rc.source_kind = 'transcription' AND t.id = rc.source_id
         WHERE rc.id = ?1
         LIMIT 1",
        rusqlite::params![record_id],
        |row| {
            let source_kind: String = row.get(7)?;
            let start_char = row.get::<_, i64>(6)? as usize;
            Ok(SourceRecord {
                asset_id: row.get(0)?,
                item_id: row.get(1)?,
                item_title: row.get(2)?,
                collection_id: row.get(3)?,
                collection_name: row.get(4)?,
                text_content: row.get(5)?,
                segments_json: row.get(8)?,
                transcription_offset_chars: (source_kind == "transcription").then_some(0),
                source_start_char: start_char,
                chunk: Some(ChunkRecordProvenance {
                    source_kind,
                    source_id: row.get(10)?,
                    chunk_ordinal: row.get(11)?,
                }),
            })
        },
    )
    .optional()
    .map_err(|error| format!("Failed to load RAG chunk source '{record_id}': {error}"))
}

/// Convierte los registros fusionados (en orden) en fuentes citables con
/// snippet y timestamps, frenando cuando el contexto total supera el tope.
/// Si el PRIMER snippet ya supera el tope, se trunca en vez de descartarse:
/// con registros disponibles nunca se devuelven cero fuentes.
#[cfg(test)]
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
        if !push_window_source(
            &mut sources,
            &mut total_chars,
            record,
            score,
            &terms,
            snippet_max_chars,
            context_max_chars,
        ) {
            break;
        }
    }

    sources
}

/// Agrega una página entera como fuente, recortada por `snippet_window`.
/// Devuelve `false` cuando el presupuesto de contexto ya no la admite: la
/// lista de fuentes debe reflejar exactamente lo que entra al prompt.
fn push_window_source(
    sources: &mut Vec<RagSource>,
    total_chars: &mut usize,
    record: SourceRecord,
    score: f64,
    terms: &[String],
    snippet_max_chars: usize,
    context_max_chars: usize,
) -> bool {
    let (mut snippet, window_start) =
        snippet_window(&record.text_content, terms, snippet_max_chars);
    let snippet_chars = snippet.chars().count();
    if *total_chars + snippet_chars > context_max_chars {
        if !sources.is_empty() {
            return false;
        }
        // Garantía defensiva: el primer registro nunca deja la respuesta
        // sin fuentes — se trunca (char-safe) al presupuesto de contexto.
        snippet = snippet.chars().take(context_max_chars).collect();
    }
    *total_chars += snippet.chars().count();

    let timestamps = resolve_source_timestamps(&record, terms, window_start);

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
        provenance: None,
        bibliography: None,
    });
    true
}

#[derive(Debug)]
struct CanonicalChunk {
    id: String,
    ordinal: i64,
    text: String,
    start_char: usize,
    end_char: usize,
}

/// Une chunks consecutivos de una misma fuente, sin repetir el solape
/// canónico. Devuelve `(texto, start_char, end_char)` del tramo contiguo que
/// arranca en el primer chunk.
fn join_chunks(chunks: &[CanonicalChunk]) -> (String, usize, usize) {
    let start = chunks.first().map_or(0, |chunk| chunk.start_char);
    let mut end = start;
    let mut text = String::new();
    for chunk in chunks {
        if chunk.start_char > end {
            break;
        }
        let overlap = end.saturating_sub(chunk.start_char);
        text.extend(chunk.text.chars().skip(overlap));
        end = end.max(chunk.end_char);
    }
    (text, start, end)
}

/// Ajusta el pasaje unido a `budget` cortando SIEMPRE en límite de chunk,
/// nunca por ventana de términos: primero cae el vecino posterior, después
/// el anterior, y el chunk recuperado (`centers`) se queda. Solo un chunk
/// que por sí mismo excede el presupuesto se corta, y por el final.
fn fit_passage(
    mut chunks: Vec<CanonicalChunk>,
    centers: &[i64],
    budget: usize,
) -> (Vec<CanonicalChunk>, String, usize, usize) {
    loop {
        let (text, start, end) = join_chunks(&chunks);
        if text.chars().count() <= budget {
            return (chunks, text, start, end);
        }
        if chunks.len() > 1 {
            let is_center = |chunk: &CanonicalChunk| centers.contains(&chunk.ordinal);
            if !chunks.last().is_some_and(is_center) || chunks.first().is_some_and(is_center) {
                chunks.pop();
            } else {
                chunks.remove(0);
            }
            continue;
        }
        let text: String = text.chars().take(budget).collect();
        let end = start + text.chars().count();
        return (chunks, text, start, end);
    }
}

/// Un bloque a empaquetar: una página entera, o los chunks recuperados de
/// una misma fuente que quedan a un vecino de distancia (`centers`, en orden
/// de ranking; el primero da la identidad y el score del bloque).
struct PackGroup {
    candidate: RrfCandidate,
    centers: Vec<i64>,
}

/// Une los candidatos de una misma fuente cuyos vecindarios (±1 chunk) se
/// tocan: dos pasajes que comparten texto son un solo pasaje, y como bloques
/// separados repetían el solape y gastaban dos fuentes. El bloque ocupa el
/// lugar de su mejor candidato.
fn group_adjacent_passages(candidates: Vec<RrfCandidate>) -> Vec<PackGroup> {
    let mut groups: Vec<PackGroup> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let Some(chunk) = candidate.record.chunk.as_ref() else {
            groups.push(PackGroup {
                candidate,
                centers: Vec::new(),
            });
            continue;
        };
        let ordinal = chunk.chunk_ordinal;
        let joins = groups.iter_mut().find(|group| {
            group.candidate.record.asset_id == candidate.record.asset_id
                && group.candidate.record.chunk.as_ref().is_some_and(|first| {
                    first.source_kind == chunk.source_kind && first.source_id == chunk.source_id
                })
                && group
                    .centers
                    .iter()
                    .any(|center| center.abs_diff(ordinal) <= 2)
        });
        match joins {
            Some(group) => group.centers.push(ordinal),
            None => groups.push(PackGroup {
                centers: vec![ordinal],
                candidate,
            }),
        }
    }
    groups
}

/// Empaqueta candidatos en bloques citables, según el tipo de cada registro:
/// la página entera (sin chunks, o modo Asset) conserva la semántica
/// histórica, recortada por `snippet_window` a `snippet_max_chars`; el chunk
/// entra ENTERO con un vecino canónico por lado, con su propio presupuesto
/// (`passage_max_chars`) y procedencia del rango efectivamente enviado. La
/// recuperación híbrida por asset mezcla los dos tipos en una misma lista.
pub(crate) fn pack_sources(
    conn: &Connection,
    candidates: Vec<RrfCandidate>,
    question: &str,
    top_k: usize,
    snippet_max_chars: usize,
    passage_max_chars: usize,
    context_max_chars: usize,
) -> Result<Vec<RagSource>, String> {
    let terms = extract_query_terms(question);
    let mut sources = Vec::new();
    let mut total_chars = 0usize;

    for PackGroup {
        candidate: RrfCandidate { mut record, score },
        centers,
    } in group_adjacent_passages(candidates).into_iter().take(top_k)
    {
        let Some(center) = record.chunk.clone() else {
            if !push_window_source(
                &mut sources,
                &mut total_chars,
                record,
                score,
                &terms,
                snippet_max_chars,
                context_max_chars,
            ) {
                break;
            }
            continue;
        };
        let min_ordinal = centers.iter().min().copied().unwrap_or(0).saturating_sub(1);
        let max_ordinal = centers.iter().max().copied().unwrap_or(0).saturating_add(1);
        let mut stmt = conn
            .prepare(
                "SELECT id, chunk_ordinal, text_content, start_char, end_char
                 FROM rag_chunks
                 WHERE asset_id = ?1
                   AND source_kind = ?2
                   AND source_id = ?3
                   AND chunk_ordinal BETWEEN ?4 AND ?5
                 ORDER BY chunk_ordinal ASC, id ASC",
            )
            .map_err(|error| format!("Failed to prepare RAG chunk expansion: {error}"))?;
        let chunks = stmt
            .query_map(
                rusqlite::params![
                    record.asset_id,
                    center.source_kind,
                    center.source_id,
                    min_ordinal,
                    max_ordinal
                ],
                |row| {
                    Ok(CanonicalChunk {
                        id: row.get(0)?,
                        ordinal: row.get(1)?,
                        text: row.get(2)?,
                        start_char: row.get::<_, i64>(3)? as usize,
                        end_char: row.get::<_, i64>(4)? as usize,
                    })
                },
            )
            .map_err(|error| format!("Failed to expand RAG chunk neighbors: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Failed to read RAG chunk neighbors: {error}"))?;
        if chunks.is_empty() {
            continue;
        }

        let remaining = context_max_chars.saturating_sub(total_chars);
        let budget = if sources.is_empty() {
            // El primer pasaje nunca deja la respuesta sin fuentes: se ajusta
            // también al presupuesto total, en límite de chunk.
            passage_max_chars.min(context_max_chars)
        } else {
            passage_max_chars
        };
        let (chunks, packed_text, packed_start, packed_end) = fit_passage(chunks, &centers, budget);
        let packed_chars = packed_text.chars().count();
        if packed_chars > remaining {
            // La lista de fuentes debe reflejar exactamente lo que entra al prompt.
            break;
        }
        let chunk_ids = chunks
            .iter()
            .filter(|chunk| chunk.start_char < packed_end && chunk.end_char > packed_start)
            .map(|chunk| chunk.id.clone())
            .collect();
        total_chars += packed_chars;

        record.text_content = packed_text;
        record.source_start_char = packed_start;
        let timestamps = resolve_source_timestamps(&record, &terms, 0);
        sources.push(RagSource {
            index: (sources.len() + 1) as u32,
            asset_id: record.asset_id,
            item_id: record.item_id,
            item_title: record.item_title,
            collection_id: record.collection_id,
            collection_name: record.collection_name,
            snippet: record.text_content,
            score,
            start_seconds: timestamps.map(|(start, _)| start),
            end_seconds: timestamps.map(|(_, end)| end),
            provenance: Some(RagSourceProvenance {
                retrieval_unit: "chunk".to_string(),
                source_kind: center.source_kind,
                source_id: center.source_id,
                chunk_ids,
                start_char: packed_start,
                end_char: packed_end,
            }),
            bibliography: None,
        });
    }

    Ok(sources)
}

/// Términos de la pregunta para anclar snippets: split por whitespace, se
/// recorta puntuación en los bordes, lowercase, se descartan las stopwords
/// (las mismas que la pierna léxica, `nlp::fts`) y se conservan términos de
/// 4+ chars, ordenados del más largo al más corto (sin duplicados).
pub(crate) fn extract_query_terms(question: &str) -> Vec<String> {
    let mut terms: Vec<String> = question
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| word.chars().count() >= MIN_TERM_CHARS)
        .filter(|word| !crate::nlp::fts::is_stopword(word))
        .collect();

    // Más largo primero: el término más específico ancla la ventana.
    terms.sort_by_key(|term| std::cmp::Reverse(term.chars().count()));

    let mut seen = HashSet::new();
    terms.retain(|term| seen.insert(term.clone()));
    terms
}

/// Minúscula Unicode sin diacríticos, como el tokenizador de FTS
/// (`unicode61 remove_diacritics 1`, vía [`crate::nlp::fuzzy::fold_latin`]),
/// 1:1 por char para que los índices sigan alineados con el texto original.
fn fold_char(c: char) -> char {
    crate::nlp::fuzzy::fold_latin(c.to_lowercase().next().unwrap_or(c))
}

fn fold(text: &str) -> Vec<char> {
    text.chars().map(fold_char).collect()
}

/// Posiciones (índice de char) donde `needle` empieza una palabra de
/// `haystack`: "mujer" encuentra "mujeres" pero "obrero" no encuentra
/// "ELOBRERO". Los dos lados llegan plegados con [`fold`].
fn word_start_matches(haystack: &[char], needle: &[char]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }
    haystack
        .windows(needle.len())
        .enumerate()
        .filter(|(position, window)| {
            *window == needle && (*position == 0 || !haystack[position - 1].is_alphanumeric())
        })
        .map(|(position, _)| position)
        .collect()
}

/// Ventana de snippet de mayor densidad de términos de la pregunta.
///
/// Cada término coincide solo al inicio de una palabra, sin distinguir
/// mayúsculas ni tildes, y pesa por su rareza dentro del texto (IDF sobre
/// las palabras del texto): el nombre de la publicación, que aparece en
/// todas partes, pesa menos que el tema. Cada ventana candidata se centra en
/// una aparición y suma el peso de los términos distintos que contiene (las
/// repeticiones suman de forma sublineal); gana la de mayor puntaje y, a
/// igualdad, la primera. Si ningún término aparece, arranca al inicio del
/// texto. Opera SIEMPRE sobre chars (texto Unicode en español).
///
/// Devuelve `(snippet, índice_de_char_donde_arranca_la_ventana)`.
pub(crate) fn snippet_window(text: &str, terms: &[String], max_chars: usize) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return (text.to_string(), 0);
    }

    let folded: Vec<char> = chars.iter().copied().map(fold_char).collect();
    let words = folded
        .iter()
        .enumerate()
        .filter(|(index, c)| {
            c.is_alphanumeric() && (*index == 0 || !folded[index - 1].is_alphanumeric())
        })
        .count()
        .max(1) as f64;
    let occurrences: Vec<Vec<usize>> = terms
        .iter()
        .map(|term| word_start_matches(&folded, &fold(term)))
        .collect();
    let weights: Vec<f64> = occurrences
        .iter()
        .map(|positions| {
            if positions.is_empty() {
                0.0
            } else {
                (1.0 + words / positions.len() as f64).ln()
            }
        })
        .collect();

    let latest_start = chars.len().saturating_sub(max_chars);
    let score = |start: usize| -> f64 {
        let end = start + max_chars;
        occurrences
            .iter()
            .zip(&weights)
            .map(|(positions, weight)| {
                let inside = positions
                    .iter()
                    .filter(|position| **position >= start && **position < end)
                    .count();
                if inside == 0 {
                    0.0
                } else {
                    weight * (1.0 + (inside as f64).ln())
                }
            })
            .sum()
    };

    let mut anchors: Vec<usize> = occurrences.iter().flatten().copied().collect();
    anchors.sort_unstable();
    let mut best: Option<(f64, usize)> = None;
    for anchor in anchors {
        let start = anchor.saturating_sub(max_chars / 2).min(latest_start);
        let candidate = score(start);
        if best.is_none_or(|(best_score, _)| candidate > best_score) {
            best = Some((candidate, start));
        }
    }
    let start = best.map_or(0, |(_, start)| start);

    let snippet: String = chars[start..].iter().take(max_chars).collect();
    (snippet, start)
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
    let anchor = record.source_start_char
        + find_term_anchor(&record.text_content, terms).unwrap_or(window_start);
    if anchor < offset {
        return None;
    }
    resolve_timestamps(record.segments_json.as_deref(), terms, anchor - offset)
}

/// Posición (en chars) de la primera ocurrencia del término más largo que
/// aparece en el texto, con la misma coincidencia por inicio de palabra de
/// `snippet_window` (los términos llegan ordenados del más largo al más corto).
fn find_term_anchor(text: &str, terms: &[String]) -> Option<usize> {
    let folded = fold(text);
    terms
        .iter()
        .find_map(|term| word_start_matches(&folded, &fold(term)).first().copied())
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

/// Corpus mínimo con el esquema real de `rag_chunks` (migración 0029) para
/// probar la decisión Chunk/Asset por asset desde el Chat y desde Escritura.
#[cfg(test)]
pub(crate) mod hybrid_fixture {
    use rusqlite::{params, Connection};

    use crate::nlp::embeddings::{
        floats_to_blob, CANONICAL_EMBEDDING_CONTRACT_V1, CANONICAL_EMBEDDING_DIMENSIONS,
        CANONICAL_EMBEDDING_MODEL,
    };

    const RAG_CHUNKS_MIGRATION_SQL: &str =
        include_str!("../../../../../packages/store/src/migrations/0029_rag_chunks.sql");

    /// Página con chunks en el espacio efectivo (canónico, sin settings).
    pub(crate) const CHUNKED_ASSET: &str = "asset-chunked";
    /// Página procesada antes de los chunks: solo vector y texto del asset.
    pub(crate) const PLAIN_ASSET: &str = "asset-plain";
    pub(crate) const CHUNKED_TEXT: &str = "el cabildo abierto de mayo";
    /// Donde empieza el único chunk de la página con chunks.
    pub(crate) const CHUNK_START: usize = 3;

    pub(crate) fn axis(index: usize) -> Vec<f32> {
        let mut vector = vec![0.0; CANONICAL_EMBEDDING_DIMENSIONS];
        vector[index] = 1.0;
        vector
    }

    fn insert_page(conn: &Connection, asset_id: &str, item_id: &str, text: &str, vector: &[f32]) {
        conn.execute(
            "INSERT OR IGNORE INTO collections(id, name) VALUES ('col-1', 'Archivo')",
            [],
        )
        .expect("collection");
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, 'col-1', ?1, '{}')",
            params![item_id],
        )
        .expect("item");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, 'p.png', 'image', 1)",
            params![asset_id, item_id],
        )
        .expect("asset");
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, method, confidence, created_at)
             VALUES (?1, ?2, ?3, 'ocr', 0.9, 1)",
            params![format!("ext-{asset_id}"), asset_id, text],
        )
        .expect("extraction");
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                asset_id,
                item_id,
                floats_to_blob(vector),
                CANONICAL_EMBEDDING_MODEL,
                CANONICAL_EMBEDDING_CONTRACT_V1,
                CANONICAL_EMBEDDING_DIMENSIONS as i64
            ],
        )
        .expect("asset vector");
        crate::nlp::fts::fts_index_item(conn, item_id, item_id, "", text).expect("fts index");
    }

    /// Dos páginas que mencionan "cabildo": una con chunks, otra sin.
    pub(crate) fn corpus() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory DB");
        conn.execute_batch(
            "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE items (id TEXT PRIMARY KEY, collection_id TEXT, title TEXT NOT NULL, metadata TEXT);
             CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL,
                                  type TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL,
                                       method TEXT, confidence REAL, created_at INTEGER NOT NULL);
             CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT UNIQUE, text_content TEXT NOT NULL,
                                          language TEXT, duration_ms INTEGER, model TEXT, segments TEXT,
                                          confidence REAL, created_at INTEGER NOT NULL);
             CREATE TABLE vec_assets (asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, embedding BLOB NOT NULL,
                                      embedding_model TEXT NOT NULL DEFAULT 'legacy',
                                      embedding_contract TEXT NOT NULL DEFAULT 'legacy',
                                      dimensions INTEGER NOT NULL DEFAULT 0);",
        )
        .expect("corpus schema");
        conn.execute_batch(crate::nlp::fts::FTS_ITEMS_DDL)
            .expect("fts_items");
        conn.execute_batch(RAG_CHUNKS_MIGRATION_SQL)
            .expect("rag_chunks migration");

        insert_page(&conn, CHUNKED_ASSET, "item-chunked", CHUNKED_TEXT, &axis(1));
        let chunk_text: String = CHUNKED_TEXT.chars().skip(CHUNK_START).collect();
        conn.execute(
            "INSERT INTO rag_chunks (
                id, asset_id, item_id, source_kind, source_id, chunk_ordinal,
                text_content, start_char, end_char, source_text_hash,
                chunking_contract, embedding, embedding_model,
                embedding_contract, dimensions
             ) VALUES ('chunk-1', ?1, 'item-chunked', 'extraction', ?2, 0, ?3, ?4, ?5,
                       'hash', 'rag-chunk-800-100-char-v1', ?6, ?7, ?8, ?9)",
            params![
                CHUNKED_ASSET,
                format!("ext-{CHUNKED_ASSET}"),
                chunk_text,
                CHUNK_START as i64,
                CHUNKED_TEXT.chars().count() as i64,
                floats_to_blob(&axis(1)),
                CANONICAL_EMBEDDING_MODEL,
                CANONICAL_EMBEDDING_CONTRACT_V1,
                CANONICAL_EMBEDDING_DIMENSIONS as i64
            ],
        )
        .expect("chunk");
        insert_page(
            &conn,
            PLAIN_ASSET,
            "item-plain",
            "el cabildo y la revolución",
            &axis(2),
        );
        conn
    }
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

            "#,
        )
        .expect("RAG test schema creation failed");
        conn.execute_batch(crate::nlp::fts::FTS_ITEMS_DDL)
            .expect("FTS5 index creation failed");
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
            source_start_char: 0,
            chunk: None,
        }
    }

    fn candidate(asset_id: &str, text: &str, score: f64) -> RrfCandidate {
        RrfCandidate {
            record: record(asset_id, text),
            score,
        }
    }

    const RAG_CHUNKS_MIGRATION_SQL: &str =
        include_str!("../../../../../packages/store/src/migrations/0029_rag_chunks.sql");

    /// Chunk único de una página, en el espacio canónico (el efectivo sin
    /// settings). El trigger de la migración lo indexa en `rag_chunks_fts`.
    fn index_canonical_chunk(conn: &Connection, chunk_id: &str, asset_id: &str, text: &str) {
        conn.execute(
            "INSERT INTO rag_chunks (
                id, asset_id, item_id, source_kind, source_id, chunk_ordinal,
                text_content, start_char, end_char, source_text_hash,
                chunking_contract, embedding, embedding_model,
                embedding_contract, dimensions
             ) VALUES (?1, ?2, (SELECT item_id FROM assets WHERE id = ?2), 'extraction',
                       'source', 0, ?3, 0, ?4, 'hash', 'rag-chunk-v1', X'00', ?5, ?6, ?7)",
            params![
                chunk_id,
                asset_id,
                text,
                text.chars().count() as i64,
                CANONICAL_EMBEDDING_MODEL,
                CANONICAL_EMBEDDING_CONTRACT_V1,
                CANONICAL_EMBEDDING_DIMENSIONS as i64
            ],
        )
        .expect("canonical chunk should insert");
    }

    #[test]
    fn chunk_lexical_search_shares_asset_any_mode_stopword_semantics() {
        let conn = setup_rag_db();
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-relevant", "Acta"),
            "asset-relevant",
            "cabildo",
            None,
        );
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-stopwords", "Nota"),
            "asset-stopwords",
            "sobre sobre sobre",
            None,
        );
        conn.execute_batch(RAG_CHUNKS_MIGRATION_SQL)
            .expect("rag_chunks migration");
        index_canonical_chunk(
            &conn,
            "chunk-stopwords",
            "asset-stopwords",
            "sobre sobre sobre",
        );
        index_canonical_chunk(&conn, "chunk-relevant", "asset-relevant", "cabildo");

        let asset_ranked =
            lexical_leg(&conn, "sobre el cabildo", 10).expect("asset search should work");
        let chunk_ranked =
            chunk_lexical_leg(&conn, "sobre el cabildo", 10).expect("chunk search should work");

        assert_eq!(asset_ranked, vec!["asset-relevant"]);
        assert_eq!(chunk_ranked, vec!["chunk-relevant"]);
    }

    // ── Approximate lexical search ───────────────────────────────────────────

    /// Four scans read "sindicato" right and one the OCR misread; the chunk
    /// index holds the same text. With the vocabulary table (migration 0037).
    fn misread_corpus() -> Connection {
        let conn = setup_rag_db();
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0037_fts_vocab.sql"
        ))
        .expect("vocabulary table");
        conn.execute_batch(RAG_CHUNKS_MIGRATION_SQL)
            .expect("rag_chunks migration");
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("settings");
        let pages = [
            ("right-1", "el sindicato reclamo"),
            ("right-2", "asamblea del sindicato"),
            ("right-3", "sindicato de pescadores"),
            ("right-4", "nota al sindicato"),
            ("misread", "reunion del sindigato"),
        ];
        for (id, text) in pages {
            insert_ocr_doc(
                &conn,
                ("col-1", "Archivo"),
                (&format!("item-{id}"), id),
                &format!("asset-{id}"),
                text,
                None,
            );
            index_canonical_chunk(&conn, &format!("chunk-{id}"), &format!("asset-{id}"), text);
        }
        conn
    }

    #[test]
    fn lexical_leg_adds_the_misread_page_after_every_exact_one() {
        let conn = misread_corpus();

        let ranked = lexical_leg(&conn, "¿qué hizo el sindicato?", 10).expect("lexical leg");

        let mut exact = ranked[..4].to_vec();
        exact.sort();
        assert_eq!(
            exact,
            [
                "asset-right-1",
                "asset-right-2",
                "asset-right-3",
                "asset-right-4"
            ]
        );
        assert_eq!(ranked[4..], ["asset-misread"]);
    }

    #[test]
    fn chunk_lexical_leg_adds_the_misread_chunk_after_every_exact_one() {
        let conn = misread_corpus();

        let ranked = chunk_lexical_leg(&conn, "¿qué hizo el sindicato?", 10).expect("chunk leg");

        assert_eq!(ranked.len(), 5);
        assert_eq!(ranked[4], "chunk-misread");
    }

    #[test]
    fn lexical_legs_stay_exact_when_approximate_search_is_off() {
        let conn = misread_corpus();
        conn.execute(
            "INSERT INTO app_settings(key, value) VALUES ('search_fuzzy', 'off')",
            [],
        )
        .expect("setting");

        let assets = lexical_leg(&conn, "sindicato", 10).expect("lexical leg");
        let chunks = chunk_lexical_leg(&conn, "sindicato", 10).expect("chunk leg");

        assert!(!assets.contains(&"asset-misread".to_string()));
        assert!(!chunks.contains(&"chunk-misread".to_string()));
    }

    #[test]
    fn a_leg_filled_by_exact_matches_takes_no_guesses() {
        let conn = misread_corpus();

        let ranked = lexical_leg(&conn, "sindicato", 4).expect("lexical leg");

        assert!(!ranked.contains(&"asset-misread".to_string()));
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
        assert!(
            !terms
                .iter()
                .any(|t| t == "cuándo" || t == "de" || t == "al"),
            "stopwords never anchor a window"
        );

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
    fn extract_query_terms_drops_stopwords_like_the_lexical_leg() {
        let terms =
            extract_query_terms("¿Qué mirada sobre la mujer tiene el periódico 'El Obrero'?");
        assert_eq!(terms, ["periódico", "mirada", "obrero", "mujer"]);
    }

    #[test]
    fn snippet_window_ignores_a_term_glued_inside_another_word() {
        let filler = "texto de relleno sin relación alguna. ".repeat(60);
        let text = format!(
            "<div align=\"center\">\n\n# ELOBRERO\n\n</div>\n\nPrecios y suscripción.\n\n\
             {filler}## La Mujer\n\nMuchos dicen que la mujer es inferior.\n\n{filler}"
        );
        let terms =
            extract_query_terms("¿Qué mirada sobre la mujer tiene el periódico 'El Obrero'?");

        let (snippet, start) = snippet_window(&text, &terms, 600);

        assert!(snippet.contains("## La Mujer"), "{snippet}");
        assert!(!snippet.contains("ELOBRERO"));
        assert!(start > 0);
    }

    #[test]
    fn snippet_window_prefers_the_window_with_more_distinct_query_terms() {
        let filler = "relleno ".repeat(150);
        let text = format!("periódico {filler} la mirada sobre la mujer {filler}");
        let terms = vec![
            "periódico".to_string(),
            "mirada".to_string(),
            "mujer".to_string(),
        ];

        let (snippet, _start) = snippet_window(&text, &terms, 200);

        assert!(snippet.contains("la mirada sobre la mujer"), "{snippet}");
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

        let embedding = [1.0, 0.0, 0.0];
        let query = RetrievalQuery {
            text: "cabildo",
            embedding: Some(&embedding),
        };
        let candidates = hybrid_retrieve_candidates(&conn, &[query], &params, RetrievalUnit::Asset)
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

        let embedding = [1.0, 0.0, 0.0];
        let query = RetrievalQuery {
            text: "cabildo",
            embedding: Some(&embedding),
        };
        let shallow_candidates =
            hybrid_retrieve_candidates(&conn, &[query], &shallow, RetrievalUnit::Asset)
                .expect("shallow retrieval should succeed");
        let deep_candidates =
            hybrid_retrieve_candidates(&conn, &[query], &deep, RetrievalUnit::Asset)
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

        let embedding = [1.0, 0.0, 0.0];
        let query = RetrievalQuery {
            text: "cabildo",
            embedding: Some(&embedding),
        };
        let candidates = hybrid_retrieve_candidates(&conn, &[query], &params, RetrievalUnit::Asset)
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
    fn vector_leg_skips_embeddings_whose_score_is_not_finite() {
        // A stored vector holding NaN or infinity (a corrupt or synced blob)
        // scores NaN. Ranking with it would break the comparator's total
        // order, which sort_by and select_nth_unstable_by may panic on.
        let conn = setup_rag_db();
        for (asset, vector) in [
            ("asset-nan", [f32::NAN, 0.0, 0.0]),
            ("asset-inf", [f32::INFINITY, 0.0, 0.0]),
            ("asset-ok", [1.0, 0.0, 0.0]),
        ] {
            insert_ocr_doc(
                &conn,
                ("col-1", "Archivo"),
                (&format!("item-{asset}"), "Documento"),
                asset,
                "Texto del documento",
                Some(&vector),
            );
        }

        let ranked =
            vector_leg(&conn, &[1.0, 0.0, 0.0], 10, 0.0).expect("vector leg should succeed");
        assert_eq!(ranked, vec!["asset-ok".to_string()]);
    }

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
    fn vector_leg_fills_its_limit_past_closer_assets_without_text() {
        let conn = setup_rag_db();
        // The closest vector has no text: the text check runs on the ranked
        // candidates, so the next ones with text must still fill the limit.
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding) VALUES ('bare', 'i0', ?1)",
            params![floats_to_blob(&[1.0, 0.0, 0.0])],
        )
        .expect("embedding insert");
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-near", "Acta"),
            "asset-near",
            "Acta del cabildo",
            Some(&[0.9, 0.1, 0.0]),
        );
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-far", "Carta"),
            "asset-far",
            "Carta al virrey",
            Some(&[0.1, 0.9, 0.0]),
        );

        let ranked =
            vector_leg(&conn, &[1.0, 0.0, 0.0], 1, 0.0).expect("vector leg should succeed");
        assert_eq!(ranked, vec!["asset-near".to_string()]);
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
    fn lexical_leg_ranks_the_pages_of_an_item_by_their_own_matches() {
        let conn = setup_rag_db();
        conn.execute(
            "INSERT INTO collections(id, name) VALUES ('col-1', 'Archivo')",
            [],
        )
        .expect("collection insert");
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata)
             VALUES ('item-paper', 'col-1', 'El Obrero, 1905', '{}')",
            [],
        )
        .expect("item insert");
        // Páginas en orden de creación: la que trata el tema es la última.
        let pages = [
            ("page-1", "precios y suscripción del periódico"),
            ("page-2", "la huelga del puerto"),
            ("page-3", "la mujer y su emancipación, dice la mujer"),
        ];
        for (index, (asset_id, text)) in pages.iter().enumerate() {
            conn.execute(
                "INSERT INTO assets(id, item_id, path, type, created_at)
                 VALUES (?1, 'item-paper', 'scan.png', 'image', ?2)",
                params![asset_id, index as i64],
            )
            .expect("asset insert");
            insert_extraction(&conn, asset_id, text, 1);
        }
        let item_text: Vec<&str> = pages.iter().map(|(_, text)| *text).collect();
        crate::nlp::fts::fts_index_item(
            &conn,
            "item-paper",
            "El Obrero, 1905",
            "",
            &item_text.join(" "),
        )
        .expect("fts index");

        let ranked = lexical_leg(&conn, "¿Qué dice El Obrero sobre la mujer?", 10)
            .expect("lexical leg should succeed");

        assert_eq!(
            ranked,
            ["page-3"],
            "only the page that matches the question, not the first created"
        );
    }

    /// Ítem de dos páginas: `-a` (creada primero) no trata el tema, `-b` sí.
    fn two_page_item(conn: &Connection, item: &str, matching_text: &str) {
        conn.execute(
            "INSERT OR IGNORE INTO collections(id, name) VALUES ('col-1', 'Archivo')",
            [],
        )
        .expect("collection insert");
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, 'col-1', ?1, '{}')",
            params![item],
        )
        .expect("item insert");
        for (suffix, created_at, text) in
            [("a", 0, "precios y suscripción"), ("b", 1, matching_text)]
        {
            let asset_id = format!("{item}-{suffix}");
            conn.execute(
                "INSERT INTO assets(id, item_id, path, type, created_at)
                 VALUES (?1, ?2, 'scan.png', 'image', ?3)",
                params![asset_id, item, created_at],
            )
            .expect("asset insert");
            insert_extraction(conn, &asset_id, text, 1);
        }
        crate::nlp::fts::fts_index_item(
            conn,
            item,
            item,
            "",
            &format!("precios y suscripción {matching_text}"),
        )
        .expect("fts index");
    }

    #[test]
    fn lexical_leg_matches_pages_written_in_accented_capitals() {
        let conn = setup_rag_db();
        two_page_item(&conn, "item-paper", "EL ÓRGANO OFICIAL DE LA FEDERACIÓN");

        let ranked =
            lexical_leg(&conn, "¿Cuál era el órgano de la federación?", 10).expect("lexical leg");

        assert_eq!(ranked, ["item-paper-b"]);
    }

    #[test]
    fn lexical_leg_reads_the_pages_of_only_the_best_matching_items() {
        let conn = setup_rag_db();
        let items = LEXICAL_SCAN_MIN_ITEMS + 2;
        for index in 0..items {
            two_page_item(&conn, &format!("item-{index:02}"), "el cabildo abierto");
        }

        let ranked = lexical_leg(&conn, "cabildo", 40).expect("lexical leg");

        // Items whose pages were read contribute only the matching page;
        // the rest keep the historical creation order, both pages.
        let unscanned: Vec<&String> = ranked.iter().filter(|id| id.ends_with("-a")).collect();
        assert_eq!(unscanned.len(), 2, "{ranked:?}");
        assert_eq!(ranked.len(), LEXICAL_SCAN_MIN_ITEMS + 2 * 2);
    }

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

    #[test]
    fn retrieval_unit_selection_is_explicit_and_defaults_to_chunk() {
        assert_eq!(
            RetrievalUnit::from_setting(None).expect("missing setting must be valid"),
            RetrievalUnit::Chunk
        );
        assert_eq!(RetrievalUnit::default(), RetrievalUnit::Chunk);
        assert_eq!(
            RetrievalUnit::from_setting(Some("asset")).expect("asset setting must be valid"),
            RetrievalUnit::Asset
        );
        assert_eq!(
            RetrievalUnit::from_setting(Some("chunk")).expect("chunk setting must be valid"),
            RetrievalUnit::Chunk
        );
        assert!(RetrievalUnit::from_setting(Some("page")).is_err());
    }

    #[test]
    fn chunk_source_records_keep_asset_identity_for_citations() {
        let conn = setup_rag_db();
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-1", "Acta"),
            "asset-1",
            "texto completo del activo",
            None,
        );
        conn.execute_batch(
            r#"
            CREATE TABLE rag_chunks (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
                source_kind TEXT NOT NULL,
                source_id TEXT NOT NULL,
                chunk_ordinal INTEGER NOT NULL,
                text_content TEXT NOT NULL,
                start_char INTEGER NOT NULL,
                end_char INTEGER NOT NULL,
                source_text_hash TEXT NOT NULL,
                chunking_contract TEXT NOT NULL,
                embedding BLOB NOT NULL,
                embedding_model TEXT NOT NULL,
                embedding_contract TEXT NOT NULL,
                dimensions INTEGER NOT NULL
            );
            INSERT INTO rag_chunks (
                id, asset_id, item_id, source_kind, source_id, chunk_ordinal,
                text_content, start_char, end_char, source_text_hash,
                chunking_contract, embedding, embedding_model,
                embedding_contract, dimensions
            ) VALUES (
                'chunk-1', 'asset-1', 'item-1', 'extraction', 'ext-asset-1', 0,
                'pasaje preciso para citar', 5, 30, 'source-hash',
                'rag-chunk-v1', X'00000000', 'bge-m3',
                'bge-m3-mean-pool-normalized-v1', 1024
            );
            "#,
        )
        .expect("chunk test row should insert");

        let record = load_source_record_for_unit(&conn, RetrievalUnit::Chunk, "chunk-1")
            .expect("chunk source load should succeed")
            .expect("chunk source should exist");
        let sources = build_sources(
            vec![RrfCandidate { record, score: 1.0 }],
            "pasaje",
            1,
            1600,
            10_000,
        );

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].asset_id, "asset-1");
        assert!(sources[0].snippet.contains("pasaje preciso"));
    }

    fn setup_chunk_packing_db() -> Connection {
        let conn = setup_rag_db();
        conn.execute_batch(
            r#"
            CREATE TABLE rag_chunks (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                source_kind TEXT NOT NULL,
                source_id TEXT NOT NULL,
                chunk_ordinal INTEGER NOT NULL,
                text_content TEXT NOT NULL,
                start_char INTEGER NOT NULL,
                end_char INTEGER NOT NULL,
                source_text_hash TEXT NOT NULL,
                chunking_contract TEXT NOT NULL,
                embedding BLOB NOT NULL,
                embedding_model TEXT NOT NULL,
                embedding_contract TEXT NOT NULL,
                dimensions INTEGER NOT NULL
            );
            "#,
        )
        .expect("chunk packing schema should initialize");
        conn
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_chunk(
        conn: &Connection,
        id: &str,
        asset_id: &str,
        item_id: &str,
        source_kind: &str,
        source_id: &str,
        ordinal: i64,
        text: &str,
        start_char: i64,
        end_char: i64,
    ) {
        conn.execute(
            "INSERT INTO rag_chunks (
                id, asset_id, item_id, source_kind, source_id, chunk_ordinal,
                text_content, start_char, end_char, source_text_hash,
                chunking_contract, embedding, embedding_model,
                embedding_contract, dimensions
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'source-hash',
                'rag-chunk-v1', X'00000000', 'bge-m3',
                'bge-m3-mean-pool-normalized-v1', 1024
             )",
            params![
                id,
                asset_id,
                item_id,
                source_kind,
                source_id,
                ordinal,
                text,
                start_char,
                end_char
            ],
        )
        .expect("canonical chunk should insert");
    }

    fn set_setting(conn: &Connection, key: &str, value: &str) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("settings table");
        conn.execute(
            "INSERT OR REPLACE INTO app_settings(key, value) VALUES (?1, ?2)",
            params![key, value],
        )
        .expect("setting insert");
    }

    /// Chunk de una sola fuente de extracción, con el vector y el sello de
    /// embedding que se le pidan.
    #[allow(clippy::too_many_arguments)]
    fn insert_embedded_chunk(
        conn: &Connection,
        id: &str,
        asset_id: &str,
        item_id: &str,
        ordinal: i64,
        text: &str,
        start_char: i64,
        embedding: &[f32],
        stamp: (&str, &str),
    ) {
        conn.execute(
            "INSERT INTO rag_chunks (
                id, asset_id, item_id, source_kind, source_id, chunk_ordinal,
                text_content, start_char, end_char, source_text_hash,
                chunking_contract, embedding, embedding_model,
                embedding_contract, dimensions
             ) VALUES (
                ?1, ?2, ?3, 'extraction', ?4, ?5, ?6, ?7, ?8, 'source-hash',
                'rag-chunk-v1', ?9, ?10, ?11, ?12
             )",
            params![
                id,
                asset_id,
                item_id,
                format!("ext-{asset_id}-1"),
                ordinal,
                text,
                start_char,
                start_char + text.chars().count() as i64,
                floats_to_blob(embedding),
                stamp.0,
                stamp.1,
                embedding.len() as i64
            ],
        )
        .expect("embedded chunk should insert");
    }

    fn unit_vector(axis: usize) -> Vec<f32> {
        let mut vector = vec![0.0; CANONICAL_EMBEDDING_DIMENSIONS];
        vector[axis] = 1.0;
        vector
    }

    #[test]
    fn chunk_vector_leg_reads_chunks_stamped_with_the_effective_local_contract() {
        let conn = setup_chunk_packing_db();
        set_setting(&conn, "embedding_provider", "local");
        let effective = crate::processing::eligibility::resolve_effective_embedding_contract(&conn)
            .expect("local provider resolves");
        assert_ne!(effective.contract, CANONICAL_EMBEDDING_CONTRACT_V1);
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-1", "Acta"),
            "asset-1",
            "texto sin palabras de la consulta",
            None,
        );
        insert_embedded_chunk(
            &conn,
            "chunk-local",
            "asset-1",
            "item-1",
            0,
            "texto sin palabras de la consulta",
            0,
            &unit_vector(3),
            (&effective.model, &effective.contract),
        );
        insert_embedded_chunk(
            &conn,
            "chunk-other-space",
            "asset-1",
            "item-1",
            1,
            "otro espacio",
            40,
            &unit_vector(3),
            ("otro-modelo", "otro-contrato"),
        );

        let ranked = chunk_vector_leg(&conn, &unit_vector(3), 10, 0.0)
            .expect("chunk vector leg should succeed");

        assert_eq!(ranked, ["chunk-local"]);
    }

    #[test]
    fn lexical_fallback_filters_chunked_pages_before_the_per_item_limit() {
        let conn = setup_chunk_packing_db();
        let effective = crate::processing::eligibility::resolve_effective_embedding_contract(&conn)
            .expect("default provider resolves");
        conn.execute(
            "INSERT INTO collections(id, name) VALUES ('col-1', 'Archivo')",
            [],
        )
        .expect("collection insert");
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata)
             VALUES ('item-big', 'col-1', 'Expediente', '{}')",
            [],
        )
        .expect("item insert");
        for index in 0..25 {
            let asset_id = format!("page-{index:02}");
            conn.execute(
                "INSERT INTO assets(id, item_id, path, type, created_at)
                 VALUES (?1, 'item-big', 'scan.png', 'image', ?2)",
                params![asset_id, index as i64],
            )
            .expect("asset insert");
            insert_extraction(&conn, &asset_id, "acta del cabildo", 1);
            if index < 24 {
                insert_embedded_chunk(
                    &conn,
                    &format!("chunk-{index:02}"),
                    &asset_id,
                    "item-big",
                    0,
                    "acta del cabildo",
                    0,
                    &unit_vector(0),
                    (&effective.model, &effective.contract),
                );
            }
        }
        crate::nlp::fts::fts_index_item(&conn, "item-big", "Expediente", "", "acta del cabildo")
            .expect("fts index");

        // Both the scanned path and the unscanned (creation order) path.
        for scan_items in [LEXICAL_SCAN_MIN_ITEMS, 0] {
            let ranked = lexical_leg_filtered(
                &conn,
                "cabildo",
                24,
                AssetFilter::WithoutChunks(&effective),
                scan_items,
            )
            .expect("lexical leg should succeed");
            assert_eq!(ranked, ["page-24"], "scan_items={scan_items}");
        }
    }

    #[test]
    fn hybrid_retrieval_falls_back_to_the_asset_for_pages_without_effective_chunks() {
        use super::hybrid_fixture::{axis, corpus, CHUNKED_ASSET, PLAIN_ASSET};
        let conn = corpus();
        let embedding = axis(2);
        let query = RetrievalQuery {
            text: "cabildo",
            embedding: Some(&embedding),
        };

        let candidates = hybrid_retrieve_candidates(
            &conn,
            &[query],
            &RagParams::default(),
            RetrievalUnit::Chunk,
        )
        .expect("hybrid retrieval should succeed");
        let kinds: Vec<(&str, bool)> = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.record.asset_id.as_str(),
                    candidate.record.chunk.is_some(),
                )
            })
            .collect();
        assert!(kinds.contains(&(CHUNKED_ASSET, true)), "{kinds:?}");
        assert!(kinds.contains(&(PLAIN_ASSET, false)), "{kinds:?}");
        assert!(
            !kinds.contains(&(CHUNKED_ASSET, false)),
            "a page with chunks is never also retrieved whole: {kinds:?}"
        );

        let sources = pack_sources(&conn, candidates, "cabildo", 6, 1600, 1600, 10_000)
            .expect("mixed evidence should pack");
        let packed: Vec<(&str, bool)> = sources
            .iter()
            .map(|source| (source.asset_id.as_str(), source.provenance.is_some()))
            .collect();
        assert!(packed.contains(&(CHUNKED_ASSET, true)), "{packed:?}");
        assert!(packed.contains(&(PLAIN_ASSET, false)), "{packed:?}");
    }

    #[test]
    fn chunk_lexical_leg_ignores_chunks_of_another_embedding_space() {
        let conn = setup_rag_db();
        conn.execute_batch(RAG_CHUNKS_MIGRATION_SQL)
            .expect("rag_chunks migration");
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-1", "Acta"),
            "asset-1",
            "cabildo abierto",
            None,
        );
        index_canonical_chunk(&conn, "chunk-current", "asset-1", "cabildo abierto");
        insert_embedded_chunk(
            &conn,
            "chunk-stale",
            "asset-1",
            "item-1",
            1,
            "cabildo cerrado",
            20,
            &unit_vector(0),
            ("modelo-anterior", "contrato-anterior"),
        );

        let ranked = chunk_lexical_leg(&conn, "cabildo", 10).expect("chunk leg");

        assert_eq!(ranked, ["chunk-current"]);
    }

    #[test]
    fn hybrid_retrieval_degrades_to_whole_pages_when_the_embedding_space_is_unknown() {
        use super::hybrid_fixture::{corpus, CHUNKED_ASSET, PLAIN_ASSET};
        let conn = corpus();
        set_setting(&conn, "embedding_provider", "mystery");
        let query = RetrievalQuery {
            text: "cabildo",
            embedding: None,
        };

        let candidates = hybrid_retrieve_candidates(
            &conn,
            &[query],
            &RagParams::default(),
            RetrievalUnit::Chunk,
        )
        .expect("an unknown provider must not fail retrieval");

        let mut pages: Vec<(&str, bool)> = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.record.asset_id.as_str(),
                    candidate.record.chunk.is_some(),
                )
            })
            .collect();
        pages.sort();
        assert_eq!(pages, [(CHUNKED_ASSET, false), (PLAIN_ASSET, false)]);
    }

    #[test]
    fn local_provider_retrieves_a_passage_with_no_lexical_match_by_the_chunk_vector_leg() {
        let conn = setup_rag_db();
        conn.execute_batch(RAG_CHUNKS_MIGRATION_SQL)
            .expect("rag_chunks migration");
        set_setting(&conn, "embedding_provider", "local");
        let effective = crate::processing::eligibility::resolve_effective_embedding_contract(&conn)
            .expect("local provider resolves");
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-1", "El Obrero"),
            "asset-1",
            "la emancipación de la mujer",
            None,
        );
        insert_embedded_chunk(
            &conn,
            "chunk-1",
            "asset-1",
            "item-1",
            0,
            "la emancipación de la mujer",
            0,
            &unit_vector(7),
            (&effective.model, &effective.contract),
        );
        let embedding = unit_vector(7);
        let query = RetrievalQuery {
            text: "feminismo libertario",
            embedding: Some(&embedding),
        };

        let candidates = hybrid_retrieve_candidates(
            &conn,
            &[query],
            &RagParams::default(),
            RetrievalUnit::Chunk,
        )
        .expect("hybrid retrieval should succeed");

        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0]
                .record
                .chunk
                .as_ref()
                .map(|chunk| chunk.chunk_ordinal),
            Some(0)
        );
    }

    #[test]
    fn chunk_coverage_counts_pages_with_text_and_those_with_effective_chunks() {
        let conn = super::hybrid_fixture::corpus();
        assert_eq!(
            chunk_coverage(&conn).expect("coverage"),
            RagChunkCoverage {
                assets_with_text: 2,
                assets_with_chunks: 1,
            }
        );

        // Chunks of another embedding space are not coverage.
        set_setting(&conn, "embedding_provider", "local");
        assert_eq!(
            chunk_coverage(&conn).expect("coverage").assets_with_chunks,
            0
        );
    }

    #[test]
    fn hybrid_retrieval_declares_the_degradation_to_whole_pages() {
        use super::hybrid_fixture::corpus;
        let conn = corpus();
        let query = RetrievalQuery {
            text: "cabildo",
            embedding: None,
        };

        let healthy =
            retrieve_reporting(&conn, &[query], &RagParams::default(), RetrievalUnit::Chunk)
                .expect("retrieval");
        assert!(healthy.degradation.is_empty());

        set_setting(&conn, "embedding_provider", "mystery");
        let degraded =
            retrieve_reporting(&conn, &[query], &RagParams::default(), RetrievalUnit::Chunk)
                .expect("an unknown provider must not fail retrieval");
        assert_eq!(degraded.degradation.len(), 1);
        assert_eq!(degraded.degradation[0].code, "whole_pages_only");
        assert!(degraded.degradation[0].detail.contains("mystery"));
        assert_eq!(degraded.candidates.len(), 2);
    }

    fn chunk_candidate(conn: &Connection, chunk_id: &str, score: f64) -> RrfCandidate {
        let record = load_source_record_for_unit(conn, RetrievalUnit::Chunk, chunk_id)
            .expect("chunk source load should succeed")
            .expect("chunk source should exist");
        RrfCandidate { record, score }
    }

    fn provenance(source: &RagSource) -> &crate::rag::RagSourceProvenance {
        source
            .provenance
            .as_ref()
            .expect("chunk evidence must retain citation provenance")
    }

    #[test]
    fn pack_sources_expands_one_hop_and_deduplicates_canonical_overlap() {
        let conn = setup_chunk_packing_db();
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-1", "Acta"),
            "asset-1",
            "texto del activo",
            None,
        );
        insert_chunk(
            &conn,
            "chunk-prev",
            "asset-1",
            "item-1",
            "extraction",
            "ext-1",
            0,
            "ABCDEFGH",
            0,
            8,
        );
        insert_chunk(
            &conn,
            "chunk-center",
            "asset-1",
            "item-1",
            "extraction",
            "ext-1",
            1,
            "GHIJKLMN",
            6,
            14,
        );
        insert_chunk(
            &conn,
            "chunk-next",
            "asset-1",
            "item-1",
            "extraction",
            "ext-1",
            2,
            "MNOPQRST",
            12,
            20,
        );
        insert_chunk(
            &conn,
            "chunk-two-hops",
            "asset-1",
            "item-1",
            "extraction",
            "ext-1",
            3,
            "STUVWXYZ",
            18,
            26,
        );

        let sources = pack_sources(
            &conn,
            vec![chunk_candidate(&conn, "chunk-center", 0.9)],
            "KLMN",
            1,
            100,
            100,
            100,
        )
        .expect("chunk evidence should pack");

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].snippet, "ABCDEFGHIJKLMNOPQRST");
        assert_eq!(sources[0].asset_id, "asset-1");
        assert_eq!(sources[0].index, 1);
        let provenance = provenance(&sources[0]);
        assert_eq!(provenance.retrieval_unit, "chunk");
        assert_eq!(provenance.source_kind, "extraction");
        assert_eq!(provenance.source_id, "ext-1");
        assert_eq!(
            provenance.chunk_ids,
            ["chunk-prev", "chunk-center", "chunk-next"]
        );
        assert_eq!((provenance.start_char, provenance.end_char), (0, 20));
    }

    #[test]
    fn pack_sources_keeps_neighbors_inside_identity_and_budgeted_span() {
        let conn = setup_chunk_packing_db();
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-main", "Acta principal"),
            "asset-main",
            "texto principal",
            None,
        );
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-other", "Acta ajena"),
            "asset-other",
            "texto ajeno",
            None,
        );
        insert_chunk(
            &conn,
            "main-prev",
            "asset-main",
            "item-main",
            "extraction",
            "source-main",
            0,
            "AAAAABBBBB",
            0,
            10,
        );
        insert_chunk(
            &conn,
            "main-center",
            "asset-main",
            "item-main",
            "extraction",
            "source-main",
            1,
            "BBBBBCCCCCDDDDD",
            5,
            20,
        );
        insert_chunk(
            &conn,
            "main-next",
            "asset-main",
            "item-main",
            "extraction",
            "source-main",
            2,
            "DDDDDEEEEE",
            15,
            25,
        );
        insert_chunk(
            &conn,
            "wrong-source-id",
            "asset-main",
            "item-main",
            "extraction",
            "source-other",
            0,
            "WRONGSOURCE",
            0,
            11,
        );
        insert_chunk(
            &conn,
            "wrong-source-kind",
            "asset-main",
            "item-main",
            "transcription",
            "source-main",
            2,
            "WRONGKIND",
            15,
            24,
        );
        insert_chunk(
            &conn,
            "wrong-asset",
            "asset-other",
            "item-other",
            "extraction",
            "source-main",
            0,
            "WRONGASSET",
            0,
            10,
        );

        let pack = |budget: usize| {
            pack_sources(
                &conn,
                vec![chunk_candidate(&conn, "main-center", 0.8)],
                "AAAAA",
                1,
                budget,
                budget,
                budget,
            )
            .expect("budgeted chunk evidence should pack")
        };

        // Over budget, the passage loses whole neighbours (the next one
        // first), never a window around the query term.
        let sources = pack(20);
        assert_eq!(sources[0].snippet, "AAAAABBBBBCCCCCDDDDD");
        let kept = provenance(&sources[0]);
        assert_eq!(kept.chunk_ids, ["main-prev", "main-center"]);
        assert_eq!((kept.start_char, kept.end_char), (0, 20));
        assert!(!sources[0].snippet.contains("WRONG"));

        // The retrieved chunk always stays; only it alone is cut, at its end.
        let sources = pack(12);
        assert_eq!(sources[0].snippet, "BBBBBCCCCCDD");
        let kept = provenance(&sources[0]);
        assert_eq!(kept.chunk_ids, ["main-center"]);
        assert_eq!((kept.start_char, kept.end_char), (5, 17));
    }

    #[test]
    fn pack_sources_budgets_and_counts_reranked_evidence_blocks() {
        let conn = setup_chunk_packing_db();
        for (asset_id, item_id, title, chunk_id, text) in [
            ("asset-a", "item-a", "Primera", "chunk-a", "AAAAA"),
            ("asset-b", "item-b", "Segunda", "chunk-b", "BBBBB"),
            ("asset-c", "item-c", "Tercera", "chunk-c", "CCCCC"),
        ] {
            insert_ocr_doc(
                &conn,
                ("col-1", "Archivo"),
                (item_id, title),
                asset_id,
                text,
                None,
            );
            insert_chunk(
                &conn,
                chunk_id,
                asset_id,
                item_id,
                "extraction",
                &format!("source-{asset_id}"),
                0,
                text,
                0,
                5,
            );
        }
        let candidates = vec![
            chunk_candidate(&conn, "chunk-b", 0.9),
            chunk_candidate(&conn, "chunk-a", 0.8),
            chunk_candidate(&conn, "chunk-c", 0.7),
        ];

        let budgeted = pack_sources(&conn, candidates.clone(), "", 3, 100, 100, 10)
            .expect("context-budgeted evidence should pack");
        let top_k_limited = pack_sources(&conn, candidates, "", 2, 100, 100, 100)
            .expect("top-k evidence should pack");

        for sources in [&budgeted, &top_k_limited] {
            assert_eq!(sources.len(), 2);
            assert_eq!(
                sources
                    .iter()
                    .map(|source| source.asset_id.as_str())
                    .collect::<Vec<_>>(),
                ["asset-b", "asset-a"]
            );
            assert_eq!(
                sources
                    .iter()
                    .map(|source| source.index)
                    .collect::<Vec<_>>(),
                [1, 2]
            );
            assert_eq!(provenance(&sources[0]).chunk_ids, ["chunk-b"]);
            assert_eq!(provenance(&sources[1]).chunk_ids, ["chunk-a"]);
        }
    }

    /// Tres chunks canónicos de 800 con solape de 100 sobre un texto de 2.200
    /// chars. El vecino anterior repite el término más largo de la pregunta;
    /// el central es el que la pierna recuperó.
    fn three_consecutive_chunks(conn: &Connection) -> String {
        let prev = format!("{} ", "periódico obrero cabecera".repeat(40));
        let center = format!("{} ", "la mujer emancipada".repeat(60));
        let next = "cierre del artículo ".repeat(80);
        let text: String = format!("{prev}{center}{next}").chars().take(2200).collect();
        insert_ocr_doc(
            conn,
            ("col-1", "Archivo"),
            ("item-1", "El Obrero"),
            "asset-1",
            &text,
            None,
        );
        let slice = |start: usize, end: usize| -> String {
            text.chars().skip(start).take(end - start).collect()
        };
        for (id, ordinal, start, end) in [
            ("chunk-prev", 0, 0, 800),
            ("chunk-center", 1, 700, 1500),
            ("chunk-next", 2, 1400, 2200),
        ] {
            insert_chunk(
                conn,
                id,
                "asset-1",
                "item-1",
                "extraction",
                "ext-1",
                ordinal,
                &slice(start, end),
                start as i64,
                end as i64,
            );
        }
        text
    }

    #[test]
    fn pack_sources_sends_the_joined_passage_whole_without_a_term_window() {
        let conn = setup_chunk_packing_db();
        let text = three_consecutive_chunks(&conn);

        let sources = pack_sources(
            &conn,
            vec![chunk_candidate(&conn, "chunk-center", 0.9)],
            "¿Qué mirada sobre la mujer tiene el periódico?",
            1,
            1600,
            crate::rag::params::DEFAULT_PASSAGE_MAX_CHARS,
            10_000,
        )
        .expect("chunk evidence should pack");

        assert_eq!(sources[0].snippet, text, "no window cut the passage");
        let provenance = provenance(&sources[0]);
        assert_eq!((provenance.start_char, provenance.end_char), (0, 2200));
        assert_eq!(
            provenance.chunk_ids,
            ["chunk-prev", "chunk-center", "chunk-next"]
        );
    }

    /// Una página con `count` chunks canónicos de 10 chars sin solape, todos
    /// con "cabildo", en el espacio efectivo.
    fn page_with_matching_chunks(conn: &Connection, count: i64) {
        conn.execute_batch(RAG_CHUNKS_MIGRATION_SQL)
            .expect("rag_chunks migration");
        insert_ocr_doc(
            conn,
            ("col-1", "Archivo"),
            ("item-1", "Acta"),
            "asset-1",
            "cabildo",
            None,
        );
        for ordinal in 0..count {
            insert_embedded_chunk(
                conn,
                &format!("chunk-{ordinal}"),
                "asset-1",
                "item-1",
                ordinal,
                &format!("cabildo {ordinal:02}"),
                ordinal * 10,
                &unit_vector(0),
                (CANONICAL_EMBEDDING_MODEL, CANONICAL_EMBEDDING_CONTRACT_V1),
            );
        }
    }

    #[test]
    fn hybrid_retrieval_keeps_up_to_three_passages_per_page() {
        let conn = setup_rag_db();
        page_with_matching_chunks(&conn, 5);
        let query = RetrievalQuery {
            text: "cabildo",
            embedding: None,
        };

        let candidates = hybrid_retrieve_candidates(
            &conn,
            &[query],
            &RagParams::default(),
            RetrievalUnit::Chunk,
        )
        .expect("hybrid retrieval should succeed");

        assert_eq!(candidates.len(), 3);
        assert!(candidates
            .iter()
            .all(|candidate| candidate.record.asset_id == "asset-1"
                && candidate.record.chunk.is_some()));
    }

    #[test]
    fn pack_sources_joins_adjacent_passages_of_one_source_into_one() {
        let conn = setup_rag_db();
        page_with_matching_chunks(&conn, 8);
        let candidates = vec![
            chunk_candidate(&conn, "chunk-2", 0.9),
            chunk_candidate(&conn, "chunk-6", 0.8),
            chunk_candidate(&conn, "chunk-3", 0.7),
        ];

        let sources = pack_sources(&conn, candidates, "cabildo", 6, 1600, 2400, 10_000)
            .expect("passages should pack");

        assert_eq!(sources.len(), 2, "chunk-3 joins chunk-2's passage");
        let joined = provenance(&sources[0]);
        assert_eq!(
            joined.chunk_ids,
            ["chunk-1", "chunk-2", "chunk-3", "chunk-4"]
        );
        assert_eq!((joined.start_char, joined.end_char), (10, 50));
        assert_eq!(
            sources[0].snippet,
            "cabildo 01cabildo 02cabildo 03cabildo 04"
        );
        let apart = provenance(&sources[1]);
        assert_eq!(apart.chunk_ids, ["chunk-5", "chunk-6", "chunk-7"]);
        assert_eq!(sources[1].index, 2);
    }

    #[test]
    fn pack_sources_packs_pages_and_passages_side_by_side() {
        let conn = setup_chunk_packing_db();
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-chunked", "Acta con chunks"),
            "asset-chunked",
            "texto del activo",
            None,
        );
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-page", "Acta sin chunks"),
            "asset-page",
            "el cabildo de la página entera",
            None,
        );
        insert_chunk(
            &conn,
            "chunk-1",
            "asset-chunked",
            "item-chunked",
            "extraction",
            "ext-1",
            0,
            "pasaje del cabildo",
            0,
            18,
        );
        let page = load_source_record(&conn, "asset-page")
            .expect("asset load should succeed")
            .expect("asset should exist");
        let candidates = vec![
            chunk_candidate(&conn, "chunk-1", 0.9),
            RrfCandidate {
                record: page,
                score: 0.8,
            },
        ];

        let sources = pack_sources(&conn, candidates, "cabildo", 6, 100, 100, 1000)
            .expect("mixed evidence should pack");

        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].snippet, "pasaje del cabildo");
        assert!(sources[0].provenance.is_some());
        assert_eq!(sources[1].asset_id, "asset-page");
        assert_eq!(sources[1].snippet, "el cabildo de la página entera");
        assert!(
            sources[1].provenance.is_none(),
            "a whole page has no chunk span"
        );
        assert_eq!(sources[1].index, 2);
    }

    #[test]
    fn pack_sources_asset_mode_preserves_snippets_timestamps_and_no_provenance() {
        let conn = setup_rag_db();
        insert_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-asset", "Acta sonora"),
            "asset-audio",
            "apertura del cabildo histórico",
            Some(r#"[{"start":2.0,"end":5.5,"text":"apertura del cabildo histórico"}]"#),
            None,
        );
        let record = load_source_record(&conn, "asset-audio")
            .expect("asset load should succeed")
            .expect("asset should exist");

        let sources = pack_sources(
            &conn,
            vec![RrfCandidate { record, score: 1.0 }],
            "cabildo",
            1,
            100,
            100,
            100,
        )
        .expect("asset evidence should pack");

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].snippet, "apertura del cabildo histórico");
        assert_eq!(sources[0].start_seconds, Some(2.0));
        assert_eq!(sources[0].end_seconds, Some(5.5));
        assert!(sources[0].provenance.is_none());
    }

    #[test]
    fn hybrid_retrieval_fuses_original_and_rewritten_query_legs() {
        let conn = setup_rag_db();
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-original", "Expediente identificado"),
            "asset-original",
            "El registro EXPEDIENTE1961 conserva el identificador exacto.",
            None,
        );
        insert_ocr_doc(
            &conn,
            ("col-1", "Archivo"),
            ("item-referent", "Historia sindical"),
            "asset-referent",
            "Juan Pérez encabezó la huelga portuaria.",
            None,
        );
        let queries = [
            RetrievalQuery {
                text: "EXPEDIENTE1961",
                embedding: None,
            },
            RetrievalQuery {
                text: "Juan Pérez huelga",
                embedding: None,
            },
        ];
        let params = RagParams {
            top_k: 4,
            rerank_depth: 4,
            candidates_per_leg: 10,
            fusion_candidate_limit: 10,
            ..RagParams::default()
        };

        let candidates = hybrid_retrieve_candidates(&conn, &queries, &params, RetrievalUnit::Asset)
            .expect("all original and rewritten query legs should fuse");
        let asset_ids = candidates
            .iter()
            .map(|candidate| candidate.record.asset_id.as_str())
            .collect::<Vec<_>>();

        assert!(
            asset_ids.contains(&"asset-original"),
            "an exact identifier present only in the original query must not be lost"
        );
        assert!(
            asset_ids.contains(&"asset-referent"),
            "the referent resolved only by the rewritten query must also be retrieved"
        );
    }

    // ── Partial selection equivalence (Phase 5.2a) ───────────────────────────

    /// The historical ranking, kept as the test reference: stable full sort by
    /// score descending and a walk that collects ids with text until `limit`.
    /// Ties keep scan order; `partial_cmp` falling back to `Equal` mirrors the
    /// production comparator (and its unspecified handling of NaN scores).
    fn reference_vector_rank(
        mut scored: Vec<(String, f64)>,
        limit: usize,
        with_text: &HashSet<String>,
    ) -> Vec<String> {
        scored.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut ranked = Vec::new();
        for (asset_id, _) in &scored {
            if ranked.len() == limit {
                break;
            }
            if with_text.contains(asset_id) {
                ranked.push(asset_id.clone());
            }
        }
        ranked
    }

    fn run_partial_selection(
        scored: &[(String, f64)],
        limit: usize,
        with_text: &HashSet<String>,
    ) -> Vec<String> {
        rank_assets_with_text(scored.to_vec(), limit, |asset_id| {
            Ok(with_text.contains(asset_id))
        })
        .expect("ranking should succeed")
    }

    /// Scores descending by index, `count` rows; only the ids in `text` have
    /// text. Scan order equals the vector order, like the production scan.
    fn scored_rows(count: usize, text: &[usize]) -> (Vec<(String, f64)>, HashSet<String>) {
        let scored: Vec<(String, f64)> = (0..count)
            .map(|index| (format!("asset-{index:04}"), 1.0 - index as f64 / 100_000.0))
            .collect();
        let with_text: HashSet<String> = text
            .iter()
            .map(|index| format!("asset-{index:04}"))
            .collect();
        (scored, with_text)
    }

    #[test]
    fn partial_selection_matches_the_full_sort_when_every_asset_has_text() {
        let all: Vec<usize> = (0..500).collect();
        let (scored, with_text) = scored_rows(500, &all);

        let got = run_partial_selection(&scored, 24, &with_text);

        assert_eq!(got.len(), 24);
        assert_eq!(got, reference_vector_rank(scored.clone(), 24, &with_text));
    }

    /// The case a fixed bounded heap breaks: the best `limit * factor`
    /// candidates all lack text, so the walk must continue past the window.
    #[test]
    fn partial_selection_falls_back_when_the_window_has_no_assets_with_text() {
        let text: Vec<usize> = (240..300).collect();
        let (scored, with_text) = scored_rows(300, &text);

        let got = run_partial_selection(&scored, 24, &with_text);

        assert_eq!(got.len(), 24, "the fallback must fill the limit: {got:?}");
        assert_eq!(got[0], "asset-0240");
        assert_eq!(got, reference_vector_rank(scored.clone(), 24, &with_text));
    }

    /// Ties straddle the window boundary on purpose: every score is equal, so
    /// the result is pure scan order and the unstable partition must not
    /// reorder it.
    #[test]
    fn partial_selection_keeps_scan_order_for_score_ties() {
        let scored: Vec<(String, f64)> = (0..1_000)
            .map(|index| (format!("asset-{index:04}"), 0.5))
            .collect();
        let with_text: HashSet<String> = (0..1_000)
            .filter(|index| index % 2 == 1)
            .map(|index| format!("asset-{index:04}"))
            .collect();
        let expected: Vec<String> = (0..48)
            .filter(|index| index % 2 == 1)
            .map(|index| format!("asset-{index:04}"))
            .collect();

        let got = run_partial_selection(&scored, 24, &with_text);

        assert_eq!(got, expected);
        assert_eq!(got, reference_vector_rank(scored.clone(), 24, &with_text));
    }

    #[test]
    fn partial_selection_handles_limits_larger_than_the_candidate_set() {
        let scored = vec![
            ("asset-a".to_string(), 0.9),
            ("asset-b".to_string(), 0.5),
            ("asset-c".to_string(), 0.1),
        ];
        let with_text: HashSet<String> = ["asset-a", "asset-b", "asset-c"]
            .into_iter()
            .map(String::from)
            .collect();

        let got = run_partial_selection(&scored, 50, &with_text);

        assert_eq!(got, vec!["asset-a", "asset-b", "asset-c"]);
        assert_eq!(got, reference_vector_rank(scored.clone(), 50, &with_text));

        // Same set with only the two weakest assets carrying text.
        let with_text: HashSet<String> = ["asset-b", "asset-c"]
            .into_iter()
            .map(String::from)
            .collect();
        let got = run_partial_selection(&scored, 50, &with_text);
        assert_eq!(got, vec!["asset-b", "asset-c"]);
        assert_eq!(got, reference_vector_rank(scored, 50, &with_text));
    }

    #[test]
    fn partial_selection_returns_nothing_for_an_empty_candidate_set() {
        let got = run_partial_selection(&[], 10, &HashSet::new());
        assert!(got.is_empty());
    }

    /// Triangulation: random sizes, duplicate scores (many ties, including
    /// across the window boundary) and random text subsets must all match the
    /// reference walk exactly.
    #[test]
    fn partial_selection_matches_the_reference_on_randomized_sets() {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for case in 0..60 {
            let len = (next() % 2_500) as usize;
            let distinct_scores = 1 + (next() % 25) as usize;
            let scored: Vec<(String, f64)> = (0..len)
                .map(|index| {
                    (
                        format!("asset-{index:05}"),
                        (next() % distinct_scores as u64) as f64,
                    )
                })
                .collect();

            let text_count = (next() as usize) % (len + 1);
            let mut with_text = HashSet::new();
            for _ in 0..text_count {
                let index = (next() as usize) % len;
                with_text.insert(scored[index].0.clone());
            }

            let limit = (next() % 40) as usize;
            let expected = reference_vector_rank(scored.clone(), limit, &with_text);
            let got = run_partial_selection(&scored, limit, &with_text);
            assert_eq!(
                got, expected,
                "case {case}: len {len}, limit {limit}, {text_count} assets with text"
            );
        }
    }

    /// Measurement of the ranking step alone (no SQLite): the historical full
    /// stable sort and walk versus the partial selection, on
    /// `CANONICAL_EMBEDDING_DIMENSIONS`-wide vectors. Ignored by default — it
    /// is a measurement, not an assertion about the machine it runs on.
    ///
    /// ```text
    /// cargo test --profile measure --lib vector_leg_ranking_cost -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "measurement, not a correctness assertion"]
    fn vector_leg_ranking_cost() {
        use std::time::{Duration, Instant};

        const LIMIT: usize = 24; // `rag_candidates_per_leg` default
        const RUNS: usize = 5;

        fn median(mut times: Vec<Duration>) -> Duration {
            times.sort_unstable();
            times[times.len() / 2]
        }

        /// Deterministic cosine similarities against a one-hot query, one per
        /// synthetic vector. Each vector is generated and dropped row by row:
        /// 200k of them stored at once would be ~780 MB, which is why the
        /// production scan streams blobs instead of collecting them.
        fn synthetic_scored_rows(count: usize) -> Vec<(String, f64)> {
            let mut state = 0x5EED_u64;
            let mut next = move || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state % 2001) as f32 / 1000.0 - 1.0
            };
            (0..count)
                .map(|index| {
                    let first = next();
                    let mut squared_norm = (first * first) as f64;
                    for _ in 1..CANONICAL_EMBEDDING_DIMENSIONS {
                        let value = next();
                        squared_norm += (value * value) as f64;
                    }
                    let similarity = if squared_norm > 0.0 {
                        first as f64 / squared_norm.sqrt()
                    } else {
                        0.0
                    };
                    (format!("asset-{index:06}"), similarity)
                })
                .collect()
        }

        for count in [10_000usize, 50_000, 200_000] {
            let scored = synthetic_scored_rows(count);
            let with_text: HashSet<String> = scored.iter().map(|(id, _)| id.clone()).collect();
            let mut old_times = Vec::with_capacity(RUNS);
            let mut new_times = Vec::with_capacity(RUNS);

            for _ in 0..RUNS {
                let old_input = scored.clone();
                let started = Instant::now();
                let old = reference_vector_rank(old_input, LIMIT, &with_text);
                old_times.push(started.elapsed());

                let new_input = scored.clone();
                let started = Instant::now();
                let new = rank_assets_with_text(new_input, LIMIT, |asset_id| {
                    Ok(with_text.contains(asset_id))
                })
                .expect("ranking should succeed");
                new_times.push(started.elapsed());

                assert_eq!(new, old, "the partial selection must match the old ranking");
            }

            println!(
                "vector_leg_ranking_cost: {count:>6} rows  dims {CANONICAL_EMBEDDING_DIMENSIONS}  limit {LIMIT}  old median {:?}  new median {:?}",
                median(old_times),
                median(new_times)
            );
        }
    }
}
