//! LLM OCR correction behind the batch queue.
//!
//! One asset is one unit: read its text, ask the configured correction flow
//! (always OpenRouter, also in Pro, exactly like the per-page button) and
//! publish the corrected text inside the commit transaction. Nothing here
//! forks the correction logic: the publisher is
//! [`crate::llm::ocr_correction::commit_asset_correction_if_current`], the
//! same call the per-document command makes, so the backup
//! (`ocr_correction_backups`, «Restaurar OCR original»), the
//! `llm_results llr-asset-{id}-correct_ocr` row and the anti-stale
//! `expected_extraction` gate stay one implementation.

use std::path::PathBuf;

use rusqlite::Connection;
use tauri::AppHandle;

use super::scheduler::{
    ClaimedTask, EngineOutput, ExecCtx, ExecOutput, ExecResult, Executor, StopFlag,
};
use crate::db::open::open_archive_connection;

/// Batch task kind for the correction operation (`ocr_correction` in the
/// batch's `operations` list).
pub const CORRECTION_TASK_KIND: &str = "ocr_correction";

/// Pinned on every correction task; bump it when the flow changes shape.
pub const OCR_CORRECTION_TASK_CONTRACT: &str = "ocr_correction:v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectionComputeOutput {
    /// The extraction the model was asked to correct. The commit refuses the
    /// answer when the page's text moved while the model was answering.
    pub expected_extraction: String,
    /// The model's corrected text, not yet committed.
    pub corrected: String,
}

/// Publishes one page's correction inside the commit transaction: the same
/// writer the per-document command uses (its body, called with the caller's
/// transaction instead of one of its own, since SQLite cannot nest), so the
/// backup row, the stored result and the replaced extraction commit with the
/// task receipt or not at all.
///
/// An empty answer writes nothing: a reply without text must never wipe a
/// page's OCR, exactly as an empty entity or triple result writes nothing.
pub fn publish_correction_output(
    conn: &Connection,
    asset_id: &str,
    output: &CorrectionComputeOutput,
) -> Result<(), String> {
    if output.corrected.trim().is_empty() || output.expected_extraction.trim().is_empty() {
        return Ok(());
    }
    crate::llm::ocr_correction::commit_asset_correction_in_owner_transaction(
        conn,
        asset_id,
        &output.corrected,
        &output.expected_extraction,
    )
    .map(|_| ())
}

pub struct OcrCorrectionExecutor {
    app: AppHandle,
    db_path: PathBuf,
}

impl OcrCorrectionExecutor {
    pub fn new(app: AppHandle, db_path: PathBuf) -> Self {
        Self { app, db_path }
    }
}

impl Executor for OcrCorrectionExecutor {
    fn kinds(&self) -> &[&str] {
        &[CORRECTION_TASK_KIND]
    }

