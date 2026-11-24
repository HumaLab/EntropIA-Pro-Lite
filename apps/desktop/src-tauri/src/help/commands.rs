//! Tauri commands of the in-app help assistant (T3): status, diagnostic
//! preview and the OpenRouter-backed answer (S1–S3, S5), since T22 through
//! the help agent loop (S15): the model drives the in-process agent MCP
//! behind the help gate (D11, D12). The prompt itself stays pure in
//! `prompt.rs`; nothing here is stored (D4).

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio::sync::oneshot;

use super::agent_llm::{
    run_agent, AgentRequest, AgentStep, ChatTransport, HelpToolExecutor, OpenRouterChatTransport,
};
use super::diagnostics::{
    build_report, scrub_report, DiagnosticInputs, KEY_SETTINGS, SETTINGS_ALLOWLIST,
};
use super::faq;
use super::gate::{ApprovalAnswer, HelpApprover, HelpGateExecutor};
use super::index::{HelpIndex, Hit};
use super::manual_tool::ManualSearch;
use super::mcp_bridge::HelpMcpBridge;
use super::prompt::{
    agent_step_status_wire, build_agent_system, build_help_prompt, normalize_history_capped,
    pick_sections_with_budget, validate_question, HelpAnswer, HelpDocument, HelpSource, HelpStatus,
    HelpStep, HelpTurn,
};
use crate::automation::access::AgentAccess;
use crate::automation::server::AgentEndpointState;

/// The event the frontend listens to for live agent steps (T24): the
/// payload carries `{ requestId, tool, args, status }`, one per executed
/// tool call as it happens. The final list in `HelpAnswer.steps` stays the
/// source of truth at the end.
pub(crate) const HELP_STEP_EVENT: &str = "help://step";

/// `help_ask` fails with exactly this when `llm_mode = local` but the local
/// Gemma file is not on disk (R22): the frontend points to Modelos locales
/// and offers OpenRouter for the question.
pub(crate) const HELP_LOCAL_MODEL_MISSING: &str = "help_local_model_missing";

/// `help_ask` fails with exactly this when a local-Gemma question asks for
/// documents (R22): only the OpenRouter agent can read them.
pub(crate) const HELP_LOCAL_NO_DOCUMENTS: &str = "help_local_no_documents";

/// Context ceiling of a local help answer: the prompt is the question, the
/// manual excerpts (`help_context_chars`), the capped history and the
/// diagnostics; the engine fits the window down to the actual prompt.
#[cfg(feature = "local-ml")]
const HELP_LOCAL_N_CTX: u32 = 16_384;

/// The picked excerpts as answer sources, in picked order.
fn sources_of(picked: &[Hit]) -> Vec<HelpSource> {
    picked
        .iter()
        .map(|section| HelpSource {
            chapter: section.chapter.clone(),
            title: section.title.clone(),
            kind: section.source.wire_kind().to_string(),
        })
        .collect()
}

/// A plain answer from the local Gemma (R22). Checks the file first, so a
/// missing model fails at once with [`HELP_LOCAL_MODEL_MISSING`] instead of
/// starting a multi-GB download inside the question.
#[cfg(feature = "local-ml")]
async fn local_help_answer(
    app: &tauri::AppHandle,
    db_path: &std::path::Path,
    prompt: String,
    max_tokens: i32,
    temperature: f32,
) -> Result<String, String> {
    let app = app.clone();
    let db_path = db_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let conn = crate::db::open::open_archive_connection(&db_path)?;
        if !crate::llm::local_gemma_model_present(&conn, &db_path) {
            return Err(HELP_LOCAL_MODEL_MISSING.to_string());
        }
        crate::llm::generate_local_chat(
            &conn,
            &db_path,
            &app,
            &prompt,
            max_tokens,
            HELP_LOCAL_N_CTX,
            temperature,
            "[help][local]",
        )
    })
    .await
    .map_err(|error| format!("local help task panicked: {error}"))?
}

/// Lite has no local engine: `HelpSettings::local_engine` is always off.
#[cfg(not(feature = "local-ml"))]
async fn local_help_answer(
    _app: &tauri::AppHandle,
    _db_path: &std::path::Path,
    _prompt: String,
    _max_tokens: i32,
    _temperature: f32,
) -> Result<String, String> {
    Err(HELP_LOCAL_MODEL_MISSING.to_string())
}

/// One live step emitted over [`HELP_STEP_EVENT`] (T24, T26): which run
/// it belongs to, the tool, the compact JSON args and the outcome on the
/// wire as `"ok"`, `"pending"`, `"denied"`, `"failed"` or
/// `"expired"` — the same shape as `HelpStep` plus the run id. A
/// `pending` step waits for the owner in the MCP dialog; `target` names
/// its object when known (never a UUID) and `approvalId` follows the
/// outcome. Both default off so older clients keep decoding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HelpStepEvent {
    pub request_id: String,
    pub tool: String,
    pub args: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
}

/// One stop flag per running agent turn (T24 «Detener»), keyed by the
/// frontend-generated `requestId`. `help_ask` inserts the flag when the run
/// starts and removes it when the run finishes; `help_stop` flips it.
/// Managed once in `lib.rs`.
#[derive(Clone, Default)]
pub(crate) struct HelpAgentRuns {
    flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl HelpAgentRuns {
    /// Registers a run and answers its stop flag. A repeated id for a run
    /// that is still registered reuses the same flag.
    pub(crate) fn start(&self, request_id: &str) -> Arc<AtomicBool> {
        self.flags
            .lock()
            .map(|mut flags| {
                flags
                    .entry(request_id.to_string())
                    .or_insert_with(|| Arc::new(AtomicBool::new(false)))
                    .clone()
            })
            .unwrap_or_else(|_| Arc::new(AtomicBool::new(false)))
    }

    /// Flips the stop flag of a running turn. Unknown (finished or never
    /// started) ids stop nothing.
    pub(crate) fn stop(&self, request_id: &str) -> bool {
        self.flags
            .lock()
            .map(|flags| {
                flags
                    .get(request_id)
                    .map(|flag| flag.store(true, Ordering::Relaxed))
                    .is_some()
            })
            .unwrap_or(false)
    }

    /// Forgets a finished run. Late `help_stop` calls for it stop nothing.
    pub(crate) fn finish(&self, request_id: &str) {
        if let Ok(mut flags) = self.flags.lock() {
            flags.remove(request_id);
        }
    }
}

/// Whether an agent-loop error is the owner pressing «Detener» (T24): the
/// run answers with [`stopped_answer`], never with a generic failure.
pub(crate) fn is_stop_error(message: &str) -> bool {
    message == "detenido"
}

/// The answer of a stopped run (T24): a clear message plus the steps done
/// so far. The round cap did not force anything: the owner did.
pub(crate) fn stopped_answer(steps: Vec<HelpStep>) -> AgentOutcome {
    AgentOutcome {
        answer: "Detenido.".to_string(),
        steps,
        round_limit_reached: false,
    }
}

/// How long an inline approval waits for the owner (D12): 10 minutes,
/// then the call is denied.
const HELP_APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);

