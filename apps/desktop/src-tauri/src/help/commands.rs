//! Tauri commands of the in-app help assistant (T3): status, diagnostic
//! preview and the OpenRouter-backed answer (S1–S3, S5). The prompt itself
//! stays pure in `prompt.rs`; nothing here is stored (D4).

use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use super::diagnostics::{
    build_report, scrub_report, DiagnosticInputs, KEY_SETTINGS, SETTINGS_ALLOWLIST,
};
use super::documents::{DOCUMENTS_MAX_CHARS, DOCUMENTS_MAX_FRAGMENTS};
use super::faq;
use super::index::{HelpIndex, Hit};
use super::prompt::{
    build_help_prompt, normalize_history_capped, pick_sections_with_budget, validate_question,
    HelpAnswer, HelpDocument, HelpSource, HelpStatus, HelpTurn,
};

/// The help index, opened lazily once per process (D6): the first question
/// after start hashes the bundled manual and rebuilds
/// `<cache_dir>/help-index.sqlite` when it changed, later questions reuse it.
/// A plain mutex is enough — `search` is a millisecond local SELECT.
static HELP_INDEX: OnceLock<Mutex<HelpIndex>> = OnceLock::new();

fn help_index(app: &tauri::AppHandle) -> Result<std::sync::MutexGuard<'static, HelpIndex>, String> {
    // Built at most twice under a race (both open the same file, the loser
    // is dropped); opening is idempotent, so no fallible once-cell API is
    // needed.
    if HELP_INDEX.get().is_none() {
        let dir = crate::path_utils::cache_dir(app)?;
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("No se pudo crear la caché de ayuda: {error}"))?;
        let fresh = HelpIndex::open(&dir.join("help-index.sqlite"), &HelpIndex::bundled_docs())
            .map(Mutex::new)?;
        let _ = HELP_INDEX.set(fresh);
    }
    HELP_INDEX
        .get()
        .ok_or_else(|| "help index unavailable".to_string())?
        .lock()
        .map_err(|error| format!("help index lock poisoned: {error}"))
}

