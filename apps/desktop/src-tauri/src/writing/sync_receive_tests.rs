use rusqlite::{types::Value as SqlValue, Connection};
use serde_json::json;

use super::journal::{self, AppendJournal};
use super::sync_envelope::{
    snapshot_document, AttachmentFileV1, AttachmentManifestV1, CitationProjectionsV1,
    CollectionAssociationV1, CorpusCitationV1, DocumentSettingsV1, WritingEnvelopeV1,
    ZoteroCitationV1, INVALID_SYNC_ENVELOPE,
};
use super::sync_receive::{
    receive_envelope, AttachmentInstallReceipt, ReceiveAuthorization, ReceiveConflictReason,
    ReceiveDeferred, ReceiveOutcome, ATTACHMENTS_NOT_READY,
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
}

fn seed_collections(connection: &Connection) {
    connection
        .execute_batch(
            "INSERT INTO collections (id, name, created_at, updated_at) VALUES
               ('collection-a', 'First', 1, 1),
               ('collection-b', 'Second', 1, 1);",
        )
        .expect("seed collections");
}

fn envelope(document_id: &str, status: &str) -> WritingEnvelopeV1 {
    WritingEnvelopeV1 {
        id: document_id.to_string(),
        envelope_version: 1,
        content_json: json!({
            "schemaVersion": 1,
            "doc": {
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "Initial text" }]
                }]
            }
        }),
        title: "Received manuscript".to_string(),
        document_type: "article".to_string(),
        status: status.to_string(),
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

fn receipt(envelope: &WritingEnvelopeV1) -> AttachmentInstallReceipt {
    AttachmentInstallReceipt::caller_asserts_installed(
        envelope.fingerprint_sha256().expect("envelope fingerprint"),
    )
}

fn create(connection: &Connection, envelope: &WritingEnvelopeV1) -> ReceiveOutcome {
    receive_envelope(
        connection,
        &envelope.id,
        envelope,
        &ReceiveAuthorization::CreateOnly,
        &receipt(envelope),
    )
    .expect("receive new document")
}

fn local_revision(connection: &Connection, document_id: &str) -> i64 {
    connection
        .query_row(
            "SELECT revision FROM writing_documents WHERE id = ?1",
            [document_id],
            |row| row.get(0),
        )
        .expect("local revision")
}

fn replace_authorization(connection: &Connection, document_id: &str) -> ReceiveAuthorization {
    let snapshot = snapshot_document(connection, document_id).expect("local snapshot");
    ReceiveAuthorization::Replace {
        expected_local_revision: local_revision(connection, document_id),
        expected_local_fingerprint_sha256: snapshot
            .fingerprint_sha256()
            .expect("local fingerprint"),
    }
}

fn expected_local_snapshot(mut envelope: WritingEnvelopeV1) -> WritingEnvelopeV1 {
    envelope.attachments_manifest = AttachmentManifestV1::PreparationRequired;
    envelope
}

#[derive(Debug, PartialEq)]
struct AggregateRows {
    document: Vec<Vec<SqlValue>>,
    collections: Vec<Vec<SqlValue>>,
    corpus: Vec<Vec<SqlValue>>,
    zotero: Vec<Vec<SqlValue>>,
}

fn aggregate_rows(connection: &Connection, document_id: &str) -> AggregateRows {
    AggregateRows {
        document: query_rows(
            connection,
            "SELECT * FROM writing_documents WHERE id = ?1 ORDER BY id",
            document_id,
        ),
        collections: query_rows(
            connection,
            "SELECT * FROM writing_document_collections WHERE document_id = ?1
              ORDER BY collection_id",
            document_id,
        ),
        corpus: query_rows(
            connection,
            "SELECT * FROM writing_document_citations WHERE document_id = ?1 ORDER BY id",
            document_id,
        ),
        zotero: query_rows(
            connection,
            "SELECT * FROM writing_zotero_citations WHERE document_id = ?1 ORDER BY id",
            document_id,
        ),
    }
}

