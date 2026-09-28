use rusqlite::{Connection, Transaction};
use serde_json::{json, Value};

use super::sync_envelope::{
    snapshot_document, AttachmentManifestV1, WritingEnvelopeV1, UNSUPPORTED_SYNC_ENVELOPE_VERSION,
};

const MIGRATION_SQL: &str =
    include_str!("../../../../../packages/store/src/migrations/0035_writing_workspace.sql");

fn migrated_connection() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory database");
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE collections (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               description TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE _migrations (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               name TEXT NOT NULL UNIQUE,
               applied_at INTEGER NOT NULL
             );",
        )
        .expect("minimal real migration prerequisites");
    connection
        .execute_batch(MIGRATION_SQL)
        .expect("apply real 0035 migration");
    connection
        .execute(
            "INSERT INTO _migrations (name, applied_at) VALUES ('0035_writing_workspace', 0)",
            [],
        )
        .expect("record migration");
    connection
}

fn seed_complete_document(tx: &Transaction<'_>) {
    tx.execute_batch(
        r#"
        INSERT INTO collections (id, name, created_at, updated_at) VALUES
          ('collection-z', 'Last', 1, 1),
          ('collection-a', 'First', 1, 1);

        INSERT INTO writing_documents
          (id, title, document_type, status, schema_version, current_content_json,
           revision, plain_text_cache, citation_style_id, citation_locale,
           bibliography_enabled, created_at, updated_at, last_opened_at)
        VALUES
          ('doc-1', 'Typed snapshot', 'article', 'archived', 7,
           '{"schemaVersion":7,"doc":{"type":"doc","content":[{"type":"writingImage","attrs":{"src":"writing-images/local-only.png"}}]}}',
           42, 'LOCAL_CACHE_SENTINEL', 'apa', 'es-AR', 0, 100, 200, 300);

        INSERT INTO writing_document_collections
          (document_id, collection_id, is_primary, created_at)
        VALUES
          ('doc-1', 'collection-z', 1, 20),
          ('doc-1', 'collection-a', 0, 10);

        INSERT INTO writing_document_citations
          (id, document_id, citation_node_id, collection_id, item_id, asset_id,
           page_number, start_char, end_char, source_region_json, quoted_text,
           source_text_hash, locator_json, metadata_snapshot_json, integrity_status,
           created_at, updated_at)
        VALUES
          ('corpus-z', 'doc-1', 'node-z', NULL, NULL, NULL,
           NULL, NULL, NULL, NULL, NULL, NULL, NULL,
           '{"title":"Minimal"}', 'unverifiable', 90, 91),
          ('corpus-a', 'doc-1', 'node-a', 'collection-a', 'item-1', 'asset-1',
           12, 30, 45, '{"y":2,"x":1}', 'quoted passage', 'source-hash',
           '{"label":"page","value":"12"}',
           '{"year":2024,"authors":["Ada","Grace"],"title":"Corpus work"}',
           'source_modified', 80, 81);

        INSERT INTO writing_zotero_citations
          (id, document_id, citation_node_id, citation_cluster_id, item_position,
           source_origin, source_instance_id, library_type, library_id, item_key,
           item_version, locator_type, locator, prefix, suffix, suppress_author,
           author_only, item_csl_json_snapshot, integrity_status, created_at, updated_at)
        VALUES
          ('zotero-z', 'doc-1', 'z-node-z', 'cluster-z', 4,
           'local', NULL, 'user', '9', 'ZZZZ9999', NULL, NULL, NULL, NULL, NULL,
           0, 0, '{"id":"ZZZZ9999","type":"book"}', 'zotero_unavailable', 70, 71),
          ('zotero-a', 'doc-1', 'z-node-a', 'cluster-a', 0,
           'web', 'https://zotero.example/users/7', 'group', '7', 'AAAA1111', 88,
           'page', '22-24', 'see ', ', ch. 2', 1, 0,
           '{"type":"article-journal","title":"Zotero work","id":"AAAA1111"}',
           'item_modified', 60, 61);

        INSERT INTO writing_document_versions
          (id, document_id, version_number, content_json, schema_version,
           document_settings_json, reason, content_hash, created_at)
        VALUES
          ('version-1', 'doc-1', 1, '{"text":"HISTORY_SENTINEL"}', 1, '{}',
           'checkpoint', 'history-hash', 50);

        INSERT INTO writing_provenance_events
          (id, document_id, version_id, origin_type, operation_type,
           source_reference_json, created_at)
        VALUES
          ('provenance-1', 'doc-1', 'version-1', 'manual', 'insert',
           '{"value":"PROVENANCE_SENTINEL"}', 51);
        "#,
    )
    .expect("seed complete writing fixture");
}

fn snapshot_fixture() -> WritingEnvelopeV1 {
    let mut connection = migrated_connection();
    let tx = connection.transaction().expect("caller transaction");
    seed_complete_document(&tx);
    snapshot_document(&tx, "doc-1").expect("snapshot uncommitted transaction state")
}

