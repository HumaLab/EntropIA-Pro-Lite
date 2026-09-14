//! Durable storage behind the batch-processing queue (plan-lote.md §5).
//!
//! Every queue mutation in later units commits through this module on a
//! connection opened with [`crate::db::open::open_archive_connection`], so a
//! task receipt and its canonical result share one transaction. This file owns
//! the small read primitives Unidad 1 needs — migration presence, effective
//! PRAGMAs, and queue counters — plus the interface the scheduler builds on:
//! `prepare_batch`, `admit_or_attach`, `claim_next`, `save_checkpoint`,
//! `commit_success`, `finish_attempt`, `control_batch`, `recover_session` and
//! `read_snapshot` (those arrive in Unidades 2–3).

use rusqlite::Connection;

/// Migration that creates the processing tables. The frontend
/// `runMigrations()` applies it; the backend never runs DDL itself — it only
/// verifies presence before admitting or recovering work.
pub const MIGRATION_NAME: &str = "0032_batch_processing";

/// Error code returned when the queue schema is not applied yet. The frontend
/// treats it as a recoverable state (schema still migrating), never as a
/// per-item failure.
pub const SCHEMA_NOT_READY: &str = "schema_not_ready";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pragmas {
    pub journal_mode: String,
    pub synchronous: i64,
    pub foreign_keys: i64,
    pub busy_timeout: i64,
}

/// Reads the effective PRAGMAs of `conn`. Verification reads them back instead
/// of trusting the open call: a connection that silently lost one of these
/// must never admit queue work.
pub fn read_pragmas(conn: &Connection) -> Result<Pragmas, String> {
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read journal_mode: {e}"))?;
    let synchronous: i64 = conn
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read synchronous: {e}"))?;
    let foreign_keys: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read foreign_keys: {e}"))?;
    let busy_timeout: i64 = conn
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read busy_timeout: {e}"))?;
    Ok(Pragmas {
        journal_mode,
        synchronous,
        foreign_keys,
        busy_timeout,
    })
}

/// True when the durable queue is usable on this connection: WAL is active,
/// commits are FULL, foreign keys are enforced, and the migration row exists.
pub fn is_schema_ready(conn: &Connection) -> Result<bool, String> {
    let pragmas = read_pragmas(conn)?;
    if !pragmas.journal_mode.eq_ignore_ascii_case("wal") {
        return Ok(false);
    }
    if pragmas.synchronous != 2 || pragmas.foreign_keys != 1 {
        return Ok(false);
    }
    Ok(is_migration_applied(conn, MIGRATION_NAME)?)
}

/// True when `_migrations` records `name`. A missing tracking table means no
/// migration has ever run here, which is also "not applied" — not an error.
pub fn is_migration_applied(conn: &Connection, name: &str) -> Result<bool, String> {
    use rusqlite::OptionalExtension as _;
    let table: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to inspect sqlite_master: {e}"))?;
    if table.is_none() {
        return Ok(false);
    }
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM _migrations WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read _migrations: {e}"))?;
    Ok(count > 0)
}

/// Queue counters for the initialize response and the recovery banner.
/// Every count is a plain aggregate over the durable tables — no events, no
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueSummary {
    pub pending_batches: i64,
    pub active_tasks: i64,
    pub interrupted_tasks: i64,
    pub failed_tasks: i64,
    pub succeeded_tasks: i64,
}

fn count(conn: &Connection, sql: &str) -> Result<i64, String> {
    conn.query_row(sql, [], |row| row.get(0))
        .map_err(|e| format!("Failed to count queue rows: {e}"))
}

/// Reads the queue summary. Callers must check [`is_schema_ready`] first: on a
/// database without the migration these tables do not exist.
pub fn read_summary(conn: &Connection) -> Result<QueueSummary, String> {
    Ok(QueueSummary {
        pending_batches: count(
            conn,
            "SELECT COUNT(*) FROM processing_batches WHERE state IN ('preparing', 'ready', 'running', 'pausing', 'paused', 'interrupted')",
        )?,
        active_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state IN ('pending', 'blocked', 'running', 'retry_wait')",
        )?,
        interrupted_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state = 'interrupted'",
        )?,
        failed_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state = 'failed'",
        )?,
        succeeded_tasks: count(
            conn,
            "SELECT COUNT(*) FROM processing_tasks WHERE state = 'succeeded'",
        )?,
    })
}
/// Task states that still own the work unit. Anything else is history: a new
/// admission after a terminal state transitions that same row (new
/// retry_cycle), never inserts a second live writer — the partial unique
/// index enforces it.
pub const TERMINAL_TASK_STATES: &[&str] = &["succeeded", "failed", "skipped", "cancelled"];

/// Operations a batch asked for, parsed from `processing_batches.operations`
/// (JSON array of `"ocr"` / `"embeddings"`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchOperations {
    pub ocr: bool,
    pub embeddings: bool,
}
pub fn batch_operations(conn: &Connection, batch_id: &str) -> Result<BatchOperations, String> {
    let raw: String = conn
        .query_row(
            "SELECT operations FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read batch {batch_id}: {e}"))?;
    let mut ops = BatchOperations::default();
    if let Ok(list) = serde_json::from_str::<Vec<String>>(&raw) {
        for entry in list {
            match entry.as_str() {
                "ocr" => ops.ocr = true,
                "embeddings" => ops.embeddings = true,
                _ => {}
            }
        }
    }
    Ok(ops)
}