fn query_rows(connection: &Connection, sql: &str, document_id: &str) -> Vec<Vec<SqlValue>> {
    let mut statement = connection.prepare(sql).expect("prepare aggregate dump");
    let column_count = statement.column_count();
    statement
        .query_map([document_id], move |row| {
            (0..column_count)
                .map(|index| row.get::<_, SqlValue>(index))
                .collect::<Result<Vec<_>, _>>()
        })
        .expect("query aggregate dump")
        .collect::<Result<Vec<_>, _>>()
        .expect("read aggregate dump")
}

#[test]
fn initial_receive_inside_caller_transaction_roundtrips_complete_semantics() {
    let mut connection = migrated_connection();
    seed_collections(&connection);
    let incoming = envelope("doc-1", "active");

    let transaction = connection.transaction().expect("caller transaction");
    let outcome = create(&transaction, &incoming);
    assert!(matches!(
        outcome,
        ReceiveOutcome::Applied {
            local_revision: 0,
            created: true,
            ..
        }
    ));
    transaction.commit().expect("caller commit");

    assert_eq!(
        snapshot_document(&connection, "doc-1").expect("export received document"),
        expected_local_snapshot(incoming)
    );
}

#[test]
fn authorized_update_replaces_complete_aggregate_and_advances_only_local_revision() {
    let connection = migrated_connection();
    seed_collections(&connection);
    let initial = envelope("doc-1", "active");
    create(&connection, &initial);
    connection
        .execute(
            "UPDATE writing_documents SET plain_text_cache = 'STALE CACHE' WHERE id = 'doc-1'",
            [],
        )
        .expect("seed derived cache");
    let authorization = replace_authorization(&connection, "doc-1");

    let mut update = initial;
    update.title = "Remote authorized title".to_string();
    update.status = "archived".to_string();
    update.settings.citation_style_id = Some("chicago-author-date".to_string());
    update.settings.citation_locale = Some("en-US".to_string());
    update.settings.bibliography_enabled = false;
    update.collection_associations = vec![CollectionAssociationV1 {
        collection_id: "collection-b".to_string(),
        is_primary: true,
    }];
    update.content_json["doc"]["content"][0]["content"][0]["text"] =
        json!("Authorized replacement");
    update.citation_projections.corpus[0].quoted_text = Some("Updated quote".to_string());
    update.citation_projections.zotero[0].suffix = Some(", updated".to_string());

    let outcome = receive_envelope(
        &connection,
        "doc-1",
        &update,
        &authorization,
        &receipt(&update),
    )
    .expect("authorized receive");

    assert!(matches!(
        outcome,
        ReceiveOutcome::Applied {
            local_revision: 1,
            created: false,
            ..
        }
    ));
    assert_eq!(local_revision(&connection, "doc-1"), 1);
    assert_eq!(
        snapshot_document(&connection, "doc-1").expect("updated snapshot"),
        expected_local_snapshot(update)
    );
    let cache: Option<String> = connection
        .query_row(
            "SELECT plain_text_cache FROM writing_documents WHERE id = 'doc-1'",
            [],
            |row| row.get(0),
        )
        .expect("cache state");
    assert_eq!(cache, None, "canonical replacement invalidates the cache");
}

