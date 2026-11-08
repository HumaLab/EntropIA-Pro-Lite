//! Semantic triple extraction behind the batch queue.
//!
//! One asset is one unit: read its text, run the configured triples flow
//! (local Gemma or OpenRouter, as the per-asset button does) and publish the
//! asset's triples inside the commit transaction. The per-asset button keeps
//! using the in-memory LLM queue; both write through the same persistence.

use std::path::PathBuf;

use rusqlite::Connection;
use tauri::AppHandle;

use super::scheduler::{
    ClaimedTask, EngineOutput, ExecCtx, ExecOutput, ExecResult, Executor, StopFlag,
};
use crate::db::open::open_archive_connection;
use crate::llm::LlmTriple;

/// Pinned on every triples task; bump it when the extraction flow changes shape.
pub const TRIPLES_TASK_CONTRACT: &str = "triples:v1";

#[derive(Debug, Clone)]
pub struct TriplesComputeOutput {
    pub item_id: String,
    pub triples: Vec<LlmTriple>,
}

/// Writes the asset's triples inside the commit transaction. An empty result
/// writes nothing: a reply without triples must never wipe an earlier run.
pub fn publish_triples_output(
    conn: &Connection,
    asset_id: &str,
    output: &TriplesComputeOutput,
) -> Result<(), String> {
    if output.triples.is_empty() {
        return Ok(());
    }
    crate::llm::replace_triples_for_asset(conn, &output.item_id, asset_id, &output.triples)
}

pub struct TriplesExecutor {
    app: AppHandle,
    db_path: PathBuf,
}

impl TriplesExecutor {
    pub fn new(app: AppHandle, db_path: PathBuf) -> Self {
        Self { app, db_path }
    }
}

impl Executor for TriplesExecutor {
    fn kinds(&self) -> &[&str] {
        &["triples"]
    }

