//! Help agent tool loop (T20, S15, D13): OpenRouter tool calling over a
//! narrow executor trait, with scripted mocks in tests. T21/T22 wire it
//! into `help_ask`; until then only tests call it.
#![cfg_attr(not(test), allow(dead_code))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Public contract (T21-T24 build on these names; keep them stable)
// ---------------------------------------------------------------------------

/// One tool the executor offers this turn.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// The outcome of one tool call: `Ok` carries the text for the model,
/// `Pending` carries a pending MCP approval (the bridge answered with
/// `{status: "pending", id}` and waits for the owner in the MCP dialog),
/// `Denied` means the call must not run (owner rejected, tool not allowed),
/// `Failed` means it ran and broke. Every variant travels back to the
/// model as plain text, never as a crash.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ToolOutcome {
    Ok {
        content: String,
    },
    /// An MCP approval the owner still has to answer (T26): `content`
    /// is the pending JSON for the model, `approval_id` follows the
    /// outcome, and `target` names the object when known (never a UUID).
    Pending {
        content: String,
        approval_id: String,
        target: Option<String>,
    },
    Denied {
        reason: String,
    },
    Failed {
        message: String,
    },
}

/// The tools the agent may call. `call` may await a long time (an owner
/// approval in T22); the loop never times it out.
pub(crate) trait HelpToolExecutor: Send + Sync {
    fn tools(&self) -> Vec<ToolSpec>;
    fn call(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> impl std::future::Future<Output = ToolOutcome> + Send;
}

/// One model-requested tool call (raw JSON-string arguments from the wire).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// One conversation message: `system`/`user`/`assistant` carry `content`,
/// an assistant turn with tools also carries `tool_calls`, and a `tool`
/// turn carries `tool_call_id` plus the outcome text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChatMessage {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
}

/// The wire-level request the transport sends.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub tool_choice: String,
    pub max_tokens: i32,
    pub temperature: f32,
}

/// The wire-level answer the transport parsed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChatResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

/// The transport: one OpenRouter chat round.
pub(crate) trait ChatTransport: Send + Sync {
    fn send(
        &self,
        request: ChatRequest,
    ) -> impl std::future::Future<Output = Result<ChatResponse, String>> + Send;
}

/// The agent-level request `run_agent` starts from: the model and budgets
/// plus the system prompt, the remembered `(role, content)` history and
/// the current question. `max_rounds` caps the tool-using model turns.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentRequest {
    pub model: String,
    pub system: String,
    pub history: Vec<(String, String)>,
    pub question: String,
    pub max_rounds: usize,
    pub max_tokens: i32,
    pub temperature: f32,
}

/// One executed tool step shown in the panel: the tool name, the compact
/// JSON args capped at [`ARGS_PREVIEW_MAX_CHARS`] chars, and the outcome.
/// A `Pending` step (T26) waits for the owner in the MCP dialog; its
/// `approval_id` follows the outcome and `target` names the object.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentStep {
    pub tool: String,
    pub args: String,
    pub status: AgentStepStatus,
    pub approval_id: Option<String>,
    pub target: Option<String>,
}

/// Whether one tool call succeeded, waits for the owner, was denied or failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AgentStepStatus {
    Ok,
    Pending,
    Denied,
    Failed,
}

/// The finished agent turn: the answer, every step in order, how many
/// tool-using model turns ran, and whether the round cap forced the final
/// no-tools call.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentResult {
    pub answer: String,
    pub steps: Vec<AgentStep>,
    pub rounds_used: usize,
    pub round_limit_reached: bool,
}

/// `tool_choice` for rounds that may call tools.
pub(crate) const TOOL_CHOICE_AUTO: &str = "auto";
/// `tool_choice` for the final answer round after the cap.
pub(crate) const TOOL_CHOICE_NONE: &str = "none";
/// Longest `AgentStep.args` preview, in chars.
pub(crate) const ARGS_PREVIEW_MAX_CHARS: usize = 300;
/// Returned when `stop` is set: the owner pressed «Detener».
const STOP_MESSAGE: &str = "detenido";
/// Nudge appended before the final no-tools call so the model answers
/// with what it gathered instead of asking for more tools.
const FINAL_ANSWER_NUDGE: &str =
    "Con lo que reuniste hasta ahora, respondé la pregunta original lo mejor posible sin usar más herramientas.";