#[test]
fn stale_revision_or_canonical_fingerprint_requires_conflict_preservation() {
    let connection = migrated_connection();
    seed_collections(&connection);
    let initial = envelope("doc-1", "active");
    create(&connection, &initial);
    let stale_authorization = replace_authorization(&connection, "doc-1");
    connection
        .execute(
            "UPDATE writing_documents SET title = 'Local rename' WHERE id = 'doc-1'",
            [],
        )
        .expect("metadata edit without revision bump");

    let mut incoming = initial;
    incoming.title = "Remote title".to_string();
    let fingerprint_conflict = receive_envelope(
        &connection,
        "doc-1",
        &incoming,
        &stale_authorization,
        &receipt(&incoming),
    )
    .expect("typed conflict");
    let ReceiveOutcome::ConflictRequired { conflict, .. } = fingerprint_conflict else {
        panic!("stale fingerprint must require conflict preservation");
    };
    assert_eq!(
        conflict.reason,
        ReceiveConflictReason::ExpectedStateMismatch
    );
    assert_eq!(conflict.existing_local_revision, Some(0));
    assert_eq!(
        conflict
            .existing_snapshot
            .as_ref()
            .expect("full existing snapshot")
            .title,
        "Local rename"
    );

    let current = snapshot_document(&connection, "doc-1").expect("current snapshot");
    let stale_revision = ReceiveAuthorization::Replace {
        expected_local_revision: 7,
        expected_local_fingerprint_sha256: current
            .fingerprint_sha256()
            .expect("current fingerprint"),
    };
    let revision_conflict = receive_envelope(
        &connection,
        "doc-1",
        &incoming,
        &stale_revision,
        &receipt(&incoming),
    )
    .expect("typed revision conflict");
    assert!(matches!(
        revision_conflict,
        ReceiveOutcome::ConflictRequired {
            conflict: super::sync_receive::ReceiveConflict {
                reason: ReceiveConflictReason::ExpectedStateMismatch,
                existing_local_revision: Some(0),
                ..
            },
            ..
        }
    ));
    assert_eq!(
        snapshot_document(&connection, "doc-1")
            .expect("unchanged local")
            .title,
        "Local rename"
    );
}

#[test]
fn pending_recovery_journal_defers_before_writes_and_remains_intact() {
    let connection = migrated_connection();
    seed_collections(&connection);
    let initial = envelope("doc-1", "active");
    create(&connection, &initial);
    let before = snapshot_document(&connection, "doc-1").expect("before snapshot");
    journal::append(
        &connection,
        AppendJournal {
            document_id: "doc-1".to_string(),
            base_revision: 0,
            schema_version: 1,
            delta_json: r#"[{"step":"still local"}]"#.to_string(),
        },
    )
    .expect("pending journal entry");
    let authorization = replace_authorization(&connection, "doc-1");
    let mut incoming = initial;
    incoming.title = "Must wait".to_string();

    let outcome = receive_envelope(
        &connection,
        "doc-1",
        &incoming,
        &authorization,
        &receipt(&incoming),
    )
    .expect("journal deferral");

    assert!(matches!(
        outcome,
        ReceiveOutcome::Deferred {
            reason: ReceiveDeferred::PendingJournal { entries: 1 },
            ..
        }
    ));
    assert_eq!(
        snapshot_document(&connection, "doc-1").expect("document unchanged"),
        before
    );
    let mut journal_statement = connection
        .prepare("SELECT delta_json FROM writing_journal WHERE document_id = 'doc-1'")
        .expect("prepare journal read");
    let journal_rows: Vec<String> = journal_statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query journal")
        .collect::<Result<Vec<_>, _>>()
        .expect("journal remains");
    assert_eq!(journal_rows, vec![r#"[{"step":"still local"}]"#]);
}

#[test]
fn missing_collection_returns_typed_deferral_without_partial_state() {
    let mut connection = migrated_connection();
    seed_collections(&connection);
    let mut incoming = envelope("doc-1", "active");
    incoming.collection_associations = vec![CollectionAssociationV1 {
        collection_id: "collection-missing".to_string(),
        is_primary: true,
    }];

    let transaction = connection.transaction().expect("caller transaction");
    let outcome = create(&transaction, &incoming);
    assert_eq!(
        outcome,
        ReceiveOutcome::Deferred {
            document_id: "doc-1".to_string(),
            reason: ReceiveDeferred::MissingCollections {
                collection_ids: vec!["collection-missing".to_string()]
            }
        }
    );
    transaction
        .commit()
        .expect("caller commits deferred receive");

    let document_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM writing_documents", [], |row| {
            row.get(0)
        })
        .expect("document count");
    let child_count: i64 = connection
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM writing_document_collections) +
               (SELECT COUNT(*) FROM writing_document_citations) +
               (SELECT COUNT(*) FROM writing_zotero_citations)",
            [],
            |row| row.get(0),
        )
        .expect("child count");
    assert_eq!((document_count, child_count), (0, 0));
}

