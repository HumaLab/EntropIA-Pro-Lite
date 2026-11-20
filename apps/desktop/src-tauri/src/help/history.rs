//! Conversation history of the in-app help assistant (S19, D17).
//!
//! Its own SQLite file `help-history.sqlite` in the app DATA dir —
//! profile-aware like the archive, never the cache dir (the OS may clear
//! caches) and never the user's archive (no migration, no sync now).
//! Sync-ready shape: stable UUID ids, `created_at` / `updated_at` in UTC
//! ms, soft delete `deleted_at`, one row per conversation and one per
//! message (role, text, sources/steps JSON). The full content is stored,
//! including answers built from the user's documents (L47).

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// The history file inside the app data dir (D17).
pub(crate) const HELP_HISTORY_FILENAME: &str = "help-history.sqlite";
/// The only schema version this build reads and writes.
pub(crate) const HELP_HISTORY_SCHEMA_VERSION: i64 = 1;
/// The most conversations `help_history_list` returns, newest first.
pub(crate) const HELP_HISTORY_LIST_LIMIT: i64 = 200;
/// A conversation title is the first question, trimmed to about this many
/// characters (D17).
pub(crate) const HELP_HISTORY_TITLE_CHARS: usize = 60;

/// UTC milliseconds of now, for `created_at` / `updated_at`.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// The title of a new conversation: the first question, trimmed to about
/// [`HELP_HISTORY_TITLE_CHARS`] characters. Never blank: an empty question
/// (the command layer already rejects those) falls back to a generic.
pub(crate) fn make_title(question: &str) -> String {
    let trimmed = question.trim();
    if trimmed.is_empty() {
        return "Conversación".to_string();
    }
    trimmed.chars().take(HELP_HISTORY_TITLE_CHARS).collect()
}

/// A fresh conversation id: a stable UUID (D17), hyphenated like the
/// frontend's `crypto.randomUUID()`.
pub(crate) fn new_conversation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// One row of `help_history_list`: id, title, last write and how many live
/// messages it holds. Not deleted, newest first, capped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistorySummary {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    pub message_count: i64,
}

/// One stored turn of `help_history_get`: the role, the text and the
/// answer metadata (sources, steps, documents, local flag) as JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryMessage {
    pub role: String,
    pub content: String,
    pub meta: serde_json::Value,
}

/// One conversation of `help_history_get`: id, title and every live
/// message in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryDetail {
    pub id: String,
    pub title: String,
    pub messages: Vec<HistoryMessage>,
}

/// The history store over one SQLite file. One connection per handle:
/// every command opens, uses and drops it, so no shared state crosses
/// Tauri.
pub(crate) struct HelpHistory {
    conn: Connection,
}

