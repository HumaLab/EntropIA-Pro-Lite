//! Tauri surface for the writing workspace.
//!
//! These commands return `Result<T, WritingError>` rather than the
//! `Result<T, String>` the rest of the backend uses. §20 asks for typed,
//! actionable errors, and the one the frontend must branch on — a stale save
//! losing to a concurrent one — is impossible to distinguish reliably by
//! matching message text. `WritingError` serialises as `{ code, message }`,
//! so the caller switches on `code` and shows `message`.

use tauri::State;

use super::repository::{
    self, DocumentRow, NewDocument, SaveDocument, WritingError, WritingResult,
};
use super::{journal, recovery, versions};
use crate::db::open::open_archive_connection;
use crate::db::state::AppDbState;

fn open(db_path: &std::path::Path) -> WritingResult<rusqlite::Connection> {
    open_archive_connection(db_path).map_err(|e| WritingError::new("db_unavailable", e))
}

fn joined(context: &str, e: tokio::task::JoinError) -> WritingError {
    WritingError::new("task_failed", format!("{context}: {e}"))
}

/// Whether the writing schema has been migrated. The frontend calls this after
/// `initStore()` resolves and keeps the section disabled until it reports true,
/// exactly as the batch queue gates on `processing_initialize`.
#[tauri::command]
pub async fn writing_is_ready(db: State<'_, AppDbState>) -> WritingResult<bool> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::is_schema_ready(&open(&db_path)?))
        .await
        .map_err(|e| joined("writing_is_ready", e))?
}

#[tauri::command]
pub async fn writing_create_document(
    db: State<'_, AppDbState>,
    input: NewDocument,
) -> WritingResult<DocumentRow> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::create_document(&open(&db_path)?, input))
        .await
        .map_err(|e| joined("writing_create_document", e))?
}

#[tauri::command]
pub async fn writing_load_document(
    db: State<'_, AppDbState>,
    id: String,
) -> WritingResult<DocumentRow> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::load_document(&open(&db_path)?, &id))
        .await
        .map_err(|e| joined("writing_load_document", e))?
}

/// The section's document list (§6.1). `statuses` filters by lifecycle state;
/// an empty list means every state, so the trash view and the active view are
/// the same call with different arguments.
#[tauri::command]
pub async fn writing_list_documents(
    db: State<'_, AppDbState>,
    statuses: Vec<String>,
) -> WritingResult<Vec<DocumentRow>> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::list_documents(&open(&db_path)?, &statuses))
        .await
        .map_err(|e| joined("writing_list_documents", e))?
}

/// Advances the manuscript one revision. Returns the new revision, so the UI
/// can only ever confirm "Guardado" against a number persistence acknowledged
/// (§16.1). A `revision_conflict` here means another window won; the caller
/// reloads and merges rather than retrying blindly.
#[tauri::command]
pub async fn writing_save_document(
    db: State<'_, AppDbState>,
    save: SaveDocument,
) -> WritingResult<i64> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = open(&db_path)?;
        repository::save_document(&mut conn, save)
    })
    .await
    .map_err(|e| joined("writing_save_document", e))?
}

/// A title is metadata, so this does not advance the content revision: renaming
/// while an edit is in flight must not turn that edit into a conflict.
#[tauri::command]
pub async fn writing_rename_document(
    db: State<'_, AppDbState>,
    id: String,
    title: String,
) -> WritingResult<()> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::rename_document(&open(&db_path)?, &id, &title))
        .await
        .map_err(|e| joined("writing_rename_document", e))?
}

/// Moves a document between `active`, `archived` and `trashed` (§9.1). One
/// command rather than three, because they are one transition with three
/// destinations; an unknown value answers `invalid_status` without touching
/// the database.
#[tauri::command]
pub async fn writing_set_status(
    db: State<'_, AppDbState>,
    id: String,
    status: String,
) -> WritingResult<()> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || repository::set_status(&open(&db_path)?, &id, &status))
        .await
        .map_err(|e| joined("writing_set_status", e))?
}

