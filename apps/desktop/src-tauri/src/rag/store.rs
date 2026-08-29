//! Persistencia SQLite de las conversaciones del chat RAG.
//!
//! Helpers puros sobre `&Connection`/`&mut Connection` para que los comandos
//! Tauri los envuelvan en `spawn_blocking` y los tests los ejerciten contra
//! una base en memoria. Las tablas (`rag_conversations`, `rag_messages`) las
//! crea la migración del frontend al iniciar la app.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension};

use super::{RagChatTurn, RagConversation, RagConversationSummary, RagMessage, RagSource};

/// Máximo de caracteres del título derivado de la primera pregunta.
const TITLE_MAX_CHARS: usize = 60;

/// Timestamp actual en milisegundos Unix (idioma del resto del codebase).
pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Título de conversación: la pregunta colapsada a una sola línea, primeros
/// 60 chars (conteo por chars, no bytes) y '…' al final si hubo truncado.
pub(crate) fn conversation_title(question: &str) -> String {
    let single_line = question.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title: String = single_line.chars().take(TITLE_MAX_CHARS).collect();
    if single_line.chars().count() > TITLE_MAX_CHARS {
        title.push('…');
    }
    title
}

/// Últimos `max_turns` mensajes de una conversación, en orden cronológico,
/// como turnos listos para `format_history`. Conversación inexistente o sin
/// mensajes → vector vacío. Solo cuentan los turnos con contenido: las
/// respuestas vacías de los intercambios "sin resultados" no deben desalojar
/// historial real de la ventana (el filtro de `format_history` queda como
/// defensa adicional).
pub(crate) fn load_history(
    conn: &Connection,
    conversation_id: &str,
    max_turns: usize,
) -> Result<Vec<RagChatTurn>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT role, content
             FROM rag_messages
             WHERE conversation_id = ?1
               AND TRIM(content) <> ''
             ORDER BY sort_index DESC
             LIMIT ?2",
        )
        .map_err(|e| format!("Failed to prepare RAG history query: {e}"))?;

    let mut turns = stmt
        .query_map(
            rusqlite::params![conversation_id, max_turns as i64],
            |row| {
                Ok(RagChatTurn {
                    role: row.get(0)?,
                    content: row.get(1)?,
                })
            },
        )
        .map_err(|e| format!("Failed to run RAG history query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read RAG history rows: {e}"))?;

    // La query trae los más recientes primero; el historial va cronológico.
    turns.reverse();
    Ok(turns)
}

/// Persiste un intercambio pregunta/respuesta en UNA transacción atómica.
///
/// Si `conversation_id` es `None` o no existe (borrada en vuelo), crea una
/// conversación NUEVA con id fresco — nunca resucita el id viejo. Inserta el
/// mensaje del usuario y el del asistente con `sort_index` consecutivos
/// (`COALESCE(MAX(sort_index),-1)+1`) y bumpea `updated_at`. Devuelve el id
/// real de la conversación persistida.
pub(crate) fn persist_exchange(
    conn: &mut Connection,
    conversation_id: Option<&str>,
    question: &str,
    answer: &str,
    sources: &[RagSource],
    model: &str,
    now: i64,
) -> Result<String, String> {
    let sources_json = serde_json::to_string(sources)
        .map_err(|e| format!("Failed to serialize RAG sources: {e}"))?;

    let tx = conn
        .transaction()
        .map_err(|e| format!("Failed to start RAG persistence transaction: {e}"))?;

    let existing = match conversation_id {
        Some(id) => tx
            .query_row(
                "SELECT id FROM rag_conversations WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("Failed to check RAG conversation '{id}': {e}"))?,
        None => None,
    };

    let conversation_id = match existing {
        Some(id) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO rag_conversations(id, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?3)",
                rusqlite::params![id, conversation_title(question), now],
            )
            .map_err(|e| format!("Failed to create RAG conversation: {e}"))?;
            id
        }
    };

    let base: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(sort_index), -1) + 1
             FROM rag_messages WHERE conversation_id = ?1",
            rusqlite::params![conversation_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to compute RAG message sort_index: {e}"))?;

    tx.execute(
        "INSERT INTO rag_messages(id, conversation_id, sort_index, role, content, sources, model, created_at)
         VALUES (?1, ?2, ?3, 'user', ?4, NULL, NULL, ?5)",
        rusqlite::params![
            uuid::Uuid::new_v4().to_string(),
            conversation_id,
            base,
            question,
            now
        ],
    )
    .map_err(|e| format!("Failed to insert RAG user message: {e}"))?;

    tx.execute(
        "INSERT INTO rag_messages(id, conversation_id, sort_index, role, content, sources, model, created_at)
         VALUES (?1, ?2, ?3, 'assistant', ?4, ?5, ?6, ?7)",
        rusqlite::params![
            uuid::Uuid::new_v4().to_string(),
            conversation_id,
            base + 1,
            answer,
            sources_json,
            model,
            now
        ],
    )
    .map_err(|e| format!("Failed to insert RAG assistant message: {e}"))?;

    tx.execute(
        "UPDATE rag_conversations SET updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now, conversation_id],
    )
    .map_err(|e| format!("Failed to bump RAG conversation updated_at: {e}"))?;

    tx.commit()
        .map_err(|e| format!("Failed to commit RAG persistence transaction: {e}"))?;

    Ok(conversation_id)
}

