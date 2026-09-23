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

/// One passage opening: the expansion for the highlight surface, the
/// opened path when a file resolved, and the resolver reason otherwise.
/// The highlight renders in both cases — opening is a courtesy, never a
/// gate.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenPassageResponse {
    pub chunk_id: String,
    pub item_id: String,
    pub item_key: String,
    pub title: String,
    pub text: String,
    pub spans: Vec<(i64, i64, i64)>,
    pub pages: Vec<OpenPassagePageDto>,
    pub opened_path: Option<String>,
    pub open_error: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenPassagePageDto {
    pub page_number: i64,
    pub text: String,
    pub highlights: Vec<(i64, i64)>,
}

/// Opens one passage's original PDF through the attachment resolver and
/// the validated OS file opener, answering the expansion for the
/// highlight surface either way: `openedPath` names the file the OS took,
/// `openError` carries the resolver or opener reason when none did.
#[tauri::command]
pub async fn bibliography_open_passage(
    chunk_id: String,
    db: State<'_, AppDbState>,
) -> Result<OpenPassageResponse, String> {
    use crate::bibliography::retrieval::prepare_passage_open;
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let data_dir = crate::settings::get_setting(
            &conn,
            crate::bibliography::processing::ZOTERO_DATA_DIR_SETTING_KEY,
        );
        let plan = prepare_passage_open(&conn, &chunk_id, data_dir.as_deref())
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
        let (opened_path, open_error) = match plan.path {
            Some(path) => match crate::bibliography::attachment::open_attachment_file(&path) {
                Ok(()) => (Some(path.to_string_lossy().to_string()), None),
                Err(error) => (None, Some(error)),
            },
            None => (
                None,
                plan.reason
                    .map(|(reason, detail)| format!("{reason}: {detail}")),
            ),
        };
        Ok(OpenPassageResponse {
            chunk_id: plan.expansion.chunk_id,
            item_id: plan.expansion.item_id,
            item_key: plan.expansion.item_key,
            title: plan.expansion.title,
            text: plan.expansion.text,
            spans: plan.expansion.spans,
            pages: plan
                .expansion
                .pages
                .into_iter()
                .map(|page| OpenPassagePageDto {
                    page_number: page.page_number,
                    text: page.text,
                    highlights: page.highlights,
                })
                .collect(),
            opened_path,
            open_error,
        })
    })
    .await
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
