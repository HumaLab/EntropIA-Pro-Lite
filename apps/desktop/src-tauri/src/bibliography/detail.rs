//! Confirmed-catalog work detail reads (ODD slice E1c-3, Rust half).
//!
//! Read-only projection over the E1b catalog for one work. The UI half renders
//! the ficha from this DTO: a confirmed snapshot with its collections, tags
//! and attachment metadata; a tombstone with the last snapshot when Zotero no
//! longer carries the item; or an explicit state telling the UI to render from
//! its held CSL instead. No file is resolved or opened here, nothing is
//! scheduled, and nothing is written.

use rusqlite::{Connection, OptionalExtension};

use super::repository::{
    read_item, read_item_tombstone, BibliographicItem, BibliographyError, BibliographyResult,
    TombstoneRecord,
};
use crate::writing::zotero::Library;

/// One Zotero creator parsed from `creators_json`.
///
/// Lenient by design: unknown fields are ignored so creator shapes Zotero adds
/// tomorrow still parse, and every field is optional so a partial object still
/// carries what it has.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Creator {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Attachment metadata only. `native_path` is deliberately absent: the detail
/// never resolves a file, it only describes what the catalog remembers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDetail {
    pub attachment_key: String,
    pub content_type: Option<String>,
    pub link_mode: Option<String>,
    pub filename: Option<String>,
    pub url: Option<String>,
}

/// The last catalog metadata snapshot for one work, with its membership edges
/// projected as names, texts and attachment metadata.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDetailItem {
    pub item_key: String,
    pub item_type: Option<String>,
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creators: Option<Vec<Creator>>,
    pub publication_title: Option<String>,
    pub publisher: Option<String>,
    pub date: Option<String>,
    pub doi: Option<String>,
    pub isbn: Option<String>,
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    pub language: Option<String>,
    pub url: Option<String>,
    pub item_version: Option<i64>,
    pub collections: Vec<String>,
    pub tags: Vec<String>,
    pub attachments: Vec<AttachmentDetail>,
}

/// What the ficha can honestly show for one work.
///
/// `Confirmed` carries a live snapshot the completed reconciliation accounts
/// for. `LostLink` carries the tombstone plus the last snapshot, when the row
/// survives, for the 'Ítem no disponible en Zotero' copy. `NotInCatalog` means
/// the namespace is confirmed but this key is neither live nor tombstoned.
/// `CatalogUnavailable` means the UI renders from its held CSL with the
/// 'Copia local sin verificar ahora' copy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ItemDetail {
    #[serde(rename_all = "camelCase")]
    Confirmed {
        verified_at: i64,
        item: ItemDetailItem,
    },
    #[serde(rename_all = "camelCase")]
    LostLink {
        tombstone: TombstoneRecord,
        item: Option<ItemDetailItem>,
    },
    NotInCatalog,
    CatalogUnavailable,
}

