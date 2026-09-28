use rusqlite::{params, Connection};
use serde_json::{json, Value};

use super::sync_conflict::{
    conflict_copy_document_id, preserve_conflict_copy, preserve_loser_then_receive_winner,
    ConflictCopyOutcome, ConflictSourceMetadata, ConflictThenReceiveOutcome,
    CONFLICT_COPY_COLLISION,
};
use super::sync_envelope::{
    snapshot_document, AttachmentFileV1, AttachmentManifestV1, CitationProjectionsV1,
    CollectionAssociationV1, CorpusCitationV1, DocumentSettingsV1, WritingEnvelopeV1,
    ZoteroCitationV1,
};
use super::sync_receive::{
    receive_envelope, AttachmentInstallReceipt, ReceiveAuthorization, ReceiveDeferred,
    ReceiveOutcome, ATTACHMENTS_NOT_READY, ATTACHMENT_RECEIPT_MISMATCH,
};

const MIGRATION_0035: &str =
    include_str!("../../../../../packages/store/src/migrations/0035_writing_workspace.sql");
const MIGRATION_0036: &str =
    include_str!("../../../../../packages/store/src/migrations/0036_writing_journal.sql");

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
        .execute_batch(&format!(
            "BEGIN IMMEDIATE;\n{MIGRATION_0035}\n{MIGRATION_0036}\n\
             INSERT INTO _migrations (name, applied_at) VALUES ('0035_writing_workspace', 0);\n\
             INSERT INTO _migrations (name, applied_at) VALUES ('0036_writing_journal', 0);\n\
             COMMIT;"
        ))
        .expect("apply real writing migrations");
    connection
        .execute_batch(
            "INSERT INTO collections (id, name, created_at, updated_at) VALUES
               ('collection-a', 'First', 1, 1),
               ('collection-b', 'Second', 1, 1);",
        )
        .expect("seed collections");
    connection
}

fn envelope(document_id: &str, text: &str) -> WritingEnvelopeV1 {
    WritingEnvelopeV1 {
        id: document_id.to_string(),
        envelope_version: 1,
        content_json: json!({
            "schemaVersion": 1,
            "doc": {
                "type": "doc",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": text }]
                    },
                    {
                        "type": "image",
                        "attrs": { "src": "writing-images/figure-1.png" }
                    }
                ]
            }
        }),
        title: "Shared manuscript".to_string(),
        document_type: "article".to_string(),
        status: "active".to_string(),
        schema_version: 1,
        settings: DocumentSettingsV1 {
            citation_style_id: Some("apa".to_string()),
            citation_locale: Some("es-AR".to_string()),
            bibliography_enabled: true,
        },
        collection_associations: vec![CollectionAssociationV1 {
            collection_id: "collection-a".to_string(),
            is_primary: true,
        }],
        citation_projections: CitationProjectionsV1 {
            corpus: vec![CorpusCitationV1 {
                id: "shared-corpus-id".to_string(),
                citation_node_id: "corpus-node-1".to_string(),
                collection_id: Some("collection-a".to_string()),
                item_id: Some("item-1".to_string()),
                asset_id: Some("asset-1".to_string()),
                page_number: Some(9),
                start_char: Some(12),
                end_char: Some(28),
                source_region_json: Some(json!({ "height": 20, "width": 10 })),
                quoted_text: Some("Quoted source".to_string()),
                source_text_hash: Some("source-hash".to_string()),
                locator_json: Some(json!({ "label": "page", "value": "9" })),
                metadata_snapshot_json: json!({
                    "authors": ["Ada"],
                    "title": "Corpus snapshot",
                    "year": 2025
                }),
                integrity_status: "source_modified".to_string(),
            }],
            zotero: vec![ZoteroCitationV1 {
                id: "shared-zotero-id".to_string(),
                citation_node_id: "zotero-node-1".to_string(),
                citation_cluster_id: "cluster-1".to_string(),
                item_position: 0,
                source_origin: "web".to_string(),
                source_instance_id: Some("https://zotero.example/users/7".to_string()),
                library_type: "group".to_string(),
                library_id: "7".to_string(),
                item_key: "ITEM0001".to_string(),
                item_version: Some(42),
                locator_type: Some("page".to_string()),
                locator: Some("10-11".to_string()),
                prefix: Some("see ".to_string()),
                suffix: Some(", section 2".to_string()),
                suppress_author: true,
                author_only: false,
                item_csl_json_snapshot: json!({
                    "id": "ITEM0001",
                    "title": "Zotero snapshot",
                    "type": "article-journal"
                }),
                integrity_status: "item_modified".to_string(),
            }],
        },
        attachments_manifest: AttachmentManifestV1::Validated {
            files: vec![AttachmentFileV1 {
                sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                rel_path: "writing-images/figure-1.png".to_string(),
                size: 3,
                media_type: "image/png".to_string(),
            }],
        },
    }
}