/// Appends one delta to the recovery journal and returns its **sequence
/// number** — never a revision. §16.2 draws the line hard: persisting here
/// leaves the UI on "Cambios pendientes"; only `writing_save_document`
/// advancing the revision earns "Guardado".
#[tauri::command]
pub async fn writing_append_journal(
    db: State<'_, AppDbState>,
    entry: journal::AppendJournal,
) -> WritingResult<i64> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || journal::append(&open(&db_path)?, entry))
        .await
        .map_err(|e| joined("writing_append_journal", e))?
}

/// What can be recovered for a document, and what cannot (§16.3). Read-only:
/// building a plan changes nothing, so it is safe to call on every open.
#[tauri::command]
pub async fn writing_recovery_plan(
    db: State<'_, AppDbState>,
    document_id: String,
) -> WritingResult<recovery::RecoveryPlan> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || recovery::plan(&open(&db_path)?, &document_id))
        .await
        .map_err(|e| joined("writing_recovery_plan", e))?
}

/// Drops journal entries a confirmed revision already contains. Deliberately
/// not automatic on save: §16.1 requires the journal protecting a sequence to
/// survive until a canonical version containing it exists, so the caller prunes
/// once it has seen the new revision come back.
#[tauri::command]
pub async fn writing_prune_journal(
    db: State<'_, AppDbState>,
    document_id: String,
    confirmed_revision: i64,
) -> WritingResult<usize> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        journal::prune_confirmed(&open(&db_path)?, &document_id, confirmed_revision)
    })
    .await
    .map_err(|e| joined("writing_prune_journal", e))?
}

/// Discards a recovery offer outright. §16.3 asks for a confirmation before
/// this runs and for the copy to stay recoverable for a while; both belong to
/// the caller, because only it knows the human said yes.
#[tauri::command]
pub async fn writing_discard_journal(
    db: State<'_, AppDbState>,
    document_id: String,
) -> WritingResult<usize> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || journal::discard_all(&open(&db_path)?, &document_id))
        .await
        .map_err(|e| joined("writing_discard_journal", e))?
}

/// Writes a snapshot of the document as it stands (§9.3). `reason` records why:
/// only "auto" is ever compacted by retention.
#[tauri::command]
pub async fn writing_snapshot_version(
    db: State<'_, AppDbState>,
    document_id: String,
    reason: String,
) -> WritingResult<i64> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || versions::snapshot(&open(&db_path)?, &document_id, &reason))
        .await
        .map_err(|e| joined("writing_snapshot_version", e))?
}

/// The history panel's list, newest first (§16.4).
#[tauri::command]
pub async fn writing_list_versions(
    db: State<'_, AppDbState>,
    document_id: String,
) -> WritingResult<Vec<versions::VersionSummary>> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || versions::list(&open(&db_path)?, &document_id))
        .await
        .map_err(|e| joined("writing_list_versions", e))?
}

/// One version's content plus the settings that rendered it, so the caller can
/// preview it and derive the projections a restore will need.
#[tauri::command]
pub async fn writing_read_version(
    db: State<'_, AppDbState>,
    document_id: String,
    version_number: i64,
) -> WritingResult<versions::VersionContent> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        versions::read(&open(&db_path)?, &document_id, version_number)
    })
    .await
    .map_err(|e| joined("writing_read_version", e))?
}

/// Reinstates an earlier version as a NEW revision (§16.4). The history after
/// it survives, and the state being replaced is snapshotted first.
#[tauri::command]
pub async fn writing_restore_version(
    db: State<'_, AppDbState>,
    restore: versions::RestoreVersion,
) -> WritingResult<i64> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = open(&db_path)?;
        versions::restore(&mut conn, restore)
    })
    .await
    .map_err(|e| joined("writing_restore_version", e))?
}

