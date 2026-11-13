//! Tauri commands of the in-app help assistant (T3): status, diagnostic
//! preview and the OpenRouter-backed answer (S1–S3, S5). The prompt itself
//! stays pure in `prompt.rs`; nothing here is stored (D4).

use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use super::diagnostics::{
    build_report, scrub_report, DiagnosticInputs, KEY_SETTINGS, SETTINGS_ALLOWLIST,
};
use super::faq;
use super::index::{HelpIndex, Hit};
use super::prompt::{
    build_help_prompt, normalize_history, pick_sections, validate_question, HelpAnswer, HelpSource,
    HelpStatus, HelpTurn, ANSWER_MAX_TOKENS, ANSWER_TEMPERATURE,
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
/// network touched. Questions with diagnostics attached always go to the
/// model.
#[tauri::command]
pub async fn help_ask(
    app: tauri::AppHandle,
    question: String,
    history: Option<Vec<HelpTurn>>,
    diagnostics: Option<String>,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<HelpAnswer, String> {
    let question = validate_question(&question)?;
    let history = normalize_history(history)?;
    // What the user previewed is what leaves, re-scrubbed on the way out in
    // case it was edited in between (D3). Anything else diagnostic stays home.
    let diagnostics = match diagnostics {
        Some(text) if !text.trim().is_empty() => Some(scrub_report(&text)),
        _ => None,
    };
    let with_diagnostics = diagnostics.is_some();
    // S10: deterministic local answer first, before the key gate and without
    // any network call. It carries the entry's Fuente as its single source.
    if let Some(local) = faq::answer_locally(diagnostics.as_deref(), &question) {
        return Ok(HelpAnswer {
            answer: local.answer,
            sources: vec![HelpSource {
                chapter: local.chapter,
                title: local.title,
            }],
            local: true,
        });
    }
    let conn = db.ui_conn.clone();
    let (api_key, model) = tokio::task::spawn_blocking(move || {
        let guard = conn.lock().map_err(|e| e.to_string())?;
        // The frontend shows its fixed no-key page on exactly this string (S6).
        let api_key = crate::llm::require_openrouter_key(&guard)
            .map_err(|_| "help_no_openrouter_key".to_string())?;
        let model = crate::settings::get_setting(&guard, "openrouter_model")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| crate::settings::DEFAULT_OPENROUTER_MODEL.to_string());
        Ok::<_, String>((api_key, model))
    })
    .await
    .map_err(|e| format!("help settings task panicked: {e}"))??;
    let picked: Vec<Hit> = {
        // The guard drops before the network call below: the index is only
        // borrowed to rank, never across an await.
        let index = help_index(&app)?;
        pick_sections(&index, &question, with_diagnostics)
    };
    let prompt = build_help_prompt(&question, &history, &picked, diagnostics.as_deref());
    let sources = picked
        .iter()
        .map(|section| HelpSource {
            chapter: section.chapter.clone(),
            title: section.title.clone(),
        })
        .collect();
    let params = crate::llm::generation::OpenRouterGenerationParams::provider_defaults(
        ANSWER_MAX_TOKENS,
        ANSWER_TEMPERATURE,
    );
    let answer = crate::llm::openrouter::OpenRouterClient::new(api_key, model)
        .generate(&prompt, &params)
        .await?;
    Ok(HelpAnswer {
        answer,
        sources,
        local: false,
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