/// The event the frontend listens to for inline approvals (D12): the
/// payload carries `{ id, tool, summary }`, answered with
/// `help_approval_answer(id, approved)`.
pub(crate) const HELP_APPROVAL_EVENT: &str = "help://approval";

/// Pending inline approvals (D12): one oneshot sender per card, keyed by id.
/// Managed once in `lib.rs`; `help_approval_answer` takes the answer.
#[derive(Clone, Default)]
pub(crate) struct HelpApprovalPending {
    senders: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
}

/// Answers one inline approval card (D12): `id` is the `help://approval`
/// payload's, `approved` the owner's choice. Unknown ids mean the card
/// already timed out and were answered: an error either way.
#[tauri::command]
pub async fn help_approval_answer(
    state: tauri::State<'_, HelpApprovalPending>,
    id: String,
    approved: bool,
) -> Result<(), String> {
    let sender = state
        .senders
        .lock()
        .map_err(|error| error.to_string())?
        .remove(&id);
    match sender {
        Some(answer) => answer
            .send(approved)
            .map_err(|_| "El pedido ya venció o fue respondido.".to_string()),
        None => Err("El pedido ya venció o fue respondido.".to_string()),
    }
}

/// The approval counter for inline card ids (`help-apr-<n>`).
static HELP_APPROVAL_SEQ: AtomicU64 = AtomicU64::new(1);

/// Asks the owner about a local write over the `help://approval` event and
/// waits for `help_approval_answer`, at most [`HELP_APPROVAL_TIMEOUT`]
/// (D12, S15). No Tauri type crosses `gate.rs`: this is the production
/// [`HelpApprover`]. When the owner presses «Detener» (T24) while the card
/// waits, the card resolves as rejected so the run stops quickly instead of
/// hanging on the 10-minute timeout.
struct TauriHelpApprover {
    app: tauri::AppHandle,
    pending: HelpApprovalPending,
    stop: Arc<AtomicBool>,
}

impl HelpApprover for TauriHelpApprover {
    async fn request_approval(
        &self,
        tool: &str,
        _args: serde_json::Value,
        summary: String,
    ) -> ApprovalAnswer {
        let id = format!(
            "help-apr-{}",
            HELP_APPROVAL_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let (tx, rx) = oneshot::channel();
        match self.pending.senders.lock() {
            Ok(mut senders) => {
                senders.insert(id.clone(), tx);
            }
            Err(_) => return ApprovalAnswer::TimedOut,
        }
        let _ = self.app.emit(
            HELP_APPROVAL_EVENT,
            serde_json::json!({ "id": id, "tool": tool, "summary": summary }),
        );
        // The stop branch watches the «Detener» flag: when the owner stops
        // the run mid-approval, the card counts as rejected and the agent
        // loop sees the denial, then its own stop check ends the run.
        let answer = tokio::select! {
            biased;
            message = rx => match message {
                Ok(true) => ApprovalAnswer::Approved,
                Ok(false) => ApprovalAnswer::Rejected,
                Err(_) => ApprovalAnswer::TimedOut,
            },
            _ = async {
                while !self.stop.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            } => ApprovalAnswer::Rejected,
            _ = tokio::time::sleep(HELP_APPROVAL_TIMEOUT) => ApprovalAnswer::TimedOut,
        };
        if let Ok(mut senders) = self.pending.senders.lock() {
            senders.remove(&id);
        }
        answer
    }
}

/// The agent-level inputs `help_ask` assembled: everything [`run_help_agent`]
/// needs besides the transport and the executor.
pub(crate) struct AgentInputs {
    pub system: String,
    pub history: Vec<HelpTurn>,
    pub question: String,
    pub model: String,
    pub max_tokens: i32,
    pub temperature: f32,
    pub max_rounds: usize,
}

/// The agent turn mapped to answer parts: the final text, every tool step
/// and whether the round cap forced the closing call.
#[derive(Debug)]
pub(crate) struct AgentOutcome {
    pub answer: String,
    pub steps: Vec<HelpStep>,
    pub round_limit_reached: bool,
}

/// Runs the bounded agent loop and maps the result to answer parts. Pure
/// over the transport and the executor, so the whole decision flow — model
/// calls a read tool, an action asks inline and is forwarded or denied,
/// content tools gated by the box — is unit-testable with mocks. `stop`
/// ends the run when the owner presses «Detener» (T24); `on_step` sees
/// every executed step as it happens, so `help_ask` can emit it live.
pub(crate) async fn run_help_agent<T, E>(
    transport: &T,
    executor: &E,
    inputs: AgentInputs,
    stop: &AtomicBool,
    on_step: impl FnMut(AgentStep),
) -> Result<AgentOutcome, String>
where
    T: ChatTransport,
    E: HelpToolExecutor,
{
    let request = AgentRequest {
        model: inputs.model,
        system: inputs.system,
        history: inputs
            .history
            .iter()
            .map(|turn| (turn.role.clone(), turn.content.clone()))
            .collect(),
        question: inputs.question,
        max_rounds: inputs.max_rounds,
        max_tokens: inputs.max_tokens,
        temperature: inputs.temperature,
    };
    let result = run_agent(transport, executor, request, stop, on_step).await?;
    Ok(AgentOutcome {
        answer: result.answer,
        steps: result
            .steps
            .iter()
            .map(|step| HelpStep {
                tool: step.tool.clone(),
                args: step.args.clone(),
                status: agent_step_status_wire(&step.status).to_string(),
                target: step.target.clone(),
                approval_id: step.approval_id.clone(),
            })
            .collect(),
        round_limit_reached: result.round_limit_reached,
    })
}

/// Whether an agent-loop error means the model or provider rejected tool
/// use (an OpenRouter error saying the endpoint does not support tools or
/// tool choice): the question is retried once without tools, with
/// `toolsUnavailable` set. Anything else — key, credit, network — travels
/// to the user unchanged.
pub(crate) fn tools_unsupported_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("tool")
        && (lower.contains("support")
            || lower.contains("tool_choice")
            || lower.contains("function call"))
}

