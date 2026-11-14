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

/// One live step emitted over [`HELP_STEP_EVENT`] (T24): which run it
/// belongs to, the tool, the compact JSON args and the outcome on the wire
/// as `"ok"`, `"denied"` or `"failed"` — the same shape as `HelpStep`
/// plus the run id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HelpStepEvent {
    pub request_id: String,
    pub tool: String,
    pub args: String,
    pub status: String,
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

/// Answers a help question through the help agent (T22, S15): the manual
/// excerpts (and the opted-in diagnostic report) ground the system prompt on
/// the user's own OpenRouter key (S1–S3, S5), and the model drives the
/// in-process agent MCP behind the help gate (D11, D12). The conversation
/// lives in memory only; nothing is persisted (D4).
///
/// Before the key gate, a question that matches a bundled FAQ entry (S10)
/// is answered locally with that entry's text, verbatim: no key needed, no
/// network touched — unless `help_local_answers` is off, which sends every
/// question to the model. Questions with diagnostics attached always go to
/// the model. `documents` (S12) opts into the CONTENT tools for this question
/// only, camelCase on the wire, absent means off: document text reaches the
/// model only through those tools (D16), and a question with documents on
/// never gets a local FAQ answer. `requestId` (T24) is the frontend-generated
/// id of this question: while the agent runs, every executed tool call is
/// emitted live as a `help://step` `{ requestId, tool, args, status }`
/// event, and `help_stop` ends the run through it. Absent means no live
/// events and no stopping (older clients keep working).
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
                kind: super::index::Source::Faq.wire_kind().to_string(),
            }],
            local: true,
            documents: Vec::new(),
            steps: Vec::new(),
            round_limit_reached: false,
            tools_unavailable: false,
        });
    }
    let api_key = key_result?;
    let history = normalize_history_capped(history, help.history_turns)?;
    let picked: Vec<Hit> = {
        // The guard drops before the network calls below: the index is only
        // borrowed to rank, never across an await.
        let index = help_index(&app)?;
        pick_sections_with_budget(&index, &question, with_diagnostics, help.context_chars)
    };
    // The agent system prompt (T22): manual/map excerpts plus diagnostics
    // when opted in. Document text is never injected here (D16): with the
    // box ticked the model reads it through the CONTENT tools instead.
    let system = build_agent_system(&picked, diagnostics.as_deref());
    let sources = picked
        .iter()
        .map(|section| HelpSource {
            chapter: section.chapter.clone(),
            title: section.title.clone(),
            kind: section.source.wire_kind().to_string(),
        })
        .collect();
    // The SAME agent MCP the app serves, in-process (D11, T21), behind the
    // help gate (D12, T22).
    let bridge = HelpMcpBridge::start(
        db.db_path.clone(),
        endpoint.cache_dir(),
        endpoint.approvals(),
        endpoint.corpus_events(),
        Arc::clone(&access),
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
    let gate = HelpGateExecutor::new(bridge, approver, with_documents);
    let inputs = AgentInputs {
        system,
        history: history.clone(),
        question: question.clone(),
        model: help.model.clone(),
        max_tokens: help.max_tokens,
        temperature: help.temperature,
        max_rounds: help.max_rounds,
    };
    // Live steps (T24): each executed tool call emits `help://step` as it
    // happens; the panel renders them under «Pensando…» until the final
    // list in the answer replaces them. The same steps are collected here
    // so a stopped run answers with what it had done so far.
    let live_app = app.clone();
    let live_run = run_id.clone();
    let mut live_steps: Vec<HelpStep> = Vec::new();
    let outcome = run_help_agent(&transport, &gate, inputs, &stop, |step: AgentStep| {
        let status = agent_step_status_wire(&step.status).to_string();
        live_steps.push(HelpStep {
            tool: step.tool.clone(),
            args: step.args.clone(),
            status: status.clone(),
        });
        if let Some(run) = live_run.as_deref() {
            let _ = live_app.emit(
                HELP_STEP_EVENT,
                HelpStepEvent {
                    request_id: run.to_string(),
                    tool: step.tool.clone(),
                    args: step.args.clone(),
                    status,
                },
            );
        }
    })
    .await;
    let answer = match outcome {
        Ok(outcome) => Ok(HelpAnswer {
            answer: outcome.answer,
            sources,
            local: false,
            documents: gate
                .content_titles()
                .into_iter()
                .map(|title| HelpDocument { title })
                .collect(),
            steps: outcome.steps,
            round_limit_reached: outcome.round_limit_reached,
            tools_unavailable: false,
        }),
        Err(error) if is_stop_error(&error) => {
            // «Detener» (T24): a clear message plus the steps done so far,
            // with the titles of the documents read up to the stop.
            let stopped = stopped_answer(live_steps);
            Ok(HelpAnswer {
                answer: stopped.answer,
                sources,
                local: false,
                documents: gate
                    .content_titles()
                    .into_iter()
                    .map(|title| HelpDocument { title })
                    .collect(),
                steps: stopped.steps,
                round_limit_reached: false,
                tools_unavailable: false,
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
                sources,
                local: false,
                documents: Vec::new(),
                steps: Vec::new(),
                round_limit_reached: false,
                tools_unavailable: true,
            })
        }
        Err(error) => Err(error),
    };
    if let Some(run) = run_id.as_deref() {
        runs.finish(run);
    }
    answer
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
        let gate = HelpGateExecutor::new(bridge.clone(), approver.clone(), false);
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
        let gate = HelpGateExecutor::new(bridge.clone(), approver.clone(), false);
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
        let gate = HelpGateExecutor::new(bridge.clone(), approver.clone(), false);
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
        let gate = HelpGateExecutor::new(bridge, approver, false);
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
        let gate = HelpGateExecutor::new(bridge, approver, true);
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
        assert_eq!(HELP_STEP_EVENT, "help://step");
        let event = HelpStepEvent {
            request_id: "r-1".to_string(),
            tool: "collections_list".to_string(),
            args: "{}".to_string(),
            status: "ok".to_string(),
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
        let gate = HelpGateExecutor::new(bridge.clone(), approver.clone(), false);
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
