//! Previous transcripts of an audio asset (migration 0066).
//!
//! `transcriptions` holds one row per asset. Before that row is replaced, a
//! re-transcription copies it into `transcription_versions` in the same
//! savepoint, so a replacement never loses the text, the speaker names typed
//! into it, or its segments. Restoring a version swaps it with the current
//! transcript: the current one becomes a version in turn.
//!
//! The table is local to the device; sync does not carry it.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// One replaced transcript, as the transcript panel lists it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptVersion {
    pub id: String,
    pub text_content: String,
    pub model: Option<String>,
    pub language: Option<String>,
    pub duration_ms: Option<i64>,
    /// When this transcript was made.
    pub transcribed_at: i64,
    /// When it stopped being the current transcript.
    pub replaced_at: i64,
}

/// How many replaced transcripts an asset keeps; the oldest goes first.
pub(crate) const MAX_VERSIONS_PER_ASSET: usize = 10;

/// Runs `work` inside a savepoint: everything it wrote is kept, or nothing.
///
/// On a connection with no open transaction the savepoint would start a
/// DEFERRED one, which reads under a snapshot and cannot upgrade to a write
/// once another connection has committed: `database is locked` at once,
/// busy_timeout notwithstanding. So the outermost call takes the write lock
/// first with `BEGIN IMMEDIATE` (waiting its busy_timeout) and commits at the
/// end; a nested call only adds the savepoint.
pub(crate) fn in_savepoint<T>(
    conn: &Connection,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let outermost = conn.is_autocommit();
    if outermost {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| format!("Failed to open the transcript transaction: {e}"))?;
    }
    let end = |commit: bool| {
        if outermost {
            conn.execute_batch(if commit { "COMMIT" } else { "ROLLBACK" })
        } else {
            Ok(())
        }
    };
    if let Err(error) = conn.execute_batch("SAVEPOINT transcript_swap") {
        let _ = end(false);
        return Err(format!("Failed to open the transcript savepoint: {error}"));
    }
    let committed = match work() {
        Ok(value) => conn
            .execute_batch("RELEASE transcript_swap")
            .and_then(|()| end(true))
            .map(|()| value)
            .map_err(|error| format!("Failed to commit the transcript change: {error}")),
        Err(error) => Err(error),
    };
    if committed.is_err() && !conn.is_autocommit() {
        // Never leave the change open on a shared connection.
        let _ = conn.execute_batch("ROLLBACK TO transcript_swap; RELEASE transcript_swap");
        let _ = end(false);
    }
    committed
}

/// Copies the asset's current transcript into the versions, stamped
/// `replaced_at`. Does nothing when the asset has no transcript or only an
/// empty one. Keeps the asset's [`MAX_VERSIONS_PER_ASSET`] most recently
/// replaced versions and drops older ones. Call inside the savepoint of the
/// write that replaces it.
pub(crate) fn archive_current(
    conn: &Connection,
    asset_id: &str,
    replaced_at: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO transcription_versions
           (id, asset_id, text_content, language, duration_ms, model, segments, confidence,
            transcribed_at, replaced_at)
         SELECT ?1, asset_id, text_content, language, duration_ms, model, segments, confidence,
                created_at, ?2
           FROM transcriptions
          WHERE asset_id = ?3 AND trim(text_content) <> ''",
        params![uuid::Uuid::new_v4().to_string(), replaced_at, asset_id],
    )
    .map_err(|e| format!("Failed to keep the previous transcript: {e}"))?;
    conn.execute(
        "DELETE FROM transcription_versions
          WHERE asset_id = ?1
            AND id NOT IN (SELECT id FROM transcription_versions
                            WHERE asset_id = ?1
                            ORDER BY replaced_at DESC, rowid DESC
                            LIMIT ?2)",
        params![asset_id, MAX_VERSIONS_PER_ASSET as i64],
    )
    .map_err(|e| format!("Failed to drop the oldest transcript versions: {e}"))?;
    Ok(())
}

/// The asset's previous transcripts, the most recently replaced first.
pub(crate) fn list(conn: &Connection, asset_id: &str) -> Result<Vec<TranscriptVersion>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, text_content, model, language, duration_ms, transcribed_at, replaced_at
               FROM transcription_versions
              WHERE asset_id = ?1
              ORDER BY replaced_at DESC, rowid DESC",
        )
        .map_err(|e| format!("Failed to read the previous transcripts: {e}"))?;
    let rows = stmt
        .query_map([asset_id], |row| {
            Ok(TranscriptVersion {
                id: row.get(0)?,
                text_content: row.get(1)?,
                model: row.get(2)?,
                language: row.get(3)?,
                duration_ms: row.get(4)?,
                transcribed_at: row.get(5)?,
                replaced_at: row.get(6)?,
            })
        })
        .map_err(|e| format!("Failed to read the previous transcripts: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read the previous transcripts: {e}"))
}