fn envelope_with_multiple_children(document_id: &str, text: &str) -> WritingEnvelopeV1 {
    let mut value = envelope(document_id, text);
    value.collection_associations.push(CollectionAssociationV1 {
        collection_id: "collection-b".to_string(),
        is_primary: false,
    });
    value.citation_projections.corpus.push(CorpusCitationV1 {
        id: "second-corpus-id".to_string(),
        citation_node_id: "corpus-node-2".to_string(),
        collection_id: Some("collection-b".to_string()),
        item_id: Some("item-2".to_string()),
        asset_id: Some("asset-2".to_string()),
        page_number: Some(2),
        start_char: Some(1),
        end_char: Some(8),
        source_region_json: Some(json!({ "x": 3, "y": 4 })),
        quoted_text: Some("Second quote".to_string()),
        source_text_hash: Some("second-source-hash".to_string()),
        locator_json: Some(json!({ "value": "2", "label": "page" })),
        metadata_snapshot_json: json!({ "title": "Second corpus snapshot" }),
        integrity_status: "valid".to_string(),
    });
    value.citation_projections.zotero.push(ZoteroCitationV1 {
        id: "second-zotero-id".to_string(),
        citation_node_id: "zotero-node-2".to_string(),
        citation_cluster_id: "cluster-2".to_string(),
        item_position: 0,
        source_origin: "local".to_string(),
        source_instance_id: None,
        library_type: "user".to_string(),
        library_id: "8".to_string(),
        item_key: "ITEM0002".to_string(),
        item_version: Some(7),
        locator_type: None,
        locator: None,
        prefix: None,
        suffix: None,
        suppress_author: false,
        author_only: true,
        item_csl_json_snapshot: json!({
            "id": "ITEM0002",
            "title": "Second Zotero snapshot",
            "type": "book"
        }),
        integrity_status: "valid".to_string(),
    });
    let AttachmentManifestV1::Validated { files } = &mut value.attachments_manifest else {
        unreachable!("fixture has validated attachments");
    };
    files.push(AttachmentFileV1 {
        sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        rel_path: "writing-images/figure-2.webp".to_string(),
        size: 11,
        media_type: "image/webp".to_string(),
    });
    value
}

fn receipt(envelope: &WritingEnvelopeV1) -> AttachmentInstallReceipt {
    AttachmentInstallReceipt::caller_asserts_installed(
        envelope.fingerprint_sha256().expect("envelope fingerprint"),
    )
}

fn source(document_id: &str, label: &str, captured_at: i64) -> ConflictSourceMetadata {
    ConflictSourceMetadata {
        source_document_id: document_id.to_string(),
        source_device_id: "device-a".to_string(),
        source_device_label: label.to_string(),
        source_captured_at_ms: captured_at,
    }
}

fn create(connection: &Connection, envelope: &WritingEnvelopeV1) {
    let outcome = receive_envelope(
        connection,
        &envelope.id,
        envelope,
        &ReceiveAuthorization::CreateOnly,
        &receipt(envelope),
    )
    .expect("create document through receiver");
    assert!(matches!(
        outcome,
        ReceiveOutcome::Applied {
            local_revision: 0,
            created: true,
            ..
        }
    ));
}

fn expected_state(connection: &Connection, document_id: &str) -> (i64, String) {
    let revision = connection
        .query_row(
            "SELECT revision FROM writing_documents WHERE id = ?1",
            [document_id],
            |row| row.get(0),
        )
        .expect("local revision");
    let fingerprint = snapshot_document(connection, document_id)
        .expect("local snapshot")
        .fingerprint_sha256()
        .expect("local fingerprint");
    (revision, fingerprint)
}

