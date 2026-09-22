//! Bibliographic search commands (E3c-WU3): the thin IPC boundary over
//! [`super::retrieval`]. The command resolves the effective contract from
//! settings, embeds the query through the configured engine only when a
//! queryable space exists, and answers with labeled provenance — it never
//! invents vectors or generations.

use tauri::State;

use crate::bibliography::processing::{EngineProfileEmbedder, ProfileEmbedder};
use crate::bibliography::retrieval::{
    search_works, HybridAnswer, HybridQuery, WorkFilters, WorkHit,
};
use crate::db::open::open_archive_connection;
use crate::db::state::AppDbState;

async fn blocking<F, T>(task: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| format!("bibliography search task failed: {e}"))?
}

/// One hybrid work-search request from the UI.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWorksRequest {
    pub text: String,
    pub top_k: Option<usize>,
    pub library_ids: Option<Vec<String>>,
    pub year_from: Option<i64>,
    pub year_to: Option<i64>,
    pub item_types: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWorkHitDto {
    pub item_id: String,
    pub item_key: String,
    pub library_id: String,
    pub title: String,
    pub method: String,
    pub lexical_score: Option<f64>,
    pub vector_score: Option<f64>,
    pub fused_score: f64,
    pub contract_hash: Option<String>,
    pub generation_id: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWorksResponse {
    pub hits: Vec<SearchWorkHitDto>,
    pub vector_available: bool,
    pub active_generation_id: Option<String>,
    pub contract_hash: String,
}

fn hit_dto(hit: WorkHit) -> SearchWorkHitDto {
    SearchWorkHitDto {
        item_id: hit.item_id,
        item_key: hit.item_key,
        library_id: hit.library_id,
        title: hit.title,
        method: hit.method,
        lexical_score: hit.lexical_score,
        vector_score: hit.vector_score,
        fused_score: hit.fused_score,
        contract_hash: hit.contract_hash,
        generation_id: hit.generation_id,
    }
}

fn answer_dto(answer: HybridAnswer) -> SearchWorksResponse {
    SearchWorksResponse {
        hits: answer.hits.into_iter().map(hit_dto).collect(),
        vector_available: answer.vector_available,
        active_generation_id: answer.active_generation_id,
        contract_hash: answer.contract_hash,
    }
}

/// Hybrid work search: lexical FTS5 plus vector cosine over the active
/// generation of the effective contract, fused by rank with catalog
/// filters and full provenance. With no queryable space the answer is
/// lexical-only and says so (`vectorAvailable == false`).
#[tauri::command]
pub async fn bibliography_search_works(
    request: SearchWorksRequest,
    db: State<'_, AppDbState>,
) -> Result<SearchWorksResponse, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let effective =
            crate::processing::eligibility::resolve_effective_embedding_contract(&conn)?;
        let query = HybridQuery {
            text: request.text,
            top_k: request.top_k.unwrap_or(20).clamp(1, 100),
            filters: WorkFilters {
                library_ids: request.library_ids.unwrap_or_default(),
                year_from: request.year_from,
                year_to: request.year_to,
                item_types: request.item_types.unwrap_or_default(),
                tags: request.tags.unwrap_or_default(),
            },
        };
        let embedder = EngineProfileEmbedder::new(db_path);
        let answer = search_works(&conn, &effective.hash, &query, &|text| embedder.embed(text))
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
        Ok(answer_dto(answer))
    })
    .await
}
