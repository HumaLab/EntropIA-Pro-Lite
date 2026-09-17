//! Reviewable agent assistance (plan-editor.md §14).
//!
//! # The rule the whole module serves
//!
//! §14.2 opens with it: *"El agente nunca modificará silenciosamente el
//! texto."* Every proposal is a suggestion the writer reads before anything
//! happens, and nothing here writes into a manuscript — it records what was
//! proposed and decides whether an acceptance may proceed.
//!
//! # Why acceptance is a state transition and not a flag
//!
//! §14.2 also asks that applying a suggestion and recording its acceptance be
//! **one persistent operation**, and that repeating an already-confirmed
//! request not insert the text twice. A double click, a retried command, a
//! window reopened on a stale panel — all of them send the same acceptance
//! again, and none of them should produce two copies of a paragraph.
//!
//! So acceptance is a conditional `UPDATE ... WHERE status = 'pending'` whose
//! affected-row count is checked, exactly like the revision guard in
//! [`super::repository`]. The first one wins and is told to apply the text; the
//! second is told the suggestion is already resolved and applies nothing. The
//! decision and the record are the same write, so there is no window in which
//! one happened without the other.
//!
//! # Why the hash is of the selection
//!
//! §14.2 requires the target range to be verified before applying, and §8.5 is
//! specific that what is hashed is *the target content*, not the whole
//! manuscript. Hashing the manuscript would invalidate every pending suggestion
//! the moment anyone typed anywhere, which trains people to ignore the warning.
//! Hashing the selection invalidates exactly the suggestions whose target
//! actually moved.

use rusqlite::Connection;

use super::repository::{now_ms, require_schema, WritingError, WritingResult};

/// A suggestion has been resolved already. Not an error: the first acceptance
/// succeeded, and this is the second one arriving.
pub const ALREADY_RESOLVED: &str = "suggestion_already_resolved";
/// The text under the suggestion is not what it was proposed against.
pub const TARGET_CHANGED: &str = "suggestion_target_changed";
pub const SUGGESTION_NOT_FOUND: &str = "suggestion_not_found";

/// What the writer may do with a proposal (§14.2).
pub const RESOLUTIONS: [&str; 3] = ["accepted", "inserted_below", "discarded"];

/// One proposal, as it is recorded before the writer has seen it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewSuggestion {
    pub id: String,
    pub document_id: String,
    /// Identity of the target, never a document position: spike S2 measured
    /// that a persisted `{from, to}` points at a different paragraph after a
    /// reload.
    pub selection_anchor_json: Option<String>,
    pub source_revision: i64,
    /// A hash of the selected text — the target, not the manuscript (§8.5).
    pub selected_content_hash: String,
    pub action_type: String,
    pub original_text: Option<String>,
    pub suggested_text: Option<String>,
    pub rationale: Option<String>,
    /// What the proposal rests on: corpus fragments, Zotero items, and whether
    /// the text was actually consulted (§14.2).
    pub evidence_json: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SuggestionRow {
    pub id: String,
    pub document_id: String,
    pub selection_anchor_json: Option<String>,
    pub source_revision: i64,
    pub selected_content_hash: String,
    pub action_type: String,
    pub original_text: Option<String>,
    pub suggested_text: Option<String>,
    pub rationale: Option<String>,
    pub evidence_json: String,
    pub status: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
}

/// Whether the caller should now write the text into the manuscript.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Resolution {
    /// True only for the acceptance that actually changed the status.
    pub apply: bool,
    pub status: String,
    pub suggested_text: Option<String>,
}

/// Records a proposal. Nothing is applied and nothing is changed.
pub fn record(conn: &Connection, input: NewSuggestion) -> WritingResult<SuggestionRow> {
    require_schema(conn)?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO writing_agent_suggestions
           (id, document_id, selection_anchor_json, source_revision, selected_content_hash,
            action_type, original_text, suggested_text, rationale, evidence_json,
            status, provider, model, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11, ?12, ?13)",
        rusqlite::params![
            input.id,
            input.document_id,
            input.selection_anchor_json,
            input.source_revision,
            input.selected_content_hash,
            input.action_type,
            input.original_text,
            input.suggested_text,
            input.rationale,
            input.evidence_json.unwrap_or_else(|| "{}".to_string()),
            input.provider,
            input.model,
            now
        ],
    )
    .map_err(|e| WritingError::sql("Failed to record the suggestion", e))?;
    load(conn, &input.id)
}