/// Reads the detail for one work in a Zotero library. See [`ItemDetail`] for
/// what each status promises.
///
/// The confirmation reuses the `confirmed_local_personal_catalog` rules for
/// the namespace: exactly one local user NULL-instance library, a completed
/// finalize reconciliation, the item seen with a matching remote version and
/// verified no later than the completion, and no tombstone. A tombstoned item
/// with a persisted snapshot reads `LostLink`; a confirmed namespace holding
/// neither reads `NotInCatalog`. Anything that cannot be trusted — group
/// libraries, missing tables, a missing or ambiguous namespace, no completed
/// run — reads `CatalogUnavailable`, never an error.
pub fn item_detail(
    conn: &Connection,
    library: &Library,
    item_key: &str,
) -> BibliographyResult<ItemDetail> {
    if item_key.trim().is_empty() {
        return Err(BibliographyError::new(
            "invalid_input",
            "item key must not be empty",
        ));
    }
    if !library.is_user() {
        return Ok(ItemDetail::CatalogUnavailable);
    }
    if !has_detail_tables(conn)? {
        return Ok(ItemDetail::CatalogUnavailable);
    }

    let namespace_count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM zotero_libraries l
               JOIN zotero_connections c ON c.id = l.connection_id
              WHERE c.source_origin = 'local'
                AND c.source_instance_id IS NULL
                AND l.library_type = 'user'
                AND l.library_id = ?1",
            [&library.library_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to resolve Zotero library namespace: {error}"),
            )
        })?;
    if namespace_count != 1 {
        return Ok(ItemDetail::CatalogUnavailable);
    }
    let library_row_id: String = conn
        .query_row(
            "SELECT l.id
               FROM zotero_libraries l
               JOIN zotero_connections c ON c.id = l.connection_id
              WHERE c.source_origin = 'local'
                AND c.source_instance_id IS NULL
                AND l.library_type = 'user'
                AND l.library_id = ?1",
            [&library.library_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read Zotero library namespace: {error}"),
            )
        })?;

    let completed: Option<(String, i64)> = conn
        .query_row(
            "SELECT run_id, completed_at
               FROM zotero_reconciliation_runs
              WHERE library_id = ?1
                AND state = 'completed'
                AND phase = 'finalize'
                AND completed_at IS NOT NULL",
            [&library_row_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read confirmed reconciliation: {error}"),
            )
        })?;
    let Some((run_id, completed_at)) = completed else {
        return Ok(ItemDetail::CatalogUnavailable);
    };

    let has_snapshot: bool = conn
        .query_row(
            "SELECT 1 FROM bibliographic_items WHERE library_id = ?1 AND item_key = ?2",
            rusqlite::params![&library_row_id, item_key],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read bibliographic item: {error}"),
            )
        })?
        .is_some();
    if !has_snapshot {
        // No snapshot row means no tombstone either: the tombstone FK
        // requires the item row, so this key is simply unknown here.
        return Ok(ItemDetail::NotInCatalog);
    };
    let item = read_item(conn, &library_row_id, item_key)?;
    let tombstone = read_item_tombstone(conn, &item.id)?;
    let snapshot = project_item(conn, &library_row_id, &item)?;

    if let Some(tombstone) = tombstone {
        return Ok(ItemDetail::LostLink {
            tombstone,
            item: Some(snapshot),
        });
    }

    let seen_remote_version: Option<Option<i64>> = conn
        .query_row(
            "SELECT remote_version
               FROM zotero_reconciliation_seen
              WHERE library_id = ?1
                AND run_id = ?2
                AND entity_kind = 'item'
                AND entity_key = ?3",
            rusqlite::params![&library_row_id, &run_id, item_key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read confirmed Zotero item: {error}"),
            )
        })?;
    let version_matches = match seen_remote_version {
        None => false,
        Some(None) => true,
        Some(Some(seen)) => item.item_version == Some(seen),
    };
    let eligible_version = matches!(item.item_version, Some(version) if version >= 0);
    if version_matches && eligible_version && item.verified_at <= completed_at {
        return Ok(ItemDetail::Confirmed {
            verified_at: item.verified_at,
            item: snapshot,
        });
    }
    Ok(ItemDetail::NotInCatalog)
}

/// Every table the detail reads. A missing one means there is no catalog to
/// confirm yet, so the ficha renders from its held CSL.
fn has_detail_tables(conn: &Connection) -> BibliographyResult<bool> {
    let present: i64 = conn
        .query_row(
            "SELECT COUNT(*)
               FROM sqlite_master
              WHERE type = 'table'
                AND name IN (
                    'zotero_connections', 'zotero_libraries', 'bibliographic_items',
                    'zotero_reconciliation_runs', 'zotero_reconciliation_seen',
                    'zotero_item_tombstones', 'zotero_collections',
                    'zotero_item_collections', 'zotero_tags', 'zotero_item_tags',
                    'zotero_attachments'
                )",
            [],
            |row| row.get(0),
        )
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to inspect bibliography tables: {error}"),
            )
        })?;
    Ok(present == 11)
}

