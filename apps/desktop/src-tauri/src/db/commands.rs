use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rusqlite::types::Value;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tauri::State;

use crate::db::state::AppDbState;
use crate::db::util::{is_safe_identifier, json_to_sql_param, quote_identifier};

const DB_BROWSER_HIDDEN_TABLES: &[&str] = &["app_settings", "_migrations", "fts_items"];
const DB_BROWSER_CANDIDATE_TABLES: &[&str] = &[
    "collections",
    "items",
    "assets",
    "notes",
    "extractions",
    "transcriptions",
    "entities",
    "triples",
    "topics",
    "item_topics",
    "vec_assets",
    "layouts",
    "llm_results",
    "annotations",
];

#[derive(Serialize)]
pub struct ExecuteResult {
    pub rows_affected: u64,
}

#[derive(Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DbBrowserTableInfo {
    pub name: String,
}

#[derive(Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DbBrowserColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub is_primary_key: bool,
}

#[derive(Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbBrowserQueryResponse {
    pub table: String,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub rows: Vec<serde_json::Value>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbBrowserQueryRequest {
    pub table: String,
    pub page: u32,
    pub page_size: u32,
    pub sort_column: Option<String>,
    pub sort_direction: Option<String>,
    pub search: Option<String>,
}

/// Run rusqlite work on the blocking thread pool so IPC commands never
/// execute SQL on the main thread (where the window event loop runs).
async fn run_blocking_db_task<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| format!("DB task failed: {e}"))?
}

