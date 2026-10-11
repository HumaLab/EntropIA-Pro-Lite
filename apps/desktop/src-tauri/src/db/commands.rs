use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rusqlite::types::Value;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::authorizer::RendererSqlAuthorizer;
use crate::db::sql_split::split_sql_statements;
use crate::db::state::AppDbState;
use crate::db::util::{is_safe_identifier, json_to_sql_param, quote_identifier};

/// Tables that exist but must never reach the renderer: `app_settings` holds
/// API keys (see [`sql_references_sensitive_table`]).
const DB_BROWSER_HIDDEN_TABLES: &[&str] = &["app_settings"];

/// Every ordinary table and view of the main schema, read from SQLite itself
/// so a table added by a migration shows up without touching this file.
/// `pragma_table_list` types FTS5 indexes as `virtual` and their storage as
/// `shadow`; both are derived from other tables and stay out, as do the
/// `sqlite_*` internals.
const DB_BROWSER_SCHEMA_SQL: &str = "SELECT name FROM pragma_table_list \
     WHERE schema = 'main' AND type IN ('table', 'view') \
     AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
     ORDER BY name";

#[derive(Debug, Serialize)]
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
pub(crate) async fn run_blocking_db_task<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| format!("DB task failed: {e}"))?
}

/// Renderer-facing message when the layer-2 authorizer (S-02a) denies a
/// statement. SQLite reports every denial as the same generic `SQLITE_AUTH`
/// error, so the policy detail stays on the Rust side.
const SQL_AUTHORIZER_DENIED_ERROR: &str =
    "Restricted SQL statement: denied by the database authorizer";

/// Maps SQLite failures to renderer-facing strings, keeping the layer-2
/// denial recognizable instead of leaking the generic "not authorized".
fn map_db_error(err: rusqlite::Error) -> String {
    if err.sqlite_error_code() == Some(rusqlite::ErrorCode::AuthorizationForStatementDenied) {
        SQL_AUTHORIZER_DENIED_ERROR.to_string()
    } else {
        err.to_string()
    }
}

/// Execute multiple SQL statements atomically within a transaction.
/// Used for cascade deletes and other multi-statement operations.
///
/// Statements run one at a time so the batch stops at the first error and the
/// backend can roll back an open renderer transaction; DDL is rejected before
/// SQLite sees it (layer 1) and again by the authorizer (layer 2).
#[tauri::command]
pub async fn db_execute_batch(db: State<'_, AppDbState>, sql: String) -> Result<(), String> {
    let db = db.inner().clone();
    run_blocking_db_task(move || execute_batch_on(&db, &sql)).await
}

/// Body of `db_execute_batch`, factored out so tests exercise the real
/// validation + authorizer path without a Tauri runtime.
fn execute_batch_on(db: &AppDbState, sql: &str) -> Result<(), String> {
    validate_sql_batch(sql)?;
    let conn = db.ui_conn.lock().map_err(|e| e.to_string())?;
    for statement in split_sql_statements(sql) {
        if let Err(error) = execute_batch_statement(&conn, &statement) {
            rollback_open_transaction(&conn);
            return Err(error);
        }
    }
    Ok(())
}

/// A failed statement after an explicit renderer `BEGIN` leaves the connection
/// inside a transaction until someone sends `ROLLBACK`. The backend issues that
/// rollback itself, on the still-locked connection, so no open transaction can
/// leak into the next IPC call. This is backend SQL, not renderer SQL, so it
/// runs without the authorizer; a rollback failure is swallowed because the
/// statement error is the one the renderer must see.
fn rollback_open_transaction(conn: &Connection) {
    if !conn.is_autocommit() {
        let _ = conn.execute_batch("ROLLBACK");
    }
}

/// Runs one split statement of a batch under the renderer authorizer.
fn execute_batch_statement(conn: &Connection, statement: &str) -> Result<(), String> {
    let _authorizer = RendererSqlAuthorizer::install(conn);
    conn.execute_batch(statement).map_err(map_db_error)
}

#[derive(Debug, Deserialize)]
pub struct ParameterizedStatement {
    sql: String,
    #[serde(default)]
    params: Vec<serde_json::Value>,
}

/// Execute parameterized DML statements atomically on the UI database connection.
#[tauri::command]
pub async fn db_execute_transaction(
    db: State<'_, AppDbState>,
    statements: Vec<ParameterizedStatement>,
) -> Result<(), String> {
    let db = db.inner().clone();
    run_blocking_db_task(move || execute_transaction_on(&db, &statements)).await
}

/// Body of `db_execute_transaction`, factored out for tests.
fn execute_transaction_on(
    db: &AppDbState,
    statements: &[ParameterizedStatement],
) -> Result<(), String> {
    for statement in statements {
        validate_sql_execute(&statement.sql)?;
    }

    let mut conn = db.ui_conn.lock().map_err(|e| e.to_string())?;
    // IMMEDIATE takes the write lock up front, so a busy archive makes this
    // wait out busy_timeout. A DEFERRED transaction that reads first fails
    // instantly with `database is locked` when a worker is mid-write:
    // SQLite refuses to upgrade a read lock rather than risk a deadlock.
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    // SQLite authorizes trigger bodies when they fire, not when the outer
    // statement is prepared, so the guard stays installed while the statements
    // run. It is lifted before COMMIT because the borrow checker cannot move
    // `tx` while the guard borrows it; the policy allows the transaction
    // actions themselves anyway.
    let authorizer = RendererSqlAuthorizer::install(&tx);

    for statement in statements {
        let params: Vec<Box<dyn rusqlite::ToSql>> =
            statement.params.iter().map(json_to_sql_param).collect();
        let params_ref: Vec<&dyn rusqlite::ToSql> =
            params.iter().map(|param| param.as_ref()).collect();
        tx.execute(&statement.sql, params_ref.as_slice())
            .map_err(map_db_error)?;
    }

    drop(authorizer);
    tx.commit().map_err(map_db_error)
}

