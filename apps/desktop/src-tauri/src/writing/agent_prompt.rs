//! What the model is asked, and how its answer is read (plan-editor.md §14).
//!
//! # Why this is a separate, pure module
//!
//! The prompt is the part that decides what the agent will do, and a prompt
//! assembled inside a command is a prompt nobody reads again. Here it can be
//! tested: that the instruction names the action, that the passage arrives
//! whole, that the context is labelled by what each piece *is*, and that
//! nothing else of the manuscript goes.
//!
//! # The rule the system prompt carries
//!
//! §14.2: the agent never silently changes the text. The model is told it is
//! writing a **proposal** that a person will read and accept or refuse, and
//! that it must not claim sources it was not given — an invented citation in an
//! academic article is worse than no suggestion at all.

use serde_json::{json, Value};

/// One labelled piece of what the model is told (§14.3).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ContextPiece {
    /// What the piece is: `selection`, `corpus`, `note`, `zotero`…
    pub kind: String,
    pub label: String,
    pub text: String,
}

/// What the model answered, read back into the fields a suggestion needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub suggested_text: String,
    pub rationale: Option<String>,
}

/// The instruction for each action of §14.1, in the language of the manuscript.
///
/// Spanish because the manuscripts are, and an instruction in another language
/// invites the model to answer in it. The action ids stay English, like every
/// other identifier in the codebase.
fn instruction(action: &str) -> &'static str {
    match action {
        "fix_spelling" => "Corregí la ortografía y la gramática. No cambies el estilo ni el contenido.",
        "review_wording" => "Revisá la redacción: mejorá lo que suene torpe, sin alterar lo que se afirma.",
        "improve_clarity" => "Hacé el texto más claro. No agregues información que no esté.",
        "improve_argument" => "Mejorá la articulación del argumento con lo que ya se dice. No agregues afirmaciones nuevas.",
        "shorten" => "Acortá el texto conservando todo lo que afirma.",
        "expand" => "Desarrollá el texto a partir de lo que ya dice y de la evidencia dada. No inventes datos.",
        "summarise" => "Resumí el texto.",
        "rephrase" => "Reformulá el texto diciendo lo mismo de otra manera.",
        "detect_repetition" => "Señalá las reiteraciones y proponé una versión sin ellas.",
        "detect_contradiction" => "Señalá contradicciones internas y proponé una versión que las resuelva, o explicá por qué no se puede sin más información.",
        "find_evidence" => "Proponé cómo sostener esta afirmación con la evidencia dada, citándola.",
        "find_counter_evidence" => "Señalá qué evidencia dada tensiona esta afirmación.",
        "find_counterexamples" => "Señalá contraejemplos presentes en la evidencia dada.",
        "recall_notes" => "Relacioná este pasaje con las notas dadas.",
        _ => "Revisá este pasaje.",
    }
}

/// The rules the model works under, stated once.
const SYSTEM: &str = "\
Sos un asistente de escritura académica. Escribís una PROPUESTA que una persona \
va a leer y aceptar o rechazar: nunca reemplazás el texto por tu cuenta.

