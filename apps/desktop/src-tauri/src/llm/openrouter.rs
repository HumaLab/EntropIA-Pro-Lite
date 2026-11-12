use std::{io::Cursor, path::Path, sync::RwLock, time::Duration};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use image::GenericImageView;
use serde::{Deserialize, Serialize};

use super::generation::{OpenRouterGenerationParams, ReasoningConfig};

// ---------------------------------------------------------------------------
// Fallback provider (T-45)
// ---------------------------------------------------------------------------

/// Vercel AI Gateway: OpenAI-compatible, with the same `provider/model` ids.
const VERCEL_AI_GATEWAY_URL: &str = "https://ai-gateway.vercel.sh/v1/chat/completions";

/// The Vercel AI Gateway key, when the user stored one. Kept here because the
/// clients are built in many places with only the OpenRouter key; settings
/// refresh it on startup and whenever the key is saved or deleted.
static FALLBACK_KEY: RwLock<Option<String>> = RwLock::new(None);

pub fn set_fallback_key(key: Option<String>) {
    let key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    *FALLBACK_KEY.write().unwrap_or_else(|p| p.into_inner()) = key;
}

fn fallback_key() -> Option<String> {
    FALLBACK_KEY
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

/// OpenRouter-only variant suffixes (`:free`, `:nitro`) mean nothing to the
/// gateway; the `provider/model` part is shared.
fn fallback_model(model: &str) -> String {
    model.split(':').next().unwrap_or(model).to_string()
}

// ---------------------------------------------------------------------------
// OpenRouter API types
// ---------------------------------------------------------------------------

// `Clone` on this serialization-only chain exists so `send_request` can hold the
// built request across a retry attempt. It is never used to share state.
#[derive(Serialize, Clone)]
struct ChatMessage {
    role: String,
    content: ChatMessageContent,
}

#[derive(Serialize, Clone)]
#[serde(untagged)]
enum ChatMessageContent {
    Text(String),
    Parts(Vec<ChatContentPart>),
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChatContentPart {
    Text { text: String },
    ImageUrl { image_url: ChatImageUrl },
}

#[derive(Serialize, Clone)]
struct ChatImageUrl {
    url: String,
    detail: &'static str,
}

#[derive(Serialize, Clone)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: i32,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
}

/// A single failed generation attempt, classified by whether repeating it is
/// safe.
#[derive(Debug)]
enum RequestFailure {
    /// Connection-level failure (DNS, TCP, TLS) or a timeout. The provider may
    /// never have received the request, so repeating it is safe.
    Transport { message: String, timeout: bool },
    /// The provider answered with a non-success status.
    Status {
        status: reqwest::StatusCode,
        body: String,
    },
    /// The provider answered and we consumed the body. NEVER retried: the
    /// completion may already have been generated and billed.
    Payload(String),
}

impl RequestFailure {
    fn from_transport(error: reqwest::Error) -> Self {
        Self::Transport {
            timeout: error.is_timeout(),
            message: crate::nlp::embeddings::error_chain(&error),
        }
    }

    /// Only transport failures and provider-side 5xx are repeatable. A 4xx is a
    /// deterministic rejection (bad key, bad model, context too long) and
    /// retrying it just doubles the latency before the same error.
    fn is_retryable(&self) -> bool {
        match self {
            Self::Transport { .. } => true,
            Self::Status { status, .. } => status.is_server_error(),
            Self::Payload(_) => false,
        }
    }

    /// Whether OpenRouter itself is the problem, so another provider may
    /// answer: unreachable, 5xx, rate-limited (429) or out of credits (402).
    /// Other 4xx (bad model, prompt too long) would fail there too.
    fn should_fall_back(&self) -> bool {
        match self {
            Self::Transport { .. } => true,
            Self::Status { status, .. } => {
                status.is_server_error() || matches!(status.as_u16(), 402 | 429)
            }
            Self::Payload(_) => false,
        }
    }

    fn message(&self) -> String {
        match self {
            // Distinguishable on purpose: a timeout is actionable by the user
            // (shorter prompt, faster model) in a way a generic failure is not.
            Self::Transport {
                message,
                timeout: true,
            } => format!(
                "OpenRouter request timed out after {}s: {message}",
                OpenRouterClient::REQUEST_TIMEOUT.as_secs()
            ),
            Self::Transport {
                message,
                timeout: false,
            } => format!("OpenRouter request failed: {message}"),
            Self::Status { status, body } => format!("OpenRouter API error ({status}): {body}"),
            Self::Payload(message) => message.clone(),
        }
    }
}

/// Whether this failure means the model refused the `reasoning` override, so
/// the same request can be resent once without it. Some models make reasoning
/// mandatory and answer `{"effort":"none"}` with a `400` that names it; no
/// other `400` is worth repeating, hence the body check.
fn should_drop_reasoning(failure: &RequestFailure, request: &ChatCompletionRequest) -> bool {
    request.reasoning.is_some()
        && matches!(
            failure,
            RequestFailure::Status { status, body }
                if *status == reqwest::StatusCode::BAD_REQUEST
                    && body.to_ascii_lowercase().contains("reasoning")
        )
}

/// The success body, read leniently: OpenRouter sends `200 OK` before the
/// model produces a token, so a provider failure after that point arrives as
/// a `200` whose body holds an `error` (top-level or on the choice) and maybe
/// no `choices` or a `null` content.
#[derive(Deserialize)]
struct ChatCompletionBody {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    error: Option<ProviderError>,
}

#[derive(Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: Option<ChatResponseMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    error: Option<ProviderError>,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct ProviderError {
    #[serde(default)]
    code: Option<serde_json::Value>,
    #[serde(default)]
    message: Option<String>,
}

