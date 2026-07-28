use rusqlite::Connection;

use crate::{llm::prompt, settings};

pub const DEFAULT_NER_OPENROUTER_MODEL: &str = settings::DEFAULT_OPENROUTER_MODEL;
const DEFAULT_TEMPERATURE: f32 = 0.3;
const DEFAULT_TOP_P: f32 = 1.0;
const DEFAULT_TOP_K: i32 = 0;
const DEFAULT_PRESENCE_PENALTY: f32 = 0.0;
const DEFAULT_FREQUENCY_PENALTY: f32 = 0.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationFlow {
    OcrCorrection,
    Summary,
    Ner,
    Triplets,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenRouterGenerationParams {
    pub max_tokens: i32,
    pub temperature: f32,
    pub top_p: Option<f32>,
    pub top_k: Option<i32>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub stop: Vec<String>,
}

impl OpenRouterGenerationParams {
    pub fn provider_defaults(max_tokens: i32, temperature: f32) -> Self {
        Self {
            max_tokens,
            temperature,
            top_p: None,
            top_k: None,
            presence_penalty: None,
            frequency_penalty: None,
            stop: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowGenerationConfig {
    pub model: String,
    pub params: OpenRouterGenerationParams,
}

#[derive(Clone, Copy)]
struct FlowSettings {
    prompt: &'static str,
    model: &'static str,
    temperature: &'static str,
    max_tokens: &'static str,
    top_p: &'static str,
    top_k: &'static str,
    presence_penalty: &'static str,
    frequency_penalty: &'static str,
    stop_sequences: &'static str,
    default_prompt: &'static str,
    default_model: &'static str,
    default_max_tokens: i32,
}

impl GenerationFlow {
    fn settings(self) -> FlowSettings {
        match self {
            Self::OcrCorrection => FlowSettings {
                prompt: "prompt_ocr_correction",
                model: "llm_ocr_correction_model",
                temperature: "llm_ocr_correction_temperature",
                max_tokens: "llm_ocr_correction_max_tokens",
                top_p: "llm_ocr_correction_top_p",
                top_k: "llm_ocr_correction_top_k",
                presence_penalty: "llm_ocr_correction_presence_penalty",
                frequency_penalty: "llm_ocr_correction_frequency_penalty",
                stop_sequences: "llm_ocr_correction_stop_sequences",
                default_prompt: prompt::DEFAULT_OCR_CORRECTION_PROMPT,
                default_model: settings::DEFAULT_OPENROUTER_MODEL,
                default_max_tokens: 8192,
            },
            Self::Summary => FlowSettings {
                prompt: "prompt_summary",
                model: "llm_summary_model",
                temperature: "llm_summary_temperature",
                max_tokens: "llm_summary_max_tokens",
                top_p: "llm_summary_top_p",
                top_k: "llm_summary_top_k",
                presence_penalty: "llm_summary_presence_penalty",
                frequency_penalty: "llm_summary_frequency_penalty",
                stop_sequences: "llm_summary_stop_sequences",
                default_prompt: prompt::DEFAULT_SUMMARY_PROMPT,
                default_model: settings::DEFAULT_OPENROUTER_MODEL,
                default_max_tokens: 1024,
            },
            Self::Ner => FlowSettings {
                prompt: "prompt_ner",
                // Preserve the existing NER model key so current overrides keep working.
                model: "openrouter_ner_model",
                temperature: "llm_ner_temperature",
                max_tokens: "llm_ner_max_tokens",
                top_p: "llm_ner_top_p",
                top_k: "llm_ner_top_k",
                presence_penalty: "llm_ner_presence_penalty",
                frequency_penalty: "llm_ner_frequency_penalty",
                stop_sequences: "llm_ner_stop_sequences",
                default_prompt: prompt::DEFAULT_NER_PROMPT,
                default_model: DEFAULT_NER_OPENROUTER_MODEL,
                default_max_tokens: 4096,
            },
            Self::Triplets => FlowSettings {
                prompt: "prompt_triplets",
                model: "llm_triplets_model",
                temperature: "llm_triplets_temperature",
                max_tokens: "llm_triplets_max_tokens",
                top_p: "llm_triplets_top_p",
                top_k: "llm_triplets_top_k",
                presence_penalty: "llm_triplets_presence_penalty",
                frequency_penalty: "llm_triplets_frequency_penalty",
                stop_sequences: "llm_triplets_stop_sequences",
                default_prompt: prompt::DEFAULT_TRIPLETS_PROMPT,
                default_model: settings::DEFAULT_OPENROUTER_MODEL,
                default_max_tokens: 4096,
            },
        }
    }

    pub fn defaults(self) -> FlowGenerationConfig {
        let settings = self.settings();
        FlowGenerationConfig {
            model: settings.default_model.to_string(),
            params: OpenRouterGenerationParams {
                max_tokens: settings.default_max_tokens,
                temperature: DEFAULT_TEMPERATURE,
                top_p: Some(DEFAULT_TOP_P),
                top_k: Some(DEFAULT_TOP_K),
                presence_penalty: Some(DEFAULT_PRESENCE_PENALTY),
                frequency_penalty: Some(DEFAULT_FREQUENCY_PENALTY),
                stop: Vec::new(),
            },
        }
    }

    fn accepts_prompt(self, template: &str) -> bool {
        if !template.contains("{text}") {
            return false;
        }

        match self {
            Self::Ner => ["PER", "LOC", "ORG", "DATE", "MISC"]
                .iter()
                .all(|label| template.contains(label)),
            Self::Triplets => ["subject", "predicate", "object"]
                .iter()
                .all(|key| template.contains(key)),
            Self::OcrCorrection | Self::Summary => true,
        }
    }
}

pub fn prompt_template_from_settings(conn: &Connection, flow: GenerationFlow) -> String {
    let keys = flow.settings();
    non_empty_setting(conn, keys.prompt)
        .filter(|template| flow.accepts_prompt(template))
        .unwrap_or_else(|| keys.default_prompt.to_string())
}

pub fn render_prompt_from_settings(conn: &Connection, flow: GenerationFlow, text: &str) -> String {
    prompt::render_template(&prompt_template_from_settings(conn, flow), text)
}

pub fn generation_config_from_settings(
    conn: &Connection,
    flow: GenerationFlow,
) -> FlowGenerationConfig {
    let keys = flow.settings();
    let defaults = flow.defaults();
    let model = non_empty_setting(conn, keys.model)
        .or_else(|| non_empty_setting(conn, "openrouter_model"))
        .unwrap_or(defaults.model);

    FlowGenerationConfig {
        model,
        params: OpenRouterGenerationParams {
            max_tokens: parse_i32_setting(
                conn,
                keys.max_tokens,
                defaults.params.max_tokens,
                1,
                16_000,
            ),
            temperature: parse_f32_setting(
                conn,
                keys.temperature,
                defaults.params.temperature,
                0.0,
                2.0,
            ),
            top_p: Some(parse_f32_setting(
                conn,
                keys.top_p,
                defaults.params.top_p.unwrap_or(DEFAULT_TOP_P),
                0.0,
                1.0,
            )),
            top_k: Some(parse_i32_setting(
                conn,
                keys.top_k,
                defaults.params.top_k.unwrap_or(DEFAULT_TOP_K),
                0,
                1000,
            )),
            presence_penalty: Some(parse_f32_setting(
                conn,
                keys.presence_penalty,
                defaults
                    .params
                    .presence_penalty
                    .unwrap_or(DEFAULT_PRESENCE_PENALTY),
                -2.0,
                2.0,
            )),
            frequency_penalty: Some(parse_f32_setting(
                conn,
                keys.frequency_penalty,
                defaults
                    .params
                    .frequency_penalty
                    .unwrap_or(DEFAULT_FREQUENCY_PENALTY),
                -2.0,
                2.0,
            )),
            stop: settings::get_setting(conn, keys.stop_sequences)
                .unwrap_or_default()
                .lines()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .take(4)
                .map(str::to_string)
                .collect(),
        },
    }
}

fn non_empty_setting(conn: &Connection, key: &str) -> Option<String> {
    settings::get_setting(conn, key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_i32_setting(conn: &Connection, key: &str, default: i32, min: i32, max: i32) -> i32 {
    settings::get_setting(conn, key)
        .and_then(|value| value.trim().parse::<i32>().ok())
        .filter(|value| *value >= min && *value <= max)
        .unwrap_or(default)
}

fn parse_f32_setting(conn: &Connection, key: &str, default: f32, min: f32, max: f32) -> f32 {
    settings::get_setting(conn, key)
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value >= min && *value <= max)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn exposes_the_effective_defaults_for_all_user_visible_flows() {
        let expected = [
            (
                GenerationFlow::OcrCorrection,
                settings::DEFAULT_OPENROUTER_MODEL,
                8192,
            ),
            (
                GenerationFlow::Summary,
                settings::DEFAULT_OPENROUTER_MODEL,
                1024,
            ),
            (GenerationFlow::Ner, DEFAULT_NER_OPENROUTER_MODEL, 4096),
            (
                GenerationFlow::Triplets,
                settings::DEFAULT_OPENROUTER_MODEL,
                4096,
            ),
        ];

        for (flow, model, max_tokens) in expected {
            let config = flow.defaults();
            assert_eq!(config.model, model);
            assert_eq!(config.params.max_tokens, max_tokens);
            assert_eq!(config.params.temperature, 0.3);
            assert_eq!(config.params.top_p, Some(1.0));
            assert_eq!(config.params.top_k, Some(0));
            assert_eq!(config.params.presence_penalty, Some(0.0));
            assert_eq!(config.params.frequency_penalty, Some(0.0));
            assert!(config.params.stop.is_empty());
        }
    }

    #[test]
    fn resolves_every_ner_override_from_settings() {
        let conn = settings_conn();
        let values = [
            ("openrouter_ner_model", "openai/gpt-test"),
            ("llm_ner_temperature", "0.1"),
            ("llm_ner_max_tokens", "4096"),
            ("llm_ner_top_p", "0.8"),
            ("llm_ner_top_k", "25"),
            ("llm_ner_presence_penalty", "0.2"),
            ("llm_ner_frequency_penalty", "-0.3"),
            ("llm_ner_stop_sequences", "END\nSTOP"),
        ];
        for (key, value) in values {
            settings::set_setting(&conn, key, value).unwrap();
        }

        let config = generation_config_from_settings(&conn, GenerationFlow::Ner);
        assert_eq!(config.model, "openai/gpt-test");
        assert_eq!(config.params.max_tokens, 4096);
        assert_eq!(config.params.temperature, 0.1);
        assert_eq!(config.params.top_p, Some(0.8));
        assert_eq!(config.params.top_k, Some(25));
        assert_eq!(config.params.presence_penalty, Some(0.2));
        assert_eq!(config.params.frequency_penalty, Some(-0.3));
        assert_eq!(config.params.stop, ["END", "STOP"]);
    }

    #[test]
    fn resolves_independent_models_and_token_limits_for_all_flows() {
        let conn = settings_conn();
        let cases = [
            (
                GenerationFlow::OcrCorrection,
                "llm_ocr_correction_model",
                "llm_ocr_correction_max_tokens",
                "vendor/ocr",
                3000,
            ),
            (
                GenerationFlow::Summary,
                "llm_summary_model",
                "llm_summary_max_tokens",
                "vendor/summary",
                700,
            ),
            (
                GenerationFlow::Ner,
                "openrouter_ner_model",
                "llm_ner_max_tokens",
                "vendor/ner",
                4096,
            ),
            (
                GenerationFlow::Triplets,
                "llm_triplets_model",
                "llm_triplets_max_tokens",
                "vendor/triplets",
                1800,
            ),
        ];

        for (flow, model_key, max_tokens_key, model, max_tokens) in cases {
            settings::set_setting(&conn, model_key, model).unwrap();
            settings::set_setting(&conn, max_tokens_key, &max_tokens.to_string()).unwrap();
            let config = generation_config_from_settings(&conn, flow);
            assert_eq!(config.model, model);
            assert_eq!(config.params.max_tokens, max_tokens);
        }
    }

    #[test]
    fn falls_back_to_the_legacy_global_model_and_safe_numeric_defaults() {
        let conn = settings_conn();
        settings::set_setting(&conn, "openrouter_model", "anthropic/legacy").unwrap();
        settings::set_setting(&conn, "llm_summary_max_tokens", "0").unwrap();
        settings::set_setting(&conn, "llm_summary_temperature", "NaN").unwrap();

        let config = generation_config_from_settings(&conn, GenerationFlow::Summary);
        assert_eq!(config.model, "anthropic/legacy");
        assert_eq!(config.params.max_tokens, 1024);
        assert_eq!(config.params.temperature, 0.3);
    }

    #[test]
    fn rejects_generation_limits_above_sixteen_thousand_tokens() {
        let conn = settings_conn();
        settings::set_setting(&conn, "llm_ner_max_tokens", "16001").unwrap();

        let config = generation_config_from_settings(&conn, GenerationFlow::Ner);
        assert_eq!(config.params.max_tokens, 4096);
    }

    #[test]
    fn renders_persisted_prompt_templates_for_every_flow() {
        let conn = settings_conn();
        let cases = [
            (
                GenerationFlow::OcrCorrection,
                "prompt_ocr_correction",
                "Custom OCR: {text}",
                "Custom OCR: source text",
            ),
            (
                GenerationFlow::Summary,
                "prompt_summary",
                "Custom summary: {text}",
                "Custom summary: source text",
            ),
            (
                GenerationFlow::Ner,
                "prompt_ner",
                "PER LOC ORG DATE MISC: {text}",
                "PER LOC ORG DATE MISC: source text",
            ),
            (
                GenerationFlow::Triplets,
                "prompt_triplets",
                "subject predicate object: {text}",
                "subject predicate object: source text",
            ),
        ];

        for (flow, key, template, expected) in cases {
            settings::set_setting(&conn, key, template).unwrap();
            assert_eq!(
                render_prompt_from_settings(&conn, flow, "source text"),
                expected
            );
        }
    }

    #[test]
    fn invalid_or_empty_prompt_templates_fall_back_to_defaults() {
        let conn = settings_conn();
        settings::set_setting(&conn, "prompt_ocr_correction", "missing placeholder").unwrap();
        settings::set_setting(&conn, "prompt_ner", "NER without labels {text}").unwrap();
        settings::set_setting(&conn, "prompt_triplets", "Triplets without keys {text}").unwrap();

        assert_eq!(
            prompt_template_from_settings(&conn, GenerationFlow::OcrCorrection),
            prompt::DEFAULT_OCR_CORRECTION_PROMPT
        );
        assert_eq!(
            prompt_template_from_settings(&conn, GenerationFlow::Ner),
            prompt::DEFAULT_NER_PROMPT
        );
        assert_eq!(
            prompt_template_from_settings(&conn, GenerationFlow::Triplets),
            prompt::DEFAULT_TRIPLETS_PROMPT
        );
        assert_eq!(
            prompt_template_from_settings(&conn, GenerationFlow::Summary),
            prompt::DEFAULT_SUMMARY_PROMPT
        );
    }
}
