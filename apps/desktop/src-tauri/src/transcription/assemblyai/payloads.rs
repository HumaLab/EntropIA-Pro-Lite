//! Payload construction for AssemblyAI's `POST /v2/transcript`.
//!
//! Each [`SchemaSpec`] produces a different JSON payload. The current
//! schema uses `speech_models` as a list of models in priority order;
//! the legacy fallback uses `speech_model` as a single string. Exactly
//! one of those two fields is emitted per call — never both — so the
//! body stays spec-clean even when the fallback chain is exercised.
//!
//! This module is intentionally pure: no I/O, no reqwest, no async.
//! That's what makes it cheap to unit-test that the current protocol
//! payload has the right shape and that the legacy fallback uses the
//! other field.

use serde_json::{json, Value};

use super::config::{legacy_single_model, AssemblyAiConfig, SchemaKind, SchemaSpec};

/// Options that feed the create-transcript payload and are independent
/// of which schema we're using.
#[derive(Debug, Clone)]
pub struct CreateTranscriptOptions {
    pub audio_url: String,
    pub enable_role_speaker_identification: bool,
}

/// Build the create-transcript payload for a specific schema. Exactly
/// one of `speech_model` / `speech_models` is emitted per call —
/// [`crate::transcription::assemblyai::create_transcript_with_compat`]
/// guards against ever combining the two.
pub fn build_create_transcript_payload(
    config: &AssemblyAiConfig,
    schema: &SchemaSpec,
    options: &CreateTranscriptOptions,
) -> Value {
    let mut payload = json!({
        "audio_url": options.audio_url,
        "language_detection": true,
        "temperature": 0,
    });

    match schema.kind {
        SchemaKind::SpeechModelsList => {
            // Current official protocol: list of models in priority
            // order. AssemblyAI walks the list if a higher-priority
            // model fails for this particular audio.
            payload[schema.kind.field_name()] = json!(config.default_models);
        }
        SchemaKind::SpeechModelSingle => {
            // Legacy fallback: single model name.
            payload[schema.kind.field_name()] = json!(legacy_single_model(config));
        }
    }

    if options.enable_role_speaker_identification {
        payload["speaker_labels"] = json!(true);
        payload["speech_understanding"] = json!({
            "request": {
                "speaker_identification": {
                    "speaker_type": "role",
                    "known_values": ["Entrevistador", "Entrevistado"],
                }
            }
        });
    }

    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(kind: SchemaKind, name: &str) -> SchemaSpec {
        SchemaSpec {
            name: name.to_string(),
            kind,
        }
    }

    fn options(audio_url: &str, with_roles: bool) -> CreateTranscriptOptions {
        CreateTranscriptOptions {
            audio_url: audio_url.to_string(),
            enable_role_speaker_identification: with_roles,
        }
    }

    #[test]
    fn current_protocol_payload_uses_speech_models_as_list_and_omits_speech_model() {
        // The contract the user pinned: `speech_models` is a JSON array,
        // `speech_model` is NOT present. If AssemblyAI rolls this field
        // back, this test breaks first so we know to flip the default.
        let config = AssemblyAiConfig::default();
        let payload = build_create_transcript_payload(
            &config,
            &schema(SchemaKind::SpeechModelsList, "current"),
            &options("https://example.test/audio.mp3", false),
        );

        let speech_models = payload
            .get("speech_models")
            .and_then(|v| v.as_array())
            .expect("speech_models must be present and an array");
        assert!(
            !speech_models.is_empty(),
            "speech_models list must not be empty"
        );
        assert!(
            speech_models.iter().all(|v| v.is_string()),
            "speech_models entries must be strings"
        );
        assert_eq!(
            payload.get("speech_model"),
            None,
            "speech_model must NOT be present alongside speech_models"
        );
        assert_eq!(
            payload.get("audio_url").unwrap(),
            "https://example.test/audio.mp3"
        );
        assert_eq!(payload.get("language_detection").unwrap(), true);
        assert_eq!(payload.get("temperature").unwrap(), 0);
        assert_eq!(payload.get("speaker_labels"), None);
        assert_eq!(payload.get("speech_understanding"), None);
    }

    #[test]
    fn legacy_payload_uses_speech_model_single_string_and_omits_speech_models() {
        // The fallback path: a single string, no list. Symmetric to the
        // current-schema test so the two never accidentally overlap.
        let config = AssemblyAiConfig::default();
        let payload = build_create_transcript_payload(
            &config,
            &schema(SchemaKind::SpeechModelSingle, "legacy"),
            &options("https://example.test/audio.mp3", false),
        );

        let speech_model = payload
            .get("speech_model")
            .and_then(|v| v.as_str())
            .expect("speech_model must be present and a string");
        assert!(!speech_model.is_empty());
        assert_eq!(
            payload.get("speech_models"),
            None,
            "speech_models must NOT be present alongside speech_model"
        );
    }

    #[test]
    fn legacy_payload_uses_first_configured_model() {
        let config = AssemblyAiConfig {
            default_models: vec!["best-1".to_string(), "best-2".to_string()],
            ..AssemblyAiConfig::default()
        };
        let payload = build_create_transcript_payload(
            &config,
            &schema(SchemaKind::SpeechModelSingle, "legacy"),
            &options("https://example.test/audio.mp3", false),
        );
        assert_eq!(payload.get("speech_model").unwrap(), "best-1");
    }

    #[test]
    fn role_speaker_identification_is_only_emitted_when_enabled() {
        let config = AssemblyAiConfig::default();
        let enabled = build_create_transcript_payload(
            &config,
            &schema(SchemaKind::SpeechModelsList, "current"),
            &options("https://example.test/audio.mp3", true),
        );
        assert_eq!(enabled.get("speaker_labels").unwrap(), true);
        assert_eq!(
            enabled.get("speech_understanding"),
            Some(&json!({
                "request": {
                    "speaker_identification": {
                        "speaker_type": "role",
                        "known_values": ["Entrevistador", "Entrevistado"],
                    }
                }
            }))
        );

        let disabled = build_create_transcript_payload(
            &config,
            &schema(SchemaKind::SpeechModelsList, "current"),
            &options("https://example.test/audio.mp3", false),
        );
        assert_eq!(disabled.get("speaker_labels"), None);
        assert_eq!(disabled.get("speech_understanding"), None);
    }
}