fn copy_id(envelope: &WritingEnvelopeV1) -> String {
    let semantic = envelope
        .conflict_semantic_fingerprint_sha256()
        .expect("semantic fingerprint");
    conflict_copy_document_id(&envelope.id, &semantic)
}

fn document_text(envelope: &WritingEnvelopeV1) -> &str {
    envelope.content_json["doc"]["content"][0]["content"][0]["text"]
        .as_str()
        .expect("fixture text")
}

fn count(connection: &Connection, table: &str, document_id: &str) -> i64 {
    connection
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE document_id = ?1"),
            [document_id],
            |row| row.get(0),
        )
        .expect("row count")
}

fn document_count(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM writing_documents", [], |row| {
            row.get(0)
        })
        .expect("document count")
}

#[test]
fn preserve_conflict_copy_keeps_both_texts_and_the_full_losing_aggregate() {
    let connection = migrated_connection();
    let original = envelope("doc-1", "Local text survives");
    create(&connection, &original);
    let mut losing = envelope("doc-1", "Losing remote text survives");
    losing.settings.citation_style_id = Some("chicago-author-date".to_string());
    losing.settings.bibliography_enabled = false;
    losing.citation_projections.corpus[0].quoted_text = Some("Complete quote".to_string());
    losing.citation_projections.zotero[0].suffix = Some(", complete".to_string());

    let outcome = preserve_conflict_copy(
        &connection,
        &losing,
        &source("doc-1", "Laptop", 100),
        &receipt(&losing),
    )
    .expect("preserve conflict copy");
    let ConflictCopyOutcome::Preserved {
        document_id,
        created: true,
        ..
    } = outcome
    else {
        panic!("first preservation must create a copy");
    };

    let original_after = snapshot_document(&connection, "doc-1").expect("original snapshot");
    let mut copy = snapshot_document(&connection, &document_id).expect("conflict snapshot");
    let conflict_title = copy.title.clone();
    copy.title = losing.title.clone();
    copy.attachments_manifest = losing.attachments_manifest.clone();

    assert_eq!(
        (
            document_text(&original_after),
            document_text(&copy),
            copy.conflict_semantic_fingerprint_sha256()
                .expect("copy semantic fingerprint"),
            connection
                .query_row(
                    "SELECT revision FROM writing_documents WHERE id = ?1",
                    [&document_id],
                    |row| row.get::<_, i64>(0),
                )
                .expect("copy revision"),
            conflict_title.contains("Conflict copy"),
        ),
        (
            "Local text survives",
            "Losing remote text survives",
            losing
                .conflict_semantic_fingerprint_sha256()
                .expect("loser semantic fingerprint"),
            0,
            true,
        )
    );
    assert_eq!(
        (
            copy.settings,
            copy.collection_associations,
            copy.citation_projections.corpus.len(),
            copy.citation_projections.zotero.len(),
        ),
        (
            losing.settings,
            losing.collection_associations,
            losing.citation_projections.corpus.len(),
            losing.citation_projections.zotero.len(),
        )
    );
}

#[test]
fn same_loser_replay_creates_one_copy_and_keeps_first_metadata() {
    let connection = migrated_connection();
    let losing = envelope("doc-1", "Same losing version");
    create(&connection, &losing);

    let first = preserve_conflict_copy(
        &connection,
        &losing,
        &source("doc-1", "First label", 100),
        &receipt(&losing),
    )
    .expect("first preservation");
    let second = preserve_conflict_copy(
        &connection,
        &losing,
        &source("doc-1", "Renamed device", 999),
        &receipt(&losing),
    )
    .expect("idempotent preservation");
    let copy_id = copy_id(&losing);
    let marker: String = connection
        .query_row(
            "SELECT source_reference_json FROM writing_provenance_events
              WHERE document_id = ?1 AND origin_type = 'import'",
            [&copy_id],
            |row| row.get(0),
        )
        .expect("origin marker");
    let marker: Value = serde_json::from_str(&marker).expect("marker json");

    assert!(matches!(
        first,
        ConflictCopyOutcome::Preserved { created: true, .. }
    ));
    assert!(matches!(
        second,
        ConflictCopyOutcome::Preserved { created: false, .. }
    ));
    assert_eq!(document_count(&connection), 2);
    assert_eq!(count(&connection, "writing_provenance_events", &copy_id), 1);
    assert_eq!(
        (
            marker["source_device_label"].as_str(),
            marker["source_captured_at_ms"].as_i64(),
        ),
        (Some("First label"), Some(100))
    );
}

