//! Persistence contract for the E1b-1b bibliography catalog relations.
//!
//! These tests use only synthetic rows. They exercise the public repository seam
//! rather than private SQL helpers, while both catalog migrations are also
//! applied twice to prove that the checked-in schema is safe to replay.

use entropia_desktop_lib::bibliography::repository::{
    tombstone_attachment, tombstone_collection, tombstone_item, tombstone_tag,
    untombstone_attachment, untombstone_collection, untombstone_item, untombstone_tag,
    upsert_attachment, upsert_collection, upsert_connection, upsert_item, upsert_item_collection,
    upsert_item_tag, upsert_library, upsert_tag, AttachmentInput, BibliographicItemInput,
    CollectionInput, ItemCollectionInput, ItemTagInput, LibraryType, SourceOrigin, TagInput,
    TombstoneInput, UpsertConnection, UpsertLibrary,
};
use rusqlite::Connection;

const MIGRATION_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0038_bibliography_catalog.sql");
const RELATIONS_MIGRATION_SQL: &str =
    include_str!("../../../../packages/store/src/migrations/0039_bibliography_relations.sql");
const MIGRATION_NAME: &str = "0038_bibliography_catalog";
const RELATIONS_MIGRATION_NAME: &str = "0039_bibliography_relations";

fn migrated_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory database");
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE _migrations (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             name TEXT NOT NULL UNIQUE,
             applied_at INTEGER NOT NULL
         );",
    )
    .expect("tracking table");

    for _ in 0..2 {
        conn.execute_batch(&format!("BEGIN IMMEDIATE;\n{MIGRATION_SQL}\nCOMMIT;"))
            .expect("apply bibliography foundation migration");
        conn.execute_batch(&format!(
            "BEGIN IMMEDIATE;\n{RELATIONS_MIGRATION_SQL}\nCOMMIT;"
        ))
        .expect("apply bibliography relations migration");
    }
    for name in [MIGRATION_NAME, RELATIONS_MIGRATION_NAME] {
        conn.execute(
            "INSERT OR IGNORE INTO _migrations (name, applied_at) VALUES (?1, 1)",
            [name],
        )
        .expect("record migration");
    }
    conn
}

fn connection(id: &str, instance: Option<&str>) -> UpsertConnection {
    UpsertConnection {
        id: id.to_string(),
        source_origin: SourceOrigin::Local,
        source_instance_id: instance.map(str::to_string),
        endpoint: Some("http://synthetic.invalid".to_string()),
        capabilities_json: r#"{"read":true}"#.to_string(),
    }
}

fn library(connection_id: &str, library_type: LibraryType, library_id: &str) -> UpsertLibrary {
    UpsertLibrary {
        connection_id: connection_id.to_string(),
        library_type,
        library_id: library_id.to_string(),
        name: format!("Synthetic {library_id}"),
        last_modified_version: Some(7),
    }
}

fn item(key: &str, version: i64, native: &str, csl: &str) -> BibliographicItemInput {
    BibliographicItemInput {
        item_key: key.to_string(),
        item_version: Some(version),
        native_json_snapshot: native.to_string(),
        csl_json_snapshot: csl.to_string(),
        item_type: Some("book".to_string()),
        title: Some("Synthetic work".to_string()),
        ..Default::default()
    }
}

