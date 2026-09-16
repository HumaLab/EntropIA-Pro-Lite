//! Version history, retention and non-destructive restore (§9.3, §16.4).
//!
//! A version has to carry enough to reproduce its citations and bibliography,
//! not just its text (§8.4), so each snapshot stores the content together with
//! the document settings that rendered it.
//!
//! Restoring never destroys. §16.4 is explicit: restoring creates a new current
//! version and does not remove the history after it. So a restore snapshots the
//! state it is about to replace, advances the revision, and leaves every later
//! version in place — reaching back to an old draft is always reversible.

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::repository::{
    self, now_ms, require_schema, DocumentCitationInput, ProvenanceEventInput, WritingError,
    WritingResult, DOCUMENT_NOT_FOUND,
};

/// The requested version does not exist for this document.
pub const VERSION_NOT_FOUND: &str = "version_not_found";

/// Reasons a version gets written. `auto` is the only one retention compacts;
/// everything else records a human decision and is kept.
pub const AUTOMATIC_REASON: &str = "auto";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct VersionSummary {
    pub version_number: i64,
    pub reason: String,
    pub schema_version: i64,
    pub content_hash: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct VersionContent {
    pub version_number: i64,
    pub content_json: String,
    pub document_settings_json: String,
    pub schema_version: i64,
    pub reason: String,
    pub created_at: i64,
}

/// A restore replaces content the same way a save does, so it carries the
/// projections the caller derived from the version it is reinstating. The
/// backend does not parse ProseMirror and will not invent them.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RestoreVersion {
    pub document_id: String,
    pub version_number: i64,
    pub expected_revision: i64,
    #[serde(default)]
    pub citations: Vec<DocumentCitationInput>,
    #[serde(default)]
    pub zotero_citations: Vec<super::repository::ZoteroCitationInput>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceEventInput>,
}

fn hash_of(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn settings_json(conn: &Connection, document_id: &str) -> WritingResult<String> {
    conn.query_row(
        "SELECT json_object(
                  'citation_style_id',    citation_style_id,
                  'citation_locale',      citation_locale,
                  'bibliography_enabled', bibliography_enabled,
                  'document_type',        document_type,
                  'title',                title)
           FROM writing_documents WHERE id = ?1",
        rusqlite::params![document_id],
        |row| row.get(0),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => WritingError::new(
            DOCUMENT_NOT_FOUND,
            format!("no document with id {document_id}"),
        ),
        other => WritingError::sql("Failed to read document settings", other),
    })
}

/// Writes a snapshot of the document as it stands. Returns the version number.
pub fn snapshot(conn: &Connection, document_id: &str, reason: &str) -> WritingResult<i64> {
    require_schema(conn)?;
    let settings = settings_json(conn, document_id)?;
    let next: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version_number), 0) + 1
               FROM writing_document_versions WHERE document_id = ?1",
            rusqlite::params![document_id],
            |row| row.get(0),
        )
        .map_err(|e| WritingError::sql("Failed to pick the next version number", e))?;

    conn.execute(
        "INSERT INTO writing_document_versions
           (id, document_id, version_number, content_json, schema_version,
            document_settings_json, reason, content_hash, created_at)
         SELECT ?1, id, ?2, current_content_json, schema_version, ?3, ?4,
                ?5, ?6
           FROM writing_documents WHERE id = ?7",
        rusqlite::params![
            uuid::Uuid::new_v4().to_string(),
            next,
            settings,
            reason,
            // Hash the content we are about to copy, read in the same statement.
            hash_of(&repository::load_document(conn, document_id)?.current_content_json),
            now_ms(),
            document_id
        ],
    )
    .map_err(|e| WritingError::sql("Failed to write version", e))?;
    Ok(next)
}