#[test]
fn differing_losing_versions_create_distinct_conflict_documents() {
    let connection = migrated_connection();
    let first = envelope("doc-1", "Version one");
    let second = envelope("doc-1", "Version two");

    let first_outcome = preserve_conflict_copy(
        &connection,
        &first,
        &source("doc-1", "Laptop", 100),
        &receipt(&first),
    )
    .expect("first version");
    let second_outcome = preserve_conflict_copy(
        &connection,
        &second,
        &source("doc-1", "Laptop", 101),
        &receipt(&second),
    )
    .expect("second version");

    let (
        ConflictCopyOutcome::Preserved {
            document_id: first_id,
            ..
        },
        ConflictCopyOutcome::Preserved {
            document_id: second_id,
            ..
        },
    ) = (first_outcome, second_outcome)
    else {
        panic!("both versions must be preserved");
    };
    assert_ne!(first_id, second_id);
    assert_eq!(document_count(&connection), 2);
}

#[test]
fn reordered_aggregate_arrays_replay_as_one_semantic_conflict() {
    let connection = migrated_connection();
    let first = envelope_with_multiple_children("doc-1", "Order-independent");
    let mut reordered = first.clone();
    reordered.collection_associations.reverse();
    reordered.citation_projections.corpus.reverse();
    reordered.citation_projections.zotero.reverse();
    let AttachmentManifestV1::Validated { files } = &mut reordered.attachments_manifest else {
        unreachable!("fixture has validated attachments");
    };
    files.reverse();

    let first_outcome = preserve_conflict_copy(
        &connection,
        &first,
        &source("doc-1", "Laptop", 100),
        &receipt(&first),
    )
    .expect("first ordering");
    let replay = preserve_conflict_copy(
        &connection,
        &reordered,
        &source("doc-1", "Laptop", 101),
        &receipt(&reordered),
    )
    .expect("reordered replay");

    assert_eq!(
        (first_outcome, replay),
        (
            ConflictCopyOutcome::Preserved {
                document_id: copy_id(&first),
                semantic_fingerprint_sha256: first
                    .conflict_semantic_fingerprint_sha256()
                    .expect("semantic fingerprint"),
                created: true,
            },
            ConflictCopyOutcome::Preserved {
                document_id: copy_id(&first),
                semantic_fingerprint_sha256: first
                    .conflict_semantic_fingerprint_sha256()
                    .expect("semantic fingerprint"),
                created: false,
            },
        )
    );
}

#[test]
fn receive_remapped_child_ids_keep_the_same_conflict_identity() {
    let connection = migrated_connection();
    let blocker = envelope("blocker", "Owns incoming child ids");
    create(&connection, &blocker);
    let original_shape = envelope("doc-1", "Remapped source");
    create(&connection, &original_shape);
    let mut remapped = snapshot_document(&connection, "doc-1").expect("remapped snapshot");
    remapped.attachments_manifest = original_shape.attachments_manifest.clone();
    assert_ne!(
        remapped.citation_projections.corpus[0].id,
        original_shape.citation_projections.corpus[0].id
    );

    let first = preserve_conflict_copy(
        &connection,
        &original_shape,
        &source("doc-1", "Laptop", 100),
        &receipt(&original_shape),
    )
    .expect("original child ids");
    let replay = preserve_conflict_copy(
        &connection,
        &remapped,
        &source("doc-1", "Laptop", 101),
        &receipt(&remapped),
    )
    .expect("remapped child ids");

    assert!(matches!(
        first,
        ConflictCopyOutcome::Preserved { created: true, .. }
    ));
    assert!(matches!(
        replay,
        ConflictCopyOutcome::Preserved { created: false, .. }
    ));
    assert_eq!(document_count(&connection), 3);
}

