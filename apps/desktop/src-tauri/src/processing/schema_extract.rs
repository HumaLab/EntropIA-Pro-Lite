//! Extraction with a user-defined schema behind the batch queue (T-51).
//!
//! The user names the fields once ("barco", "carga" repeatable, "destino")
//! and a batch fills one record per case the text describes. One asset is one
//! unit, like triples: read its text, ask the configured model for a JSON
//! array, and replace that asset's records of that schema inside the commit
//! transaction. The schema id travels in the task contract, so a task always
//! runs the schema it was admitted for.

use std::path::PathBuf;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::{AppHandle, State};

use super::scheduler::{
    ClaimedTask, EngineOutput, ExecCtx, ExecOutput, ExecResult, Executor, StopFlag,
};
use crate::db::open::open_archive_connection;
use crate::db::state::AppDbState;

pub const SCHEMA_TASK_KIND: &str = "schema_extract";
const CONTRACT_PREFIX: &str = "schema_extract:v1:";

/// The task contract pins the schema id; bump `v1` when the flow changes shape.
pub fn contract_for(schema_id: &str) -> String {
    format!("{CONTRACT_PREFIX}{schema_id}")
}

fn schema_id_from_contract(contract: &str) -> Option<&str> {
    contract.strip_prefix(CONTRACT_PREFIX)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaField {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// One ship, many cargoes: the field holds a list.
    #[serde(default)]
    pub repeatable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionSchema {
    pub id: String,
    pub name: String,
    pub fields: Vec<SchemaField>,
    /// OpenRouter model for this schema; empty uses the general one.
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct SchemaComputeOutput {
    pub item_id: String,
    pub schema_id: String,
    pub records: Vec<Value>,
}

fn load_schema(conn: &Connection, schema_id: &str) -> Result<Option<ExtractionSchema>, String> {
    let row: Option<(String, String, String)> = conn
        .query_row(
            "SELECT name, fields_json, model FROM extraction_schemas WHERE id = ?1",
            [schema_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("Failed to read schema {schema_id}: {e}"))?;
    row.map(|(name, fields_json, model)| {
        serde_json::from_str(&fields_json)
            .map(|fields| ExtractionSchema {
                id: schema_id.to_string(),
                name,
                fields,
                model,
            })
            .map_err(|e| format!("Schema {schema_id} has invalid fields: {e}"))
    })
    .transpose()
}

/// Keeps the objects of the first JSON array in `raw`, each reduced to the
/// schema's keys: a repeatable field always becomes a list, any other field
/// a string or null. Records with every field empty are dropped.
pub fn parse_records(raw: &str, fields: &[SchemaField]) -> Vec<Value> {
    let (Some(start), Some(end)) = (raw.find('['), raw.rfind(']')) else {
        return Vec::new();
    };
    if end < start {
        return Vec::new();
    }
    let Ok(items) = serde_json::from_str::<Vec<Value>>(&raw[start..=end]) else {
        return Vec::new();
    };
    let text_of = |value: &Value| match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    };
    items
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|object| {
            let mut record = Map::new();
            let mut filled = false;
            for field in fields {
                let value = object.get(&field.name).unwrap_or(&Value::Null);
                let cleaned = if field.repeatable {
                    let list: Vec<Value> = match value {
                        Value::Array(values) => values
                            .iter()
                            .filter_map(text_of)
                            .map(Value::String)
                            .collect(),
                        other => text_of(other).map(Value::String).into_iter().collect(),
                    };
                    filled |= !list.is_empty();
                    Value::Array(list)
                } else {
                    match text_of(value) {
                        Some(text) => {
                            filled = true;
                            Value::String(text)
                        }
                        None => Value::Null,
                    }
                };
                record.insert(field.name.clone(), cleaned);
            }
            filled.then_some(Value::Object(record))
        })
        .collect()
}

/// Replaces the asset's records of the schema inside the commit transaction.
/// An empty result writes nothing: a reply without records must never wipe an
/// earlier run.
pub fn publish_schema_output(
    conn: &Connection,
    asset_id: &str,
    output: &SchemaComputeOutput,
) -> Result<(), String> {
    if output.records.is_empty() {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM extraction_records WHERE schema_id = ?1 AND asset_id = ?2",
        params![output.schema_id, asset_id],
    )
    .map_err(|e| format!("Failed to clear extraction records: {e}"))?;
    let now = super::repository::now_ms();
    for record in &output.records {
        conn.execute(
            "INSERT INTO extraction_records (id, schema_id, asset_id, item_id, record_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                uuid::Uuid::new_v4().to_string(),
                output.schema_id,
                asset_id,
                output.item_id,
                record.to_string(),
                now
            ],
        )
        .map_err(|e| format!("Failed to store extraction record: {e}"))?;
    }
    Ok(())
}

pub struct SchemaExtractExecutor {
    app: AppHandle,
    db_path: PathBuf,
}

impl SchemaExtractExecutor {
    pub fn new(app: AppHandle, db_path: PathBuf) -> Self {
        Self { app, db_path }
    }
}

impl Executor for SchemaExtractExecutor {
    fn kinds(&self) -> &[&str] {
        &[SCHEMA_TASK_KIND]
    }

    fn run(&self, _ctx: &ExecCtx, task: &ClaimedTask, stop: &StopFlag) -> ExecResult {
        let done = |output: ExecOutput, engine_output: Option<SchemaComputeOutput>| ExecResult {
            checkpoints: Vec::new(),
            progress_total: None,
            engine_output: engine_output.map(EngineOutput::Schema),
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
        let Some(schema_id) = schema_id_from_contract(&task.contract_hash) else {
            return fatal(
                "invalid_contract",
                format!("Task {} carries no schema", task.task_id),
            );
        };
        let conn = match open_archive_connection(&self.db_path) {
            Ok(conn) => conn,
            Err(error) => return fatal("storage_unavailable", error),
        };
        let schema = match load_schema(&conn, schema_id) {
            Ok(Some(schema)) => schema,
            Ok(None) => {
                return fatal(
                    "schema_deleted",
                    format!("Schema {schema_id} no longer exists"),
                )
            }
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
            return done(
                ExecOutput::Success {
                    outcome: "no_source_text".to_string(),
                    receipt: r#"{"records":0}"#.to_string(),
                },
                None,
            );
        }
        if stop.stopped() {
            return done(ExecOutput::Stopped, None);
        }
        let fields = schema
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.description.clone(), f.repeatable))
            .collect();
        match crate::llm::extract_schema_for_asset_blocking(
            &self.app,
            &self.db_path,
            &task.asset_id,
            fields,
            Some(schema.model.clone()).filter(|model| !model.trim().is_empty()),
        ) {
            Ok(raw) => {
                let records = parse_records(&raw, &schema.fields);
                let count = records.len();
                done(
                    ExecOutput::Success {
                        outcome: if count == 0 { "no_records" } else { "records" }.to_string(),
                        receipt: format!(r#"{{"records":{count}}}"#),
                    },
                    Some(SchemaComputeOutput {
                        item_id,
                        schema_id: schema.id,
                        records,
                    }),
                )
            }
            // Same mapping as NER: missing key parks, transport retries, the rest fails.
            Err(error) => done(
                match super::ner::map_ner_error(&error) {
                    ExecOutput::Fatal { message, .. } => ExecOutput::Fatal {
                        code: "schema_extract_failed".to_string(),
                        message,
                    },
                    other => other,
                },
                None,
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Commands: the schemas and their records, for the batch tab.
// ---------------------------------------------------------------------------

fn open(db: &State<'_, AppDbState>) -> Result<Connection, String> {
    open_archive_connection(&db.db_path)
}

fn validate(schema: &ExtractionSchema) -> Result<(), String> {
    if schema.name.trim().is_empty() {
        return Err("invalid_schema: the schema needs a name".to_string());
    }
    if schema.fields.is_empty() {
        return Err("invalid_schema: the schema needs at least one field".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    for field in &schema.fields {
        let name = field.name.trim();
        if name.is_empty() || !seen.insert(name.to_lowercase()) {
            return Err(format!(
                "invalid_schema: empty or repeated field name {name:?}"
            ));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn extraction_schemas_list(
    db: State<'_, AppDbState>,
) -> Result<Vec<ExtractionSchema>, String> {
    let conn = open(&db)?;
    let mut stmt = conn
        .prepare("SELECT id FROM extraction_schemas ORDER BY name COLLATE NOCASE")
        .map_err(|e| format!("Failed to list schemas: {e}"))?;
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| format!("Failed to list schemas: {e}"))?;
    ids.iter()
        .filter_map(|id| load_schema(&conn, id).transpose())
        .collect()
}

/// Creates the schema when `id` is empty, otherwise replaces its name and
/// fields. Records already extracted keep the fields they were made with.
#[tauri::command]
pub async fn extraction_schema_save(
    mut schema: ExtractionSchema,
    db: State<'_, AppDbState>,
) -> Result<ExtractionSchema, String> {
    for field in &mut schema.fields {
        field.name = field.name.trim().to_string();
        field.description = field.description.trim().to_string();
    }
    schema.name = schema.name.trim().to_string();
    schema.model = schema.model.trim().to_string();
    validate(&schema)?;
    if schema.id.is_empty() {
        schema.id = uuid::Uuid::new_v4().to_string();
    }
    let fields_json = serde_json::to_string(&schema.fields).map_err(|e| e.to_string())?;
    let now = super::repository::now_ms();
    open(&db)?
        .execute(
            "INSERT INTO extraction_schemas (id, name, fields_json, model, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, fields_json = excluded.fields_json,
               model = excluded.model, updated_at = excluded.updated_at",
            params![schema.id, schema.name, fields_json, schema.model, now],
        )
        .map_err(|e| format!("Failed to save schema: {e}"))?;
    Ok(schema)
}

/// Deletes the schema and, by cascade, every record extracted with it.
#[tauri::command]
pub async fn extraction_schema_delete(
    schema_id: String,
    db: State<'_, AppDbState>,
) -> Result<(), String> {
    open(&db)?
        .execute("DELETE FROM extraction_schemas WHERE id = ?1", [schema_id])
        .map(|_| ())
        .map_err(|e| format!("Failed to delete schema: {e}"))
}

#[derive(Debug, Serialize)]
pub struct ExtractionRecordRow {
    pub item_id: String,
    pub item_title: String,
    pub asset_id: String,
    pub record: Value,
}

#[tauri::command]
pub async fn extraction_records_list(
    schema_id: String,
    db: State<'_, AppDbState>,
) -> Result<Vec<ExtractionRecordRow>, String> {
    let conn = open(&db)?;
    let mut stmt = conn
        .prepare(
            "SELECT r.item_id, COALESCE(i.title, ''), r.asset_id, r.record_json
               FROM extraction_records r LEFT JOIN items i ON i.id = r.item_id
              WHERE r.schema_id = ?1
              ORDER BY i.title COLLATE NOCASE, r.asset_id, r.created_at",
        )
        .map_err(|e| format!("Failed to list records: {e}"))?;
    let rows = stmt
        .query_map([&schema_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| format!("Failed to list records: {e}"))?;
    Ok(rows
        .into_iter()
        .map(
            |(item_id, item_title, asset_id, json)| ExtractionRecordRow {
                item_id,
                item_title,
                asset_id,
                record: serde_json::from_str(&json).unwrap_or(Value::Null),
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> Vec<SchemaField> {
        vec![
            SchemaField {
                name: "barco".into(),
                description: String::new(),
                repeatable: false,
            },
            SchemaField {
                name: "carga".into(),
                description: String::new(),
                repeatable: true,
            },
        ]
    }

    #[test]
    fn parsing_keeps_schema_keys_lists_repeatables_and_drops_empty_records() {
        let raw = r#"Acá va: [
            {"barco": "La Esperanza", "carga": ["cueros", "sebo", ""], "capitán": "X"},
            {"barco": "Fragata", "carga": "yerba"},
            {"barco": null, "carga": []}
        ] listo"#;
        let records = parse_records(raw, &fields());
        assert_eq!(
            records,
            vec![
                serde_json::json!({"barco": "La Esperanza", "carga": ["cueros", "sebo"]}),
                serde_json::json!({"barco": "Fragata", "carga": ["yerba"]}),
            ]
        );
        assert!(parse_records("no hay nada", &fields()).is_empty());
    }

    #[test]
    fn the_contract_carries_the_schema_id() {
        assert_eq!(schema_id_from_contract(&contract_for("s-1")), Some("s-1"));
        assert_eq!(schema_id_from_contract("triples:v1"), None);
    }
}
