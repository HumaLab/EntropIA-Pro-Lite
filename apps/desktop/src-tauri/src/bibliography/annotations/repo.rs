//! Persistence of annotation rows (`zotero_annotation_items`) and of the
//! per-library sync state (`zotero_annotation_sync_state`), migration 0059.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;

use super::machine::RowAction;
use super::model::{check_invariants, AnnotationRow, Sent, SyncState};
use super::wire::Data;

/// The library and Web account a row belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub library_row_id: String,
    pub web_user_id: String,
}

const COLUMNS: &str = "annotation_key, parent_key, base_version, base_json, remote_trashed, \
    local_json, local_deleted, edit_gen, sent_gen, sent_json, sent_uncertain, sync_state, \
    conflict_fields, conflict_json, last_error";

fn json_object(text: Option<String>) -> rusqlite::Result<Option<Data>> {
    match text {
        None => Ok(None),
        Some(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(object)) => Ok(Some(object)),
            _ => Err(rusqlite::Error::InvalidColumnType(
                0,
                "json object".into(),
                rusqlite::types::Type::Text,
            )),
        },
    }
}

fn to_text(data: &Option<Data>) -> Option<String> {
    data.as_ref()
        .map(|object| Value::Object(object.clone()).to_string())
}

fn read_row(row: &Row<'_>) -> rusqlite::Result<AnnotationRow> {
    let sent_text: Option<String> = row.get(9)?;
    let sent_json = match sent_text {
        Some(text) => Some(serde_json::from_str::<Sent>(&text).map_err(|_| {
            rusqlite::Error::InvalidColumnType(9, "sent_json".into(), rusqlite::types::Type::Text)
        })?),
        None => None,
    };
    let state: String = row.get(11)?;
    let conflict_fields: String = row.get(12)?;
    Ok(AnnotationRow {
        annotation_key: row.get(0)?,
        parent_key: row.get(1)?,
        base_version: row.get(2)?,
        base_json: json_object(row.get(3)?)?,
        remote_trashed: row.get::<_, i64>(4)? != 0,
        local_json: json_object(row.get(5)?)?,
        local_deleted: row.get::<_, i64>(6)? != 0,
        edit_gen: row.get(7)?,
        sent_gen: row.get(8)?,
        sent_json,
        sent_uncertain: row.get::<_, i64>(10)? != 0,
        sync_state: SyncState::parse(&state).ok_or_else(|| {
            rusqlite::Error::InvalidColumnType(11, "sync_state".into(), rusqlite::types::Type::Text)
        })?,
        conflict_fields: serde_json::from_str(&conflict_fields).unwrap_or_default(),
        conflict_json: json_object(row.get(13)?)?,
        last_error: row.get(14)?,
    })
}

pub fn load(conn: &Connection, scope: &Scope, key: &str) -> Result<Option<AnnotationRow>, String> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS} FROM zotero_annotation_items \
             WHERE library_row_id = ?1 AND web_user_id = ?2 AND annotation_key = ?3"
        ),
        params![scope.library_row_id, scope.web_user_id, key],
        read_row,
    )
    .optional()
    .map_err(|error| format!("annotation read failed: {error}"))
}

