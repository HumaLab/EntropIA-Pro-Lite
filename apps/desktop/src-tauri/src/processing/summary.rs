//! Page summaries behind the batch queue (S2).
//!
//! One asset is one unit: read its text, run the configured summary flow (the
//! local engine or OpenRouter, chosen by the saved settings, exactly like the
//! per-page button) and store the result inside the commit transaction. The
//! publisher is [`crate::llm::persist_asset_summary`] — the same `llm_results`
//! (`target_type = asset`, `job_type = summarize`) writer the per-document
//! command uses, so ItemView reads a batch summary exactly as it reads a
//! manual one.

use std::path::PathBuf;

use rusqlite::Connection;
use tauri::AppHandle;

use super::scheduler::{
    ClaimedTask, EngineOutput, ExecCtx, ExecOutput, ExecResult, Executor, StopFlag,
};
use crate::db::open::open_archive_connection;

/// Batch task kind for the summary operation (`summary` in the batch's
/// `operations` list).
pub const SUMMARY_TASK_KIND: &str = "summary";

/// Pinned on every summary task; bump it when the flow changes shape.
pub const SUMMARY_TASK_CONTRACT: &str = "summary:v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryComputeOutput {
    pub summary: String,
}

/// Publishes one page's summary inside the commit transaction. An empty
/// summary writes nothing: a reply without text must never replace a summary
/// an earlier run stored.
pub fn publish_summary_output(
    conn: &Connection,
    asset_id: &str,
    output: &SummaryComputeOutput,
) -> Result<(), String> {
    if output.summary.trim().is_empty() {
        return Ok(());
    }
    crate::llm::persist_asset_summary(conn, asset_id, &output.summary)
}

pub struct SummaryExecutor {
    app: AppHandle,
    db_path: PathBuf,
}

impl SummaryExecutor {
    pub fn new(app: AppHandle, db_path: PathBuf) -> Self {
        Self { app, db_path }
    }
}

impl Executor for SummaryExecutor {
    fn kinds(&self) -> &[&str] {
        &[SUMMARY_TASK_KIND]
    }

    fn run(&self, _ctx: &ExecCtx, task: &ClaimedTask, stop: &StopFlag) -> ExecResult {
        let done = |output: ExecOutput, engine_output: Option<SummaryComputeOutput>| ExecResult {
            checkpoints: Vec::new(),
            progress_total: None,
            engine_output: engine_output.map(EngineOutput::Summary),
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
        let exists: bool = match conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM assets WHERE id = ?1)",
            [&task.asset_id],
            |row| row.get(0),
        ) {
            Ok(exists) => exists,
            Err(error) => return fatal("storage_unavailable", error.to_string()),
        };
        if !exists {
            return fatal(
                "source_deleted",
                format!("Asset {} no longer exists", task.asset_id),
            );
        }
        let text = match crate::nlp::text_provider::get_asset_text(&conn, &task.asset_id) {
            Ok(text) => text,
            Err(error) => return fatal("storage_unavailable", error),
        };
        drop(conn);
        if text.trim().is_empty() {
            // OCR finished with a blank page: nothing to read, nothing to fail.
            return done(
                ExecOutput::Success {
                    outcome: "no_source_text".to_string(),
                    receipt: r#"{"summary":0}"#.to_string(),
                },
                Some(SummaryComputeOutput {
                    summary: String::new(),
                }),
            );
        }
        if stop.stopped() {
            return done(ExecOutput::Stopped, None);
        }
        match crate::llm::summarize_for_asset_blocking(&self.app, &self.db_path, &task.asset_id) {
            Ok(summary) => {
                let chars = summary.chars().count();
                done(
                    ExecOutput::Success {
                        outcome: "summary".to_string(),
                        receipt: format!(r#"{{"characters":{chars}}}"#),
                    },
                    Some(SummaryComputeOutput { summary }),
                )
            }
            // Same mapping as NER and triples: missing key/model parks,
            // transport retries, anything else fails this page only.
            Err(error) => done(
                super::ner::map_remote_llm_error(&error, "summary_failed"),
                None,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary_db() -> Connection {
        let conn = Connection::open_in_memory().expect("db");
        conn.execute_batch(
            "CREATE TABLE llm_results (
                id TEXT PRIMARY KEY,
                target_id TEXT NOT NULL,
                target_type TEXT NOT NULL,
                job_type TEXT NOT NULL,
                result TEXT NOT NULL,
                created_at INTEGER NOT NULL);",
        )
        .expect("fixture");
        conn
    }

    /// S2: the batch stores the page's summary where the per-page summary of
    /// ItemView reads it — one `llm_results` row for the asset, job
    /// `summarize` — and it counts as that page's done marker (S5).
    #[test]
    fn publishing_stores_the_page_summary_the_item_view_reads() {
        let conn = summary_db();
        let output = SummaryComputeOutput {
            summary: "Resumen de la página.".to_string(),
        };
        publish_summary_output(&conn, "a1", &output).expect("publish");
        let (target, target_type, job, result): (String, String, String, String) = conn
            .query_row(
                "SELECT target_id, target_type, job_type, result FROM llm_results",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("stored summary");
        assert_eq!(
            (target.as_str(), target_type.as_str(), job.as_str()),
            ("a1", "asset", "summarize")
        );
        assert_eq!(result, "Resumen de la página.");
        // Republishing keeps one row (the per-document writer upserts).
        publish_summary_output(&conn, "a1", &output).expect("republish");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM llm_results", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 1);
    }

    /// An empty answer never replaces a stored summary.
    #[test]
    fn an_empty_summary_writes_nothing() {
        let conn = summary_db();
        publish_summary_output(
            &conn,
            "a1",
            &SummaryComputeOutput {
                summary: "  ".to_string(),
            },
        )
        .expect("no-op");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM llm_results", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 0);
    }
}