/// Compacts automatic snapshots, keeping the most recent `keep` (§9.3). A
/// checkpoint, a close, a restore or a migration records a human decision and
/// is never a candidate.
#[tauri::command]
pub async fn writing_apply_retention(
    db: State<'_, AppDbState>,
    document_id: String,
    keep: usize,
) -> WritingResult<usize> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        versions::apply_retention(&open(&db_path)?, &document_id, keep)
    })
    .await
    .map_err(|e| joined("writing_apply_retention", e))?
}

/// Duplicates a document per §8.4: own identity, own citation occurrences, a
/// recorded origin, and none of the original's pending suggestions or history.
/// The Zotero client, built once. `reqwest` pools connections, so rebuilding it
/// per call would open a fresh socket for every keystroke in the search box.
fn zotero_client() -> WritingResult<reqwest::Client> {
    reqwest::Client::builder()
        .build()
        .map_err(|e| WritingError::new("http_unavailable", format!("{e}")))
}

/// What can honestly be said about Zotero right now (§11.3).
///
/// Never fails: an unreachable Zotero is a state to report, not an error. §11.3
/// is explicit that Zotero's problems must not block editing or saving, and a
/// command that returned `Err` here would make the panel look broken rather
/// than the library look absent.
#[tauri::command]
pub async fn writing_zotero_probe() -> WritingResult<super::zotero::ZoteroState> {
    Ok(super::zotero::connector::probe(&zotero_client()?).await)
}

/// One page of the library, as CSL-JSON (§11.2).
///
/// The page is the caller's to choose but not to remove: `Page::new` clamps it,
/// so no caller can ask for the whole library in one response. S5 measured what
/// that costs — 7.9 MB in a single payload.
#[tauri::command]
pub async fn writing_zotero_items(
    library: String,
    start: u32,
    limit: u32,
    query: Option<String>,
) -> WritingResult<super::zotero::connector::LibraryPage> {
    let client = zotero_client()?;
    let page = super::zotero::connector::Page::new(start, limit);
    super::zotero::connector::fetch_items(&client, &library, page, query.as_deref())
        .await
        .map_err(|state| {
            // The state is the diagnosis; the code is what the frontend
            // branches on. Both travel, because "api_disabled" needs different
            // words on screen than "timeout".
            let code = match state {
                super::zotero::ZoteroState::ApiDisabled => "zotero_api_disabled",
                super::zotero::ZoteroState::Timeout => "zotero_timeout",
                super::zotero::ZoteroState::EndpointUnavailable => "zotero_unavailable",
                _ => "zotero_invalid_response",
            };
            WritingError::new(code, format!("{state:?}"))
        })
}

/// Renders one citation cluster (§11.5).
///
/// Nothing is stored: the text is derived from the cluster's CSL data every
/// time, which is what makes a change of style re-render the whole manuscript
/// instead of leaving old strings behind.
#[tauri::command]
pub async fn writing_csl_render(
    items: Vec<super::csl::render::ClusterItem>,
    style: super::csl::render::StyleSource,
) -> Result<super::csl::render::RenderedCluster, super::csl::render::CslError> {
    tokio::task::spawn_blocking(move || super::csl::render::render_cluster(&items, &style))
        .await
        .map_err(|e| super::csl::render::CslError {
            code: "task_failed".into(),
            message: format!("writing_csl_render: {e}"),
        })?
}

/// Renders every citation of a manuscript together (§11.5).
///
/// Disambiguation is a property of the manuscript: which of two works by one
/// author in one year reads `2015a` depends on all the others. Rendering one
/// citation at a time can only ever produce `2015` twice, so the whole document
/// goes through one engine.
#[tauri::command]
pub async fn writing_csl_render_document(
    clusters: Vec<Vec<super::csl::render::ClusterItem>>,
    style: super::csl::render::StyleSource,
) -> Result<Vec<super::csl::render::RenderedCluster>, super::csl::render::CslError> {
    tokio::task::spawn_blocking(move || super::csl::render::render_document(&clusters, &style))
        .await
        .map_err(|e| super::csl::render::CslError {
            code: "task_failed".into(),
            message: format!("writing_csl_render_document: {e}"),
        })?
}

