//! The `manual_search` tool (S22, D19): the agent's on-demand reader for the
//! bundled manual and screen map, over the SAME FTS5 index the prompts used
//! to inject (`pick_sections_with_budget`: same budget, same troubleshooting
//! lead, same manual-first crowding). The gate answers it from the local
//! index, so it runs freely (READ class) with or without the documents box,
//! and every returned section is recorded: `HelpAnswer.sources` lists ONLY
//! what the model actually consulted.

use std::sync::{Arc, Mutex};

use super::agent_llm::{ToolOutcome, ToolSpec};
use super::index::{HelpIndex, Hit, Source};
use super::prompt::pick_sections_with_budget;

/// The tool name the model calls.
pub(crate) const MANUAL_SEARCH_TOOL_NAME: &str = "manual_search";
/// Default and maximum excerpts per call: the same bound the prompt
/// injection used ([`super::prompt::MAX_RANKED_SECTIONS`]).
pub(crate) const MANUAL_SEARCH_DEFAULT_LIMIT: usize = super::prompt::MAX_RANKED_SECTIONS;
pub(crate) const MANUAL_SEARCH_MAX_LIMIT: usize = super::prompt::MAX_RANKED_SECTIONS;

/// Whether the gate answers `name` from the local index.
pub(crate) fn is_manual_tool(name: &str) -> bool {
    name == MANUAL_SEARCH_TOOL_NAME
}

/// The spec offered to the model alongside the MCP and web tools.
pub(crate) fn manual_tool_spec() -> ToolSpec {
    ToolSpec {
        name: MANUAL_SEARCH_TOOL_NAME.to_string(),
        description: "Busca en el manual de EntropIA y el mapa de pantallas. Usala para preguntas sobre cómo usar la app (funciones, botones, menús, pasos, errores) antes de responder; podés llamarla varias veces con distintas consultas.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Lo que buscás en el manual, por ejemplo «importar PDF»."},
                "limit": {"type": "integer", "description": "Cuántos fragmentos devolver (1-4, por defecto 4).", "minimum": 1, "maximum": 4},
            },
            "required": ["query"],
            "additionalProperties": false,
        }),
    }
}

/// The local index behind `manual_search`, with the budget the prompts used
/// and the log of every section returned so far, in call order.
pub(crate) struct ManualSearch {
    index: Arc<Mutex<HelpIndex>>,
    budget_chars: usize,
    consulted: Arc<Mutex<Vec<Hit>>>,
}

