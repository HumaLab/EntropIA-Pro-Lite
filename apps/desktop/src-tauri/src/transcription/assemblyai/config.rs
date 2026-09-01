//! AssemblyAI provider configuration.
//!
//! Two layers feed [`AssemblyAiConfig`]:
//!
//! 1. **Built-in defaults** ([`AssemblyAiConfig::default`]) — used when no
//!    bundled `provider-compatibility.json` is present or fails to load.
//!    Keeps the binary self-contained even if the resource file is missing.
//! 2. **Bundled `provider-compatibility.json`** (loaded via
//!    [`AssemblyAiConfig::load`]) — extends the schema fallback chain
//!    without a code change. A future release may additionally fetch a
//!    remote copy from the official EntropIA Lite repo, cache it locally,
//!    and fall back to the bundled copy offline. Remote copies must never
//!    contain API keys (the loader rejects credential-shaped config keys
//!    and obvious credential-bearing assignments/URLs in string values).
//!
//! Centralizing endpoint, default models, and the compatibility schema
//! list here means future API parameter changes only need a JSON update
//! (and a new [`SchemaKind`] variant in this module if a brand-new shape
//! appears). Payload construction lives in [`super::payloads`] and is
//! driven by the [`SchemaKind`] returned from the loaded config.

use serde::Deserialize;

const DEFAULT_API_BASE: &str = "https://api.assemblyai.com/v2";
const DEFAULT_MODEL_PRIMARY: &str = "universal-3-5-pro";
const DEFAULT_MODEL_FALLBACK: &str = "universal-2";
const DEFAULT_LEGACY_MODEL: &str = "universal";

/// Bound every AssemblyAI HTTP operation (upload, transcript create,
/// polling, connection test). A timeout is a single-attempt transport
/// failure; callers must not retry the create-transcript request.
pub const REQUEST_TIMEOUT_SECS: u64 = 120;
/// Existing polling cadence, centralized beside the other provider
/// constants so future tuning is explicit.
pub const POLL_INTERVAL_SECS: u64 = 3;
/// Upper bound for non-terminal AssemblyAI polling responses. With the
/// 3s interval this allows roughly 30 minutes after the first status
/// request, which is long enough for slow files without letting the
/// worker loop forever.
pub const MAX_POLL_ATTEMPTS: u16 = 600;

/// Wire shape for the `speech_model` / `speech_models` field on
/// `POST /v2/transcript`. Each variant produces a different JSON layout
/// in [`super::payloads::build_create_transcript_payload`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaKind {
    /// Current official AssemblyAI protocol: `speech_models` is a list of
    /// speech models in priority order. If omitted, AssemblyAI defaults
    /// to `["universal-3-pro", "universal-2"]`.
    SpeechModelsList,
    /// Legacy AssemblyAI protocol: `speech_model` is a single model name
    /// (e.g. `"universal"`). Kept as a fallback in case the current
    /// schema is ever rejected with a parameter-name compatibility
    /// error.
    SpeechModelSingle,
}

impl SchemaKind {
    /// Canonical JSON field name for this schema kind.
    pub fn field_name(&self) -> &'static str {
        match self {
            SchemaKind::SpeechModelsList => "speech_models",
            SchemaKind::SpeechModelSingle => "speech_model",
        }
    }
}

/// One entry in the schema fallback chain. The first schema is the
/// current protocol; subsequent schemas are tried only if AssemblyAI
/// returns an HTTP 400 whose body suggests the field we sent was
/// obsolete, unknown, or incompatible.
#[derive(Debug, Clone, Deserialize)]
pub struct SchemaSpec {
    /// Human-readable identifier used in logs and error messages
    /// (e.g. `"speech_models_list"`, `"speech_model_legacy"`).
    pub name: String,
    /// Wire shape for the speech-model field.
    pub kind: SchemaKind,
}

/// AssemblyAI provider config: endpoint, default model list (used by
/// the current schema and the legacy fallback), and the ordered list
/// of schema variants to try.
#[derive(Debug, Clone)]
pub struct AssemblyAiConfig {
    /// API base URL (no trailing slash). Always the trusted official
    /// AssemblyAI base: `https://api.assemblyai.com/v2`.
    pub api_base: String,
    /// Models to send in priority order for [`SchemaKind::SpeechModelsList`]
    /// (current protocol) and as the single model for
    /// [`SchemaKind::SpeechModelSingle`] (legacy fallback uses the
    /// first entry).
    pub default_models: Vec<String>,
    /// Ordered fallback chain. Index 0 is the current protocol and is
    /// tried first; later entries are only tried on a compatible 400.
    pub schemas: Vec<SchemaSpec>,
}