/// The bibliography of a manuscript: a derived view of what it cites (§11.6).
#[tauri::command]
pub async fn writing_csl_bibliography(
    cited: Vec<String>,
    style: super::csl::render::StyleSource,
) -> Result<Vec<String>, super::csl::render::CslError> {
    tokio::task::spawn_blocking(move || super::csl::render::render_bibliography(&cited, &style))
        .await
        .map_err(|e| super::csl::render::CslError {
            code: "task_failed".into(),
            message: format!("writing_csl_bibliography: {e}"),
        })?
}

/// Checks a `.csl` file before it is ever chosen (§11.6).
#[tauri::command]
pub async fn writing_csl_validate_style(
    xml: String,
) -> Result<super::csl::render::StyleInfo, super::csl::render::CslError> {
    tokio::task::spawn_blocking(move || super::csl::render::validate_style(&xml))
        .await
        .map_err(|e| super::csl::render::CslError {
            code: "task_failed".into(),
            message: format!("writing_csl_validate_style: {e}"),
        })?
}

/// What the agent can actually do here (§14.1, gap G9).
///
/// Published before any action is offered, and an action that cannot run is
/// listed as such rather than hidden. Offering one and failing at the moment of
/// use is the worst outcome: the writer has already chosen a passage and formed
/// an intention.
#[tauri::command]
pub async fn writing_agent_actions(
    has_chat: bool,
    has_retrieval: bool,
) -> WritingResult<Vec<super::agent_actions::AgentAction>> {
    Ok(super::agent_actions::matrix(has_chat, has_retrieval))
}

/// Records a proposal. Nothing is written into the manuscript (§14.2).
#[tauri::command]
pub async fn writing_agent_record_suggestion(
    db: State<'_, AppDbState>,
    input: super::agent::NewSuggestion,
) -> WritingResult<super::agent::SuggestionRow> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || super::agent::record(&open(&db_path)?, input))
        .await
        .map_err(|e| joined("writing_agent_record_suggestion", e))?
}

/// What the frontend asks the agent for (§14.3).
///
/// The passage and its context arrive already assembled and already shown to
/// the writer — `agent-context.ts` builds one object that the preview renders,
/// the request carries and the record reports, so the three cannot disagree.
/// This command adds nothing to it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct AgentAsk {
    pub id: String,
    pub document_id: String,
    pub action_type: String,
    /// The passage itself. What the agent is asked about, and nothing more.
    pub selection: String,
    pub selection_anchor_json: Option<String>,
    pub source_revision: i64,
    /// Hashed by the frontend, which also computes the hash it will be compared
    /// against when the proposal is resolved. One hashing authority, so the
    /// guard in [`super::agent::resolve`] compares like with like.
    pub selected_content_hash: String,
    pub context: Vec<super::agent_prompt::ContextPiece>,
    pub evidence_json: Option<String>,
}

/// No OpenRouter credential is configured. Nameable, because the fix is a
/// setting and the panel can say which one.
pub const NO_CREDENTIAL: &str = "agent_no_credential";

