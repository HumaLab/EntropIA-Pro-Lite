//! Help assistant settings (T11, S14, D10): the model override, the
//! manual/excerpt budget, the generation limits, the history cap and the
//! local-answers switch, all stored in `app_settings` like the other flows.
//!
//! One pure resolver reads them through a lookup closure, so clamping,
//! defaults and the model fallback chain are unit-testable with no database
//! and no network; `commands.rs` only supplies the closure and applies the
//! resolved values.

use super::faq::{answer_locally, LocalAnswer};
use super::intent::is_action_request;

/// Manual/excerpt budget: characters sent with a question (D10).
pub(crate) const HELP_CONTEXT_CHARS_DEFAULT: usize = 24_000;
pub(crate) const HELP_CONTEXT_CHARS_MIN: usize = 4_000;
pub(crate) const HELP_CONTEXT_CHARS_MAX: usize = 60_000;
/// Generation budget for a help answer (D10).
pub(crate) const HELP_MAX_TOKENS_DEFAULT: i32 = 1500;
pub(crate) const HELP_MAX_TOKENS_MIN: i32 = 200;
pub(crate) const HELP_MAX_TOKENS_MAX: i32 = 8000;
/// Answer randomness (D10).
pub(crate) const HELP_TEMPERATURE_DEFAULT: f32 = 0.2;
pub(crate) const HELP_TEMPERATURE_MIN: f32 = 0.0;
pub(crate) const HELP_TEMPERATURE_MAX: f32 = 1.0;
/// History turns sent to the model (D10).
pub(crate) const HELP_HISTORY_TURNS_DEFAULT: usize = 6;
pub(crate) const HELP_HISTORY_TURNS_MAX: usize = 12;

/// The resolved help settings `help_ask` runs with.
pub(crate) struct HelpSettings {
    pub model: String,
    pub context_chars: usize,
    pub max_tokens: i32,
    pub temperature: f32,
    pub history_turns: usize,
    pub local_answers: bool,
}

