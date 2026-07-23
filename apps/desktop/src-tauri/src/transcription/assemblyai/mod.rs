//! AssemblyAI integration with a schema-fallback compatibility layer.
//!
//! ## Why this module exists
//!
//! AssemblyAI replaces wire-level parameters from time to time. The
//! concrete incident that motivated this refactor: `speech_model`
//! (single string) was replaced by `speech_models` (list). Sending
//! the old field against the current API breaks dictation outright.
//!
//! To survive the next rename without another hotfix we:
//!
//! 1. **Centralize** the endpoint, default models, and the list of
//!    alternative schemas in [`config::AssemblyAiConfig`].
//! 2. **Separate** payload construction
//!    ([`payloads::build_create_transcript_payload`]) from request
//!    sending ([`CreateTransporter`]), so each layer is testable on
//!    its own.
//! 3. **Try alternative schemas** only when AssemblyAI returns HTTP
//!    400 with a body that looks like an obsolete/unknown/incompatible
//!    parameter error (see [`compatibility_error_meta`]). Never on
//!    network errors, timeouts, 5xx, 401, 404, 429, or 400s that don't
//!    look like a parameter-name issue — those would risk duplicate
//!    transcripts and are surfaced verbatim.
//! 4. **Record** which schema and model list finally succeeded so an
//!    operator can see at a glance whether the fallback path is in
//!    use.
//!
//! The rest of the dictation flow (audio upload, polling, formatting,
//! role identification) is untouched.

mod config;
mod payloads;
#[cfg(test)]
mod test_support;
mod transporter;

use super::engine::{Segment, TranscriptionResult};
use config::{
    AssemblyAiConfig, SchemaKind, MAX_POLL_ATTEMPTS, POLL_INTERVAL_SECS, REQUEST_TIMEOUT_SECS,
};
use payloads::{build_create_transcript_payload, CreateTranscriptOptions};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use transporter::{CreateTranscriptSend, CreateTransporter, ReqwestCreateTransporter};

#[derive(Deserialize)]
struct AssemblyAiApiError {
    error: Option<String>,
}

#[derive(Deserialize)]
struct UploadResponse {
    upload_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CreateTranscriptResponse {
    id: String,
}

#[derive(Deserialize)]
struct TranscriptStatusResponse {
    status: String,
    text: Option<String>,
    error: Option<String>,
    language_code: Option<String>,
    audio_duration: Option<f64>,
    utterances: Option<Vec<TranscriptUtterance>>,
    speech_understanding: Option<SpeechUnderstandingStatus>,
}

#[derive(Deserialize)]
struct TranscriptUtterance {
    speaker: Option<String>,
    speaker_label: Option<String>,
    text: String,
}

#[derive(Deserialize)]
struct SpeechUnderstandingStatus {
    response: Option<SpeechUnderstandingResponse>,
}

#[derive(Deserialize)]
struct SpeechUnderstandingResponse {
    speaker_identification: Option<SpeakerIdentificationResponse>,
}

#[derive(Deserialize)]
struct SpeakerIdentificationResponse {
    mapping: Option<HashMap<String, String>>,
}

/// AssemblyAI STT client. Holds the API key, the shared reqwest client
/// (used for upload + polling), and the provider config (endpoint,
/// models, schema fallback chain).
pub struct AssemblyAiClient {
    http: reqwest::Client,
    api_key: String,
    config: AssemblyAiConfig,
}

impl AssemblyAiClient {
    pub fn new(api_key: String) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("EntropIA-Desktop/0.1 (historical-research-app)")
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .expect("Failed to build reqwest client");

        Self {
            http,
            api_key,
            config: AssemblyAiConfig::load(),
        }
    }

    pub async fn test_connection(&self) -> Result<(), String> {
        let response = self
            .http
            .get(format!("{}/transcript?limit=1", self.config.api_base))
            .header("Authorization", &self.api_key)
            .send()
            .await
            .map_err(|e| format!("AssemblyAI connection test failed: {e}"))?;

        ensure_success(response, "AssemblyAI").await.map(|_| ())
    }