impl Default for AssemblyAiConfig {
    fn default() -> Self {
        Self {
            api_base: DEFAULT_API_BASE.to_string(),
            default_models: vec![
                DEFAULT_MODEL_PRIMARY.to_string(),
                DEFAULT_MODEL_FALLBACK.to_string(),
            ],
            schemas: vec![
                SchemaSpec {
                    name: "speech_models_list".to_string(),
                    kind: SchemaKind::SpeechModelsList,
                },
                SchemaSpec {
                    name: "speech_model_single_legacy".to_string(),
                    kind: SchemaKind::SpeechModelSingle,
                },
            ],
        }
    }
}

impl AssemblyAiConfig {
    /// Load the bundled `provider-compatibility.json` resource if
    /// present, otherwise return the built-in defaults.
    ///
    /// The lookup checks (in order):
    /// 1. `<exe-dir>/resources/provider-compatibility.json` (Tauri
    ///    production layout — `tauri.conf.json` bundles `resources/*`
    ///    next to the binary).
    /// 2. `<exe-dir>/_up_/resources/provider-compatibility.json`
    ///    (Tauri NSIS installer nests resources one level up).
    /// 3. `$CARGO_MANIFEST_DIR/resources/provider-compatibility.json`
    ///    (covers `cargo run` and `cargo test` from source).
    ///
    /// On any read/parse/safety failure the loader logs a warning and
    /// returns [`AssemblyAiConfig::default`] — dictation must still work
    /// offline, even with a malformed bundled config.
    pub fn load() -> Self {
        for path in bundled_candidate_paths() {
            match std::fs::read_to_string(&path) {
                Ok(contents) => match parse_and_validate(&contents) {
                    Ok(config) => {
                        if config.schemas.is_empty() {
                            eprintln!(
                                "[assemblyai] provider-compatibility.json at {} has no schemas; using built-in defaults",
                                path.display()
                            );
                            return AssemblyAiConfig::default();
                        }
                        return config;
                    }
                    Err(error) => {
                        eprintln!(
                            "[assemblyai] provider-compatibility.json at {} is invalid: {error}; using built-in defaults",
                            path.display()
                        );
                        return AssemblyAiConfig::default();
                    }
                },
                Err(_) => continue,
            }
        }
        AssemblyAiConfig::default()
    }
}

fn bundled_candidate_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            paths.push(parent.join("resources/provider-compatibility.json"));
            paths.push(parent.join("_up_/resources/provider-compatibility.json"));
        }
    }
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        paths.push(
            std::path::PathBuf::from(manifest_dir).join("resources/provider-compatibility.json"),
        );
    }
    paths
}

#[derive(Deserialize)]
struct ProviderCompatibilityFile {
    #[serde(default)]
    #[allow(dead_code)]
    version: u32,
    #[serde(default)]
    providers: Providers,
}

#[derive(Deserialize, Default)]
struct Providers {
    #[serde(default)]
    assemblyai: Option<ProviderEntry>,
}

#[derive(Deserialize)]
struct ProviderEntry {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    default_models: Option<Vec<String>>,
    schemas: Vec<SchemaSpec>,
}

fn parse_and_validate(contents: &str) -> Result<AssemblyAiConfig, String> {
    let value: serde_json::Value =
        serde_json::from_str(contents).map_err(|e| format!("invalid JSON: {e}"))?;
    validate_no_credentials(&value, "$")?;

    let parsed: ProviderCompatibilityFile =
        serde_json::from_value(value).map_err(|e| format!("invalid JSON: {e}"))?;

    let entry = parsed
        .providers
        .assemblyai
        .ok_or_else(|| "missing providers.assemblyai entry".to_string())?;

    if entry.schemas.is_empty() {
        return Err("providers.assemblyai.schemas must not be empty".to_string());
    }

    if let Some(api_base) = entry.api_base.as_deref() {
        if normalize_api_base(api_base) != DEFAULT_API_BASE {
            return Err(format!(
                "providers.assemblyai.api_base must be the official AssemblyAI endpoint ({DEFAULT_API_BASE})"
            ));
        }
    }

    let default_models = match entry.default_models {
        Some(models) => validate_default_models(models)?,
        None => vec![
            DEFAULT_MODEL_PRIMARY.to_string(),
            DEFAULT_MODEL_FALLBACK.to_string(),
        ],
    };

    Ok(AssemblyAiConfig {
        api_base: DEFAULT_API_BASE.to_string(),
        default_models,
        schemas: entry.schemas,
    })
}

fn validate_default_models(models: Vec<String>) -> Result<Vec<String>, String> {
    let normalized = models
        .into_iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect::<Vec<_>>();
    if normalized.is_empty() {
        return Err("providers.assemblyai.default_models must not be empty".to_string());
    }
    Ok(normalized)
}