/// Snapshots the batch scope: one member row per asset currently in the given
/// collections. A single `INSERT ... SELECT` statement, so concurrent imports
/// cannot leak half a scope into the batch — assets added afterwards belong
/// to a later batch. Returns the number of members added.
#[allow(dead_code)]
pub fn prepare_membership(
    conn: &Connection,
    batch_id: &str,
    collection_ids: &[String],
) -> Result<usize, String> {
    if collection_ids.is_empty() {
        return Ok(0);
    }
    let scope = serde_json::to_string(collection_ids)
        .map_err(|e| format!("Failed to encode batch scope: {e}"))?;
    let added = conn
        .execute(
            "INSERT OR IGNORE INTO processing_batch_members
               (batch_id, ordinal, asset_id_snapshot, item_id_snapshot, collection_id_snapshot, title_snapshot)
             SELECT ?1, ROW_NUMBER() OVER (ORDER BY a.created_at, a.id) - 1,
                    a.id, a.item_id, i.collection_id, i.title
             FROM assets a JOIN items i ON i.id = a.item_id
             WHERE i.collection_id IN (SELECT value FROM json_each(?2))",
            rusqlite::params![batch_id, scope],
        )
        .map_err(|e| format!("Failed to snapshot scope of batch {batch_id}: {e}"))?;
    Ok(added as usize)
}

/// Outcome of admitting one work unit for one batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmitOutcome {
    pub task_id: String,
    /// False when another batch (or an earlier page) already owned the unit
    /// and this batch only attached to it.
    pub created: bool,
}

fn live_task(conn: &Connection, kind: &str, asset_id: &str) -> Result<Option<String>, String> {
    use rusqlite::OptionalExtension as _;
    // Single source of truth with TERMINAL_TASK_STATES: the literals below
    // must stay in sync with the partial unique index, so they are built
    // from the constant instead of repeated by hand.
    let excluded = TERMINAL_TASK_STATES
        .iter()
        .map(|state| format!("'{state}'"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.query_row(
        &format!(
            "SELECT id FROM processing_tasks WHERE kind = ?1 AND asset_id_snapshot = ?2 AND state NOT IN ({excluded})"
        ),
        rusqlite::params![kind, asset_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| format!("Failed to look up live {kind} task for {asset_id}: {e}"))
}

fn link_batch_task(
    conn: &Connection,
    batch_id: &str,
    task_id: &str,
    kind: &str,
    asset_id: &str,
    dependency_task_id: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO processing_batch_tasks
           (batch_id, task_id, kind, asset_id_snapshot, request_state, dependency_task_id)
         VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
        rusqlite::params![batch_id, task_id, kind, asset_id, dependency_task_id],
    )
    .map_err(|e| format!("Failed to link task {task_id} to batch {batch_id}: {e}"))?;
    Ok(())
}

/// Admits one work unit for one batch, or attaches the batch to the live unit
/// another batch already owns. Two batches over the same asset share one
/// physical task and never start two workers on it; per-batch pause/cancel
/// only flips the link's `request_state` (Unidad 3).
#[allow(dead_code)]
pub fn admit_or_attach(
    conn: &Connection,
    batch_id: &str,
    kind: &str,
    asset_id: &str,
    input_revision: i64,
    input_fingerprint: &str,
    contract_hash: &str,
    dependency_task_id: Option<&str>,
) -> Result<AdmitOutcome, String> {
    if let Some(task_id) = live_task(conn, kind, asset_id)? {
        link_batch_task(conn, batch_id, &task_id, kind, asset_id, dependency_task_id)?;
        return Ok(AdmitOutcome {
            task_id,
            created: false,
        });
    }
    // Deterministic id: re-running a crashed classification page re-issues the
    // same INSERT, which the partial unique index turns into a lookup.
    let task_id = format!("{kind}-{asset_id}");
    let state = if dependency_task_id.is_some() {
        "blocked"
    } else {
        "pending"
    };
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO processing_tasks
               (id, kind, asset_id_snapshot, input_revision, input_fingerprint, contract_hash, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%s', 'now') * 1000, strftime('%s', 'now') * 1000)",
            rusqlite::params![
                task_id,
                kind,
                asset_id,
                input_revision,
                input_fingerprint,
                contract_hash,
                state
            ],
        )
        .map_err(|e| format!("Failed to admit {kind} task for {asset_id}: {e}"))?;
    if inserted == 0 {
        // Lost a race with a concurrent admitter (or a terminal row for the
        // same unit exists): fall back to the live row when there is one.
        if let Some(existing) = live_task(conn, kind, asset_id)? {
            link_batch_task(
                conn,
                batch_id,
                &existing,
                kind,
                asset_id,
                dependency_task_id,
            )?;
            return Ok(AdmitOutcome {
                task_id: existing,
                created: false,
            });
        }
        return Err(format!(
            "A terminal {kind} task already exists for {asset_id}; requeue it through an explicit retry"
        ));
    }
    link_batch_task(conn, batch_id, &task_id, kind, asset_id, dependency_task_id)?;
    Ok(AdmitOutcome {
        task_id,
        created: true,
    })
}

/// Cheap identity of the OCR input: asset id, stored path, and byte size.
/// v1 never regenerates finished OCR, so this only has to catch a source
/// swap before a claim or COMMIT — not invisible same-size rewrites.
fn ocr_fingerprint(conn: &Connection, asset_id: &str) -> Result<Option<String>, String> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT id || '|' || path || '|' || COALESCE(size, -1) FROM assets WHERE id = ?1",
        [asset_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| format!("Failed to fingerprint {asset_id}: {e}"))
}

fn source_revision(conn: &Connection, asset_id: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT source_revision FROM processing_asset_revisions WHERE asset_id = ?1",
        [asset_id],
        |row| row.get(0),
    )
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(0),
        other => Err(format!("Failed to read revision of {asset_id}: {other}")),
    })
}