impl HelpHistory {
    /// Opens (or creates) the history file, creating parent directories
    /// and bringing the schema up to the only version this build knows.
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("No se pudo crear el historial de ayuda: {error}"))?;
            }
        }
        let conn = Connection::open(path)
            .map_err(|error| format!("No se pudo abrir el historial de ayuda: {error}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta(
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS conversations(
               id TEXT PRIMARY KEY,
               title TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               deleted_at INTEGER NULL
             );
             CREATE TABLE IF NOT EXISTS messages(
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               seq INTEGER NOT NULL,
               role TEXT NOT NULL,
               content TEXT NOT NULL,
               meta_json TEXT NOT NULL DEFAULT '{}',
               created_at INTEGER NOT NULL,
               deleted_at INTEGER NULL
             );
             CREATE INDEX IF NOT EXISTS idx_messages_conversation
               ON messages(conversation_id, seq);",
        )
        .map_err(|error| format!("No se pudo crear el historial de ayuda: {error}"))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![HELP_HISTORY_SCHEMA_VERSION.to_string()],
        )
        .map_err(|error| format!("No se pudo crear el historial de ayuda: {error}"))?;
        let stored: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))?;
        if stored != HELP_HISTORY_SCHEMA_VERSION.to_string() {
            return Err(format!(
                "Historial de ayuda con versión no soportada: {stored}."
            ));
        }
        Ok(Self { conn })
    }

    /// The stored schema version, for the meta-row test.
    #[cfg(test)]
    pub(crate) fn schema_version(&self) -> Result<i64, String> {
        let stored: String = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))?;
        stored
            .parse::<i64>()
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))
    }

    /// Whether the conversation exists and is not deleted.
    fn is_live(&self, id: &str) -> Result<bool, String> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM conversations WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |row| row.get(0),
            )
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))?;
        Ok(count > 0)
    }

    /// Creates the conversation when missing (with `title`) and answers
    /// whether it was created. Never touches an existing row: the title
    /// stays the first question, and a soft-deleted id stays deleted.
    pub(crate) fn ensure_conversation(
        &self,
        id: &str,
        title: &str,
        now_ms: i64,
    ) -> Result<bool, String> {
        let changed = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO conversations(id, title, created_at, updated_at, deleted_at)
                 VALUES (?1, ?2, ?3, ?3, NULL)",
                params![id, title, now_ms],
            )
            .map_err(|error| format!("No se pudo guardar en el historial de ayuda: {error}"))?;
        Ok(changed == 1)
    }

    /// The next message sequence of a conversation.
    fn next_seq(&self, conversation_id: &str) -> Result<i64, String> {
        let seq: Option<i64> = self
            .conn
            .query_row(
                "SELECT MAX(seq) FROM messages WHERE conversation_id = ?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))?;
        Ok(seq.unwrap_or(0) + 1)
    }

    /// Appends one message and bumps the conversation's `updated_at`.
    /// The conversation must exist and not be deleted; the role is
    /// `user` or `assistant`. `meta_json` travels verbatim.
    pub(crate) fn append_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        meta_json: &str,
        now_ms: i64,
    ) -> Result<(), String> {
        if role != "user" && role != "assistant" {
            return Err(format!("Rol del historial desconocido: {role}"));
        }
        if !self.is_live(conversation_id)? {
            return Err("La conversación no existe.".to_string());
        }
        let seq = self.next_seq(conversation_id)?;
        self.conn
            .execute(
                "INSERT INTO messages(id, conversation_id, seq, role, content, meta_json, created_at, deleted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    conversation_id,
                    seq,
                    role,
                    content,
                    meta_json,
                    now_ms
                ],
            )
            .map_err(|error| format!("No se pudo guardar en el historial de ayuda: {error}"))?;
        self.conn
            .execute(
                "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                params![now_ms, conversation_id],
            )
            .map_err(|error| format!("No se pudo guardar en el historial de ayuda: {error}"))?;
        Ok(())
    }

    /// Appends the user question and the final answer to `conversation_id`,
    /// creating the conversation (titled with the question) when missing.
    /// `answer_meta_json` is stored on the assistant message. Answers the
    /// conversation id, so the caller echoes it back on the wire.
    pub(crate) fn append_turn(
        &self,
        conversation_id: Option<&str>,
        question: &str,
        answer: &str,
        answer_meta_json: &str,
        now_ms: i64,
    ) -> Result<String, String> {
        let id = conversation_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .unwrap_or_else(new_conversation_id);
        self.ensure_conversation(&id, &make_title(question), now_ms)?;
        self.append_message(&id, "user", question, "{}", now_ms)?;
        self.append_message(&id, "assistant", answer, answer_meta_json, now_ms)?;
        Ok(id)
    }

    /// Every live conversation, newest first, capped at
    /// [`HELP_HISTORY_LIST_LIMIT`].
    pub(crate) fn list(&self) -> Result<Vec<HistorySummary>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.id, c.title, c.updated_at,
                   (SELECT COUNT(*) FROM messages m
                     WHERE m.conversation_id = c.id AND m.deleted_at IS NULL)
                 FROM conversations c
                 WHERE c.deleted_at IS NULL
                 ORDER BY c.updated_at DESC, c.created_at DESC
                 LIMIT ?1",
            )
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))?;
        let rows = stmt
            .query_map(params![HELP_HISTORY_LIST_LIMIT], |row| {
                Ok(HistorySummary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated_at: row.get(2)?,
                    message_count: row.get(3)?,
                })
            })
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))
    }

    /// One live conversation with every live message in order, or `None`
    /// when missing or deleted.
    pub(crate) fn get(&self, id: &str) -> Result<Option<HistoryDetail>, String> {
        let head: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT id, title FROM conversations WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))?;
        let Some((id, title)) = head else {
            return Ok(None);
        };
        let mut stmt = self
            .conn
            .prepare(
                "SELECT role, content, meta_json FROM messages
                 WHERE conversation_id = ?1 AND deleted_at IS NULL
                 ORDER BY seq ASC",
            )
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))?;
        let rows = stmt
            .query_map(params![id], |row| {
                let meta_json: String = row.get(2)?;
                Ok(HistoryMessage {
                    role: row.get(0)?,
                    content: row.get(1)?,
                    meta: serde_json::from_str(&meta_json)
                        .unwrap_or(serde_json::Value::Object(Default::default())),
                })
            })
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))?;
        let messages = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("No se pudo leer el historial de ayuda: {error}"))?;
        Ok(Some(HistoryDetail {
            id,
            title,
            messages,
        }))
    }

    /// Soft-deletes one conversation and all its messages. Missing ids
    /// are a no-op.
    pub(crate) fn delete(&self, id: &str, now_ms: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE conversations SET deleted_at = ?1, updated_at = ?1
                 WHERE id = ?2 AND deleted_at IS NULL",
                params![now_ms, id],
            )
            .map_err(|error| format!("No se pudo borrar el historial de ayuda: {error}"))?;
        self.conn
            .execute(
                "UPDATE messages SET deleted_at = ?1
                 WHERE conversation_id = ?2 AND deleted_at IS NULL",
                params![now_ms, id],
            )
            .map_err(|error| format!("No se pudo borrar el historial de ayuda: {error}"))?;
        Ok(())
    }

    /// Soft-deletes every conversation and every message.
    pub(crate) fn clear(&self, now_ms: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE conversations SET deleted_at = ?1, updated_at = ?1
                 WHERE deleted_at IS NULL",
                params![now_ms],
            )
            .map_err(|error| format!("No se pudo borrar el historial de ayuda: {error}"))?;
        self.conn
            .execute(
                "UPDATE messages SET deleted_at = ?1 WHERE deleted_at IS NULL",
                params![now_ms],
            )
            .map_err(|error| format!("No se pudo borrar el historial de ayuda: {error}"))?;
        Ok(())
    }
}