/// The help index, opened lazily once per process (D6): the first question
/// after start hashes the bundled manual and rebuilds
/// `<cache_dir>/help-index.sqlite` when it changed, later questions reuse it.
/// Shared as an `Arc`, so the agent run answers `manual_search` from the
/// same handle with only a per-call lock (S22, D19): no long-held guard
/// across awaits, no second open.
static HELP_INDEX: OnceLock<Arc<Mutex<HelpIndex>>> = OnceLock::new();

fn help_index_shared(app: &tauri::AppHandle) -> Result<Arc<Mutex<HelpIndex>>, String> {
    // Built at most twice under a race (both open the same file, the loser
    // is dropped); opening is idempotent, so no fallible once-cell API is
    // needed.
    if HELP_INDEX.get().is_none() {
        let dir = crate::path_utils::cache_dir(app)?;
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("No se pudo crear la caché de ayuda: {error}"))?;
        let fresh = HelpIndex::open(&dir.join("help-index.sqlite"), &HelpIndex::bundled_docs())
            .map(|index| Arc::new(Mutex::new(index)))?;
        let _ = HELP_INDEX.set(fresh);
    }
    HELP_INDEX
        .get()
        .cloned()
        .ok_or_else(|| "help index unavailable".to_string())
}

/// Whether the assistant can run on the user's own OpenRouter key (S3, S6),
/// on which model, and whether `llm_mode = local` sends it to the local
/// Gemma instead (R22). The raw rows only: resolving the keyring value here
/// would be pointless, the key itself never leaves through this command.
#[tauri::command]
pub async fn help_status(
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<HelpStatus, String> {
    let conn = db.ui_conn.clone();
    let db_path = db.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let guard = conn.lock().map_err(|e| e.to_string())?;
        let present =
            crate::settings::api_key_configured(&guard, crate::settings::OPENROUTER_API_KEY);
        let help = super::settings::resolve_help_settings(&|key| {
            crate::settings::get_setting(&guard, key)
        });
        Ok(HelpStatus {
            has_openrouter_key: present,
            model: help.model,
            local_engine: help.local_engine,
            local_model_ready: local_model_ready(&guard, &db_path),
        })
    })
    .await
    .map_err(|e| format!("help status task panicked: {e}"))?
}

