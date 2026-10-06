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
/// original the in-app viewer may show, and the reason when there is none.
/// The highlight renders in both cases: the original is a courtesy, never a
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
    /// `"pdf"` or `"html"` when the original can be shown inside the app.
    pub original_kind: Option<String>,
    /// The canonical PDF path the viewer was just granted (and only that
    /// one file). `None` for HTML snapshots and for the no-grant context.
    pub original_path: Option<String>,
    pub open_error: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenPassagePageDto {
    pub page_number: i64,
    pub text: String,
    pub highlights: Vec<(i64, i64)>,
}

/// Prepares one passage's original for viewing inside EntropIA. The
/// frontend names a chunk, never a path: the file comes from the chunk's
/// registered attachment, is canonicalized and validated as an existing
/// regular PDF, and only then is that single file allowed on the asset
/// protocol at runtime (the static scope is untouched). Nothing is launched
/// outside the app and a directory is never opened.
#[tauri::command]
pub async fn bibliography_open_passage(
    chunk_id: String,
    app: tauri::AppHandle,
    db: State<'_, AppDbState>,
) -> Result<OpenPassageResponse, String> {
    use tauri::Manager;
    let db_path = db.db_path.clone();
    blocking(move || {
        let (plan, _conn) = prepare_plan(&db_path, &chunk_id)?;
        let mut open_error = plan_reason(&plan);
        let mut granted = None;
        if let Some(path) = plan
            .original
            .as_ref()
            .and_then(|original| original.path.as_ref())
        {
            match app.asset_protocol_scope().allow_file(path) {
                Ok(()) => granted = Some(path.to_string_lossy().to_string()),
                Err(error) => {
                    open_error = Some(format!("scope_grant_failed: {error}"));
                }
            }
        }
        let mut response = passage_response(plan, granted.clone(), open_error);
        if granted.is_none() && response.original_kind.as_deref() == Some("pdf") {
            response.original_kind = None;
        }
        Ok(response)
    })
    .await
}

/// The same expansion without granting anything: what the in-app passage
/// reader shows (page text with the cited range marked). `openError` says why
/// the original would not open, so the reader can say so before the user
/// asks; it is `None` when an original is viewable.
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
    original_path: Option<String>,
    open_error: Option<String>,
) -> OpenPassageResponse {
    let original_kind = plan.original.as_ref().map(|original| match original.kind {
        crate::bibliography::retrieval::OriginalKind::Pdf => "pdf".to_string(),
        crate::bibliography::retrieval::OriginalKind::Html => "html".to_string(),
    });
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
        original_kind,
        original_path,
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
    app_handle: tauri::AppHandle,
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
            .map_err(|error| {
                let cause = format!("{}: {}", error.code, error.message);
                crate::app_logs::error(
                    &app_handle,
                    "bibliografia/obras",
                    format!(
                        "La búsqueda de obras falló: {}",
                        crate::rag::scope::short_cause(&cause)
                    ),
                );
                cause
            })?;
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
    /// Also match close variants of the words. `None` follows the shared
    /// search preference (the Corpus tab's switch).
    pub fuzzy: Option<bool>,
    /// Restrict the search to one Zotero library (type + native id), as the
    /// Writing -> Zotero tab names it; both or neither.
    pub zotero_library_type: Option<String>,
    pub zotero_library_id: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPassagesResponse {
    pub passages: Vec<crate::rag::scope::PassageResult>,
    /// Why nothing was searched (same codes as the chat's Biblioteca notice:
    /// `no_library_synced`, `no_embeddings`, `embedding_unavailable`,
    /// `failed`); `None` when the search ran.
    pub notice: Option<String>,
    /// Short, secret-free cause behind `failed` / `embedding_unavailable`.
    pub notice_detail: Option<String>,
}

/// Passages of the synced Zotero libraries for a query: the ones whose text
/// carries the words (exact, or close variants) and the ones whose vector is
/// near, fused by rank (with no active generation the answer is empty and
/// says `no_embeddings`). Same leg, floor and location rules as the chat.
#[tauri::command]
pub async fn bibliography_search_passages(
    request: SearchPassagesRequest,
    db: State<'_, AppDbState>,
    app_handle: tauri::AppHandle,
) -> Result<SearchPassagesResponse, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let effective =
            crate::processing::eligibility::resolve_effective_embedding_contract(&conn)?;
        let mut params = crate::rag::params::rag_params_from_settings(&conn);
        params.top_k = request.top_k.unwrap_or(12).clamp(1, 50);
        let embedder = EngineProfileEmbedder::new(db_path);
        let libraries = match (&request.zotero_library_type, &request.zotero_library_id) {
            (Some(library_type), Some(library_id)) => vec![crate::rag::scope::RagLibraryRef {
                library_type: library_type.clone(),
                library_id: library_id.clone(),
            }],
            _ => Vec::new(),
        };
        let fuzzy = request
            .fuzzy
            .unwrap_or_else(|| crate::nlp::fuzzy::fuzzy_enabled(&conn));
        let found = crate::rag::scope::passage_search(
            &conn,
            &effective.hash,
            &request.text,
            &libraries,
            &params,
            fuzzy,
            &|text| embedder.embed(text),
        );
        if let (Some(notice), Some(detail)) = (found.notice, &found.detail) {
            crate::app_logs::warn(
                &app_handle,
                "bibliografia/pasajes",
                format!(
                    "La búsqueda de pasajes no aportó ({}): {detail}",
                    notice.code()
                ),
            );
        }
        Ok(SearchPassagesResponse {
            passages: found.passages,
            notice: found.notice.map(|notice| notice.code().to_string()),
            notice_detail: found.detail,
        })
    })
    .await
}