fn select(
    conn: &Connection,
    scope: &Scope,
    filter: &str,
    extra: &[&dyn rusqlite::ToSql],
) -> Result<Vec<AnnotationRow>, String> {
    let sql = format!(
        "SELECT {COLUMNS} FROM zotero_annotation_items \
         WHERE library_row_id = ?1 AND web_user_id = ?2 {filter} ORDER BY annotation_key"
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|error| format!("annotation query failed: {error}"))?;
    let mut values: Vec<&dyn rusqlite::ToSql> = vec![&scope.library_row_id, &scope.web_user_id];
    values.extend_from_slice(extra);
    let rows = statement
        .query_map(values.as_slice(), read_row)
        .map_err(|error| format!("annotation query failed: {error}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("annotation read failed: {error}"))
}

/// Every row of one parent attachment.
pub fn by_parent(
    conn: &Connection,
    scope: &Scope,
    parent_key: &str,
) -> Result<Vec<AnnotationRow>, String> {
    select(conn, scope, "AND parent_key = ?3", &[&parent_key])
}

/// Rows with a request recorded as sent.
pub fn outstanding(conn: &Connection, scope: &Scope) -> Result<Vec<AnnotationRow>, String> {
    select(conn, scope, "AND sent_gen IS NOT NULL", &[])
}

pub fn in_state(
    conn: &Connection,
    scope: &Scope,
    state: SyncState,
) -> Result<Vec<AnnotationRow>, String> {
    select(conn, scope, "AND sync_state = ?3", &[&state.as_str()])
}

/// Writes a row after checking the section-3 invariants.
pub fn save(
    conn: &Connection,
    scope: &Scope,
    row: &AnnotationRow,
    now_ms: i64,
) -> Result<(), String> {
    check_invariants(row).map_err(|message| format!("annotation invariant broken: {message}"))?;
    let sent = row
        .sent_json
        .as_ref()
        .map(|sent| serde_json::to_string(sent).unwrap_or_default());
    conn.execute(
        "INSERT INTO zotero_annotation_items (library_row_id, web_user_id, annotation_key, parent_key, \
            base_version, base_json, remote_trashed, local_json, local_deleted, edit_gen, sent_gen, \
            sent_json, sent_uncertain, sync_state, conflict_fields, conflict_json, last_error, \
            created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?18) \
         ON CONFLICT (library_row_id, annotation_key) DO UPDATE SET \
            web_user_id = excluded.web_user_id, parent_key = excluded.parent_key, \
            base_version = excluded.base_version, base_json = excluded.base_json, \
            remote_trashed = excluded.remote_trashed, local_json = excluded.local_json, \
            local_deleted = excluded.local_deleted, edit_gen = excluded.edit_gen, \
            sent_gen = excluded.sent_gen, sent_json = excluded.sent_json, \
            sent_uncertain = excluded.sent_uncertain, sync_state = excluded.sync_state, \
            conflict_fields = excluded.conflict_fields, conflict_json = excluded.conflict_json, \
            last_error = excluded.last_error, updated_at = excluded.updated_at",
        params![
            scope.library_row_id,
            scope.web_user_id,
            row.annotation_key,
            row.parent_key,
            row.base_version,
            to_text(&row.base_json),
            row.remote_trashed as i64,
            to_text(&row.local_json),
            row.local_deleted as i64,
            row.edit_gen,
            row.sent_gen,
            sent,
            row.sent_uncertain as i64,
            row.sync_state.as_str(),
            serde_json::to_string(&row.conflict_fields).unwrap_or_else(|_| "[]".into()),
            to_text(&row.conflict_json),
            row.last_error,
            now_ms,
        ],
    )
    .map_err(|error| format!("annotation write failed: {error}"))?;
    Ok(())
}

pub fn remove(conn: &Connection, scope: &Scope, key: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM zotero_annotation_items WHERE library_row_id = ?1 AND annotation_key = ?2",
        params![scope.library_row_id, key],
    )
    .map_err(|error| format!("annotation delete failed: {error}"))?;
    Ok(())
}

/// Carries out what a transition asked for.
pub fn apply(
    conn: &Connection,
    scope: &Scope,
    key_before: &str,
    row: &AnnotationRow,
    action: &RowAction,
    now_ms: i64,
) -> Result<(), String> {
    match action {
        RowAction::Save => save(conn, scope, row, now_ms),
        RowAction::Remove => remove(conn, scope, key_before),
        RowAction::Rekeyed { old_key } => {
            remove(conn, scope, old_key)?;
            save(conn, scope, row, now_ms)
        }
    }
}

/// The pull cursor and pause of one library for one Web account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibrarySyncState {
    pub web_library_version: i64,
    pub paused_until: Option<i64>,
    pub pause_reason: Option<String>,
}

