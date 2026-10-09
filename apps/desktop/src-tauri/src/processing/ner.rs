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
    let lower = error.to_lowercase();
    if lower.contains("unavailable") || lower.contains("api key") {
        return ExecOutput::Blocked {
            code: "configuration_required".to_string(),
            message: error.to_string(),
        };
    }
    match super::embedding::map_embedding_error(error) {
        ExecOutput::Fatal { message, .. } => ExecOutput::Fatal {
            code: "ner_failed".to_string(),
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
}