#[tauri::command]
pub async fn db_execute(
    db: State<'_, AppDbState>,
    sql: String,
    params: Vec<serde_json::Value>,
) -> Result<ExecuteResult, String> {
    let db = db.inner().clone();
    run_blocking_db_task(move || execute_on(&db, &sql, &params)).await
}

/// Body of `db_execute`, factored out for tests.
fn execute_on(
    db: &AppDbState,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<ExecuteResult, String> {
    validate_sql_execute(sql)?;
    let conn = db.ui_conn.lock().map_err(|e| e.to_string())?;
    let _authorizer = RendererSqlAuthorizer::install(&conn);
    let params_ref: Vec<Box<dyn rusqlite::ToSql>> = params.iter().map(json_to_sql_param).collect();
    let params_as_refs: Vec<&dyn rusqlite::ToSql> = params_ref.iter().map(|b| b.as_ref()).collect();
    let rows_affected = conn
        .execute(sql, params_as_refs.as_slice())
        .map_err(map_db_error)?;
    Ok(ExecuteResult {
        rows_affected: rows_affected as u64,
    })
}

#[tauri::command]
pub async fn db_select(
    db: State<'_, AppDbState>,
    sql: String,
    params: Vec<serde_json::Value>,
) -> Result<Vec<serde_json::Value>, String> {
    let db = db.inner().clone();
    run_blocking_db_task(move || select_on(&db, &sql, &params)).await
}

/// Body of `db_select`, factored out for tests.
fn select_on(
    db: &AppDbState,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<Vec<serde_json::Value>, String> {
    validate_sql_row_query(sql)?;
    let conn = db.ui_conn.lock().map_err(|e| e.to_string())?;
    let _authorizer = RendererSqlAuthorizer::install(&conn);
    let params_ref: Vec<Box<dyn rusqlite::ToSql>> = params.iter().map(json_to_sql_param).collect();
    let params_as_refs: Vec<&dyn rusqlite::ToSql> = params_ref.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(sql).map_err(map_db_error)?;
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
        .map_err(map_db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_db_error)?;

    Ok(rows)
}

/// Returns rows as arrays in column order — required by Drizzle sqlite-proxy
/// to guarantee correct column mapping (Object.values() order is not guaranteed).
#[tauri::command]
pub async fn db_select_rows(
    db: State<'_, AppDbState>,
    sql: String,
    params: Vec<serde_json::Value>,
) -> Result<Vec<Vec<serde_json::Value>>, String> {
    let db = db.inner().clone();
    run_blocking_db_task(move || select_rows_on(&db, &sql, &params)).await
}

/// Body of `db_select_rows`, factored out for tests.
fn select_rows_on(
    db: &AppDbState,
    sql: &str,
    params: &[serde_json::Value],
) -> Result<Vec<Vec<serde_json::Value>>, String> {
    validate_sql_row_query(sql)?;
    let conn = db.ui_conn.lock().map_err(|e| e.to_string())?;
    let _authorizer = RendererSqlAuthorizer::install(&conn);
    let params_ref: Vec<Box<dyn rusqlite::ToSql>> = params.iter().map(json_to_sql_param).collect();
    let params_as_refs: Vec<&dyn rusqlite::ToSql> = params_ref.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(sql).map_err(map_db_error)?;
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
        .map_err(map_db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_db_error)?;

    Ok(rows)
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
        .prepare(DB_BROWSER_SCHEMA_SQL)
        .map_err(|e| format!("Failed to inspect sqlite schema: {e}"))?;

    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("Failed to query sqlite schema: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read sqlite schema: {e}"))?;

    // A name the query path would refuse to quote is not listed either, so
    // everything the selector offers can actually be opened.
    Ok(names
        .into_iter()
        .filter(|name| !DB_BROWSER_HIDDEN_TABLES.contains(&name.as_str()))
        .filter(|name| is_safe_identifier(name))
        .map(|name| DbBrowserTableInfo { name })
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

/// Error returned when a statement cannot be lexed safely: an unterminated
/// string literal, quoted identifier or block comment. Failing closed rejects
/// the statement instead of guessing where it ends.
const MALFORMED_SQL_ERROR: &str =
    "Malformed SQL statement: unterminated string literal, identifier, or comment";

/// Removes SQL comments (`-- …` to end of line and `/* … */`) before any
/// keyword or table-name analysis, preserving string literals and quoted
/// identifiers so `'a -- b'` or `'/* not a comment */'` survives untouched.
/// Each comment is replaced by one space so `INSERT/*c*/INTO` cannot glue two
/// tokens into one.
///
/// The scanner tracks single-quoted strings (`''` escapes), double-quoted and
/// backtick identifiers, and bracketed identifiers. It fails CLOSED: a run
/// that ends inside a string, identifier, or block comment is an error rather
/// than an educated guess. It is still lexical only — not a SQL parser — but
/// it closes the comment-prefix evasions the audit found.
fn strip_sql_comments(sql: &str) -> Result<String, String> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ScanState {
        Normal,
        LineComment,
        BlockComment,
        SingleQuote,
        DoubleQuote,
        Backtick,
        Bracket,
    }

    let mut out = String::with_capacity(sql.len());
    let mut state = ScanState::Normal;
    let mut chars = sql.chars().peekable();

    while let Some(ch) = chars.next() {
        match state {
            ScanState::Normal => match ch {
                '-' if chars.peek() == Some(&'-') => {
                    chars.next();
                    out.push(' ');
                    state = ScanState::LineComment;
                }
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    out.push(' ');
                    state = ScanState::BlockComment;
                }
                '\'' => {
                    out.push(ch);
                    state = ScanState::SingleQuote;
                }
                '"' => {
                    out.push(ch);
                    state = ScanState::DoubleQuote;
                }
                '`' => {
                    out.push(ch);
                    state = ScanState::Backtick;
                }
                '[' => {
                    out.push(ch);
                    state = ScanState::Bracket;
                }
                _ => out.push(ch),
            },
            ScanState::LineComment => {
                if ch == '\n' {
                    out.push('\n');
                    state = ScanState::Normal;
                }
            }
            ScanState::BlockComment => {
                if ch == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    state = ScanState::Normal;
                }
            }
            ScanState::SingleQuote => {
                out.push(ch);
                if ch == '\'' {
                    if chars.peek() == Some(&'\'') {
                        chars.next();
                        out.push('\'');
                    } else {
                        state = ScanState::Normal;
                    }
                }
            }
            ScanState::DoubleQuote => {
                out.push(ch);
                if ch == '"' {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        out.push('"');
                    } else {
                        state = ScanState::Normal;
                    }
                }
            }
            ScanState::Backtick => {
                out.push(ch);
                if ch == '`' {
                    if chars.peek() == Some(&'`') {
                        chars.next();
                        out.push('`');
                    } else {
                        state = ScanState::Normal;
                    }
                }
            }
            ScanState::Bracket => {
                out.push(ch);
                if ch == ']' {
                    state = ScanState::Normal;
                }
            }
        }
    }

    match state {
        // A line comment may run to end of input; everything else unbalanced
        // is malformed.
        ScanState::Normal | ScanState::LineComment => Ok(out),
        _ => Err(MALFORMED_SQL_ERROR.to_string()),
    }
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