// ── Biblioteca (P2) ─────────────────────────────────────────────────────────

/// One page of the Biblioteca listing from the UI: paging, an optional
/// substring filter, the optional Zotero library scope (type + native id,
/// both or neither — resolved here to the internal library rows), and the
/// order to list in (`"title"` — the default, and what older callers omit —
/// or `"recent"`).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListWorksRequest {
    pub offset: Option<i64>,
    pub limit: Option<i64>,
    pub query: Option<String>,
    pub sort: Option<String>,
    pub zotero_library_type: Option<String>,
    pub zotero_library_id: Option<String>,
}

/// One page of the catalog's works in the requested order (tombstones
/// excluded) with the scope's total, so the Biblioteca can page a library of
/// thousands without counting on its own. Read-only.
#[tauri::command]
pub async fn bibliography_list_works(
    request: ListWorksRequest,
    db: State<'_, AppDbState>,
) -> Result<crate::bibliography::work_view::WorkListPage, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let sort = match request.sort.as_deref() {
            // The order every caller without a preference names, and what
            // older callers that send no `sort` mean.
            None | Some("title") => crate::bibliography::work_view::WorkListSort::Title,
            Some("recent") => crate::bibliography::work_view::WorkListSort::Recent,
            Some(other) => return Err(format!("invalid_sort: unknown work order {other:?}")),
        };
        crate::bibliography::work_view::list_works(
            &conn,
            &crate::bibliography::work_view::WorkListRequest {
                library_type: request.zotero_library_type.as_deref(),
                library_native_id: request.zotero_library_id.as_deref(),
                query: request.query.as_deref(),
                sort,
                offset: request.offset.unwrap_or(0),
                limit: request.limit.unwrap_or(50),
            },
        )
        .map_err(|error| format!("{}: {}", error.code, error.message))
    })
    .await
}

/// One work's ficha for the Biblioteca work view: the display line (authors,
/// year, library) plus the catalog projection of the item — metadata,
/// collections, tags and attachment metadata. Read-only.
#[tauri::command]
pub async fn bibliography_work_detail(
    item_id: String,
    db: State<'_, AppDbState>,
) -> Result<crate::bibliography::work_view::WorkDetail, String> {
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        crate::bibliography::work_view::work_detail(&conn, &item_id)
            .map_err(|error| format!("{}: {}", error.code, error.message))
    })
    .await
}

/// One work attachment prepared for the in-app viewer, with the extracted
/// page texts beside it. Same grant discipline as `bibliography_open_passage`:
/// the frontend names the work and the attachment key, never a path; the file
/// comes from the cataloged attachment row, is validated as an existing
/// regular PDF (HTML snapshots need no file), and only then is that single
/// file allowed on the asset protocol at runtime. The extracted texts travel
/// even when no original is viewable — the "Texto" tab is never gated on the
/// file.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenWorkAttachmentResponse {
    pub item_id: String,
    pub item_key: String,
    pub title: String,
    pub attachment_key: String,
    /// `"pdf"` or `"html"` when the original can be shown inside the app.
    pub original_kind: Option<String>,
    /// The canonical PDF path the viewer was just granted (and only that one
    /// file). `None` for HTML snapshots and whenever nothing was granted.
    pub original_path: Option<String>,
    pub open_error: Option<String>,
    pub pages: Vec<crate::bibliography::work_view::WorkPageText>,
    /// The whole-document extraction, empty when never extracted.
    pub snapshot_text: String,
    /// False when neither page texts nor a whole-document extraction exist.
    pub extracted: bool,
}

#[tauri::command]
pub async fn bibliography_open_work_attachment(
    item_id: String,
    attachment_key: String,
    app: tauri::AppHandle,
    db: State<'_, AppDbState>,
) -> Result<OpenWorkAttachmentResponse, String> {
    use tauri::Manager;
    let db_path = db.db_path.clone();
    blocking(move || {
        let conn = open_archive_connection(&db_path)?;
        let data_dir = crate::settings::get_setting(
            &conn,
            crate::bibliography::processing::ZOTERO_DATA_DIR_SETTING_KEY,
        );
        let plan = crate::bibliography::work_view::prepare_work_attachment_open(
            &conn,
            &item_id,
            &attachment_key,
            data_dir.as_deref(),
        )
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
        let mut open_error = plan
            .reason
            .as_ref()
            .map(|(reason, detail)| format!("{reason}: {detail}"));
        let mut granted = None;
        if let Some(path) = plan
            .original
            .as_ref()
            .and_then(|original| original.path.as_ref())
        {
            match app.asset_protocol_scope().allow_file(path) {
                Ok(()) => granted = Some(path.to_string_lossy().to_string()),
                Err(error) => {
                    open_error = Some(format!("scope_grant_failed: {error}"));
                }
            }
        }
        let mut original_kind = plan.original.as_ref().map(|original| match original.kind {
            crate::bibliography::attachment::OriginalKind::Pdf => "pdf".to_string(),
            crate::bibliography::attachment::OriginalKind::Html => "html".to_string(),
        });
        // A PDF only shows when its one file was actually granted.
        if granted.is_none() && original_kind.as_deref() == Some("pdf") {
            original_kind = None;
        }
        Ok(OpenWorkAttachmentResponse {
            item_id: plan.item_id,
            item_key: plan.item_key,
            title: plan.title,
            attachment_key: plan.attachment_key,
            original_kind,
            original_path: granted,
            open_error,
            pages: plan.pages,
            snapshot_text: plan.snapshot_text,
            extracted: plan.extracted,
        })
    })
    .await
}