/// Asks the model, and records what it answered as a pending proposal (§14).
///
/// # Why this ends in a record and not in the manuscript
///
/// §14.2's first rule is that the agent never silently changes the text, so
/// there is deliberately no path from here into the document. The answer
/// becomes a row with status `pending`, which is what the Agente tab shows and
/// what [`writing_agent_resolve`] later applies exactly once.
///
/// # Why the round trip is not streamed
///
/// A proposal is judged whole — against what it would replace and what it rests
/// on. Streaming it would put half a suggestion on screen, which invites
/// accepting something before having read it.
#[tauri::command]
pub async fn writing_agent_ask(
    db: State<'_, AppDbState>,
    input: AgentAsk,
) -> WritingResult<super::agent::SuggestionRow> {
    use entropia_agent::cliente_llm::{ClienteLlm, ClienteLlmOpenRouter, TurnoAgente};

    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = open(&db_path)?;

        let key = crate::settings::get_setting(&conn, crate::settings::OPENROUTER_API_KEY)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                WritingError::new(
                    NO_CREDENTIAL,
                    "Configura la credencial OpenRouter en Configuración",
                )
            })?;
        let model = super::agent_prompt::pick_model(
            crate::settings::get_setting(&conn, "rag_model"),
            crate::settings::get_setting(&conn, "openrouter_model"),
            ClienteLlmOpenRouter::MODELO_DEFAULT,
        );
        let llm = ClienteLlmOpenRouter::new(key, model);

        let messages =
            super::agent_prompt::messages(&input.action_type, &input.selection, &input.context);

        // No tools. A writing action is one question about one passage, and a
        // tool loop here would let the agent reach for material the writer
        // never saw in the preview — which is exactly what §14.4 rules out.
        let turn = llm
            .turno_agente(&messages, &[])
            .map_err(|e| WritingError::new("agent_unavailable", e))?;

        let answer = match turn {
            TurnoAgente::Texto(text) => text,
            // Unreachable while no tools are offered, and reported rather than
            // unwrapped so that it stays unreachable by evidence, not by hope.
            TurnoAgente::Herramientas(_) => {
                return Err(WritingError::new(
                    "agent_unexpected_tools",
                    "el modelo pidió herramientas que esta acción no ofrece",
                ))
            }
        };

        let proposal = super::agent_prompt::parse(&answer);
        if proposal.suggested_text.trim().is_empty() {
            // An empty proposal would sit in the pending list looking like a
            // suggestion and replace the passage with nothing if accepted.
            return Err(WritingError::new(
                "agent_empty_answer",
                "el modelo no devolvió ninguna propuesta",
            ));
        }

        super::agent::record(
            &conn,
            super::agent::NewSuggestion {
                id: input.id,
                document_id: input.document_id,
                selection_anchor_json: input.selection_anchor_json,
                source_revision: input.source_revision,
                selected_content_hash: input.selected_content_hash,
                action_type: input.action_type,
                // Kept so the panel can show the proposal beside what it would
                // replace: §14.2 asks for the comparison, not just the result.
                original_text: Some(input.selection),
                suggested_text: Some(proposal.suggested_text),
                rationale: proposal.rationale,
                evidence_json: input.evidence_json,
                provider: Some("openrouter".into()),
                // Asked of the client rather than of the settings, so the record
                // names the model that actually answered.
                model: Some(llm.modelo().to_string()),
            },
        )
    })
    .await
    .map_err(|e| joined("writing_agent_ask", e))?
}

