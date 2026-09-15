//! Comparing the canonical document against its journal on open (§16.3).
//!
//! The rule that shapes everything here: a truncated or incompatible journal is
//! never applied partially and never overwrites the confirmed document. So a
//! plan replays the verified prefix, stops at the first entry that fails its
//! checksum, and **deletes nothing** — what is left behind stays for diagnosis
//! or an explicit recovery.

use rusqlite::Connection;

use super::journal::{self, JournalEntry};
use super::repository::{self, WritingResult};

/// What can be recovered for one document, and what cannot.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecoveryPlan {
    pub document_id: String,
    /// The last revision persistence acknowledged.
    pub canonical_revision: i64,
    /// Verified entries, in order, safe to replay on top of the canonical
    /// content. Empty means there is nothing to recover.
    pub replayable: Vec<JournalEntry>,
    /// Sequence of the first entry that failed verification. Replay stops
    /// before it; the entry itself is kept.
    pub stopped_at_seq: Option<i64>,
    /// Entries behind the stop, still on disk, not offered.
    pub withheld: usize,
}

/// Builds the plan for one document. Read-only: it changes nothing.
pub fn plan(conn: &Connection, document_id: &str) -> WritingResult<RecoveryPlan> {
    let doc = repository::load_document(conn, document_id)?;
    let pending = journal::read_pending(conn, document_id, doc.revision)?;

    let mut replayable = Vec::new();
    let mut stopped_at_seq = None;
    let mut withheld = 0;

    for (entry, intact) in pending {
        if stopped_at_seq.is_some() {
            withheld += 1;
            continue;
        }
        if intact {
            replayable.push(entry);
        } else {
            stopped_at_seq = Some(entry.seq);
            withheld += 1;
        }
    }

    Ok(RecoveryPlan {
        document_id: document_id.to_string(),
        canonical_revision: doc.revision,
        replayable,
        stopped_at_seq,
        withheld,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writing::journal::AppendJournal;
    use crate::writing::repository::{
        create_document, load_document, save_document, NewDocument, SaveDocument,
        DOCUMENT_NOT_FOUND,
    };

    const MIGRATION_0035: &str =
        include_str!("../../../../../packages/store/src/migrations/0035_writing_workspace.sql");
    const MIGRATION_0036: &str =
        include_str!("../../../../../packages/store/src/migrations/0036_writing_journal.sql");

    fn migrated_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = crate::db::open::open_archive_connection(&dir.path().join("entropia.sqlite"))
            .expect("open");
        conn.execute_batch(
            "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);",
        )
        .expect("minimal corpus");
        conn.execute_batch(&format!(
            "BEGIN IMMEDIATE;\n{MIGRATION_0035}\n{MIGRATION_0036}\n\
             INSERT INTO _migrations (name, applied_at) VALUES ('0035_writing_workspace', 0);\n\
             INSERT INTO _migrations (name, applied_at) VALUES ('0036_writing_journal', 0);\nCOMMIT;"
        ))
        .expect("apply writing migrations");
        (dir, conn)
    }

    fn a_document(conn: &Connection, id: &str) {
        create_document(
            conn,
            NewDocument {
                id: id.to_string(),
                title: "Articulo".into(),
                document_type: "article".into(),
                schema_version: 1,
                content_json: r#"{"type":"doc"}"#.into(),
            },
        )
        .expect("create");
    }

    fn append(conn: &Connection, id: &str, base_revision: i64, delta: &str) -> i64 {
        journal::append(
            conn,
            AppendJournal {
                document_id: id.to_string(),
                base_revision,
                schema_version: 1,
                delta_json: delta.to_string(),
            },
        )
        .expect("append")
    }

    #[test]
    fn a_document_with_no_journal_has_nothing_to_recover() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        let p = plan(&conn, "d1").expect("plan");
        assert!(p.replayable.is_empty());
        assert_eq!(p.canonical_revision, 0);
        assert_eq!(p.stopped_at_seq, None);
    }

    #[test]
    fn sequence_numbers_advance_per_document() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        a_document(&conn, "d2");
        assert_eq!(append(&conn, "d1", 0, "[1]"), 1);
        assert_eq!(append(&conn, "d1", 0, "[2]"), 2);
        assert_eq!(
            append(&conn, "d2", 0, "[1]"),
            1,
            "each document counts alone"
        );
    }

    #[test]
    fn appending_for_an_unknown_document_is_reported() {
        let (_dir, conn) = migrated_db();
        let err = journal::append(
            &conn,
            AppendJournal {
                document_id: "ghost".into(),
                base_revision: 0,
                schema_version: 1,
                delta_json: "[]".into(),
            },
        )
        .expect_err("must fail");
        assert_eq!(err.code, DOCUMENT_NOT_FOUND);
    }

    #[test]
    fn unconfirmed_deltas_come_back_in_order() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        append(&conn, "d1", 0, r#"["uno"]"#);
        append(&conn, "d1", 0, r#"["dos"]"#);
        append(&conn, "d1", 0, r#"["tres"]"#);

        let p = plan(&conn, "d1").expect("plan");
        assert!(!p.replayable.is_empty());
        let deltas: Vec<&str> = p.replayable.iter().map(|e| e.delta_json.as_str()).collect();
        assert_eq!(deltas, vec![r#"["uno"]"#, r#"["dos"]"#, r#"["tres"]"#]);
    }

    #[test]
    fn deltas_already_folded_into_the_canonical_content_are_not_offered() {
        let (_dir, mut conn) = migrated_db();
        a_document(&conn, "d1");
        append(&conn, "d1", 0, r#"["vieja"]"#);

        save_document(
            &mut conn,
            SaveDocument {
                document_id: "d1".into(),
                expected_revision: 0,
                content_json: r#"{"saved":true}"#.into(),
                schema_version: 1,
                plain_text_cache: None,
                citations: Vec::new(),
                provenance: Vec::new(),
            },
        )
        .expect("save");
        append(&conn, "d1", 1, r#"["nueva"]"#);

        let p = plan(&conn, "d1").expect("plan");
        assert_eq!(p.canonical_revision, 1);
        let deltas: Vec<&str> = p.replayable.iter().map(|e| e.delta_json.as_str()).collect();
        assert_eq!(
            deltas,
            vec![r#"["nueva"]"#],
            "only work the canonical revision does not already contain"
        );
    }

    #[test]
    fn a_gap_in_the_sequence_is_not_corruption() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        append(&conn, "d1", 0, r#"["uno"]"#);
        // A crash between the commit and the writer's acknowledgement leaves an
        // entry nobody was told about; deleting it creates exactly this gap.
        conn.execute("DELETE FROM writing_journal WHERE seq = 1", [])
            .expect("simulate");
        append(&conn, "d1", 0, r#"["dos"]"#);

        let p = plan(&conn, "d1").expect("plan");
        assert_eq!(p.stopped_at_seq, None, "a gap must not read as corruption");
        assert_eq!(p.replayable.len(), 1);
    }

    #[test]
    fn a_tampered_entry_stops_the_replay_without_destroying_anything() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        append(&conn, "d1", 0, r#"["uno"]"#);
        append(&conn, "d1", 0, r#"["dos"]"#);
        append(&conn, "d1", 0, r#"["tres"]"#);

        // Truncated on disk: the content no longer matches its checksum.
        conn.execute(
            "UPDATE writing_journal SET delta_json = '[\"tru' WHERE seq = 2",
            [],
        )
        .expect("truncate");

        let p = plan(&conn, "d1").expect("plan");
        assert_eq!(p.stopped_at_seq, Some(2));
        assert_eq!(
            p.replayable.len(),
            1,
            "only the verified prefix may be replayed"
        );
        assert_eq!(p.replayable[0].delta_json, r#"["uno"]"#);
        assert_eq!(p.withheld, 2, "the bad entry and everything behind it");

        let still_there: i64 = conn
            .query_row("SELECT COUNT(*) FROM writing_journal", [], |r| r.get(0))
            .expect("count");
        assert_eq!(
            still_there, 3,
            "nothing is deleted; it is kept for diagnosis"
        );
        assert_eq!(
            load_document(&conn, "d1")
                .expect("load")
                .current_content_json,
            r#"{"type":"doc"}"#,
            "the confirmed document is untouched"
        );
    }

    #[test]
    fn the_checksum_is_computed_by_the_backend_not_supplied() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        append(&conn, "d1", 0, r#"["uno"]"#);

        // A caller cannot pass a checksum, so the only way to make one agree
        // with tampered content is to recompute it — which an attacker editing
        // the file will not do, and a truncated write cannot.
        conn.execute(
            "UPDATE writing_journal SET checksum = 'obviously-wrong' WHERE seq = 1",
            [],
        )
        .expect("tamper");

        let p = plan(&conn, "d1").expect("plan");
        assert_eq!(p.stopped_at_seq, Some(1));
        assert!(p.replayable.is_empty());
    }

    #[test]
    fn pruning_keeps_whatever_the_confirmed_revision_does_not_contain() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        append(&conn, "d1", 0, r#"["vieja"]"#);
        append(&conn, "d1", 1, r#"["encima-de-1"]"#);

        let removed = journal::prune_confirmed(&conn, "d1", 1).expect("prune");
        assert_eq!(removed, 1, "only the delta revision 1 already contains");

        let left: Vec<String> = conn
            .prepare("SELECT delta_json FROM writing_journal WHERE document_id='d1'")
            .and_then(|mut s| {
                s.query_map([], |r| r.get::<_, String>(0))
                    .map(|rows| rows.filter_map(Result::ok).collect())
            })
            .expect("query");
        assert_eq!(
            left,
            vec![r#"["encima-de-1"]"#.to_string()],
            "a delta recorded on top of the confirmed revision is still its only durable copy"
        );
    }

    #[test]
    fn discarding_clears_the_journal_but_not_the_document() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        append(&conn, "d1", 0, r#"["uno"]"#);
        append(&conn, "d1", 0, r#"["dos"]"#);

        let removed = journal::discard_all(&conn, "d1").expect("discard");
        assert_eq!(removed, 2);
        assert!(plan(&conn, "d1").expect("plan").replayable.is_empty());
        assert_eq!(load_document(&conn, "d1").expect("load").revision, 0);
    }

    #[test]
    fn a_journal_entry_never_advances_the_revision() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        append(&conn, "d1", 0, r#"["uno"]"#);
        append(&conn, "d1", 0, r#"["dos"]"#);

        assert_eq!(
            load_document(&conn, "d1").expect("load").revision,
            0,
            "persisting the journal is not a canonical save (§16.2)"
        );
    }
}