/// Lista los resúmenes de conversación, más reciente primero.
pub(crate) fn list_conversations(conn: &Connection) -> Result<Vec<RagConversationSummary>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.title, c.created_at, c.updated_at,
                    (SELECT COUNT(*) FROM rag_messages m WHERE m.conversation_id = c.id)
             FROM rag_conversations c
             ORDER BY c.updated_at DESC",
        )
        .map_err(|e| format!("Failed to prepare RAG conversations query: {e}"))?;

    let conversations = stmt
        .query_map([], |row| {
            Ok(RagConversationSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
                message_count: row.get(4)?,
            })
        })
        .map_err(|e| format!("Failed to run RAG conversations query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read RAG conversation rows: {e}"))?;

    Ok(conversations)
}

/// Literal, case-insensitive search across titles and all message content.
/// Results preserve the order returned by `list_conversations`.
pub(crate) fn search_conversations(
    conn: &Connection,
    query: &str,
) -> Result<Vec<RagConversationSummary>, String> {
    let needle = query.trim().to_lowercase();
    let conversations = list_conversations(conn)?;
    if needle.is_empty() {
        return Ok(conversations);
    }

    let mut stmt = conn
        .prepare("SELECT conversation_id, content FROM rag_messages")
        .map_err(|e| format!("Failed to prepare RAG conversation search query: {e}"))?;
    let mut message_matches = HashSet::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("Failed to run RAG conversation search query: {e}"))?;

    for row in rows {
        let (conversation_id, content) =
            row.map_err(|e| format!("Failed to read RAG conversation search row: {e}"))?;
        if content.to_lowercase().contains(&needle) {
            message_matches.insert(conversation_id);
        }
    }

    Ok(conversations
        .into_iter()
        .filter(|conversation| {
            conversation.title.to_lowercase().contains(&needle)
                || message_matches.contains(&conversation.id)
        })
        .collect())
}