    fn run(&self, _ctx: &ExecCtx, task: &ClaimedTask, stop: &StopFlag) -> ExecResult {
        let done =
            |output: ExecOutput, engine_output: Option<CorrectionComputeOutput>| ExecResult {
                checkpoints: Vec::new(),
                progress_total: None,
                engine_output: engine_output.map(EngineOutput::Correction),
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
            // Nothing to correct: a blank page is not a failure.
            return done(
                ExecOutput::Success {
                    outcome: "no_source_text".to_string(),
                    receipt: r#"{"corrected":0}"#.to_string(),
                },
                Some(CorrectionComputeOutput {
                    expected_extraction: String::new(),
                    corrected: String::new(),
                }),
            );
        }
        if stop.stopped() {
            return done(ExecOutput::Stopped, None);
        }
        match crate::llm::correct_ocr_for_asset_blocking(&self.app, &self.db_path, &task.asset_id) {
            Ok(run) => {
                let chars = run.corrected.chars().count();
                done(
                    ExecOutput::Success {
                        outcome: "corrected".to_string(),
                        receipt: format!(r#"{{"characters":{chars}}}"#),
                    },
                    Some(CorrectionComputeOutput {
                        expected_extraction: run.expected_extraction,
                        corrected: run.corrected,
                    }),
                )
            }
            // Missing key or model parks the unit; transport retries; the rest
            // fails this page only, as the per-document command words it.
            Err(error) => done(
                super::ner::map_remote_llm_error(&error, "ocr_correction_failed"),
                None,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correction_db() -> Connection {
        let conn = Connection::open_in_memory().expect("db");
        conn.execute_batch(
            "CREATE TABLE assets (id TEXT PRIMARY KEY);
             CREATE TABLE extractions (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL,
                text_content TEXT NOT NULL,
                method TEXT NOT NULL,
                created_at INTEGER NOT NULL);
             CREATE TABLE llm_results (
                id TEXT PRIMARY KEY,
                target_id TEXT NOT NULL,
                target_type TEXT NOT NULL,
                job_type TEXT NOT NULL,
                result TEXT NOT NULL,
                created_at INTEGER NOT NULL);
             CREATE TABLE vec_assets (asset_id TEXT PRIMARY KEY, embedding BLOB NOT NULL);
             CREATE TABLE entities (id TEXT PRIMARY KEY, asset_id TEXT, item_id TEXT, source TEXT, value TEXT);
             CREATE TABLE triples (id TEXT PRIMARY KEY, asset_id TEXT, item_id TEXT, subject TEXT, predicate TEXT, object TEXT);
             INSERT INTO assets (id) VALUES ('a1');
             INSERT INTO extractions (id, asset_id, text_content, method, created_at)
               VALUES ('e1', 'a1', 'texto original del OCR', 'ocr', 1);
             INSERT INTO vec_assets (asset_id, embedding) VALUES ('a1', zeroblob(8));
             INSERT INTO entities (id, asset_id, item_id, source, value) VALUES ('n1', 'a1', 'i1', 'spacy', 'Barco');
             INSERT INTO triples (id, asset_id, item_id, subject, predicate, object)
               VALUES ('t1', 'a1', 'i1', 'Barco', 'lleva', 'carga');",
        )
        .expect("fixture");
        conn
    }

    fn one<T>(conn: &Connection, sql: &str) -> T
    where
        T: rusqlite::types::FromSql,
    {
        conn.query_row(sql, [], |row| row.get(0)).expect("query")
    }

    /// S3: the batch correction replaces the page's text exactly as the
    /// per-document command does — backup kept, `correct_ocr` result stored —
    /// and leaves the data already derived from the old text untouched.
    #[test]
    fn publishing_replaces_the_text_with_a_backup_and_leaves_derived_data_alone() {
        let conn = correction_db();
        let output = CorrectionComputeOutput {
            expected_extraction: "texto original del OCR".to_string(),
            corrected: "Texto original del OCR.".to_string(),
        };
        // The supervisor's commit transaction is the owner: the publisher must
        // join it instead of opening a nested one SQLite would refuse.
        conn.execute_batch("BEGIN IMMEDIATE")
            .expect("commit transaction");
        publish_correction_output(&conn, "a1", &output).expect("publish inside the commit");
        conn.execute_batch("COMMIT").expect("commit");
        assert_eq!(
            one::<String>(
                &conn,
                "SELECT text_content FROM extractions WHERE asset_id = 'a1'"
            ),
            "Texto original del OCR."
        );
        assert_eq!(
            one::<String>(
                &conn,
                "SELECT original_text_content FROM ocr_correction_backups WHERE asset_id = 'a1'"
            ),
            "texto original del OCR",
            "«Restaurar OCR original» must still have the original"
        );
        assert_eq!(
            one::<String>(
                &conn,
                "SELECT result FROM llm_results WHERE id = 'llr-asset-a1-correct_ocr'"
            ),
            "Texto original del OCR."
        );
        // S3: derived data made from the old text stays as it is.
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM vec_assets"), 1);
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM entities"), 1);
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM triples"), 1);
        // The correction is now the page's done marker (S5).
        assert_eq!(
            super::super::eligibility::correction_decision(&conn, "a1").expect("decision"),
            super::super::eligibility::CorrectionDecision::AlreadyDone
        );
        // A second publish from the old snapshot is refused, not applied.
        assert_eq!(
            publish_correction_output(&conn, "a1", &output).expect_err("stale"),
            crate::llm::ocr_correction::STALE_OCR_CORRECTION_ERROR
        );
        assert_eq!(
            one::<String>(
                &conn,
                "SELECT text_content FROM extractions WHERE asset_id = 'a1'"
            ),
            "Texto original del OCR."
        );
    }

    /// A correction that never commits writes nothing: the rows join the
    /// caller's transaction, so a failed task cannot leave a half-corrected
    /// page behind.
    #[test]
    fn a_rolled_back_commit_writes_nothing() {
        let conn = correction_db();
        crate::llm::ocr_correction::ensure_schema(&conn).expect("correction schema");
        let output = CorrectionComputeOutput {
            expected_extraction: "texto original del OCR".to_string(),
            corrected: "Texto original del OCR.".to_string(),
        };
        conn.execute_batch("BEGIN IMMEDIATE")
            .expect("commit transaction");
        publish_correction_output(&conn, "a1", &output).expect("publish");
        conn.execute_batch("ROLLBACK").expect("rollback");
        assert_eq!(
            one::<String>(
                &conn,
                "SELECT text_content FROM extractions WHERE asset_id = 'a1'"
            ),
            "texto original del OCR"
        );
        assert_eq!(
            one::<i64>(&conn, "SELECT COUNT(*) FROM ocr_correction_backups"),
            0
        );
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM llm_results"), 0);
    }

    /// An empty answer writes nothing: no backup, no replaced text, no result.
    #[test]
    fn an_empty_answer_never_wipes_the_page() {
        let conn = correction_db();
        crate::llm::ocr_correction::ensure_schema(&conn).expect("correction schema");
        let output = CorrectionComputeOutput {
            expected_extraction: "texto original del OCR".to_string(),
            corrected: "   ".to_string(),
        };
        publish_correction_output(&conn, "a1", &output).expect("no-op");
        assert_eq!(
            one::<String>(
                &conn,
                "SELECT text_content FROM extractions WHERE asset_id = 'a1'"
            ),
            "texto original del OCR"
        );
        assert_eq!(
            one::<i64>(&conn, "SELECT COUNT(*) FROM ocr_correction_backups"),
            0
        );
        assert_eq!(one::<i64>(&conn, "SELECT COUNT(*) FROM llm_results"), 0);
    }
}