/// Opens the history store in the app DATA dir (D17): profile-aware
/// like the archive through [`crate::path_utils::data_dir`], never the
/// cache dir and never the user's archive.
pub(crate) fn history_store(app: &tauri::AppHandle) -> Result<HelpHistory, String> {
    let dir = crate::path_utils::data_dir(app)?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("No se pudo crear el historial de ayuda: {error}"))?;
    HelpHistory::open(&dir.join(HELP_HISTORY_FILENAME))
}

/// The JSON stored on an assistant message: everything the panel shows
/// under an answer, so reopening it renders the same (S19, D17). Pure,
/// so the shape is pinned by a test.
pub(crate) fn answer_meta_json(answer: &super::prompt::HelpAnswer) -> String {
    serde_json::json!({
        "kind": "answer",
        "sources": answer.sources,
        "documents": answer.documents,
        "steps": answer.steps,
        "local": answer.local,
        "roundLimitReached": answer.round_limit_reached,
        "toolsUnavailable": answer.tools_unavailable,
        "localModel": answer.local_model,
    })
    .to_string()
}

/// The meta JSON stored on a decided-approval notice (T26, T27): plain
/// assistant content with its kind, so a reopened conversation still
/// shows what happened.
const NOTICE_META_JSON: &str = r#"{"kind":"notice"}"#;