/// Versions newest first, for the history panel (§16.4).
pub fn list(conn: &Connection, document_id: &str) -> WritingResult<Vec<VersionSummary>> {
    require_schema(conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT version_number, reason, schema_version, content_hash, created_at
               FROM writing_document_versions
              WHERE document_id = ?1
              ORDER BY version_number DESC",
        )
        .map_err(|e| WritingError::sql("Failed to list versions", e))?;
    stmt.query_map(rusqlite::params![document_id], |row| {
        Ok(VersionSummary {
            version_number: row.get(0)?,
            reason: row.get(1)?,
            schema_version: row.get(2)?,
            content_hash: row.get(3)?,
            created_at: row.get(4)?,
        })
    })
    .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
    .map_err(|e| WritingError::sql("Failed to list versions", e))
}

/// One version's content and the settings that rendered it.
pub fn read(
    conn: &Connection,
    document_id: &str,
    version_number: i64,
) -> WritingResult<VersionContent> {
    require_schema(conn)?;
    conn.query_row(
        "SELECT version_number, content_json, document_settings_json, schema_version, reason, created_at
           FROM writing_document_versions
          WHERE document_id = ?1 AND version_number = ?2",
        rusqlite::params![document_id, version_number],
        |row| {
            Ok(VersionContent {
                version_number: row.get(0)?,
                content_json: row.get(1)?,
                document_settings_json: row.get(2)?,
                schema_version: row.get(3)?,
                reason: row.get(4)?,
                created_at: row.get(5)?,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => WritingError::new(
            VERSION_NOT_FOUND,
            format!("document {document_id} has no version {version_number}"),
        ),
        other => WritingError::sql("Failed to read version", other),
    })
}

/// Reinstates an earlier version as the current content, without destroying
/// anything: the state being replaced is snapshotted first, and every version
/// after the restored one stays exactly where it was (§16.4).
///
/// The revision check is the same one a save makes, so a restore racing another
/// window loses cleanly instead of overwriting it.
pub fn restore(conn: &mut Connection, input: RestoreVersion) -> WritingResult<i64> {
    require_schema(conn)?;
    let target = read(conn, &input.document_id, input.version_number)?;

    // Snapshot what we are about to replace, before anything changes. This
    // cannot share the save's transaction, so a save that does not go through
    // has to take its snapshot back with it: a restore that changed nothing
    // must leave nothing behind, least of all a version in the history panel.
    let taken = snapshot(conn, &input.document_id, "restore")?;
    let document_id = input.document_id.clone();

    let outcome = repository::save_document(
        conn,
        repository::SaveDocument {
            document_id: input.document_id,
            expected_revision: input.expected_revision,
            content_json: target.content_json,
            schema_version: target.schema_version,
            plain_text_cache: None,
            citations: input.citations,
            zotero_citations: input.zotero_citations,
            provenance: input.provenance,
        },
    );

    if outcome.is_err() {
        // Best effort: if this also fails the snapshot is harmless, and the
        // save's error is the one worth reporting.
        let _ = conn.execute(
            "DELETE FROM writing_document_versions
              WHERE document_id = ?1 AND version_number = ?2 AND reason = 'restore'",
            rusqlite::params![document_id, taken],
        );
    }
    outcome
}

/// Compacts automatic snapshots, keeping the `keep` most recent (§9.3).
///
/// Only `auto` versions are touched. A checkpoint, a close, a restore or a
/// migration records a decision someone made, and retention is not entitled to
/// throw those away.
pub fn apply_retention(conn: &Connection, document_id: &str, keep: usize) -> WritingResult<usize> {
    require_schema(conn)?;
    conn.execute(
        "DELETE FROM writing_document_versions
          WHERE document_id = ?1
            AND reason = ?2
            AND version_number NOT IN (
              SELECT version_number FROM writing_document_versions
               WHERE document_id = ?1 AND reason = ?2
               ORDER BY version_number DESC LIMIT ?3
            )",
        rusqlite::params![document_id, AUTOMATIC_REASON, keep as i64],
    )
    .map_err(|e| WritingError::sql("Failed to apply retention", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writing::repository::{
        create_document, load_document, NewDocument, SaveDocument, REVISION_CONFLICT,
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
                content_json: r#"{"v":0}"#.into(),
            },
        )
        .expect("create");
    }

    fn save(conn: &mut Connection, id: &str, expected: i64, content: &str) -> i64 {
        repository::save_document(
            conn,
            SaveDocument {
                document_id: id.to_string(),
                expected_revision: expected,
                content_json: content.to_string(),
                schema_version: 1,
                plain_text_cache: None,
                citations: Vec::new(),
                zotero_citations: Vec::new(),
                provenance: Vec::new(),
            },
        )
        .expect("save")
    }

    #[test]
    fn a_snapshot_captures_the_content_and_the_settings_that_rendered_it() {
        let (_dir, mut conn) = migrated_db();
        a_document(&conn, "d1");
        save(&mut conn, "d1", 0, r#"{"v":1}"#);

        let n = snapshot(&conn, "d1", "checkpoint").expect("snapshot");
        assert_eq!(n, 1);

        let v = read(&conn, "d1", 1).expect("read");
        assert_eq!(v.content_json, r#"{"v":1}"#);
        assert_eq!(v.reason, "checkpoint");
        assert!(
            v.document_settings_json.contains("bibliography_enabled"),
            "a version must reproduce its citations, not just its text: {}",
            v.document_settings_json
        );
    }

    #[test]
    fn version_numbers_advance_per_document() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        a_document(&conn, "d2");
        assert_eq!(snapshot(&conn, "d1", "auto").expect("s"), 1);
        assert_eq!(snapshot(&conn, "d1", "auto").expect("s"), 2);
        assert_eq!(snapshot(&conn, "d2", "auto").expect("s"), 1);
    }

    #[test]
    fn versions_are_listed_newest_first() {
        let (_dir, mut conn) = migrated_db();
        a_document(&conn, "d1");
        snapshot(&conn, "d1", "auto").expect("s");
        save(&mut conn, "d1", 0, r#"{"v":1}"#);
        snapshot(&conn, "d1", "checkpoint").expect("s");

        let list = list(&conn, "d1").expect("list");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].version_number, 2);
        assert_eq!(list[0].reason, "checkpoint");
    }

    #[test]
    fn reading_a_version_that_does_not_exist_is_reported() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        let err = read(&conn, "d1", 99).expect_err("must fail");
        assert_eq!(err.code, VERSION_NOT_FOUND);
    }

    #[test]
    fn restoring_reinstates_old_content_as_a_new_revision() {
        let (_dir, mut conn) = migrated_db();
        a_document(&conn, "d1");
        save(&mut conn, "d1", 0, r#"{"v":"vieja"}"#);
        snapshot(&conn, "d1", "checkpoint").expect("s");
        save(&mut conn, "d1", 1, r#"{"v":"nueva"}"#);

        let new_revision = restore(
            &mut conn,
            RestoreVersion {
                document_id: "d1".into(),
                version_number: 1,
                expected_revision: 2,
                citations: Vec::new(),
                zotero_citations: Vec::new(),
                provenance: Vec::new(),
            },
        )
        .expect("restore");

        assert_eq!(new_revision, 3, "restoring moves forward, never backward");
        let doc = load_document(&conn, "d1").expect("load");
        assert_eq!(doc.current_content_json, r#"{"v":"vieja"}"#);
        assert_eq!(doc.revision, 3);
    }

    #[test]
    fn restoring_snapshots_what_it_replaces_and_keeps_later_history() {
        let (_dir, mut conn) = migrated_db();
        a_document(&conn, "d1");
        save(&mut conn, "d1", 0, r#"{"v":"vieja"}"#);
        snapshot(&conn, "d1", "checkpoint").expect("v1");
        save(&mut conn, "d1", 1, r#"{"v":"nueva"}"#);
        snapshot(&conn, "d1", "checkpoint").expect("v2");

        restore(
            &mut conn,
            RestoreVersion {
                document_id: "d1".into(),
                version_number: 1,
                expected_revision: 2,
                citations: Vec::new(),
                zotero_citations: Vec::new(),
                provenance: Vec::new(),
            },
        )
        .expect("restore");

        let list = list(&conn, "d1").expect("list");
        assert_eq!(list.len(), 3, "the restore added one, removed none");
        assert_eq!(list[0].reason, "restore");
        // The state that was replaced is recoverable.
        assert_eq!(
            read(&conn, "d1", 3).expect("read").content_json,
            r#"{"v":"nueva"}"#
        );
        // And the version that existed after the one we restored is still there.
        assert_eq!(
            read(&conn, "d1", 2).expect("read").content_json,
            r#"{"v":"nueva"}"#
        );
    }

    #[test]
    fn a_restore_racing_another_window_loses_cleanly() {
        let (_dir, mut conn) = migrated_db();
        a_document(&conn, "d1");
        save(&mut conn, "d1", 0, r#"{"v":"vieja"}"#);
        snapshot(&conn, "d1", "checkpoint").expect("v1");
        save(&mut conn, "d1", 1, r#"{"v":"nueva"}"#);

        let err = restore(
            &mut conn,
            RestoreVersion {
                document_id: "d1".into(),
                version_number: 1,
                expected_revision: 1, // stale: the document is at 2
                citations: Vec::new(),
                zotero_citations: Vec::new(),
                provenance: Vec::new(),
            },
        )
        .expect_err("must fail");
        assert_eq!(err.code, REVISION_CONFLICT);

        let doc = load_document(&conn, "d1").expect("load");
        assert_eq!(doc.current_content_json, r#"{"v":"nueva"}"#);
        assert_eq!(doc.revision, 2, "a losing restore leaves the content alone");
    }

    #[test]
    fn retention_compacts_automatic_snapshots_only() {
        let (_dir, mut conn) = migrated_db();
        a_document(&conn, "d1");
        for i in 0..5 {
            save(&mut conn, "d1", i, &format!("{{\"v\":{}}}", i + 1));
            snapshot(&conn, "d1", "auto").expect("auto");
        }
        snapshot(&conn, "d1", "checkpoint").expect("manual");

        let removed = apply_retention(&conn, "d1", 2).expect("retention");
        assert_eq!(removed, 3, "five automatic snapshots, two kept");

        let list = list(&conn, "d1").expect("list");
        let reasons: Vec<&str> = list.iter().map(|v| v.reason.as_str()).collect();
        assert_eq!(reasons, vec!["checkpoint", "auto", "auto"]);
        assert_eq!(
            list[0].version_number, 6,
            "the human's checkpoint is never a retention candidate"
        );
    }

    #[test]
    fn a_losing_restore_leaves_no_orphan_snapshot_in_the_history() {
        let (_dir, mut conn) = migrated_db();
        a_document(&conn, "d1");
        save(&mut conn, "d1", 0, r#"{"v":"vieja"}"#);
        snapshot(&conn, "d1", "checkpoint").expect("v1");
        save(&mut conn, "d1", 1, r#"{"v":"nueva"}"#);
        let before = list(&conn, "d1").expect("list").len();

        restore(
            &mut conn,
            RestoreVersion {
                document_id: "d1".into(),
                version_number: 1,
                expected_revision: 1, // stale
                citations: Vec::new(),
                zotero_citations: Vec::new(),
                provenance: Vec::new(),
            },
        )
        .expect_err("must fail");

        assert_eq!(
            list(&conn, "d1").expect("list").len(),
            before,
            "a restore that changed nothing must not litter the history panel"
        );
    }

    #[test]
    fn retention_never_touches_a_restore_or_a_checkpoint() {
        let (_dir, conn) = migrated_db();
        a_document(&conn, "d1");
        for reason in ["checkpoint", "close", "restore", "migration"] {
            snapshot(&conn, "d1", reason).expect("snapshot");
        }

        let removed = apply_retention(&conn, "d1", 0).expect("retention");
        assert_eq!(removed, 0, "none of these record an automatic save");
        assert_eq!(list(&conn, "d1").expect("list").len(), 4);
    }
}