/// One classified member: what the batch will do about it, if anything.
#[derive(Debug, Clone)]
struct MemberWork {
    ordinal: i64,
    asset_id: String,
    classification: &'static str,
    reason: String,
    ocr_task: Option<(String, String)>,
    emb_task: Option<(String, String, Option<String>)>,
}

/// Progress of one classification page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifyPageOutcome {
    pub processed: usize,
    pub admitted: usize,
    /// True when no members remain unclassified.
    pub done: bool,
}

/// Classifies the next page of snapshot members and admits the needed tasks.
/// Read phase (decisions + fingerprints) runs outside the write transaction;
/// the page then commits atomically — member rows, task rows, links, cursor —
/// so a crash replays the page without duplicates. `page_size` 200 matches
/// the plan; callers repeat until `done`.
#[allow(dead_code)]
pub fn classify_batch_page(
    conn: &Connection,
    batch_id: &str,
    page_size: usize,
) -> Result<ClassifyPageOutcome, String> {
    let ops = batch_operations(conn, batch_id)?;
    let cursor: i64 = conn
        .query_row(
            "SELECT planning_cursor FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read cursor of batch {batch_id}: {e}"))?;
    let planning_done: i64 = conn
        .query_row(
            "SELECT planning_done FROM processing_batches WHERE id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read planning state of batch {batch_id}: {e}"))?;
    if planning_done == 1 {
        return Ok(ClassifyPageOutcome {
            processed: 0,
            admitted: 0,
            done: true,
        });
    }
    let mut stmt = conn
        .prepare(
            "SELECT ordinal, asset_id_snapshot FROM processing_batch_members
             WHERE batch_id = ?1 AND ordinal >= ?2 ORDER BY ordinal LIMIT ?3",
        )
        .map_err(|e| format!("Failed to read members of batch {batch_id}: {e}"))?;
    let page = stmt
        .query_map(
            rusqlite::params![batch_id, cursor, page_size as i64],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|e| format!("Failed to read members of batch {batch_id}: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read members of batch {batch_id}: {e}"))?;
    drop(stmt);
    if page.is_empty() {
        conn.execute(
            "UPDATE processing_batches SET planning_done = 1, updated_at = strftime('%s', 'now') * 1000 WHERE id = ?1",
            [batch_id],
        )
        .map_err(|e| format!("Failed to close planning of batch {batch_id}: {e}"))?;
        return Ok(ClassifyPageOutcome {
            processed: 0,
            admitted: 0,
            done: true,
        });
    }
    // Read phase: eligibility verdicts for every member, no write lock held.
    let mut works = Vec::with_capacity(page.len());
    for (ordinal, asset_id) in &page {
        works.push(plan_member(conn, *ordinal, asset_id, &ops)?);
    }
    // Write phase: one atomic page.
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("Failed to begin classification page: {e}"))?;
    let outcome = (|| -> Result<ClassifyPageOutcome, String> {
        let mut admitted = 0;
        let mut last_ordinal = cursor;
        for work in &works {
            if let Some((kind, fingerprint)) = &work.ocr_task {
                let revision = source_revision(conn, &work.asset_id)?;
                let out = admit_or_attach(
                    conn,
                    batch_id,
                    kind,
                    &work.asset_id,
                    revision,
                    fingerprint,
                    "",
                    None,
                )?;
                if out.created {
                    admitted += 1;
                }
            }
            if let Some((kind, fingerprint, dependency)) = &work.emb_task {
                let revision = source_revision(conn, &work.asset_id)?;
                let out = admit_or_attach(
                    conn,
                    batch_id,
                    kind,
                    &work.asset_id,
                    revision,
                    fingerprint,
                    &super::eligibility::current_embedding_contract_hash(),
                    dependency.as_deref(),
                )?;
                if out.created {
                    admitted += 1;
                }
            }
            conn.execute(
                "UPDATE processing_batch_members SET classification = ?1, reason = ?2
                 WHERE batch_id = ?3 AND ordinal = ?4",
                rusqlite::params![work.classification, work.reason, batch_id, work.ordinal],
            )
            .map_err(|e| format!("Failed to classify member {}: {e}", work.ordinal))?;
            last_ordinal = work.ordinal;
        }
        conn.execute(
            "UPDATE processing_batches SET planning_cursor = ?1, updated_at = strftime('%s', 'now') * 1000 WHERE id = ?2",
            rusqlite::params![last_ordinal + 1, batch_id],
        )
        .map_err(|e| format!("Failed to advance cursor of batch {batch_id}: {e}"))?;
        Ok(ClassifyPageOutcome {
            processed: works.len(),
            admitted,
            done: false,
        })
    })();
    match outcome {
        Ok(done) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| format!("Failed to commit classification page: {e}"))?;
            Ok(done)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Decides one member: eligibility per selected operation, task admissions to