#[test]
fn forced_mid_insert_error_rolls_back_savepoint_even_when_caller_commits() {
    let mut connection = migrated_connection();
    seed_collections(&connection);
    let initial = envelope("doc-1", "active");
    create(&connection, &initial);
    let before = aggregate_rows(&connection, "doc-1");
    let authorization = replace_authorization(&connection, "doc-1");
    connection
        .execute_batch(
            "CREATE TEMP TRIGGER fail_received_zotero
             BEFORE INSERT ON writing_zotero_citations
             WHEN NEW.item_key = 'FORCED_FAILURE'
             BEGIN
               SELECT RAISE(ABORT, 'forced receive failure');
             END;",
        )
        .expect("install test-only failure trigger");
    let mut incoming = initial;
    incoming.title = "Must roll back".to_string();
    incoming.citation_projections.corpus[0].quoted_text = Some("Inserted first".to_string());
    incoming.citation_projections.zotero[0].item_key = "FORCED_FAILURE".to_string();

    let transaction = connection.transaction().expect("caller transaction");
    let error = receive_envelope(
        &transaction,
        "doc-1",
        &incoming,
        &authorization,
        &receipt(&incoming),
    )
    .expect_err("forced child insert failure");
    assert_eq!(error.code, "sql_error");
    transaction
        .commit()
        .expect("caller may still commit unrelated transaction");

    assert_eq!(aggregate_rows(&connection, "doc-1"), before);
}

#[test]
fn semantically_identical_receive_is_an_exact_noop() {
    let connection = migrated_connection();
    seed_collections(&connection);
    let incoming = envelope("doc-1", "active");
    create(&connection, &incoming);
    let before = aggregate_rows(&connection, "doc-1");

    let outcome = create(&connection, &incoming);

    assert!(matches!(
        outcome,
        ReceiveOutcome::NoOp {
            local_revision: 0,
            ..
        }
    ));
    assert_eq!(aggregate_rows(&connection, "doc-1"), before);
}

#[test]
fn child_id_collision_never_overwrites_another_document_and_remains_idempotent() {
    let connection = migrated_connection();
    seed_collections(&connection);
    let first = envelope("doc-a", "active");
    create(&connection, &first);
    let mut duplicate = first.clone();
    duplicate.id = "doc-b".to_string();
    duplicate.title = "Duplicate snapshot".to_string();

    create(&connection, &duplicate);

    let first_corpus: String = connection
        .query_row(
            "SELECT id FROM writing_document_citations WHERE document_id = 'doc-a'",
            [],
            |row| row.get(0),
        )
        .expect("first corpus id");
    let second_corpus: String = connection
        .query_row(
            "SELECT id FROM writing_document_citations WHERE document_id = 'doc-b'",
            [],
            |row| row.get(0),
        )
        .expect("second corpus id");
    let first_zotero: String = connection
        .query_row(
            "SELECT id FROM writing_zotero_citations WHERE document_id = 'doc-a'",
            [],
            |row| row.get(0),
        )
        .expect("first Zotero id");
    let second_zotero: String = connection
        .query_row(
            "SELECT id FROM writing_zotero_citations WHERE document_id = 'doc-b'",
            [],
            |row| row.get(0),
        )
        .expect("second Zotero id");
    assert_eq!(first_corpus, "shared-corpus-id");
    assert_eq!(first_zotero, "shared-zotero-id");
    assert_ne!(second_corpus, first_corpus);
    assert_ne!(second_zotero, first_zotero);

    let mut expected = expected_local_snapshot(duplicate.clone());
    expected.citation_projections.corpus[0].id = second_corpus;
    expected.citation_projections.zotero[0].id = second_zotero;
    assert_eq!(
        snapshot_document(&connection, "doc-b").expect("duplicate snapshot"),
        expected
    );
    let before_retry = aggregate_rows(&connection, "doc-b");
    assert!(matches!(
        create(&connection, &duplicate),
        ReceiveOutcome::NoOp { .. }
    ));
    assert_eq!(aggregate_rows(&connection, "doc-b"), before_retry);
    assert_eq!(
        connection
            .query_row(
                "SELECT metadata_snapshot_json FROM writing_document_citations
                  WHERE document_id = 'doc-a'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("first citation remains"),
        r#"{"authors":["Ada"],"title":"Corpus snapshot","year":2025}"#
    );
}

#[test]
fn preparation_required_attachments_are_rejected_before_any_write() {
    let connection = migrated_connection();
    seed_collections(&connection);
    let mut incoming = envelope("doc-1", "active");
    incoming.attachments_manifest = AttachmentManifestV1::PreparationRequired;
    let asserted = AttachmentInstallReceipt::caller_asserts_installed(
        incoming
            .fingerprint_sha256()
            .expect("preparation-required fingerprint"),
    );

    let error = receive_envelope(
        &connection,
        "doc-1",
        &incoming,
        &ReceiveAuthorization::CreateOnly,
        &asserted,
    )
    .expect_err("unprepared attachments rejected");

    assert_eq!(error.code, ATTACHMENTS_NOT_READY);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM writing_documents", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("no document"),
        0
    );
}