pub fn sync_state(conn: &Connection, scope: &Scope) -> Result<LibrarySyncState, String> {
    conn.query_row(
        "SELECT web_library_version, paused_until, pause_reason FROM zotero_annotation_sync_state \
         WHERE library_row_id = ?1 AND web_user_id = ?2",
        params![scope.library_row_id, scope.web_user_id],
        |row| {
            Ok(LibrarySyncState {
                web_library_version: row.get(0)?,
                paused_until: row.get(1)?,
                pause_reason: row.get(2)?,
            })
        },
    )
    .optional()
    .map(Option::unwrap_or_default)
    .map_err(|error| format!("annotation sync state read failed: {error}"))
}

pub fn set_sync_state(
    conn: &Connection,
    scope: &Scope,
    state: &LibrarySyncState,
    now_ms: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO zotero_annotation_sync_state \
            (library_row_id, web_user_id, web_library_version, paused_until, pause_reason, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT (library_row_id, web_user_id) DO UPDATE SET \
            web_library_version = excluded.web_library_version, paused_until = excluded.paused_until, \
            pause_reason = excluded.pause_reason, updated_at = excluded.updated_at",
        params![
            scope.library_row_id,
            scope.web_user_id,
            state.web_library_version,
            state.paused_until,
            state.pause_reason,
            now_ms
        ],
    )
    .map_err(|error| format!("annotation sync state write failed: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bibliography::annotations::machine::{begin_send, local_edit, next_request};
    use serde_json::json;

    fn scope() -> Scope {
        Scope {
            library_row_id: "lib1".into(),
            web_user_id: "4242".into(),
        }
    }

    fn data(value: serde_json::Value) -> Data {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn a_row_round_trips_with_its_sent_request() {
        let conn =
            crate::bibliography::annotations::repo_test_support::archive_with_library("lib1");
        let mut row = AnnotationRow::from_server(
            "ABCD2345",
            5,
            data(
                json!({"key": "ABCD2345", "version": 5, "parentItem": "PARENT22", "annotationComment": "A"}),
            ),
        );
        let mut desired = row.base_json.clone().unwrap();
        desired.insert("annotationComment".into(), json!("B"));
        local_edit(&mut row, desired);
        let request = next_request(&mut row).unwrap();
        begin_send(&mut row, &request);
        save(&conn, &scope(), &row, 1).unwrap();
        assert_eq!(
            load(&conn, &scope(), "ABCD2345").unwrap(),
            Some(row.clone())
        );
        assert_eq!(
            by_parent(&conn, &scope(), "PARENT22").unwrap(),
            vec![row.clone()]
        );
        assert_eq!(outstanding(&conn, &scope()).unwrap().len(), 1);
        remove(&conn, &scope(), "ABCD2345").unwrap();
        assert_eq!(load(&conn, &scope(), "ABCD2345").unwrap(), None);
    }

    #[test]
    fn a_row_breaking_an_invariant_is_never_written() {
        let conn =
            crate::bibliography::annotations::repo_test_support::archive_with_library("lib1");
        let mut row = AnnotationRow::from_server("ABCD2345", 5, data(json!({"parentItem": "P"})));
        row.sync_state = SyncState::Pending;
        assert!(save(&conn, &scope(), &row, 1).is_err());
        assert_eq!(load(&conn, &scope(), "ABCD2345").unwrap(), None);
    }

    #[test]
    fn the_sync_state_defaults_and_round_trips() {
        let conn =
            crate::bibliography::annotations::repo_test_support::archive_with_library("lib1");
        assert_eq!(
            sync_state(&conn, &scope()).unwrap(),
            LibrarySyncState::default()
        );
        let state = LibrarySyncState {
            web_library_version: 42,
            paused_until: Some(9),
            pause_reason: Some("backoff".into()),
        };
        set_sync_state(&conn, &scope(), &state, 1).unwrap();
        assert_eq!(sync_state(&conn, &scope()).unwrap(), state);
    }
}