/// Persists one turn when the frontend sent a conversation id (S19): the
/// question and the final answer, creating the conversation (titled with
/// the first question) when missing, and echoes the id on the answer.
/// A failed write never fails the answer: it is logged and the answer
/// travels without an id, so the next question retries the creation.
pub(crate) fn attach_history(
    app: &tauri::AppHandle,
    conversation_id: Option<String>,
    question: &str,
    mut answer: super::prompt::HelpAnswer,
) -> super::prompt::HelpAnswer {
    let Some(id) = conversation_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
    else {
        return answer;
    };
    let meta = answer_meta_json(&answer);
    match history_store(app)
        .and_then(|store| store.append_turn(Some(&id), question, &answer.answer, &meta, now_ms()))
    {
        Ok(stored) => {
            answer.conversation_id = Some(stored);
            answer
        }
        Err(error) => {
            log::warn!(target: "help", "history append failed: {error}");
            answer
        }
    }
}

/// Appends a decided-approval notice to its stored conversation (T26):
/// the background outcome watcher calls this next to emitting the
/// frontend event, so the message survives a reopen. Missing or deleted
/// conversations are an error the caller logs.
pub(crate) fn append_notice(
    app: &tauri::AppHandle,
    conversation_id: &str,
    message: &str,
) -> Result<(), String> {
    let store = history_store(app)?;
    store.append_message(
        conversation_id,
        "assistant",
        message,
        NOTICE_META_JSON,
        now_ms(),
    )
}

/// Every stored conversation, newest first, capped (S19): id, title,
/// last write and live message count. Nothing deleted travels.
#[tauri::command]
pub async fn help_history_list(app: tauri::AppHandle) -> Result<Vec<HistorySummary>, String> {
    tokio::task::spawn_blocking(move || history_store(&app)?.list())
        .await
        .map_err(|error| format!("help history task panicked: {error}"))?
}

/// One stored conversation with every live message in order (S19):
/// clicking it in «Historial» loads this into the panel.
#[tauri::command]
pub async fn help_history_get(app: tauri::AppHandle, id: String) -> Result<HistoryDetail, String> {
    tokio::task::spawn_blocking(move || {
        history_store(&app)?
            .get(id.trim())?
            .ok_or_else(|| "La conversación no existe.".to_string())
    })
    .await
    .map_err(|error| format!("help history task panicked: {error}"))?
}

/// Soft-deletes one stored conversation and all its messages (S19).
/// Missing ids are a no-op.
#[tauri::command]
pub async fn help_history_delete(app: tauri::AppHandle, id: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || history_store(&app)?.delete(id.trim(), now_ms()))
        .await
        .map_err(|error| format!("help history task panicked: {error}"))?
}

/// Soft-deletes every stored conversation and message (S19).
#[tauri::command]
pub async fn help_history_clear(app: tauri::AppHandle) -> Result<(), String> {
    tokio::task::spawn_blocking(move || history_store(&app)?.clear(now_ms()))
        .await
        .map_err(|error| format!("help history task panicked: {error}"))?
}

