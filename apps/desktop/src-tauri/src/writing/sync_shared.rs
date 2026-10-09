//! Local memory of Escritura documents shared across accounts
//! (EntropIA-Cloud PROTOCOL "Documentos compartidos de Escritura").
//!
//! A shared document arrives linked to the other account's collections, which
//! never exist on this device. Instead of deferring it forever, the receiver
//! applies it with the local links only and parks the foreign ones here, and
//! `snapshot_document` merges them back so a push never drops the other
//! account's links. Both lists live in `sync_meta`, which only exists once sync
//! was initialised; without it nothing is shared and nothing is parked.

use rusqlite::{params, Connection, OptionalExtension};

use super::repository::{WritingError, WritingResult};
use super::sync_envelope::CollectionAssociationV1;

const SHARE_PREFIX: &str = "writing_share:";
const FOREIGN_PREFIX: &str = "writing_foreign_collections:";

fn has_sync_meta(conn: &Connection) -> WritingResult<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sync_meta')",
        [],
        |row| row.get(0),
    )
    .map_err(|error| WritingError::sql("Failed to look for sync_meta", error))
}

pub(crate) fn is_shared_document(conn: &Connection, document_id: &str) -> WritingResult<bool> {
    if !has_sync_meta(conn)? {
        return Ok(false);
    }
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_meta WHERE key = ?1)",
        [format!("{SHARE_PREFIX}{document_id}")],
        |row| row.get(0),
    )
    .map_err(|error| WritingError::sql("Failed to read the shared-document list", error))
}

/// Replaces the shared-document list with the server's current answer. The
/// value is the JSON the server sent for that document, for the UI.
pub(crate) fn replace_shared_documents(
    conn: &Connection,
    shares: &[(String, String)],
) -> WritingResult<()> {
    conn.execute(
        "DELETE FROM sync_meta WHERE substr(key, 1, length(?1)) = ?1",
        [SHARE_PREFIX],
    )
    .map_err(|error| WritingError::sql("Failed to clear the shared-document list", error))?;
    for (document_id, value) in shares {
        conn.execute(
            "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)",
            params![format!("{SHARE_PREFIX}{document_id}"), value],
        )
        .map_err(|error| WritingError::sql("Failed to store a shared document", error))?;
    }
    Ok(())
}

pub(crate) fn load_parked_collections(
    conn: &Connection,
    document_id: &str,
) -> WritingResult<Vec<CollectionAssociationV1>> {
    if !has_sync_meta(conn)? {
        return Ok(Vec::new());
    }
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM sync_meta WHERE key = ?1",
            [format!("{FOREIGN_PREFIX}{document_id}")],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| WritingError::sql("Failed to read foreign collections", error))?;
    match value {
        None => Ok(Vec::new()),
        Some(json) => serde_json::from_str(&json).map_err(|error| {
            WritingError::new(
                "invalid_foreign_collections",
                format!("foreign collections of {document_id} are not valid JSON: {error}"),
            )
        }),
    }
}

pub(crate) fn store_parked_collections(
    conn: &Connection,
    document_id: &str,
    parked: &[CollectionAssociationV1],
) -> WritingResult<()> {
    let key = format!("{FOREIGN_PREFIX}{document_id}");
    let result = if parked.is_empty() {
        if !has_sync_meta(conn)? {
            return Ok(());
        }
        conn.execute("DELETE FROM sync_meta WHERE key = ?1", [key])
    } else {
        let json = serde_json::to_string(parked)
            .map_err(|error| WritingError::new("invalid_foreign_collections", error.to_string()))?;
        conn.execute(
            "INSERT INTO sync_meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, json],
        )
    };
    result
        .map(|_| ())
        .map_err(|error| WritingError::sql("Failed to store foreign collections", error))
}