impl ProviderError {
    fn describe(&self) -> String {
        let code = match &self.code {
            Some(serde_json::Value::String(code)) => code.clone(),
            Some(serde_json::Value::Null) | None => "unknown".to_string(),
            Some(other) => other.to_string(),
        };
        let message = self
            .message
            .as_deref()
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .unwrap_or("no message");
        format!("OpenRouter provider error ({code}): {message}")
    }
}

/// Longest stretch of an unreadable body quoted in the error, so the log
/// shows what came back without dumping a whole completion.
const BODY_EXCERPT_CHARS: usize = 300;

/// Reads the answer out of a `200` body, turning every way it can carry no
/// answer into a failure that says why.
fn parse_chat_completion_body(body: &str) -> Result<String, RequestFailure> {
    let parsed: ChatCompletionBody = serde_json::from_str(body).map_err(|error| {
        let excerpt: String = body.trim().chars().take(BODY_EXCERPT_CHARS).collect();
        RequestFailure::Payload(format!(
            "Failed to parse OpenRouter response: {error}; body: {excerpt}"
        ))
    })?;
    if let Some(error) = parsed.error {
        return Err(RequestFailure::Payload(error.describe()));
    }
    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| RequestFailure::Payload("OpenRouter returned no choices".to_string()))?;
    if let Some(error) = choice.error {
        return Err(RequestFailure::Payload(error.describe()));
    }
    match choice.message.and_then(|message| message.content) {
        Some(content) => Ok(content.trim().to_string()),
        None => Err(RequestFailure::Payload(format!(
            "OpenRouter returned no content (finish_reason: {})",
            choice.finish_reason.as_deref().unwrap_or("unknown")
        ))),
    }
}

#[derive(Deserialize)]
struct OpenRouterModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub context_length: Option<u64>,
}

#[derive(Deserialize)]
struct OpenRouterModelsResponse {
    data: Vec<OpenRouterModel>,
}

#[derive(Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub context_length: u64,
}

const MAX_MULTIMODAL_IMAGE_BYTES: usize = 10_000_000;
const MAX_MULTIMODAL_SOURCE_BYTES: u64 = 50_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRouterImage {
    data_url: String,
    mime_type: String,
    payload_bytes: usize,
}

impl OpenRouterImage {
    #[cfg(test)]
    pub fn from_file(path: &str, app_data_dir: &Path) -> Result<Self, String> {
        let path = Path::new(path);
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            app_data_dir.join(path)
        };
        let canonical = crate::path_utils::ensure_within_dir(candidate, app_data_dir)
            .map_err(|error| format!("OCR correction image path is not allowed: {error}"))?;
        Self::from_validated_file(&canonical)
    }

    pub(crate) fn from_validated_file(path: &Path) -> Result<Self, String> {
        if !path.is_file() {
            return Err(format!(
                "OCR correction image is not a file: {}",
                path.display()
            ));
        }

        let size = std::fs::metadata(path)
            .map_err(|error| format!("Failed to inspect OCR correction image: {error}"))?
            .len();
        if size > MAX_MULTIMODAL_SOURCE_BYTES {
            return Err(format!(
                "OCR correction image exceeds the {MAX_MULTIMODAL_SOURCE_BYTES} byte source limit"
            ));
        }

        let raw = std::fs::read(path)
            .map_err(|error| format!("Failed to read OCR correction image: {error}"))?;
        // The model must see the photo the way the person does: upright. Only
        // an image carrying an orientation tag is re-encoded; `raw` is the
        // file as it is on disk.
        let bytes = crate::image_orient::oriented_bytes(raw)
            .map_err(|error| format!("Failed to orient OCR correction image: {error}"))?;
        Self::from_bytes(&bytes)
    }

    pub(crate) fn from_validated_ocr_source(path: &Path) -> Result<Self, String> {
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            return Self::from_validated_file(path);
        }

        if !path.is_file() {
            return Err(format!(
                "OCR correction PDF is not a file: {}",
                path.display()
            ));
        }

        let size = std::fs::metadata(path)
            .map_err(|error| format!("Failed to inspect OCR correction PDF: {error}"))?
            .len();
        if size > MAX_MULTIMODAL_SOURCE_BYTES {
            return Err(format!(
                "OCR correction PDF exceeds the {MAX_MULTIMODAL_SOURCE_BYTES} byte source limit"
            ));
        }

        let bytes = std::fs::read(path)
            .map_err(|error| format!("Failed to read OCR correction PDF: {error}"))?;
        let page_image = crate::ocr::render_pdf_first_page_for_ocr_correction(&bytes)
            .map_err(|error| format!("Failed to render OCR correction PDF: {error}"))?;
        Self::from_bytes(&page_image)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let format = image::guess_format(bytes)
            .map_err(|error| format!("Could not detect OCR correction image format: {error}"))?;
        let direct_mime = match format {
            image::ImageFormat::Png => Some("image/png"),
            image::ImageFormat::Jpeg => Some("image/jpeg"),
            image::ImageFormat::WebP => Some("image/webp"),
            image::ImageFormat::Gif => Some("image/gif"),
            _ => None,
        };

        let (mime, payload) = if bytes.len() <= MAX_MULTIMODAL_IMAGE_BYTES {
            if let Some(mime) = direct_mime {
                (mime, bytes.to_vec())
            } else {
                ("image/png", normalize_image_as_bounded_png(bytes)?)
            }
        } else {
            ("image/png", normalize_image_as_bounded_png(bytes)?)
        };

        let payload_bytes = payload.len();
        Ok(Self {
            data_url: format!("data:{mime};base64,{}", BASE64_STANDARD.encode(payload)),
            mime_type: mime.to_string(),
            payload_bytes,
        })
    }

    pub(crate) fn mime_type(&self) -> &str {
        &self.mime_type
    }

    pub(crate) fn payload_bytes(&self) -> usize {
        self.payload_bytes
    }
}