/// Carga una conversación completa con sus mensajes en orden de `sort_index`.
/// Un `sources` corrupto en una fila degrada a vector vacío SOLO para ese
/// mensaje — nunca rompe la carga completa.
pub(crate) fn get_conversation(
    conn: &Connection,
    conversation_id: &str,
) -> Result<RagConversation, String> {
    let title: Option<String> = conn
        .query_row(
            "SELECT title FROM rag_conversations WHERE id = ?1",
            rusqlite::params![conversation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to load RAG conversation '{conversation_id}': {e}"))?;

    let Some(title) = title else {
        return Err("La conversación no existe o fue eliminada.".to_string());
    };

    let mut stmt = conn
        .prepare(
            "SELECT id, role, content, sources, created_at
             FROM rag_messages
             WHERE conversation_id = ?1
             ORDER BY sort_index ASC",
        )
        .map_err(|e| format!("Failed to prepare RAG messages query: {e}"))?;

    let messages = stmt
        .query_map(rusqlite::params![conversation_id], |row| {
            Ok(RagMessage {
                id: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                sources: decode_sources(row.get::<_, Option<String>>(3)?),
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| format!("Failed to run RAG messages query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read RAG message rows: {e}"))?;

    Ok(RagConversation {
        id: conversation_id.to_string(),
        title,
        messages,
    })
}

/// JSON de `rag_messages.sources` → `Vec<RagSource>`; NULL, `'[]'` o JSON
/// corrupto → vector vacío.
fn decode_sources(raw: Option<String>) -> Vec<RagSource> {
    raw.and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Updates only the persisted title. The caller receives a missing-id error
/// instead of a false successful rename.
pub(crate) fn update_conversation_title(
    conn: &Connection,
    conversation_id: &str,
    raw_title: &str,
) -> Result<(), String> {
    let title = raw_title.trim();
    if title.is_empty() {
        return Err("El nombre de la conversación no puede estar vacío.".to_string());
    }

    let affected = conn
        .execute(
            "UPDATE rag_conversations SET title = ?1 WHERE id = ?2",
            rusqlite::params![title, conversation_id],
        )
        .map_err(|e| format!("Failed to update RAG conversation title: {e}"))?;

    if affected == 0 {
        return Err("La conversación no existe o fue eliminada.".to_string());
    }

    Ok(())
}

/// Borra una conversación y sus mensajes en una transacción. Los mensajes se
/// borran EXPLÍCITAMENTE antes que la conversación: no dependemos de
/// `ON DELETE CASCADE` porque `PRAGMA foreign_keys` puede estar apagado.
/// Borrar un id inexistente es un no-op `Ok(())`.
pub(crate) fn delete_conversation(
    conn: &mut Connection,
    conversation_id: &str,
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("Failed to start RAG delete transaction: {e}"))?;
    tx.execute(
        "DELETE FROM rag_messages WHERE conversation_id = ?1",
        rusqlite::params![conversation_id],
    )
    .map_err(|e| format!("Failed to delete RAG messages: {e}"))?;
    tx.execute(
        "DELETE FROM rag_conversations WHERE id = ?1",
        rusqlite::params![conversation_id],
    )
    .map_err(|e| format!("Failed to delete RAG conversation: {e}"))?;
    tx.commit()
        .map_err(|e| format!("Failed to commit RAG delete transaction: {e}"))?;
    Ok(())
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// Réplica exacta del DDL que crea la migración del frontend.
    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory DB failed");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS rag_conversations (
               id TEXT PRIMARY KEY,
               title TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS rag_messages (
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES rag_conversations(id) ON DELETE CASCADE,
               sort_index INTEGER NOT NULL,
               role TEXT NOT NULL CHECK(role IN ('user','assistant')),
               content TEXT NOT NULL,
               sources TEXT,
               model TEXT,
               created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_rag_messages_conversation
             ON rag_messages(conversation_id, sort_index);",
        )
        .expect("RAG chat schema creation failed");
        conn
    }

    fn source(index: u32) -> RagSource {
        RagSource {
            index,
            asset_id: format!("asset-{index}"),
            item_id: format!("item-{index}"),
            item_title: "Acta del Cabildo".to_string(),
            collection_id: "col-1".to_string(),
            collection_name: "Archivo General".to_string(),
            snippet: "fragmento con ñandú".to_string(),
            score: 0.5,
            start_seconds: Some(1.5),
            end_seconds: Some(4.0),
            provenance: None,
        }
    }

    fn sort_indexes(conn: &Connection, conversation_id: &str) -> Vec<i64> {
        let mut stmt = conn
            .prepare(
                "SELECT sort_index FROM rag_messages
                 WHERE conversation_id = ?1 ORDER BY sort_index ASC",
            )
            .expect("prepare sort_index query");
        stmt.query_map(params![conversation_id], |row| row.get(0))
            .expect("run sort_index query")
            .collect::<Result<Vec<i64>, _>>()
            .expect("read sort_index rows")
    }

    // ── Title ────────────────────────────────────────────────────────────────

    #[test]
    fn conversation_title_truncates_by_chars_and_collapses_lines() {
        assert_eq!(conversation_title("hola"), "hola");
        assert_eq!(conversation_title(&"á".repeat(60)), "á".repeat(60));
        assert_eq!(
            conversation_title("línea uno\nlínea  dos"),
            "línea uno línea dos"
        );

        let truncated = conversation_title(&"á".repeat(61));
        assert_eq!(truncated.chars().count(), 61, "60 chars + ellipsis");
        assert!(truncated.ends_with('…'));
        assert_eq!(
            truncated.chars().take(60).collect::<String>(),
            "á".repeat(60)
        );
    }

    // ── Persist round-trip ───────────────────────────────────────────────────

    #[test]
    fn persist_creates_conversation_with_truncated_title_and_roundtrips_sources() {
        let mut conn = setup_conn();
        let question = "á".repeat(61);
        let sources = vec![source(1), source(2)];

        let id = persist_exchange(
            &mut conn,
            None,
            &question,
            "respuesta [1][2]",
            &sources,
            "modelo-x",
            1_000,
        )
        .expect("persist should succeed");

        let convo = get_conversation(&conn, &id).expect("conversation should load");
        assert_eq!(convo.id, id);
        assert_eq!(convo.title.chars().count(), 61, "60 'á' + '…'");
        assert!(convo.title.ends_with('…'));
        assert_eq!(
            convo.title.chars().take(60).collect::<String>(),
            "á".repeat(60)
        );

        assert_eq!(convo.messages.len(), 2);
        assert_eq!(sort_indexes(&conn, &id), vec![0, 1]);

        let user = &convo.messages[0];
        assert_eq!(user.role, "user");
        assert_eq!(user.content, question);
        assert!(user.sources.is_empty(), "user message has no sources");

        let assistant = &convo.messages[1];
        assert_eq!(assistant.role, "assistant");
        assert_eq!(assistant.content, "respuesta [1][2]");
        assert_eq!(assistant.created_at, 1_000);

        // Round-trip exacto de las fuentes a través del JSON persistido.
        assert_eq!(assistant.sources.len(), 2);
        assert_eq!(assistant.sources[0].index, 1);
        assert_eq!(assistant.sources[0].asset_id, "asset-1");
        assert_eq!(assistant.sources[0].item_title, "Acta del Cabildo");
        assert_eq!(assistant.sources[0].snippet, "fragmento con ñandú");
        assert_eq!(assistant.sources[0].score, 0.5);
        assert_eq!(assistant.sources[0].start_seconds, Some(1.5));
        assert_eq!(assistant.sources[0].end_seconds, Some(4.0));
        assert_eq!(assistant.sources[1].index, 2);

        let model: Option<String> = conn
            .query_row(
                "SELECT model FROM rag_messages WHERE conversation_id = ?1 AND role = 'assistant'",
                params![id],
                |row| row.get(0),
            )
            .expect("read assistant model");
        assert_eq!(model.as_deref(), Some("modelo-x"));
    }

    #[test]
    fn second_exchange_appends_sort_index_and_bumps_updated_at() {
        let mut conn = setup_conn();
        let id = persist_exchange(
            &mut conn,
            None,
            "primera pregunta",
            "primera respuesta",
            &[],
            "modelo-x",
            1_000,
        )
        .expect("first persist");

        let returned = persist_exchange(
            &mut conn,
            Some(&id),
            "segunda pregunta",
            "segunda respuesta",
            &[],
            "modelo-x",
            2_000,
        )
        .expect("second persist");
        assert_eq!(returned, id, "existing conversation keeps its id");

        assert_eq!(sort_indexes(&conn, &id), vec![0, 1, 2, 3]);

        let (created_at, updated_at, title): (i64, i64, String) = conn
            .query_row(
                "SELECT created_at, updated_at, title FROM rag_conversations WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read conversation row");
        assert_eq!(created_at, 1_000, "created_at never changes");
        assert_eq!(updated_at, 2_000, "updated_at bumped by second exchange");
        assert_eq!(title, "primera pregunta", "title keeps the first question");
    }

    // ── History loader ───────────────────────────────────────────────────────

    #[test]
    fn load_history_returns_turns_in_order_and_respects_window() {
        let mut conn = setup_conn();
        let mut id: Option<String> = None;
        for i in 1..=4 {
            let persisted = persist_exchange(
                &mut conn,
                id.as_deref(),
                &format!("pregunta {i}"),
                &format!("respuesta {i}"),
                &[],
                "modelo-x",
                i64::from(i) * 1_000,
            )
            .expect("persist exchange");
            id = Some(persisted);
        }
        let id = id.expect("conversation id");

        // 8 mensajes en total; la ventana de 6 deja afuera el primer intercambio.
        let turns = load_history(&conn, &id, 6).expect("history should load");
        assert_eq!(turns.len(), 6);
        let contents: Vec<&str> = turns.iter().map(|turn| turn.content.as_str()).collect();
        assert_eq!(
            contents,
            vec![
                "pregunta 2",
                "respuesta 2",
                "pregunta 3",
                "respuesta 3",
                "pregunta 4",
                "respuesta 4",
            ],
            "chronological order, oldest of the window first"
        );
        let roles: Vec<&str> = turns.iter().map(|turn| turn.role.as_str()).collect();
        assert_eq!(
            roles,
            vec![
                "user",
                "assistant",
                "user",
                "assistant",
                "user",
                "assistant"
            ]
        );

        // Conversación inexistente → historial vacío, sin error.
        let empty = load_history(&conn, "fantasma", 6).expect("missing conversation");
        assert!(empty.is_empty());
    }

    #[test]
    fn load_history_window_skips_empty_no_result_answers() {
        let mut conn = setup_conn();
        let id = persist_exchange(
            &mut conn,
            None,
            "pregunta 1",
            "respuesta 1",
            &[],
            "m",
            1_000,
        )
        .expect("persist 1");
        persist_exchange(
            &mut conn,
            Some(&id),
            "pregunta 2",
            "respuesta 2",
            &[],
            "m",
            2_000,
        )
        .expect("persist 2");
        // Tres intercambios "sin resultados": la respuesta vacía no debe
        // contar para la ventana del LIMIT ni desalojar los turnos reales.
        for i in 3..=5i64 {
            persist_exchange(
                &mut conn,
                Some(&id),
                &format!("pregunta {i}"),
                "",
                &[],
                "m",
                i * 1_000,
            )
            .expect("persist empty exchange");
        }

        let turns = load_history(&conn, &id, 6).expect("history should load");
        let contents: Vec<&str> = turns.iter().map(|turn| turn.content.as_str()).collect();
        assert_eq!(
            contents,
            vec![
                "respuesta 1",
                "pregunta 2",
                "respuesta 2",
                "pregunta 3",
                "pregunta 4",
                "pregunta 5",
            ],
            "real turns survive: empty assistant rows do not consume the window"
        );
        assert!(
            turns.iter().all(|turn| !turn.content.trim().is_empty()),
            "no empty turns leak into the history"
        );
    }

    // ── Deleted-in-flight conversation id ────────────────────────────────────

    #[test]
    fn missing_conversation_id_creates_fresh_conversation() {
        let mut conn = setup_conn();
        let id = persist_exchange(
            &mut conn,
            Some("borrada-en-vuelo"),
            "pregunta",
            "respuesta",
            &[],
            "modelo-x",
            1_000,
        )
        .expect("persist with stale id");

        assert_ne!(id, "borrada-en-vuelo", "stale id must not be resurrected");
        let stale_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM rag_conversations WHERE id = 'borrada-en-vuelo'",
                [],
                |row| row.get(0),
            )
            .expect("count stale id");
        assert_eq!(stale_count, 0, "old id stays absent");

        let convo = get_conversation(&conn, &id).expect("fresh conversation loads");
        assert_eq!(convo.messages.len(), 2);
    }

    // ── List ─────────────────────────────────────────────────────────────────

    #[test]
    fn list_orders_by_updated_at_desc_with_message_counts() {
        let mut conn = setup_conn();
        let a = persist_exchange(&mut conn, None, "conversación a", "r", &[], "m", 1_000)
            .expect("persist a");
        let b = persist_exchange(&mut conn, None, "conversación b", "r", &[], "m", 2_000)
            .expect("persist b");
        // a recibe un segundo intercambio y pasa al frente del listado.
        persist_exchange(&mut conn, Some(&a), "otra de a", "r", &[], "m", 3_000)
            .expect("persist a again");

        let list = list_conversations(&conn).expect("list should load");
        assert_eq!(list.len(), 2);

        assert_eq!(list[0].id, a);
        assert_eq!(list[0].title, "conversación a");
        assert_eq!(list[0].created_at, 1_000);
        assert_eq!(list[0].updated_at, 3_000);
        assert_eq!(list[0].message_count, 4);

        assert_eq!(list[1].id, b);
        assert_eq!(list[1].updated_at, 2_000);
        assert_eq!(list[1].message_count, 2);
    }

    // ── Get ──────────────────────────────────────────────────────────────────

    #[test]
    fn get_missing_conversation_errors_in_spanish() {
        let conn = setup_conn();
        let error = get_conversation(&conn, "fantasma").expect_err("missing id must fail");
        assert_eq!(error, "La conversación no existe o fue eliminada.");
    }

    #[test]
    fn corrupt_sources_json_degrades_to_empty_for_that_message_only() {
        let mut conn = setup_conn();
        let id = persist_exchange(
            &mut conn,
            None,
            "pregunta",
            "respuesta",
            &[source(1)],
            "modelo-x",
            1_000,
        )
        .expect("first persist");
        persist_exchange(
            &mut conn,
            Some(&id),
            "pregunta 2",
            "respuesta 2",
            &[source(2)],
            "modelo-x",
            2_000,
        )
        .expect("second persist");

        // Corrompemos SOLO las fuentes del segundo mensaje del asistente.
        conn.execute(
            "UPDATE rag_messages SET sources = 'no es json'
             WHERE conversation_id = ?1 AND sort_index = 3",
            params![id],
        )
        .expect("corrupt sources");

        let convo = get_conversation(&conn, &id).expect("load must not fail");
        assert_eq!(convo.messages.len(), 4);
        assert_eq!(
            convo.messages[1].sources.len(),
            1,
            "intact row keeps its sources"
        );
        assert_eq!(convo.messages[1].sources[0].asset_id, "asset-1");
        assert!(
            convo.messages[3].sources.is_empty(),
            "corrupt row degrades to empty sources"
        );
        assert_eq!(convo.messages[3].content, "respuesta 2");
    }

    // ── Delete ───────────────────────────────────────────────────────────────

    #[test]
    fn delete_removes_messages_and_conversation_and_missing_id_is_noop() {
        let mut conn = setup_conn();
        let id = persist_exchange(
            &mut conn,
            None,
            "pregunta",
            "respuesta",
            &[source(1)],
            "modelo-x",
            1_000,
        )
        .expect("persist");

        delete_conversation(&mut conn, &id).expect("delete should succeed");

        let (conversations, messages): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM rag_conversations),
                        (SELECT COUNT(*) FROM rag_messages)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("count rows");
        assert_eq!(conversations, 0);
        assert_eq!(messages, 0, "messages removed explicitly, not via CASCADE");

        delete_conversation(&mut conn, "fantasma").expect("missing id is a no-op Ok");
    }

    #[test]
    fn update_conversation_title_trims_and_preserves_all_other_conversation_data() {
        let mut conn = setup_conn();
        let id = persist_exchange(
            &mut conn,
            None,
            "pregunta original",
            "respuesta original",
            &[],
            "modelo-x",
            1_000,
        )
        .expect("persist exchange");

        let snapshot_messages = || {
            let mut stmt = conn
                .prepare(
                    "SELECT id, conversation_id, sort_index, role, content,
                            sources, model, created_at
                     FROM rag_messages
                     WHERE conversation_id = ?1
                     ORDER BY sort_index ASC",
                )
                .expect("prepare message snapshot query");
            stmt.query_map(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })
            .expect("run message snapshot query")
            .collect::<Result<Vec<_>, _>>()
            .expect("read message snapshot")
        };
        let messages_before = snapshot_messages();

        update_conversation_title(&conn, &id, "  título renovado  ")
            .expect("title update should succeed");

        let (stored_id, title, created_at, updated_at): (String, String, i64, i64) = conn
            .query_row(
                "SELECT id, title, created_at, updated_at FROM rag_conversations WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("read updated conversation");

        assert_eq!(stored_id, id);
        assert_eq!(title, "título renovado");
        assert_eq!(created_at, 1_000);
        assert_eq!(updated_at, 1_000);

        let messages_after = snapshot_messages();
        assert_eq!(
            messages_after, messages_before,
            "title updates must preserve every rag_messages column"
        );

        let conversation = get_conversation(&conn, &id).expect("conversation should load");
        assert_eq!(conversation.id, id);
        assert_eq!(conversation.messages.len(), 2);
        assert_eq!(conversation.messages[0].content, "pregunta original");
        assert_eq!(conversation.messages[1].content, "respuesta original");
    }

    #[test]
    fn update_conversation_title_rejects_blank_titles_and_missing_ids() {
        let mut conn = setup_conn();
        let id = persist_exchange(
            &mut conn,
            None,
            "pregunta original",
            "respuesta original",
            &[],
            "modelo-x",
            1_000,
        )
        .expect("persist exchange");

        let blank = update_conversation_title(&conn, &id, "  ").expect_err("blank title must fail");
        assert!(blank.contains("nombre"));

        let missing = update_conversation_title(&conn, "missing-id", "Nuevo título")
            .expect_err("missing conversation must fail");
        assert!(missing.contains("no existe") || missing.contains("eliminada"));
    }

    #[test]
    fn search_conversations_matches_titles_and_all_message_text_literally() {
        let conn = setup_conn();

        conn.execute(
            "INSERT INTO rag_conversations(id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params!["conv-old", "Salarios del SOIP", 100, 200],
        )
        .expect("insert old conversation");
        conn.execute(
            "INSERT INTO rag_conversations(id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params!["conv-new", "Acta sindical", 300, 400],
        )
        .expect("insert new conversation");
        conn.execute(
            "INSERT INTO rag_conversations(id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params!["conv-empty", "Sin coincidencias", 500, 600],
        )
        .expect("insert unmatched conversation");

        let messages = [
            ("msg-old-user", "conv-old", 0, "¿Cuánto ganaban?"),
            ("msg-old-assistant", "conv-old", 1, "El 100% efectivo del jornal."),
            ("msg-new-user", "conv-new", 0, "Pregunta sobre la huelga"),
            ("msg-new-assistant", "conv-new", 1, "La respuesta menciona operadores."),
        ];
        for (id, conversation_id, sort_index, content) in messages {
            conn.execute(
                "INSERT INTO rag_messages(id, conversation_id, sort_index, role, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    conversation_id,
                    sort_index,
                    if sort_index == 0 { "user" } else { "assistant" },
                    content,
                    100 + sort_index,
                ],
            )
            .expect("insert search message");
        }

        let title_match = search_conversations(&conn, "SALARIOS").expect("title search");
        assert_eq!(
            title_match.iter().map(|conversation| conversation.id.as_str()).collect::<Vec<_>>(),
            vec!["conv-old"]
        );

        let assistant_match = search_conversations(&conn, "OPERADORES").expect("assistant search");
        assert_eq!(
            assistant_match.iter().map(|conversation| conversation.id.as_str()).collect::<Vec<_>>(),
            vec!["conv-new"]
        );

        let literal_match = search_conversations(&conn, "%").expect("literal percent search");
        assert_eq!(
            literal_match.iter().map(|conversation| conversation.id.as_str()).collect::<Vec<_>>(),
            vec!["conv-old"]
        );

        let no_match = search_conversations(&conn, "AND").expect("operator-like search");
        assert!(no_match.is_empty());

        let all = search_conversations(&conn, " ").expect("empty search");
        assert_eq!(
            all.iter().map(|conversation| conversation.id.as_str()).collect::<Vec<_>>(),
            vec!["conv-empty", "conv-new", "conv-old"]
        );
    }

}