Reglas:
- Trabajá solo sobre el pasaje señalado. El resto del artículo no te fue dado.
- No inventes citas, fuentes, datos ni fechas. Si una afirmación necesita una \
fuente que no recibiste, decilo en la justificación en vez de suponerla.
- Respondé en el mismo idioma del pasaje.
- Devolvé JSON con esta forma exacta, sin texto alrededor:
  {\"texto\": \"<el texto propuesto>\", \"justificacion\": \"<una o dos oraciones>\"}";

/// Which model answers, when nobody said explicitly.
///
/// The same chain `research.rs` already follows, and deliberately so: a writer
/// who configured a model for research does not expect a different one to
/// answer here, and a model silently differing between two parts of the same
/// app is the kind of thing that gets noticed only in the bill.
///
/// `default` is the crate's own default, passed in rather than reached for, so
/// this stays a rule about precedence and not a dependency on the agent crate.
pub fn pick_model(rag: Option<String>, openrouter: Option<String>, default: &str) -> String {
    rag.into_iter()
        .chain(openrouter)
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// Builds the messages for one action over one passage.
pub fn messages(action: &str, selection: &str, context: &[ContextPiece]) -> Vec<Value> {
    let mut user = String::new();
    user.push_str(instruction(action));
    user.push_str("\n\n## Pasaje\n");
    user.push_str(selection);

    // Each piece labelled by what it is, so the model can tell a quoted source
    // from the writer's own words — and so a reader of the log can too.
    let extra: Vec<&ContextPiece> = context
        .iter()
        .filter(|piece| piece.kind != "selection" && !piece.text.trim().is_empty())
        .collect();
    if !extra.is_empty() {
        user.push_str("\n\n## Material dado");
        for piece in extra {
            user.push_str(&format!("\n\n### {} ({})\n{}", piece.label, piece.kind, piece.text));
        }
    } else {
        // Said rather than left silent: a model given nothing will otherwise
        // fill the gap, and an invented source in an academic article is worse
        // than no suggestion.
        user.push_str("\n\nNo se te dio ninguna fuente. No cites ninguna.");
    }

    vec![
        json!({ "role": "system", "content": SYSTEM }),
        json!({ "role": "user", "content": user }),
    ]
}

/// Reads the model's answer.
///
/// JSON is asked for and usually given. When it is not, the whole answer is
/// taken as the proposal rather than discarded: a model that ignored the format
/// still wrote something the person can read and judge, and throwing it away
/// would turn a formatting quirk into a failed action.
pub fn parse(answer: &str) -> Proposal {
    let trimmed = answer.trim();
    // Some models wrap JSON in a fenced block however firmly they are asked not
    // to. Unwrapping it is cheaper than losing the answer over a fence.
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.rsplit_once("```").map(|(inside, _)| inside))
        .unwrap_or(trimmed)
        .trim();

    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(body) {
        let text = map.get("texto").and_then(Value::as_str).unwrap_or("").trim();
        if !text.is_empty() {
            let rationale = map
                .get("justificacion")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            return Proposal {
                suggested_text: text.to_string(),
                rationale,
            };
        }
    }

    Proposal {
        suggested_text: trimmed.to_string(),
        rationale: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(kind: &str, label: &str, text: &str) -> ContextPiece {
        ContextPiece {
            kind: kind.into(),
            label: label.into(),
            text: text.into(),
        }
    }

    fn user_of(messages: &[Value]) -> String {
        messages[1]["content"].as_str().expect("user").to_string()
    }

    /// §14.2: the model is told it is proposing, not editing.
    #[test]
    fn the_model_is_told_it_is_writing_a_proposal() {
        let messages = messages("improve_clarity", "el parrafo", &[]);
        let system = messages[0]["content"].as_str().expect("system");

        assert!(system.contains("PROPUESTA"), "{system}");
        assert!(system.contains("nunca reemplaz"), "{system}");
    }

    /// An invented citation in an academic article is worse than no suggestion.
    #[test]
    fn the_model_is_forbidden_from_inventing_sources() {
        let messages = messages("expand", "el parrafo", &[]);
        let system = messages[0]["content"].as_str().expect("system");

        assert!(system.contains("No inventes citas"), "{system}");
    }

    /// And when it was given nothing, it is told so outright rather than left
    /// to fill the silence.
    #[test]
    fn a_request_with_no_sources_says_so() {
        let user = user_of(&messages("expand", "el parrafo", &[]));

        assert!(user.contains("No se te dio ninguna fuente"), "{user}");
    }

    #[test]
    fn the_action_becomes_an_instruction_the_model_can_follow() {
        let shorten = user_of(&messages("shorten", "el parrafo", &[]));
        let expand = user_of(&messages("expand", "el parrafo", &[]));

        assert!(shorten.contains("Acortá"), "{shorten}");
        assert!(expand.contains("Desarrollá"), "{expand}");
        assert_ne!(shorten, expand);
    }

    #[test]
    fn the_passage_arrives_whole() {
        let passage = "De este modo, para avanzar en el análisis integrado…";
        let user = user_of(&messages("improve_clarity", passage, &[]));

        assert!(user.contains(passage), "{user}");
    }

    /// §14.3 rules out sending the document. Only what was assembled goes.
    #[test]
    fn nothing_of_the_manuscript_goes_but_what_was_given() {
        let user = user_of(&messages(
            "improve_clarity",
            "el parrafo",
            &[piece("corpus", "Un fragmento", "la evidencia")],
        ));

        assert!(user.contains("la evidencia"), "{user}");
        let sections: Vec<&str> = user.lines().filter(|line| line.starts_with("## ")).collect();
        assert_eq!(sections, ["## Pasaje", "## Material dado"], "{user}");
    }

    /// Each piece labelled by what it is, so the model can tell a quoted source
    /// from the writer's own words.
    #[test]
    fn every_piece_is_labelled_by_what_it_is() {
        let user = user_of(&messages(
            "find_evidence",
            "el parrafo",
            &[
                piece("corpus", "Acta del gremio", "los obreros"),
                piece("zotero", "Ginzburg 1976", "Il formaggio"),
            ],
        ));

        assert!(user.contains("Acta del gremio (corpus)"), "{user}");
        assert!(user.contains("Ginzburg 1976 (zotero)"), "{user}");
    }

    /// The passage is already its own section; repeating it as context would
    /// spend the budget saying it twice.
    #[test]
    fn the_selection_is_not_repeated_as_context() {
        let user = user_of(&messages(
            "improve_clarity",
            "el parrafo",
            &[piece("selection", "Lo seleccionado", "el parrafo")],
        ));

        assert_eq!(user.matches("el parrafo").count(), 1, "{user}");
    }
}

#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    fn the_research_model_answers_here_too() {
        assert_eq!(
            pick_model(Some("un/modelo".into()), Some("otro/modelo".into()), "x/y"),
            "un/modelo"
        );
    }

    #[test]
    fn falls_back_to_the_openrouter_model_when_research_named_none() {
        assert_eq!(pick_model(None, Some("otro/modelo".into()), "x/y"), "otro/modelo");
    }

    /// A setting can exist and be blank — a field someone cleared. Blank is not
    /// a choice of model, so it falls through like an absent setting.
    #[test]
    fn a_blank_setting_is_not_a_choice() {
        assert_eq!(pick_model(Some("   ".into()), Some("otro/modelo".into()), "x/y"), "otro/modelo");
        assert_eq!(pick_model(Some("  ".into()), Some(" ".into()), "x/y"), "x/y");
    }

    #[test]
    fn with_nothing_configured_the_default_answers() {
        assert_eq!(pick_model(None, None, "x/y"), "x/y");
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn reads_the_shape_that_was_asked_for() {
        let out = parse(r#"{"texto":"el texto propuesto","justificacion":"mas claro"}"#);

        assert_eq!(out.suggested_text, "el texto propuesto");
        assert_eq!(out.rationale.as_deref(), Some("mas claro"));
    }

    /// Models fence their JSON however firmly they are asked not to. Losing an
    /// answer over a fence would be an absurd failure.
    #[test]
    fn unwraps_a_fenced_answer() {
        let out = parse("```json\n{\"texto\":\"propuesto\",\"justificacion\":\"porque si\"}\n```");

        assert_eq!(out.suggested_text, "propuesto");
    }

    /// A model that ignored the format still wrote something a person can read
    /// and judge. Discarding it would turn a formatting quirk into a failure.
    #[test]
    fn keeps_a_plain_answer_as_the_proposal() {
        let out = parse("Este es el texto propuesto, sin JSON.");

        assert_eq!(out.suggested_text, "Este es el texto propuesto, sin JSON.");
        assert!(out.rationale.is_none());
    }

    #[test]
    fn a_proposal_with_no_rationale_simply_has_none() {
        let out = parse(r#"{"texto":"propuesto"}"#);

        assert_eq!(out.suggested_text, "propuesto");
        assert!(out.rationale.is_none());
    }

    /// JSON whose `texto` is empty says nothing, so the raw answer is kept —
    /// which is more than an empty proposal would be.
    #[test]
    fn falls_back_when_the_shape_is_right_but_empty() {
        let out = parse(r#"{"texto":"   ","justificacion":"nada"}"#);

        assert_eq!(out.suggested_text, r#"{"texto":"   ","justificacion":"nada"}"#);
    }

    #[test]
    fn an_empty_answer_stays_empty_rather_than_becoming_something() {
        assert_eq!(parse("   ").suggested_text, "");
    }
}
