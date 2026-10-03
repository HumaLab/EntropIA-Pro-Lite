//! Bibliographic search commands (E3c-WU3): the thin IPC boundary over
//! [`super::retrieval`]. The command resolves the effective contract from
//! settings, embeds the query through the configured engine only when a
//! queryable space exists, and answers with labeled provenance — it never
//! invents vectors or generations.

use tauri::State;

use crate::bibliography::processing::{EngineProfileEmbedder, ProfileEmbedder};
use crate::bibliography::retrieval::{
    read_work_display, resolve_zotero_library_rows, search_works, HybridAnswer, HybridQuery,
    WorkDisplay, WorkFilters, WorkHit,
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
    /// Scope the search to one Zotero library as the Writing tab names it
    /// (`user`/`group` + native id). Resolved here to the internal library
    /// rows; both fields must be present for the scope to apply.
    pub zotero_library_type: Option<String>,
    pub zotero_library_id: Option<String>,
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
    /// Reading metadata and the native identity of the work's library, so a
    /// result row can be shown and its ficha opened without a second call.
    /// Empty/`None` only if the work vanished between the search and this read.
    pub authors: String,
    pub year: Option<i64>,
    pub library_name: String,
    pub library_type: String,
    pub library_native_id: String,
    pub csl_json: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWorksResponse {
    pub hits: Vec<SearchWorkHitDto>,
    pub vector_available: bool,
    pub active_generation_id: Option<String>,
    pub contract_hash: String,
    /// False only when a Zotero library scope was requested and that library
    /// has never been synced into the catalog: there is nothing to search,
    /// which is different from searching it and finding nothing.
    pub library_synced: bool,
}

fn hit_dto(hit: WorkHit, display: Option<WorkDisplay>) -> SearchWorkHitDto {
    let display = display.unwrap_or(WorkDisplay {
        authors: String::new(),
        year: None,
        library_name: String::new(),
        library_type: String::new(),
        library_native_id: String::new(),
        csl_json: String::new(),
    });
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
        authors: display.authors,
        year: display.year,
        library_name: display.library_name,
        library_type: display.library_type,
        library_native_id: display.library_native_id,
        csl_json: display.csl_json,
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
    let db_path = db.db_path.clone();
    blocking(move || {
        let (plan, _conn) = prepare_plan(&db_path, &chunk_id)?;
        let (opened_path, open_error) = match &plan.path {
            Some(path) => match crate::bibliography::attachment::open_attachment_file(path) {
                Ok(()) => (Some(path.to_string_lossy().to_string()), None),
                Err(error) => (None, Some(error)),
            },
            None => (None, plan_reason(&plan)),
        };
        Ok(passage_response(plan, opened_path, open_error))
    })
    .await
}

/// The same expansion without opening anything: what the in-app passage
/// reader shows (page text with the cited range marked). `openError` says why
/// the original file would not open, so the reader can say so before the user
/// asks; it is `None` when the file resolves.
#[tauri::command]
pub async fn bibliography_passage_context(
    chunk_id: String,
    db: State<'_, AppDbState>,
) -> Result<OpenPassageResponse, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let (plan, _conn) = prepare_plan(&db_path, &chunk_id)?;
        let open_error = plan_reason(&plan);
        Ok(passage_response(plan, None, open_error))
    })
    .await
}

fn prepare_plan(
    db_path: &std::path::Path,
    chunk_id: &str,
) -> Result<
    (
        crate::bibliography::retrieval::PassageOpenPlan,
        rusqlite::Connection,
    ),
    String,
> {
    let conn = open_archive_connection(db_path)?;
    let data_dir = crate::settings::get_setting(
        &conn,
        crate::bibliography::processing::ZOTERO_DATA_DIR_SETTING_KEY,
    );
    let plan =
        crate::bibliography::retrieval::prepare_passage_open(&conn, chunk_id, data_dir.as_deref())
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
    Ok((plan, conn))
}

fn plan_reason(plan: &crate::bibliography::retrieval::PassageOpenPlan) -> Option<String> {
    plan.reason
        .as_ref()
        .map(|(reason, detail)| format!("{reason}: {detail}"))
}

fn passage_response(
    plan: crate::bibliography::retrieval::PassageOpenPlan,
    opened_path: Option<String>,
    open_error: Option<String>,
) -> OpenPassageResponse {
    OpenPassageResponse {
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
    }
}