#[test]
fn unrelated_document_at_the_derived_id_is_never_overwritten() {
    let connection = migrated_connection();
    let losing = envelope("doc-1", "Must not overwrite");
    let derived_id = copy_id(&losing);
    let unrelated = envelope(&derived_id, "Unrelated content");
    create(&connection, &unrelated);
    let before = snapshot_document(&connection, &derived_id).expect("unrelated before");

    let error = preserve_conflict_copy(
        &connection,
        &losing,
        &source("doc-1", "Laptop", 100),
        &receipt(&losing),
    )
    .expect_err("derived id collision");

    assert_eq!(error.code, CONFLICT_COPY_COLLISION);
    assert_eq!(
        snapshot_document(&connection, &derived_id).expect("unrelated after"),
        before
    );
    assert_eq!(
        count(&connection, "writing_provenance_events", &derived_id),
        0
    );
}

#[test]
fn changed_existing_conflict_content_is_not_silently_accepted_or_overwritten() {
    let connection = migrated_connection();
    let losing = envelope("doc-1", "Original conflict content");
    preserve_conflict_copy(
        &connection,
        &losing,
        &source("doc-1", "Laptop", 100),
        &receipt(&losing),
    )
    .expect("first preservation");
    let derived_id = copy_id(&losing);
    let changed = json!({
        "schemaVersion": 1,
        "doc": {
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "Locally changed copy" }]
            }]
        }
    });
    connection
        .execute(
            "UPDATE writing_documents SET current_content_json = ?1 WHERE id = ?2",
            params![changed.to_string(), &derived_id],
        )
        .expect("change conflict copy");

    let error = preserve_conflict_copy(
        &connection,
        &losing,
        &source("doc-1", "Laptop", 101),
        &receipt(&losing),
    )
    .expect_err("changed conflict copy must not be a no-op");

    assert_eq!(error.code, CONFLICT_COPY_COLLISION);
    assert_eq!(
        document_text(&snapshot_document(&connection, &derived_id).expect("changed copy")),
        "Locally changed copy"
    );
    assert_eq!(
        count(&connection, "writing_provenance_events", &derived_id),
        1
    );
}

#[test]
fn unprepared_attachments_are_rejected_without_any_row() {
    let connection = migrated_connection();
    let mut losing = envelope("doc-1", "Unprepared");
    losing.attachments_manifest = AttachmentManifestV1::PreparationRequired;
    let asserted = receipt(&losing);

    let error = preserve_conflict_copy(
        &connection,
        &losing,
        &source("doc-1", "Laptop", 100),
        &asserted,
    )
    .expect_err("unprepared attachments");

    assert_eq!(error.code, ATTACHMENTS_NOT_READY);
    assert_eq!(document_count(&connection), 0);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM writing_provenance_events",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .expect("provenance count"),
        0
    );
}

#[test]
fn mismatched_attachment_receipt_is_rejected_without_any_row() {
    let connection = migrated_connection();
    let losing = envelope("doc-1", "Receipt-bound");
    let mut other = losing.clone();
    other.title = "Different envelope".to_string();

    let error = preserve_conflict_copy(
        &connection,
        &losing,
        &source("doc-1", "Laptop", 100),
        &receipt(&other),
    )
    .expect_err("mismatched attachment receipt");

    assert_eq!(error.code, ATTACHMENT_RECEIPT_MISMATCH);
    assert_eq!(document_count(&connection), 0);
}

