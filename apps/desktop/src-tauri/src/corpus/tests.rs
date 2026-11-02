//! Behavior of the corpus services over an archive built by the real
//! migrations.

use std::collections::BTreeMap;

use rusqlite::Connection;

use super::*;
use crate::automation::test_archive::{item_id, seeded_archive, ALFA, BETA};

fn archive() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = crate::db::open::open_archive_connection(&seeded_archive(dir.path())).unwrap();
    (dir, conn)
}

fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn search_hits(conn: &Connection, term: &str) -> Vec<String> {
    // fts_items stores no content (content=''), so its columns read as NULL:
    // the item is found through the shared rowid, as the app's search does.
    let mut statement = conn
        .prepare(
            "SELECT i.id FROM fts_items f JOIN items i ON i.rowid = f.rowid
             WHERE fts_items MATCH ?1",
        )
        .unwrap();
    statement
        .query_map([term], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect()
}

fn row_count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

#[test]
fn creating_a_collection_trims_its_name_and_records_it() {
    let (_dir, conn) = archive();
    let created = create_collection(&conn, "  Cartas  ", Some(" Epistolario "), None).unwrap();
    assert_eq!(created.name, "Cartas");
    assert_eq!(created.description.as_deref(), Some("Epistolario"));
    assert_eq!(created.created_at, created.updated_at);
    let stored: (String, Option<String>) = conn
        .query_row(
            "SELECT name, description FROM collections WHERE id = ?1",
            [&created.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        ("Cartas".to_string(), Some("Epistolario".to_string()))
    );
}

#[test]
fn a_collection_can_keep_the_id_the_app_chose() {
    let (_dir, conn) = archive();
    let created = create_collection(&conn, "Ejemplo", None, Some("sample-fixed-id")).unwrap();
    assert_eq!(created.id, "sample-fixed-id");
    let again = create_collection(&conn, "Otra", None, Some("sample-fixed-id")).unwrap_err();
    assert_eq!(
        again,
        CorpusError::InvalidArgument("a collection with id sample-fixed-id already exists".into())
    );
}

#[test]
fn an_invalid_collection_name_is_refused_and_creates_nothing() {
    let (_dir, conn) = archive();
    let before = row_count(&conn, "collections");
    for name in ["   ", "a\u{0007}b", &"x".repeat(201)] {
        let error = create_collection(&conn, name, None, None).unwrap_err();
        assert!(
            matches!(error, CorpusError::InvalidArgument(_)),
            "{name:?}: {error:?}"
        );
    }
    assert_eq!(row_count(&conn, "collections"), before);
}

#[test]
fn renaming_a_collection_is_fenced_by_its_revision_and_moves_it() {
    let (_dir, conn) = archive();
    let before: i64 = conn
        .query_row(
            "SELECT updated_at FROM collections WHERE id = ?1",
            [ALFA],
            |row| row.get(0),
        )
        .unwrap();
    let stale = update_collection(
        &conn,
        ALFA,
        &CollectionPatch {
            name: Some("Nueva".into()),
            description: None,
        },
        Some(before - 1),
    )
    .unwrap_err();
    assert_eq!(
        stale,
        CorpusError::RevisionConflict(format!(
            "collection {ALFA} is at revision {before}, not {}",
            before - 1
        ))
    );
    let renamed = update_collection(
        &conn,
        ALFA,
        &CollectionPatch {
            name: Some("Alfa renombrada".into()),
            description: Some(None),
        },
        Some(before),
    )
    .unwrap();
    assert_eq!(renamed.name, "Alfa renombrada");
    assert_eq!(
        renamed.description, None,
        "Some(None) clears the description"
    );
    assert!(
        renamed.updated_at > before,
        "a change always moves the revision"
    );
}

#[test]
fn updating_an_unknown_or_empty_collection_request_is_refused() {
    let (_dir, conn) = archive();
    assert!(matches!(
        update_collection(
            &conn,
            "col_missing",
            &CollectionPatch {
                name: Some("x".into()),
                description: None
            },
            None
        ),
        Err(CorpusError::NotFound(_))
    ));
    assert!(matches!(
        update_collection(&conn, ALFA, &CollectionPatch::default(), None),
        Err(CorpusError::InvalidArgument(_))
    ));
}

#[test]
fn editing_an_item_keeps_the_reserved_metadata_and_refreshes_search() {
    let (_dir, conn) = archive();
    let id = item_id(ALFA, 1);
    // An imported item carries the app's provenance under a reserved key.
    conn.execute(
        "UPDATE items SET metadata = ?1 WHERE id = ?2",
        rusqlite::params![
            r#"{"autor":"Viejo","__entropia_file_metadata":{"name":"a.pdf"}}"#,
            id
        ],
    )
    .unwrap();
    let revision: i64 = conn
        .query_row("SELECT updated_at FROM items WHERE id = ?1", [&id], |row| {
            row.get(0)
        })
        .unwrap();
    let updated = update_item(
        &conn,
        &id,
        &ItemPatch {
            title: Some("  Carta de Sarmiento  ".into()),
            metadata: Some(fields(&[("autor", "Sarmiento"), ("año", "1849")])),
            ..ItemPatch::default()
        },
        Some(revision),
    )
    .unwrap();
    assert_eq!(updated.title, "Carta de Sarmiento");
    let metadata: serde_json::Value =
        serde_json::from_str(updated.metadata.as_deref().unwrap()).unwrap();
    assert_eq!(
        metadata,
        serde_json::json!({
            "autor": "Sarmiento",
            "año": "1849",
            "__entropia_file_metadata": { "name": "a.pdf" }
        })
    );
    assert!(updated.updated_at > revision);
    assert_eq!(
        search_hits(&conn, "sarmiento"),
        vec![id.clone()],
        "the new text is searchable"
    );
    assert!(
        search_hits(&conn, "viejo").is_empty(),
        "the old value is gone from the index"
    );
}

#[test]
fn an_item_edit_cannot_write_a_reserved_key_or_a_stale_revision() {
    let (_dir, conn) = archive();
    let id = item_id(BETA, 1);
    let before: (String, Option<String>, i64) = conn
        .query_row(
            "SELECT title, metadata, updated_at FROM items WHERE id = ?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let reserved = update_item(
        &conn,
        &id,
        &ItemPatch {
            metadata: Some(fields(&[("__entropia_web_capture", "x")])),
            ..ItemPatch::default()
        },
        None,
    )
    .unwrap_err();
    assert!(
        matches!(reserved, CorpusError::InvalidArgument(_)),
        "{reserved:?}"
    );
    let stale = update_item(
        &conn,
        &id,
        &ItemPatch {
            title: Some("Otro".into()),
            ..ItemPatch::default()
        },
        Some(before.2 + 10),
    )
    .unwrap_err();
    assert!(
        matches!(stale, CorpusError::RevisionConflict(_)),
        "{stale:?}"
    );
    for title in ["  ", &"t".repeat(501)] {
        let error = update_item(
            &conn,
            &id,
            &ItemPatch {
                title: Some(title.to_string()),
                ..ItemPatch::default()
            },
            None,
        )
        .unwrap_err();
        assert!(
            matches!(error, CorpusError::InvalidArgument(_)),
            "{title:?}"
        );
    }
    let after: (String, Option<String>, i64) = conn
        .query_row(
            "SELECT title, metadata, updated_at FROM items WHERE id = ?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(after, before, "a refused edit changes nothing");
    assert!(matches!(
        update_item(
            &conn,
            "ite_missing",
            &ItemPatch {
                title: Some("x".into()),
                ..ItemPatch::default()
            },
            None
        ),
        Err(CorpusError::NotFound(_))
    ));
}

#[test]
fn every_write_is_captured_for_sync() {
    let (_dir, conn) = archive();
    // The app installs the sync tables and their capture triggers at startup.
    crate::sync::schema::ensure_sync_schema(&conn).unwrap();
    crate::sync::capture::ensure_capture(&conn).unwrap();
    // Capture runs while a sync session is on, which is what sets this flag.
    conn.execute(
        "INSERT OR REPLACE INTO sync_meta (key, value) VALUES ('capture_enabled', '1')",
        [],
    )
    .unwrap();
    let before = row_count(&conn, "sync_oplog");
    let created = create_collection(&conn, "Sincronizada", None, None).unwrap();
    update_collection(
        &conn,
        &created.id,
        &CollectionPatch {
            name: Some("Sincronizada 2".into()),
            description: None,
        },
        None,
    )
    .unwrap();
    update_item(
        &conn,
        &item_id(ALFA, 2),
        &ItemPatch {
            title: Some("Retitulado".into()),
            ..ItemPatch::default()
        },
        None,
    )
    .unwrap();
    assert!(
        row_count(&conn, "sync_oplog") >= before + 3,
        "the sync triggers saw every write"
    );
}

#[test]
fn the_app_can_store_the_metadata_json_it_built_itself() {
    let (_dir, conn) = archive();
    let id = item_id(ALFA, 3);
    let json = r#"{"autor":"Mitre","__entropia_file_metadata":{"name":"b.pdf"}}"#;
    let updated = update_item(
        &conn,
        &id,
        &ItemPatch {
            metadata_json: Some(json.to_string()),
            ..ItemPatch::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(
        updated.metadata.as_deref(),
        Some(json),
        "stored exactly as the app sent it"
    );
    assert_eq!(search_hits(&conn, "mitre"), vec![id.clone()]);

    for (patch, what) in [
        (
            ItemPatch {
                metadata_json: Some("[1,2]".into()),
                ..ItemPatch::default()
            },
            "not an object",
        ),
        (
            ItemPatch {
                metadata_json: Some("{".into()),
                ..ItemPatch::default()
            },
            "not JSON",
        ),
        (
            ItemPatch {
                metadata: Some(fields(&[("a", "b")])),
                metadata_json: Some("{}".into()),
                ..ItemPatch::default()
            },
            "both forms",
        ),
    ] {
        let error = update_item(&conn, &id, &patch, None).unwrap_err();
        assert!(
            matches!(error, CorpusError::InvalidArgument(_)),
            "{what}: {error:?}"
        );
    }
}