    pub async fn transcribe_file<F>(
        &self,
        audio_path: &Path,
        asset_id: Option<&str>,
        enable_role_speaker_identification: bool,
        mut on_progress: F,
    ) -> Result<TranscriptionResult, String>
    where
        F: FnMut(u8, &str),
    {
        on_progress(20, "uploading");

        let audio_bytes = tokio::fs::read(audio_path)
            .await
            .map_err(|e| format!("Failed to read audio file {}: {e}", audio_path.display()))?;

        let upload_response = self
            .http
            .post(format!("{}/upload", self.config.api_base))
            .header("Authorization", &self.api_key)
            .header("Content-Type", "application/octet-stream")
            .body(audio_bytes)
            .send()
            .await
            .map_err(|e| format!("AssemblyAI upload failed: {e}"))?;

        let upload: UploadResponse = ensure_success(upload_response, "AssemblyAI")
            .await?
            .json()
            .await
            .map_err(|e| format!("Failed to parse AssemblyAI upload response: {e}"))?;

        on_progress(40, "submitting_remote");

        let transporter = ReqwestCreateTransporter::new(self.http.clone());
        let url = format!("{}/transcript", self.config.api_base);
        let compat = create_transcript_with_compat(
            &transporter,
            &url,
            &self.api_key,
            &self.config,
            CreateTranscriptOptions {
                audio_url: upload.upload_url,
                enable_role_speaker_identification,
            },
        )
        .await?;
        let created = compat.response;
        // `compat.schema_used` is retained for the structured log line below —
        // we already emitted the "schema accepted" message inside the fallback
        // loop, but operators want to see it near the asset id when that flow
        // has one. Dictation has no asset row yet, so omit asset_id instead of
        // logging a misleading placeholder.
        eprintln!(
            "{}",
            format_create_transcript_success_log_line(
                asset_id,
                &compat.schema_used.schema_name,
                compat.schema_used.schema_kind,
                &self.config.default_models,
            )
        );

        let mut poll_attempt = 0_u16;
        loop {
            poll_attempt = poll_attempt.saturating_add(1);
            let progress = 45_u8
                .saturating_add((poll_attempt.saturating_sub(1).min(9) as u8).saturating_mul(5));
            on_progress(progress.min(90), "polling_remote");

            let status_response = self
                .http
                .get(format!(
                    "{}/transcript/{}",
                    self.config.api_base, created.id
                ))
                .header("Authorization", &self.api_key)
                .send()
                .await
                .map_err(|e| format!("AssemblyAI polling failed: {e}"))?;

            let transcript: TranscriptStatusResponse =
                ensure_success(status_response, "AssemblyAI")
                    .await?
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse AssemblyAI polling response: {e}"))?;

            match transcript.status.as_str() {
                "completed" => {
                    let text = format_transcript_text(
                        transcript.text.unwrap_or_default(),
                        transcript.utterances,
                        transcript.speech_understanding,
                    );
                    let duration_ms = transcript
                        .audio_duration
                        .map(|seconds| (seconds * 1000.0).round() as u64)
                        .unwrap_or(0);
                    let segments = if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![Segment {
                            start: 0.0,
                            end: duration_ms as f64 / 1000.0,
                            text: text.clone(),
                        }]
                    };

                    return Ok(TranscriptionResult {
                        text,
                        language: transcript
                            .language_code
                            .unwrap_or_else(|| "auto".to_string()),
                        segments,
                        duration_ms,
                    });
                }
                "error" => {
                    return Err(transcript.error.unwrap_or_else(|| {
                        "AssemblyAI returned an unknown transcription error".to_string()
                    }))
                }
                _ => {
                    if poll_attempt >= MAX_POLL_ATTEMPTS {
                        return Err(polling_exhausted_error(&transcript.status, poll_attempt));
                    }
                    tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
                }
            }
        }
    }
}

/// Outcome of the compatibility fallback loop. Exposed for tests and
/// for the structured log line emitted at the end of a successful
/// (or exhausted) attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompatAttempt {
    pub schema_name: String,
    pub schema_kind: SchemaKind,
}