/// Appends one line to a stored conversation (S19): the inline-approval
/// decisions (T27, «Aprobaste: …» / «Rechazaste: …») the frontend adds
/// while the run is still in flight. The conversation must exist; the
/// role is `user` or `assistant`.
#[tauri::command]
pub async fn help_history_append(
    app: tauri::AppHandle,
    conversation_id: String,
    role: String,
    content: String,
) -> Result<(), String> {
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err("El mensaje está vacío.".to_string());
    }
    tokio::task::spawn_blocking(move || {
        history_store(&app)?.append_message(&conversation_id, &role, &content, "{}", now_ms())
    })
    .await
    .map_err(|error| format!("help history task panicked: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp() -> (tempfile::TempDir, HelpHistory) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = HelpHistory::open(&dir.path().join(HELP_HISTORY_FILENAME)).expect("open");
        (dir, store)
    }

    #[test]
    fn open_creates_the_file_and_the_schema_version_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(HELP_HISTORY_FILENAME);
        assert!(!path.exists(), "nothing before open");
        let store = HelpHistory::open(&path).expect("open");
        assert!(path.exists(), "the file lives where it was opened");
        assert_eq!(
            store.schema_version().expect("version"),
            HELP_HISTORY_SCHEMA_VERSION
        );
    }

    #[test]
    fn a_first_turn_creates_the_conversation_titled_with_the_question() {
        let (_dir, store) = open_temp();
        let id = store
            .append_turn(
                None,
                "¿Cómo importo un PDF?",
                "Arrastrá el PDF a la colección.",
                "{}",
                1_700_000_000_000,
            )
            .expect("append");
        assert!(!id.is_empty(), "an id is answered");
        let detail = store.get(&id).expect("get").expect("created");
        assert_eq!(detail.title, "¿Cómo importo un PDF?");
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(detail.messages[0].role, "user");
        assert_eq!(detail.messages[0].content, "¿Cómo importo un PDF?");
        assert_eq!(detail.messages[1].role, "assistant");
        assert_eq!(
            detail.messages[1].content,
            "Arrastrá el PDF a la colección."
        );
    }

    #[test]
    fn the_title_is_the_first_question_trimmed_to_about_sixty_chars() {
        let (_dir, store) = open_temp();
        let long = "¿Cómo hago para importar un documento PDF muy largo a mi colección nueva?";
        assert!(long.chars().count() > HELP_HISTORY_TITLE_CHARS);
        let id = store
            .append_turn(None, long, "así.", "{}", 1_700_000_000_000)
            .expect("append");
        let detail = store.get(&id).expect("get").expect("created");
        assert!(detail.title.chars().count() <= HELP_HISTORY_TITLE_CHARS);
        assert!(long.starts_with(&detail.title));
        // A second turn keeps the first question as the title.
        store
            .append_turn(
                Some(&id),
                "¿Y después?",
                "después así.",
                "{}",
                1_700_000_000_001,
            )
            .expect("second turn");
        let detail = store.get(&id).expect("get").expect("still there");
        assert!(long.starts_with(&detail.title));
        assert_eq!(detail.messages.len(), 4);
    }

    #[test]
    fn list_is_newest_first_capped_and_counts_live_messages() {
        let (_dir, store) = open_temp();
        let first = store
            .append_turn(None, "primera", "r1", "{}", 1_700_000_000_000)
            .expect("first");
        let second = store
            .append_turn(None, "segunda", "r2", "{}", 1_700_000_000_001)
            .expect("second");
        // A newer turn on the first conversation brings it back on top.
        store
            .append_turn(
                Some(&first),
                "primera otra vez",
                "r3",
                "{}",
                1_700_000_000_002,
            )
            .expect("third");
        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, first);
        assert_eq!(listed[0].message_count, 4);
        assert_eq!(listed[1].id, second);
        assert_eq!(listed[1].message_count, 2);
        assert_eq!(listed[0].title, "primera");
    }

    #[test]
    fn get_returns_messages_in_order_with_their_meta() {
        let (_dir, store) = open_temp();
        let meta = serde_json::json!({"local": true, "sources": []}).to_string();
        let id = store
            .append_turn(None, "pregunta", "respuesta", &meta, 1_700_000_000_000)
            .expect("turn");
        let detail = store.get(&id).expect("get").expect("created");
        assert_eq!(detail.id, id);
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(
            detail.messages[1].meta,
            serde_json::json!({"local": true, "sources": []})
        );
    }

    #[test]
    fn delete_hides_the_conversation_but_keeps_its_rows_soft_deleted() {
        let (_dir, store) = open_temp();
        let id = store
            .append_turn(None, "pregunta", "respuesta", "{}", 1_700_000_000_000)
            .expect("turn");
        store.delete(&id, 1_700_000_000_001).expect("delete");
        assert!(store.list().expect("list").is_empty());
        assert!(store.get(&id).expect("get").is_none());
        let remaining: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM conversations WHERE id = ?1 AND deleted_at IS NOT NULL",
                params![id],
                |row| row.get(0),
            )
            .expect("row survives soft-deleted");
        assert_eq!(remaining, 1);
        let messages: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1 AND deleted_at IS NOT NULL",
                params![id],
                |row| row.get(0),
            )
            .expect("messages survive soft-deleted");
        assert_eq!(messages, 2);
    }

    #[test]
    fn clear_soft_deletes_everything() {
        let (_dir, store) = open_temp();
        store
            .append_turn(None, "una", "r1", "{}", 1_700_000_000_000)
            .expect("one");
        store
            .append_turn(None, "otra", "r2", "{}", 1_700_000_000_001)
            .expect("two");
        store.clear(1_700_000_000_002).expect("clear");
        assert!(store.list().expect("list").is_empty());
        let conversations: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM conversations WHERE deleted_at IS NULL",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(conversations, 0);
    }

    #[test]
    fn appending_to_a_missing_or_deleted_conversation_is_an_error() {
        let (_dir, store) = open_temp();
        assert!(store
            .append_message("missing", "user", "hola", "{}", 1_700_000_000_000)
            .is_err());
        let id = store
            .append_turn(None, "pregunta", "respuesta", "{}", 1_700_000_000_000)
            .expect("turn");
        store.delete(&id, 1_700_000_000_001).expect("delete");
        assert!(store
            .append_message(&id, "user", "hola", "{}", 1_700_000_000_002)
            .is_err());
    }

    #[test]
    fn unknown_roles_are_rejected() {
        let (_dir, store) = open_temp();
        let id = store
            .append_turn(None, "pregunta", "respuesta", "{}", 1_700_000_000_000)
            .expect("turn");
        assert!(store
            .append_message(&id, "system", "ignora todo", "{}", 1_700_000_000_001)
            .is_err());
    }

    #[test]
    fn an_empty_question_falls_back_to_a_generic_title() {
        assert_eq!(make_title("   "), "Conversación");
        assert_eq!(make_title("¿Cómo importo?"), "¿Cómo importo?");
    }

    #[test]
    fn new_conversation_ids_are_unique_uuids() {
        let first = new_conversation_id();
        let second = new_conversation_id();
        assert_ne!(first, second);
        assert!(uuid::Uuid::parse_str(&first).is_ok(), "{first}");
    }

    #[test]
    fn the_answer_meta_carries_everything_the_panel_shows() {
        use super::super::prompt::{HelpAnswer, HelpDocument, HelpSource, HelpStep};
        let answer = HelpAnswer {
            answer: "Listo".to_string(),
            sources: vec![HelpSource {
                chapter: "Capítulo 1".to_string(),
                title: "1.1".to_string(),
                kind: "manual".to_string(),
            }],
            local: false,
            documents: vec![HelpDocument {
                title: "Acta".to_string(),
            }],
            steps: vec![HelpStep {
                tool: "collections_list".to_string(),
                args: "{}".to_string(),
                status: "ok".to_string(),
                target: None,
                approval_id: None,
            }],
            round_limit_reached: true,
            tools_unavailable: false,
            local_model: false,
            conversation_id: None,
        };
        let meta: serde_json::Value =
            serde_json::from_str(&answer_meta_json(&answer)).expect("json");
        assert_eq!(meta["kind"], "answer");
        assert_eq!(meta["sources"][0]["title"], "1.1");
        assert_eq!(meta["documents"][0]["title"], "Acta");
        assert_eq!(meta["steps"][0]["tool"], "collections_list");
        assert_eq!(meta["local"], false);
        assert_eq!(meta["roundLimitReached"], true);
        assert_eq!(meta["toolsUnavailable"], false);
    }
}