/// Projects one persisted snapshot with its membership edges. Attachment
/// columns are metadata only; `native_path` never leaves the database.
fn project_item(
    conn: &Connection,
    library_row_id: &str,
    item: &BibliographicItem,
) -> BibliographyResult<ItemDetailItem> {
    let mut collections_statement = conn
        .prepare(
            "SELECT c.name
               FROM zotero_collections c
               JOIN zotero_item_collections m ON m.collection_id = c.id
              WHERE m.library_id = ?1 AND m.item_id = ?2
              ORDER BY c.name ASC, c.collection_key ASC",
        )
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read confirmed Zotero collections: {error}"),
            )
        })?;
    let collections: Vec<String> = collections_statement
        .query_map(rusqlite::params![library_row_id, &item.id], |row| {
            row.get(0)
        })
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read confirmed Zotero collections: {error}"),
            )
        })?
        .collect::<Result<_, _>>()
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to decode confirmed Zotero collections: {error}"),
            )
        })?;

    let mut tags_statement = conn
        .prepare(
            "SELECT t.tag_text
               FROM zotero_tags t
               JOIN zotero_item_tags m ON m.tag_id = t.id
              WHERE m.library_id = ?1 AND m.item_id = ?2
              ORDER BY t.tag_text ASC",
        )
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read confirmed Zotero tags: {error}"),
            )
        })?;
    let tags: Vec<String> = tags_statement
        .query_map(rusqlite::params![library_row_id, &item.id], |row| {
            row.get(0)
        })
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read confirmed Zotero tags: {error}"),
            )
        })?
        .collect::<Result<_, _>>()
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to decode confirmed Zotero tags: {error}"),
            )
        })?;

    let mut attachments_statement = conn
        .prepare(
            "SELECT attachment_key, content_type, link_mode, filename, url
               FROM zotero_attachments
              WHERE item_id = ?1
              ORDER BY attachment_key ASC",
        )
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read confirmed Zotero attachments: {error}"),
            )
        })?;
    let attachments: Vec<AttachmentDetail> = attachments_statement
        .query_map([&item.id], |row| {
            Ok(AttachmentDetail {
                attachment_key: row.get(0)?,
                content_type: row.get(1)?,
                link_mode: row.get(2)?,
                filename: row.get(3)?,
                url: row.get(4)?,
            })
        })
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read confirmed Zotero attachments: {error}"),
            )
        })?
        .collect::<Result<_, _>>()
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to decode confirmed Zotero attachments: {error}"),
            )
        })?;

    Ok(ItemDetailItem {
        item_key: item.identity.item_key.clone(),
        item_type: item.item_type.clone(),
        title: item.title.clone(),
        creators: parse_creators(item.creators_json.as_deref()),
        publication_title: item.publication_title.clone(),
        publisher: item.publisher.clone(),
        date: item.date.clone(),
        doi: item.doi.clone(),
        isbn: item.isbn.clone(),
        abstract_text: item.abstract_text.clone(),
        language: item.language.clone(),
        url: item.url.clone(),
        item_version: item.item_version,
        collections,
        tags,
        attachments,
    })
}

/// Parses `creators_json` when it is a JSON array. Anything else — absent,
/// not an array, or unreadable as creators — omits creators rather than
/// failing the read. Storage already guarantees `json_valid`, so the
/// malformed half cannot arrive from the database; this stays total anyway.
fn parse_creators(creators_json: Option<&str>) -> Option<Vec<Creator>> {
    creators_json.and_then(|raw| serde_json::from_str(raw).ok())
}

#[cfg(test)]
mod tests {
    use super::super::reconciliation::{
        begin_run, checkpoint_page, finalize_run, BeginReconciliationInput,
        ReconciliationEntityKind, ReconciliationPageInput, ReconciliationPhase, ReconciliationRun,
        ReconciliationRunRef, ReconciliationSeenInput,
    };
    use super::super::repository::{
        tombstone_item, upsert_attachment, upsert_collection, upsert_connection, upsert_item,
        upsert_item_collection, upsert_item_tag, upsert_library, upsert_tag, AttachmentInput,
        BibliographicItemInput, CollectionInput, ItemCollectionInput, ItemTagInput,
        LibraryType as CatalogLibraryType, SourceOrigin, TagInput, TombstoneInput,
        UpsertConnection, UpsertLibrary,
    };
    use super::*;
    use rusqlite::Connection;

    const CATALOG_MIGRATION_SQL: &str =
        include_str!("../../../../../packages/store/src/migrations/0038_bibliography_catalog.sql");
    const RELATIONS_MIGRATION_SQL: &str = include_str!(
        "../../../../../packages/store/src/migrations/0039_bibliography_relations.sql"
    );
    const RECONCILIATION_MIGRATION_SQL: &str = include_str!(
        "../../../../../packages/store/src/migrations/0040_bibliography_reconciliation.sql"
    );