/// Same chat URL as `llm/openrouter.rs` (D13: help-specific client reusing
/// the constant and the user agent, not a change to the shared client).
const OPENROUTER_CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
/// Same budgets as `OpenRouterClient`: a long RAG-style answer may take a
/// while, but a stalled connection still fails fast on connect.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Longest provider-body excerpt quoted in an error.
const BODY_EXCERPT_CHARS: usize = 300;

/// The compact JSON preview stored on [`AgentStep.args`].
pub(crate) fn compact_args_preview(args: &serde_json::Value) -> String {
    let compact = serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string());
    compact.chars().take(ARGS_PREVIEW_MAX_CHARS).collect()
}

/// The exact OpenRouter JSON the transport posts: model, messages, the
/// `tools: [{type: "function", function: {name, description, parameters}}]`
/// array, `tool_choice`, `max_tokens` and `temperature`. The reqwest client
/// below and the shape test share this builder.
pub(crate) fn chat_request_body(request: &ChatRequest) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = request
        .messages
        .iter()
        .map(|message| {
            if message.role == "tool" {
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": message.tool_call_id.clone().unwrap_or_default(),
                    "content": message.content.clone().unwrap_or_default(),
                })
            } else if message.tool_calls.is_empty() {
                serde_json::json!({
                    "role": message.role,
                    "content": message.content.clone().unwrap_or_default(),
                })
            } else {
                let calls: Vec<serde_json::Value> = message
                    .tool_calls
                    .iter()
                    .map(|call| {
                        serde_json::json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": call.arguments,
                            },
                        })
                    })
                    .collect();
                let content = message
                    .content
                    .clone()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null);
                serde_json::json!({
                    "role": message.role,
                    "content": content,
                    "tool_calls": calls,
                })
            }
        })
        .collect();
    let tools: Vec<serde_json::Value> = request
        .tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                },
            })
        })
        .collect();
    serde_json::json!({
        "model": request.model,
        "messages": messages,
        "tools": tools,
        "tool_choice": request.tool_choice,
        "max_tokens": request.max_tokens,
        "temperature": request.temperature,
    })
}

/// Spanish HTTP error for an OpenRouter status: 401 names the invalid key,
/// 402 the missing credit, 429 the rate limit, 5xx the unavailable service,
/// anything else keeps the status plus a bounded body excerpt.
pub(crate) fn openrouter_status_message(status: u16, body: &str) -> String {
    let excerpt: String = body.trim().chars().take(BODY_EXCERPT_CHARS).collect();
    match status {
        401 => format!(
            "Clave de OpenRouter inválida (401): revisá la clave en Configuración → APIs remotas. {excerpt}"
        ),
        402 => format!(
            "Sin crédito en OpenRouter (402): la cuenta no tiene crédito disponible, cargá saldo y reintentá. {excerpt}"
        ),
        429 => format!(
            "Límite de uso de OpenRouter (429): demasiadas solicitudes, esperá un momento y reintentá. {excerpt}"
        ),
        500..=599 => format!(
            "Servicio de OpenRouter no disponible ({status}): intentá de nuevo más tarde. {excerpt}"
        ),
        _ => format!("OpenRouter devolvió un error ({status}): {excerpt}"),
    }
}

#[derive(serde::Deserialize)]
struct WireBody {
    #[serde(default)]
    choices: Vec<WireChoice>,
}

#[derive(serde::Deserialize)]
struct WireChoice {
    #[serde(default)]
    message: Option<WireMessage>,
}

#[derive(serde::Deserialize)]
struct WireMessage {
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    tool_calls: Option<Vec<WireToolCall>>,
}

#[derive(serde::Deserialize)]
struct WireToolCall {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<WireFunction>,
}