fn collection(key: &str, name: &str, parent: Option<&str>) -> CollectionInput {
    CollectionInput {
        collection_key: key.to_string(),
        name: name.to_string(),
        parent_collection_key: parent.map(str::to_string),
        native_json_snapshot: format!(r#"{{"key":"{key}","name":"{name}"}}"#),
        native_version: Some(1),
    }
}

fn tag(text: &str, tag_type: Option<&str>) -> TagInput {
    TagInput {
        tag_text: text.to_string(),
        tag_type: tag_type.map(str::to_string),
        native_json_snapshot: format!(r#"{{"tag":"{text}"}}"#),
        native_version: Some(1),
    }
}

fn attachment(key: &str) -> AttachmentInput {
    AttachmentInput {
        attachment_key: key.to_string(),
        content_type: Some("application/pdf".to_string()),
        link_mode: Some("linked_file".to_string()),
        filename: Some("raw-name.pdf".to_string()),
        native_path: Some(r"C:\\fixture\\raw-name.pdf".to_string()),
        url: Some("https://synthetic.invalid/raw-name.pdf".to_string()),
        md5: Some("raw-md5".to_string()),
        mtime: Some(1_700_000_000_123),
        native_json_snapshot: format!(r#"{{"key":"{key}"}}"#),
        native_version: Some(1),
    }
}

#[test]
fn catalog_migration_is_idempotent_and_records_all_catalog_tables() {
    let conn = migrated_db();

    for table in [
        "zotero_connections",
        "zotero_libraries",
        "bibliographic_items",
        "zotero_collections",
        "zotero_tags",
        "zotero_attachments",
        "zotero_item_collections",
        "zotero_item_tags",
        "zotero_item_tombstones",
        "zotero_collection_tombstones",
        "zotero_tag_tombstones",
        "zotero_attachment_tombstones",
    ] {
        let present: Option<String> = conn
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(present.as_deref(), Some(table), "missing table {table}");
    }

    let foreign_keys: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('bibliographic_items')
             WHERE \"table\" = 'zotero_libraries'",
            [],
            |row| row.get(0),
        )
        .expect("foreign-key metadata");
    assert_eq!(foreign_keys, 1);
}

#[test]
fn the_same_native_key_in_different_libraries_stays_distinct() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let personal = upsert_library(&mut conn, library(&source.id, LibraryType::User, "0"))
        .expect("personal library");
    let group = upsert_library(
        &mut conn,
        library(&source.id, LibraryType::Group, "6680944"),
    )
    .expect("group library");

    let first = upsert_item(
        &mut conn,
        &personal.id,
        item(
            "SAMEKEY1",
            1,
            r#"{"key":"SAMEKEY1","version":1}"#,
            r#"{"id":"personal-csl","type":"book"}"#,
        ),
    )
    .expect("personal item");
    let second = upsert_item(
        &mut conn,
        &group.id,
        item(
            "SAMEKEY1",
            1,
            r#"{"key":"SAMEKEY1","version":1}"#,
            r#"{"id":"group-csl","type":"book"}"#,
        ),
    )
    .expect("group item");

    assert_ne!(first.id, second.id);
    assert_eq!(first.identity.library_id, "0");
    assert_eq!(second.identity.library_id, "6680944");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE item_key='SAMEKEY1'",
            [],
            |row| row.get(0),
        )
        .expect("count items");
    assert_eq!(count, 2);
}

#[test]
fn native_and_csl_snapshots_round_trip_without_rewriting_identity() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let library =
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0")).expect("library");
    let native = r#"{"key":"ROUNDTRP","version":9,"data":{"title":"Synthetic"}}"#;
    let csl = r#"{"id":"different-csl-id","type":"book","title":"Synthetic"}"#;

    let saved =
        upsert_item(&mut conn, &library.id, item("ROUNDTRP", 9, native, csl)).expect("item");

    assert_eq!(saved.identity.source_origin, SourceOrigin::Local);
    assert_eq!(saved.identity.source_instance_id, None);
    assert_eq!(saved.identity.library_type, LibraryType::User);
    assert_eq!(saved.identity.library_id, "0");
    assert_eq!(saved.identity.item_key, "ROUNDTRP");
    assert_eq!(saved.native_json_snapshot, native);
    assert_eq!(saved.csl_json_snapshot, csl);
    assert!(saved.created_at > 0);
    assert!(saved.updated_at >= saved.created_at);
}

#[test]
fn malformed_creators_json_is_rejected() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let zotero_library =
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0")).expect("library");
    let mut malformed = item(
        "BADJSON1",
        1,
        r#"{"key":"BADJSON1","version":1}"#,
        r#"{"id":"bad-json-csl","type":"book"}"#,
    );
    malformed.creators_json = Some(r#"[{"name":"Ada"}"#.to_string());

    let error = upsert_item(&mut conn, &zotero_library.id, malformed)
        .expect_err("malformed optional creators_json must be rejected");

    assert_eq!(error.code, "invalid_json");
}

