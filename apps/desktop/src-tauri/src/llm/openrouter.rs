use std::{io::Cursor, path::Path, time::Duration};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use image::GenericImageView;
use serde::{Deserialize, Serialize};

use super::generation::OpenRouterGenerationParams;

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
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
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
            message: error.to_string(),
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

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
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
                "OCR correction image exceeds the {} byte source limit",
                MAX_MULTIMODAL_SOURCE_BYTES
            ));
        }

        let bytes = std::fs::read(path)
            .map_err(|error| format!("Failed to read OCR correction image: {error}"))?;
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
                "OCR correction image exceeds the {} byte request limit even at 1x1 pixels",
                MAX_MULTIMODAL_IMAGE_BYTES
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
            .user_agent("EntropIA-Desktop/0.1 (historical-research-app)")
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

    /// Sends the request, retrying at most once and only for failures that are
    /// safe to repeat (see [`RequestFailure::is_retryable`]).
    async fn send_request(&self, request: ChatCompletionRequest) -> Result<String, String> {
        let mut attempt = 1;
        loop {
            match self.try_send_request(&request).await {
                Ok(answer) => return Ok(answer),
                Err(failure) => {
                    if attempt >= Self::MAX_ATTEMPTS || !failure.is_retryable() {
                        return Err(failure.message());
                    }
                    eprintln!(
                        "[openrouter] attempt {attempt} failed ({}); retrying once",
                        failure.message()
                    );
                    attempt += 1;
                }
            }
        }
    }

    async fn try_send_request(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<String, RequestFailure> {
        let response = self
            .client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
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

        let parsed: ChatCompletionResponse = response.json().await.map_err(|e| {
            RequestFailure::Payload(format!("Failed to parse OpenRouter response: {e}"))
        })?;

        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content.trim().to_string())
            .ok_or_else(|| RequestFailure::Payload("OpenRouter returned no choices".to_string()))
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
        }
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
            .map_err(|e| format!("OpenRouter connection test failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("OpenRouter API error ({}): {}", status, body));
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