#[test]
fn archived_and_trashed_statuses_roundtrip_without_wire_revision_import() {
    let connection = migrated_connection();
    seed_collections(&connection);
    let archived = envelope("doc-1", "archived");
    create(&connection, &archived);
    assert_eq!(
        snapshot_document(&connection, "doc-1")
            .expect("archived snapshot")
            .status,
        "archived"
    );

    let authorization = replace_authorization(&connection, "doc-1");
    let mut trashed = archived;
    trashed.status = "trashed".to_string();
    let outcome = receive_envelope(
        &connection,
        "doc-1",
        &trashed,
        &authorization,
        &receipt(&trashed),
    )
    .expect("trash receive");

    assert!(matches!(
        outcome,
        ReceiveOutcome::Applied {
            local_revision: 1,
            ..
        }
    ));
    assert_eq!(
        snapshot_document(&connection, "doc-1")
            .expect("trashed snapshot")
            .status,
        "trashed"
    );
}

#[test]
fn authoritative_document_id_mismatch_is_rejected() {
    let connection = migrated_connection();
    seed_collections(&connection);
    let incoming = envelope("doc-1", "active");

    let error = receive_envelope(
        &connection,
        "different-id",
        &incoming,
        &ReceiveAuthorization::CreateOnly,
        &receipt(&incoming),
    )
    .expect_err("id mismatch");

    assert_eq!(error.code, INVALID_SYNC_ENVELOPE);
}

#[test]
fn mismatched_content_schema_is_rejected() {
    let connection = migrated_connection();
    seed_collections(&connection);
    let mut incoming = envelope("doc-1", "active");
    incoming.content_json["schemaVersion"] = json!(2);
    let asserted = AttachmentInstallReceipt::caller_asserts_installed(
        "0000000000000000000000000000000000000000000000000000000000000000",
    );

    let error = receive_envelope(
        &connection,
        "doc-1",
        &incoming,
        &ReceiveAuthorization::CreateOnly,
        &asserted,
    )
    .expect_err("schema mismatch");

    assert_eq!(error.code, INVALID_SYNC_ENVELOPE);
}

#[test]
fn unknown_envelope_fields_are_rejected_instead_of_discarded() {
    let incoming = envelope("doc-1", "active");
    let mut value = serde_json::to_value(incoming).expect("envelope value");
    value["future_destructive_semantics"] = json!({ "enabled": true });

    let error = WritingEnvelopeV1::from_json(&value.to_string()).expect_err("unknown field");

    assert_eq!(error.code, INVALID_SYNC_ENVELOPE);
}