pub fn load(conn: &Connection, id: &str) -> WritingResult<SuggestionRow> {
    require_schema(conn)?;
    conn.query_row(
        "SELECT id, document_id, selection_anchor_json, source_revision, selected_content_hash,
                action_type, original_text, suggested_text, rationale, evidence_json,
                status, provider, model, created_at, resolved_at
           FROM writing_agent_suggestions WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            Ok(SuggestionRow {
                id: row.get(0)?,
                document_id: row.get(1)?,
                selection_anchor_json: row.get(2)?,
                source_revision: row.get(3)?,
                selected_content_hash: row.get(4)?,
                action_type: row.get(5)?,
                original_text: row.get(6)?,
                suggested_text: row.get(7)?,
                rationale: row.get(8)?,
                evidence_json: row.get(9)?,
                status: row.get(10)?,
                provider: row.get(11)?,
                model: row.get(12)?,
                created_at: row.get(13)?,
                resolved_at: row.get(14)?,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            WritingError::new(SUGGESTION_NOT_FOUND, format!("no suggestion {id}"))
        }
        other => WritingError::sql("Failed to read the suggestion", other),
    })
}

/// The proposals still waiting on this document.
pub fn pending(conn: &Connection, document_id: &str) -> WritingResult<Vec<SuggestionRow>> {
    require_schema(conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT id FROM writing_agent_suggestions
              WHERE document_id = ?1 AND status = 'pending'
              ORDER BY created_at",
        )
        .map_err(|e| WritingError::sql("Failed to prepare the pending query", e))?;
    let ids: Vec<String> = stmt
        .query_map([document_id], |row| row.get::<_, String>(0))
        .map_err(|e| WritingError::sql("Failed to read pending suggestions", e))?
        .collect::<Result<_, _>>()
        .map_err(|e| WritingError::sql("Failed to read a pending suggestion", e))?;

    ids.iter().map(|id| load(conn, id)).collect()
}