#[test]
fn repeated_upsert_updates_one_row_in_place() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", Some("fixture-instance")))
        .expect("connection");
    let zotero_library =
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0")).expect("library");
    assert_eq!(
        upsert_connection(&mut conn, connection("conn-1", Some("fixture-instance")))
            .expect("repeated connection upsert"),
        source
    );
    assert_eq!(
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0"))
            .expect("repeated library upsert"),
        zotero_library
    );

    let first = upsert_item(
        &mut conn,
        &zotero_library.id,
        item(
            "REPEAT01",
            1,
            r#"{"key":"REPEAT01","version":1}"#,
            r#"{"id":"repeat-csl-v1","type":"book"}"#,
        ),
    )
    .expect("first upsert");
    let second = upsert_item(
        &mut conn,
        &zotero_library.id,
        item(
            "REPEAT01",
            2,
            r#"{"key":"REPEAT01","version":2}"#,
            r#"{"id":"repeat-csl-v2","type":"book"}"#,
        ),
    )
    .expect("second upsert");
    let third = upsert_item(
        &mut conn,
        &zotero_library.id,
        item(
            "REPEAT01",
            2,
            r#"{"key":"REPEAT01","version":2}"#,
            r#"{"id":"repeat-csl-v2","type":"book"}"#,
        ),
    )
    .expect("unchanged repeated upsert");

    assert_eq!(first.id, second.id);
    assert_eq!(second, third);
    assert_eq!(second.item_version, Some(2));
    assert_eq!(second.revision, first.revision + 1);
    assert_eq!(
        second.identity.source_instance_id.as_deref(),
        Some("fixture-instance")
    );
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_items WHERE library_id=?1 AND item_key=?2",
            rusqlite::params![zotero_library.id, "REPEAT01"],
            |row| row.get(0),
        )
        .expect("count rows");
    assert_eq!(count, 1);
}

#[test]
fn collections_preserve_roots_opaque_parents_and_library_identity() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let personal = upsert_library(&mut conn, library(&source.id, LibraryType::User, "0"))
        .expect("personal library");
    let group = upsert_library(
        &mut conn,
        library(&source.id, LibraryType::Group, "6680944"),
    )
    .expect("group library");

    let root = upsert_collection(&mut conn, &personal.id, collection("ROOT", "Root", None))
        .expect("root collection");
    let child = upsert_collection(
        &mut conn,
        &personal.id,
        collection("CHILD", "Child", Some("parent-not-yet-seen")),
    )
    .expect("opaque-parent collection");
    let same_key_in_group =
        upsert_collection(&mut conn, &group.id, collection("ROOT", "Group root", None))
            .expect("group collection");

    assert_eq!(root.parent_collection_key, None);
    assert_eq!(
        child.parent_collection_key.as_deref(),
        Some("parent-not-yet-seen")
    );
    assert_ne!(root.id, same_key_in_group.id);
    assert_ne!(personal.id, group.id);
    assert_eq!(root.collection_key, "ROOT");
    assert_eq!(same_key_in_group.collection_key, "ROOT");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_collections WHERE collection_key='ROOT'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("collection count"),
        2
    );
}

#[test]
fn tags_keep_exact_raw_identity_and_nullable_type() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let personal =
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0")).expect("library");

    let first = upsert_tag(&mut conn, &personal.id, tag("  Raw Tag  ", None)).expect("raw tag");
    let repeated = upsert_tag(&mut conn, &personal.id, tag("  Raw Tag  ", Some("native")))
        .expect("tag type refresh");
    let case_variant = upsert_tag(&mut conn, &personal.id, tag("  raw tag  ", Some("native")))
        .expect("case variant tag");

    assert_eq!(first.id, repeated.id);
    assert_eq!(repeated.tag_text, "  Raw Tag  ");
    assert_eq!(repeated.tag_type.as_deref(), Some("native"));
    assert_ne!(repeated.id, case_variant.id);
    assert_eq!(case_variant.tag_text, "  raw tag  ");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_tags WHERE library_id=?1",
            [&personal.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("tag count"),
        2
    );
}