/// Corpus passages relevant to a passage of the manuscript (§14.1, §14.3).
///
/// # Why this exists beside `rag_ask`
///
/// `rag_ask` retrieves, reranks, builds a prompt and generates a reply. The
/// four evidence actions of §14.1 want none of that: they want the passages, so
/// that a proposal rests on something the writer can open and check. Asking
/// `rag_ask` and discarding its answer would pay for a generation nobody reads.
///
/// So the retrieval is the one that already exists — the same hybrid legs, the
/// same parameters from the same settings — stopped before the part that costs
/// a model call. The rerank is skipped deliberately: it is a provider call per
/// retrieval, and these are pressed while writing.
///
/// # Why a failed vector leg is not a failure
///
/// The embedding engine may be unavailable, and `rag_ask` already treats that
/// as "use the lexical leg only" rather than as an error. Evidence found by
/// words alone is still evidence, and a writer mid-sentence is better served by
/// fewer passages than by a dialog.
#[tauri::command]
pub async fn writing_corpus_retrieve(
    db: State<'_, AppDbState>,
    passage: String,
    limit: usize,
) -> WritingResult<Vec<super::retrieval::RetrievedPassage>> {
    let db_path = db.db_path.clone();
    let worker = db.worker_conn.clone();
    let wanted = super::retrieval::bounded(limit);

    tokio::task::spawn_blocking(move || {
        let query = super::retrieval::require_query(&passage)?.to_string();

        let (params, unit_setting) = {
            let conn = open(&db_path)?;
            (
                crate::rag::params::rag_params_from_settings(&conn),
                crate::settings::get_setting(&conn, "rag_retrieval_unit"),
            )
        };
        let unit = crate::rag::retrieval::RetrievalUnit::from_setting(unit_setting.as_deref())
            .map_err(|e| WritingError::new("retrieval_unit_invalid", e))?;

        let embedding = crate::rag::commands::embed_query_for_writing(&db_path, &query);
        let queries = vec![crate::rag::retrieval::RetrievalQuery {
            text: &query,
            embedding: embedding.as_deref(),
        }];

        let conn = worker.lock().map_err(|_| {
            WritingError::new("db_unavailable", "la conexión de trabajo está tomada")
        })?;
        let candidates =
            crate::rag::retrieval::hybrid_retrieve_candidates(&conn, &queries, &params, unit)
                .map_err(|e| WritingError::new("retrieval_failed", e))?;

        Ok(super::retrieval::passages(
            candidates,
            wanted,
            params.snippet_max_chars,
        ))
    })
    .await
    .map_err(|e| joined("writing_corpus_retrieve", e))?
}

/// The proposals still waiting on a document.
#[tauri::command]
pub async fn writing_agent_pending(
    db: State<'_, AppDbState>,
    document_id: String,
) -> WritingResult<Vec<super::agent::SuggestionRow>> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || super::agent::pending(&open(&db_path)?, &document_id))
        .await
        .map_err(|e| joined("writing_agent_pending", e))?
}

/// Resolves a proposal and says whether its text should now be applied (§14.2).
///
/// The status change *is* the decision, so the answer and the record are one
/// write. A second acceptance of the same suggestion changes no row and is told
/// to apply nothing, which is what keeps a double click from inserting a
/// paragraph twice.
#[tauri::command]
pub async fn writing_agent_resolve(
    db: State<'_, AppDbState>,
    id: String,
    status: String,
    current_content_hash: Option<String>,
) -> WritingResult<super::agent::Resolution> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        super::agent::resolve(
            &open(&db_path)?,
            &id,
            &status,
            current_content_hash.as_deref(),
        )
    })
    .await
    .map_err(|e| joined("writing_agent_resolve", e))?
}

/// Which manuscripts cite an asset, so a deletion can announce what it costs
/// (§10.3).
///
/// A warning, never a veto: §29.1 settled the policy as deletion with
/// confirmation and a preserved snapshot. The citation keeps its quoted text
/// and metadata and goes on existing, which is why `asset_id` is a snapshot
/// column with no foreign key behind it.
#[tauri::command]
pub async fn writing_citations_for_asset(
    db: State<'_, AppDbState>,
    asset_id: String,
) -> WritingResult<Vec<repository::AssetDependency>> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        repository::citations_for_asset(&open(&db_path)?, &asset_id)
    })
    .await
    .map_err(|e| joined("writing_citations_for_asset", e))?
}

#[tauri::command]
pub async fn writing_duplicate_document(
    db: State<'_, AppDbState>,
    source_id: String,
    new_id: String,
    new_title: String,
) -> WritingResult<DocumentRow> {
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = open(&db_path)?;
        repository::duplicate_document(&mut conn, &source_id, &new_id, &new_title)
    })
    .await
    .map_err(|e| joined("writing_duplicate_document", e))?
}