/// Resolves a suggestion, and says whether the text should now be applied.
///
/// `current_content_hash` is the hash of the text under the suggestion **as it
/// stands now**. It is compared before anything is written: a target that has
/// changed since the proposal was made means the agent was reasoning about
/// different words, and §14.2 asks for the proposal to be reviewed again rather
/// than applied to whatever happens to be there.
///
/// The status change is the operation. A second acceptance of the same
/// suggestion changes no row, and is told so — it applies nothing, which is
/// what keeps a double click from inserting a paragraph twice.
pub fn resolve(
    conn: &Connection,
    id: &str,
    status: &str,
    current_content_hash: Option<&str>,
) -> WritingResult<Resolution> {
    require_schema(conn)?;
    if !RESOLUTIONS.contains(&status) {
        return Err(WritingError::new(
            "invalid_resolution",
            format!(
                "unknown resolution {status:?}; expected one of {}",
                RESOLUTIONS.join(", ")
            ),
        ));
    }

    let suggestion = load(conn, id)?;

    // Only a resolution that writes into the manuscript needs its target to
    // still be there. Discarding a suggestion whose target moved is exactly
    // what someone would want to do about it.
    let writes = status != "discarded";
    if writes {
        if let Some(current) = current_content_hash {
            if current != suggestion.selected_content_hash {
                return Err(WritingError::new(
                    TARGET_CHANGED,
                    "the text this suggestion was made about has changed since",
                ));
            }
        }
    }

    let changed = conn
        .execute(
            "UPDATE writing_agent_suggestions
                SET status = ?1, resolved_at = ?2
              WHERE id = ?3 AND status = 'pending'",
            rusqlite::params![status, now_ms(), id],
        )
        .map_err(|e| WritingError::sql("Failed to resolve the suggestion", e))?;

    if changed == 0 {
        // Someone got here first. Not an error — the work is done — but the
        // caller must not apply the text again.
        let settled = load(conn, id)?;
        return Ok(Resolution {
            apply: false,
            status: settled.status,
            suggested_text: None,
        });
    }

    Ok(Resolution {
        apply: writes,
        status: status.to_string(),
        suggested_text: suggestion.suggested_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIGRATION_SQL: &str =
        include_str!("../../../../../packages/store/src/migrations/0035_writing_workspace.sql");

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn =
            crate::db::open::open_archive_connection(&dir.path().join("e.sqlite")).expect("open");
        conn.execute_batch(
            "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);",
        )
        .expect("minimal corpus");
        conn.execute_batch(&format!(
            "BEGIN IMMEDIATE;\n{MIGRATION_SQL}\nINSERT INTO _migrations (name, applied_at) VALUES ('0035_writing_workspace', 0);\nCOMMIT;"
        ))
        .expect("migrate");
        super::super::repository::create_document(
            &conn,
            super::super::repository::NewDocument {
                id: "d1".into(),
                title: "Articulo".into(),
                document_type: "article".into(),
                schema_version: 1,
                content_json: r#"{"type":"doc","content":[]}"#.into(),
            },
        )
        .expect("document");
        (dir, conn)
    }

    fn proposal(id: &str) -> NewSuggestion {
        NewSuggestion {
            id: id.to_string(),
            document_id: "d1".into(),
            selection_anchor_json: Some(r#"{"nodeId":"n1","offset":0}"#.into()),
            source_revision: 0,
            selected_content_hash: "hash-del-parrafo".into(),
            action_type: "improve_clarity".into(),
            original_text: Some("el texto original".into()),
            suggested_text: Some("el texto propuesto".into()),
            rationale: Some("mas claro".into()),
            evidence_json: Some(r#"{"corpus":[],"zotero":[]}"#.into()),
            provider: Some("openrouter".into()),
            model: Some("claude-sonnet-5".into()),
        }
    }

    /// §14.2: nothing is applied when a proposal is made. It waits to be read.
    #[test]
    fn a_recorded_proposal_is_pending_and_changes_nothing() {
        let (_dir, conn) = db();

        let row = record(&conn, proposal("s1")).expect("record");

        assert_eq!(row.status, "pending");
        assert_eq!(row.suggested_text.as_deref(), Some("el texto propuesto"));
        assert!(row.resolved_at.is_none());
    }

    #[test]
    fn accepting_says_to_apply_and_hands_back_the_text() {
        let (_dir, conn) = db();
        record(&conn, proposal("s1")).expect("record");

        let out = resolve(&conn, "s1", "accepted", Some("hash-del-parrafo")).expect("resolve");

        assert!(out.apply);
        assert_eq!(out.status, "accepted");
        assert_eq!(out.suggested_text.as_deref(), Some("el texto propuesto"));
    }

    /// The requirement §14.2 states outright: repeating a confirmed acceptance
    /// must not insert the text twice. A double click, a retried command and a
    /// stale panel all send the same acceptance again.
    #[test]
    fn accepting_twice_applies_once() {
        let (_dir, conn) = db();
        record(&conn, proposal("s1")).expect("record");

        let first = resolve(&conn, "s1", "accepted", Some("hash-del-parrafo")).expect("first");
        let second = resolve(&conn, "s1", "accepted", Some("hash-del-parrafo")).expect("second");

        assert!(first.apply, "the first acceptance did not apply");
        assert!(!second.apply, "the second acceptance applied again");
        assert_eq!(second.status, "accepted");
        assert!(
            second.suggested_text.is_none(),
            "the text was handed out twice"
        );
    }

    /// And a second resolution cannot change what the first decided.
    #[test]
    fn a_discard_after_an_acceptance_does_not_undo_it() {
        let (_dir, conn) = db();
        record(&conn, proposal("s1")).expect("record");
        resolve(&conn, "s1", "accepted", Some("hash-del-parrafo")).expect("accept");

        let out = resolve(&conn, "s1", "discarded", None).expect("discard");

        assert!(!out.apply);
        assert_eq!(out.status, "accepted");
        assert_eq!(load(&conn, "s1").expect("load").status, "accepted");
    }

    /// §14.2: a target that moved means the agent was reasoning about different
    /// words. The proposal is reviewed again rather than applied to whatever is
    /// there now.
    #[test]
    fn a_target_that_changed_refuses_to_apply() {
        let (_dir, conn) = db();
        record(&conn, proposal("s1")).expect("record");

        let error = resolve(&conn, "s1", "accepted", Some("otro-hash")).unwrap_err();

        assert_eq!(error.code, TARGET_CHANGED);
        // And it is still pending, so it can be reviewed rather than lost.
        assert_eq!(load(&conn, "s1").expect("load").status, "pending");
    }

    /// Discarding is what someone does *about* a target that moved, so it must
    /// not be the one thing they cannot do.
    #[test]
    fn a_suggestion_whose_target_moved_can_still_be_discarded() {
        let (_dir, conn) = db();
        record(&conn, proposal("s1")).expect("record");

        let out = resolve(&conn, "s1", "discarded", Some("otro-hash")).expect("discard");

        assert_eq!(out.status, "discarded");
        assert!(!out.apply);
    }

    /// Inserting below writes text too, so it is guarded the same way.
    #[test]
    fn inserting_below_applies_once_and_is_guarded_the_same() {
        let (_dir, conn) = db();
        record(&conn, proposal("s1")).expect("record");

        let first =
            resolve(&conn, "s1", "inserted_below", Some("hash-del-parrafo")).expect("first");
        let second =
            resolve(&conn, "s1", "inserted_below", Some("hash-del-parrafo")).expect("second");

        assert!(first.apply);
        assert!(!second.apply);
    }

    #[test]
    fn an_unknown_resolution_is_refused_by_name() {
        let (_dir, conn) = db();
        record(&conn, proposal("s1")).expect("record");

        let error = resolve(&conn, "s1", "aplicado_a_medias", None).unwrap_err();

        assert_eq!(error.code, "invalid_resolution");
    }

    #[test]
    fn a_suggestion_that_does_not_exist_is_reported_as_such() {
        let (_dir, conn) = db();

        assert_eq!(
            resolve(&conn, "s9", "accepted", None).unwrap_err().code,
            SUGGESTION_NOT_FOUND
        );
    }

    #[test]
    fn only_the_unresolved_ones_are_pending() {
        let (_dir, conn) = db();
        record(&conn, proposal("s1")).expect("s1");
        record(&conn, proposal("s2")).expect("s2");
        resolve(&conn, "s1", "discarded", None).expect("discard");

        let waiting = pending(&conn, "d1").expect("pending");

        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].id, "s2");
    }

    /// The evidence travels with the proposal: §14.2 requires the sources to be
    /// shown beside it, and a suggestion whose evidence was dropped cannot be
    /// judged.
    #[test]
    fn the_evidence_is_kept_with_the_proposal() {
        let (_dir, conn) = db();
        let mut input = proposal("s1");
        input.evidence_json = Some(r#"{"corpus":["as1"],"zotero":["ABCD1234"]}"#.into());

        let row = record(&conn, input).expect("record");

        assert!(row.evidence_json.contains("as1"));
        assert!(row.evidence_json.contains("ABCD1234"));
    }

    /// Model and provider are part of the record §14.5 asks for: a reader
    /// should be able to see which model wrote a sentence.
    #[test]
    fn the_model_and_provider_are_recorded() {
        let (_dir, conn) = db();

        let row = record(&conn, proposal("s1")).expect("record");

        assert_eq!(row.provider.as_deref(), Some("openrouter"));
        assert_eq!(row.model.as_deref(), Some("claude-sonnet-5"));
    }
}