    fn run(&self, _ctx: &ExecCtx, task: &ClaimedTask, stop: &StopFlag) -> ExecResult {
        let done = |output: ExecOutput, engine_output: Option<TriplesComputeOutput>| ExecResult {
            checkpoints: Vec::new(),
            progress_total: None,
            engine_output: engine_output.map(EngineOutput::Triples),
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
                    receipt: r#"{"triples":0}"#.to_string(),
                },
                None,
            );
        }
        if stop.stopped() {
            return done(ExecOutput::Stopped, None);
        }
        match crate::llm::extract_triples_for_asset_blocking(
            &self.app,
            &self.db_path,
            &task.asset_id,
        ) {
            Ok(triples) => {
                let count = triples.len();
                done(
                    ExecOutput::Success {
                        outcome: if count == 0 { "no_triples" } else { "triples" }.to_string(),
                        receipt: format!(r#"{{"triples":{count}}}"#),
                    },
                    Some(TriplesComputeOutput { item_id, triples }),
                )
            }
            // Same mapping as NER: missing key parks, transport retries, the rest fails.
            Err(error) => done(
                match super::ner::map_ner_error(&error) {
                    ExecOutput::Fatal { message, .. } => ExecOutput::Fatal {
                        code: "triples_failed".to_string(),
                        message,
                    },
                    other => other,
                },
                None,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triple(subject: &str) -> LlmTriple {
        LlmTriple {
            subject: subject.to_string(),
            predicate: "llevó".to_string(),
            object: "cueros".to_string(),
            evidence: None,
        }
    }

    fn with_evidence(evidence: &str) -> LlmTriple {
        LlmTriple {
            evidence: Some(evidence.to_string()),
            ..triple("El barco")
        }
    }

    /// (evidence_text, evidence_start, evidence_end) of the only triple of a1.
    fn evidence_row(conn: &Connection) -> (Option<String>, Option<i64>, Option<i64>) {
        conn.query_row(
            "SELECT evidence_text, evidence_start, evidence_end FROM triples WHERE asset_id = 'a1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
    }

    fn publish_with_text(text: &str, evidence: LlmTriple) -> Connection {
        let conn = triples_db();
        conn.execute_batch(
            "CREATE TABLE extractions (asset_id TEXT, text_content TEXT, created_at INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO extractions (asset_id, text_content, created_at) VALUES ('a1', ?1, 0)",
            [text],
        )
        .unwrap();
        let output = TriplesComputeOutput {
            item_id: "i1".to_string(),
            triples: vec![evidence],
        };
        publish_triples_output(&conn, "a1", &output).unwrap();
        conn
    }

    /// UTF-16 length, the unit the stored span and the frontend use.
    fn utf16(text: &str) -> i64 {
        text.encode_utf16().count() as i64
    }

    #[test]
    fn publishing_stores_the_evidence_and_its_utf16_span() {
        let before = "𝔄cta. Señor: ";
        let sentence = "el barco llevó cueros a Cádiz.";
        let conn = publish_with_text(&format!("{before}{sentence} Fin."), with_evidence(sentence));
        let start = utf16(before);
        assert_eq!(
            evidence_row(&conn),
            (
                Some(sentence.to_string()),
                Some(start),
                Some(start + utf16(sentence))
            )
        );
    }

    #[test]
    fn the_span_tolerates_whitespace_and_covers_the_texts_own_spacing() {
        let text = "Inicio. el barco\nllevó  cueros a Cádiz. Fin.";
        let conn = publish_with_text(text, with_evidence("el barco llevó cueros a Cádiz."));
        let (stored, start, end) = evidence_row(&conn);
        assert_eq!(stored.as_deref(), Some("el barco llevó cueros a Cádiz."));
        let units: Vec<u16> = text.encode_utf16().collect();
        let span =
            String::from_utf16(&units[start.unwrap() as usize..end.unwrap() as usize]).unwrap();
        assert_eq!(span, "el barco\nllevó  cueros a Cádiz.");
    }

    #[test]
    fn evidence_not_found_verbatim_is_kept_without_a_span() {
        let conn = publish_with_text(
            "El barco llevó cueros a Cádiz.",
            with_evidence("La fragata trajo sal de Lisboa."),
        );
        assert_eq!(
            evidence_row(&conn),
            (
                Some("La fragata trajo sal de Lisboa.".to_string()),
                None,
                None
            )
        );
    }

    #[test]
    fn a_triple_without_evidence_stores_nulls() {
        let conn = publish_with_text("El barco llevó cueros.", triple("El barco"));
        assert_eq!(evidence_row(&conn), (None, None, None));
    }

    #[test]
    fn without_readable_text_the_evidence_is_kept_without_a_span() {
        let conn = triples_db();
        let output = TriplesComputeOutput {
            item_id: "i1".to_string(),
            triples: vec![with_evidence("el barco llevó cueros.")],
        };
        publish_triples_output(&conn, "a1", &output).unwrap();
        assert_eq!(
            evidence_row(&conn),
            (Some("el barco llevó cueros.".to_string()), None, None)
        );
    }

    fn triples_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE triples (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, asset_id TEXT,
               subject TEXT NOT NULL, predicate TEXT NOT NULL, object TEXT NOT NULL,
               created_at INTEGER NOT NULL DEFAULT 0, evidence_text TEXT,
               evidence_start INTEGER, evidence_end INTEGER);
             INSERT INTO triples (id, item_id, asset_id, subject, predicate, object)
               VALUES ('old', 'i1', 'a1', 'Viejo', 'era', 'previo'),
                      ('other', 'i1', 'a2', 'Otra', 'página', 'intacta');",
        )
        .unwrap();
        conn
    }

    fn subjects(conn: &Connection, asset: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT subject FROM triples WHERE asset_id = ?1 ORDER BY subject")
            .unwrap();
        stmt.query_map([asset], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn publishing_replaces_only_this_assets_triples() {
        let conn = triples_db();
        let output = TriplesComputeOutput {
            item_id: "i1".to_string(),
            triples: vec![triple("Barco"), triple("Fragata")],
        };
        publish_triples_output(&conn, "a1", &output).unwrap();
        assert_eq!(subjects(&conn, "a1"), ["Barco", "Fragata"]);
        assert_eq!(subjects(&conn, "a2"), ["Otra"]);
    }

    #[test]
    fn an_empty_result_keeps_what_an_earlier_run_stored() {
        let conn = triples_db();
        let output = TriplesComputeOutput {
            item_id: "i1".to_string(),
            triples: Vec::new(),
        };
        publish_triples_output(&conn, "a1", &output).unwrap();
        assert_eq!(subjects(&conn, "a1"), ["Viejo"]);
    }
}