fn normalize_api_base(value: &str) -> &str {
    value.trim().trim_end_matches('/')
}

fn validate_no_credentials(value: &serde_json::Value, path: &str) -> Result<(), String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let child_path = format!("{path}.{key}");
                if credential_key(key) {
                    return Err(format!("must not contain credential key at {child_path}"));
                }
                validate_no_credentials(child, &child_path)?;
            }
            Ok(())
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                validate_no_credentials(child, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        serde_json::Value::String(text) => {
            if credential_assignment(text) {
                return Err(format!(
                    "must not contain credential-bearing string at {path}"
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn credential_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "apikey" | "token" | "secret" | "password" | "accesstoken" | "refreshtoken"
    )
}

fn credential_assignment(text: &str) -> bool {
    let lowered = text.to_lowercase();
    const NAMES: &[&str] = &[
        "api_key",
        "apikey",
        "api-key",
        "token",
        "secret",
        "password",
        "access_token",
        "refresh_token",
    ];
    const ASSIGNERS: &[&str] = &["=", ":", "%3d"];

    NAMES.iter().any(|name| {
        ASSIGNERS.iter().any(|assigner| {
            lowered.contains(&format!("{name}{assigner}"))
                || lowered.contains(&format!("?{name}{assigner}"))
                || lowered.contains(&format!("&{name}{assigner}"))
        })
    })
}

/// Convenience: the single model string sent under
/// [`SchemaKind::SpeechModelSingle`]. Falls back to `"universal"` if
/// the configured default list is empty (shouldn't happen with the
/// default config — this is just defensive).
pub fn legacy_single_model(config: &AssemblyAiConfig) -> String {
    config
        .default_models
        .first()
        .cloned()
        .unwrap_or_else(|| DEFAULT_LEGACY_MODEL.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_current_protocol_first_and_legacy_fallback_second() {
        let config = AssemblyAiConfig::default();
        assert_eq!(config.api_base, DEFAULT_API_BASE);
        assert_eq!(
            config.default_models,
            vec![
                DEFAULT_MODEL_PRIMARY.to_string(),
                DEFAULT_MODEL_FALLBACK.to_string()
            ]
        );
        assert_eq!(config.schemas.len(), 2);
        assert_eq!(config.schemas[0].kind, SchemaKind::SpeechModelsList);
        assert_eq!(config.schemas[1].kind, SchemaKind::SpeechModelSingle);
    }

    #[test]
    fn schema_kind_field_names_match_official_protocol() {
        // Pins the wire contract: current protocol = `speech_models`
        // (list), legacy = `speech_model` (single). If AssemblyAI
        // renames either, both this enum and the JSON config need to
        // move together.
        assert_eq!(SchemaKind::SpeechModelsList.field_name(), "speech_models");
        assert_eq!(SchemaKind::SpeechModelSingle.field_name(), "speech_model");
    }

    #[test]
    fn parse_accepts_minimal_provider_entry() {
        let json = r#"{
            "version": 1,
            "providers": {
                "assemblyai": {
                    "schemas": [
                        { "name": "speech_models_list", "kind": "speech_models_list" },
                        { "name": "speech_model_single", "kind": "speech_model_single" }
                    ]
                }
            }
        }"#;
        let config = parse_and_validate(json).expect("valid bundled config");
        assert_eq!(config.api_base, DEFAULT_API_BASE);
        assert_eq!(
            config.default_models,
            vec![
                DEFAULT_MODEL_PRIMARY.to_string(),
                DEFAULT_MODEL_FALLBACK.to_string()
            ]
        );
        assert_eq!(config.schemas.len(), 2);
    }

    #[test]
    fn bundled_provider_compatibility_json_parses_without_self_rejection() {
        let config = parse_and_validate(include_str!(
            "../../../resources/provider-compatibility.json"
        ))
        .expect("bundled provider-compatibility.json must parse");

        assert_eq!(config.api_base, DEFAULT_API_BASE);
        assert_eq!(config.schemas[0].kind, SchemaKind::SpeechModelsList);
    }

    #[test]
    fn parse_accepts_official_api_base_and_model_overrides() {
        let json = r#"{
            "providers": {
                "assemblyai": {
                    "api_base": "https://api.assemblyai.com/v2/",
                    "default_models": ["universal-3-pro"],
                    "schemas": [
                        { "name": "current", "kind": "speech_models_list" }
                    ]
                }
            }
        }"#;
        let config = parse_and_validate(json).expect("valid bundled config");
        assert_eq!(config.api_base, DEFAULT_API_BASE);
        assert_eq!(config.default_models, vec!["universal-3-pro"]);
        assert_eq!(config.schemas.len(), 1);
    }

    #[test]
    fn parse_rejects_empty_default_models_override() {
        let json = r#"{
            "providers": {
                "assemblyai": {
                    "default_models": [],
                    "schemas": [
                        { "name": "current", "kind": "speech_models_list" }
                    ]
                }
            }
        }"#;
        let err = parse_and_validate(json).expect_err("empty default_models must be rejected");
        assert!(err.contains("default_models"), "got: {err}");
    }

    #[test]
    fn parse_rejects_blank_default_models_override() {
        let json = r#"{
            "providers": {
                "assemblyai": {
                    "default_models": ["", "  "],
                    "schemas": [
                        { "name": "current", "kind": "speech_models_list" }
                    ]
                }
            }
        }"#;
        let err = parse_and_validate(json).expect_err("blank default_models must be rejected");
        assert!(err.contains("default_models"), "got: {err}");
    }

    #[test]
    fn parse_rejects_malicious_api_base_redirect() {
        let json = r#"{
            "providers": {
                "assemblyai": {
                    "api_base": "https://evil.example.test/v2",
                    "schemas": [
                        { "name": "current", "kind": "speech_models_list" }
                    ]
                }
            }
        }"#;
        let err = parse_and_validate(json).expect_err("non-official api_base must be rejected");
        assert!(err.contains("official AssemblyAI endpoint"), "got: {err}");
    }

    #[test]
    fn parse_rejects_credentials_in_bundled_config() {
        // SAFETY: a remote fetch must never smuggle API keys through
        // this file — even if some other code path tried to write it,
        // the loader must refuse actual credential-shaped keys or
        // credential-bearing string assignments.
        let cases = [
            r#"{ "providers": { "assemblyai": { "api_key": "leak", "schemas": [{ "name": "x", "kind": "speech_models_list" }] } } }"#,
            r#"{ "providers": { "assemblyai": { "API_KEY": "leak", "schemas": [{ "name": "x", "kind": "speech_models_list" }] } } }"#,
            r#"{ "providers": { "assemblyai": { "token": "leak", "schemas": [{ "name": "x", "kind": "speech_models_list" }] } } }"#,
            r#"{ "providers": { "assemblyai": { "schemas": [{ "name": "x", "kind": "speech_models_list" }] } }, "_meta": { "url": "https://example.test/config?api_key=leak" } }"#,
            r#"{ "providers": { "assemblyai": { "schemas": [{ "name": "x", "kind": "speech_models_list" }] } }, "_meta": { "assignment": "token: leak" } }"#,
        ];
        for json in cases {
            let err = parse_and_validate(json).expect_err("credential must be rejected");
            assert!(
                err.contains("credential"),
                "expected credentials error, got: {err}"
            );
        }
    }

    #[test]
    fn parse_allows_explanatory_credential_text_in_metadata() {
        let json = r#"{
            "version": 1,
            "_meta": {
                "remote_update_policy": "Remote copies must never contain API keys, tokens, secrets, or passwords."
            },
            "providers": {
                "assemblyai": {
                    "schemas": [
                        { "name": "speech_models_list", "kind": "speech_models_list" }
                    ]
                }
            }
        }"#;
        let config = parse_and_validate(json).expect("explanatory metadata must be allowed");
        assert_eq!(config.api_base, DEFAULT_API_BASE);
    }

    #[test]
    fn parse_rejects_empty_schema_chain() {
        let json = r#"{ "providers": { "assemblyai": { "schemas": [] } } }"#;
        let err = parse_and_validate(json).expect_err("empty schemas must be rejected");
        assert!(err.contains("schemas"), "got: {err}");
    }

    #[test]
    fn parse_rejects_unknown_schema_kind() {
        let json = r#"{ "providers": { "assemblyai": { "schemas": [{ "name": "x", "kind": "speech_models_v3" }] } } }"#;
        let err = parse_and_validate(json).expect_err("unknown kind must be rejected");
        assert!(
            err.contains("invalid JSON") || err.contains("JSON"),
            "got: {err}"
        );
    }

    #[test]
    fn legacy_single_model_uses_first_default() {
        let config = AssemblyAiConfig {
            default_models: vec!["foo".to_string(), "bar".to_string()],
            ..AssemblyAiConfig::default()
        };
        assert_eq!(legacy_single_model(&config), "foo");
    }

    #[test]
    fn legacy_single_model_falls_back_when_no_defaults_configured() {
        let config = AssemblyAiConfig {
            default_models: Vec::new(),
            ..AssemblyAiConfig::default()
        };
        assert_eq!(legacy_single_model(&config), DEFAULT_LEGACY_MODEL);
    }
}