/// Execute multiple SQL statements atomically within a transaction.
/// Used for cascade deletes and other multi-statement operations.
#[tauri::command]
pub async fn db_execute_batch(db: State<'_, AppDbState>, sql: String) -> Result<(), String> {
    validate_sql_batch(&sql)?;
    let conn = db.ui_conn.clone();
    run_blocking_db_task(move || {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch(&sql).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn db_execute(
    db: State<'_, AppDbState>,
    sql: String,
    params: Vec<serde_json::Value>,
) -> Result<ExecuteResult, String> {
    validate_sql_execute(&sql)?;
    let conn = db.ui_conn.clone();
    run_blocking_db_task(move || {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let params_ref: Vec<Box<dyn rusqlite::ToSql>> =
            params.iter().map(json_to_sql_param).collect();
        let params_as_refs: Vec<&dyn rusqlite::ToSql> =
            params_ref.iter().map(|b| b.as_ref()).collect();
        let rows_affected = conn
            .execute(&sql, params_as_refs.as_slice())
            .map_err(|e| e.to_string())?;
        Ok(ExecuteResult {
            rows_affected: rows_affected as u64,
        })
    })
    .await
}

#[tauri::command]
pub async fn db_select(
    db: State<'_, AppDbState>,
    sql: String,
    params: Vec<serde_json::Value>,
) -> Result<Vec<serde_json::Value>, String> {
    validate_sql_row_query(&sql)?;
    let conn = db.ui_conn.clone();
    run_blocking_db_task(move || {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let params_ref: Vec<Box<dyn rusqlite::ToSql>> =
            params.iter().map(json_to_sql_param).collect();
        let params_as_refs: Vec<&dyn rusqlite::ToSql> =
            params_ref.iter().map(|b| b.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let col_count = stmt.column_count();
        let col_names: Vec<String> = (0..col_count)
            .map(|i| stmt.column_name(i).unwrap_or("").to_string())
            .collect();

        let rows = stmt
            .query_map(params_as_refs.as_slice(), |row| {
                let mut map = serde_json::Map::new();
                for (i, name) in col_names.iter().enumerate() {
                    let val: Value = row.get(i)?;
                    map.insert(name.clone(), rusqlite_value_to_json(val));
                }
                Ok(serde_json::Value::Object(map))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(rows)
    })
    .await
}

/// Returns rows as arrays in column order — required by Drizzle sqlite-proxy
/// to guarantee correct column mapping (Object.values() order is not guaranteed).
#[tauri::command]
pub async fn db_select_rows(
    db: State<'_, AppDbState>,
    sql: String,
    params: Vec<serde_json::Value>,
) -> Result<Vec<Vec<serde_json::Value>>, String> {
    validate_sql_row_query(&sql)?;
    let conn = db.ui_conn.clone();
    run_blocking_db_task(move || {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let params_ref: Vec<Box<dyn rusqlite::ToSql>> =
            params.iter().map(json_to_sql_param).collect();
        let params_as_refs: Vec<&dyn rusqlite::ToSql> =
            params_ref.iter().map(|b| b.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let col_count = stmt.column_count();

        let rows = stmt
            .query_map(params_as_refs.as_slice(), |row| {
                let mut values = Vec::with_capacity(col_count);
                for i in 0..col_count {
                    let val: Value = row.get(i)?;
                    values.push(rusqlite_value_to_json(val));
                }
                Ok(values)
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(rows)
    })
    .await
}

#[tauri::command]
pub async fn db_browser_list_tables(
    db: State<'_, AppDbState>,
) -> Result<Vec<DbBrowserTableInfo>, String> {
    let conn = db.ui_conn.clone();
    run_blocking_db_task(move || {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        list_db_browser_tables(&conn)
    })
    .await
}

#[tauri::command]
pub async fn db_browser_describe_table(
    db: State<'_, AppDbState>,
    table: String,
) -> Result<Vec<DbBrowserColumnInfo>, String> {
    let conn = db.ui_conn.clone();
    run_blocking_db_task(move || {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        describe_db_browser_table(&conn, &table)
    })
    .await
}

#[tauri::command]
pub async fn db_browser_query_rows(
    db: State<'_, AppDbState>,
    table: String,
    page: u32,
    page_size: u32,
    sort_column: Option<String>,
    sort_direction: Option<String>,
    search: Option<String>,
) -> Result<DbBrowserQueryResponse, String> {
    let conn = db.ui_conn.clone();
    run_blocking_db_task(move || {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        query_db_browser_rows(
            &conn,
            DbBrowserQueryRequest {
                table,
                page,
                page_size,
                sort_column,
                sort_direction,
                search,
            },
        )
    })
    .await
}

fn list_db_browser_tables(conn: &Connection) -> Result<Vec<DbBrowserTableInfo>, String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type IN ('table', 'view')")
        .map_err(|e| format!("Failed to inspect sqlite schema: {e}"))?;

    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("Failed to query sqlite schema: {e}"))?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|e| format!("Failed to read sqlite schema: {e}"))?;

    Ok(DB_BROWSER_CANDIDATE_TABLES
        .iter()
        .filter(|table| !DB_BROWSER_HIDDEN_TABLES.contains(table) && names.contains(**table))
        .map(|name| DbBrowserTableInfo {
            name: (*name).to_string(),
        })
        .collect())
}

fn describe_db_browser_table(
    conn: &Connection,
    table: &str,
) -> Result<Vec<DbBrowserColumnInfo>, String> {
    ensure_db_browser_table_allowed(conn, table)?;

    let sql = format!("PRAGMA table_info({})", quote_identifier(table));
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to inspect table '{table}': {e}"))?;

    let columns = stmt
        .query_map([], |row| {
            Ok(DbBrowserColumnInfo {
                name: row.get::<_, String>(1)?,
                data_type: row.get::<_, String>(2).unwrap_or_default(),
                nullable: row.get::<_, i64>(3)? == 0,
                is_primary_key: row.get::<_, i64>(5)? > 0,
            })
        })
        .map_err(|e| format!("Failed to read columns for '{table}': {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect columns for '{table}': {e}"))?;

    if columns.is_empty() {
        return Err(format!("Table '{table}' has no browsable columns"));
    }

    Ok(columns)
}

fn query_db_browser_rows(
    conn: &Connection,
    request: DbBrowserQueryRequest,
) -> Result<DbBrowserQueryResponse, String> {
    let table = request.table.trim();
    let columns = describe_db_browser_table(conn, table)?;
    let column_names: Vec<String> = columns.iter().map(|column| column.name.clone()).collect();
    let sort_column = request
        .sort_column
        .as_deref()
        .filter(|name| column_names.iter().any(|column| column == name))
        .map(str::to_string)
        .unwrap_or_else(|| {
            columns
                .iter()
                .find(|column| column.is_primary_key)
                .map(|column| column.name.clone())
                .unwrap_or_else(|| column_names[0].clone())
        });
    let sort_direction = parse_sort_direction(request.sort_direction.as_deref());
    let page_size = request.page_size.clamp(1, 100);
    let page = request.page.max(1);
    let offset = (page.saturating_sub(1) as i64) * (page_size as i64);
    let search = request.search.unwrap_or_default().trim().to_string();
    let quoted_table = quote_identifier(table);
    let quoted_sort_column = quote_identifier(&sort_column);

    let search_clause = if search.is_empty() {
        String::new()
    } else {
        let clauses = column_names
            .iter()
            .map(|column| {
                format!(
                    "CAST({} AS TEXT) LIKE ?1 COLLATE NOCASE ESCAPE '\\'",
                    quote_identifier(column)
                )
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        format!(" WHERE {clauses}")
    };

    let total_sql = format!("SELECT COUNT(*) FROM {quoted_table}{search_clause}");
    let data_sql = format!(
        "SELECT * FROM {quoted_table}{search_clause} ORDER BY {quoted_sort_column} {sort_direction} LIMIT ?{} OFFSET ?{}",
        if search.is_empty() { "1" } else { "2" },
        if search.is_empty() { "2" } else { "3" }
    );

    let total = if search.is_empty() {
        conn.query_row(&total_sql, [], |row| row.get::<_, i64>(0))
    } else {
        let pattern = format!("%{}%", escape_like_pattern(&search));
        conn.query_row(&total_sql, rusqlite::params![pattern], |row| {
            row.get::<_, i64>(0)
        })
    }
    .map_err(|e| format!("Failed to count rows for '{table}': {e}"))?
    .max(0) as u64;

    let mut stmt = conn
        .prepare(&data_sql)
        .map_err(|e| format!("Failed to prepare rows query for '{table}': {e}"))?;

    let rows = if search.is_empty() {
        stmt.query_map(rusqlite::params![page_size as i64, offset], |row| {
            Ok(row_to_json(row, &column_names))
        })
        .map_err(|e| format!("Failed to query rows for '{table}': {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect rows for '{table}': {e}"))?
    } else {
        let pattern = format!("%{}%", escape_like_pattern(&search));
        stmt.query_map(
            rusqlite::params![pattern, page_size as i64, offset],
            |row| Ok(row_to_json(row, &column_names)),
        )
        .map_err(|e| format!("Failed to query rows for '{table}': {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect rows for '{table}': {e}"))?
    };

    Ok(DbBrowserQueryResponse {
        table: table.to_string(),
        page,
        page_size,
        total,
        rows,
    })
}

fn row_to_json(row: &rusqlite::Row<'_>, column_names: &[String]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (index, name) in column_names.iter().enumerate() {
        let value = row.get::<_, Value>(index).unwrap_or(Value::Null);
        map.insert(name.clone(), rusqlite_value_to_json(value));
    }
    serde_json::Value::Object(map)
}

fn ensure_db_browser_table_allowed(conn: &Connection, table: &str) -> Result<(), String> {
    if !is_safe_identifier(table) {
        return Err("Invalid table name".to_string());
    }

    let allowed = list_db_browser_tables(conn)?;
    if allowed.iter().any(|candidate| candidate.name == table) {
        Ok(())
    } else {
        Err(format!(
            "Table '{table}' is not available in the DB browser"
        ))
    }
}

/// Escape LIKE wildcards (`%`, `_`) and the escape character itself so user
/// searches match those characters literally. Pairs with `ESCAPE '\'` in the
/// LIKE clauses built above.
fn escape_like_pattern(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn parse_sort_direction(value: Option<&str>) -> &'static str {
    match value.unwrap_or("asc").to_ascii_lowercase().as_str() {
        "desc" => "DESC",
        _ => "ASC",
    }
}

fn rusqlite_value_to_json(val: Value) -> serde_json::Value {
    match val {
        Value::Null => serde_json::Value::Null,
        Value::Integer(i) => serde_json::Value::Number(i.into()),
        Value::Real(f) => serde_json::json!(f),
        Value::Text(s) => serde_json::Value::String(s),
        Value::Blob(b) => serde_json::Value::String(BASE64_STANDARD.encode(b)),
    }
}

fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// True when a statement references the sensitive `app_settings` table, which
/// holds API keys and other secrets. EntropIA Pro is 100% local, so these
/// secrets live in the same SQLite file as user data; the renderer must never
/// reach them through the generic db_* IPC surface.
fn sql_references_sensitive_table(sql: &str) -> bool {
    sql.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .any(|token| token == "app_settings")
}

/// Error returned when the renderer tries to write the sync state (DESIGN §6.2).
const SYNC_PROTECTION_ERROR: &str =
    "Restricted SQL statement: sync_* tables and trg_sync_* triggers are managed by the sync engine";

/// Detects whether a single normalized statement would WRITE the sync state:
/// any DML/DDL targeting a `sync_*` table, or any CREATE/DROP of a
/// `trg_sync_*` trigger (DESIGN §6.2). Reads (`SELECT … FROM sync_*`) are NOT
/// blocked — the renderer may inspect sync status, it just must never mutate
/// it, even by accident.
///
/// Matching is verb-anchored against the leading keyword so that a literal like
/// `'sync_oplog'` inside a write to a NON-sync table is not falsely rejected.
/// `normalized` is the lowercased, whitespace-collapsed statement produced by
/// [`normalize_sql`].
fn statement_writes_sync_objects(normalized: &str) -> bool {
    let leading = normalized.split(' ').next().unwrap_or("");
    match leading {
        // `INSERT INTO sync_x`, `INSERT OR REPLACE INTO sync_x`, `REPLACE INTO sync_x`.
        "insert" | "replace" => {
            statement_target_after("into", normalized).is_some_and(target_is_sync_table)
        }
        "update" => normalized
            .split(' ')
            .nth(1)
            .is_some_and(target_is_sync_table),
        // `DELETE FROM sync_x`.
        "delete" => statement_target_after("from", normalized).is_some_and(target_is_sync_table),
        // `ALTER TABLE sync_x`, `CREATE TABLE … sync_x`, `DROP TABLE … sync_x`,
        // `CREATE/DROP TRIGGER … trg_sync_x` (and their INDEX variants on sync_*).
        "alter" | "create" | "drop" => statement_touches_sync_ddl(normalized),
        _ => false,
    }
}

/// Returns the token immediately following `keyword` in the normalized
/// statement (the conventional position of the target object name).
fn statement_target_after<'a>(keyword: &str, normalized: &'a str) -> Option<&'a str> {
    let mut tokens = normalized.split(' ');
    while let Some(token) = tokens.next() {
        if token == keyword {
            return tokens.next();
        }
    }
    None
}

/// True when a CREATE/DROP/ALTER statement targets a sync object: a `sync_*`
/// table/index or a `trg_sync_*` trigger. DDL syntax varies (`IF NOT EXISTS`,
/// schema qualifiers, `ON table`), so this scans the statement's tokens for the
/// managed prefixes rather than anchoring on a fixed position — failing CLOSED.
fn statement_touches_sync_ddl(normalized: &str) -> bool {
    normalized
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|token| target_is_sync_table(token) || token.starts_with("trg_sync_"))
}

/// True when a bare object name refers to a sync-managed table. Strips an
/// optional double-quote wrap and a `main.`/`temp.` schema qualifier.
fn target_is_sync_table(token: &str) -> bool {
    let token = token.trim_matches('"');
    let bare = token.rsplit('.').next().unwrap_or(token);
    bare.starts_with("sync_")
}

fn validate_sql_row_query(sql: &str) -> Result<(), String> {
    let normalized = normalize_sql(sql);

    if normalized.contains(';') {
        return Err("db_select/db_select_rows accept only a single SQL statement".to_string());
    }

    for forbidden in ["pragma ", "attach ", "detach ", "vacuum "] {
        if normalized.starts_with(forbidden) || normalized.contains(&format!(" {forbidden}")) {
            return Err("Restricted SQL statement for db_select/db_select_rows".to_string());
        }
    }

    if sql_references_sensitive_table(&normalized) {
        return Err("Restricted sensitive table for db_select/db_select_rows".to_string());
    }

    if normalized.starts_with("select ") || normalized.starts_with("with ") {
        return Ok(());
    }

    let is_dml = normalized.starts_with("insert ")
        || normalized.starts_with("update ")
        || normalized.starts_with("delete ");

    if is_dml && normalized.contains(" returning ") {
        return Ok(());
    }

    Err(
        "Only row-returning queries (SELECT/WITH or DML with RETURNING) are allowed in db_select/db_select_rows"
            .to_string(),
    )
}

fn validate_sql_execute(sql: &str) -> Result<(), String> {
    let normalized = normalize_sql(sql);

    if normalized.contains(';') {
        return Err("db_execute accepts only a single SQL statement".to_string());
    }

    if normalized.starts_with("pragma ")
        || normalized.starts_with("attach ")
        || normalized.starts_with("detach ")
        || normalized.starts_with("vacuum ")
    {
        return Err("Restricted SQL statement for db_execute".to_string());
    }

    if sql_references_sensitive_table(&normalized) {
        return Err("Restricted sensitive table for db_execute".to_string());
    }

    if statement_writes_sync_objects(&normalized) {
        return Err(SYNC_PROTECTION_ERROR.to_string());
    }

    if normalized.starts_with("insert ")
        || normalized.starts_with("update ")
        || normalized.starts_with("delete ")
    {
        return Ok(());
    }

    Err("Only INSERT, UPDATE, or DELETE statements are allowed in db_execute".to_string())
}

/// Validate a multi-statement batch per statement: split on `;`, normalize
/// each statement, and check its LEADING keyword against the denylist used by
/// the single-statement validators. Substring matching is intentionally
/// avoided — a literal like `'please attach the file'` inside an INSERT must
/// not be rejected, while real ATTACH/DETACH/VACUUM/PRAGMA statements stay
/// blocked.
///
/// Limitation: the `;` split is NOT string-literal aware. A literal that
/// itself contains a semicolon followed by a denylisted keyword (e.g.
/// `'…;pragma …'`) is split mid-literal and the fragment after the `;` is
/// checked as if it started a statement, rejecting the batch. This fails
/// CLOSED — a legitimate batch may be falsely rejected, never the reverse.
///
/// Caller contract: batch callers must only interpolate semicolon-free
/// escaped identifiers/values (today: UUIDs) into batch SQL, which keeps the
/// false positive unreachable. Free-form user text must go through the
/// parameterized single-statement commands instead.
fn validate_sql_batch(sql: &str) -> Result<(), String> {
    for statement in sql.split(';') {
        let normalized = normalize_sql(statement);
        if normalized.is_empty() {
            continue;
        }
        let leading_keyword = normalized.split(' ').next().unwrap_or("");
        if matches!(leading_keyword, "attach" | "detach" | "vacuum" | "pragma") {
            return Err("Restricted SQL statement in db_execute_batch".to_string());
        }
        if sql_references_sensitive_table(&normalized) {
            return Err("Restricted sensitive table in db_execute_batch".to_string());
        }
        if statement_writes_sync_objects(&normalized) {
            return Err(SYNC_PROTECTION_ERROR.to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db_browser_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        conn.execute_batch(
            r#"
            CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at INTEGER NOT NULL);
            CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, created_at INTEGER NOT NULL);
            CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO collections (id, name, created_at) VALUES
                ('col-1', 'Archivo histórico', 10),
                ('col-2', 'Fotografías', 20);
            INSERT INTO items (id, title, collection_id, created_at) VALUES
                ('item-1', 'Acta fundacional', 'col-1', 10),
                ('item-2', 'Carta manuscrita', 'col-1', 20);
            "#,
        )
        .expect("test schema should be created");
        conn
    }

    #[test]
    fn db_browser_list_tables_excludes_sensitive_tables() {
        let conn = setup_db_browser_test_db();

        let tables = list_db_browser_tables(&conn).unwrap();
        let names: Vec<String> = tables.into_iter().map(|table| table.name).collect();

        assert!(names.contains(&"collections".to_string()));
        assert!(names.contains(&"items".to_string()));
        assert!(!names.contains(&"app_settings".to_string()));
    }

    #[test]
    fn sql_validators_reject_sensitive_app_settings_table() {
        assert_eq!(
            validate_sql_row_query("SELECT key, value FROM app_settings").unwrap_err(),
            "Restricted sensitive table for db_select/db_select_rows"
        );
        assert_eq!(
            validate_sql_row_query("SELECT key FROM \"app_settings\"").unwrap_err(),
            "Restricted sensitive table for db_select/db_select_rows"
        );
        assert_eq!(
            validate_sql_execute("UPDATE app_settings SET value = ? WHERE key = ?").unwrap_err(),
            "Restricted sensitive table for db_execute"
        );
        assert_eq!(
            validate_sql_batch("BEGIN; DELETE FROM app_settings; COMMIT;").unwrap_err(),
            "Restricted sensitive table in db_execute_batch"
        );
    }

    #[test]
    fn sql_validators_protect_sync_state_from_renderer_writes() {
        // The renderer must never mutate sync bookkeeping (DESIGN §6.2).
        assert_eq!(
            validate_sql_execute("INSERT INTO sync_meta (key, value) VALUES ('x', '1')")
                .unwrap_err(),
            SYNC_PROTECTION_ERROR
        );
        assert_eq!(
            validate_sql_execute("UPDATE sync_meta SET value = '1' WHERE key = 'x'").unwrap_err(),
            SYNC_PROTECTION_ERROR
        );
        assert_eq!(
            validate_sql_execute("DELETE FROM sync_oplog WHERE seq = 1").unwrap_err(),
            SYNC_PROTECTION_ERROR
        );
        assert_eq!(
            validate_sql_execute("INSERT OR REPLACE INTO sync_row_versions VALUES ('t','r',1)")
                .unwrap_err(),
            SYNC_PROTECTION_ERROR
        );
        // Batch path blocks sync DML and sync DDL (tables and trg_sync_* triggers).
        assert_eq!(
            validate_sql_batch("DELETE FROM sync_oplog; DELETE FROM sync_conflicts;").unwrap_err(),
            SYNC_PROTECTION_ERROR
        );
        assert_eq!(
            validate_sql_batch("DROP TABLE sync_oplog;").unwrap_err(),
            SYNC_PROTECTION_ERROR
        );
        assert_eq!(
            validate_sql_batch("ALTER TABLE sync_meta ADD COLUMN x TEXT;").unwrap_err(),
            SYNC_PROTECTION_ERROR
        );
        assert_eq!(
            validate_sql_batch(
                "CREATE TRIGGER trg_sync_items_u AFTER UPDATE ON items BEGIN SELECT 1; END;"
            )
            .unwrap_err(),
            SYNC_PROTECTION_ERROR
        );
        assert_eq!(
            validate_sql_batch("DROP TRIGGER IF EXISTS trg_sync_items_d;").unwrap_err(),
            SYNC_PROTECTION_ERROR
        );
    }

    #[test]
    fn sql_validators_allow_sync_reads_and_literal_mentions() {
        // Reads of sync_* are allowed — the renderer may inspect status.
        assert!(
            validate_sql_row_query("SELECT value FROM sync_meta WHERE key = 'pending'").is_ok()
        );
        // A 'sync_oplog' string literal inside a write to a NON-sync table is not
        // falsely rejected (verb-anchored matching).
        assert!(validate_sql_execute(
            "INSERT INTO notes (id, content) VALUES ('n-1', 'remember to sync_oplog later')"
        )
        .is_ok());
    }

    #[test]
    fn sql_validators_still_allow_regular_store_queries() {
        assert!(
            validate_sql_row_query("SELECT id, title FROM items WHERE collection_id = ?").is_ok()
        );
        assert!(validate_sql_execute("UPDATE items SET title = ? WHERE id = ?").is_ok());
        assert!(
            validate_sql_execute("INSERT INTO notes (id, item_id, content) VALUES (?, ?, ?)")
                .is_ok()
        );
        assert!(validate_sql_execute("DELETE FROM notes WHERE id = ?").is_ok());
        assert!(
            validate_sql_batch("BEGIN; DELETE FROM notes WHERE item_id = 'item-1'; COMMIT;")
                .is_ok()
        );
    }

    #[test]
    fn db_execute_rejects_schema_mutating_statements() {
        assert_eq!(
            validate_sql_execute("DROP TABLE items").unwrap_err(),
            "Only INSERT, UPDATE, or DELETE statements are allowed in db_execute"
        );
        assert_eq!(
            validate_sql_execute("ALTER TABLE items ADD COLUMN unsafe TEXT").unwrap_err(),
            "Only INSERT, UPDATE, or DELETE statements are allowed in db_execute"
        );
        assert_eq!(
            validate_sql_execute("CREATE TABLE unsafe_table (id TEXT)").unwrap_err(),
            "Only INSERT, UPDATE, or DELETE statements are allowed in db_execute"
        );
    }

    #[test]
    fn db_browser_query_rows_rejects_invalid_identifier() {
        let conn = setup_db_browser_test_db();

        let result = query_db_browser_rows(
            &conn,
            DbBrowserQueryRequest {
                table: "collections; DROP TABLE items".to_string(),
                page: 1,
                page_size: 25,
                sort_column: None,
                sort_direction: None,
                search: None,
            },
        );

        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), "Invalid table name");
    }

    #[test]
    fn db_browser_query_rows_applies_search_sort_and_pagination() {
        let conn = setup_db_browser_test_db();

        let response = query_db_browser_rows(
            &conn,
            DbBrowserQueryRequest {
                table: "collections".to_string(),
                page: 1,
                page_size: 1,
                sort_column: Some("name".to_string()),
                sort_direction: Some("desc".to_string()),
                search: Some("a".to_string()),
            },
        )
        .unwrap();

        assert_eq!(response.total, 2);
        assert_eq!(response.rows.len(), 1);
        assert_eq!(
            response.rows[0]["name"],
            serde_json::Value::String("Fotografías".to_string())
        );
    }

    #[test]
    fn db_browser_query_rows_matches_like_wildcards_literally() {
        let conn = setup_db_browser_test_db();
        conn.execute_batch(
            r#"
            INSERT INTO collections (id, name, created_at) VALUES
                ('col-3', '100% algodón', 30),
                ('col-4', '1000 hilados', 40);
            "#,
        )
        .expect("wildcard rows should insert");

        let response = query_db_browser_rows(
            &conn,
            DbBrowserQueryRequest {
                table: "collections".to_string(),
                page: 1,
                page_size: 25,
                sort_column: None,
                sort_direction: None,
                search: Some("100%".to_string()),
            },
        )
        .unwrap();

        assert_eq!(response.total, 1);
        assert_eq!(
            response.rows[0]["name"],
            serde_json::Value::String("100% algodón".to_string())
        );
    }

    #[test]
    fn validate_sql_batch_allows_denylist_words_inside_literals() {
        // Substring false positives: denylist words inside string literals or
        // identifiers must not block legitimate statements.
        assert!(validate_sql_batch(
            "INSERT INTO notes (id, content) VALUES ('n-1', 'please attach the file');"
        )
        .is_ok());
        assert!(validate_sql_batch(
            "UPDATE items SET title = 'vacuum cleaner manual' WHERE id = 'item-1';
             DELETE FROM notes WHERE content LIKE '%pragma%';"
        )
        .is_ok());
    }

    #[test]
    fn validate_sql_batch_blocks_restricted_leading_keywords() {
        assert!(validate_sql_batch("ATTACH DATABASE 'evil.db' AS evil;").is_err());
        assert!(validate_sql_batch("detach evil;").is_err());
        assert!(validate_sql_batch("PRAGMA journal_mode=DELETE;").is_err());
        assert!(validate_sql_batch("VACUUM").is_err());
        // Restricted statements hidden after legitimate ones stay blocked.
        assert!(validate_sql_batch(
            "DELETE FROM notes WHERE id = 'n-1'; ATTACH DATABASE 'evil.db' AS evil;"
        )
        .is_err());
        assert!(validate_sql_batch("DELETE FROM notes; \n  pragma temp_store = 2").is_err());
    }

    #[test]
    fn validate_sql_batch_allows_multi_statement_dml() {
        assert!(validate_sql_batch(
            "BEGIN;
             DELETE FROM assets WHERE item_id = 'item-1';
             DELETE FROM items WHERE id = 'item-1';
             COMMIT;"
        )
        .is_ok());
        assert!(validate_sql_batch("").is_ok());
        assert!(validate_sql_batch(";;").is_ok());
    }

    #[test]
    fn blob_values_encode_as_standard_base64_with_padding() {
        assert_eq!(
            rusqlite_value_to_json(Value::Blob(b"hi".to_vec())),
            serde_json::Value::String("aGk=".to_string())
        );
        assert_eq!(
            rusqlite_value_to_json(Value::Blob(vec![0xfb, 0xff, 0xbf])),
            serde_json::Value::String("+/+/".to_string())
        );
    }
}