#[derive(serde::Deserialize)]
struct WireFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// Parses an OpenRouter `200` body into content plus raw tool calls:
/// `choices[0].message.content` (string or null) and
/// `choices[0].message.tool_calls[] {id, function: {name, arguments}}`
/// where `arguments` stays the raw JSON string for `run_agent` to validate.
pub(crate) fn parse_chat_response(body: &str) -> Result<ChatResponse, String> {
    let parsed: WireBody = serde_json::from_str(body).map_err(|error| {
        let excerpt: String = body.trim().chars().take(BODY_EXCERPT_CHARS).collect();
        format!("No se pudo leer la respuesta de OpenRouter: {error}; cuerpo: {excerpt}")
    })?;
    let message = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message)
        .ok_or_else(|| "OpenRouter no devolvió opciones".to_string())?;
    let content = match message.content {
        Some(serde_json::Value::String(text)) => Some(text),
        _ => None,
    };
    let tool_calls = message
        .tool_calls
        .unwrap_or_default()
        .into_iter()
        .map(|call| {
            let function = call.function.unwrap_or(WireFunction {
                name: None,
                arguments: None,
            });
            ToolCall {
                id: call.id.unwrap_or_default(),
                name: function.name.unwrap_or_default(),
                arguments: function.arguments.unwrap_or_default(),
            }
        })
        .collect();
    Ok(ChatResponse {
        content,
        tool_calls,
    })
}

/// The reqwest OpenRouter transport (D13): `Authorization: Bearer <key>`,
/// model, messages, tools, `tool_choice`, `max_tokens`, `temperature`.
/// Tests use the scripted mock below; this type waits for T21 to be built.
#[allow(dead_code)]
pub(crate) struct OpenRouterChatTransport {
    client: reqwest::Client,
    api_key: String,
}

#[allow(dead_code)]
impl OpenRouterChatTransport {
    pub(crate) fn new(api_key: String) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(crate::USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .expect("Failed to build help reqwest client");
        Self { client, api_key }
    }
}