/// Reads the `help_*` keys (stored like every other setting) with clamping
/// and defaults. `help_model` empty or missing inherits the general
/// `openrouter_model`, then the product default; every other key falls back
/// to its default when missing, blank or unparsable.
pub(crate) fn resolve_help_settings(lookup: &dyn Fn(&str) -> Option<String>) -> HelpSettings {
    let raw = |key: &str| {
        lookup(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let model = raw("help_model")
        .or_else(|| raw("openrouter_model"))
        .unwrap_or_else(|| crate::settings::DEFAULT_OPENROUTER_MODEL.to_string());
    HelpSettings {
        model,
        context_chars: parse_clamped(
            raw("help_context_chars"),
            HELP_CONTEXT_CHARS_MIN as i64,
            HELP_CONTEXT_CHARS_MAX as i64,
            HELP_CONTEXT_CHARS_DEFAULT as i64,
        ) as usize,
        max_tokens: parse_clamped(
            raw("help_max_tokens"),
            HELP_MAX_TOKENS_MIN as i64,
            HELP_MAX_TOKENS_MAX as i64,
            HELP_MAX_TOKENS_DEFAULT as i64,
        ) as i32,
        temperature: parse_temperature(raw("help_temperature")),
        history_turns: parse_clamped(
            raw("help_history_turns"),
            0,
            HELP_HISTORY_TURNS_MAX as i64,
            HELP_HISTORY_TURNS_DEFAULT as i64,
        ) as usize,
        local_answers: parse_enabled(raw("help_local_answers")),
    }
}

/// An integer setting: missing, blank or unparsable means the default,
/// a parsed value is clamped into `[min, max]` (negatives land on `min`).
fn parse_clamped(raw: Option<String>, min: i64, max: i64, default: i64) -> i64 {
    raw.and_then(|value| value.parse::<i64>().ok())
        .map(|parsed| parsed.clamp(min, max))
        .unwrap_or(default)
}

/// The temperature setting: like [`parse_clamped`] over 0–1, with
/// non-finite parses (`NaN`, infinities) falling back to the default.
fn parse_temperature(raw: Option<String>) -> f32 {
    raw.and_then(|value| value.parse::<f32>().ok())
        .filter(|parsed| parsed.is_finite())
        .map(|parsed| parsed.clamp(HELP_TEMPERATURE_MIN, HELP_TEMPERATURE_MAX))
        .unwrap_or(HELP_TEMPERATURE_DEFAULT)
}

/// The local-answers switch: missing or blank is on; only an explicit off
/// word (`0`, `false`, `no`, `off`) turns it off.
fn parse_enabled(raw: Option<String>) -> bool {
    match raw {
        None => true,
        Some(value) => !matches!(
            value.trim().to_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
    }
}

/// The deterministic local answer for `question`, or `None` when it must go
/// to the model: the switch is off, diagnostics travel along (they need the
/// model), the user opted into document excerpts (the question needs the
/// archive), or the question asks the assistant to do something (S17/D15:
/// action requests go to the model, which T20–T22 give tools to). Pure and
/// synchronous: no key, no network.
pub(crate) fn local_answer_for(
    settings: &HelpSettings,
    diagnostics: Option<&str>,
    documents: bool,
    question: &str,
) -> Option<LocalAnswer> {
    if !settings.local_answers {
        return None;
    }
    if documents {
        return None;
    }
    if is_action_request(question) {
        return None;
    }
    answer_locally(diagnostics, question)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::settings::DEFAULT_OPENROUTER_MODEL;

    fn lookup(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn defaults_match_the_previous_hard_coded_contract() {
        // The values `help_ask` used before the settings existed
        // (24_000 / 1500 / 0.2 / 6) stay the defaults, and the history default
        // stays the frontend cap (`help.ts` slices to 6 before sending), so
        // both ends agree out of the box.
        assert_eq!(HELP_CONTEXT_CHARS_DEFAULT, 24_000);
        assert_eq!(HELP_MAX_TOKENS_DEFAULT, 1500);
        assert_eq!(HELP_TEMPERATURE_DEFAULT, 0.2);
        assert_eq!(HELP_HISTORY_TURNS_DEFAULT, 6);
    }

    #[test]
    fn empty_settings_resolve_to_defaults_inheriting_the_general_model() {
        let stored = lookup(&[("openrouter_model", "vendor/general-model")]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        assert_eq!(help.model, "vendor/general-model");
        assert_eq!(help.context_chars, HELP_CONTEXT_CHARS_DEFAULT);
        assert_eq!(help.max_tokens, HELP_MAX_TOKENS_DEFAULT);
        assert_eq!(help.temperature, HELP_TEMPERATURE_DEFAULT);
        assert_eq!(help.history_turns, HELP_HISTORY_TURNS_DEFAULT);
        assert!(help.local_answers);
    }

    #[test]
    fn missing_general_model_falls_back_to_the_product_default() {
        let stored = lookup(&[]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        assert_eq!(help.model, DEFAULT_OPENROUTER_MODEL);
    }

    #[test]
    fn help_model_override_wins_and_blank_inherits() {
        let stored = lookup(&[
            ("openrouter_model", "vendor/general-model"),
            ("help_model", "vendor/help-model"),
        ]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        assert_eq!(help.model, "vendor/help-model");

        let stored = lookup(&[
            ("openrouter_model", "vendor/general-model"),
            ("help_model", "   "),
        ]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        assert_eq!(help.model, "vendor/general-model");
    }

    #[test]
    fn numeric_settings_clamp_to_their_ranges_and_reject_garbage() {
        let stored = lookup(&[
            ("help_context_chars", "999999"),
            ("help_max_tokens", "1"),
            ("help_temperature", "5"),
            ("help_history_turns", "99"),
        ]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        assert_eq!(help.context_chars, HELP_CONTEXT_CHARS_MAX);
        assert_eq!(help.max_tokens, HELP_MAX_TOKENS_MIN);
        assert_eq!(help.temperature, HELP_TEMPERATURE_MAX);
        assert_eq!(help.history_turns, HELP_HISTORY_TURNS_MAX);

        let stored = lookup(&[
            ("help_context_chars", "100"),
            ("help_max_tokens", "abc"),
            ("help_temperature", "tibia"),
            ("help_history_turns", "-3"),
        ]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        assert_eq!(help.context_chars, HELP_CONTEXT_CHARS_MIN);
        assert_eq!(help.max_tokens, HELP_MAX_TOKENS_DEFAULT);
        assert_eq!(help.temperature, HELP_TEMPERATURE_DEFAULT);
        assert_eq!(help.history_turns, 0);
    }

    #[test]
    fn local_answers_defaults_to_on_and_honours_off_words() {
        let stored = lookup(&[]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        assert!(help.local_answers);

        for off in ["0", "false", "no", "off", "  OFF  "] {
            let stored = lookup(&[("help_local_answers", off)]);
            let help = resolve_help_settings(&|key| stored.get(key).cloned());
            assert!(!help.local_answers, "{off:?} must switch local answers off");
        }

        let stored = lookup(&[("help_local_answers", "true")]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        assert!(help.local_answers);
    }

    #[test]
    fn local_switch_off_sends_a_faq_question_past_the_local_answer() {
        let stored = lookup(&[]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        let question = "¿Cómo importo un PDF?";
        assert!(
            local_answer_for(&help, None, false, question).is_some(),
            "the FAQ answer must stay local while the switch is on"
        );

        let stored = lookup(&[("help_local_answers", "false")]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        assert!(
            local_answer_for(&help, None, false, question).is_none(),
            "with the switch off the question must go to the model"
        );
    }

    #[test]
    fn diagnostics_never_get_a_local_answer() {
        let stored = lookup(&[]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        assert!(local_answer_for(
            &help,
            Some("Producto: EntropIA Lite"),
            false,
            "¿Cómo importo un PDF?"
        )
        .is_none());
    }

    #[test]
    fn documents_opt_in_bypasses_the_local_answer() {
        // S12: with «Buscar en mis documentos» on, the question needs the
        // archive, so even a perfect FAQ match goes to the model.
        let stored = lookup(&[]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        let question = "¿Cómo importo un PDF?";
        assert!(local_answer_for(&help, None, false, question).is_some());
        assert!(local_answer_for(&help, None, true, question).is_none());
    }

    #[test]
    fn action_requests_skip_the_local_answer() {
        // S17/D15 (L33): a request to do something goes to the model
        // (T20–T22 give it tools), never to a local FAQ answer — even when
        // it matches an entry. The how-to twin still answers locally.
        let stored = lookup(&[]);
        let help = resolve_help_settings(&|key| stored.get(key).cloned());
        assert!(local_answer_for(
            &help,
            None,
            false,
            "Creá una nueva colección con el nombre 'SIMAPE'"
        )
        .is_none());
        assert!(local_answer_for(&help, None, false, "Importá este PDF a mi colección").is_none());
        assert!(local_answer_for(&help, None, false, "¿Cómo creo mi primera colección?").is_some());
    }
}