/// Layer-1 error for DDL (4.4a-2): the renderer owns only DML and reads; every
/// schema statement belongs to the backend migration runner. Layer 2 denies
/// the same actions semantically, so this message adds the detail SQLite's
/// generic authorization error cannot carry.
const DDL_STATEMENT_ERROR: &str =
    "DDL statements (CREATE, DROP, ALTER) are not allowed from the renderer";

/// Detects whether a single normalized statement would WRITE the sync state:
/// any DML/DDL targeting a `sync_*` table, or any CREATE/DROP of a
/// `trg_sync_*` trigger (DESIGN §6.2). Reads (`SELECT … FROM sync_*`) are NOT
/// blocked — the renderer may inspect sync status, it just must never mutate
/// it, even by accident.
///
/// Matching is verb-anchored so a literal like `'sync_oplog'` inside a write to
/// a NON-sync table is not falsely rejected, but every verb position is scanned
/// (not just the leading keyword) so `WITH … INSERT/UPDATE/DELETE` is covered;
/// `UPDATE OR REPLACE|IGNORE` skips the conflict clause before reading the
/// target. `normalized` is the lowercased, whitespace-collapsed statement
/// produced by [`normalize_sql`].
fn statement_writes_sync_objects(normalized: &str) -> bool {
    let leading = normalized.split(' ').next().unwrap_or("");
    if matches!(leading, "alter" | "create" | "drop") {
        return statement_touches_sync_ddl(normalized);
    }
    if !matches!(leading, "insert" | "replace" | "update" | "delete" | "with") {
        return false;
    }

    let tokens: Vec<&str> = normalized.split(' ').collect();
    tokens.iter().enumerate().any(|(index, token)| {
        let target = match *token {
            // `INSERT INTO sync_x`, `INSERT OR REPLACE INTO sync_x`,
            // `REPLACE INTO sync_x`.
            "insert" | "replace" => target_after_in(&tokens[index + 1..], "into"),
            // `UPDATE sync_x`, `UPDATE OR REPLACE|IGNORE sync_x`.
            "update" => update_target_in(&tokens[index + 1..]),
            // `DELETE FROM sync_x`.
            "delete" => target_after_in(&tokens[index + 1..], "from"),
            _ => None,
        };
        target.is_some_and(target_is_sync_table)
    })
}

/// Returns the token immediately following `keyword` in `tokens` (the
/// conventional position of the target object name).
fn target_after_in<'a>(tokens: &[&'a str], keyword: &str) -> Option<&'a str> {
    let at = tokens.iter().position(|token| *token == keyword)?;
    tokens.get(at + 1).copied()
}