fn normalize_image_as_bounded_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut candidate = image::load_from_memory(bytes)
        .map_err(|error| format!("Failed to decode OCR correction image: {error}"))?;

    loop {
        let mut png = Vec::new();
        candidate
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|error| format!("Failed to encode OCR correction image as PNG: {error}"))?;
        if png.len() <= MAX_MULTIMODAL_IMAGE_BYTES {
            return Ok(png);
        }

        let (width, height) = candidate.dimensions();
        if width == 1 && height == 1 {
            return Err(format!(
                "OCR correction image exceeds the {MAX_MULTIMODAL_IMAGE_BYTES} byte request limit even at 1x1 pixels"
            ));
        }
        candidate = candidate.resize(
            (width / 2).max(1),
            (height / 2).max(1),
            image::imageops::FilterType::Triangle,
        );
    }
}

// ---------------------------------------------------------------------------
// OpenRouter client
// ---------------------------------------------------------------------------

const OPENROUTER_CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

pub struct OpenRouterClient {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl OpenRouterClient {
    pub const DEFAULT_CONTEXT_WINDOW: u32 = 128_000;

    /// Total budget for one generation request. Generous, because a long RAG
    /// prompt against a slow model legitimately takes a while — but bounded, so
    /// a stalled connection cannot hang the chat forever (the frontend has no
    /// cancellation yet).
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
    /// Connect phase only. A provider that never completes the TCP/TLS
    /// handshake should fail fast instead of burning the full request budget.
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
    /// One extra attempt, and only for failures that are safe to repeat.
    const MAX_ATTEMPTS: u32 = 2;

    pub fn new(api_key: String, model: String) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(crate::USER_AGENT)
            .timeout(Self::REQUEST_TIMEOUT)
            .connect_timeout(Self::CONNECT_TIMEOUT)
            .build()
            .expect("Failed to build reqwest client");
        Self {
            client,
            api_key,
            model,
        }
    }

    /// Generate a completion from the prompt text.
    /// The prompt should be the raw instruction text (NOT wrapped in Gemma format).
    pub async fn generate(
        &self,
        prompt: &str,
        params: &OpenRouterGenerationParams,
    ) -> Result<String, String> {
        self.send_request(self.build_request(prompt, params)).await
    }

    pub async fn generate_with_image(
        &self,
        prompt: &str,
        image: &OpenRouterImage,
        params: &OpenRouterGenerationParams,
    ) -> Result<String, String> {
        self.send_request(self.build_multimodal_request(prompt, image, params))
            .await
    }

    /// When OpenRouter fails for reasons of its own and a Vercel AI Gateway key
    /// is stored, the same request goes there once (T-45).
    async fn send_request(&self, request: ChatCompletionRequest) -> Result<String, String> {
        let failure = match self.send_to_openrouter(&request).await {
            Ok(answer) => return Ok(answer),
            Err(failure) => failure,
        };
        let Some(key) = fallback_key().filter(|_| failure.should_fall_back()) else {
            return Err(failure.message());
        };
        log::warn!(
            target: "openrouter",
            "OpenRouter failed ({}); trying the Vercel AI Gateway",
            failure.message()
        );
        let mut fallback = request;
        fallback.model = fallback_model(&fallback.model);
        // `reasoning` is an OpenRouter extension: the gateway gets the request
        // as it was before NER started asking for the override.
        fallback.reasoning = None;
        self.try_send_request(VERCEL_AI_GATEWAY_URL, &key, &fallback)
            .await
            .map_err(|second| {
                format!(
                    "{} — Vercel AI Gateway also failed: {}",
                    failure.message(),
                    second.message()
                )
            })
    }