#[test]
fn snapshot_contains_complete_document_and_citation_fields() {
    let snapshot = snapshot_fixture();

    assert_eq!(
        serde_json::to_value(snapshot).expect("serialize snapshot"),
        json!({
            "id": "doc-1",
            "envelope_version": 1,
            "content_json": {
                "doc": {
                    "content": [{
                        "attrs": { "src": "writing-images/local-only.png" },
                        "type": "writingImage"
                    }],
                    "type": "doc"
                },
                "schemaVersion": 7
            },
            "title": "Typed snapshot",
            "type": "article",
            "status": "archived",
            "schema_version": 7,
            "settings": {
                "citation_style_id": "apa",
                "citation_locale": "es-AR",
                "bibliography_enabled": false
            },
            "collection_associations": [
                { "collection_id": "collection-a", "is_primary": false },
                { "collection_id": "collection-z", "is_primary": true }
            ],
            "citation_projections": {
                "corpus": [
                    {
                        "id": "corpus-a",
                        "citation_node_id": "node-a",
                        "collection_id": "collection-a",
                        "item_id": "item-1",
                        "asset_id": "asset-1",
                        "page_number": 12,
                        "start_char": 30,
                        "end_char": 45,
                        "source_region_json": { "x": 1, "y": 2 },
                        "quoted_text": "quoted passage",
                        "source_text_hash": "source-hash",
                        "locator_json": { "label": "page", "value": "12" },
                        "metadata_snapshot_json": {
                            "authors": ["Ada", "Grace"],
                            "title": "Corpus work",
                            "year": 2024
                        },
                        "integrity_status": "source_modified"
                    },
                    {
                        "id": "corpus-z",
                        "citation_node_id": "node-z",
                        "collection_id": null,
                        "item_id": null,
                        "asset_id": null,
                        "page_number": null,
                        "start_char": null,
                        "end_char": null,
                        "source_region_json": null,
                        "quoted_text": null,
                        "source_text_hash": null,
                        "locator_json": null,
                        "metadata_snapshot_json": { "title": "Minimal" },
                        "integrity_status": "unverifiable"
                    }
                ],
                "zotero": [
                    {
                        "id": "zotero-a",
                        "citation_node_id": "z-node-a",
                        "citation_cluster_id": "cluster-a",
                        "item_position": 0,
                        "source_origin": "web",
                        "source_instance_id": "https://zotero.example/users/7",
                        "library_type": "group",
                        "library_id": "7",
                        "item_key": "AAAA1111",
                        "item_version": 88,
                        "locator_type": "page",
                        "locator": "22-24",
                        "prefix": "see ",
                        "suffix": ", ch. 2",
                        "suppress_author": true,
                        "author_only": false,
                        "item_csl_json_snapshot": {
                            "id": "AAAA1111",
                            "title": "Zotero work",
                            "type": "article-journal"
                        },
                        "integrity_status": "item_modified"
                    },
                    {
                        "id": "zotero-z",
                        "citation_node_id": "z-node-z",
                        "citation_cluster_id": "cluster-z",
                        "item_position": 4,
                        "source_origin": "local",
                        "source_instance_id": null,
                        "library_type": "user",
                        "library_id": "9",
                        "item_key": "ZZZZ9999",
                        "item_version": null,
                        "locator_type": null,
                        "locator": null,
                        "prefix": null,
                        "suffix": null,
                        "suppress_author": false,
                        "author_only": false,
                        "item_csl_json_snapshot": { "id": "ZZZZ9999", "type": "book" },
                        "integrity_status": "zotero_unavailable"
                    }
                ]
            },
            "attachments_manifest": { "state": "preparation_required" }
        })
    );
}

#[test]
fn snapshot_excludes_local_revision_cache_history_and_provenance() {
    let serialized = snapshot_fixture()
        .to_canonical_json()
        .expect("canonical JSON");
    let value: Value = serde_json::from_str(&serialized).expect("snapshot JSON");
    let object = value.as_object().expect("envelope object");

    for excluded in [
        "revision",
        "plain_text_cache",
        "last_opened_at",
        "created_at",
        "updated_at",
        "journal",
        "history",
        "provenance",
    ] {
        assert!(
            !object.contains_key(excluded),
            "unexpected field {excluded}"
        );
    }
    assert!(!serialized.contains("LOCAL_CACHE_SENTINEL"));
    assert!(!serialized.contains("HISTORY_SENTINEL"));
    assert!(!serialized.contains("PROVENANCE_SENTINEL"));
    assert!(value["citation_projections"]["corpus"]
        .as_array()
        .expect("corpus citations")
        .iter()
        .all(
            |citation| citation.get("created_at").is_none() && citation.get("updated_at").is_none()
        ));
}

#[test]
fn canonical_roundtrip_and_fingerprint_ignore_aggregate_array_order() {
    let snapshot = snapshot_fixture();
    let canonical_json = snapshot.to_canonical_json().expect("canonical JSON");
    let canonical_fingerprint = snapshot.fingerprint_sha256().expect("fingerprint");

    let mut reordered = snapshot.clone();
    reordered.collection_associations.reverse();
    reordered.citation_projections.corpus.reverse();
    reordered.citation_projections.zotero.reverse();

    assert_eq!(
        reordered
            .fingerprint_sha256()
            .expect("reordered fingerprint"),
        canonical_fingerprint
    );
    assert_eq!(
        reordered.to_canonical_json().expect("reordered JSON"),
        canonical_json
    );
    let roundtrip = WritingEnvelopeV1::from_json(&canonical_json).expect("validated roundtrip");
    assert_eq!(
        roundtrip.to_canonical_json().expect("roundtrip JSON"),
        canonical_json
    );
}

#[test]
fn deserialization_rejects_an_unknown_envelope_version() {
    let mut value = serde_json::to_value(snapshot_fixture()).expect("snapshot value");
    value["envelope_version"] = json!(2);

    let error = WritingEnvelopeV1::from_json(&value.to_string()).expect_err("version 2 rejected");

    assert_eq!(error.code, UNSUPPORTED_SYNC_ENVELOPE_VERSION);
}

#[test]
fn image_bearing_snapshot_requires_attachment_preparation() {
    let snapshot = snapshot_fixture();
    let image_type = &snapshot.content_json["doc"]["content"][0]["type"];

    assert_eq!(image_type, "writingImage");
    assert_eq!(
        snapshot.attachments_manifest,
        AttachmentManifestV1::PreparationRequired
    );
    assert!(!snapshot.attachments_manifest.is_transfer_ready());
}