#[test]
fn source_deletion_keeps_the_copy_without_history_journal_or_suggestions() {
    let connection = migrated_connection();
    let original = envelope("doc-1", "Clean imported copy");
    create(&connection, &original);
    connection
        .execute_batch(
            "INSERT INTO writing_document_versions
               (id, document_id, version_number, content_json, schema_version,
                document_settings_json, reason, content_hash, created_at)
             VALUES ('version-1', 'doc-1', 1, '{}', 1, '{}', 'auto', 'hash', 1);
             INSERT INTO writing_journal
               (document_id, seq, base_revision, schema_version, delta_json, checksum, created_at)
             VALUES ('doc-1', 1, 0, 1, '[]', 'checksum', 1);
             INSERT INTO writing_agent_suggestions
               (id, document_id, source_revision, selected_content_hash, action_type,
                evidence_json, status, created_at)
             VALUES ('suggestion-1', 'doc-1', 0, 'selected', 'rewrite', '{}', 'pending', 1);
             INSERT INTO writing_provenance_events
               (id, document_id, origin_type, operation_type, created_at)
             VALUES ('source-event', 'doc-1', 'manual', 'insert', 1);",
        )
        .expect("seed local-only rows");

    preserve_conflict_copy(
        &connection,
        &original,
        &source("doc-1", "Laptop", 100),
        &receipt(&original),
    )
    .expect("preserve copy");
    let derived_id = copy_id(&original);
    connection
        .execute("DELETE FROM writing_documents WHERE id = 'doc-1'", [])
        .expect("delete source document");
    let copy = snapshot_document(&connection, &derived_id).expect("copy survives source deletion");

    assert_eq!(document_text(&copy), "Clean imported copy");
    assert_eq!(
        (
            count(&connection, "writing_document_versions", &derived_id),
            count(&connection, "writing_journal", &derived_id),
            count(&connection, "writing_agent_suggestions", &derived_id),
            count(&connection, "writing_provenance_events", &derived_id),
            copy.citation_projections.corpus.len(),
            copy.citation_projections.zotero.len(),
        ),
        (0, 0, 0, 1, 1, 1)
    );
}

#[test]
fn combined_operation_preserves_loser_before_applying_guarded_winner() {
    let connection = migrated_connection();
    let loser = envelope("doc-1", "Local losing text");
    create(&connection, &loser);
    let (revision, fingerprint) = expected_state(&connection, "doc-1");
    let winner = envelope("doc-1", "Remote winning text");

    let outcome = preserve_loser_then_receive_winner(
        &connection,
        &loser,
        &source("doc-1", "Laptop", 100),
        &receipt(&loser),
        &winner,
        revision,
        &fingerprint,
        &receipt(&winner),
    )
    .expect("combined conflict resolution");
    let ConflictThenReceiveOutcome::Applied {
        conflict_document_id,
        conflict_created: true,
        winner:
            ReceiveOutcome::Applied {
                local_revision: 1,
                created: false,
                ..
            },
        ..
    } = outcome
    else {
        panic!("winner and conflict copy must both apply");
    };

    assert_eq!(
        (
            document_text(&snapshot_document(&connection, "doc-1").expect("winner snapshot")),
            document_text(
                &snapshot_document(&connection, &conflict_document_id)
                    .expect("loser conflict snapshot"),
            ),
        ),
        ("Remote winning text", "Local losing text")
    );
}

#[test]
fn combined_apply_failure_rolls_back_original_copy_children_and_marker() {
    let connection = migrated_connection();
    let loser = envelope("doc-1", "Local before failure");
    create(&connection, &loser);
    let before = snapshot_document(&connection, "doc-1").expect("before snapshot");
    let (revision, fingerprint) = expected_state(&connection, "doc-1");
    let mut winner = envelope("doc-1", "Winner must roll back");
    winner.citation_projections.zotero[0].item_key = "FORCED_FAILURE".to_string();
    connection
        .execute_batch(
            "CREATE TEMP TRIGGER fail_combined_winner
             BEFORE INSERT ON writing_zotero_citations
             WHEN NEW.document_id = 'doc-1' AND NEW.item_key = 'FORCED_FAILURE'
             BEGIN
               SELECT RAISE(ABORT, 'forced combined receive failure');
             END;",
        )
        .expect("install test-only failure trigger");
    let derived_id = copy_id(&loser);

    let error = preserve_loser_then_receive_winner(
        &connection,
        &loser,
        &source("doc-1", "Laptop", 100),
        &receipt(&loser),
        &winner,
        revision,
        &fingerprint,
        &receipt(&winner),
    )
    .expect_err("forced winner failure");

    assert_eq!(error.code, "sql_error");
    assert_eq!(
        snapshot_document(&connection, "doc-1").expect("original rolled back"),
        before
    );
    assert_eq!(
        (
            connection
                .query_row(
                    "SELECT COUNT(*) FROM writing_documents WHERE id = ?1",
                    [&derived_id],
                    |row| row.get::<_, i64>(0),
                )
                .expect("copy count"),
            count(&connection, "writing_document_collections", &derived_id),
            count(&connection, "writing_document_citations", &derived_id),
            count(&connection, "writing_zotero_citations", &derived_id),
            count(&connection, "writing_provenance_events", &derived_id),
        ),
        (0, 0, 0, 0, 0)
    );
}