#[derive(Debug, Clone)]
pub(crate) struct CompatOutcome {
    pub response: CreateTranscriptResponse,
    pub schema_used: CompatAttempt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompatibilityErrorMeta {
    body_len: usize,
    class: &'static str,
}

/// Submit a create-transcript request, walking the configured schema
/// fallback chain. Behaviour:
///
/// - **Success (2xx)** — return immediately. Log the schema/model
///   used.
/// - **HTTP 400 with a body that looks like an obsolete/unknown/
///   incompatible parameter** — try the next schema. If the chain is
///   exhausted, return a descriptive error.
/// - **Anything else** (other 4xx, 5xx, network errors, timeouts) —
///   return the error verbatim. NEVER retry, to avoid duplicate
///   transcripts on the AssemblyAI side.
///
/// Splitting this out as a free function (instead of a method on
/// [`AssemblyAiClient`]) is what lets tests inject a
/// [`transporter::CreateTransporter`] mock and assert the fallback
/// behaviour without touching the network.
pub async fn create_transcript_with_compat<T: CreateTransporter>(
    transporter: &T,
    url: &str,
    api_key: &str,
    config: &AssemblyAiConfig,
    options: CreateTranscriptOptions,
) -> Result<CompatOutcome, String> {
    if config.schemas.is_empty() {
        return Err("AssemblyAI config has no schemas to try".to_string());
    }

    let mut last_compat_error: Option<CompatibilityErrorMeta> = None;

    for (index, schema) in config.schemas.iter().enumerate() {
        let payload = build_create_transcript_payload(config, schema, &options);

        match transporter
            .post_create_transcript(url, api_key, payload)
            .await
        {
            CreateTranscriptSend::Response(outcome) => {
                if outcome.status >= 200 && outcome.status < 300 {
                    let response: CreateTranscriptResponse = serde_json::from_str(&outcome.body)
                        .map_err(|e| {
                            format!(
                                "Failed to parse AssemblyAI transcript response (schema={}): {e}",
                                schema.name
                            )
                        })?;
                    log_schema_used(&schema.name, &config.default_models, &outcome.body);
                    return Ok(CompatOutcome {
                        response,
                        schema_used: CompatAttempt {
                            schema_name: schema.name.clone(),
                            schema_kind: schema.kind,
                        },
                    });
                }

                if outcome.status == 400 {
                    if let Some(meta) = compatibility_error_meta(&outcome.body) {
                        eprintln!(
                        "[assemblyai] Schema '{}' rejected with compatibility error (status=400, body_bytes={}, class={})",
                        schema.name,
                        meta.body_len,
                        meta.class
                    );
                        last_compat_error = Some(meta);
                        // Move to the next schema.
                        continue;
                    }
                }

                // Non-compatibility failure (any other 4xx/5xx). Surface
                // it verbatim and DO NOT retry — duplicate transcripts
                // on the AssemblyAI side are the worst-case failure
                // mode we want to avoid.
                let parsed = serde_json::from_str::<AssemblyAiApiError>(&outcome.body)
                    .ok()
                    .and_then(|p| p.error)
                    .unwrap_or_else(|| outcome.body.trim().to_string());
                return Err(format!(
                    "AssemblyAI API error ({}): {}",
                    outcome.status, parsed
                ));
            }
            CreateTranscriptSend::TransportError(error) => {
                // Network/timeout/TLS error. No retry — log and bail.
                eprintln!(
                    "[assemblyai] Transport error posting transcript with schema '{}': {error}",
                    schema.name
                );
                return Err(format!(
                    "AssemblyAI transcript request failed (schema={}, attempt={}): {error}",
                    schema.name,
                    index + 1
                ));
            }
        }
    }

    let last = last_compat_error
        .map(|meta| {
            format!(
                " last 400 metadata: body_bytes={}, class={}",
                meta.body_len, meta.class
            )
        })
        .unwrap_or_default();
    Err(format!(
        "AssemblyAI rejected every configured transcript schema ({n} tried).{last}",
        n = config.schemas.len()
    ))
}

/// Heuristic: does this 400 body look like an
/// obsolete/unknown/incompatible parameter error? If yes, the
/// fallback chain may try a different schema; if no, the body is
/// surfaced to the caller verbatim.
///
/// We only retry when the 400 clearly points to the AssemblyAI speech
/// model field transition this compatibility layer can actually fix.
/// Broad words like `invalid`, `unsupported`, `field`, `parameter`,
/// `schema`, or `not allowed` are NOT enough on their own: messages
/// about `language`, `audio_url`, generic required fields, or generic
/// invalid requests are user/input errors, not speech-model schema
/// compatibility signals. Matches are case-insensitive and use either
/// the structured `error` field or the raw body.
fn compatibility_error_meta(body: &str) -> Option<CompatibilityErrorMeta> {
    classify_compatibility_error_body(body).map(|class| CompatibilityErrorMeta {
        body_len: body.len(),
        class,
    })
}

fn classify_compatibility_error_body(body: &str) -> Option<&'static str> {
    let parsed = serde_json::from_str::<AssemblyAiApiError>(body)
        .ok()
        .and_then(|p| p.error)
        .unwrap_or_else(|| body.to_string());
    let haystack = parsed.to_lowercase();