    /// Sends the request to OpenRouter, retrying at most once and only for
    /// failures that are safe to repeat (see [`RequestFailure::is_retryable`]).
    /// A request carrying a `reasoning` override is also resent once without it
    /// when the model rejects the override (see [`should_drop_reasoning`]).
    async fn send_to_openrouter(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<String, RequestFailure> {
        // A copy, so a rejected `reasoning` override can be removed for the
        // resend without losing what the caller built.
        let mut current = request.clone();
        let mut attempt = 1;
        loop {
            match self
                .try_send_request(OPENROUTER_CHAT_URL, &self.api_key, &current)
                .await
            {
                Ok(answer) => return Ok(answer),
                Err(failure) => {
                    // The resend is a different request (no override), so it
                    // does not spend the retry budget: a rejected override
                    // never bills and the situation is trivially fixable.
                    // Clearing the override also makes this branch one-shot.
                    if should_drop_reasoning(&failure, &current) {
                        log::warn!(
                            target: "openrouter",
                            "attempt {attempt} rejected the reasoning override ({}); resending without it",
                            failure.message()
                        );
                        current.reasoning = None;
                        continue;
                    }
                    if attempt >= Self::MAX_ATTEMPTS || !failure.is_retryable() {
                        return Err(failure);
                    }
                    log::warn!(
                        target: "openrouter",
                        "attempt {attempt} failed ({}); retrying once",
                        failure.message()
                    );
                    attempt += 1;
                }
            }
        }
    }

    async fn try_send_request(
        &self,
        url: &str,
        api_key: &str,
        request: &ChatCompletionRequest,
    ) -> Result<String, RequestFailure> {
        let response = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("HTTP-Referer", "https://hlab.com.ar/")
            .header("X-Title", "EntropIA")
            .json(request)
            .send()
            .await
            .map_err(RequestFailure::from_transport)?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(RequestFailure::Status { status, body });
        }

        // Read the body as text first: a decode error must be able to say
        // what actually came back.
        let body = response.text().await.map_err(|error| {
            RequestFailure::Payload(format!(
                "Failed to read OpenRouter response: {}",
                crate::nlp::embeddings::error_chain(&error)
            ))
        })?;
        parse_chat_completion_body(&body)
    }