/// Makes a version the current transcript again and keeps the one it
/// replaces as a version. Returns the asset id.
pub(crate) fn restore(conn: &Connection, version_id: &str, now: i64) -> Result<String, String> {
    in_savepoint(conn, || {
        type Row = (
            String,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<f64>,
            i64,
        );
        let version: Option<Row> = conn
            .query_row(
                "SELECT asset_id, text_content, language, duration_ms, model, segments,
                        confidence, transcribed_at
                   FROM transcription_versions WHERE id = ?1",
                [version_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("Failed to read the transcript version: {e}"))?;
        let Some((asset_id, text, language, duration_ms, model, segments, confidence, made_at)) =
            version
        else {
            return Err("transcript_version_not_found".to_string());
        };

        // The restored version leaves the list before the current transcript
        // enters it, so a swap at the cap drops no other version.
        conn.execute(
            "DELETE FROM transcription_versions WHERE id = ?1",
            [version_id],
        )
        .map_err(|e| format!("Failed to restore the transcript: {e}"))?;
        archive_current(conn, &asset_id, now)?;
        // Same deterministic id and UPSERT as `save_transcription`, so sync
        // sees an UPDATE of the one row per asset.
        conn.execute(
            "INSERT INTO transcriptions(id, asset_id, text_content, language, duration_ms, model,
                                        segments, confidence, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(asset_id) DO UPDATE SET
               id = excluded.id,
               text_content = excluded.text_content,
               language = excluded.language,
               duration_ms = excluded.duration_ms,
               model = excluded.model,
               segments = excluded.segments,
               confidence = excluded.confidence,
               created_at = excluded.created_at",
            params![
                format!("trx-{asset_id}"),
                asset_id,
                text,
                language,
                duration_ms,
                model.clone().unwrap_or_default(),
                segments,
                confidence,
                made_at,
            ],
        )
        .map_err(|e| format!("Failed to restore the transcript: {e}"))?;

        crate::asset_history::record_best_effort(
            conn,
            &crate::asset_history::ProcessEvent::new(
                &asset_id,
                crate::asset_history::PROCESS_TRANSCRIPTION,
                "EntropIA (versión restaurada)",
            )
            .model(model.as_deref().unwrap_or(""))
            .output_text(&text),
        );
        Ok(asset_id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../tests/fixtures/schema_full.sql"))
            .unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             INSERT INTO collections (id, name, created_at, updated_at) VALUES ('c1', 'C', 0, 0);
             INSERT INTO items (id, title, collection_id, created_at, updated_at) VALUES ('i1', 'I', 'c1', 0, 0);
             INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('a1', 'i1', 'assets/a.mp3', 'audio', 0);",
        )
        .unwrap();
        conn
    }

    fn set_current(conn: &Connection, text: &str, model: &str, created_at: i64) {
        conn.execute(
            "INSERT INTO transcriptions(id, asset_id, text_content, language, duration_ms, model,
                                        segments, confidence, created_at)
             VALUES ('trx-a1', 'a1', ?1, 'es', 1000, ?2, '[]', NULL, ?3)
             ON CONFLICT(asset_id) DO UPDATE SET text_content = excluded.text_content,
               model = excluded.model, created_at = excluded.created_at",
            params![text, model, created_at],
        )
        .unwrap();
    }

    fn current(conn: &Connection) -> (String, String, i64) {
        conn.query_row(
            "SELECT text_content, model, created_at FROM transcriptions WHERE asset_id = 'a1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
    }

    #[test]
    fn archiving_keeps_the_replaced_transcript_with_its_speaker_names() {
        let conn = archive();
        set_current(
            &conn,
            "Entrevistadora: hola\nAna: buenas",
            "assemblyai/best",
            10,
        );
        archive_current(&conn, "a1", 20).unwrap();
        let versions = list(&conn, "a1").unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(
            versions[0].text_content,
            "Entrevistadora: hola\nAna: buenas"
        );
        assert_eq!(versions[0].model.as_deref(), Some("assemblyai/best"));
        assert_eq!(versions[0].transcribed_at, 10);
        assert_eq!(versions[0].replaced_at, 20);
    }

    #[test]
    fn archiving_skips_a_missing_or_empty_transcript() {
        let conn = archive();
        archive_current(&conn, "a1", 5).unwrap();
        set_current(&conn, "   ", "parakeet/x", 10);
        archive_current(&conn, "a1", 20).unwrap();
        assert!(list(&conn, "a1").unwrap().is_empty());
    }

    #[test]
    fn versions_list_newest_replacement_first() {
        let conn = archive();
        set_current(&conn, "first", "m1", 1);
        archive_current(&conn, "a1", 10).unwrap();
        set_current(&conn, "second", "m2", 11);
        archive_current(&conn, "a1", 20).unwrap();
        let texts: Vec<String> = list(&conn, "a1")
            .unwrap()
            .into_iter()
            .map(|v| v.text_content)
            .collect();
        assert_eq!(texts, vec!["second", "first"]);
    }

    #[test]
    fn restoring_swaps_the_version_with_the_current_transcript() {
        let conn = archive();
        set_current(&conn, "with roles", "assemblyai/best", 10);
        archive_current(&conn, "a1", 20).unwrap();
        set_current(&conn, "drifted english", "parakeet/x", 20);
        let version = list(&conn, "a1").unwrap().remove(0);

        assert_eq!(restore(&conn, &version.id, 30).unwrap(), "a1");

        assert_eq!(
            current(&conn),
            ("with roles".to_string(), "assemblyai/best".to_string(), 10)
        );
        let versions = list(&conn, "a1").unwrap();
        assert_eq!(versions.len(), 1, "the restored version leaves the list");
        assert_eq!(versions[0].text_content, "drifted english");
        assert_eq!(versions[0].replaced_at, 30);
        let id: String = conn
            .query_row(
                "SELECT id FROM transcriptions WHERE asset_id = 'a1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(id, "trx-a1");
    }

    #[test]
    fn restoring_an_unknown_version_changes_nothing() {
        let conn = archive();
        set_current(&conn, "current", "m", 10);
        assert_eq!(
            restore(&conn, "missing", 30).unwrap_err(),
            "transcript_version_not_found"
        );
        assert_eq!(current(&conn).0, "current");
        assert!(list(&conn, "a1").unwrap().is_empty());
    }

    #[test]
    fn a_failed_write_rolls_the_savepoint_back() {
        let conn = archive();
        set_current(&conn, "current", "m", 10);
        let result: Result<(), String> = in_savepoint(&conn, || {
            archive_current(&conn, "a1", 20)?;
            Err("boom".to_string())
        });
        assert_eq!(result.unwrap_err(), "boom");
        assert!(list(&conn, "a1").unwrap().is_empty());
    }

    /// Restoring reads the version, then writes. A worker that commits in
    /// between must not make the restore fail: the swap takes the write lock
    /// before its first read, so the worker waits instead.
    #[test]
    fn a_commit_from_another_connection_mid_swap_does_not_fail_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("entropia.sqlite");
        let conn = crate::db::open::open_archive_connection(&path).unwrap();
        conn.execute_batch(include_str!("../../tests/fixtures/schema_full.sql"))
            .unwrap();
        conn.execute_batch(
            "INSERT INTO collections (id, name, created_at, updated_at) VALUES ('c1', 'C', 0, 0);
             INSERT INTO items (id, title, collection_id, created_at, updated_at) VALUES ('i1', 'I', 'c1', 0, 0);
             INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('a1', 'i1', 'assets/a.mp3', 'audio', 0);",
        )
        .unwrap();
        set_current(&conn, "current", "m", 10);
        let worker = Connection::open(&path).unwrap();
        worker.busy_timeout(std::time::Duration::ZERO).unwrap();

        let result = in_savepoint(&conn, || {
            let _seen: i64 = conn
                .query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            // The worker's commit lands between the swap's read and its write.
            let _ = worker.execute("UPDATE collections SET name = 'worker' WHERE id = 'c1'", []);
            archive_current(&conn, "a1", 20)
        });

        assert_eq!(result, Ok(()));
        assert_eq!(list(&conn, "a1").unwrap().len(), 1);
        assert!(conn.is_autocommit(), "the swap left its transaction open");
    }

    fn version_texts(conn: &Connection) -> Vec<String> {
        list(conn, "a1")
            .unwrap()
            .into_iter()
            .map(|version| version.text_content)
            .collect()
    }

    #[test]
    fn an_audio_keeps_its_ten_most_recent_versions() {
        let conn = archive();
        for n in 1..=11 {
            set_current(&conn, &format!("take {n}"), "m", n);
            archive_current(&conn, "a1", 100 + n).unwrap();
        }

        let texts = version_texts(&conn);
        assert_eq!(texts.len(), MAX_VERSIONS_PER_ASSET);
        assert_eq!(texts.first().map(String::as_str), Some("take 11"));
        assert_eq!(texts.last().map(String::as_str), Some("take 2"));
    }

    #[test]
    fn restoring_at_the_cap_keeps_every_other_version() {
        let conn = archive();
        for n in 1..=10 {
            set_current(&conn, &format!("take {n}"), "m", n);
            archive_current(&conn, "a1", 100 + n).unwrap();
        }
        set_current(&conn, "current", "m", 50);
        let oldest = list(&conn, "a1").unwrap().pop().unwrap();
        assert_eq!(oldest.text_content, "take 1");
        let middle = list(&conn, "a1").unwrap().remove(4);

        restore(&conn, &middle.id, 200).unwrap();

        let texts = version_texts(&conn);
        assert_eq!(texts.len(), MAX_VERSIONS_PER_ASSET);
        assert_eq!(texts.first().map(String::as_str), Some("current"));
        assert!(texts.iter().any(|text| text == "take 1"), "{texts:?}");
        assert!(!texts.contains(&middle.text_content), "{texts:?}");
    }

    #[test]
    fn deleting_the_asset_deletes_its_versions() {
        let conn = archive();
        set_current(&conn, "current", "m", 10);
        archive_current(&conn, "a1", 20).unwrap();
        conn.execute("DELETE FROM assets WHERE id = 'a1'", [])
            .unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM transcription_versions", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(left, 0);
    }
}