#[test]
fn dirty_journal_defers_combined_operation_without_copy_or_overwrite() {
    let connection = migrated_connection();
    let loser = envelope("doc-1", "Unsaved local text");
    create(&connection, &loser);
    let before = snapshot_document(&connection, "doc-1").expect("before snapshot");
    let (revision, fingerprint) = expected_state(&connection, "doc-1");
    connection
        .execute(
            "INSERT INTO writing_journal
               (document_id, seq, base_revision, schema_version, delta_json, checksum, created_at)
             VALUES ('doc-1', 1, 0, 1, '[{\"step\":\"pending\"}]', 'checksum', 1)",
            [],
        )
        .expect("pending journal");
    let winner = envelope("doc-1", "Remote text must wait");

    let outcome = preserve_loser_then_receive_winner(
        &connection,
        &loser,
        &source("doc-1", "Laptop", 100),
        &receipt(&loser),
        &winner,
        revision,
        &fingerprint,
        &receipt(&winner),
    )
    .expect("typed journal deferral");

    assert!(matches!(
        outcome,
        ConflictThenReceiveOutcome::WinnerNotApplied {
            winner: ReceiveOutcome::Deferred {
                reason: ReceiveDeferred::PendingJournal { entries: 1 },
                ..
            }
        }
    ));
    assert_eq!(
        snapshot_document(&connection, "doc-1").expect("dirty original unchanged"),
        before
    );
    assert_eq!(document_count(&connection), 1);
    assert_eq!(count(&connection, "writing_journal", "doc-1"), 1);
}

#[test]
fn missing_winner_collection_defers_before_conflict_preservation() {
    let connection = migrated_connection();
    let loser = envelope("doc-1", "Local collection state");
    create(&connection, &loser);
    let before = snapshot_document(&connection, "doc-1").expect("before snapshot");
    let (revision, fingerprint) = expected_state(&connection, "doc-1");
    let mut winner = envelope("doc-1", "Remote missing dependency");
    winner.collection_associations = vec![CollectionAssociationV1 {
        collection_id: "missing-collection".to_string(),
        is_primary: true,
    }];

    let outcome = preserve_loser_then_receive_winner(
        &connection,
        &loser,
        &source("doc-1", "Laptop", 100),
        &receipt(&loser),
        &winner,
        revision,
        &fingerprint,
        &receipt(&winner),
    )
    .expect("typed collection deferral");

    assert!(matches!(
        outcome,
        ConflictThenReceiveOutcome::WinnerNotApplied {
            winner: ReceiveOutcome::Deferred {
                reason: ReceiveDeferred::MissingCollections { .. },
                ..
            }
        }
    ));
    assert_eq!(
        snapshot_document(&connection, "doc-1").expect("original unchanged"),
        before
    );
    assert_eq!(document_count(&connection), 1);
}

#[test]
fn stale_expected_state_requires_conflict_without_creating_a_copy() {
    let connection = migrated_connection();
    let loser = envelope("doc-1", "Current local state");
    create(&connection, &loser);
    let (_, fingerprint) = expected_state(&connection, "doc-1");
    let winner = envelope("doc-1", "Remote winner");

    let outcome = preserve_loser_then_receive_winner(
        &connection,
        &loser,
        &source("doc-1", "Laptop", 100),
        &receipt(&loser),
        &winner,
        99,
        &fingerprint,
        &receipt(&winner),
    )
    .expect("typed stale-state conflict");

    assert!(matches!(
        outcome,
        ConflictThenReceiveOutcome::WinnerNotApplied {
            winner: ReceiveOutcome::ConflictRequired { .. }
        }
    ));
    assert_eq!(document_count(&connection), 1);
    assert_eq!(
        document_text(&snapshot_document(&connection, "doc-1").expect("original")),
        "Current local state"
    );
}