    fn build_request(
        &self,
        prompt: &str,
        params: &OpenRouterGenerationParams,
    ) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text(prompt.to_string()),
            }],
            max_tokens: params.max_tokens,
            temperature: params.temperature,
            top_p: params.top_p,
            top_k: params.top_k,
            presence_penalty: params.presence_penalty,
            frequency_penalty: params.frequency_penalty,
            stop: params.stop.clone(),
            reasoning: params.reasoning,
        }
    }

    fn build_multimodal_request(
        &self,
        prompt: &str,
        image: &OpenRouterImage,
        params: &OpenRouterGenerationParams,
    ) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Parts(vec![
                    ChatContentPart::Text {
                        text: prompt.to_string(),
                    },
                    ChatContentPart::ImageUrl {
                        image_url: ChatImageUrl {
                            url: image.data_url.clone(),
                            detail: "high",
                        },
                    },
                ]),
            }],
            max_tokens: params.max_tokens,
            temperature: params.temperature,
            top_p: params.top_p,
            top_k: params.top_k,
            presence_penalty: params.presence_penalty,
            frequency_penalty: params.frequency_penalty,
            stop: params.stop.clone(),
            reasoning: params.reasoning,
        }
    }

    /// Tests the stored Vercel AI Gateway key with a tiny paid request: the
    /// gateway's model list answers without a key, and a key on an account
    /// without a card on file only fails once it is asked to generate.
    pub async fn test_fallback_connection() -> Result<(), String> {
        let key = fallback_key().ok_or("No hay clave de Vercel guardada")?;
        let request = ChatCompletionRequest {
            model: "openai/gpt-4o-mini".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("ping".to_string()),
            }],
            // The gateway rejects fewer than 16 output tokens.
            max_tokens: 16,
            temperature: 0.0,
            top_p: None,
            top_k: None,
            presence_penalty: None,
            frequency_penalty: None,
            stop: Vec::new(),
            reasoning: None,
        };
        Self::new(String::new(), String::new())
            .try_send_request(VERCEL_AI_GATEWAY_URL, &key, &request)
            .await
            .map(|_| ())
            .map_err(|failure| match failure {
                RequestFailure::Status { status, body } => {
                    format!("Vercel AI Gateway ({status}): {body}")
                }
                other => other.message().replace("OpenRouter", "Vercel AI Gateway"),
            })
    }

    /// Test the connection by listing available models.
    /// Returns Ok with a list of model IDs on success, Err on failure.
    pub async fn test_connection(&self) -> Result<Vec<ModelInfo>, String> {
        let response = self
            .client
            .get("https://openrouter.ai/api/v1/models")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| {
                format!(
                    "OpenRouter connection test failed: {}",
                    crate::nlp::embeddings::error_chain(&e)
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("OpenRouter API error ({status}): {body}"));
        }

        let parsed: OpenRouterModelsResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse OpenRouter models response: {e}"))?;

        Ok(parsed
            .data
            .into_iter()
            .map(|m| ModelInfo {
                id: m.id,
                name: m.name,
                context_length: m.context_length.unwrap_or(4096),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::generation::{generation_config_from_settings, GenerationFlow};

    fn settings_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        conn
    }

    // ── Reading a 200 body ───────────────────────────────────────────────────

    fn payload_message(result: Result<String, RequestFailure>) -> String {
        match result {
            Err(RequestFailure::Payload(message)) => message,
            Err(other) => panic!("expected a payload failure, got {other:?}"),
            Ok(answer) => panic!("expected a failure, got the answer {answer:?}"),
        }
    }

    #[test]
    fn a_completion_body_yields_its_trimmed_content() {
        let body = r#"{"id":"gen-1","choices":[{"message":{"role":"assistant","content":"  [] \n"},"finish_reason":"stop"}]}"#;
        assert_eq!(parse_chat_completion_body(body).unwrap(), "[]");
    }

    #[test]
    fn an_error_only_200_body_reports_the_provider_error() {
        // OpenRouter's documented shape when the provider fails after the
        // 200 went out: an `error` object and no `choices`.
        let body = r#"{"id":"gen-2","error":{"code":502,"message":"Provider disconnected","metadata":{"error_type":"provider_unavailable"}}}"#;
        assert_eq!(
            payload_message(parse_chat_completion_body(body)),
            "OpenRouter provider error (502): Provider disconnected"
        );
    }

    #[test]
    fn an_error_on_the_choice_reports_the_provider_error() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"[{\"value\":"},"finish_reason":"error","error":{"code":429,"message":"Rate limit exceeded"}}]}"#;
        assert_eq!(
            payload_message(parse_chat_completion_body(body)),
            "OpenRouter provider error (429): Rate limit exceeded"
        );
    }

    #[test]
    fn a_null_content_names_the_finish_reason() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":null},"finish_reason":"length"}]}"#;
        assert_eq!(
            payload_message(parse_chat_completion_body(body)),
            "OpenRouter returned no content (finish_reason: length)"
        );
    }

    #[test]
    fn an_empty_choice_list_keeps_its_legacy_message() {
        assert_eq!(
            payload_message(parse_chat_completion_body(r#"{"choices":[]}"#)),
            "OpenRouter returned no choices"
        );
    }

    #[test]
    fn an_unreadable_body_quotes_the_parse_error_and_a_bounded_excerpt() {
        let message = payload_message(parse_chat_completion_body("<html>Bad gateway</html>"));
        assert!(
            message.starts_with("Failed to parse OpenRouter response: expected value"),
            "unexpected: {message}"
        );
        assert!(
            message.ends_with("body: <html>Bad gateway</html>"),
            "unexpected: {message}"
        );

        let long = "x".repeat(BODY_EXCERPT_CHARS * 3);
        let message = payload_message(parse_chat_completion_body(&long));
        let excerpt = message.split("body: ").nth(1).unwrap();
        assert_eq!(excerpt.chars().count(), BODY_EXCERPT_CHARS);
    }

    // ── Timeout and retry classification (F5) ────────────────────────────────

    #[test]
    fn generation_client_is_built_with_a_bounded_request_budget() {
        // A stalled provider must not hang the chat forever: the frontend has
        // no cancellation, so the timeout is the only escape hatch.
        assert!(
            OpenRouterClient::REQUEST_TIMEOUT > Duration::ZERO,
            "generation requests must be time-bounded"
        );
        assert!(
            OpenRouterClient::CONNECT_TIMEOUT < OpenRouterClient::REQUEST_TIMEOUT,
            "connect timeout must fail faster than the total budget"
        );
        // Building must not panic with the timeouts configured.
        let _ = OpenRouterClient::new("secret".to_string(), "openai/test".to_string());
    }

    #[test]
    fn only_transport_and_server_errors_are_retried() {
        let transport = RequestFailure::Transport {
            message: "connection reset".to_string(),
            timeout: false,
        };
        let timeout = RequestFailure::Transport {
            message: "operation timed out".to_string(),
            timeout: true,
        };
        let server = RequestFailure::Status {
            status: reqwest::StatusCode::BAD_GATEWAY,
            body: String::new(),
        };
        let client_error = RequestFailure::Status {
            status: reqwest::StatusCode::UNAUTHORIZED,
            body: "bad key".to_string(),
        };
        let context_too_long = RequestFailure::Status {
            status: reqwest::StatusCode::BAD_REQUEST,
            body: "context length exceeded".to_string(),
        };
        let payload = RequestFailure::Payload("Failed to parse".to_string());

        assert!(transport.is_retryable());
        assert!(timeout.is_retryable());
        assert!(server.is_retryable());
        // A rejected key or an oversized prompt is deterministic — retrying
        // only doubles the wait before the identical error.
        assert!(!client_error.is_retryable());
        assert!(!context_too_long.is_retryable());
        // The completion may already have been billed; never send it twice.
        assert!(!payload.is_retryable());
    }

    #[test]
    fn only_openrouter_side_failures_fall_back_to_vercel() {
        let status = |code: u16| RequestFailure::Status {
            status: reqwest::StatusCode::from_u16(code).unwrap(),
            body: String::new(),
        };
        let transport = RequestFailure::Transport {
            message: "dns".to_string(),
            timeout: false,
        };
        assert!(transport.should_fall_back());
        assert!(status(502).should_fall_back());
        assert!(status(429).should_fall_back());
        assert!(status(402).should_fall_back());
        assert!(!status(400).should_fall_back());
        assert!(!status(401).should_fall_back());
        assert!(!RequestFailure::Payload("x".to_string()).should_fall_back());
    }

    #[test]
    fn the_fallback_model_drops_openrouter_variants() {
        assert_eq!(fallback_model("openai/gpt-4o-mini"), "openai/gpt-4o-mini");
        assert_eq!(
            fallback_model("meta-llama/llama-3.3-70b:free"),
            "meta-llama/llama-3.3-70b"
        );
    }

    #[test]
    fn a_blank_fallback_key_is_no_key() {
        set_fallback_key(Some("  ".to_string()));
        assert_eq!(fallback_key(), None);
    }

    #[test]
    fn retry_budget_allows_exactly_one_extra_attempt() {
        assert_eq!(OpenRouterClient::MAX_ATTEMPTS, 2);
    }

    #[test]
    fn timeout_failures_are_distinguishable_from_generic_transport_failures() {
        let timeout = RequestFailure::Transport {
            message: "operation timed out".to_string(),
            timeout: true,
        }
        .message();
        let generic = RequestFailure::Transport {
            message: "connection reset".to_string(),
            timeout: false,
        }
        .message();

        assert!(timeout.contains("timed out"), "unexpected: {timeout}");
        assert!(
            timeout.contains(&OpenRouterClient::REQUEST_TIMEOUT.as_secs().to_string()),
            "the message should tell the user the budget: {timeout}"
        );
        assert!(!generic.contains("timed out"), "unexpected: {generic}");
    }

    #[test]
    fn status_and_payload_failure_messages_keep_their_legacy_shape() {
        assert_eq!(
            RequestFailure::Status {
                status: reqwest::StatusCode::UNAUTHORIZED,
                body: "bad key".to_string(),
            }
            .message(),
            "OpenRouter API error (401 Unauthorized): bad key"
        );
        assert_eq!(
            RequestFailure::Payload("OpenRouter returned no choices".to_string()).message(),
            "OpenRouter returned no choices"
        );
    }

    #[test]
    fn serializes_all_user_editable_generation_parameters() {
        let client = OpenRouterClient::new("secret".to_string(), "openai/test".to_string());
        let params = OpenRouterGenerationParams {
            max_tokens: 4096,
            temperature: 0.1,
            top_p: Some(0.8),
            top_k: Some(25),
            presence_penalty: Some(0.2),
            frequency_penalty: Some(-0.3),
            stop: vec!["END".to_string(), "STOP".to_string()],
            reasoning: None,
        };

        let value = serde_json::to_value(client.build_request("prompt", &params)).unwrap();
        assert_eq!(value["model"], "openai/test");
        assert_eq!(value["max_tokens"], 4096);
        assert!((value["temperature"].as_f64().unwrap() - 0.1).abs() < 1e-6);
        assert!((value["top_p"].as_f64().unwrap() - 0.8).abs() < 1e-6);
        assert_eq!(value["top_k"], 25);
        assert!((value["presence_penalty"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert!((value["frequency_penalty"].as_f64().unwrap() + 0.3).abs() < 1e-6);
        assert_eq!(value["stop"], serde_json::json!(["END", "STOP"]));
    }

    #[test]
    fn omits_provider_default_optional_parameters() {
        let client = OpenRouterClient::new("secret".to_string(), "openai/test".to_string());
        let params = OpenRouterGenerationParams::provider_defaults(512, 0.3);

        let value = serde_json::to_value(client.build_request("prompt", &params)).unwrap();
        assert!(value.get("top_p").is_none());
        assert!(value.get("top_k").is_none());
        assert!(value.get("presence_penalty").is_none());
        assert!(value.get("frequency_penalty").is_none());
        assert!(value.get("stop").is_none());
    }

    // ── The NER reasoning override ───────────────────────────────────────────

    fn request_json(params: &OpenRouterGenerationParams) -> serde_json::Value {
        let client = OpenRouterClient::new(
            "secret".to_string(),
            "anthropic/claude-haiku-5.5".to_string(),
        );
        serde_json::to_value(client.build_request("prompt", params)).unwrap()
    }

    #[test]
    fn the_ner_flow_turns_the_model_reasoning_off() {
        // Reasoning is on by default for some models and its tokens bill
        // against `max_tokens`, which emptied the NER completion with
        // `finish_reason: "length"` and no content.
        let params = crate::nlp::ner::openrouter::ner_request_params(
            OpenRouterGenerationParams::provider_defaults(4096, 0.3),
        );

        let value = request_json(&params);
        assert_eq!(value["reasoning"], serde_json::json!({"effort": "none"}));
    }

    #[test]
    fn every_other_flow_sends_no_reasoning_key() {
        // A differential guarantee: RAG, titles, triples, summary and OCR
        // correction keep the exact request shape they had before.
        let conn = settings_conn();
        let mut requests = vec![request_json(
            &OpenRouterGenerationParams::provider_defaults(128, 0.0),
        )];
        for flow in [
            GenerationFlow::OcrCorrection,
            GenerationFlow::Summary,
            GenerationFlow::Ner,
            GenerationFlow::Triplets,
        ] {
            requests.push(request_json(
                &generation_config_from_settings(&conn, flow).params,
            ));
        }
        let image = OpenRouterImage {
            data_url: "data:image/png;base64,AQID".to_string(),
            mime_type: "image/png".to_string(),
            payload_bytes: 3,
        };
        requests.push(
            serde_json::to_value(
                OpenRouterClient::new("secret".to_string(), "vendor/ocr".to_string())
                    .build_multimodal_request(
                        "prompt",
                        &image,
                        &OpenRouterGenerationParams::provider_defaults(1024, 0.3),
                    ),
            )
            .unwrap(),
        );

        for value in requests {
            assert!(
                value.get("reasoning").is_none(),
                "a flow must not gain a reasoning key: {value}"
            );
        }
    }

    fn request_with_reasoning(reasoning: Option<ReasoningConfig>) -> ChatCompletionRequest {
        let mut params = OpenRouterGenerationParams::provider_defaults(4096, 0.3);
        params.reasoning = reasoning;
        let client = OpenRouterClient::new(
            "secret".to_string(),
            "anthropic/claude-haiku-5.5".to_string(),
        );
        client.build_request("prompt", &params)
    }

    fn status_failure(status: u16, body: &str) -> RequestFailure {
        RequestFailure::Status {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            body: body.to_string(),
        }
    }

    #[test]
    fn a_400_that_names_reasoning_drops_the_override() {
        let request = request_with_reasoning(Some(ReasoningConfig::NONE));

        assert!(should_drop_reasoning(
            &status_failure(
                400,
                r#"{"error":{"message":"Reasoning is mandatory for this model"}}"#
            ),
            &request
        ));
        // Providers do not agree on capitalisation.
        assert!(should_drop_reasoning(
            &status_failure(400, "REASONING effort is not supported"),
            &request
        ));
    }

    #[test]
    fn other_failures_never_drop_the_override() {
        let request = request_with_reasoning(Some(ReasoningConfig::NONE));

        // A 400 about anything else would fail again in exactly the same way.
        assert!(!should_drop_reasoning(
            &status_failure(400, "context length exceeded"),
            &request
        ));
        // Other statuses keep their existing handling (retry/fallback).
        assert!(!should_drop_reasoning(
            &status_failure(502, "reasoning"),
            &request
        ));
        assert!(!should_drop_reasoning(
            &status_failure(401, "reasoning"),
            &request
        ));
        assert!(!should_drop_reasoning(
            &RequestFailure::Payload("reasoning".to_string()),
            &request
        ));
        // Nothing to drop when the request never carried the override.
        assert!(!should_drop_reasoning(
            &status_failure(400, "reasoning"),
            &request_with_reasoning(None)
        ));
    }

    #[test]
    fn serializes_multimodal_prompt_with_text_and_image_parts() {
        let client = OpenRouterClient::new("secret".to_string(), "google/gemma-4".to_string());
        let params = OpenRouterGenerationParams::provider_defaults(1024, 0.3);
        let image = OpenRouterImage {
            data_url: "data:image/png;base64,AQID".to_string(),
            mime_type: "image/png".to_string(),
            payload_bytes: 3,
        };

        let value = serde_json::to_value(client.build_multimodal_request(
            "Correct the OCR",
            &image,
            &params,
        ))
        .unwrap();

        assert_eq!(value["messages"][0]["content"][0]["type"], "text");
        assert_eq!(
            value["messages"][0]["content"][0]["text"],
            "Correct the OCR"
        );
        assert_eq!(value["messages"][0]["content"][1]["type"], "image_url");
        assert_eq!(
            value["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AQID"
        );
        assert_eq!(
            value["messages"][0]["content"][1]["image_url"]["detail"],
            "high"
        );
    }

    #[test]
    fn loads_images_only_from_inside_app_data() {
        let app_data = tempfile::tempdir().unwrap();
        let image_path = app_data.path().join("assets").join("page.png");
        std::fs::create_dir_all(image_path.parent().unwrap()).unwrap();
        let mut png = Vec::new();
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&image_path, png).unwrap();

        let image =
            OpenRouterImage::from_file(image_path.to_string_lossy().as_ref(), app_data.path())
                .unwrap();
        assert!(image.data_url.starts_with("data:image/png;base64,"));
        assert_eq!(image.mime_type(), "image/png");
        assert!(image.payload_bytes() > 0);
        assert!(OpenRouterImage::from_file("assets/page.png", app_data.path()).is_ok());

        let outside = tempfile::NamedTempFile::new().unwrap();
        let error =
            OpenRouterImage::from_file(outside.path().to_string_lossy().as_ref(), app_data.path())
                .unwrap_err();
        assert!(error.contains("outside the allowed directory"));
    }

    fn payload_of(image: &OpenRouterImage) -> Vec<u8> {
        let (_, base64) = image.data_url.split_once(";base64,").expect("data url");
        BASE64_STANDARD.decode(base64).expect("base64 payload")
    }

    #[test]
    fn sends_a_camera_photo_upright_and_an_untagged_image_as_stored() {
        use crate::image_orient::test_support::{
            assert_upright_quadrants, camera_jpeg, encode_jpeg, upright_quadrants,
        };
        let app_data = tempfile::tempdir().unwrap();

        let photo_path = app_data.path().join("photo.jpg");
        let photo = camera_jpeg(&upright_quadrants(64, 32), 6);
        std::fs::write(&photo_path, &photo).unwrap();
        let image = OpenRouterImage::from_validated_file(&photo_path).unwrap();
        assert_eq!(image.mime_type(), "image/jpeg");
        let sent = payload_of(&image);
        assert_eq!(crate::image_orient::exif_orientation_bytes(&sent), 1);
        let decoded = image::load_from_memory(&sent).unwrap();
        assert_upright_quadrants(&decoded, 64, 32, "LLM image, orientation 6");
        // The file itself is never rewritten.
        assert_eq!(std::fs::read(&photo_path).unwrap(), photo);

        let scan_path = app_data.path().join("scan.jpg");
        let scan = encode_jpeg(upright_quadrants(64, 32));
        std::fs::write(&scan_path, &scan).unwrap();
        let image = OpenRouterImage::from_validated_file(&scan_path).unwrap();
        assert_eq!(payload_of(&image), scan);
    }

    #[test]
    fn rejects_source_images_above_the_bounded_read_limit() {
        let app_data = tempfile::tempdir().unwrap();
        let image_path = app_data.path().join("oversized.png");
        let file = std::fs::File::create(&image_path).unwrap();
        file.set_len(MAX_MULTIMODAL_SOURCE_BYTES + 1).unwrap();

        let error =
            OpenRouterImage::from_file(image_path.to_string_lossy().as_ref(), app_data.path())
                .unwrap_err();
        assert!(error.contains("source limit"));
    }

    #[test]
    fn serializes_persisted_settings_for_every_user_visible_flow() {
        let conn = settings_conn();
        let cases = [
            (
                GenerationFlow::OcrCorrection,
                "llm_ocr_correction",
                "vendor/ocr",
                3000,
            ),
            (
                GenerationFlow::Summary,
                "llm_summary",
                "vendor/summary",
                700,
            ),
            (GenerationFlow::Ner, "llm_ner", "vendor/ner", 4096),
            (
                GenerationFlow::Triplets,
                "llm_triplets",
                "vendor/triplets",
                1800,
            ),
        ];

        for (flow, prefix, model, max_tokens) in cases {
            let model_key = if flow == GenerationFlow::Ner {
                "openrouter_ner_model".to_string()
            } else {
                format!("{prefix}_model")
            };
            let values = [
                (model_key, model.to_string()),
                (format!("{prefix}_max_tokens"), max_tokens.to_string()),
                (format!("{prefix}_temperature"), "0.2".to_string()),
                (format!("{prefix}_top_p"), "0.85".to_string()),
                (format!("{prefix}_top_k"), "12".to_string()),
                (format!("{prefix}_presence_penalty"), "0.1".to_string()),
                (format!("{prefix}_frequency_penalty"), "-0.2".to_string()),
                (format!("{prefix}_stop_sequences"), "END\nSTOP".to_string()),
            ];
            for (key, value) in values {
                crate::settings::set_setting(&conn, &key, &value).unwrap();
            }

            let config = generation_config_from_settings(&conn, flow);
            let client = OpenRouterClient::new("secret".to_string(), config.model.clone());
            let value =
                serde_json::to_value(client.build_request("prompt", &config.params)).unwrap();

            assert_eq!(value["model"], model);
            assert_eq!(value["max_tokens"], max_tokens);
            assert!((value["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
            assert!((value["top_p"].as_f64().unwrap() - 0.85).abs() < 1e-6);
            assert_eq!(value["top_k"], 12);
            assert!((value["presence_penalty"].as_f64().unwrap() - 0.1).abs() < 1e-6);
            assert!((value["frequency_penalty"].as_f64().unwrap() + 0.2).abs() < 1e-6);
            assert_eq!(value["stop"], serde_json::json!(["END", "STOP"]));
        }
    }
}