#[test]
fn attachments_preserve_nullable_raw_metadata_and_require_a_parent_item() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let personal =
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0")).expect("library");
    let parent = upsert_item(
        &mut conn,
        &personal.id,
        item(
            "ATTACH01",
            1,
            r#"{"key":"ATTACH01"}"#,
            r#"{"id":"attach-csl","type":"book"}"#,
        ),
    )
    .expect("parent item");

    let saved = upsert_attachment(&mut conn, &parent.id, attachment("PDF01")).expect("attachment");
    let nulls = upsert_attachment(
        &mut conn,
        &parent.id,
        AttachmentInput {
            attachment_key: "NULL01".to_string(),
            content_type: None,
            link_mode: None,
            filename: None,
            native_path: None,
            url: None,
            md5: None,
            mtime: None,
            native_json_snapshot: r#"{"key":"NULL01"}"#.to_string(),
            native_version: None,
        },
    )
    .expect("nullable attachment metadata");
    let error = upsert_attachment(&mut conn, "missing-parent-item", attachment("ORPHAN"))
        .expect_err("attachment parent must be mandatory");

    assert_eq!(saved.parent_item_id, parent.id);
    assert_eq!(saved.content_type.as_deref(), Some("application/pdf"));
    assert_eq!(saved.link_mode.as_deref(), Some("linked_file"));
    assert_eq!(saved.filename.as_deref(), Some("raw-name.pdf"));
    assert_eq!(
        saved.native_path.as_deref(),
        Some(r"C:\\fixture\\raw-name.pdf")
    );
    assert_eq!(
        saved.url.as_deref(),
        Some("https://synthetic.invalid/raw-name.pdf")
    );
    assert_eq!(saved.md5.as_deref(), Some("raw-md5"));
    assert_eq!(saved.mtime, Some(1_700_000_000_123));
    assert_eq!(nulls.content_type, None);
    assert_eq!(nulls.native_path, None);
    assert_eq!(nulls.native_json_snapshot, r#"{"key":"NULL01"}"#);
    assert_eq!(nulls.native_version, None);
    assert_eq!(error.code, "sql_error");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_attachments WHERE item_id=?1",
            [&parent.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("attachment count"),
        2
    );
}

#[test]
fn memberships_are_idempotent_and_reject_cross_library_edges() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let personal = upsert_library(&mut conn, library(&source.id, LibraryType::User, "0"))
        .expect("personal library");
    let group = upsert_library(
        &mut conn,
        library(&source.id, LibraryType::Group, "6680944"),
    )
    .expect("group library");
    let personal_item = upsert_item(
        &mut conn,
        &personal.id,
        item(
            "MEMBER01",
            1,
            r#"{"key":"MEMBER01"}"#,
            r#"{"id":"member-personal","type":"book"}"#,
        ),
    )
    .expect("personal item");
    let group_item = upsert_item(
        &mut conn,
        &group.id,
        item(
            "MEMBER01",
            1,
            r#"{"key":"MEMBER01"}"#,
            r#"{"id":"member-group","type":"book"}"#,
        ),
    )
    .expect("group item");
    let personal_collection = upsert_collection(
        &mut conn,
        &personal.id,
        collection("COLL01", "Personal", None),
    )
    .expect("personal collection");
    let group_collection =
        upsert_collection(&mut conn, &group.id, collection("COLL01", "Group", None))
            .expect("group collection");
    let personal_tag =
        upsert_tag(&mut conn, &personal.id, tag("Shared raw", None)).expect("personal tag");
    let group_tag = upsert_tag(&mut conn, &group.id, tag("Shared raw", None)).expect("group tag");

    let collection_edge = upsert_item_collection(
        &mut conn,
        &personal.id,
        ItemCollectionInput {
            item_id: personal_item.id.clone(),
            collection_id: personal_collection.id.clone(),
        },
    )
    .expect("item collection edge");
    let repeated_collection_edge = upsert_item_collection(
        &mut conn,
        &personal.id,
        ItemCollectionInput {
            item_id: personal_item.id.clone(),
            collection_id: personal_collection.id.clone(),
        },
    )
    .expect("repeated item collection edge");
    let tag_edge = upsert_item_tag(
        &mut conn,
        &personal.id,
        ItemTagInput {
            item_id: personal_item.id.clone(),
            tag_id: personal_tag.id.clone(),
        },
    )
    .expect("item tag edge");
    let repeated_tag_edge = upsert_item_tag(
        &mut conn,
        &personal.id,
        ItemTagInput {
            item_id: personal_item.id.clone(),
            tag_id: personal_tag.id.clone(),
        },
    )
    .expect("repeated item tag edge");

    let cross_library_collection = upsert_item_collection(
        &mut conn,
        &personal.id,
        ItemCollectionInput {
            item_id: group_item.id.clone(),
            collection_id: personal_collection.id.clone(),
        },
    )
    .expect_err("cross-library item/collection edge must fail");
    let cross_library_tag = upsert_item_tag(
        &mut conn,
        &personal.id,
        ItemTagInput {
            item_id: personal_item.id.clone(),
            tag_id: group_tag.id.clone(),
        },
    )
    .expect_err("cross-library item/tag edge must fail");
    let cross_library_collection_owner = upsert_item_collection(
        &mut conn,
        &group.id,
        ItemCollectionInput {
            item_id: group_item.id,
            collection_id: personal_collection.id.clone(),
        },
    )
    .expect_err("cross-library collection owner must fail");

    assert_eq!(collection_edge, repeated_collection_edge);
    assert_ne!(personal_collection.id, group_collection.id);
    assert_ne!(personal_tag.id, group_tag.id);
    assert_eq!(tag_edge, repeated_tag_edge);
    assert_eq!(cross_library_collection.code, "sql_error");
    assert_eq!(cross_library_tag.code, "sql_error");
    assert_eq!(cross_library_collection_owner.code, "sql_error");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM zotero_item_collections", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("collection edge count"),
        1
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM zotero_item_tags", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("tag edge count"),
        1
    );
}