/// Whether the local Gemma file is on disk (never downloads it). Lite has no
/// local engine.
fn local_model_ready(conn: &rusqlite::Connection, db_path: &std::path::Path) -> bool {
    #[cfg(feature = "local-ml")]
    {
        crate::llm::local_gemma_model_present(conn, db_path)
    }
    #[cfg(not(feature = "local-ml"))]
    {
        let _ = (conn, db_path);
        false
    }
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

/// Answers a help question through the help agent (T22, S15): the system
/// prompt carries the local date and time (and the opted-in diagnostic
/// report) on the user's own OpenRouter key (S1–S3, S5), and the model
/// drives the in-process agent MCP behind the help gate (D11, D12),
/// consulting the manual on demand through `manual_search` (S22, D19). The
/// question and the final answer are stored in the `conversationId`
/// conversation (S19, D17) when the frontend sends one, creating it when
/// missing.
///
/// Before the key gate, greetings, thanks and date/time questions (S22)
/// are answered locally and deterministically with no key, no network and
/// no sources — and a question that matches a bundled FAQ entry (S10) is
/// answered locally with that entry's text, verbatim — unless
/// `help_local_answers` is off, which sends every question to the model.
/// Questions with diagnostics attached or documents on always go to the
/// model. `documents` (S12) opts into the CONTENT tools for this question
/// only, camelCase on the wire, absent means off: document text reaches the
/// model only through those tools (D16), and a question with documents on
/// never gets a local FAQ answer. `requestId` (T24) is the frontend-generated
/// id of this question: while the agent runs, every executed tool call is
/// emitted live as a `help://step` `{ requestId, tool, args, status }`
/// event, and `help_stop` ends the run through it. Absent means no live
/// events and no stopping (older clients keep working). `conversationId`
/// (S19) is the stored conversation the question and the final answer are
/// appended to, creating it when missing; the id is echoed back in
/// `HelpAnswer.conversationId`. Absent means nothing is stored.
///
/// With `llm_mode = local` in Pro (R22) the question is answered by the local
/// Gemma from the manual excerpts (and the diagnostics, which then never leave
/// the computer): no key, no network, no actions and no documents, since the
/// local engine has no tool use. `remote` sends this one question to
/// OpenRouter anyway. A missing Gemma file fails with
/// [`HELP_LOCAL_MODEL_MISSING`] instead of downloading it inside the question.
// A Tauri command: one State per shared service it touches.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn help_ask(
    app: tauri::AppHandle,
    question: String,
    history: Option<Vec<HelpTurn>>,
    diagnostics: Option<String>,
    documents: Option<bool>,
    request_id: Option<String>,
    conversation_id: Option<String>,
    remote: Option<bool>,
    db: tauri::State<'_, crate::db::state::AppDbState>,
    endpoint: tauri::State<'_, Arc<AgentEndpointState>>,
    access: tauri::State<'_, Arc<AgentAccess>>,
    pending: tauri::State<'_, HelpApprovalPending>,
    runs: tauri::State<'_, HelpAgentRuns>,
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
    // S19: the stored conversation the question and the final answer are
    // appended to. Blank travels as absent (older clients send nothing).
    let conversation_id = conversation_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());
    let with_diagnostics = diagnostics.is_some();
    // S22/D19: greetings, thanks and date/time questions are answered
    // locally and deterministically — no API call, no key, no sources —
    // before anything else. Never with diagnostics or documents on: those
    // need the model and the archive, so they go to the normal path.
    let now = super::clock::now_local();
    if !with_diagnostics && !with_documents {
        if let Some(answer) = super::smalltalk::answer_small_talk(&question, &now) {
            return Ok(super::history::attach_history(
                &app,
                conversation_id.clone(),
                &question,
                HelpAnswer {
                    answer,
                    sources: Vec::new(),
                    local: true,
                    documents: Vec::new(),
                    steps: Vec::new(),
                    round_limit_reached: false,
                    tools_unavailable: false,
                    local_model: false,
                    conversation_id: None,
                },
            ));
        }
    }
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
        return Ok(super::history::attach_history(
            &app,
            conversation_id.clone(),
            &question,
            HelpAnswer {
                answer: local.answer,
                sources: vec![HelpSource {
                    chapter: local.chapter,
                    title: local.title,
                    kind: super::index::Source::Faq.wire_kind().to_string(),
                }],
                local: true,
                documents: Vec::new(),
                steps: Vec::new(),
                round_limit_reached: false,
                tools_unavailable: false,
                local_model: false,
                conversation_id: None,
            },
        ));
    }
    let history = normalize_history_capped(history, help.history_turns)?;
    // Excerpts for the no-tools fallback and the local Gemma (D19, R22):
    // both inject these into a plain prompt. The agent path consults the
    // same index on demand through `manual_search`.
    let picked: Vec<Hit> = {
        // The guard drops before the network calls below: the index is only
        // borrowed to rank, never across an await.
        let shared = help_index_shared(&app)?;
        let index = shared
            .lock()
            .map_err(|error| format!("help index lock poisoned: {error}"))?;
        pick_sections_with_budget(&index, &question, with_diagnostics, help.context_chars)
    };
    // R22: before the key gate, so the local Gemma needs no key at all.
    if help.local_engine && !remote.unwrap_or(false) {
        // Documents reach the model only through the CONTENT tools (D16),
        // which the local engine cannot drive. The UI does not offer the
        // box here; this guards older clients.
        if with_documents {
            return Err(HELP_LOCAL_NO_DOCUMENTS.to_string());
        }
        let prompt = build_help_prompt(&question, &history, &picked, diagnostics.as_deref());
        let answer =
            local_help_answer(&app, &db.db_path, prompt, help.max_tokens, help.temperature)
                .await
                .inspect_err(|error| {
                    // The panel only says «No se pudo responder»; the cause
                    // has to reach the diagnostic log.
                    if error != HELP_LOCAL_MODEL_MISSING {
                        crate::app_logs::warn(
                            &app,
                            "help",
                            format!("Gemma local no respondió: {error}"),
                        );
                    }
                })?;
        return Ok(super::history::attach_history(
            &app,
            conversation_id,
            &question,
            HelpAnswer {
                answer,
                sources: sources_of(&picked),
                local: false,
                documents: Vec::new(),
                steps: Vec::new(),
                round_limit_reached: false,
                tools_unavailable: false,
                local_model: true,
                conversation_id: None,
            },
        ));
    }
    let api_key = key_result?;
    // The agent system prompt (T22, S22): the local date and time plus
    // diagnostics when opted in. Manual excerpts travel only through
    // `manual_search` (D19). Document text is never injected here (D16):
    // with the box ticked the model reads it through the CONTENT tools.
    let system = build_agent_system(&now, diagnostics.as_deref());
    // The fallback's sources: the excerpts the plain prompt injects. The
    // agent path reports only what `manual_search` returned instead.
    let fallback_sources = sources_of(&picked);
    // The SAME agent MCP the app serves, in-process (D11, T21), behind the
    // help gate (D12, T22).
    let bridge = HelpMcpBridge::start(
        db.db_path.clone(),
        endpoint.cache_dir(),
        endpoint.approvals(),
        endpoint.corpus_events(),
        Arc::clone(&access),
        Some(app.clone()),
    )
    .await?;
    let transport = OpenRouterChatTransport::new(api_key.clone());
    // «Detener» (T24): the run registers its stop flag under the
    // frontend's id; `help_stop` flips it, the agent loop checks it before
    // every request and every tool call, and a pending inline approval for
    // the run resolves as rejected. The flag is forgotten when the run ends.
    let run_id = request_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let stop = run_id
        .as_deref()
        .map(|id| runs.start(id))
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let approver = TauriHelpApprover {
        app: app.clone(),
        pending: (*pending).clone(),
        stop: Arc::clone(&stop),
    };
    let gate = HelpGateExecutor::new(
        bridge,
        approver,
        with_documents,
        ManualSearch::new(help_index_shared(&app)?, help.context_chars),
    );
    let inputs = AgentInputs {
        system,
        history: history.clone(),
        question: question.clone(),
        model: help.model.clone(),
        max_tokens: help.max_tokens,
        temperature: help.temperature,
        max_rounds: help.max_rounds,
    };
    // Live steps (T24, T26): each executed tool call emits `help://step`
    // as it happens; the panel renders them under «Pensando…» until the
    // final list in the answer replaces them. A pending MCP step carries
    // its human target (never a UUID) and its approval id, so the panel
    // already says «Pidió borrar …» while the owner decides. The same
    // steps are collected here so a stopped run answers with what it had
    // done so far.
    let live_app = app.clone();
    let live_run = run_id.clone();
    let mut live_steps: Vec<HelpStep> = Vec::new();
    let outcome = run_help_agent(&transport, &gate, inputs, &stop, |step: AgentStep| {
        let status = agent_step_status_wire(&step.status).to_string();
        live_steps.push(HelpStep {
            tool: step.tool.clone(),
            args: step.args.clone(),
            status: status.clone(),
            target: step.target.clone(),
            approval_id: step.approval_id.clone(),
        });
        if let Some(run) = live_run.as_deref() {
            let _ = live_app.emit(
                HELP_STEP_EVENT,
                HelpStepEvent {
                    request_id: run.to_string(),
                    tool: step.tool.clone(),
                    args: step.args.clone(),
                    status,
                    target: step.target.clone(),
                    approval_id: step.approval_id.clone(),
                },
            );
        }
    })
    .await;
    // MCP approval outcomes (T26): every pending approval of this run is
    // followed in the background through the shared store — the same one
    // the MCP dialog decides — and reported as `help://approval-outcome`
    // with a clear Spanish message. The watch outlives the answer (up to
    // 10 minutes) because the owner usually decides after reading it. With
    // a stored conversation (S19), the message is also appended there, so
    // a reopened conversation still shows what happened.
    {
        let mut pendings = gate.pending_approvals();
        for pending in &mut pendings {
            pending.request_id = run_id.clone();
            pending.conversation_id = conversation_id.clone();
        }
        if !pendings.is_empty() {
            let watch_app = app.clone();
            let watch_store = endpoint.approvals();
            let notice_conversations: std::collections::HashMap<String, String> = pendings
                .iter()
                .filter_map(|pending| {
                    pending
                        .conversation_id
                        .clone()
                        .map(|conversation| (pending.approval_id.clone(), conversation))
                })
                .collect();
            tokio::spawn(async move {
                let reader = super::outcome::StoreApprovals::new(watch_store);
                super::outcome::watch_pending_approvals(
                    &reader,
                    pendings,
                    super::outcome::OUTCOME_WATCH_TIMEOUT,
                    super::outcome::OUTCOME_POLL_INTERVAL,
                    |event| {
                        let _ = watch_app.emit(super::outcome::HELP_APPROVAL_OUTCOME_EVENT, &event);
                        if let Some(conversation) = notice_conversations.get(&event.approval_id) {
                            if let Err(error) = super::history::append_notice(
                                &watch_app,
                                conversation,
                                &event.message,
                            ) {
                                log::warn!(target: "help", "history notice failed: {error}");
                            }
                        }
                    },
                )
                .await;
            });
        }
    }
    // S22/D19: `HelpAnswer.sources` lists ONLY what the model actually
    // consulted through `manual_search` — nothing injected, nothing
    // guessed — deduplicated, in call order, `kind` manual/screenMap as
    // today. An answer with no consulted sections returns `sources: []`.
    let consulted_sources: Vec<HelpSource> = gate
        .consulted_manual_sections()
        .iter()
        .map(|section| HelpSource {
            chapter: section.chapter.clone(),
            title: section.title.clone(),
            kind: section.source.wire_kind().to_string(),
        })
        .collect();
    let answer = match outcome {
        Ok(outcome) => Ok(HelpAnswer {
            answer: outcome.answer,
            sources: consulted_sources,
            local: false,
            documents: gate
                .content_titles()
                .into_iter()
                .map(|title| HelpDocument { title })
                .collect(),
            steps: outcome.steps,
            round_limit_reached: outcome.round_limit_reached,
            tools_unavailable: false,
            local_model: false,
            conversation_id: None,
        }),
        Err(error) if is_stop_error(&error) => {
            // «Detener» (T24): a clear message plus the steps done so far,
            // with the titles of the documents read up to the stop.
            let stopped = stopped_answer(live_steps);
            Ok(HelpAnswer {
                answer: stopped.answer,
                sources: consulted_sources,
                local: false,
                documents: gate
                    .content_titles()
                    .into_iter()
                    .map(|title| HelpDocument { title })
                    .collect(),
                steps: stopped.steps,
                round_limit_reached: false,
                tools_unavailable: false,
                local_model: false,
                conversation_id: None,
            })
        }
        Err(error) if tools_unsupported_error(&error) => {
            // The model or provider rejected tool use: retry once without
            // tools (plain answer as before, no actions) and say so in the
            // metadata. Document titles stay empty: without the CONTENT
            // tools no document text travelled (D16).
            let prompt = build_help_prompt(&question, &history, &picked, diagnostics.as_deref());
            let params = crate::llm::generation::OpenRouterGenerationParams::provider_defaults(
                help.max_tokens,
                help.temperature,
            );
            let answer = crate::llm::openrouter::OpenRouterClient::new(api_key, help.model)
                .generate(&prompt, &params)
                .await?;
            Ok(HelpAnswer {
                answer,
                sources: fallback_sources,
                local: false,
                documents: Vec::new(),
                steps: Vec::new(),
                round_limit_reached: false,
                tools_unavailable: true,
                local_model: false,
                conversation_id: None,
            })
        }
        Err(error) => Err(error),
    };
    if let Some(run) = run_id.as_deref() {
        runs.finish(run);
    }
    // S19: the question and the final answer join the stored
    // conversation when the frontend sent one.
    answer.map(|answer| super::history::attach_history(&app, conversation_id, &question, answer))
}

