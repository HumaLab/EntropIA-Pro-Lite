//! Entity extraction (NER) behind the batch queue.
//!
//! One asset is one unit: read its text, run the configured NER chain (spaCy
//! then the LLM fallback under Pro, OpenRouter under Lite), and publish the
//! automatic entities inside the commit transaction. The per-item button keeps
//! using the in-memory NLP queue; both write through the same persistence.

use std::path::PathBuf;

use rusqlite::Connection;
use tauri::AppHandle;

use super::scheduler::{
    ClaimedTask, EngineOutput, ExecCtx, ExecOutput, ExecResult, Executor, StopFlag,
};
use crate::db::open::open_archive_connection;
use crate::nlp::ner::{self, types::Entity};

/// Pinned on every NER task; bump it when the extraction chain changes shape.
pub const NER_TASK_CONTRACT: &str = "ner:v1";

#[derive(Debug, Clone)]
pub struct NerComputeOutput {
    pub item_id: String,
    pub entities: Vec<Entity>,
}

/// Writes the asset's automatic entities inside the commit transaction. An
/// empty result writes nothing: text without entities must never wipe what a
/// richer earlier run (or Gemma) stored.
pub fn publish_ner_output(
    conn: &Connection,
    asset_id: &str,
    output: &NerComputeOutput,
) -> Result<(), String> {
    if output.entities.is_empty() {
        return Ok(());
    }
    ner::replace_entities_for_asset(conn, &output.item_id, asset_id, &output.entities)
}

/// Missing keys or engines park the task for the person to fix settings;
/// transport trouble retries; anything else fails this asset only.
pub(crate) fn map_ner_error(error: &str) -> ExecOutput {
    map_remote_llm_error(error, "ner_failed")
}

/// The same mapping for every remote-flow kind: one gate, one wording, and a
/// kind-specific fatal code so a failed summary never reads as a failed NER.
pub(crate) fn map_remote_llm_error(error: &str, fatal_code: &str) -> ExecOutput {
    let lower = error.to_lowercase();
    if lower.contains("unavailable") || lower.contains("api key") {
        return ExecOutput::Blocked {
            code: "configuration_required".to_string(),
            message: error.to_string(),
        };
    }
    match super::embedding::map_embedding_error(error) {
        ExecOutput::Fatal { message, .. } => ExecOutput::Fatal {
            code: fatal_code.to_string(),
            message,
        },
        other => other,
    }
}

pub struct NerExecutor {
    app: AppHandle,
    db_path: PathBuf,
}

impl NerExecutor {
    pub fn new(app: AppHandle, db_path: PathBuf) -> Self {
        Self { app, db_path }
    }
}

impl Executor for NerExecutor {
    fn kinds(&self) -> &[&str] {
        &["ner"]
    }