impl ChatTransport for OpenRouterChatTransport {
    async fn send(&self, request: ChatRequest) -> Result<ChatResponse, String> {
        let body = chat_request_body(&request);
        let response = self
            .client
            .post(OPENROUTER_CHAT_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://hlab.com.ar/")
            .header("X-Title", "EntropIA")
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("No se pudo contactar a OpenRouter: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(openrouter_status_message(status.as_u16(), &text));
        }
        let text = response
            .text()
            .await
            .map_err(|error| format!("No se pudo leer la respuesta de OpenRouter: {error}"))?;
        parse_chat_response(&text)
    }
}

/// Runs the bounded agent loop: each round sends the conversation with
/// `tool_choice: "auto"`; a content-only answer ends the turn; tool calls
/// run in order through the executor and their outcome text (denials and
/// failures included) joins the conversation as `role: tool` messages with
/// the matching `tool_call_id`, plus one [`AgentStep`] per call. Malformed
/// `arguments` JSON never reaches the executor: it becomes a `Failed`
/// outcome for the model. Unknown tool names are `Denied` without calling
/// anything. After `max_rounds` tool-using turns with the model still
/// asking for tools, one final `tool_choice: "none"` call (plus a nudge
/// asking for the best answer so far) closes the turn and sets
/// `round_limit_reached`. `stop` is checked before every request and every
/// tool call and returns `Err("detenido")` without running more tools.
/// `rounds_used` counts the tool-allowed model turns; the final no-tools
/// call is extra and uncounted.
pub(crate) async fn run_agent<T, E>(
    transport: &T,
    executor: &E,
    request: AgentRequest,
    stop: &AtomicBool,
    mut on_step: impl FnMut(AgentStep),
) -> Result<AgentResult, String>
where
    T: ChatTransport,
    E: HelpToolExecutor,
{
    if stop.load(Ordering::Relaxed) {
        return Err(STOP_MESSAGE.to_string());
    }
    let tools = executor.tools();
    let mut messages: Vec<ChatMessage> = Vec::new();
    if !request.system.trim().is_empty() {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: Some(request.system.clone()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        });
    }
    for (role, content) in &request.history {
        messages.push(ChatMessage {
            role: role.clone(),
            content: Some(content.clone()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        });
    }
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: Some(request.question.clone()),
        tool_calls: Vec::new(),
        tool_call_id: None,
    });

    let mut steps: Vec<AgentStep> = Vec::new();
    if request.max_rounds == 0 {
        if stop.load(Ordering::Relaxed) {
            return Err(STOP_MESSAGE.to_string());
        }
        let closing = ChatRequest {
            model: request.model.clone(),
            messages: messages.clone(),
            tools: tools.clone(),
            tool_choice: TOOL_CHOICE_NONE.to_string(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
        };
        let closing_response = transport.send(closing).await?;
        return Ok(AgentResult {
            answer: closing_response.content.unwrap_or_default(),
            steps,
            rounds_used: 0,
            round_limit_reached: false,
        });
    }

    let mut rounds_used = 0usize;
    for _ in 0..request.max_rounds {
        if stop.load(Ordering::Relaxed) {
            return Err(STOP_MESSAGE.to_string());
        }
        let outgoing = ChatRequest {
            model: request.model.clone(),
            messages: messages.clone(),
            tools: tools.clone(),
            tool_choice: TOOL_CHOICE_AUTO.to_string(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
        };
        let response = transport.send(outgoing).await?;
        rounds_used += 1;
        if response.tool_calls.is_empty() {
            return Ok(AgentResult {
                answer: response.content.unwrap_or_default(),
                steps,
                rounds_used,
                round_limit_reached: false,
            });
        }
        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: response.content.clone(),
            tool_calls: response.tool_calls.clone(),
            tool_call_id: None,
        });
        for tool_call in &response.tool_calls {
            if stop.load(Ordering::Relaxed) {
                return Err(STOP_MESSAGE.to_string());
            }
            let parsed: Result<serde_json::Value, serde_json::Error> =
                serde_json::from_str(&tool_call.arguments);
            let (outcome_text, status, approval_id, target, preview) = match parsed {
                Err(error) => {
                    let preview: String = tool_call
                        .arguments
                        .chars()
                        .take(ARGS_PREVIEW_MAX_CHARS)
                        .collect();
                    let detail = format!("Argumentos inválidos para '{}': {error}", tool_call.name);
                    (
                        format!("La herramienta falló: {detail}"),
                        AgentStepStatus::Failed,
                        None,
                        None,
                        preview,
                    )
                }
                Ok(value) => {
                    let preview = compact_args_preview(&value);
                    if !tools.iter().any(|spec| spec.name == tool_call.name) {
                        let reason = format!("herramienta desconocida '{}'", tool_call.name);
                        (
                            format!("Herramienta denegada: {reason}"),
                            AgentStepStatus::Denied,
                            None,
                            None,
                            preview,
                        )
                    } else {
                        // No timeout on purpose: an owner approval (T22)
                        // may take as long as the person needs.
                        match executor.call(&tool_call.name, value).await {
                            ToolOutcome::Ok { content } => {
                                (content, AgentStepStatus::Ok, None, None, preview)
                            }
                            // A pending MCP approval (T26): the model sees
                            // the pending JSON, the panel follows the id.
                            ToolOutcome::Pending {
                                content,
                                approval_id,
                                target,
                            } => (
                                content,
                                AgentStepStatus::Pending,
                                Some(approval_id),
                                target,
                                preview,
                            ),
                            ToolOutcome::Denied { reason } => (
                                format!("Herramienta denegada: {reason}"),
                                AgentStepStatus::Denied,
                                None,
                                None,
                                preview,
                            ),
                            ToolOutcome::Failed { message } => (
                                format!("La herramienta falló: {message}"),
                                AgentStepStatus::Failed,
                                None,
                                None,
                                preview,
                            ),
                        }
                    }
                }
            };
            let step = AgentStep {
                tool: tool_call.name.clone(),
                args: preview,
                status,
                approval_id,
                target,
            };
            steps.push(step.clone());
            on_step(step);
            messages.push(ChatMessage {
                role: "tool".to_string(),
                content: Some(outcome_text),
                tool_calls: Vec::new(),
                tool_call_id: Some(tool_call.id.clone()),
            });
        }
    }

    if stop.load(Ordering::Relaxed) {
        return Err(STOP_MESSAGE.to_string());
    }
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: Some(FINAL_ANSWER_NUDGE.to_string()),
        tool_calls: Vec::new(),
        tool_call_id: None,
    });
    let closing = ChatRequest {
        model: request.model.clone(),
        messages: messages.clone(),
        tools: tools.clone(),
        tool_choice: TOOL_CHOICE_NONE.to_string(),
        max_tokens: request.max_tokens,
        temperature: request.temperature,
    };
    let closing_response = transport.send(closing).await?;
    Ok(AgentResult {
        answer: closing_response.content.unwrap_or_default(),
        steps,
        rounds_used,
        round_limit_reached: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

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

        fn recorded(&self) -> Vec<ChatRequest> {
            self.recorded.lock().expect("lock").clone()
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

    struct MockExecutor {
        specs: Vec<ToolSpec>,
        outcomes: Mutex<HashMap<String, ToolOutcome>>,
        calls: Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl MockExecutor {
        fn new(specs: Vec<ToolSpec>) -> Self {
            Self {
                specs,
                outcomes: Mutex::new(HashMap::new()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn with_outcome(self, name: &str, outcome: ToolOutcome) -> Self {
            self.outcomes
                .lock()
                .expect("lock")
                .insert(name.to_string(), outcome);
            self
        }

        fn calls(&self) -> Vec<(String, serde_json::Value)> {
            self.calls.lock().expect("lock").clone()
        }
    }

    impl HelpToolExecutor for MockExecutor {
        fn tools(&self) -> Vec<ToolSpec> {
            self.specs.clone()
        }

        async fn call(&self, name: &str, args: serde_json::Value) -> ToolOutcome {
            self.calls
                .lock()
                .expect("lock")
                .push((name.to_string(), args.clone()));
            self.outcomes
                .lock()
                .expect("lock")
                .get(name)
                .cloned()
                .unwrap_or(ToolOutcome::Ok {
                    content: format!("eco de {name}"),
                })
        }
    }

    fn tool(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.to_string(),
            description: format!("hace {name}"),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn answer(content: &str) -> ChatResponse {
        ChatResponse {
            content: Some(content.to_string()),
            tool_calls: Vec::new(),
        }
    }

    fn tool_response(calls: Vec<ToolCall>) -> ChatResponse {
        ChatResponse {
            content: None,
            tool_calls: calls,
        }
    }

    fn call(id: &str, name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args.to_string(),
        }
    }

    fn agent_request(max_rounds: usize) -> AgentRequest {
        AgentRequest {
            model: "vendor/modelo".to_string(),
            system: "sos ayuda".to_string(),
            history: Vec::new(),
            question: "¿cómo sigo?".to_string(),
            max_rounds,
            max_tokens: 200,
            temperature: 0.2,
        }
    }

    #[tokio::test]
    async fn plain_answer_returns_in_one_round() {
        let transport = MockTransport::new(vec![answer("listo")]);
        let executor = MockExecutor::new(vec![tool("leer")]);
        let stop = AtomicBool::new(false);
        let mut steps = Vec::new();
        let result = run_agent(&transport, &executor, agent_request(3), &stop, |step| {
            steps.push(step)
        })
        .await
        .expect("debe responder");
        assert_eq!(result.answer, "listo");
        assert!(result.steps.is_empty());
        assert_eq!(result.rounds_used, 1);
        assert!(!result.round_limit_reached);
        assert!(executor.calls().is_empty());
        assert_eq!(transport.recorded().len(), 1);
    }

    #[tokio::test]
    async fn one_tool_call_then_answer_carries_id_and_content() {
        let transport = MockTransport::new(vec![
            tool_response(vec![call("id-1", "leer", r#"{"ruta":"a"}"#)]),
            answer("hecho"),
        ]);
        let executor = MockExecutor::new(vec![tool("leer")]).with_outcome(
            "leer",
            ToolOutcome::Ok {
                content: "texto leído".to_string(),
            },
        );
        let stop = AtomicBool::new(false);
        let mut steps = Vec::new();
        let result = run_agent(&transport, &executor, agent_request(3), &stop, |step| {
            steps.push(step)
        })
        .await
        .expect("debe responder");
        assert_eq!(result.answer, "hecho");
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].tool, "leer");
        assert_eq!(result.steps[0].status, AgentStepStatus::Ok);
        let calls = executor.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "leer");
        let recorded = transport.recorded();
        assert_eq!(recorded.len(), 2);
        let tool_messages: Vec<_> = recorded[1]
            .messages
            .iter()
            .filter(|message| message.role == "tool")
            .collect();
        assert_eq!(tool_messages.len(), 1);
        assert_eq!(
            tool_messages[0].tool_call_id.as_deref(),
            Some("id-1"),
            "el mensaje tool lleva el tool_call_id"
        );
        assert_eq!(
            tool_messages[0].content.as_deref(),
            Some("texto leído"),
            "el mensaje tool lleva el contenido"
        );
    }

    #[tokio::test]
    async fn two_tool_calls_in_one_turn_run_in_order() {
        let transport = MockTransport::new(vec![
            tool_response(vec![
                call("id-1", "a", r#"{"n":1}"#),
                call("id-2", "b", r#"{"n":2}"#),
            ]),
            answer("fin"),
        ]);
        let executor = MockExecutor::new(vec![tool("a"), tool("b")]);
        let stop = AtomicBool::new(false);
        let mut steps = Vec::new();
        let result = run_agent(&transport, &executor, agent_request(3), &stop, |step| {
            steps.push(step)
        })
        .await
        .expect("debe responder");
        assert_eq!(result.answer, "fin");
        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.steps[0].tool, "a");
        assert_eq!(result.steps[1].tool, "b");
        let calls = executor.calls();
        assert_eq!(
            calls
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>(),
            vec!["a".to_string(), "b".to_string()],
            "las llamadas corren en orden"
        );
    }

    #[tokio::test]
    async fn malformed_arguments_become_failed_and_continue() {
        let transport = MockTransport::new(vec![
            tool_response(vec![call("id-9", "leer", "{no json")]),
            answer("sigo igual"),
        ]);
        let executor = MockExecutor::new(vec![tool("leer")]);
        let stop = AtomicBool::new(false);
        let mut steps = Vec::new();
        let result = run_agent(&transport, &executor, agent_request(3), &stop, |step| {
            steps.push(step)
        })
        .await
        .expect("debe responder");
        assert_eq!(result.answer, "sigo igual");
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].status, AgentStepStatus::Failed);
        assert!(
            executor.calls().is_empty(),
            "argumentos rotos no llegan al ejecutor"
        );
        let recorded = transport.recorded();
        assert_eq!(recorded.len(), 2);
        let tool_text = recorded[1]
            .messages
            .iter()
            .find(|message| message.role == "tool")
            .and_then(|message| message.content.clone())
            .unwrap_or_default();
        assert!(
            !tool_text.trim().is_empty(),
            "el fallo vuelve al modelo como texto"
        );
    }

    #[tokio::test]
    async fn unknown_tool_becomes_denied() {
        let transport = MockTransport::new(vec![
            tool_response(vec![call("id-x", "inexistente", r#"{}"#)]),
            answer("sin herramienta"),
        ]);
        let executor = MockExecutor::new(vec![tool("leer")]);
        let stop = AtomicBool::new(false);
        let mut steps = Vec::new();
        let result = run_agent(&transport, &executor, agent_request(3), &stop, |step| {
            steps.push(step)
        })
        .await
        .expect("debe responder");
        assert_eq!(result.answer, "sin herramienta");
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].status, AgentStepStatus::Denied);
        assert!(
            executor.calls().is_empty(),
            "una herramienta desconocida no se ejecuta"
        );
    }

    #[tokio::test]
    async fn round_cap_requests_a_final_answer_without_tools() {
        let transport = MockTransport::new(vec![
            tool_response(vec![call("id-1", "leer", r#"{}"#)]),
            answer("lo mejor con lo que hay"),
        ]);
        let executor = MockExecutor::new(vec![tool("leer")]).with_outcome(
            "leer",
            ToolOutcome::Ok {
                content: "algo".to_string(),
            },
        );
        let stop = AtomicBool::new(false);
        let mut steps = Vec::new();
        let result = run_agent(&transport, &executor, agent_request(1), &stop, |step| {
            steps.push(step)
        })
        .await
        .expect("debe responder");
        assert_eq!(result.answer, "lo mejor con lo que hay");
        assert!(result.round_limit_reached);
        assert_eq!(result.rounds_used, 1);
        let recorded = transport.recorded();
        assert_eq!(
            recorded.len(),
            2,
            "una ronda con tools y un cierre sin tools"
        );
        assert_eq!(recorded[0].tool_choice, TOOL_CHOICE_AUTO);
        assert_eq!(
            recorded[1].tool_choice, TOOL_CHOICE_NONE,
            "el cierre pide la respuesta sin herramientas"
        );
    }

    #[tokio::test]
    async fn stop_flag_halts_before_more_calls() {
        let transport = MockTransport::new(vec![
            tool_response(vec![call("id-1", "a", r#"{}"#), call("id-2", "b", r#"{}"#)]),
            answer("nunca"),
        ]);
        let executor = MockExecutor::new(vec![tool("a"), tool("b")]);
        let stop = AtomicBool::new(false);
        let mut steps = Vec::new();
        let result = run_agent(&transport, &executor, agent_request(3), &stop, |step| {
            steps.push(step);
            if steps.len() == 1 {
                stop.store(true, Ordering::Relaxed);
            }
        })
        .await;
        assert_eq!(result.unwrap_err(), "detenido");
        assert_eq!(executor.calls().len(), 1, "sin más llamadas tras detener");
    }

    #[test]
    fn request_json_shape_carries_tools_and_choice() {
        let request = ChatRequest {
            model: "vendor/modelo".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some("hola".to_string()),
                tool_calls: Vec::new(),
                tool_call_id: None,
            }],
            tools: vec![tool("leer")],
            tool_choice: TOOL_CHOICE_AUTO.to_string(),
            max_tokens: 200,
            temperature: 0.2,
        };
        let body = chat_request_body(&request);
        let tools = body
            .get("tools")
            .and_then(|value| value.as_array())
            .expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(
            tools[0].get("type").and_then(|value| value.as_str()),
            Some("function")
        );
        assert_eq!(
            tools[0]
                .pointer("/function/name")
                .and_then(|value| value.as_str()),
            Some("leer")
        );
        assert!(
            tools[0].pointer("/function/description").is_some(),
            "la descripción viaja para el modelo"
        );
        assert!(
            tools[0].pointer("/function/parameters").is_some(),
            "el esquema viaja como parameters"
        );
        assert_eq!(
            body.get("tool_choice").and_then(|value| value.as_str()),
            Some("auto")
        );
        assert_eq!(
            body.get("model").and_then(|value| value.as_str()),
            Some("vendor/modelo")
        );
    }

    #[test]
    fn http_status_mapping_is_spanish_and_names_the_status() {
        for (status, hint) in [
            (401u16, "clave"),
            (402u16, "crédito"),
            (429u16, "límite"),
            (500u16, "servicio"),
            (503u16, "servicio"),
        ] {
            let message = openrouter_status_message(status, "detalle");
            assert!(message.contains(&status.to_string()), "{status}: {message}");
            assert!(
                message.to_lowercase().contains(hint),
                "{status} debe mencionar {hint:?}: {message}"
            );
        }
    }

    #[tokio::test]
    async fn executor_denied_and_failed_travel_back_as_readable_text() {
        // The executor's own Denied/Failed (owner rejected, tool broke)
        // must reach the model as plain `role: tool` text, like the
        // unknown/malformed cases, and mark the steps accordingly.
        let transport = MockTransport::new(vec![
            tool_response(vec![
                call("id-1", "cambiar", r#"{}"#),
                call("id-2", "leer", r#"{}"#),
            ]),
            answer("entendido"),
        ]);
        let executor = MockExecutor::new(vec![tool("cambiar"), tool("leer")])
            .with_outcome(
                "cambiar",
                ToolOutcome::Denied {
                    reason: "el dueño lo rechazó".to_string(),
                },
            )
            .with_outcome(
                "leer",
                ToolOutcome::Failed {
                    message: "se rompió".to_string(),
                },
            );
        let stop = AtomicBool::new(false);
        let mut steps = Vec::new();
        let result = run_agent(&transport, &executor, agent_request(3), &stop, |step| {
            steps.push(step)
        })
        .await
        .expect("debe responder");
        assert_eq!(result.answer, "entendido");
        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.steps[0].status, AgentStepStatus::Denied);
        assert_eq!(result.steps[1].status, AgentStepStatus::Failed);
        let recorded = transport.recorded();
        assert_eq!(recorded.len(), 2);
        let texts: Vec<String> = recorded[1]
            .messages
            .iter()
            .filter(|message| message.role == "tool")
            .filter_map(|message| message.content.clone())
            .collect();
        assert_eq!(texts.len(), 2);
        assert!(
            texts[0].contains("el dueño lo rechazó"),
            "el rechazo vuelve al modelo: {}",
            texts[0]
        );
        assert!(
            texts[1].contains("se rompió"),
            "el fallo vuelve al modelo: {}",
            texts[1]
        );
    }

    #[tokio::test]
    async fn pending_approvals_travel_as_pending_steps_with_id_and_target() {
        // T26: an MCP approval waiting for the owner marks the step
        // pending (not ok), keeps the id for the outcome watch and the
        // human target for the label. The model still sees the pending
        // JSON as the tool text.
        let pending = serde_json::json!({
            "status": "pending",
            "id": "apr_1",
        })
        .to_string();
        let transport = MockTransport::new(vec![
            tool_response(vec![call("id-1", "borrar", r#"{"collectionId":"col_1"}"#)]),
            answer("listo"),
        ]);
        let executor = MockExecutor::new(vec![tool("borrar")]).with_outcome(
            "borrar",
            ToolOutcome::Pending {
                content: pending.clone(),
                approval_id: "apr_1".to_string(),
                target: Some("SIMAPE".to_string()),
            },
        );
        let stop = AtomicBool::new(false);
        let mut steps = Vec::new();
        let result = run_agent(&transport, &executor, agent_request(3), &stop, |step| {
            steps.push(step)
        })
        .await
        .expect("debe responder");
        assert_eq!(result.steps.len(), 1);
        assert_eq!(result.steps[0].status, AgentStepStatus::Pending);
        assert_eq!(result.steps[0].approval_id, Some("apr_1".to_string()));
        assert_eq!(result.steps[0].target, Some("SIMAPE".to_string()));
        let recorded = transport.recorded();
        let text = recorded[1]
            .messages
            .iter()
            .find(|message| message.role == "tool")
            .and_then(|message| message.content.clone())
            .unwrap_or_default();
        assert!(
            text.contains("apr_1"),
            "the pending id reaches the model: {text}"
        );
    }

    #[test]
    fn args_preview_is_compact_and_capped() {
        let big = serde_json::json!({"texto": "x".repeat(500)});
        let preview = compact_args_preview(&big);
        assert!(preview.chars().count() <= ARGS_PREVIEW_MAX_CHARS);
        assert!(!preview.contains('\n'), "compacto, sin saltos: {preview}");
    }

    #[test]
    fn wire_response_parses_content_and_raw_tool_calls() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "hola",
                    "tool_calls": [{
                        "id": "id-7",
                        "function": {"name": "leer", "arguments": r#"{"a":1}"#},
                    }],
                },
            }],
        })
        .to_string();
        let parsed = parse_chat_response(&body).expect("parse");
        assert_eq!(parsed.content.as_deref(), Some("hola"));
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].id, "id-7");
        assert_eq!(parsed.tool_calls[0].name, "leer");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&parsed.tool_calls[0].arguments)
                .expect("args json")["a"],
            1
        );
    }
}