/// Stops a running help agent turn (T24 «Detener»): flips the stop flag of
/// `requestId`, so the run returns quickly with a «Detenido» answer and a
/// pending inline approval for that run resolves as rejected. Unknown
/// (finished or never started) ids are an error.
#[tauri::command]
pub async fn help_stop(
    request_id: String,
    runs: tauri::State<'_, HelpAgentRuns>,
) -> Result<(), String> {
    if runs.stop(request_id.trim()) {
        Ok(())
    } else {
        Err("No hay una pregunta en curso con ese identificador.".to_string())
    }
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

#[cfg(test)]
mod tests {
    use super::super::agent_llm::{
        ChatRequest, ChatResponse, ChatTransport, HelpToolExecutor, ToolCall, ToolOutcome, ToolSpec,
    };
    use super::*;
    use std::collections::VecDeque;

    /// One scripted OpenRouter round-trip: records every request, answers
    /// from the queue.
    struct MockTransport {
        responses: Mutex<VecDeque<ChatResponse>>,
        recorded: Mutex<Vec<ChatRequest>>,
    }

    impl MockTransport {
        fn new(responses: Vec<ChatResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                recorded: Mutex::new(Vec::new()),
            }
        }

        fn tool_names(&self, index: usize) -> Vec<String> {
            self.recorded.lock().expect("lock")[index]
                .tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect()
        }

        fn tool_texts(&self, index: usize) -> Vec<String> {
            self.recorded.lock().expect("lock")[index]
                .messages
                .iter()
                .filter(|message| message.role == "tool")
                .filter_map(|message| message.content.clone())
                .collect()
        }
    }

    impl ChatTransport for MockTransport {
        async fn send(&self, request: ChatRequest) -> Result<ChatResponse, String> {
            self.recorded.lock().expect("lock").push(request);
            self.responses
                .lock()
                .expect("lock")
                .pop_front()
                .ok_or_else(|| "sin respuesta programada".to_string())
        }
    }

    /// A scripted bridge: offers `tools`, answers per name from `outcomes`.
    #[derive(Clone)]
    struct MockBridge {
        tools: Vec<String>,
        outcomes: Arc<Mutex<HashMap<String, ToolOutcome>>>,
        calls: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    }

    impl MockBridge {
        fn new(tools: &[&str]) -> Self {
            Self {
                tools: tools.iter().map(|name| name.to_string()).collect(),
                outcomes: Arc::new(Mutex::new(HashMap::new())),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn calls(&self) -> Vec<(String, serde_json::Value)> {
            self.calls.lock().expect("lock").clone()
        }
    }

    impl HelpToolExecutor for MockBridge {
        fn tools(&self) -> Vec<ToolSpec> {
            self.tools
                .iter()
                .map(|name| ToolSpec {
                    name: name.clone(),
                    description: format!("hace {name}"),
                    input_schema: serde_json::json!({"type": "object"}),
                })
                .collect()
        }

        async fn call(&self, name: &str, args: serde_json::Value) -> ToolOutcome {
            self.calls
                .lock()
                .expect("lock")
                .push((name.to_string(), args));
            self.outcomes
                .lock()
                .expect("lock")
                .remove(name)
                .unwrap_or(ToolOutcome::Ok {
                    content: format!("eco de {name}"),
                })
        }
    }

    /// A scripted owner: answers inline cards from the queue.
    #[derive(Clone)]
    struct MockApprover {
        answers: Arc<Mutex<VecDeque<ApprovalAnswer>>>,
        requests: Arc<Mutex<Vec<(String, serde_json::Value, String)>>>,
    }

    impl MockApprover {
        fn new(answers: Vec<ApprovalAnswer>) -> Self {
            Self {
                answers: Arc::new(Mutex::new(answers.into())),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn requests(&self) -> Vec<(String, serde_json::Value, String)> {
            self.requests.lock().expect("lock").clone()
        }
    }

    impl HelpApprover for MockApprover {
        async fn request_approval(
            &self,
            tool: &str,
            args: serde_json::Value,
            summary: String,
        ) -> ApprovalAnswer {
            self.requests
                .lock()
                .expect("lock")
                .push((tool.to_string(), args, summary));
            self.answers
                .lock()
                .expect("lock")
                .pop_front()
                .unwrap_or(ApprovalAnswer::Rejected)
        }
    }

    const BRIDGE_TOOLS: &[&str] = &[
        "collections_list",
        "collections_create",
        "search_items",
        "documents_get_text",
        "search_passages",
    ];

    fn inputs(question: &str) -> AgentInputs {
        AgentInputs {
            system: "sos ayuda".to_string(),
            history: Vec::new(),
            question: question.to_string(),
            model: "vendor/modelo".to_string(),
            max_tokens: 200,
            temperature: 0.2,
            max_rounds: super::super::settings::HELP_MAX_ROUNDS_DEFAULT,
        }
    }

    fn answer(content: &str) -> ChatResponse {
        ChatResponse {
            content: Some(content.to_string()),
            tool_calls: Vec::new(),
        }
    }

    fn tool_call(id: &str, name: &str, args: &str) -> ChatResponse {
        ChatResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: args.to_string(),
            }],
        }
    }

    #[tokio::test]
    async fn a_question_runs_a_read_tool_and_answers() {
        let transport = MockTransport::new(vec![
            tool_call("id-1", "collections_list", "{}"),
            answer("Tenés 3 colecciones."),
        ]);
        let bridge = MockBridge::new(BRIDGE_TOOLS);
        let approver = MockApprover::new(vec![]);
        let gate = HelpGateExecutor::new(
            bridge.clone(),
            approver.clone(),
            false,
            super::super::manual_tool::memory_searcher(),
        );
        let outcome = run_help_agent(
            &transport,
            &gate,
            inputs("¿qué colecciones tengo?"),
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .expect("debe responder");
        assert_eq!(outcome.answer, "Tenés 3 colecciones.");
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].tool, "collections_list");
        assert_eq!(outcome.steps[0].status, "ok");
        assert!(!outcome.round_limit_reached);
        assert_eq!(bridge.calls().len(), 1);
        assert!(approver.requests().is_empty(), "reads never ask");
    }

    /// Saturday, 14 November 2026, 14:05: the fixed clock the date-line
    /// tests run on.
    fn fixed_now() -> super::super::clock::LocalDateTime {
        super::super::clock::LocalDateTime {
            year: 2026,
            month: 11,
            day: 14,
            weekday: super::super::clock::Weekday::Saturday,
            hour: 14,
            minute: 5,
        }
    }

    #[tokio::test]
    async fn agent_request_offers_manual_search_with_date_line_but_no_excerpts() {
        // S22/D19: the agent request offers `manual_search`; the system
        // prompt carries the local date and time, never manual text. The
        // agent calls the tool before answering, without any approval.
        let transport = MockTransport::new(vec![
            tool_call(
                "id-1",
                "manual_search",
                r#"{"query": "importar archivos pdf"}"#,
            ),
            answer("Arrastrá los PDF a la colección."),
        ]);
        let bridge = MockBridge::new(BRIDGE_TOOLS);
        let approver = MockApprover::new(vec![]);
        let gate = HelpGateExecutor::new(
            bridge.clone(),
            approver.clone(),
            false,
            super::super::manual_tool::memory_searcher(),
        );
        let mut request = inputs("¿cómo importo un pdf?");
        request.system = build_agent_system(&fixed_now(), None);
        let outcome = run_help_agent(&transport, &gate, request, &AtomicBool::new(false), |_| {})
            .await
            .expect("debe responder");
        assert_eq!(outcome.answer, "Arrastrá los PDF a la colección.");
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].tool, "manual_search");
        assert_eq!(outcome.steps[0].status, "ok");
        assert!(approver.requests().is_empty(), "manual_search never asks");
        assert!(
            bridge.calls().is_empty(),
            "the index answers, never the bridge"
        );
        let names = transport.tool_names(0);
        assert!(
            names.contains(&"manual_search".to_string()),
            "manual_search is offered: {names:?}"
        );
        let body = super::super::agent_llm::chat_request_body(
            &transport.recorded.lock().expect("lock")[0],
        );
        let system = body
            .pointer("/messages/0/content")
            .and_then(|value| value.as_str())
            .expect("the system travels first");
        assert!(
            system.contains("manual_search"),
            "the system tells the model to consult: {system}"
        );
        assert!(
            system.contains("14 de noviembre de 2026"),
            "the date line travels: {system}"
        );
        assert!(system.contains("14:05"), "the time travels: {system}");
        assert!(
            !system.contains("Arrastrá los PDF"),
            "no excerpt is injected: {system}"
        );
    }

    #[tokio::test]
    async fn sources_list_only_what_manual_search_returned() {
        // S22/D19: `HelpAnswer.sources` are exactly the sections the
        // `manual_search` calls returned — one call lists those sources,
        // no call lists none — `kind` manual/screenMap as today.
        let transport = MockTransport::new(vec![
            tool_call(
                "id-1",
                "manual_search",
                r#"{"query": "importar archivos pdf"}"#,
            ),
            answer("Arrastrá los PDF a la colección."),
        ]);
        let bridge = MockBridge::new(BRIDGE_TOOLS);
        let approver = MockApprover::new(vec![]);
        let gate = HelpGateExecutor::new(
            bridge.clone(),
            approver.clone(),
            false,
            super::super::manual_tool::memory_searcher(),
        );
        let mut request = inputs("¿cómo importo un pdf?");
        request.system = build_agent_system(&fixed_now(), None);
        run_help_agent(&transport, &gate, request, &AtomicBool::new(false), |_| {})
            .await
            .expect("debe responder");
        let sources: Vec<HelpSource> = gate
            .consulted_manual_sections()
            .iter()
            .map(|section| HelpSource {
                chapter: section.chapter.clone(),
                title: section.title.clone(),
                kind: section.source.wire_kind().to_string(),
            })
            .collect();
        assert!(!sources.is_empty(), "one call lists its sources");
        assert!(sources[0].title.contains("Importar"), "{sources:?}");
        assert_eq!(sources[0].kind, "manual", "{sources:?}");
        assert!(
            sources
                .iter()
                .all(|source| source.kind == "manual" || source.kind == "screenMap"),
            "kinds as today: {sources:?}"
        );

        let transport = MockTransport::new(vec![answer("¡Hola!")]);
        let gate = HelpGateExecutor::new(
            MockBridge::new(BRIDGE_TOOLS),
            MockApprover::new(vec![]),
            false,
            super::super::manual_tool::memory_searcher(),
        );
        let mut request = inputs("hola");
        request.system = build_agent_system(&fixed_now(), None);
        run_help_agent(&transport, &gate, request, &AtomicBool::new(false), |_| {})
            .await
            .expect("debe responder");
        assert!(
            gate.consulted_manual_sections().is_empty(),
            "no call means no sources"
        );
    }

    #[tokio::test]
    async fn an_approval_tool_step_carries_target_and_pending_status() {
        // T26: a step for an MCP approval tool carries the approval id
        // and the collection NAME (never the UUID) with status pending.
        let transport = MockTransport::new(vec![
            tool_call(
                "id-1",
                "collections_delete",
                r#"{"collectionId":"29189757-1545-411c-b1cc-93132f7aaab7"}"#,
            ),
            answer("Pedí borrarla; aprobala en el diálogo."),
        ]);
        let bridge = MockBridge::new(&["collections_delete"]);
        bridge.outcomes.lock().expect("lock").insert(
            "collections_delete".to_string(),
            super::super::agent_llm::ToolOutcome::Ok {
                content: serde_json::json!({
                    "status": "pending",
                    "id": "apr_1",
                    "clientId": "in-app-help",
                    "clientName": "Ayuda",
                    "collections": ["SIMAPE"],
                    "files": 2,
                })
                .to_string(),
            },
        );
        let approver = MockApprover::new(vec![]);
        let gate = HelpGateExecutor::new(
            bridge.clone(),
            approver.clone(),
            false,
            super::super::manual_tool::memory_searcher(),
        );
        let outcome = run_help_agent(
            &transport,
            &gate,
            inputs("Borrá la colección SIMAPE"),
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .expect("debe responder");
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].tool, "collections_delete");
        assert_eq!(outcome.steps[0].status, "pending");
        assert_eq!(outcome.steps[0].target, Some("SIMAPE".to_string()));
        assert_eq!(outcome.steps[0].approval_id, Some("apr_1".to_string()));
        let pendings = gate.pending_approvals();
        assert_eq!(pendings.len(), 1);
        assert_eq!(pendings[0].approval_id, "apr_1");
    }

    #[tokio::test]
    async fn an_action_asks_inline_and_forwards_when_approved() {
        let transport = MockTransport::new(vec![
            tool_call(
                "id-1",
                "collections_create",
                r#"{"requestId":"r-1","name":"SIMAPE"}"#,
            ),
            answer("Colección SIMAPE creada."),
        ]);
        let bridge = MockBridge::new(BRIDGE_TOOLS);
        let approver = MockApprover::new(vec![ApprovalAnswer::Approved]);
        let gate = HelpGateExecutor::new(
            bridge.clone(),
            approver.clone(),
            false,
            super::super::manual_tool::memory_searcher(),
        );
        let outcome = run_help_agent(
            &transport,
            &gate,
            inputs("Creá una colección SIMAPE"),
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .expect("debe responder");
        assert_eq!(outcome.answer, "Colección SIMAPE creada.");
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].tool, "collections_create");
        assert_eq!(outcome.steps[0].status, "ok");
        let forwarded: Vec<_> = bridge
            .calls()
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        assert_eq!(forwarded, vec!["collections_create".to_string()]);
        let requests = approver.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "collections_create");
        assert_eq!(requests[0].2, "Crear la colección «SIMAPE»");
    }

    #[tokio::test]
    async fn a_rejected_action_returns_the_denial_to_the_model() {
        let transport = MockTransport::new(vec![
            tool_call(
                "id-1",
                "collections_create",
                r#"{"requestId":"r-1","name":"SIMAPE"}"#,
            ),
            answer("No puedo crearla sin tu aprobación."),
        ]);
        let bridge = MockBridge::new(BRIDGE_TOOLS);
        let approver = MockApprover::new(vec![ApprovalAnswer::Rejected]);
        let gate = HelpGateExecutor::new(
            bridge.clone(),
            approver.clone(),
            false,
            super::super::manual_tool::memory_searcher(),
        );
        let outcome = run_help_agent(
            &transport,
            &gate,
            inputs("Creá una colección SIMAPE"),
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .expect("debe responder");
        assert_eq!(outcome.answer, "No puedo crearla sin tu aprobación.");
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].status, "denied");
        assert!(bridge.calls().is_empty(), "rejected means never forwarded");
        let texts = transport.tool_texts(1);
        assert_eq!(texts.len(), 1);
        assert!(
            texts[0].contains("rechazó"),
            "el modelo ve el rechazo: {}",
            texts[0]
        );
    }

    #[tokio::test]
    async fn content_tools_are_absent_from_the_request_without_the_box() {
        let transport = MockTransport::new(vec![answer("listo")]);
        let bridge = MockBridge::new(BRIDGE_TOOLS);
        let approver = MockApprover::new(vec![]);
        let gate = HelpGateExecutor::new(
            bridge,
            approver,
            false,
            super::super::manual_tool::memory_searcher(),
        );
        run_help_agent(
            &transport,
            &gate,
            inputs("¿qué dice mi acta?"),
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .expect("debe responder");
        let names = transport.tool_names(0);
        assert!(!names.contains(&"documents_get_text".to_string()));
        assert!(!names.contains(&"search_passages".to_string()));
        assert!(names.contains(&"search_items".to_string()));
        assert!(names.contains(&"collections_list".to_string()));
    }

    #[tokio::test]
    async fn content_tools_are_present_with_the_box() {
        let transport = MockTransport::new(vec![answer("listo")]);
        let bridge = MockBridge::new(BRIDGE_TOOLS);
        let approver = MockApprover::new(vec![]);
        let gate = HelpGateExecutor::new(
            bridge,
            approver,
            true,
            super::super::manual_tool::memory_searcher(),
        );
        run_help_agent(
            &transport,
            &gate,
            inputs("¿qué dice mi acta?"),
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .expect("debe responder");
        let names = transport.tool_names(0);
        assert!(names.contains(&"documents_get_text".to_string()));
        assert!(names.contains(&"search_passages".to_string()));
    }

    #[test]
    fn tools_unsupported_error_names_the_provider_rejection() {
        assert!(tools_unsupported_error(
            "OpenRouter devolvió un error (400): this endpoint does not support tool use"
        ));
        assert!(tools_unsupported_error(
            "No se pudo leer la respuesta: unexpected tool_choice"
        ));
        assert!(!tools_unsupported_error(
            "Clave de OpenRouter inválida (401): revisá la clave"
        ));
        assert!(!tools_unsupported_error(
            "No se pudo contactar a OpenRouter: timeout"
        ));
        assert!(!tools_unsupported_error("La herramienta falló: se rompió"));
    }

    #[test]
    fn step_events_carry_request_tool_args_and_status() {
        // T24: while the agent runs, the panel follows `help://step` live.
        // T26 adds the human target and the approval id (both default off).
        assert_eq!(HELP_STEP_EVENT, "help://step");
        let event = HelpStepEvent {
            request_id: "r-1".to_string(),
            tool: "collections_list".to_string(),
            args: "{}".to_string(),
            status: "ok".to_string(),
            target: None,
            approval_id: None,
        };
        let value = serde_json::to_value(&event).expect("serializes");
        assert_eq!(
            value,
            serde_json::json!({
                "requestId": "r-1",
                "tool": "collections_list",
                "args": "{}",
                "status": "ok",
            })
        );
        let pending = HelpStepEvent {
            request_id: "r-1".to_string(),
            tool: "collections_delete".to_string(),
            args: "{}".to_string(),
            status: "pending".to_string(),
            target: Some("SIMAPE".to_string()),
            approval_id: Some("apr_1".to_string()),
        };
        let value = serde_json::to_value(&pending).expect("serializes");
        assert_eq!(
            value,
            serde_json::json!({
                "requestId": "r-1",
                "tool": "collections_delete",
                "args": "{}",
                "status": "pending",
                "target": "SIMAPE",
                "approvalId": "apr_1",
            })
        );
    }

    #[test]
    fn stop_flag_runs_are_recognised_and_answered_as_stopped() {
        // T24 «Detener»: a run stopped mid-loop answers quickly with a
        // clear message and keeps the steps done so far.
        assert!(is_stop_error("detenido"));
        assert!(!is_stop_error("La herramienta falló: se rompió"));
        let steps = vec![HelpStep {
            tool: "collections_list".to_string(),
            args: "{}".to_string(),
            status: "ok".to_string(),
            target: None,
            approval_id: None,
        }];
        let stopped = stopped_answer(steps.clone());
        assert!(
            stopped.answer.contains("Detenido"),
            "the answer says the run stopped: {}",
            stopped.answer
        );
        assert_eq!(stopped.steps, steps);
        assert!(!stopped.round_limit_reached);
    }

    #[test]
    fn help_runs_track_a_stop_flag_per_request() {
        // T24: `help_stop(requestId)` flips the flag of that run only;
        // the flag is removed when the run finishes.
        let runs = HelpAgentRuns::default();
        assert!(!runs.stop("missing"), "unknown requests stop nothing");
        let flag = runs.start("r-1");
        assert!(!flag.load(std::sync::atomic::Ordering::Relaxed));
        assert!(runs.stop("r-1"));
        assert!(flag.load(std::sync::atomic::Ordering::Relaxed));
        assert!(!runs.stop("other"), "one run stops only itself");
        runs.finish("r-1");
        assert!(!runs.stop("r-1"), "finished runs are gone");
    }

    #[tokio::test]
    async fn a_stopped_run_makes_no_more_executor_calls() {
        // T24: the stop flag set between rounds means no more tool calls —
        // the same path `help_stop` flips while `help_ask` is awaiting.
        let transport = MockTransport::new(vec![
            tool_call("id-1", "collections_list", "{}"),
            answer("nunca"),
        ]);
        let bridge = MockBridge::new(BRIDGE_TOOLS);
        let approver = MockApprover::new(vec![]);
        let gate = HelpGateExecutor::new(
            bridge.clone(),
            approver.clone(),
            false,
            super::super::manual_tool::memory_searcher(),
        );
        let stop = AtomicBool::new(false);
        let result = run_help_agent(
            &transport,
            &gate,
            inputs("¿qué colecciones tengo?"),
            &stop,
            |_| {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            },
        )
        .await;
        assert_eq!(result.unwrap_err(), "detenido");
        assert_eq!(bridge.calls().len(), 1, "sin más llamadas tras detener");
    }
}