#[test]
fn explicit_tombstones_preserve_rows_and_live_upserts_revive_every_entity() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let personal =
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0")).expect("library");
    let saved_item = upsert_item(
        &mut conn,
        &personal.id,
        item(
            "TOMB01",
            4,
            r#"{"key":"TOMB01","version":4,"title":"Before"}"#,
            r#"{"id":"tomb-csl","type":"book","title":"Before"}"#,
        ),
    )
    .expect("item");
    let saved_collection = upsert_collection(
        &mut conn,
        &personal.id,
        collection("TOMB-COLL", "Tombstone collection", Some("opaque-parent")),
    )
    .expect("collection");
    let saved_tag = upsert_tag(&mut conn, &personal.id, tag("Tombstone tag", None)).expect("tag");
    let saved_attachment =
        upsert_attachment(&mut conn, &saved_item.id, attachment("TOMB-ATT")).expect("attachment");
    upsert_item_collection(
        &mut conn,
        &personal.id,
        ItemCollectionInput {
            item_id: saved_item.id.clone(),
            collection_id: saved_collection.id.clone(),
        },
    )
    .expect("membership");
    upsert_item_tag(
        &mut conn,
        &personal.id,
        ItemTagInput {
            item_id: saved_item.id.clone(),
            tag_id: saved_tag.id.clone(),
        },
    )
    .expect("tag membership");

    for table in [
        "zotero_item_tombstones",
        "zotero_collection_tombstones",
        "zotero_tag_tombstones",
        "zotero_attachment_tombstones",
    ] {
        assert_eq!(
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("initial tombstone count"),
            0,
            "upserts must not infer deletion for {table}"
        );
    }

    let invalid_reason = tombstone_item(
        &mut conn,
        &personal.id,
        "TOMB01",
        TombstoneInput {
            remote_version: None,
            reason: "   ".to_string(),
        },
    )
    .expect_err("tombstone reason must be non-empty");
    assert_eq!(invalid_reason.code, "invalid_input");

    let item_tombstone = tombstone_item(
        &mut conn,
        &personal.id,
        "TOMB01",
        TombstoneInput {
            remote_version: Some(8),
            reason: "remote deletion page".to_string(),
        },
    )
    .expect("item tombstone");
    let collection_tombstone = tombstone_collection(
        &mut conn,
        &personal.id,
        "TOMB-COLL",
        TombstoneInput {
            remote_version: Some(8),
            reason: "remote deletion page".to_string(),
        },
    )
    .expect("collection tombstone");
    let tag_tombstone = tombstone_tag(
        &mut conn,
        &personal.id,
        "Tombstone tag",
        TombstoneInput {
            remote_version: None,
            reason: "remote deletion page".to_string(),
        },
    )
    .expect("tag tombstone");
    let attachment_tombstone = tombstone_attachment(
        &mut conn,
        &saved_item.id,
        "TOMB-ATT",
        TombstoneInput {
            remote_version: Some(8),
            reason: "remote deletion page".to_string(),
        },
    )
    .expect("attachment tombstone");

    assert_eq!(
        item_tombstone.entity.native_json_snapshot,
        saved_item.native_json_snapshot
    );
    assert_eq!(
        item_tombstone.entity.csl_json_snapshot,
        saved_item.csl_json_snapshot
    );
    assert_eq!(item_tombstone.tombstone.remote_version, Some(8));
    assert_eq!(
        collection_tombstone.entity.parent_collection_key.as_deref(),
        Some("opaque-parent")
    );
    assert_eq!(tag_tombstone.entity.tag_text, "Tombstone tag");
    assert_eq!(
        attachment_tombstone.entity.native_path,
        saved_attachment.native_path
    );
    assert!(item_tombstone.tombstone.observed_at > 0);
    assert_eq!(item_tombstone.tombstone.reason, "remote deletion page");

    let repeated_item_tombstone = tombstone_item(
        &mut conn,
        &personal.id,
        "TOMB01",
        TombstoneInput {
            remote_version: Some(9),
            reason: "remote deletion page retry".to_string(),
        },
    )
    .expect("replayed item tombstone");
    assert_eq!(repeated_item_tombstone.entity.id, saved_item.id);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_item_tombstones WHERE item_id=?1",
            [&saved_item.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("one item tombstone"),
        1
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM zotero_item_collections", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("preserved collection membership"),
        1
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM zotero_item_tags", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("preserved tag membership"),
        1
    );

    let revived_item = upsert_item(
        &mut conn,
        &personal.id,
        item(
            "TOMB01",
            4,
            r#"{"key":"TOMB01","version":4,"title":"Before"}"#,
            r#"{"id":"tomb-csl","type":"book","title":"Before"}"#,
        ),
    )
    .expect("item revival");
    let revived_collection = upsert_collection(
        &mut conn,
        &personal.id,
        collection("TOMB-COLL", "Tombstone collection", Some("opaque-parent")),
    )
    .expect("collection revival");
    let revived_tag =
        upsert_tag(&mut conn, &personal.id, tag("Tombstone tag", None)).expect("tag revival");
    let revived_attachment = upsert_attachment(&mut conn, &saved_item.id, attachment("TOMB-ATT"))
        .expect("attachment revival");

    assert_eq!(revived_item.id, saved_item.id);
    assert_eq!(revived_collection.id, saved_collection.id);
    assert_eq!(revived_tag.id, saved_tag.id);
    assert_eq!(revived_attachment.id, saved_attachment.id);
    for table in [
        "zotero_item_tombstones",
        "zotero_collection_tombstones",
        "zotero_tag_tombstones",
        "zotero_attachment_tombstones",
    ] {
        assert_eq!(
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("revived tombstone count"),
            0,
            "live upsert must clear {table}"
        );
    }
}

