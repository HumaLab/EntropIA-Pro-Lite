use serde::Deserialize;

use super::RagChatTurn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntentRoute {
    Direct,
    Retrieval,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentDecision {
    needs_retrieval: bool,
    intent: IntentKind,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IntentKind {
    Conversational,
    CorpusQuestion,
    Ambiguous,
}

pub(crate) fn route_classifier_result<T, E>(result: Result<T, E>) -> IntentRoute
where
    T: AsRef<str>,
{
    match result {
        Ok(raw) => match parse_intent_route(raw.as_ref()) {
            Some(route) => route,
            None => {
                eprintln!("[rag] Intent classifier output rejected; continuing with retrieval");
                IntentRoute::Retrieval
            }
        },
        Err(_) => IntentRoute::Retrieval,
    }
}

fn parse_intent_route(raw: &str) -> Option<IntentRoute> {
    let decision: IntentDecision = serde_json::from_str(raw.trim()).ok()?;
    Some(match (decision.needs_retrieval, decision.intent) {
        (false, IntentKind::Conversational) => IntentRoute::Direct,
        _ => IntentRoute::Retrieval,
    })
}

pub(crate) fn format_history(
    history: &[RagChatTurn],
    max_turns: usize,
    turn_max_chars: usize,
) -> String {
    history
        .iter()
        .skip(history.len().saturating_sub(max_turns))
        .filter(|turn| !turn.content.trim().is_empty())
        .map(|turn| {
            let prefix = if turn.role == "assistant" {
                "Asistente"
            } else {
                "Usuario"
            };
            let content: String = turn.content.trim().chars().take(turn_max_chars).collect();
            format!("{prefix}: {content}")
        })
        .collect::<Vec<String>>()
        .join("\n")
}

pub(crate) fn build_intent_prompt(
    question: &str,
    history: &[RagChatTurn],
    max_turns: usize,
    turn_max_chars: usize,
) -> String {
    let history = format_history(history, max_turns, turn_max_chars);
    let history = if history.is_empty() {
        "(sin historial reciente)"
    } else {
        &history
    };

    format!(
        r#"Clasificá la intención del mensaje actual para decidir si el chat debe consultar su corpus documental.

Devolvé ÚNICAMENTE un objeto JSON con exactamente estas dos propiedades y sin markdown:
{{"needs_retrieval":true|false,"intent":"conversational|corpus_question|ambiguous"}}

Categorías:
- conversational: saludo, agradecimiento, bienestar, capacidades del asistente o funcionamiento general del chat; se puede responder sin evidencia del corpus. Usá needs_retrieval=false.
- corpus_question: pide hechos, comparaciones, citas, fuentes o evidencia que podrían estar en transcripciones o documentos OCR. Usá needs_retrieval=true.
- ambiguous: el historial o el mensaje no alcanzan para asegurar que la respuesta sea puramente conversacional, o la pregunta podría depender del corpus. Usá needs_retrieval=true.

Regla conservadora: ante cualquier duda elegí ambiguous y needs_retrieval=true. El historial solo ayuda a interpretar seguimientos; nunca convierte una pregunta documental en conversacional.

Ejemplos:
Hola => {{"needs_retrieval":false,"intent":"conversational"}}
¿Qué podés hacer? => {{"needs_retrieval":false,"intent":"conversational"}}
Gracias => {{"needs_retrieval":false,"intent":"conversational"}}
¿Qué huelgas protagonizó el SOIP en 1965? => {{"needs_retrieval":true,"intent":"corpus_question"}}
¿Qué dicen las fuentes sobre el conflicto de abril? => {{"needs_retrieval":true,"intent":"corpus_question"}}
¿Hay evidencia de conflictos laborales durante 1965? => {{"needs_retrieval":true,"intent":"ambiguous"}}

Historial reciente:
{history}

Mensaje actual:
{question}"#
    )
}

pub(crate) enum IntentProvider<'a> {
    OpenRouter {
        api_key: &'a str,
        model: &'a str,
    },
    #[cfg(feature = "local-ml")]
    Local {
        app_handle: &'a tauri::AppHandle,
        db_path: &'a std::path::Path,
    },
}

pub(crate) async fn classify_intent(
    provider: IntentProvider<'_>,
    question: &str,
    history: &[RagChatTurn],
    max_turns: usize,
    turn_max_chars: usize,
) -> IntentRoute {
    let prompt = build_intent_prompt(question, history, max_turns, turn_max_chars);
    let generated = match provider {
        IntentProvider::OpenRouter { api_key, model } => {
            let client = crate::llm::openrouter::OpenRouterClient::new(
                api_key.to_string(),
                model.to_string(),
            );
            let params =
                crate::llm::generation::OpenRouterGenerationParams::provider_defaults(128, 0.0);
            client.generate(&prompt, &params).await
        }
        #[cfg(feature = "local-ml")]
        IntentProvider::Local {
            app_handle,
            db_path,
        } => {
            let app_handle = app_handle.clone();
            let db_path = db_path.to_path_buf();
            match tokio::task::spawn_blocking(move || -> Result<String, String> {
                let conn = rusqlite::Connection::open(&db_path)
                    .map_err(|error| format!("Failed to open DB for intent routing: {error}"))?;
                let engine =
                    crate::llm::get_or_init_local_gemma_engine(&conn, &db_path, &app_handle)?;
                let engine = engine
                    .lock()
                    .map_err(|error| format!("Local LLM engine lock poisoned: {error}"))?;
                engine.generate_chat_with_ctx(&prompt, 128, 4_096, 0.0, "[rag][intent]")
            })
            .await
            {
                Ok(result) => result,
                Err(_) => Err("intent routing worker failed".to_string()),
            }
        }
    };

    if generated.is_err() {
        eprintln!("[rag] Intent classifier unavailable; continuing with retrieval");
    }
    route_classifier_result(generated)
}

#[cfg(test)]
mod tests {
    use super::{build_intent_prompt, route_classifier_result, IntentRoute};
    use crate::rag::RagChatTurn;

    fn turn(role: &str, content: &str) -> RagChatTurn {
        RagChatTurn {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn approved_conversational_examples_route_directly() {
        let cases = [
            "Hola",
            "¿Cómo estás?",
            "¿En qué me podés ayudar?",
            "¿Qué podés hacer?",
            "¿Cómo funciona este chat?",
            "Gracias",
        ];

        for question in cases {
            let prompt = build_intent_prompt(question, &[], 6, 500);
            assert!(
                prompt.contains(question),
                "the classifier prompt must include the current question: {question}"
            );
            assert_eq!(
                route_classifier_result(Ok::<_, ()>(
                    r#"{"needs_retrieval":false,"intent":"conversational"}"#,
                )),
                IntentRoute::Direct,
                "conversational example should skip retrieval: {question}"
            );
        }
    }

    #[test]
    fn approved_corpus_and_ambiguous_examples_route_to_retrieval() {
        let cases = [
            (
                "¿Qué huelgas protagonizó el SOIP en 1965?",
                r#"{"needs_retrieval":true,"intent":"corpus_question"}"#,
            ),
            (
                "¿Qué dicen las fuentes sobre el conflicto de abril?",
                r#"{"needs_retrieval":true,"intent":"corpus_question"}"#,
            ),
            (
                "Compará las posiciones del SOIP y el Sindicato de la Alimentación",
                r#"{"needs_retrieval":true,"intent":"corpus_question"}"#,
            ),
            (
                "¿Hay evidencia de conflictos laborales durante 1965?",
                r#"{"needs_retrieval":true,"intent":"ambiguous"}"#,
            ),
        ];

        for (question, classifier_output) in cases {
            let prompt = build_intent_prompt(question, &[], 6, 500);
            assert!(
                prompt.contains(question),
                "the classifier prompt must include the current question: {question}"
            );
            assert_eq!(
                route_classifier_result(Ok::<_, ()>(classifier_output)),
                IntentRoute::Retrieval,
                "corpus-plausible questions must retrieve conservatively: {question}"
            );
        }
    }

    #[test]
    fn uncertain_classifier_outcomes_route_to_retrieval() {
        let uncertain_outputs = [
            ("malformed JSON", Ok(r#"{"needs_retrieval":false"#)),
            (
                "unknown extra field",
                Ok(r#"{"needs_retrieval":false,"intent":"conversational","confidence":1.0}"#),
            ),
            (
                "corpus intent contradicts direct flag",
                Ok(r#"{"needs_retrieval":false,"intent":"corpus_question"}"#),
            ),
            (
                "unknown intent",
                Ok(r#"{"needs_retrieval":false,"intent":"smalltalk"}"#),
            ),
            (
                "conversational intent contradicts retrieval flag",
                Ok(r#"{"needs_retrieval":true,"intent":"conversational"}"#),
            ),
            ("classifier provider failure", Err(())),
        ];

        for (case, result) in uncertain_outputs {
            assert_eq!(
                route_classifier_result(result),
                IntentRoute::Retrieval,
                "uncertainty must never bypass retrieval: {case}"
            );
        }
    }

    #[test]
    fn intent_prompt_includes_bounded_history_for_a_follow_up() {
        let history = vec![
            turn("user", "OLDEST_USER_SENTINEL that must be excluded"),
            turn(
                "assistant",
                "OLDEST_ASSISTANT_SENTINEL that must be excluded",
            ),
            turn("user", "RECENT_USER_SENTINEL plus discarded detail"),
            turn(
                "assistant",
                "RECENT_ASSISTANT_SENTINEL plus discarded detail",
            ),
        ];

        let prompt = build_intent_prompt("¿Y qué pasó después?", &history, 2, 20);

        assert!(prompt.contains("¿Y qué pasó después?"));
        assert!(prompt.contains("RECENT_USER_SENTINEL"));
        assert!(prompt.contains("RECENT_ASSISTANT_SEN"));
        assert!(!prompt.contains("OLDEST_USER_SENTINEL"));
        assert!(!prompt.contains("OLDEST_ASSISTANT_SENTINEL"));
        assert!(!prompt.contains("plus discarded detail"));
    }
}