    fn migrated_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(CATALOG_MIGRATION_SQL)
            .expect("apply bibliography foundation migration");
        conn.execute_batch(RELATIONS_MIGRATION_SQL)
            .expect("apply bibliography relations migration");
        conn.execute_batch(RECONCILIATION_MIGRATION_SQL)
            .expect("apply bibliography reconciliation migration");
        conn
    }

    fn connection(id: &str) -> UpsertConnection {
        UpsertConnection {
            id: id.to_string(),
            source_origin: SourceOrigin::Local,
            source_instance_id: None,
            endpoint: Some("http://synthetic.invalid".to_string()),
            capabilities_json: r#"{"read":true}"#.to_string(),
        }
    }

    fn library(
        connection_id: &str,
        library_type: CatalogLibraryType,
        library_id: &str,
    ) -> UpsertLibrary {
        UpsertLibrary {
            connection_id: connection_id.to_string(),
            library_type,
            library_id: library_id.to_string(),
            name: format!("Synthetic {library_id}"),
            last_modified_version: Some(7),
        }
    }

    fn rich_item(key: &str, version: i64) -> BibliographicItemInput {
        BibliographicItemInput {
            item_key: key.to_string(),
            item_version: Some(version),
            native_json_snapshot: format!(r#"{{"key":"{key}"}}"#),
            csl_json_snapshot: format!(r#"{{"id":"csl-{key}","type":"book"}}"#),
            item_type: Some("book".to_string()),
            title: Some("Synthetic work".to_string()),
            creators_json: Some(
                r#"[{"creatorType":"author","firstName":"Ada","lastName":"Lovelace"}]"#.to_string(),
            ),
            publication_title: Some("Synthetic press".to_string()),
            publisher: Some("EntropIA".to_string()),
            date: Some("2024".to_string()),
            doi: Some("10.0000/synthetic".to_string()),
            isbn: Some("978-0-00-000000-0".to_string()),
            abstract_text: Some("A synthetic abstract.".to_string()),
            language: Some("en".to_string()),
            url: Some("https://synthetic.invalid/detail1".to_string()),
        }
    }

    fn seen_item(
        key: &str,
        remote_version: Option<i64>,
        observed_at: i64,
    ) -> ReconciliationSeenInput {
        ReconciliationSeenInput {
            entity_kind: ReconciliationEntityKind::Item,
            entity_key: key.to_string(),
            parent_key: None,
            remote_version,
            observed_at,
        }
    }

    fn run_ref(run: &ReconciliationRun) -> ReconciliationRunRef {
        ReconciliationRunRef {
            library_id: run.library_id.clone(),
            run_id: run.run_id.clone(),
            connection_revision: run.connection_revision,
        }
    }

    fn complete_reconciliation(
        conn: &mut Connection,
        library_id: &str,
        connection_revision: i64,
        remote_total: i64,
        seen: Vec<ReconciliationSeenInput>,
    ) -> ReconciliationRun {
        let run = begin_run(
            conn,
            BeginReconciliationInput {
                library_id: library_id.to_string(),
                connection_revision,
                cursor_start: 0,
                cursor_limit: 100,
                remote_total: Some(remote_total),
                target_version: Some(9),
            },
        )
        .expect("begin synthetic reconciliation");
        let checkpointed = checkpoint_page(
            conn,
            ReconciliationPageInput {
                run: run_ref(&run),
                phase: ReconciliationPhase::Versions,
                cursor_start: 0,
                next_cursor_start: remote_total,
                remote_total: Some(remote_total),
                seen,
            },
        )
        .expect("checkpoint synthetic reconciliation");
        finalize_run(conn, run_ref(&checkpointed)).expect("finalize synthetic reconciliation")
    }

    /// Personal namespace with one fully described item and a completed run.
    /// Returns the open connection plus the item's `verified_at`.
    fn confirmed_namespace(conn: &mut Connection, item_key: &str) -> i64 {
        let source = upsert_connection(conn, connection("conn-local")).expect("connection");
        let personal = upsert_library(conn, library(&source.id, CatalogLibraryType::User, "0"))
            .expect("personal library");
        let saved = upsert_item(conn, &personal.id, rich_item(item_key, 3)).expect("item");

        let beta = upsert_collection(
            conn,
            &personal.id,
            CollectionInput {
                collection_key: "BETA-COLL".to_string(),
                name: "Beta collection".to_string(),
                parent_collection_key: None,
                native_json_snapshot: r#"{"key":"BETA-COLL"}"#.to_string(),
                native_version: Some(1),
            },
        )
        .expect("beta collection");
        let alpha = upsert_collection(
            conn,
            &personal.id,
            CollectionInput {
                collection_key: "ALPHA-COLL".to_string(),
                name: "Alpha collection".to_string(),
                parent_collection_key: None,
                native_json_snapshot: r#"{"key":"ALPHA-COLL"}"#.to_string(),
                native_version: Some(1),
            },
        )
        .expect("alpha collection");
        for collection in [&beta, &alpha] {
            upsert_item_collection(
                conn,
                &personal.id,
                ItemCollectionInput {
                    item_id: saved.id.clone(),
                    collection_id: collection.id.clone(),
                },
            )
            .expect("collection membership");
        }
        for text in ["thesis", "draft"] {
            let tag = upsert_tag(
                conn,
                &personal.id,
                TagInput {
                    tag_text: text.to_string(),
                    tag_type: None,
                    native_json_snapshot: format!(r#"{{"tag":"{text}"}}"#),
                    native_version: Some(1),
                },
            )
            .expect("tag");
            upsert_item_tag(
                conn,
                &personal.id,
                ItemTagInput {
                    item_id: saved.id.clone(),
                    tag_id: tag.id.clone(),
                },
            )
            .expect("tag membership");
        }
        upsert_attachment(
            conn,
            &saved.id,
            AttachmentInput {
                attachment_key: "PDF01".to_string(),
                content_type: Some("application/pdf".to_string()),
                link_mode: Some("linked_file".to_string()),
                filename: Some("chapter.pdf".to_string()),
                native_path: Some(r"C:\fixture\chapter.pdf".to_string()),
                url: Some("https://synthetic.invalid/chapter.pdf".to_string()),
                md5: Some("synthetic-md5".to_string()),
                mtime: Some(1_700_000_000_123),
                native_json_snapshot: r#"{"key":"PDF01"}"#.to_string(),
                native_version: Some(1),
            },
        )
        .expect("attachment");

        complete_reconciliation(
            conn,
            &personal.id,
            source.revision,
            1,
            vec![seen_item(item_key, Some(3), 1)],
        );
        saved.verified_at
    }

    /// E1c-3 RED: a confirmed item projects its catalog metadata, memberships
    /// and attachment metadata — never a resolved path.
    #[test]
    fn e1c3_confirmed_detail_projects_metadata_edges_and_attachments() {
        let mut conn = migrated_db();
        let verified_at = confirmed_namespace(&mut conn, "DETAIL1");

        let detail = item_detail(&conn, &Library::user("0"), "DETAIL1").expect("read detail");

        let ItemDetail::Confirmed {
            verified_at: got,
            item,
        } = detail
        else {
            panic!("confirmed item must read confirmed, got {detail:?}");
        };
        assert_eq!(got, verified_at);
        assert_eq!(item.item_key, "DETAIL1");
        assert_eq!(item.item_type.as_deref(), Some("book"));
        assert_eq!(item.title.as_deref(), Some("Synthetic work"));
        assert_eq!(
            item.creators,
            Some(vec![Creator {
                creator_type: Some("author".to_string()),
                first_name: Some("Ada".to_string()),
                last_name: Some("Lovelace".to_string()),
                name: None,
            }])
        );
        assert_eq!(item.publication_title.as_deref(), Some("Synthetic press"));
        assert_eq!(item.publisher.as_deref(), Some("EntropIA"));
        assert_eq!(item.date.as_deref(), Some("2024"));
        assert_eq!(item.doi.as_deref(), Some("10.0000/synthetic"));
        assert_eq!(item.isbn.as_deref(), Some("978-0-00-000000-0"));
        assert_eq!(item.abstract_text.as_deref(), Some("A synthetic abstract."));
        assert_eq!(item.language.as_deref(), Some("en"));
        assert_eq!(
            item.url.as_deref(),
            Some("https://synthetic.invalid/detail1")
        );
        assert_eq!(item.item_version, Some(3));
        assert_eq!(
            item.collections,
            vec![
                "Alpha collection".to_string(),
                "Beta collection".to_string()
            ]
        );
        assert_eq!(item.tags, vec!["draft".to_string(), "thesis".to_string()]);
        assert_eq!(item.attachments.len(), 1);
        let attachment = &item.attachments[0];
        assert_eq!(attachment.attachment_key, "PDF01");
        assert_eq!(attachment.content_type.as_deref(), Some("application/pdf"));
        assert_eq!(attachment.link_mode.as_deref(), Some("linked_file"));
        assert_eq!(attachment.filename.as_deref(), Some("chapter.pdf"));
        assert_eq!(
            attachment.url.as_deref(),
            Some("https://synthetic.invalid/chapter.pdf")
        );
    }

    /// E1c-3 RED: `creators_json` present but not a creator array omits
    /// creators instead of failing the read. Storage guarantees `json_valid`
    /// (the upsert rejects the rest), so these are the shapes that can
    /// arrive: valid JSON that is not an array, or an array that is not
    /// creators.
    #[test]
    fn e1c3_unreadable_creators_are_omitted_not_failed() {
        for broken in [
            r#"{"creatorType":"author"}"#,
            r#""just a string""#,
            "42",
            "null",
            "[1]",
            r#"["author"]"#,
        ] {
            let mut conn = migrated_db();
            confirmed_namespace(&mut conn, "DETAIL1");
            conn.execute(
                "UPDATE bibliographic_items SET creators_json = ?1 WHERE item_key = 'DETAIL1'",
                [broken],
            )
            .expect("break creators_json");

            let detail = item_detail(&conn, &Library::user("0"), "DETAIL1").expect("read detail");

            let ItemDetail::Confirmed { item, .. } = detail else {
                panic!("item with broken creators must stay confirmed, got {detail:?}");
            };
            assert_eq!(
                item.creators, None,
                "broken creators_json {broken:?} must be omitted"
            );
        }
    }

    /// E1c-3 RED: a tombstoned item with a persisted snapshot reads lost_link
    /// with the tombstone and the last metadata.
    #[test]
    fn e1c3_tombstoned_item_reads_lost_link_with_snapshot() {
        let mut conn = migrated_db();
        confirmed_namespace(&mut conn, "DETAIL1");
        let library_row_id: String = conn
            .query_row(
                "SELECT id FROM zotero_libraries WHERE library_id = '0'",
                [],
                |row| row.get(0),
            )
            .expect("library row id");
        tombstone_item(
            &mut conn,
            &library_row_id,
            "DETAIL1",
            TombstoneInput {
                remote_version: Some(4),
                reason: "remote deletion page".to_string(),
            },
        )
        .expect("tombstone item");

        let detail = item_detail(&conn, &Library::user("0"), "DETAIL1").expect("read detail");

        let ItemDetail::LostLink { tombstone, item } = detail else {
            panic!("tombstoned item must read lost_link, got {detail:?}");
        };
        assert_eq!(tombstone.remote_version, Some(4));
        assert_eq!(tombstone.reason, "remote deletion page");
        assert!(tombstone.observed_at > 0);
        let snapshot = item.expect("lost_link carries the last snapshot");
        assert_eq!(snapshot.item_key, "DETAIL1");
        assert_eq!(snapshot.title.as_deref(), Some("Synthetic work"));
    }

    /// E1c-3 RED: a confirmed namespace with an unknown key reads
    /// not_in_catalog — neither confirmed nor tombstoned.
    #[test]
    fn e1c3_unknown_key_in_confirmed_namespace_is_not_in_catalog() {
        let mut conn = migrated_db();
        confirmed_namespace(&mut conn, "DETAIL1");

        let detail = item_detail(&conn, &Library::user("0"), "NEVER-SEEN").expect("read detail");

        assert_eq!(detail, ItemDetail::NotInCatalog);
    }

    /// E1c-3 RED: group libraries never read the confirmed catalog, even with
    /// a completed personal namespace present.
    #[test]
    fn e1c3_group_library_is_catalog_unavailable() {
        let mut conn = migrated_db();
        confirmed_namespace(&mut conn, "DETAIL1");

        let detail =
            item_detail(&conn, &Library::group("6680944"), "DETAIL1").expect("read detail");

        assert_eq!(detail, ItemDetail::CatalogUnavailable);
    }

    /// E1c-3 RED: a namespace without a completed reconciliation cannot back
    /// the ficha.
    #[test]
    fn e1c3_missing_reconciliation_is_catalog_unavailable() {
        let mut conn = migrated_db();
        let source = upsert_connection(&mut conn, connection("conn-local")).expect("connection");
        let personal = upsert_library(
            &mut conn,
            library(&source.id, CatalogLibraryType::User, "0"),
        )
        .expect("personal library");
        upsert_item(&mut conn, &personal.id, rich_item("DETAIL1", 3)).expect("item");

        let detail = item_detail(&conn, &Library::user("0"), "DETAIL1").expect("read detail");

        assert_eq!(detail, ItemDetail::CatalogUnavailable);
    }

    /// A database without the bibliography tables has no catalog to confirm:
    /// unavailable, never an error.
    #[test]
    fn e1c3_absent_tables_are_catalog_unavailable() {
        let conn = Connection::open_in_memory().expect("open synthetic database");

        let detail = item_detail(&conn, &Library::user("0"), "DETAIL1").expect("read detail");

        assert_eq!(detail, ItemDetail::CatalogUnavailable);
    }

    /// E1c-3 RED: a blank item key is invalid input, never a lookup.
    #[test]
    fn e1c3_blank_item_key_is_invalid() {
        let mut conn = migrated_db();
        confirmed_namespace(&mut conn, "DETAIL1");

        let error = item_detail(&conn, &Library::user("0"), "   ").expect_err("blank key");

        assert_eq!(error.code, "invalid_input");
    }

    /// TRIANGULATE: a seen item whose snapshot version no longer matches the
    /// reconciled remote version is not confirmed.
    #[test]
    fn e1c3_version_mismatch_is_not_in_catalog() {
        let mut conn = migrated_db();
        confirmed_namespace(&mut conn, "DETAIL1");
        let source =
            upsert_connection(&mut conn, connection("conn-local")).expect("re-read connection");
        let personal = upsert_library(
            &mut conn,
            library(&source.id, CatalogLibraryType::User, "0"),
        )
        .expect("re-read library");
        // Same key, newer local snapshot the completed run never saw.
        let mut newer = rich_item("DETAIL1", 99);
        newer.creators_json = None;
        upsert_item(&mut conn, &personal.id, newer).expect("newer snapshot");
        conn.execute(
            "UPDATE bibliographic_items SET verified_at = 1 WHERE item_key = 'DETAIL1'",
            [],
        )
        .expect("keep verification old so only the version disqualifies");

        let detail = item_detail(&conn, &Library::user("0"), "DETAIL1").expect("read detail");

        assert_eq!(detail, ItemDetail::NotInCatalog);
    }

    /// TRIANGULATE: a snapshot verified after the completion cannot be
    /// confirmed by that run.
    #[test]
    fn e1c3_late_verification_is_not_in_catalog() {
        let mut conn = migrated_db();
        confirmed_namespace(&mut conn, "DETAIL1");
        let completed_at: i64 = conn
            .query_row(
                "SELECT completed_at FROM zotero_reconciliation_runs",
                [],
                |row| row.get(0),
            )
            .expect("completion timestamp");
        conn.execute(
            "UPDATE bibliographic_items SET verified_at = ?1 WHERE item_key = 'DETAIL1'",
            [completed_at + 1000],
        )
        .expect("make verification newer than the completion");

        let detail = item_detail(&conn, &Library::user("0"), "DETAIL1").expect("read detail");

        assert_eq!(detail, ItemDetail::NotInCatalog);
    }

    /// TRIANGULATE: a live snapshot the completed run never saw is not
    /// confirmed, even though the namespace is.
    #[test]
    fn e1c3_unseen_live_snapshot_is_not_in_catalog() {
        let mut conn = migrated_db();
        confirmed_namespace(&mut conn, "DETAIL1");
        let source =
            upsert_connection(&mut conn, connection("conn-local")).expect("re-read connection");
        let personal = upsert_library(
            &mut conn,
            library(&source.id, CatalogLibraryType::User, "0"),
        )
        .expect("re-read library");
        upsert_item(&mut conn, &personal.id, rich_item("UNSEEN", 1)).expect("unseen item");

        let detail = item_detail(&conn, &Library::user("0"), "UNSEEN").expect("read detail");

        assert_eq!(detail, ItemDetail::NotInCatalog);
    }

    /// TRIANGULATE: without a completed reconciliation even a tombstoned item
    /// reads unavailable — the namespace cannot be trusted yet.
    #[test]
    fn e1c3_tombstone_without_reconciliation_is_catalog_unavailable() {
        let mut conn = migrated_db();
        let source = upsert_connection(&mut conn, connection("conn-local")).expect("connection");
        let personal = upsert_library(
            &mut conn,
            library(&source.id, CatalogLibraryType::User, "0"),
        )
        .expect("personal library");
        upsert_item(&mut conn, &personal.id, rich_item("DETAIL1", 3)).expect("item");
        tombstone_item(
            &mut conn,
            &personal.id,
            "DETAIL1",
            TombstoneInput {
                remote_version: Some(4),
                reason: "remote deletion page".to_string(),
            },
        )
        .expect("tombstone item");

        let detail = item_detail(&conn, &Library::user("0"), "DETAIL1").expect("read detail");

        assert_eq!(detail, ItemDetail::CatalogUnavailable);
    }

    /// TRIANGULATE: a seen item without an eligible version is not confirmed.
    #[test]
    fn e1c3_null_version_is_not_in_catalog() {
        let mut conn = migrated_db();
        let source = upsert_connection(&mut conn, connection("conn-local")).expect("connection");
        let personal = upsert_library(
            &mut conn,
            library(&source.id, CatalogLibraryType::User, "0"),
        )
        .expect("personal library");
        upsert_item(
            &mut conn,
            &personal.id,
            BibliographicItemInput {
                item_key: "UNVERSIONED".to_string(),
                item_version: None,
                native_json_snapshot: r#"{"key":"UNVERSIONED"}"#.to_string(),
                csl_json_snapshot: r#"{"id":"csl-unversioned","type":"book"}"#.to_string(),
                ..Default::default()
            },
        )
        .expect("unversioned item");
        complete_reconciliation(
            &mut conn,
            &personal.id,
            source.revision,
            1,
            vec![seen_item("UNVERSIONED", None, 1)],
        );

        let detail = item_detail(&conn, &Library::user("0"), "UNVERSIONED").expect("read detail");

        assert_eq!(detail, ItemDetail::NotInCatalog);
    }

    /// The wire shape the UI half will consume: tagged status, camelCase
    /// detail, `abstract` for the abstract, tombstone with observedAt.
    #[test]
    fn e1c3_wire_shape_is_tagged_camel_case() {
        let confirmed = serde_json::to_value(ItemDetail::Confirmed {
            verified_at: 42,
            item: ItemDetailItem {
                item_key: "DETAIL1".to_string(),
                item_type: Some("book".to_string()),
                title: Some("Synthetic work".to_string()),
                creators: Some(vec![Creator {
                    creator_type: Some("author".to_string()),
                    first_name: Some("Ada".to_string()),
                    last_name: Some("Lovelace".to_string()),
                    name: None,
                }]),
                publication_title: None,
                publisher: None,
                date: None,
                doi: None,
                isbn: None,
                abstract_text: Some("A synthetic abstract.".to_string()),
                language: None,
                url: None,
                item_version: Some(3),
                collections: vec!["Alpha collection".to_string()],
                tags: vec![],
                attachments: vec![AttachmentDetail {
                    attachment_key: "PDF01".to_string(),
                    content_type: Some("application/pdf".to_string()),
                    link_mode: None,
                    filename: None,
                    url: None,
                }],
            },
        })
        .expect("serialize confirmed");
        assert_eq!(
            confirmed,
            serde_json::json!({
                "status": "confirmed",
                "verifiedAt": 42,
                "item": {
                    "itemKey": "DETAIL1",
                    "itemType": "book",
                    "title": "Synthetic work",
                    "creators": [
                        { "creatorType": "author", "firstName": "Ada", "lastName": "Lovelace" }
                    ],
                    "publicationTitle": null,
                    "publisher": null,
                    "date": null,
                    "doi": null,
                    "isbn": null,
                    "abstract": "A synthetic abstract.",
                    "language": null,
                    "url": null,
                    "itemVersion": 3,
                    "collections": ["Alpha collection"],
                    "tags": [],
                    "attachments": [
                        {
                            "attachmentKey": "PDF01",
                            "contentType": "application/pdf",
                            "linkMode": null,
                            "filename": null,
                            "url": null,
                        }
                    ],
                }
            })
        );

        let lost = serde_json::to_value(ItemDetail::LostLink {
            tombstone: TombstoneRecord {
                observed_at: 7,
                remote_version: Some(4),
                reason: "remote deletion page".to_string(),
            },
            item: None,
        })
        .expect("serialize lost_link");
        assert_eq!(
            lost,
            serde_json::json!({
                "status": "lost_link",
                "tombstone": {
                    "observedAt": 7,
                    "remoteVersion": 4,
                    "reason": "remote deletion page",
                },
                "item": null,
            })
        );

        assert_eq!(
            serde_json::to_value(ItemDetail::NotInCatalog).expect("serialize not_in_catalog"),
            serde_json::json!({ "status": "not_in_catalog" })
        );
        assert_eq!(
            serde_json::to_value(ItemDetail::CatalogUnavailable)
                .expect("serialize catalog_unavailable"),
            serde_json::json!({ "status": "catalog_unavailable" })
        );
    }
}