    let mentions_model_schema = mentions_speech_model_field(&haystack);
    let has_compat_action = [
        "unknown",
        "unrecognized",
        "invalid",
        "deprecated",
        "obsolete",
        "not allowed",
        "unsupported",
        "unexpected",
        "incompatible",
        "replaced",
        "schema",
    ]
    .iter()
    .any(|needle| haystack.contains(needle));
    if mentions_model_schema && has_compat_action {
        return Some("speech_model_schema");
    }

    None
}

fn mentions_speech_model_field(text: &str) -> bool {
    [
        "speech_models",
        "speech-models",
        "speech models",
        "speech_model",
        "speech-model",
        "speech model",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn log_schema_used(schema_name: &str, models: &[String], body: &str) {
    // We log the response body length rather than the body itself so
    // transcripts (which can contain user audio content once
    // AssemblyAI includes them) aren't dumped to disk.
    eprintln!(
        "[assemblyai] Schema '{}' accepted (models={:?}, response_bytes={})",
        schema_name,
        models,
        body.len()
    );
}

fn format_create_transcript_success_log_line(
    asset_id: Option<&str>,
    schema: &str,
    kind: SchemaKind,
    models: &[String],
) -> String {
    let base = "[transcription] assemblyai.create_transcript";
    match asset_id {
        Some(asset_id) => {
            format!("{base} asset_id={asset_id} schema={schema} kind={kind:?} models={models:?}")
        }
        None => format!("{base} schema={schema} kind={kind:?} models={models:?}"),
    }
}

fn polling_exhausted_error(last_status: &str, attempts: u16) -> String {
    format!(
        "AssemblyAI polling did not reach a terminal status after {attempts} attempts (max_attempts={MAX_POLL_ATTEMPTS}, last_status={last_status})"
    )
}

async fn ensure_success(
    response: reqwest::Response,
    provider_name: &str,
) -> Result<reqwest::Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let body = response.text().await.unwrap_or_default();
    let api_error = serde_json::from_str::<AssemblyAiApiError>(&body)
        .ok()
        .and_then(|parsed| parsed.error)
        .unwrap_or_else(|| body.trim().to_string());

    Err(format!("{provider_name} API error ({status}): {api_error}"))
}

fn format_transcript_text(
    fallback_text: String,
    utterances: Option<Vec<TranscriptUtterance>>,
    speech_understanding: Option<SpeechUnderstandingStatus>,
) -> String {
    let speaker_mapping = speech_understanding
        .and_then(|status| status.response)
        .and_then(|response| response.speaker_identification)
        .and_then(|speaker_identification| speaker_identification.mapping);

    if let (Some(utterances), Some(mapping)) = (utterances, speaker_mapping.as_ref()) {
        let formatted = utterances
            .into_iter()
            .filter_map(|utterance| {
                let speaker_key = utterance.speaker.or(utterance.speaker_label)?;
                let label = mapping
                    .get(&speaker_key)
                    .map(|value| display_speaker_role(value))
                    .unwrap_or_else(|| speaker_key.trim().to_string());
                let text = utterance.text.trim();
                if text.is_empty() {
                    None
                } else {
                    Some(format!("{label}: {text}"))
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        if !formatted.is_empty() {
            return formatted;
        }
    }

    if let Some(mapping) = speaker_mapping.as_ref() {
        let remapped = remap_speaker_prefixes(&fallback_text, mapping);
        if !remapped.is_empty() {
            return remapped;
        }
    }

    fallback_text.trim().to_string()
}

fn remap_speaker_prefixes(text: &str, mapping: &HashMap<String, String>) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim();
            let Some((speaker_key, content)) = trimmed.split_once(':') else {
                return trimmed.to_string();
            };

            let Some(mapped_speaker) = mapping.get(speaker_key.trim()) else {
                return trimmed.to_string();
            };

            let content = content.trim();
            if content.is_empty() {
                display_speaker_role(mapped_speaker)
            } else {
                format!("{}: {}", display_speaker_role(mapped_speaker), content)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn display_speaker_role(role: &str) -> String {
    match role.trim() {
        "Entrevistador" => "Entrevistador/a".to_string(),
        "Entrevistado" => "Entrevistado/a".to_string(),
        other => other.to_string(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::test_support::{MockCreateTransporter, ScriptedCreateTransporter};
    use super::transporter::CreateTranscriptOutcome;
    use super::*;
    use serde_json::json;

    fn current_only_config() -> AssemblyAiConfig {
        AssemblyAiConfig {
            api_base: "https://api.assemblyai.com/v2".to_string(),
            default_models: vec!["universal-3-5-pro".to_string(), "universal-2".to_string()],
            schemas: vec![config::SchemaSpec {
                name: "speech_models_list".to_string(),
                kind: SchemaKind::SpeechModelsList,
            }],
        }
    }

    fn default_with_fallback() -> AssemblyAiConfig {
        AssemblyAiConfig {
            api_base: "https://api.assemblyai.com/v2".to_string(),
            default_models: vec!["universal-3-5-pro".to_string(), "universal-2".to_string()],
            schemas: vec![
                config::SchemaSpec {
                    name: "speech_models_list".to_string(),
                    kind: SchemaKind::SpeechModelsList,
                },
                config::SchemaSpec {
                    name: "speech_model_single_legacy".to_string(),
                    kind: SchemaKind::SpeechModelSingle,
                },
            ],
        }
    }

    fn options(audio_url: &str) -> CreateTranscriptOptions {
        CreateTranscriptOptions {
            audio_url: audio_url.to_string(),
            enable_role_speaker_identification: false,
        }
    }

    fn url() -> &'static str {
        "https://api.assemblyai.com/v2/transcript"
    }

    #[test]
    fn create_transcript_success_log_line_includes_real_asset_id_when_available() {
        let models = vec!["universal-3-5-pro".to_string(), "universal-2".to_string()];

        let line = format_create_transcript_success_log_line(
            Some("asset-123"),
            "speech_models_list",
            SchemaKind::SpeechModelsList,
            &models,
        );

        assert!(line.contains("asset_id=asset-123"), "got: {line}");
        assert!(line.contains("schema=speech_models_list"), "got: {line}");
        assert!(line.contains("kind=SpeechModelsList"), "got: {line}");
        assert!(
            line.contains("models=[\"universal-3-5-pro\", \"universal-2\"]"),
            "got: {line}"
        );
    }

    #[test]
    fn create_transcript_success_log_line_omits_asset_id_for_dictation() {
        let models = vec!["universal-3-5-pro".to_string(), "universal-2".to_string()];

        let line = format_create_transcript_success_log_line(
            None,
            "speech_models_list",
            SchemaKind::SpeechModelsList,
            &models,
        );

        assert!(!line.contains("asset_id=unknown"), "got: {line}");
        assert!(!line.contains("asset_id="), "got: {line}");
        assert!(line.contains("schema=speech_models_list"), "got: {line}");
        assert!(line.contains("kind=SpeechModelsList"), "got: {line}");
        assert!(
            line.contains("models=[\"universal-3-5-pro\", \"universal-2\"]"),
            "got: {line}"
        );
    }

    #[tokio::test]
    async fn first_attempt_uses_current_protocol_and_skips_fallback_when_accepted() {
        let transporter = MockCreateTransporter::single(CreateTranscriptOutcome {
            status: 200,
            body: r#"{"id":"abc123"}"#.to_string(),
        });

        let outcome = create_transcript_with_compat(
            &transporter,
            url(),
            "k",
            &default_with_fallback(),
            options("https://x/y.mp3"),
        )
        .await
        .expect("should succeed");

        assert_eq!(outcome.schema_used.schema_name, "speech_models_list");
        assert_eq!(
            outcome.schema_used.schema_kind,
            SchemaKind::SpeechModelsList
        );
        assert_eq!(outcome.response.id, "abc123");

        let attempts = transporter.take_attempts();
        assert_eq!(attempts.len(), 1, "fallback must not be tried on 200");
        // Current-protocol payload uses speech_models (list) and omits speech_model.
        let payload = &attempts[0].payload;
        assert!(
            payload
                .get("speech_models")
                .map(|v| v.is_array())
                .unwrap_or(false),
            "current protocol must send speech_models as a list, got: {payload}"
        );
        assert!(
            payload.get("speech_model").is_none(),
            "current protocol must NOT send speech_model"
        );
    }

    #[tokio::test]
    async fn fallback_attempts_next_schema_only_after_compatibility_400_and_each_payload_uses_one_field(
    ) {
        // Two-script test:
        // 1) Current schema → 400 with "speech_models is unexpected" → fallback
        // 2) Legacy schema   → 200 OK
        // Each payload must contain exactly one of speech_model / speech_models,
        // never both. If a future refactor accidentally emits both, the
        // fallback could double-charge the user or be rejected outright.
        let transporter = ScriptedCreateTransporter::new(vec![
            CreateTranscriptOutcome {
                status: 400,
                body: r#"{"error":"speech_models is not allowed for this endpoint"}"#.to_string(),
            },
            CreateTranscriptOutcome {
                status: 200,
                body: r#"{"id":"legacy-ok"}"#.to_string(),
            },
        ]);

        let outcome = create_transcript_with_compat(
            &transporter,
            url(),
            "k",
            &default_with_fallback(),
            options("https://x/y.mp3"),
        )
        .await
        .expect("fallback should reach 200");

        assert_eq!(
            outcome.schema_used.schema_name,
            "speech_model_single_legacy"
        );
        assert_eq!(
            outcome.schema_used.schema_kind,
            SchemaKind::SpeechModelSingle
        );
        assert_eq!(outcome.response.id, "legacy-ok");

        let attempts = transporter.take_attempts();
        assert_eq!(attempts.len(), 2, "expected exactly two attempts");

        // Attempt 1: current protocol — speech_models as list, no speech_model.
        let first = &attempts[0].payload;
        assert!(first
            .get("speech_models")
            .map(|v| v.is_array())
            .unwrap_or(false));
        assert!(first.get("speech_model").is_none());

        // Attempt 2: legacy single — speech_model as string, no speech_models.
        let second = &attempts[1].payload;
        assert!(second
            .get("speech_model")
            .map(|v| v.is_string())
            .unwrap_or(false));
        assert!(second.get("speech_models").is_none());
    }

    #[tokio::test]
    async fn compatibility_error_message_detection_matches_known_keywords() {
        // Positive cases — should all be flagged as compat errors.
        let positives = [
            r#"{"error":"speech_models is unknown"}"#,
            r#"{"error":"Invalid value for parameter 'speech_models'"}"#,
            r#"{"error":"Field speech_model is deprecated"}"#,
            r#"{"error":"speech-models is unsupported"}"#,
            r#"{"error":"speech model field is not allowed"}"#,
            r#"{"error":"speech_model has been replaced by speech_models"}"#,
        ];
        for body in positives {
            assert!(
                compatibility_error_meta(body).is_some(),
                "expected compat error for: {body}"
            );
        }

        // Negative cases — should NOT be flagged as compat errors.
        let negatives = [
            r#"{"error":"Audio duration exceeds plan limit"}"#,
            r#"{"error":"Authorization header missing"}"#,
            r#"{"error":"Quota exceeded"}"#,
            r#"{"error":"Internal server error"}"#,
            r#"rate limit reached, retry later"#,
            r#"{"error":"invalid audio_url"}"#,
            r#"{"error":"unsupported audio format"}"#,
            r#"{"error":"unsupported language"}"#,
            r#"{"error":"field audio_url is required"}"#,
            r#"{"error":"parameter language is unsupported"}"#,
            r#"{"error":"field audio_url is not allowed"}"#,
            r#"{"error":"field is required"}"#,
            r#"{"error":"invalid request"}"#,
            r#"{"error":"schema not allowed"}"#,
            r#"plain text saying the parameter is unsupported"#,
            r#"{"error":"unexpected field"}"#,
            r#"{"error":"payload schema mismatch"}"#,
            "",
        ];
        for body in negatives {
            assert!(
                compatibility_error_meta(body).is_none(),
                "expected NON-compat error for: {body}"
            );
        }
    }

    #[tokio::test]
    async fn no_second_attempt_when_400_is_not_a_compatibility_error() {
        // These 400s are request/input errors, not the speech-model
        // schema transition this compatibility layer can fix. Retrying
        // would risk creating a duplicate transcript with different
        // parameters.
        let cases = [
            "Audio duration exceeds plan limit",
            "parameter language is unsupported",
            "field audio_url is not allowed",
            "field is required",
            "invalid request",
        ];

        for message in cases {
            let transporter = MockCreateTransporter::single(CreateTranscriptOutcome {
                status: 400,
                body: format!(r#"{{"error":"{message}"}}"#),
            });

            let err = create_transcript_with_compat(
                &transporter,
                url(),
                "k",
                &default_with_fallback(),
                options("https://x/y.mp3"),
            )
            .await
            .expect_err("non-compat 400 must surface as an error");
            assert!(
                err.contains(message),
                "raw non-compat 400 message must be preserved, got: {err}"
            );
            assert_eq!(
                transporter.take_attempts().len(),
                1,
                "fallback must not be tried for non-compat 400: {message}"
            );
        }
    }

    #[tokio::test]
    async fn no_second_attempt_on_transport_errors_or_5xx() {
        // Three scenarios that must NEVER retry:
        //   a) transport error (timeout / DNS / TLS)
        //   b) HTTP 401
        //   c) HTTP 500
        // Each must surface verbatim and the mock must see exactly one
        // attempt — duplicate transcript protection.
        let cases: Vec<(&'static str, CreateTranscriptSend)> = vec![
            (
                "transport_error",
                CreateTranscriptSend::TransportError("connection timed out".to_string()),
            ),
            (
                "401",
                CreateTranscriptSend::Response(CreateTranscriptOutcome {
                    status: 401,
                    body: r#"{"error":"Unauthorized"}"#.to_string(),
                }),
            ),
            (
                "500",
                CreateTranscriptSend::Response(CreateTranscriptOutcome {
                    status: 500,
                    body: r#"{"error":"Internal server error"}"#.to_string(),
                }),
            ),
            (
                "404",
                CreateTranscriptSend::Response(CreateTranscriptOutcome {
                    status: 404,
                    body: r#"{"error":"not found"}"#.to_string(),
                }),
            ),
            (
                "429",
                CreateTranscriptSend::Response(CreateTranscriptOutcome {
                    status: 429,
                    body: r#"{"error":"rate limited"}"#.to_string(),
                }),
            ),
        ];

        for (label, send) in cases {
            let transporter = MockCreateTransporter::single_send(send);
            let err = create_transcript_with_compat(
                &transporter,
                url(),
                "k",
                &default_with_fallback(),
                options("https://x/y.mp3"),
            )
            .await
            .expect_err(&format!("{label}: expected error, got success"));

            let attempts = transporter.take_attempts();
            assert_eq!(
                attempts.len(),
                1,
                "{label}: must not retry — duplicate-transcript risk. Got: {attempts:?}"
            );
            // Transport errors and 5xx should be propagated verbatim.
            match label {
                "transport_error" => assert!(
                    err.contains("timed out") || err.contains("transcript request failed"),
                    "{label}: error should propagate transport detail, got: {err}"
                ),
                "401" => assert!(
                    err.contains("401"),
                    "{label}: status code must be preserved, got: {err}"
                ),
                "500" => assert!(
                    err.contains("500"),
                    "{label}: status code must be preserved, got: {err}"
                ),
                "404" => assert!(
                    err.contains("404"),
                    "{label}: status code must be preserved, got: {err}"
                ),
                "429" => assert!(
                    err.contains("429"),
                    "{label}: status code must be preserved, got: {err}"
                ),
                _ => unreachable!(),
            }
        }
    }

    #[tokio::test]
    async fn fallback_chain_exhausted_returns_descriptive_error_when_all_schemas_400_compat() {
        // If every schema in the chain comes back with a compat 400,
        // we want an error that names how many were tried so the
        // operator can extend the JSON config.
        let transporter = ScriptedCreateTransporter::new(vec![
            CreateTranscriptOutcome {
                status: 400,
                body: r#"{"error":"speech_models unknown for audio_url=https://signed.example/audio.wav"}"#.to_string(),
            },
            CreateTranscriptOutcome {
                status: 400,
                body: r#"{"error":"speech_model deprecated for request field payload"}"#.to_string(),
            },
        ]);

        let err = create_transcript_with_compat(
            &transporter,
            url(),
            "k",
            &default_with_fallback(),
            options("https://x/y.mp3"),
        )
        .await
        .expect_err("both schemas failing must surface as error");
        assert!(
            err.contains("2 tried"),
            "error must mention how many schemas were tried, got: {err}"
        );
        assert!(
            err.contains("body_bytes="),
            "error should keep metadata, got: {err}"
        );
        assert!(
            err.contains("class="),
            "error should keep keyword class, got: {err}"
        );
        assert!(
            !err.contains("signed.example") && !err.contains("audio_url"),
            "compatibility exhaustion error must not leak raw provider body, got: {err}"
        );
        assert_eq!(
            transporter.take_attempts().len(),
            2,
            "should have tried every configured schema exactly once"
        );
    }

    #[tokio::test]
    async fn current_protocol_payload_assertion_matches_user_specified_model_list() {
        // Direct pin to the user's example:
        //   speech_models must be sent as ["universal-3-5-pro", "universal-2"]
        // even when our default config gets overridden. This is the
        // single most important wire contract; if it breaks, dictation
        // breaks.
        let transporter = MockCreateTransporter::single(CreateTranscriptOutcome {
            status: 200,
            body: r#"{"id":"id"}"#.to_string(),
        });

        let _ = create_transcript_with_compat(
            &transporter,
            url(),
            "k",
            &current_only_config(),
            options("https://x/y.mp3"),
        )
        .await
        .expect("ok");

        let payload = &transporter.take_attempts()[0].payload;
        assert_eq!(
            payload.get("speech_models"),
            Some(&json!(["universal-3-5-pro", "universal-2"])),
            "payload must pin to the user-specified model list"
        );
    }

    #[test]
    fn compatibility_error_metadata_does_not_include_raw_body() {
        let body =
            r#"{"error":"speech_models unknown for audio_url=https://signed.example/audio.wav"}"#;
        let meta = compatibility_error_meta(body).expect("compatibility metadata");
        assert_eq!(meta.body_len, body.len());
        assert_eq!(meta.class, "speech_model_schema");
    }

    #[test]
    fn polling_exhausted_error_is_clear_and_bounded() {
        let err = polling_exhausted_error("queued", MAX_POLL_ATTEMPTS);
        assert!(
            err.contains("did not reach a terminal status"),
            "got: {err}"
        );
        assert!(err.contains(&MAX_POLL_ATTEMPTS.to_string()), "got: {err}");
        assert!(err.contains("last_status=queued"), "got: {err}");
    }
}