/// Whether the assistant can run on the user's own OpenRouter key (S3, S6).
/// The raw row only: resolving the keyring value here would be pointless, the
/// key itself never leaves through this command.
#[tauri::command]
pub async fn help_status(
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<HelpStatus, String> {
    let conn = db.ui_conn.clone();
    tokio::task::spawn_blocking(move || {
        let guard = conn.lock().map_err(|e| e.to_string())?;
        let present = crate::settings::get_raw_setting(&guard, "openrouter_api_key")
            .is_some_and(|value| !value.trim().is_empty());
        Ok(HelpStatus {
            has_openrouter_key: present,
        })
    })
    .await
    .map_err(|e| format!("help status task panicked: {e}"))?
}

/// The exact diagnostic text `help_ask` would send (D3, S5): the user previews
/// this and opts in per question. Settings come from the allowlist only, keys
/// as configured-or-not, logs scrubbed line by line.
#[tauri::command]
pub async fn help_diagnostics_preview(
    db: tauri::State<'_, crate::db::state::AppDbState>,
    logs: tauri::State<'_, crate::app_logs::AppLogsState>,
) -> Result<String, String> {
    let entries = logs.recent_entries(2000);
    let conn = db.ui_conn.clone();
    tokio::task::spawn_blocking(move || {
        let guard = conn.lock().map_err(|e| e.to_string())?;
        let keys = SETTINGS_ALLOWLIST
            .iter()
            .copied()
            .chain(KEY_SETTINGS.iter().map(|entry| entry.0));
        let mut settings = Vec::new();
        for key in keys {
            if let Some(value) = crate::settings::get_raw_setting(&guard, key) {
                settings.push((key.to_string(), value));
            }
        }
        Ok(build_report(&DiagnosticInputs {
            product: if cfg!(feature = "local-ml") {
                "EntropIA Pro"
            } else {
                "EntropIA Lite"
            }
            .to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            settings,
            logs: entries,
        }))
    })
    .await
    .map_err(|e| format!("help diagnostics task panicked: {e}"))?
}

/// Answers a help question from the manual excerpts (and the opted-in
/// diagnostic report) on the user's own OpenRouter key (S1–S3, S5). The
/// conversation lives in memory only; nothing is persisted (D4).
///
/// Before the key gate, a question that matches a bundled FAQ entry (S10)
/// is answered locally with that entry's text, verbatim: no key needed, no
/// network touched — unless `help_local_answers` is off, which sends every
/// question to the model. Questions with diagnostics attached always go to
/// the model. `documents` (S12) opts into local archive excerpts for this
/// question only, camelCase on the wire, absent means off: the Chat's lexical
/// retrieval runs locally (no embeddings) and its fragments join the prompt,
/// and a question with documents on never gets a local FAQ answer.
#[tauri::command]
pub async fn help_ask(
    app: tauri::AppHandle,
    question: String,
    history: Option<Vec<HelpTurn>>,
    diagnostics: Option<String>,
    documents: Option<bool>,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<HelpAnswer, String> {
    let question = validate_question(&question)?;
    // S12: opt-in per question, off when absent (S5 stands unchanged).
    let with_documents = documents.unwrap_or(false);
    // What the user previewed is what leaves, re-scrubbed on the way out in
    // case it was edited in between (D3). Anything else diagnostic stays home.
    let diagnostics = match diagnostics {
        Some(text) if !text.trim().is_empty() => Some(scrub_report(&text)),
        _ => None,
    };
    let with_diagnostics = diagnostics.is_some();
    // One settings read (S14, D10): the model override with its fallback
    // chain, the clamped budgets and the local-answers switch. The key gate
    // stays below the local answer, so it is attempted here but only
    // unwrapped once the question is known to need the model.
    let conn = db.ui_conn.clone();
    let (key_result, help) = tokio::task::spawn_blocking(move || {
        let guard = conn.lock().map_err(|e| e.to_string())?;
        let help = super::settings::resolve_help_settings(&|key| {
            crate::settings::get_setting(&guard, key)
        });
        // The frontend shows its fixed no-key page on exactly this string (S6).
        let api_key = crate::llm::require_openrouter_key(&guard)
            .map_err(|_| "help_no_openrouter_key".to_string());
        Ok::<_, String>((api_key, help))
    })
    .await
    .map_err(|e| format!("help settings task panicked: {e}"))??;
    // S10: deterministic local answer first, before the key gate and without
    // any network call. It carries the entry's Fuente as its single source.
    // With documents on there is no local answer: the question needs the
    // archive (S12).
    if let Some(local) =
        super::settings::local_answer_for(&help, diagnostics.as_deref(), with_documents, &question)
    {
        return Ok(HelpAnswer {
            answer: local.answer,
            sources: vec![HelpSource {
                chapter: local.chapter,
                title: local.title,
            }],
            local: true,
            documents: Vec::new(),
        });
    }
    let api_key = key_result?;
    let history = normalize_history_capped(history, help.history_turns)?;
    // S12: the opted-in excerpts, retrieved locally on the worker connection
    // like rag_ask. The key gate above already ran, so documents on with no
    // key still reports help_no_openrouter_key without touching the archive.
    let fragments = if with_documents {
        let worker = db.worker_conn.clone();
        let owned = question.clone();
        tokio::task::spawn_blocking(move || {
            let guard = worker.lock().map_err(|e| e.to_string())?;
            super::documents::document_fragments(
                &guard,
                &owned,
                DOCUMENTS_MAX_FRAGMENTS,
                DOCUMENTS_MAX_CHARS,
            )
        })
        .await
        .map_err(|e| format!("help documents task panicked: {e}"))??
    } else {
        Vec::new()
    };
    let picked: Vec<Hit> = {
        // The guard drops before the network call below: the index is only
        // borrowed to rank, never across an await.
        let index = help_index(&app)?;
        pick_sections_with_budget(&index, &question, with_diagnostics, help.context_chars)
    };
    let prompt = build_help_prompt(
        &question,
        &history,
        &picked,
        diagnostics.as_deref(),
        &fragments,
    );
    let sources = picked
        .iter()
        .map(|section| HelpSource {
            chapter: section.chapter.clone(),
            title: section.title.clone(),
        })
        .collect();
    let params = crate::llm::generation::OpenRouterGenerationParams::provider_defaults(
        help.max_tokens,
        help.temperature,
    );
    let answer = crate::llm::openrouter::OpenRouterClient::new(api_key, help.model)
        .generate(&prompt, &params)
        .await?;
    Ok(HelpAnswer {
        answer,
        sources,
        local: false,
        documents: fragments
            .iter()
            .map(|fragment| HelpDocument {
                title: fragment.title.clone(),
            })
            .collect(),
    })
}

/// One starter button of the help chat (S10): the group it belongs to and
/// the question it asks. The frontend shows the «Primeros pasos» group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HelpFaqItem {
    pub group: String,
    pub question: String,
}

/// Every bundled FAQ entry as a starter button (S10): answered locally, so
/// the list needs no key and no network.
#[tauri::command]
pub async fn help_faq_list() -> Result<Vec<HelpFaqItem>, String> {
    Ok(faq::entries()
        .iter()
        .map(|entry| HelpFaqItem {
            group: entry.group.clone(),
            question: entry.question.clone(),
        })
        .collect())
}