/// The synced libraries the chat's scope control offers.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStatusDto {
    pub library_type: String,
    pub library_id: String,
    pub name: String,
    pub works: i64,
    pub passages: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStatusResponse {
    pub libraries: Vec<LibraryStatusDto>,
    pub vector_ready: bool,
}

/// Which Zotero libraries are synced into the catalog (with work and passage
/// counts) and whether an embedding generation is active, so the chat can say
/// honestly why a Biblioteca question would find nothing.
#[tauri::command]
pub async fn bibliography_library_status(
    db: State<'_, AppDbState>,
) -> Result<LibraryStatusResponse, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let effective =
            crate::processing::eligibility::resolve_effective_embedding_contract(&conn)?;
        let status = crate::bibliography::retrieval::library_status(&conn, &effective.hash)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
        Ok(LibraryStatusResponse {
            libraries: status
                .libraries
                .into_iter()
                .map(|row| LibraryStatusDto {
                    library_type: row.library_type,
                    library_id: row.library_native_id,
                    name: row.name,
                    works: row.works,
                    passages: row.passages,
                })
                .collect(),
            vector_ready: status.vector_ready,
        })
    })
    .await
}

fn answer_dto(conn: &rusqlite::Connection, answer: HybridAnswer) -> SearchWorksResponse {
    SearchWorksResponse {
        hits: answer
            .hits
            .into_iter()
            .map(|hit| {
                let display = read_work_display(conn, &hit.item_id).ok().flatten();
                hit_dto(hit, display)
            })
            .collect(),
        vector_available: answer.vector_available,
        active_generation_id: answer.active_generation_id,
        contract_hash: answer.contract_hash,
        library_synced: true,
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
        let mut library_ids = request.library_ids.unwrap_or_default();
        if let (Some(library_type), Some(native_id)) =
            (&request.zotero_library_type, &request.zotero_library_id)
        {
            let rows = resolve_zotero_library_rows(&conn, library_type, native_id)
                .map_err(|error| format!("{}: {}", error.code, error.message))?;
            if rows.is_empty() {
                return Ok(SearchWorksResponse {
                    hits: Vec::new(),
                    vector_available: false,
                    active_generation_id: None,
                    contract_hash: effective.hash,
                    library_synced: false,
                });
            }
            library_ids.extend(rows);
        }
        let query = HybridQuery {
            text: request.text,
            top_k: request.top_k.unwrap_or(20).clamp(1, 100),
            filters: WorkFilters {
                library_ids,
                year_from: request.year_from,
                year_to: request.year_to,
                item_types: request.item_types.unwrap_or_default(),
                tags: request.tags.unwrap_or_default(),
            },
        };
        let embedder = EngineProfileEmbedder::new(db_path);
        let answer = search_works(&conn, &effective.hash, &query, &|text| embedder.embed(text))
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
        Ok(answer_dto(&conn, answer))
    })
    .await
}

/// One passage search request from the Writing "Obras" tab.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPassagesRequest {
    pub text: String,
    pub top_k: Option<usize>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPassagesResponse {
    pub passages: Vec<crate::rag::scope::PassageResult>,
    /// Why nothing was searched (same codes as the chat's Biblioteca notice:
    /// `no_library_synced`, `no_embeddings`, `embedding_unavailable`,
    /// `failed`); `None` when the search ran.
    pub notice: Option<String>,
}

/// Passages of the synced Zotero libraries for a query, ranked by similarity
/// (vector-only: with no active generation the answer is empty and says
/// `no_embeddings`). Same leg, floor and location rules as the research chat.
#[tauri::command]
pub async fn bibliography_search_passages(
    request: SearchPassagesRequest,
    db: State<'_, AppDbState>,
) -> Result<SearchPassagesResponse, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let effective =
            crate::processing::eligibility::resolve_effective_embedding_contract(&conn)?;
        let mut params = crate::rag::params::rag_params_from_settings(&conn);
        params.top_k = request.top_k.unwrap_or(12).clamp(1, 50);
        let embedder = EngineProfileEmbedder::new(db_path);
        let found = crate::rag::scope::passage_search(
            &conn,
            &effective.hash,
            &request.text,
            &[],
            &params,
            &|text| embedder.embed(text),
        );
        Ok(SearchPassagesResponse {
            passages: found.passages,
            notice: found.notice.map(|notice| notice.code().to_string()),
        })
    })
    .await
}