#[test]
fn explicit_untombstone_is_a_replay_safe_noop_when_already_live() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let personal =
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0")).expect("library");
    let saved_item = upsert_item(
        &mut conn,
        &personal.id,
        item(
            "UNTOMB01",
            1,
            r#"{"key":"UNTOMB01"}"#,
            r#"{"id":"untomb-csl","type":"book"}"#,
        ),
    )
    .expect("item");
    let saved_collection = upsert_collection(
        &mut conn,
        &personal.id,
        collection("UNTOMB-COLL", "Collection", None),
    )
    .expect("collection");
    let saved_tag = upsert_tag(&mut conn, &personal.id, tag("Untombstone tag", None)).expect("tag");
    let saved_attachment =
        upsert_attachment(&mut conn, &saved_item.id, attachment("UNTOMB-ATT")).expect("attachment");

    tombstone_item(
        &mut conn,
        &personal.id,
        "UNTOMB01",
        TombstoneInput {
            remote_version: None,
            reason: "explicit test".to_string(),
        },
    )
    .expect("item tombstone");
    tombstone_collection(
        &mut conn,
        &personal.id,
        "UNTOMB-COLL",
        TombstoneInput {
            remote_version: None,
            reason: "explicit test".to_string(),
        },
    )
    .expect("collection tombstone");
    tombstone_tag(
        &mut conn,
        &personal.id,
        "Untombstone tag",
        TombstoneInput {
            remote_version: None,
            reason: "explicit test".to_string(),
        },
    )
    .expect("tag tombstone");
    tombstone_attachment(
        &mut conn,
        &saved_item.id,
        "UNTOMB-ATT",
        TombstoneInput {
            remote_version: None,
            reason: "explicit test".to_string(),
        },
    )
    .expect("attachment tombstone");

    assert_eq!(
        untombstone_item(&mut conn, &personal.id, "UNTOMB01").expect("item"),
        saved_item
    );
    assert_eq!(
        untombstone_collection(&mut conn, &personal.id, "UNTOMB-COLL")
            .expect("collection")
            .id,
        saved_collection.id
    );
    assert_eq!(
        untombstone_tag(&mut conn, &personal.id, "Untombstone tag")
            .expect("tag")
            .id,
        saved_tag.id
    );
    assert_eq!(
        untombstone_attachment(&mut conn, &saved_item.id, "UNTOMB-ATT")
            .expect("attachment")
            .id,
        saved_attachment.id
    );
    assert_eq!(
        untombstone_item(&mut conn, &personal.id, "UNTOMB01").expect("repeated item"),
        saved_item
    );
}

