use super::RagChatTurn;

const MAX_REWRITE_CHARS: usize = 4_000;

pub(crate) struct RewriteRequest {
    pub(crate) prompt: String,
}

pub(crate) fn build_rewrite_request(
    question: &str,
    history: &[RagChatTurn],
    max_turns: usize,
    turn_max_chars: usize,
) -> Option<RewriteRequest> {
    if history.is_empty() || max_turns == 0 || !is_context_dependent(question) {
        return None;
    }

    let history_start = history.len().saturating_sub(max_turns);
    let mut bounded_history = String::new();
    for turn in &history[history_start..] {
        let content: String = turn.content.chars().take(turn_max_chars).collect();
        bounded_history.push_str(&turn.role);
        bounded_history.push_str(": ");
        bounded_history.push_str(content.trim());
        bounded_history.push('\n');
    }

    Some(RewriteRequest {
        prompt: format!(
            "Reescribí la pregunta de seguimiento como una consulta independiente breve y en el mismo idioma.\n\
             Preservá exactamente nombres, fechas, citas e identificadores presentes en la conversación.\n\
             No respondas la pregunta ni agregues información: devolvé únicamente la consulta independiente.\n\
             Historial reciente:\n{bounded_history}\
             Pregunta de seguimiento:\n{question}"
        ),
    })
}

fn is_context_dependent(question: &str) -> bool {
    let lowered = question.trim().to_lowercase();
    let padded = format!(" {lowered} ");
    [
        " él ",
        " ella ",
        " ellos ",
        " ellas ",
        " eso ",
        " esa ",
        " ese ",
        " aquello ",
        " lo anterior ",
        " qué pasó después ",
        " que pasó después ",
        " y qué ",
        " y que ",
        " entonces ",
        " him ",
        " her ",
        " them ",
        " that ",
        " what happened next ",
        " and what ",
    ]
    .iter()
    .any(|marker| padded.contains(marker))
}

pub(crate) fn normalize_rewrite(original: &str, raw: &str) -> Option<String> {
    let mut cleaned = raw.trim();
    if cleaned.starts_with("```") {
        cleaned = cleaned
            .split_once('\n')
            .map(|(_, body)| body)
            .unwrap_or_default();
        cleaned = cleaned.trim();
        if let Some(body) = cleaned.strip_suffix("```") {
            cleaned = body.trim();
        }
    }

    const LABELS: &[&str] = &[
        "consulta independiente:",
        "consulta reescrita:",
        "consulta:",
        "standalone query:",
        "rewritten query:",
        "rewrite:",
    ];
    let lowered = cleaned.to_lowercase();
    if let Some(label) = LABELS.iter().find(|label| lowered.starts_with(**label)) {
        cleaned = cleaned[label.len()..].trim();
    }
    cleaned = cleaned
        .trim_matches(|character| matches!(character, '"' | '\'' | '“' | '”' | '‘' | '’'))
        .trim();

    let length = cleaned.chars().count();
    if length == 0 || length > MAX_REWRITE_CHARS || cleaned.eq_ignore_ascii_case(original.trim()) {
        return None;
    }
    Some(cleaned.to_string())
}

#[cfg(test)]
mod tests {
    use super::{build_rewrite_request, normalize_rewrite};
    use crate::rag::RagChatTurn;

    fn turn(role: &str, content: &str) -> RagChatTurn {
        RagChatTurn {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn build_rewrite_request_skips_without_history_or_for_standalone_question() {
        assert!(build_rewrite_request("¿Y qué pasó después con él?", &[], 4, 200,).is_none());

        let history = vec![turn(
            "assistant",
            "Juan Pérez encabezó la huelga portuaria de 1961.",
        )];
        assert!(build_rewrite_request(
            "¿En qué año comenzó la huelga portuaria de Montevideo?",
            &history,
            4,
            200,
        )
        .is_none());
    }

    #[test]
    fn build_rewrite_request_bounds_history_and_requests_only_a_faithful_standalone_query() {
        let history = vec![
            turn("user", "MARCADOR_ANTIGUO que no debe entrar"),
            turn(
                "assistant",
                "Juan Pérez presidió el sindicato portuario en 1961. TEXTO_FUERA_DEL_LIMITE",
            ),
            turn("user", "¿Qué hizo durante la huelga?"),
        ];

        let request = build_rewrite_request("¿Y qué pasó después con él?", &history, 2, 50)
            .expect("an explicitly dependent follow-up needs contextual rewriting");
        let prompt = request.prompt;
        let normalized_prompt = prompt.to_lowercase();

        assert!(prompt.contains("¿Y qué pasó después con él?"));
        assert!(prompt.contains("Juan Pérez presidió el sindicato portuario"));
        assert!(prompt.contains("¿Qué hizo durante la huelga?"));
        assert!(!prompt.contains("MARCADOR_ANTIGUO"));
        assert!(!prompt.contains("TEXTO_FUERA_DEL_LIMITE"));
        assert!(normalized_prompt.contains("consulta independiente"));
        assert!(normalized_prompt.contains("mismo idioma"));
        assert!(normalized_prompt.contains("breve"));
        assert!(normalized_prompt.contains("nombres"));
        assert!(normalized_prompt.contains("fechas"));
        assert!(normalized_prompt.contains("citas"));
        assert!(normalized_prompt.contains("identificadores"));
        assert!(normalized_prompt.contains("no respondas"));
        assert!(prompt.chars().count() < 1_500);
    }

    #[test]
    fn normalize_rewrite_rejects_empty_unchanged_and_oversized_outputs() {
        let original = "¿Y qué pasó después con él?";

        assert_eq!(normalize_rewrite(original, "   \n"), None);
        assert_eq!(normalize_rewrite(original, original), None);
        assert_eq!(
            normalize_rewrite(original, &format!("consulta {}", "x".repeat(4_001))),
            None
        );
    }

    #[test]
    fn normalize_rewrite_removes_fences_labels_and_wrapping_quotes() {
        let normalized = normalize_rewrite(
            "¿Y qué pasó después con él?",
            "```text\nConsulta independiente: \"¿Qué ocurrió después con Juan Pérez en 1961?\"\n```",
        );

        assert_eq!(
            normalized.as_deref(),
            Some("¿Qué ocurrió después con Juan Pérez en 1961?")
        );
    }
}