    fn run(&self, _ctx: &ExecCtx, task: &ClaimedTask, stop: &StopFlag) -> ExecResult {
        let done = |output: ExecOutput, engine_output: Option<NerComputeOutput>| ExecResult {
            checkpoints: Vec::new(),
            progress_total: None,
            engine_output: engine_output.map(EngineOutput::Ner),
            output,
        };
        let fatal = |code: &str, message: String| {
            done(
                ExecOutput::Fatal {
                    code: code.to_string(),
                    message,
                },
                None,
            )
        };
        let conn = match open_archive_connection(&self.db_path) {
            Ok(conn) => conn,
            Err(error) => return fatal("storage_unavailable", error),
        };
        let item_id = match crate::nlp::lookup_item_id_for_asset(&conn, &task.asset_id) {
            Ok(Some(item_id)) => item_id,
            Ok(None) => {
                return fatal(
                    "source_deleted",
                    format!("Asset {} no longer exists", task.asset_id),
                )
            }
            Err(error) => return fatal("storage_unavailable", error),
        };
        let input = match ner::prepare_ner_candidates_for_asset(&conn, &item_id, &task.asset_id) {
            Ok(input) => input,
            Err(error) => return fatal("storage_unavailable", error),
        };
        let fallback = crate::nlp::ner_fallback_config(&conn);
        drop(conn);
        if input.text.trim().is_empty() {
            // OCR finished with a blank page: nothing to read, nothing to fail.
            return done(
                ExecOutput::Success {
                    outcome: "no_source_text".to_string(),
                    receipt: r#"{"entities":0}"#.to_string(),
                },
                Some(NerComputeOutput {
                    item_id,
                    entities: Vec::new(),
                }),
            );
        }
        if stop.stopped() {
            return done(ExecOutput::Stopped, None);
        }
        let result = tauri::async_runtime::block_on(crate::nlp::run_configured_ner_input(
            &self.app,
            &self.db_path,
            fallback,
            input,
        ));
        match result {
            Ok(entities) => {
                let count = entities.len();
                done(
                    ExecOutput::Success {
                        outcome: if count == 0 {
                            "no_entities"
                        } else {
                            "entities"
                        }
                        .to_string(),
                        receipt: format!(r#"{{"entities":{count}}}"#),
                    },
                    Some(NerComputeOutput { item_id, entities }),
                )
            }
            Err(error) => done(map_ner_error(&error), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ner_errors_map_to_block_retry_or_fatal() {
        assert!(matches!(
            map_ner_error("OpenRouter NER unavailable: OpenRouter API key no configurada. Configure OpenRouter API key/model."),
            ExecOutput::Blocked { .. }
        ));
        assert!(matches!(
            map_ner_error("NER extraction failed: request timed out"),
            ExecOutput::Retryable { .. }
        ));
        assert!(matches!(
            map_ner_error("NER extraction failed: bad json"),
            ExecOutput::Fatal { code, .. } if code == "ner_failed"
        ));
    }

    /// What the executor sees for a given OpenRouter client failure.
    fn mapped(client_error: &str) -> ExecOutput {
        map_ner_error(&format!(
            "NER extraction failed: {}",
            crate::nlp::ner::openrouter::describe_openrouter_failure(client_error)
        ))
    }

    #[test]
    fn provider_trouble_retries_instead_of_asking_for_configuration() {
        // A provider failing after the 200, or a rate limit, is transient:
        // it must not park the asset as if the key were missing.
        assert!(matches!(
            mapped("OpenRouter provider error (502): Provider disconnected"),
            ExecOutput::Retryable { .. }
        ));
        assert!(matches!(
            mapped("OpenRouter API error (429 Too Many Requests): rate-limited upstream"),
            ExecOutput::Retryable { .. }
        ));
        assert!(matches!(
            mapped("OpenRouter request timed out after 180s: operation timed out"),
            ExecOutput::Retryable { .. }
        ));
        // An answer that cannot be read fails this asset only.
        assert!(matches!(
            mapped("Failed to parse OpenRouter response: expected value at line 1 column 1; body: <html>"),
            ExecOutput::Fatal { code, .. } if code == "ner_failed"
        ));
        assert!(matches!(
            mapped("OpenRouter returned no content (finish_reason: length)"),
            ExecOutput::Fatal { code, .. } if code == "ner_failed"
        ));
    }

    #[test]
    fn credential_and_model_rejections_still_ask_for_configuration() {
        for client_error in [
            "OpenRouter API error (401 Unauthorized): bad key",
            "OpenRouter API error (402 Payment Required): insufficient credits",
            "OpenRouter API error (403 Forbidden): key disabled",
            "OpenRouter API error (404 Not Found): no such model",
        ] {
            match mapped(client_error) {
                ExecOutput::Blocked { code, message } => {
                    assert_eq!(code, "configuration_required");
                    assert_eq!(
                        message,
                        format!(
                            "NER extraction failed: OpenRouter NER unavailable: {client_error}. Configure OpenRouter API key/model."
                        )
                    );
                }
                other => panic!("{client_error}: expected Blocked, got {other:?}"),
            }
        }
    }
}