impl ManualSearch {
    pub(crate) fn new(index: Arc<Mutex<HelpIndex>>, budget_chars: usize) -> Self {
        Self {
            index,
            budget_chars,
            consulted: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Answers one `manual_search` call: the ranked excerpts headed by
    /// `chapter › title`, exactly as the injected excerpts looked. A missing
    /// or blank `query` is `Failed` (never searched); a question with no
    /// hits answers `Ok` with a marker and records nothing.
    pub(crate) async fn call(&self, args: serde_json::Value) -> ToolOutcome {
        let query = args
            .get("query")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or("");
        if query.is_empty() {
            return ToolOutcome::Failed {
                message:
                    "manual_search necesita el parámetro «query» con lo que buscás en el manual."
                        .to_string(),
            };
        }
        let limit = args
            .get("limit")
            .and_then(|value| value.as_u64())
            .map(|value| value as usize)
            .unwrap_or(MANUAL_SEARCH_DEFAULT_LIMIT)
            .clamp(1, MANUAL_SEARCH_MAX_LIMIT);
        // The same ranking the prompts used to inject: troubleshooting
        // leads on failures, manual first, at most one screen-map hit.
        let hits = {
            let index = match self.index.lock() {
                Ok(index) => index,
                Err(_) => {
                    return ToolOutcome::Failed {
                        message: "El índice del manual no está disponible.".to_string(),
                    };
                }
            };
            pick_sections_with_budget(&index, query, false, self.budget_chars)
                .into_iter()
                .take(limit)
                .collect::<Vec<_>>()
        };
        if hits.is_empty() {
            return ToolOutcome::Ok {
                content:
                    "(la búsqueda en el manual no encontró fragmentos relevantes para esa consulta)"
                        .to_string(),
            };
        }
        self.record(&hits);
        ToolOutcome::Ok {
            content: format_hits(&hits),
        }
    }

    /// Every section returned so far, deduplicated, in first-seen order:
    /// the runs' `HelpAnswer.sources` come from here and nowhere else.
    pub(crate) fn consulted_sections(&self) -> Vec<Hit> {
        let consulted = self
            .consulted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut seen = std::collections::HashSet::new();
        consulted
            .iter()
            .filter(|hit| seen.insert((hit.source, hit.chapter.clone(), hit.title.clone())))
            .cloned()
            .collect()
    }

    /// Records returned sections, in call order.
    fn record(&self, hits: &[Hit]) {
        if let Ok(mut consulted) = self.consulted.lock() {
            consulted.extend(hits.iter().cloned());
        }
    }
}

/// The excerpts as tool text, headed by `chapter › title` exactly as the
/// injected excerpts looked (screen-map hits say so).
fn format_hits(hits: &[Hit]) -> String {
    let mut out = String::new();
    for hit in hits {
        if hit.source == Source::ScreenMap {
            out.push_str(&format!(
                "## Mapa de pantallas › {} › {}\n{}\n",
                hit.chapter, hit.title, hit.body
            ));
        } else {
            out.push_str(&format!(
                "## {} › {}\n{}\n",
                hit.chapter, hit.title, hit.body
            ));
        }
    }
    out
}

impl Clone for ManualSearch {
    fn clone(&self) -> Self {
        Self {
            index: Arc::clone(&self.index),
            budget_chars: self.budget_chars,
            consulted: Arc::clone(&self.consulted),
        }
    }
}

/// A searcher over a tiny in-memory index, shared by the gate and command
/// tests: no temp dir, no files, the same dispatch code as production.
#[cfg(test)]
pub(crate) fn memory_searcher() -> ManualSearch {
    use super::index::{KnowledgeDoc, Source};

    let docs = vec![
        KnowledgeDoc {
            source: Source::Manual,
            chapter: "Capítulo 1. Inicio rápido".to_string(),
            title: "1.1. Importar archivos".to_string(),
            body: "Arrastrá los PDF a la colección para importarlos.".to_string(),
        },
        KnowledgeDoc {
            source: Source::Manual,
            chapter: "Capítulo 2. Transcribir".to_string(),
            title: "2.1. Transcripción automática".to_string(),
            body: "La transcripción convierte el audio en texto.".to_string(),
        },
        KnowledgeDoc {
            source: Source::ScreenMap,
            chapter: "Biblioteca".to_string(),
            title: "Barra de importación".to_string(),
            body: "- «Importar» `toolbar.import` abre el diálogo de importación.".to_string(),
        },
    ];
    let index = HelpIndex::open(std::path::Path::new(":memory:"), &docs).expect("memory index");
    ManualSearch::new(Arc::new(Mutex::new(index)), 24_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_and_spec_shape() {
        assert_eq!(MANUAL_SEARCH_TOOL_NAME, "manual_search");
        assert!(is_manual_tool("manual_search"));
        assert!(!is_manual_tool("repo_tree"));
        assert!(!is_manual_tool("collections_list"));
        let spec = manual_tool_spec();
        assert_eq!(spec.name, "manual_search");
        assert!(
            !spec.description.trim().is_empty(),
            "the model needs to know when to call it"
        );
        let required = spec
            .input_schema
            .get("required")
            .and_then(|value| value.as_array())
            .expect("required lists query");
        assert!(required.iter().any(|entry| entry == "query"));
    }

    #[tokio::test]
    async fn search_returns_excerpts_with_chapter_title_headings() {
        let search = memory_searcher();
        match search
            .call(serde_json::json!({"query": "importar archivos pdf"}))
            .await
        {
            ToolOutcome::Ok { content } => {
                assert!(
                    content.contains("Capítulo 1. Inicio rápido › 1.1. Importar archivos"),
                    "headed like the injected excerpts: {content}"
                );
                assert!(
                    content.contains("Arrastrá los PDF"),
                    "the body travels: {content}"
                );
            }
            other => panic!("search must run: {other:?}"),
        }
    }

    #[tokio::test]
    async fn limit_caps_the_returned_excerpts() {
        let search = memory_searcher();
        match search
            .call(serde_json::json!({"query": "importar transcripcion biblioteca", "limit": 1}))
            .await
        {
            ToolOutcome::Ok { content } => {
                assert_eq!(
                    content.matches("## ").count(),
                    1,
                    "one excerpt for limit 1: {content}"
                );
            }
            other => panic!("search must run: {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_or_blank_query_fails_without_searching() {
        let search = memory_searcher();
        for args in [
            serde_json::json!({}),
            serde_json::json!({"query": ""}),
            serde_json::json!({"query": "   "}),
        ] {
            match search.call(args).await {
                ToolOutcome::Failed { message } => assert!(!message.trim().is_empty()),
                other => panic!("a query-less call must fail: {other:?}"),
            }
        }
        assert!(
            search.consulted_sections().is_empty(),
            "failed calls record nothing"
        );
    }

    #[tokio::test]
    async fn no_hits_answer_ok_with_a_marker_and_record_nothing() {
        let search = memory_searcher();
        match search
            .call(serde_json::json!({"query": "xqztlkj wvbfp"}))
            .await
        {
            ToolOutcome::Ok { content } => assert!(!content.trim().is_empty()),
            other => panic!("no hits still answer Ok: {other:?}"),
        }
        assert!(
            search.consulted_sections().is_empty(),
            "a miss records nothing"
        );
    }

    #[tokio::test]
    async fn consulted_sections_hold_only_what_was_returned_in_order() {
        let search = memory_searcher();
        assert!(search.consulted_sections().is_empty());
        let _ = search
            .call(serde_json::json!({"query": "transcripción automática audio"}))
            .await;
        let first_round = search.consulted_sections();
        assert!(!first_round.is_empty(), "the first call returns sections");
        assert!(
            first_round[0].title.contains("Transcripción"),
            "best first: {first_round:?}"
        );
        let _ = search
            .call(serde_json::json!({"query": "importar archivos pdf"}))
            .await;
        let consulted = search.consulted_sections();
        assert!(
            consulted.len() > first_round.len(),
            "the second call adds sections: {consulted:?}"
        );
        assert!(
            consulted.iter().any(|hit| hit.title.contains("Importar")),
            "the second call's sections are there: {consulted:?}"
        );
        assert_eq!(
            &consulted[..first_round.len()],
            &first_round[..],
            "call order kept: {consulted:?}"
        );
        // A repeated call adds nothing twice.
        let len = consulted.len();
        let _ = search
            .call(serde_json::json!({"query": "importar archivos pdf"}))
            .await;
        assert_eq!(search.consulted_sections().len(), len, "deduped");
    }
}