#[test]
fn relation_entities_round_trip_native_snapshots_and_versions_without_revision_noise() {
    let mut conn = migrated_db();
    let source = upsert_connection(&mut conn, connection("conn-1", None)).expect("connection");
    let personal =
        upsert_library(&mut conn, library(&source.id, LibraryType::User, "0")).expect("library");

    let collection_native_v1 = r#"{ "key": "LOSSLESS-COLL", "extra": [1, 2] }"#;
    let collection_v1 = CollectionInput {
        collection_key: "LOSSLESS-COLL".to_string(),
        name: "Lossless collection".to_string(),
        parent_collection_key: Some("opaque-parent".to_string()),
        native_json_snapshot: collection_native_v1.to_string(),
        native_version: Some(11),
    };
    let saved_collection =
        upsert_collection(&mut conn, &personal.id, collection_v1.clone()).expect("collection");
    assert_eq!(saved_collection.native_json_snapshot, collection_native_v1);
    assert_eq!(saved_collection.native_version, Some(11));

    let unchanged_collection =
        upsert_collection(&mut conn, &personal.id, collection_v1).expect("unchanged collection");
    assert_eq!(unchanged_collection.id, saved_collection.id);
    assert_eq!(unchanged_collection.revision, saved_collection.revision);

    let collection_native_v2 = r#"{ "key": "LOSSLESS-COLL", "extra": [1, 2, 3] }"#;
    let changed_collection = upsert_collection(
        &mut conn,
        &personal.id,
        CollectionInput {
            collection_key: "LOSSLESS-COLL".to_string(),
            name: "Lossless collection".to_string(),
            parent_collection_key: Some("opaque-parent".to_string()),
            native_json_snapshot: collection_native_v2.to_string(),
            native_version: Some(11),
        },
    )
    .expect("changed collection snapshot");
    assert_eq!(
        changed_collection.native_json_snapshot,
        collection_native_v2
    );
    assert_eq!(changed_collection.native_version, Some(11));
    assert_eq!(changed_collection.revision, saved_collection.revision + 1);

    let changed_collection_version = upsert_collection(
        &mut conn,
        &personal.id,
        CollectionInput {
            collection_key: "LOSSLESS-COLL".to_string(),
            name: "Lossless collection".to_string(),
            parent_collection_key: Some("opaque-parent".to_string()),
            native_json_snapshot: collection_native_v2.to_string(),
            native_version: Some(12),
        },
    )
    .expect("changed collection version");
    assert_eq!(changed_collection_version.native_version, Some(12));
    assert_eq!(
        changed_collection_version.revision,
        changed_collection.revision + 1
    );

    let collection_tombstone = tombstone_collection(
        &mut conn,
        &personal.id,
        "LOSSLESS-COLL",
        TombstoneInput {
            remote_version: Some(99),
            reason: "snapshot contract test".to_string(),
        },
    )
    .expect("collection tombstone");
    assert_eq!(collection_tombstone.entity.native_version, Some(12));
    assert_eq!(collection_tombstone.tombstone.remote_version, Some(99));
    let revived_collection = upsert_collection(
        &mut conn,
        &personal.id,
        CollectionInput {
            collection_key: "LOSSLESS-COLL".to_string(),
            name: "Lossless collection".to_string(),
            parent_collection_key: Some("opaque-parent".to_string()),
            native_json_snapshot: collection_native_v2.to_string(),
            native_version: Some(12),
        },
    )
    .expect("unchanged collection revival");
    assert_eq!(
        revived_collection.revision,
        changed_collection_version.revision
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_collection_tombstones WHERE collection_id=?1",
            [&saved_collection.id],
            |row| row.get::<_, i64>(0),
        )
        .expect("collection tombstone count"),
        0
    );

    let tag_native_v1 = r#"{ "tag": "  Raw Tag  ", "meta": {"kind": "native"} }"#;
    let tag_v1 = TagInput {
        tag_text: "  Raw Tag  ".to_string(),
        tag_type: None,
        native_json_snapshot: tag_native_v1.to_string(),
        native_version: Some(21),
    };
    let saved_tag = upsert_tag(&mut conn, &personal.id, tag_v1.clone()).expect("tag");
    assert_eq!(saved_tag.native_json_snapshot, tag_native_v1);
    assert_eq!(saved_tag.native_version, Some(21));
    let unchanged_tag = upsert_tag(&mut conn, &personal.id, tag_v1).expect("unchanged tag");
    assert_eq!(unchanged_tag.revision, saved_tag.revision);

    let changed_tag = upsert_tag(
        &mut conn,
        &personal.id,
        TagInput {
            tag_text: "  Raw Tag  ".to_string(),
            tag_type: None,
            native_json_snapshot: tag_native_v1.to_string(),
            native_version: Some(22),
        },
    )
    .expect("changed tag version");
    assert_eq!(changed_tag.native_version, Some(22));
    assert_eq!(changed_tag.revision, saved_tag.revision + 1);

    let parent = upsert_item(
        &mut conn,
        &personal.id,
        item(
            "LOSSLESS-PARENT",
            1,
            r#"{"key":"LOSSLESS-PARENT"}"#,
            r#"{"id":"lossless-parent-csl","type":"book"}"#,
        ),
    )
    .expect("parent item");
    let attachment_native_v1 = r#"{ "key": "LOSSLESS-ATT", "path": null }"#;
    let attachment_v1 = AttachmentInput {
        attachment_key: "LOSSLESS-ATT".to_string(),
        content_type: None,
        link_mode: Some("linked_file".to_string()),
        filename: None,
        native_path: None,
        url: None,
        md5: None,
        mtime: None,
        native_json_snapshot: attachment_native_v1.to_string(),
        native_version: Some(31),
    };
    let saved_attachment =
        upsert_attachment(&mut conn, &parent.id, attachment_v1.clone()).expect("attachment");
    assert_eq!(saved_attachment.native_json_snapshot, attachment_native_v1);
    assert_eq!(saved_attachment.native_version, Some(31));
    let unchanged_attachment =
        upsert_attachment(&mut conn, &parent.id, attachment_v1).expect("unchanged attachment");
    assert_eq!(unchanged_attachment.revision, saved_attachment.revision);

    let attachment_native_v2 = r#"{ "key": "LOSSLESS-ATT", "path": "opaque/path" }"#;
    let changed_attachment = upsert_attachment(
        &mut conn,
        &parent.id,
        AttachmentInput {
            attachment_key: "LOSSLESS-ATT".to_string(),
            content_type: None,
            link_mode: Some("linked_file".to_string()),
            filename: None,
            native_path: None,
            url: None,
            md5: None,
            mtime: None,
            native_json_snapshot: attachment_native_v2.to_string(),
            native_version: Some(31),
        },
    )
    .expect("changed attachment");
    assert_eq!(
        changed_attachment.native_json_snapshot,
        attachment_native_v2
    );
    assert_eq!(changed_attachment.native_version, Some(31));
    assert_eq!(changed_attachment.revision, saved_attachment.revision + 1);

    let bad_collection = upsert_collection(
        &mut conn,
        &personal.id,
        CollectionInput {
            collection_key: "BAD-COLLECTION-JSON".to_string(),
            name: "Rejected".to_string(),
            parent_collection_key: None,
            native_json_snapshot: "{".to_string(),
            native_version: None,
        },
    )
    .expect_err("malformed collection snapshot must be rejected");
    assert_eq!(bad_collection.code, "invalid_json");

    let bad_tag = upsert_tag(
        &mut conn,
        &personal.id,
        TagInput {
            tag_text: "BAD-TAG-JSON".to_string(),
            tag_type: None,
            native_json_snapshot: "{".to_string(),
            native_version: None,
        },
    )
    .expect_err("malformed tag snapshot must be rejected");
    assert_eq!(bad_tag.code, "invalid_json");

    let bad_attachment = upsert_attachment(
        &mut conn,
        &parent.id,
        AttachmentInput {
            attachment_key: "BAD-ATTACHMENT-JSON".to_string(),
            content_type: None,
            link_mode: None,
            filename: None,
            native_path: None,
            url: None,
            md5: None,
            mtime: None,
            native_json_snapshot: "{".to_string(),
            native_version: None,
        },
    )
    .expect_err("malformed attachment snapshot must be rejected");
    assert_eq!(bad_attachment.code, "invalid_json");

    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_collections WHERE collection_key=?1",
            ["BAD-COLLECTION-JSON"],
            |row| row.get::<_, i64>(0),
        )
        .expect("collection rejection count"),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_tags WHERE tag_text=?1",
            ["BAD-TAG-JSON"],
            |row| row.get::<_, i64>(0),
        )
        .expect("tag rejection count"),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM zotero_attachments WHERE attachment_key=?1",
            ["BAD-ATTACHMENT-JSON"],
            |row| row.get::<_, i64>(0),
        )
        .expect("attachment rejection count"),
        0
    );
}