/// issue, and the durable classification for the batch preview.
fn plan_member(
    conn: &Connection,
    ordinal: i64,
    asset_id: &str,
    ops: &BatchOperations,
) -> Result<MemberWork, String> {
    let mut ocr_task = None;
    let mut emb_task = None;
    let mut notes: Vec<String> = Vec::new();
    // OCR first: an eligible OCR task becomes the embedding dependency below.
    let mut ocr_open: Option<String> = None;
    if ops.ocr {
        match super::eligibility::ocr_decision(conn, asset_id)? {
            super::eligibility::OcrDecision::Eligible => match ocr_fingerprint(conn, asset_id)? {
                Some(fingerprint) => {
                    ocr_task = Some(("ocr".to_string(), fingerprint));
                    ocr_open = Some(format!("ocr-{asset_id}"));
                    notes.push("ocr:admit".to_string());
                }
                None => notes.push("ocr:source_missing".to_string()),
            },
            super::eligibility::OcrDecision::AlreadyDone { method } => {
                notes.push(format!("ocr:already_done({method})"));
            }
            super::eligibility::OcrDecision::UnsupportedType { asset_type } => {
                notes.push(format!("ocr:unsupported({asset_type})"));
            }
            super::eligibility::OcrDecision::ParentHasPages { page_count } => {
                notes.push(format!("ocr:parent_has_pages({page_count})"));
            }
        }
    }
    if ops.embeddings {
        match super::eligibility::embedding_decision(conn, asset_id)? {
            super::eligibility::EmbeddingDecision::Eligible { reason } => {
                // Sources are non-empty here (empty text reports NoSourceText
                // below), so the fingerprint pins the real input set.
                let fingerprint = super::eligibility::embedding_input_fingerprint(conn, asset_id)?;
                let _ = reason;
                emb_task = Some(("embedding".to_string(), fingerprint, None));
                notes.push("embeddings:admit".to_string());
            }
            super::eligibility::EmbeddingDecision::Fresh => {
                notes.push("embeddings:fresh".to_string());
            }
            super::eligibility::EmbeddingDecision::NoSourceText => {
                if ocr_open.is_some() {
                    emb_task = Some(("embedding".to_string(), String::new(), ocr_open.clone()));
                    notes.push("embeddings:wait_for_ocr".to_string());
                } else {
                    notes.push("embeddings:no_source_text".to_string());
                }
            }
        }
    }
    let classification = if ocr_task.is_some() || emb_task.is_some() {
        "admitted"
    } else if notes.iter().any(|n| n.starts_with("ocr:unsupported")) {
        "unsupported_type"
    } else if notes.iter().any(|n| n.starts_with("ocr:parent_has_pages")) {
        "parent_has_pages"
    } else if notes
        .iter()
        .any(|n| n.starts_with("embeddings:no_source_text"))
    {
        "no_source_text"
    } else {
        "already_done"
    };
    Ok(MemberWork {
        ordinal,
        asset_id: asset_id.to_string(),
        classification,
        reason: notes.join("; "),
        ocr_task,
        emb_task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mirrored migration file, exercised here so registry/file drift
    /// breaks a test instead of reaching a user database.
    const MIGRATION_SQL: &str =
        include_str!("../../../../../packages/store/src/migrations/0032_batch_processing.sql");

    fn migrated_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("entropia.sqlite");
        let conn = crate::db::open::open_archive_connection(&db_path).expect("open");
        conn.execute_batch("CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
            .expect("minimal collections");
        conn.execute_batch("CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, metadata TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
            .expect("minimal items");
        conn.execute_batch("CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, sort_index INTEGER NOT NULL DEFAULT 0, size INTEGER, parent_asset_id TEXT REFERENCES assets(id) ON DELETE CASCADE, page_number INTEGER, created_at INTEGER NOT NULL);")
            .expect("minimal assets");
        conn.execute_batch("CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, method TEXT NOT NULL, confidence REAL, created_at INTEGER NOT NULL);")
            .expect("minimal extractions");
        conn.execute_batch("CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, language TEXT, duration_ms INTEGER, model TEXT NOT NULL, segments TEXT, confidence REAL, created_at INTEGER NOT NULL);")
            .expect("minimal transcriptions");
        conn.execute_batch("CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);")
            .expect("migrations tracking");
        // Same single-batch application the frontend runner uses for 0032.
        conn.execute_batch(&format!("BEGIN IMMEDIATE;\n{MIGRATION_SQL}\nCOMMIT;"))
            .expect("apply 0032 mirror");
        conn.execute(
            "INSERT INTO _migrations (name, applied_at) VALUES (?1, 1)",
            [MIGRATION_NAME],
        )
        .expect("track 0032");
        (dir, conn)
    }

    #[test]
    fn an_unmigrated_database_is_not_schema_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = crate::db::open::open_archive_connection(&dir.path().join("entropia.sqlite"))
            .expect("open");
        assert_eq!(
            is_schema_ready(&conn).expect("check"),
            false,
            "no migration row, no tables: the gate must stay closed"
        );
    }

    #[test]
    fn the_mirrored_migration_applies_and_opens_the_gate() {
        let (_dir, conn) = migrated_db();
        assert!(
            is_schema_ready(&conn).expect("check"),
            "tables + tracking row + durable PRAGMAs must read ready"
        );
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'processing_%' ORDER BY name")
            .expect("tables query")
            .query_map([], |row| row.get(0))
            .expect("map")
            .collect::<Result<_, _>>()
            .expect("collect");
        assert_eq!(tables.len(), 9, "all nine processing tables: {tables:?}");
    }

    #[test]
    fn a_committed_task_survives_closing_and_reopening_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("entropia.sqlite");
        {
            let conn = crate::db::open::open_archive_connection(&db_path).expect("open");
            conn.execute_batch("CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
                .expect("collections");
            conn.execute_batch("CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL, collection_id TEXT NOT NULL, metadata TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);")
                .expect("items");
            conn.execute_batch("CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, created_at INTEGER NOT NULL);")
                .expect("assets");
            conn.execute_batch("CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, method TEXT NOT NULL, confidence REAL, created_at INTEGER NOT NULL);")
                .expect("extractions");
            conn.execute_batch("CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE, text_content TEXT NOT NULL, language TEXT, duration_ms INTEGER, model TEXT NOT NULL, segments TEXT, confidence REAL, created_at INTEGER NOT NULL);")
                .expect("transcriptions");
            conn.execute_batch("CREATE TABLE _migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, applied_at INTEGER NOT NULL);")
                .expect("tracking");
            conn.execute_batch(&format!("BEGIN IMMEDIATE;\n{MIGRATION_SQL}\nCOMMIT;"))
                .expect("apply 0032");
            conn.execute(
                "INSERT INTO _migrations (name, applied_at) VALUES (?1, 1)",
                [MIGRATION_NAME],
            )
            .expect("track");
            conn.execute(
                "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t1', 'ocr', 'a1', 'succeeded', 1, 1)",
                [],
            )
            .expect("insert task");
        }
        // Process "death": drop every connection, then reopen the same file.
        let reopened = crate::db::open::open_archive_connection(&db_path).expect("reopen");
        assert!(is_schema_ready(&reopened).expect("check"));
        let state: String = reopened
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = 't1'",
                [],
                |row| row.get(0),
            )
            .expect("task survived the restart");
        assert_eq!(state, "succeeded");
    }

    #[test]
    fn the_active_task_exclusion_rejects_a_second_live_writer() {
        let (_dir, conn) = migrated_db();
        conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t1', 'ocr', 'a1', 'pending', 1, 1)",
            [],
        )
        .expect("first admission");
        let second = conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t2', 'ocr', 'a1', 'pending', 1, 1)",
            [],
        );
        assert!(
            second.is_err(),
            "two live rows for one operation+asset would let two workers compute twice"
        );
    }

    #[test]
    fn source_text_writes_bump_the_asset_revision() {
        let (_dir, conn) = migrated_db();
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('a1', 'i1', 'p', 'image', 1)",
            [],
        )
        .expect("asset");
        conn.execute(
            "INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES ('e1', 'a1', 'hola', 'ocr', 1)",
            [],
        )
        .expect("extraction");
        let rev: i64 = conn
            .query_row(
                "SELECT source_revision FROM processing_asset_revisions WHERE asset_id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("revision bumped by the trigger");
        assert_eq!(rev, 1);
        conn.execute(
            "UPDATE extractions SET text_content = 'hola mundo' WHERE id = 'e1'",
            [],
        )
        .expect("edit");
        let rev: i64 = conn
            .query_row(
                "SELECT source_revision FROM processing_asset_revisions WHERE asset_id = 'a1'",
                [],
                |row| row.get(0),
            )
            .expect("revision bumped again");
        assert_eq!(rev, 2);
    }

    #[test]
    fn invalid_states_are_rejected_by_check_constraints() {
        let (_dir, conn) = migrated_db();
        let bad = conn.execute(
            "INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES ('t9', 'ocr', 'a9', 'finished', 1, 1)",
            [],
        );
        assert!(
            bad.is_err(),
            "'finished' is not a task state: the UI must never invent one"
        );
    }
    // ── Unidad 2: admission, eligibility, invalidation ──────────────────────

    fn batch_db() -> (tempfile::TempDir, Connection) {
        use crate::nlp::embeddings as emb;
        let (dir, conn) = migrated_db();
        conn.execute_batch(
            "CREATE TABLE vec_assets(asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL,
               embedding BLOB NOT NULL, embedding_model TEXT NOT NULL DEFAULT 'legacy',
               embedding_contract TEXT NOT NULL DEFAULT 'legacy',
               dimensions INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE rag_chunks(id TEXT PRIMARY KEY, asset_id TEXT NOT NULL,
               item_id TEXT NOT NULL, source_kind TEXT NOT NULL, source_id TEXT NOT NULL,
               chunk_ordinal INTEGER NOT NULL, text_content TEXT NOT NULL,
               start_char INTEGER NOT NULL, end_char INTEGER NOT NULL,
               source_text_hash TEXT NOT NULL, chunking_contract TEXT NOT NULL,
               embedding BLOB NOT NULL, embedding_model TEXT NOT NULL,
               embedding_contract TEXT NOT NULL, dimensions INTEGER NOT NULL);",
        )
        .expect("embedding tables");
        conn.execute(
            "INSERT INTO collections (id, name, created_at, updated_at) VALUES
               ('c1', 'legajo', 1, 1), ('c2', 'fotos', 2, 2)",
            [],
        )
        .expect("collections");
        conn.execute(
            "INSERT INTO items (id, title, collection_id, created_at, updated_at) VALUES
               ('i1', 'doc', 'c1', 1, 1), ('i2', 'foto', 'c2', 2, 2)",
            [],
        )
        .expect("items");
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES
               ('a1', 'i1', 'a1.png', 'image', 1),
               ('a2', 'i1', 'a2.png', 'image', 2),
               ('a3', 'i1', 'a3.mp3', 'audio', 3),
               ('a4', 'i1', 'a4.pdf', 'pdf', 4),
               ('a5', 'i1', 'a5.pdf', 'pdf', 5),
               ('a5p1', 'i1', 'a5p1.png', 'image', 6),
               ('a5p2', 'i1', 'a5p2.png', 'image', 7),
               ('a6', 'i2', 'a6.png', 'image', 8),
               ('a7', 'i1', 'a7.png', 'image', 9)",
            [],
        )
        .expect("assets");
        conn.execute(
            "UPDATE assets SET parent_asset_id = 'a5', page_number = 1 WHERE id = 'a5p1'",
            [],
        )
        .expect("page 1");
        conn.execute(
            "UPDATE assets SET parent_asset_id = 'a5', page_number = 2 WHERE id = 'a5p2'",
            [],
        )
        .expect("page 2");
        let short_text = "hola mundo, texto con contenido suficiente".to_string();
        let long_text = "lorem ipsum ".repeat(200);
        conn.execute(
            "INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES
               ('e2', 'a2', ?1, 'ocr', 1),
               ('e4', 'a4', 'texto nativo del pdf', 'native', 1),
               ('e5p2', 'a5p2', 'texto de la pagina dos', 'ocr', 1),
               ('e7', 'a7', ?2, 'ocr', 1)",
            rusqlite::params![short_text, long_text],
        )
        .expect("extractions");
        conn.execute(
            "INSERT INTO transcriptions (id, asset_id, text_content, model, created_at)
             VALUES ('t6', 'a6', 'texto hablado en el audio', 'whisper', 1)",
            [],
        )
        .expect("transcriptions");
        // a2: current vector + complete chunk set → Fresh.
        conn.execute(
            "INSERT INTO vec_assets (asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions)
             VALUES ('a2', 'i1', zeroblob(16), ?1, ?2, ?3)",
            rusqlite::params![
                emb::CANONICAL_EMBEDDING_MODEL,
                emb::CANONICAL_EMBEDDING_CONTRACT_V1,
                emb::CANONICAL_EMBEDDING_DIMENSIONS as i64,
            ],
        )
        .expect("a2 vector");
        insert_matching_chunks(&conn, "a2", "i1", "e2", &short_text);
        // a6: legacy vector → ContractMismatch.
        conn.execute(
            "INSERT INTO vec_assets (asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions)
             VALUES ('a6', 'i2', zeroblob(16), 'legacy', 'legacy', 0)",
            [],
        )
        .expect("a6 legacy vector");
        // a7: current vector but no chunks → IncompleteChunks.
        conn.execute(
            "INSERT INTO vec_assets (asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions)
             VALUES ('a7', 'i1', zeroblob(16), ?1, ?2, ?3)",
            rusqlite::params![
                emb::CANONICAL_EMBEDDING_MODEL,
                emb::CANONICAL_EMBEDDING_CONTRACT_V1,
                emb::CANONICAL_EMBEDDING_DIMENSIONS as i64,
            ],
        )
        .expect("a7 vector");
        (dir, conn)
    }

    fn insert_matching_chunks(
        conn: &Connection,
        asset_id: &str,
        item_id: &str,
        source_id: &str,
        text: &str,
    ) {
        use crate::nlp::embeddings as emb;
        let source = emb::RagChunkSource {
            asset_id: asset_id.to_string(),
            item_id: item_id.to_string(),
            source_kind: emb::RagChunkSourceKind::Extraction,
            source_id: source_id.to_string(),
            text: text.to_string(),
        };
        for draft in emb::plan_rag_chunks(&source).expect("plan fixture chunks") {
            conn.execute(
                "INSERT INTO rag_chunks (id, asset_id, item_id, source_kind, source_id, chunk_ordinal,
                   text_content, start_char, end_char, source_text_hash, chunking_contract,
                   embedding, embedding_model, embedding_contract, dimensions)
                 VALUES (?1, ?2, ?3, 'extraction', ?4, ?5, ?6, ?7, ?8, ?9, ?10, zeroblob(8), ?11, ?12, ?13)",
                rusqlite::params![
                    draft.id,
                    draft.asset_id,
                    draft.item_id,
                    draft.source_id,
                    draft.chunk_ordinal as i64,
                    draft.text_content,
                    draft.start_char as i64,
                    draft.end_char as i64,
                    draft.source_text_hash,
                    draft.chunking_contract,
                    emb::CANONICAL_EMBEDDING_MODEL,
                    emb::CANONICAL_EMBEDDING_CONTRACT_V1,
                    emb::CANONICAL_EMBEDDING_DIMENSIONS as i64,
                ],
            )
            .expect("insert fixture chunk");
        }
    }

    fn insert_batch(conn: &Connection, id: &str, request: &str, ops: &str) {
        conn.execute(
            "INSERT INTO processing_batches (id, request_id, origin, state, desired_state, operations, created_at, updated_at)
             VALUES (?1, ?2, 'user', 'preparing', 'run', ?3, 1, 1)",
            rusqlite::params![id, request, ops],
        )
        .expect("insert batch");
    }

    fn task_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM processing_tasks", [], |row| {
            row.get(0)
        })
        .expect("count tasks")
    }

    fn member_class(conn: &Connection, batch: &str, asset: &str) -> (String, String) {
        conn.query_row(
            "SELECT classification, reason FROM processing_batch_members WHERE batch_id = ?1 AND asset_id_snapshot = ?2",
            rusqlite::params![batch, asset],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read member")
    }

    #[test]
    fn ocr_rule_distinguishes_missing_done_unsupported_and_parents() {
        use super::super::eligibility::{ocr_decision, OcrDecision};
        let (_dir, conn) = batch_db();
        assert_eq!(
            ocr_decision(&conn, "a1").expect("a1"),
            OcrDecision::Eligible
        );
        assert!(matches!(
            ocr_decision(&conn, "a2").expect("a2"),
            OcrDecision::AlreadyDone { .. }
        ));
        // Native extraction satisfies the text need: no rasterization.
        assert!(matches!(
            ocr_decision(&conn, "a4").expect("a4"),
            OcrDecision::AlreadyDone { .. }
        ));
        assert!(matches!(
            ocr_decision(&conn, "a3").expect("a3"),
            OcrDecision::UnsupportedType { .. }
        ));
        assert!(matches!(
            ocr_decision(&conn, "a5").expect("a5"),
            OcrDecision::ParentHasPages { page_count: 2 }
        ));
        assert_eq!(
            ocr_decision(&conn, "a5p1").expect("a5p1"),
            OcrDecision::Eligible
        );
    }

    #[test]
    fn empty_successful_ocr_is_done_not_missing() {
        use super::super::eligibility::{ocr_decision, OcrDecision};
        let (_dir, conn) = batch_db();
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('ae', 'i1', 'ae.png', 'image', 10)",
            [],
        )
        .expect("asset");
        conn.execute(
            "INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES ('ee', 'ae', '', 'ocr', 1)",
            [],
        )
        .expect("empty extraction");
        // Empty text with a canonical row means "recognized nothing", not
        // "never ran": requeueing it would loop forever on blank pages.
        assert!(matches!(
            ocr_decision(&conn, "ae").expect("ae"),
            OcrDecision::AlreadyDone { .. }
        ));
    }

    #[test]
    fn embedding_rule_covers_fresh_stale_and_textless() {
        use super::super::eligibility::{
            embedding_decision, EmbeddingDecision, EmbeddingStaleReason,
        };
        let (_dir, conn) = batch_db();
        assert_eq!(
            embedding_decision(&conn, "a2").expect("a2"),
            EmbeddingDecision::Fresh
        );
        assert_eq!(
            embedding_decision(&conn, "a1").expect("a1"),
            EmbeddingDecision::NoSourceText
        );
        assert_eq!(
            embedding_decision(&conn, "a4").expect("a4"),
            EmbeddingDecision::Eligible {
                reason: EmbeddingStaleReason::MissingVector
            }
        );
        assert_eq!(
            embedding_decision(&conn, "a6").expect("a6"),
            EmbeddingDecision::Eligible {
                reason: EmbeddingStaleReason::ContractMismatch
            }
        );
        assert_eq!(
            embedding_decision(&conn, "a7").expect("a7"),
            EmbeddingDecision::Eligible {
                reason: EmbeddingStaleReason::IncompleteChunks
            }
        );
    }

    #[test]
    fn editing_source_text_invalidates_fresh_embeddings() {
        use super::super::eligibility::{
            embedding_decision, EmbeddingDecision, EmbeddingStaleReason,
        };
        let (_dir, conn) = batch_db();
        // Simulate Unidad 4 completion: the embedding run records which
        // revision it published.
        conn.execute(
            "UPDATE processing_asset_revisions SET embedding_completed_revision = source_revision WHERE asset_id = 'a2'",
            [],
        )
        .expect("completion marker");
        assert_eq!(
            embedding_decision(&conn, "a2").expect("a2 fresh"),
            EmbeddingDecision::Fresh
        );
        // The invalidation trigger (0032) bumps the revision in the same
        // transaction as the text edit — no UI refresh needed.
        conn.execute(
            "UPDATE extractions SET text_content = 'texto corregido' WHERE id = 'e2'",
            [],
        )
        .expect("edit source");
        assert_eq!(
            embedding_decision(&conn, "a2").expect("a2 stale"),
            EmbeddingDecision::Eligible {
                reason: EmbeddingStaleReason::SourceChanged
            }
        );
    }

    #[test]
    fn prepare_and_classify_admits_exactly_the_missing_work() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr", "embeddings"]"#);
        let added = prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare");
        assert_eq!(added, 8, "every c1 asset snapshotted exactly once");
        let page = classify_batch_page(&conn, "b1", 200).expect("classify");
        assert_eq!(page.processed, 8);
        assert!(!page.done, "done flips on the draining call");
        let drain = classify_batch_page(&conn, "b1", 200).expect("drain");
        assert!(drain.done);
        // OCR tasks: a1, a5p1. Embeddings: a1(wait), a4, a5p1(wait), a5p2, a7.
        assert_eq!(task_count(&conn), 7);
        let (class_a2, _) = member_class(&conn, "b1", "a2");
        assert_eq!(class_a2, "already_done");
        let (class_a3, _) = member_class(&conn, "b1", "a3");
        assert_eq!(class_a3, "unsupported_type");
        let (class_a5, _) = member_class(&conn, "b1", "a5");
        assert_eq!(class_a5, "parent_has_pages");
        let (class_a1, reason_a1) = member_class(&conn, "b1", "a1");
        assert_eq!(class_a1, "admitted");
        assert!(reason_a1.contains("ocr:admit"), "{reason_a1}");
        // The embedding blocked on OCR carries the dependency link.
        let dep: Option<String> = conn
            .query_row(
                "SELECT dependency_task_id FROM processing_batch_tasks WHERE batch_id = 'b1' AND task_id = 'embedding-a1'",
                [],
                |row| row.get(0),
            )
            .expect("dependency link");
        assert_eq!(dep.as_deref(), Some("ocr-a1"));
        let state: String = conn
            .query_row(
                "SELECT state FROM processing_tasks WHERE id = 'embedding-a1'",
                [],
                |row| row.get(0),
            )
            .expect("blocked state");
        assert_eq!(state, "blocked");
    }

    #[test]
    fn overlapping_batches_share_one_physical_task() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr", "embeddings"]"#);
        prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare b1");
        classify_batch_page(&conn, "b1", 200).expect("classify b1");
        classify_batch_page(&conn, "b1", 200).expect("drain b1");
        assert_eq!(task_count(&conn), 7);
        // Second batch over both collections: c1 work attaches, a6 creates.
        insert_batch(&conn, "b2", "req-2", r#"["ocr", "embeddings"]"#);
        prepare_membership(&conn, "b2", &["c1".to_string(), "c2".to_string()]).expect("prepare b2");
        classify_batch_page(&conn, "b2", 200).expect("classify b2");
        classify_batch_page(&conn, "b2", 200).expect("drain b2");
        // Only a6's two tasks are new; everything else attached.
        assert_eq!(task_count(&conn), 9);
        let links: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_batch_tasks WHERE task_id = 'ocr-a1'",
                [],
                |row| row.get(0),
            )
            .expect("shared links");
        assert_eq!(links, 2);
    }

    #[test]
    fn crashed_classification_replays_without_duplicates() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr", "embeddings"]"#);
        prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare");
        let first = classify_batch_page(&conn, "b1", 3).expect("page 1");
        assert_eq!(first.processed, 3);
        assert!(!first.done);
        let tasks_after_crash = task_count(&conn);
        // Crash replay: the cursor never confirmed, so the same page runs
        // again. Deterministic ids + live-task lookup make it idempotent.
        conn.execute(
            "UPDATE processing_batches SET planning_cursor = 0 WHERE id = 'b1'",
            [],
        )
        .expect("simulate lost cursor");
        let replay = classify_batch_page(&conn, "b1", 3).expect("replay");
        assert_eq!(replay.processed, 3);
        assert_eq!(task_count(&conn), tasks_after_crash);
        let members: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_batch_members WHERE batch_id = 'b1'",
                [],
                |row| row.get(0),
            )
            .expect("members stable");
        assert_eq!(members, 8);
    }

    #[test]
    fn new_assets_after_the_snapshot_belong_to_a_later_batch() {
        let (_dir, conn) = batch_db();
        insert_batch(&conn, "b1", "req-1", r#"["ocr"]"#);
        prepare_membership(&conn, "b1", &["c1".to_string()]).expect("prepare");
        conn.execute(
            "INSERT INTO assets (id, item_id, path, type, created_at) VALUES ('a8', 'i1', 'a8.png', 'image', 10)",
            [],
        )
        .expect("late asset");
        // Classification only walks the snapshotted members.
        classify_batch_page(&conn, "b1", 200).expect("classify");
        classify_batch_page(&conn, "b1", 200).expect("drain");
        let late: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_batch_members WHERE batch_id = 'b1' AND asset_id_snapshot = 'a8'",
                [],
                |row| row.get(0),
            )
            .expect("late asset excluded");
        assert_eq!(late, 0);
        let task: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM processing_tasks WHERE id = 'ocr-a8'",
                [],
                |row| row.get(0),
            )
            .expect("no task for late asset");
        assert_eq!(task, 0);
    }
}
