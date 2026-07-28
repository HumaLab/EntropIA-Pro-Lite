use std::{io::Cursor, path::Path};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use image::GenericImageView;
use serde::{Deserialize, Serialize};

use super::generation::OpenRouterGenerationParams;

// ---------------------------------------------------------------------------
// OpenRouter API types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: ChatMessageContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum ChatMessageContent {
    Text(String),
    Parts(Vec<ChatContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChatContentPart {
    Text { text: String },
    ImageUrl { image_url: ChatImageUrl },
}

#[derive(Serialize)]
struct ChatImageUrl {
    url: String,
    detail: &'static str,
}

#[derive(Serialize)]
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
    pub const DEFAULT_CONTEXT_WINDOW: u32 = 8192;

    pub fn new(api_key: String, model: String) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("EntropIA-Desktop/0.1 (historical-research-app)")
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

    async fn send_request(&self, request: ChatCompletionRequest) -> Result<String, String> {
        let response = self
            .client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://hlab.com.ar/")
            .header("X-Title", "EntropIA")
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("OpenRouter request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("OpenRouter API error ({}): {}", status, body));
        }

        let parsed: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse OpenRouter response: {e}"))?;

        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content.trim().to_string())
            .ok_or_else(|| "OpenRouter returned no choices".to_string())
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