/// The table named by the tokens right after an `UPDATE` verb: skips an
/// optional `OR REPLACE|IGNORE|ABORT|FAIL|ROLLBACK` conflict clause.
fn update_target_in<'a>(tokens: &[&'a str]) -> Option<&'a str> {
    let mut run = tokens.iter().copied();
    let first = run.next()?;
    if first == "or" {
        run.next()?; // the conflict action
        run.next()
    } else {
        Some(first)
    }
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
    let normalized = normalize_sql(&strip_sql_comments(sql)?);

    // DDL is rejected before the single-statement check so a `CREATE TRIGGER`
    // with semicolons in its body gets the DDL message, not the batching one.
    if starts_with_ddl(&normalized) {
        return Err(DDL_STATEMENT_ERROR.to_string());
    }

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

    // `INSERT/UPDATE/DELETE … RETURNING` is a write in read clothing; it must
    // respect the sync protection like `db_execute` does.
    if statement_writes_sync_objects(&normalized) {
        return Err(SYNC_PROTECTION_ERROR.to_string());
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
    let normalized = normalize_sql(&strip_sql_comments(sql)?);

    // DDL is rejected before the single-statement check so a `CREATE TRIGGER`
    // with semicolons in its body gets the DDL message, not the batching one.
    if starts_with_ddl(&normalized) {
        return Err(DDL_STATEMENT_ERROR.to_string());
    }

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

/// Validate a multi-statement batch per statement: strip comments, split on
/// `;`, normalize each statement, and check its LEADING keyword against the
/// denylist used by the single-statement validators. Substring matching is
/// intentionally avoided — a literal like `'please attach the file'` inside an
/// INSERT must not be rejected, while real ATTACH/DETACH/VACUUM/PRAGMA
/// statements stay blocked. DDL (`CREATE`/`DROP`/`ALTER`) is rejected as well
/// (4.4a-2): migrations run in the backend, so the renderer has no schema
/// statement to make. The sensitive-table and sync guards run before it so a
/// DDL statement touching either still reports its specific protection error.
///
/// Limitation: the `;` split used by the deny checks is NOT string-literal
/// aware. A literal that itself contains a semicolon followed by a denylisted
/// keyword (e.g. `'…;pragma …'`) is split mid-literal and the fragment after
/// the `;` is checked as if it started a statement, rejecting the batch. This
/// fails CLOSED — a legitimate batch may be falsely rejected, never the
/// reverse. Layer 2 (the authorizer) stays the semantic backstop for anything
/// this text pass cannot see.
///
/// Caller contract: batch callers must only interpolate semicolon-free
/// escaped identifiers/values (today: UUIDs) into batch SQL, which keeps the
/// false positive unreachable. Free-form user text must go through the
/// parameterized single-statement commands instead.
fn validate_sql_batch(sql: &str) -> Result<(), String> {
    let stripped = strip_sql_comments(sql)?;
    for statement in stripped.split(';') {
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
        if starts_with_ddl(&normalized) {
            return Err(DDL_STATEMENT_ERROR.to_string());
        }
    }
    Ok(())
}

/// True when the normalized statement's first keyword is DDL: `CREATE`,
/// `DROP` or `ALTER`. Comments are already stripped by the caller, so a
/// leading `/* … */` cannot hide one. `EXPLAIN` and `WITH` cannot prefix DDL
/// in SQLite; layer 2 denies anything hidden from this text check anyway.
fn starts_with_ddl(normalized: &str) -> bool {
    matches!(
        normalized.split(' ').next().unwrap_or(""),
        "create" | "drop" | "alter"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_must_go_through_db_execute_batch_not_db_execute() {
        // Pro's db_execute is DML-only and REJECTS ROLLBACK, so a renderer-issued
        // rollback that still goes through the batch validator is the only one
        // accepted today. The backend issues its own rollback for a failed batch
        // (see the tests below), which never passes through either validator.
        assert!(validate_sql_execute("ROLLBACK").is_err());
        assert!(validate_sql_batch("ROLLBACK").is_ok());
    }

    #[test]
    fn a_failing_batch_rolls_back_its_open_transaction_and_reports_the_error() {
        // A renderer batch that opens a transaction and fails mid-way must not
        // leave the connection inside that transaction: the backend rolls it
        // back on the same locked connection before returning the error, so the
        // partial delete cannot survive into the next IPC call.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE assets (id TEXT PRIMARY KEY);
             CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id));
             INSERT INTO assets (id) VALUES ('a1'), ('a2');
             INSERT INTO extractions (id, asset_id) VALUES ('e1', 'a1');",
        )
        .unwrap();
        let db = test_db(conn);

        // The DELETE of a2 applies, then the FK-violating INSERT aborts the
        // batch before COMMIT.
        let error = execute_batch_on(
            &db,
            "BEGIN;\n\
             DELETE FROM assets WHERE id = 'a2';\n\
             INSERT INTO extractions (id, asset_id) VALUES ('e2', 'missing');\n\
             COMMIT;",
        )
        .unwrap_err();
        assert!(
            error.to_lowercase().contains("foreign key"),
            "unexpected error: {error}"
        );

        let conn = db.ui_conn.lock().unwrap();
        assert!(
            conn.is_autocommit(),
            "the backend must roll back the transaction the failed batch left open"
        );
        let assets: i64 = conn
            .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            assets, 2,
            "the backend rollback must undo the partial delete"
        );
    }

    #[test]
    fn a_failing_batch_without_an_open_transaction_still_reports_the_error() {
        // No BEGIN was issued, so there is nothing to roll back; the batch must
        // still surface the statement error unchanged.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE assets (id TEXT PRIMARY KEY);
             CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id));
             INSERT INTO assets (id) VALUES ('a1');",
        )
        .unwrap();
        let db = test_db(conn);

        let error = execute_batch_on(
            &db,
            "INSERT INTO extractions (id, asset_id) VALUES ('e2', 'missing');",
        )
        .unwrap_err();
        assert!(
            error.to_lowercase().contains("foreign key"),
            "unexpected error: {error}"
        );
        assert!(
            db.ui_conn.lock().unwrap().is_autocommit(),
            "a batch without BEGIN must stay in autocommit"
        );
    }

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
    fn db_browser_lists_every_ordinary_table_and_view_but_no_internals() {
        let conn = setup_db_browser_test_db();
        conn.execute_batch(
            r#"
            CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL);
            INSERT INTO _migrations (name) VALUES ('0001_initial.sql');
            CREATE TABLE writing_journal (id TEXT PRIMARY KEY);
            CREATE VIRTUAL TABLE fts_items USING fts5(item_id UNINDEXED, title);
            CREATE VIRTUAL TABLE fts_items_vocab USING fts5vocab(fts_items, 'row');
            CREATE VIEW recent_items AS SELECT id, title FROM items;
            "#,
        )
        .unwrap();

        let names: Vec<String> = list_db_browser_tables(&conn)
            .unwrap()
            .into_iter()
            .map(|table| table.name)
            .collect();

        // Sorted, every ordinary table and view, no hand-kept candidate list.
        assert_eq!(
            names,
            vec![
                "_migrations",
                "collections",
                "items",
                "recent_items",
                "writing_journal"
            ]
        );
    }

    /// Parity against a real archive: the browser must list exactly what
    /// `sqlite_master` holds minus the documented exclusions, and every listed
    /// table must describe, sort, filter and page. Run on a COPY of the live
    /// file (the check is read-only, but a copy never races the app):
    /// `ENTROPIA_DB_BROWSER_PARITY_DB=<copy>/entropia.sqlite cargo test --lib
    /// db_browser_matches_a_real_archive -- --ignored --nocapture`
    #[test]
    #[ignore = "needs ENTROPIA_DB_BROWSER_PARITY_DB pointing at a copy of a real archive"]
    fn db_browser_matches_a_real_archive() {
        let path = std::env::var("ENTROPIA_DB_BROWSER_PARITY_DB")
            .expect("set ENTROPIA_DB_BROWSER_PARITY_DB");
        let conn =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();

        // A: straight from sqlite_master, exclusions computed independently.
        let objects: Vec<(String, String)> = conn
            .prepare("SELECT name, COALESCE(sql, '') FROM sqlite_master WHERE type IN ('table', 'view') ORDER BY name")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let virtual_tables: Vec<&str> = objects
            .iter()
            .filter(|(_, sql)| sql.to_ascii_uppercase().starts_with("CREATE VIRTUAL TABLE"))
            .map(|(name, _)| name.as_str())
            .collect();
        let mut excluded = Vec::new();
        let expected: Vec<String> = objects
            .iter()
            .filter(|(name, _)| {
                let reason = if name.starts_with("sqlite_") {
                    Some("sqlite internal")
                } else if name == "app_settings" {
                    Some("secrets")
                } else if virtual_tables.contains(&name.as_str()) {
                    Some("virtual (FTS5)")
                } else if virtual_tables
                    .iter()
                    .any(|v| name.starts_with(&format!("{v}_")))
                {
                    Some("FTS5 shadow")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    excluded.push(format!("{name} ({reason})"));
                }
                reason.is_none()
            })
            .map(|(name, _)| name.clone())
            .collect();

        // B: what the DB browser offers.
        let listed: Vec<String> = list_db_browser_tables(&conn)
            .unwrap()
            .into_iter()
            .map(|table| table.name)
            .collect();

        println!("sqlite_master tables/views: {}", objects.len());
        println!("excluded ({}): {}", excluded.len(), excluded.join(", "));
        println!("listed ({}): {}", listed.len(), listed.join(", "));
        assert_eq!(listed, expected);

        for table in &listed {
            let columns = describe_db_browser_table(&conn, table).unwrap();
            let last = columns.last().unwrap().name.clone();
            for (sort, direction, search, page) in [
                (None, None, None, 1),
                (Some(last.clone()), Some("desc"), None, 2),
                (None, None, Some("a"), 1),
            ] {
                let response = query_db_browser_rows(
                    &conn,
                    DbBrowserQueryRequest {
                        table: table.clone(),
                        page,
                        page_size: 100,
                        sort_column: sort,
                        sort_direction: direction.map(str::to_string),
                        search: search.map(str::to_string),
                    },
                )
                .unwrap_or_else(|e| panic!("{table}: {e}"));
                assert!(
                    response.rows.len() <= 100,
                    "{table}: page size not honoured"
                );
            }
            println!("  {table}: {} columns ok", columns.len());
        }
    }

    #[test]
    fn db_browser_picks_up_a_table_created_after_startup() {
        let conn = setup_db_browser_test_db();
        conn.execute_batch(
            "CREATE TABLE added_by_migration (id TEXT PRIMARY KEY, label TEXT);
             INSERT INTO added_by_migration VALUES ('a-1', 'nueva');",
        )
        .unwrap();

        assert!(list_db_browser_tables(&conn)
            .unwrap()
            .iter()
            .any(|table| table.name == "added_by_migration"));
        let columns = describe_db_browser_table(&conn, "added_by_migration").unwrap();
        assert_eq!(
            columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["id", "label"]
        );
        let response = query_db_browser_rows(
            &conn,
            DbBrowserQueryRequest {
                table: "added_by_migration".to_string(),
                page: 1,
                page_size: 25,
                sort_column: None,
                sort_direction: None,
                search: Some("nue".to_string()),
            },
        )
        .unwrap();
        assert_eq!(response.total, 1);
    }

    #[test]
    fn db_browser_rejects_tables_outside_the_discovered_set() {
        let conn = setup_db_browser_test_db();
        conn.execute_batch("CREATE VIRTUAL TABLE fts_items USING fts5(title);")
            .unwrap();

        for table in [
            "app_settings",
            "sqlite_schema",
            "fts_items",
            "fts_items_data",
        ] {
            assert_eq!(
                describe_db_browser_table(&conn, table).err().unwrap(),
                format!("Table '{table}' is not available in the DB browser"),
            );
        }
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
                "CREATE TRIGGER trg_sync_items_u AFTER UPDATE ON items BEGIN SELECT 1; END;",
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
            validate_sql_batch("BEGIN; DELETE FROM notes WHERE item_id = 'item-1'; COMMIT;",)
                .is_ok()
        );
    }

    #[test]
    fn db_execute_rejects_schema_mutating_statements() {
        assert_eq!(
            validate_sql_execute("DROP TABLE items").unwrap_err(),
            DDL_STATEMENT_ERROR
        );
        assert_eq!(
            validate_sql_execute("ALTER TABLE items ADD COLUMN unsafe TEXT").unwrap_err(),
            DDL_STATEMENT_ERROR
        );
        assert_eq!(
            validate_sql_execute("CREATE TABLE unsafe_table (id TEXT)").unwrap_err(),
            DDL_STATEMENT_ERROR
        );
        assert_eq!(
            validate_sql_row_query("DROP TABLE items").unwrap_err(),
            DDL_STATEMENT_ERROR
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
            "INSERT INTO notes (id, content) VALUES ('n-1', 'please attach the file');",
        )
        .is_ok());
        assert!(validate_sql_batch(
            "UPDATE items SET title = 'vacuum cleaner manual' WHERE id = 'item-1';
             DELETE FROM notes WHERE content LIKE '%pragma%';",
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
            "DELETE FROM notes WHERE id = 'n-1'; ATTACH DATABASE 'evil.db' AS evil;",
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
             COMMIT;",
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

    // -----------------------------------------------------------------------
    // S-02a/S-02c — layer 2 (authorizer) and layer 1. The S-02b migration
    // window, its allowlist and its exceptions were removed in 4.4a-2: both
    // layers now deny every DDL statement from the renderer.
    // -----------------------------------------------------------------------

    /// The real `AppDbState` the renderer command bodies receive in production,
    /// backed by an in-memory UI connection. The worker connection is never
    /// used by the `db_*` commands.
    fn test_db(conn: Connection) -> AppDbState {
        AppDbState::new(
            conn,
            Connection::open_in_memory().expect("worker in-memory db"),
            std::path::PathBuf::from(":memory:"),
        )
    }

    /// Real synced schema with capture triggers installed and a session whose
    /// `capture_enabled` flag is on. The sync-only test helpers live behind
    /// `#[cfg(test)]`, and capture is what the authorizer must not break.
    fn capture_enabled_db() -> AppDbState {
        let conn = crate::sync::test_support::new_synced_test_db();
        crate::sync::capture::ensure_capture(&conn).expect("install capture triggers");
        crate::sync::test_support::set_session_with_capture(&conn);
        test_db(conn)
    }

    fn oplog_count(db: &AppDbState) -> i64 {
        crate::sync::test_support::oplog_count(&db.ui_conn.lock().unwrap())
    }

    #[test]
    fn comments_cannot_hide_vacuum_into_or_attach_from_the_batch_validator() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("exfiltrated.db");
        let target_sql = target.to_str().expect("utf-8 path");
        let conn = test_db(Connection::open_in_memory().unwrap());

        // `/**/VACUUM INTO` and `--x\nVACUUM INTO` evade the old leading-keyword
        // match; the target file must not exist afterwards.
        for sql in [
            format!("/**/VACUUM INTO '{target_sql}'"),
            format!("--x\nVACUUM INTO '{target_sql}'"),
        ] {
            assert!(
                execute_batch_on(&conn, &sql).is_err(),
                "should be rejected: {sql}"
            );
            assert!(!target.exists(), "VACUUM INTO wrote {target_sql}");
        }

        // Same evasion for ATTACH.
        let attached = dir.path().join("attached.db");
        let attach_sql = format!("--x\nATTACH DATABASE '{}' AS e", attached.display());
        assert!(execute_batch_on(&conn, &attach_sql).is_err());
        assert!(!attached.exists(), "ATTACH created {}", attached.display());
    }

    #[test]
    fn db_select_cannot_write_sync_oplog_through_returning() {
        let conn = capture_enabled_db();
        let err = select_on(
            &conn,
            "INSERT INTO sync_oplog (table_name, row_id, op, changed_at) \
             VALUES ('items', 'forged', 'I', 1) RETURNING *",
            &[],
        )
        .unwrap_err();
        assert_eq!(err, SYNC_PROTECTION_ERROR);
        assert_eq!(oplog_count(&conn), 0, "no forged oplog row may land");
    }

    #[test]
    fn sync_write_detection_sees_with_dml_and_update_conflict_clauses() {
        let conn = capture_enabled_db();
        let err = execute_on(
            &conn,
            "WITH x AS (SELECT 1) INSERT INTO sync_meta(key, value) SELECT 'k', 'v' FROM x",
            &[],
        )
        .unwrap_err();
        assert_eq!(err, SYNC_PROTECTION_ERROR);

        let err = execute_on(
            &conn,
            "UPDATE OR REPLACE sync_meta SET value = '0' WHERE key = 'capture_enabled'",
            &[],
        )
        .unwrap_err();
        assert_eq!(err, SYNC_PROTECTION_ERROR);
        let capture: String = conn
            .ui_conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM sync_meta WHERE key = 'capture_enabled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(capture, "1", "the capture flag must not be overwritten");
    }

    #[test]
    fn commented_app_settings_reads_stay_rejected() {
        let conn = test_db(setup_db_browser_test_db());
        for sql in [
            "SELECT * FROM /* c */ app_settings",
            "SELECT * FROM app_settings -- c",
        ] {
            let err = select_on(&conn, sql, &[]).unwrap_err();
            assert_eq!(
                err,
                "Restricted sensitive table for db_select/db_select_rows"
            );
        }
    }

    #[test]
    fn layer_two_denies_app_settings_through_a_view_and_the_guard_is_removed() {
        let conn = setup_db_browser_test_db();
        conn.execute_batch("CREATE VIEW v_secret AS SELECT key, value FROM app_settings;")
            .unwrap();
        let conn = test_db(conn);

        // Layer 1 only sees `v_secret`; SQLite expands the view and reports a
        // read of `app_settings` (accessor = the view), which layer 2 denies.
        let err = select_on(&conn, "SELECT * FROM v_secret", &[]).unwrap_err();
        assert_eq!(err, SQL_AUTHORIZER_DENIED_ERROR);

        // The authorizer must be gone after the command: the backend keeps
        // reading app_settings directly on the same connection.
        let direct: i64 = conn
            .ui_conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM app_settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(direct, 0);
    }

    #[test]
    fn layer_two_alone_denies_a_subquery_over_app_settings() {
        let conn = setup_db_browser_test_db();
        {
            let _guard = RendererSqlAuthorizer::install(&conn);
            let denied = conn.query_row(
                "SELECT COUNT(*) FROM (SELECT * FROM app_settings)",
                [],
                |row| row.get::<_, i64>(0),
            );
            assert!(
                matches!(
                    denied,
                    Err(rusqlite::Error::SqliteFailure(error, _))
                        if error.code == rusqlite::ErrorCode::AuthorizationForStatementDenied
                ),
                "layer 2 alone must deny the read, got {denied:?}"
            );
        }
        // Dropping the guard uninstalls the authorizer: the backend read works.
        let allowed: i64 = conn
            .query_row("SELECT COUNT(*) FROM app_settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(allowed, 0);
    }

    #[test]
    fn plain_vacuum_is_also_denied_by_the_attach_check() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id INTEGER); INSERT INTO t VALUES (1);")
            .unwrap();
        let _guard = RendererSqlAuthorizer::install(&conn);
        // SQLite routes plain VACUUM through the same SQLITE_ATTACH check as
        // VACUUM INTO (with a NULL filename), so the guard denies it too. In
        // production layer 1 rejects it before the authorizer runs.
        let result = conn.execute_batch("VACUUM");
        assert!(result.is_err(), "plain VACUUM unexpectedly ran: {result:?}");
    }

    #[test]
    fn capture_still_records_writes_through_every_guarded_command_path() {
        let conn = capture_enabled_db();
        assert_eq!(
            oplog_count(&conn),
            0,
            "fresh session starts with an empty oplog"
        );

        // db_execute (INSERT).
        execute_on(
            &conn,
            "INSERT INTO collections (id, name, created_at, updated_at) \
             VALUES ('c-s02a', 'S-02a', 1, 1)",
            &[],
        )
        .unwrap();
        assert_eq!(oplog_count(&conn), 1);

        // db_execute_transaction (UPDATE).
        execute_transaction_on(
            &conn,
            &[ParameterizedStatement {
                sql: "UPDATE collections SET name = ? WHERE id = ?".to_string(),
                params: vec![serde_json::json!("S-02a v2"), serde_json::json!("c-s02a")],
            }],
        )
        .unwrap();
        assert_eq!(oplog_count(&conn), 2);

        // db_execute_batch (DELETE).
        execute_batch_on(
            &conn,
            "BEGIN; DELETE FROM collections WHERE id = 'c-s02a'; COMMIT;",
        )
        .unwrap();
        assert_eq!(oplog_count(&conn), 3);
    }

    #[test]
    fn comment_stripping_preserves_literals_and_fails_closed() {
        // `--` and `/*` inside a string literal are data, not comments.
        assert!(
            validate_sql_batch("INSERT INTO notes (id, content) VALUES ('n-1', 'a -- b');").is_ok()
        );
        assert!(validate_sql_batch(
            "INSERT INTO notes (id, content) VALUES ('n-1', '/* not a comment */');",
        )
        .is_ok());
        // An unterminated literal or block comment is rejected, never guessed.
        assert!(validate_sql_batch("INSERT INTO notes (id) VALUES ('unterminated").is_err());
        assert!(validate_sql_batch("INSERT INTO notes (id) VALUES ('n-1'); /* open").is_err());
        // A comment cannot smuggle a denylisted keyword to the front of a batch.
        assert!(validate_sql_batch("SELECT 1; --x\nATTACH DATABASE 'evil.db' AS e;").is_err());
    }

    // -----------------------------------------------------------------------
    // 4.4a-2 — DDL is closed on every renderer path (layers 1 and 2).
    // -----------------------------------------------------------------------

    /// Real schema of a small archive with `items` seeded, built outside the
    /// renderer path so the commands can only read and write rows.
    fn ddl_test_db() -> AppDbState {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL);
             CREATE INDEX idx_items_title ON items(title);
             INSERT INTO items (id, title) VALUES ('item-1', 'Acta fundacional');",
        )
        .unwrap();
        test_db(conn)
    }

    fn schema_objects(db: &AppDbState, names: &str) -> i64 {
        let query = format!("SELECT COUNT(*) FROM sqlite_master WHERE name IN ({names})");
        db.ui_conn
            .lock()
            .unwrap()
            .query_row(&query, [], |row| row.get(0))
            .unwrap()
    }

    fn items_rows(db: &AppDbState) -> i64 {
        db.ui_conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn ddl_is_denied_on_every_renderer_path_and_leaves_the_schema_intact() {
        let db = ddl_test_db();
        for sql in [
            "DROP TABLE items",
            "CREATE TABLE sneaky (id TEXT)",
            "CREATE INDEX idx_sneaky ON items(title)",
            "ALTER TABLE items ADD COLUMN sneaky TEXT",
            "CREATE TRIGGER trg_sneaky AFTER INSERT ON items BEGIN SELECT 1; END;",
            "CREATE VIEW v_sneaky AS SELECT id FROM items",
            "DROP INDEX idx_items_title",
            "CREATE VIRTUAL TABLE fts_sneaky USING fts5(body)",
        ] {
            assert_eq!(
                execute_batch_on(&db, sql).unwrap_err(),
                DDL_STATEMENT_ERROR,
                "db_execute_batch accepted {sql}"
            );
            assert_eq!(
                execute_on(&db, sql, &[]).unwrap_err(),
                DDL_STATEMENT_ERROR,
                "db_execute accepted {sql}"
            );
            assert_eq!(
                execute_transaction_on(
                    &db,
                    &[ParameterizedStatement {
                        sql: sql.to_string(),
                        params: vec![],
                    }]
                )
                .unwrap_err(),
                DDL_STATEMENT_ERROR,
                "db_execute_transaction accepted {sql}"
            );
        }

        assert_eq!(items_rows(&db), 1, "the seeded row must survive");
        assert_eq!(
            schema_objects(
                &db,
                "'sneaky','idx_sneaky','trg_sneaky','v_sneaky','fts_sneaky'"
            ),
            0,
            "no denied statement may have landed"
        );
        assert_eq!(
            schema_objects(&db, "'items','idx_items_title'"),
            2,
            "the original schema must be untouched"
        );
    }

    #[test]
    fn a_leading_comment_cannot_hide_ddl_from_layer_one() {
        let db = ddl_test_db();
        for sql in [
            "/* x */ DROP TABLE items",
            "-- x\nALTER TABLE items ADD COLUMN sneaky TEXT",
            "/* x */ CREATE TABLE sneaky (id TEXT)",
            "-- x\nCREATE VIRTUAL TABLE fts_sneaky USING fts5(body)",
        ] {
            assert_eq!(
                execute_batch_on(&db, sql).unwrap_err(),
                DDL_STATEMENT_ERROR,
                "db_execute_batch accepted {sql}"
            );
            assert_eq!(
                execute_on(&db, sql, &[]).unwrap_err(),
                DDL_STATEMENT_ERROR,
                "db_execute accepted {sql}"
            );
        }
        assert_eq!(items_rows(&db), 1);
        assert_eq!(
            schema_objects(&db, "'sneaky','fts_sneaky'"),
            0,
            "no denied statement may have landed"
        );
    }

    #[test]
    fn an_injected_ddl_statement_denies_the_whole_batch() {
        let db = ddl_test_db();
        let sql = "BEGIN;\n\
             DELETE FROM items WHERE id = 'item-1';\n\
             CREATE TRIGGER trg_injected AFTER INSERT ON items BEGIN SELECT 1; END;\n\
             COMMIT;";
        assert_eq!(execute_batch_on(&db, sql).unwrap_err(), DDL_STATEMENT_ERROR);
        assert_eq!(
            items_rows(&db),
            1,
            "layer 1 must reject the batch before any statement runs"
        );
        assert_eq!(schema_objects(&db, "'trg_injected'"), 0);
    }

    #[test]
    fn layer_two_alone_denies_ddl_when_the_validators_are_bypassed() {
        // The text validators are layer 1; this test calls SQLite directly
        // with the authorizer installed, so only layer 2 can deny.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL);
             CREATE INDEX idx_items_title ON items(title);
             CREATE TRIGGER trg_items AFTER INSERT ON items BEGIN SELECT 1; END;",
        )
        .unwrap();
        {
            let _guard = RendererSqlAuthorizer::install(&conn);
            for sql in [
                "CREATE TABLE t2 (id TEXT)",
                "CREATE INDEX idx_t2 ON items(title)",
                "CREATE TRIGGER trg_t2 AFTER INSERT ON items BEGIN SELECT 1; END;",
                "CREATE VIEW v_t2 AS SELECT id FROM items",
                "CREATE VIRTUAL TABLE fts_t2 USING fts5(body)",
                "DROP INDEX idx_items_title",
                "DROP TRIGGER trg_items",
                "DROP TABLE items",
                "ALTER TABLE items ADD COLUMN sneaky TEXT",
                "REINDEX items",
                "ANALYZE items",
            ] {
                assert!(conn.execute_batch(sql).is_err(), "layer 2 allowed {sql}");
            }
        }
        // The guard's drop uninstalls the hook, so the raw backend path works.
        conn.execute_batch("INSERT INTO items (id, title) VALUES ('item-1', 'Acta');")
            .unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn pragmas_are_denied_from_the_renderer() {
        let db = ddl_test_db();
        // Layer 1 rejects every PRAGMA in a batch; layer 2 denies the same
        // actions semantically when the text validator is bypassed.
        assert_eq!(
            execute_batch_on(&db, "PRAGMA defer_foreign_keys=ON").unwrap_err(),
            "Restricted SQL statement in db_execute_batch"
        );
        assert_eq!(
            execute_batch_on(&db, "PRAGMA quick_check").unwrap_err(),
            "Restricted SQL statement in db_execute_batch"
        );
        assert!(execute_on(&db, "PRAGMA defer_foreign_keys=ON", &[]).is_err());
        assert!(execute_on(&db, "PRAGMA quick_check", &[]).is_err());

        let conn = Connection::open_in_memory().unwrap();
        for sql in ["PRAGMA defer_foreign_keys=ON", "PRAGMA quick_check"] {
            assert_eq!(
                execute_batch_statement(&conn, sql).unwrap_err(),
                SQL_AUTHORIZER_DENIED_ERROR,
                "layer 2 allowed {sql}"
            );
        }
    }

    #[test]
    fn renderer_fts5_query_and_delete_all_still_work() {
        // The renderer's contentless FTS5 index (migration 0034) is rebuilt by
        // `INSERT INTO fts_items(fts_items) VALUES ('delete-all')` followed by
        // row inserts; the closed DDL policy must not break either path, nor
        // the value-less `PRAGMA data_version` FTS5 asks for on first use.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL);
             CREATE VIRTUAL TABLE fts_items USING fts5(
                 item_id UNINDEXED, title, content='', contentless_delete=1);",
        )
        .unwrap();
        let db = test_db(conn);

        // First FTS5 use on the guarded connection (the constructor pragma).
        let rows = select_on(&db, "SELECT COUNT(*) AS n FROM fts_items", &[]).unwrap();
        assert_eq!(rows.len(), 1);

        execute_on(
            &db,
            "INSERT INTO fts_items(fts_items) VALUES ('delete-all')",
            &[],
        )
        .unwrap();
        execute_on(
            &db,
            "INSERT INTO fts_items(rowid, item_id, title) \
             VALUES (1, 'item-1', 'Acta fundacional')",
            &[],
        )
        .unwrap();
        let hits = select_on(
            &db,
            "SELECT item_id FROM fts_items WHERE fts_items MATCH 'fundacional'",
            &[],
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn ordinary_dml_multi_statement_batches_and_rollback_still_work() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE notes (id TEXT PRIMARY KEY, content TEXT NOT NULL);")
            .unwrap();
        let db = test_db(conn);

        execute_on(
            &db,
            "INSERT INTO notes (id, content) VALUES ('n-1', 'hola')",
            &[],
        )
        .unwrap();
        execute_transaction_on(
            &db,
            &[ParameterizedStatement {
                sql: "INSERT INTO notes (id, content) VALUES (?, ?)".to_string(),
                params: vec![serde_json::json!("n-2"), serde_json::json!("tx")],
            }],
        )
        .unwrap();
        execute_batch_on(
            &db,
            "BEGIN;\n\
             UPDATE notes SET content = 'chau' WHERE id = 'n-1';\n\
             DELETE FROM notes WHERE id = 'n-2';\n\
             COMMIT;",
        )
        .unwrap();
        assert_eq!(
            execute_on(
                &db,
                "UPDATE notes SET content = 'final' WHERE id = 'n-1'",
                &[]
            )
            .unwrap()
            .rows_affected,
            1
        );
        // ROLLBACK goes through the same batch path and is allowed.
        execute_batch_on(&db, "BEGIN; DELETE FROM notes; ROLLBACK;").unwrap();
        assert_eq!(
            select_on(&db, "SELECT id FROM notes", &[]).unwrap().len(),
            1,
            "ROLLBACK must have undone the batch delete"
        );
    }
}
