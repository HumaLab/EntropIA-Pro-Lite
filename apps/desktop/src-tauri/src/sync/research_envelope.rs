//! Inactive v1 snapshot adapter for the negotiated `research_envelopes` row.
//!
//! One terminal Investigations job (`done` or `failed`) becomes one immutable
//! aggregate with the readable provenance another device needs to show it:
//! the job summary, the current non-obsolete structured report plus its
//! version, a deterministic de-duplicated source manifest derived from the
//! current archive evidence, and the content-addressed `report.md` manifest
//! when the engine rendered the file under `research/artifacts/<job_id>`.
//!
//! This module only builds and validates a coherent aggregate. It is not
//! connected to capture, transport, or apply; the receive/conflict slice will
//! consume [`ResearchEnvelopeV1::from_json`] /
//! [`ResearchEnvelopeV1::fingerprint_sha256`].
//!
//! The reader opens `estado.sqlite` directly, read-only: EntropIA-Agent's raw
//! connection is crate-private. It rejects missing jobs, non-terminal jobs and
//! malformed required JSON instead of fabricating data. A report-less failed
//! job (and a cancelled `done` job) stays representable: `report` is simply
//! `None`. Active or human-gated jobs are out of contract (PROTOCOL: the
//! research aggregate is terminal-only) and are rejected here too.

// Inactive slice: the future engine calls these helpers after catch-up.
#![allow(dead_code)]

use std::fs::File;
use std::io::Read;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) const RESEARCH_ENVELOPE_VERSION: u32 = 1;
pub(crate) const INVALID_RESEARCH_ENVELOPE: &str = "invalid_research_envelope";
pub(crate) const UNSUPPORTED_RESEARCH_ENVELOPE_VERSION: &str =
    "unsupported_research_envelope_version";
pub(crate) const RESEARCH_JOB_NOT_FOUND: &str = "research_job_not_found";
pub(crate) const RESEARCH_JOB_NOT_TERMINAL: &str = "research_job_not_terminal";
pub(crate) const RESEARCH_SNAPSHOT_UNREADABLE: &str = "research_snapshot_unreadable";
pub(crate) const RESEARCH_REPORT_FILE_MISMATCH: &str = "research_report_file_mismatch";

/// Bounded read ceiling for the optional `report.md` manifest. A larger report
/// file is a hard error, never a truncated hash.
pub(crate) const MAX_REPORT_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// The rendered report file the engine writes into `research/artifacts/<job_id>`.
pub(crate) const REPORT_FILE_REL_PATH: &str = "report.md";

/// The `jobs.modo` of the Investigations engine. Other job kinds in the same
/// `estado.sqlite` (for example `paper`) are never captured as research.
pub(crate) const RESEARCH_JOB_MODO: &str = "research";

const TERMINAL_STATUSES: [&str; 2] = ["done", "failed"];
const CLOSE_REASONS: [&str; 4] = ["completed", "cancelled", "budget_exhausted", "blocked"];

/// A coded error so the future transport can branch on `code` instead of
/// matching message text. Mirrors `writing::repository::WritingError`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ResearchError {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl ResearchError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub(crate) fn sql(context: &str, err: rusqlite::Error) -> Self {
        Self::new("sql_error", format!("{context}: {err}"))
    }
}

pub(crate) type ResearchResult<T> = Result<T, ResearchError>;

/// One terminal research job as one opaque wire row (`row_id = job_id`,
/// `payload.id == row_id` per PROTOCOL).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResearchEnvelopeV1 {
    pub(crate) id: String,
    pub(crate) envelope_version: u32,
    pub(crate) job: ResearchJobSummaryV1,
    pub(crate) report: Option<ResearchReportV1>,
    pub(crate) sources: Vec<ResearchSourceEntryV1>,
    pub(crate) report_file: Option<ResearchFileManifestV1>,
}

/// Identity and summary of the job. `created_at`/`updated_at` are epoch
/// seconds exactly as EntropIA-Agent stores them (`estado::ahora`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResearchJobSummaryV1 {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) close_reason: String,
    pub(crate) title: String,
    pub(crate) question: String,
    pub(crate) project: String,
    pub(crate) corpus_snapshot_id: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

/// The current non-obsolete structured report artifact: its full `content_json`
/// (structured report, citations, rendered markdown and provenance blocks)
/// plus the artifact version that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResearchReportV1 {
    pub(crate) version: i64,
    pub(crate) content_json: Value,
}

/// One de-duplicated entry of the source manifest derived from the current
/// archive evidence. Identifiers are the stable ones the engine records:
/// `source_id` is the provenance-scoped source key (split parts of one chunk
/// share it), `evidence_id` is the concrete evidence entry id, and
/// `item_id`/`chunk_id` are the corpus identifiers. `text_hash` and `title`
/// travel only when the archive evidence carries them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResearchSourceEntryV1 {
    pub(crate) source_id: String,
    pub(crate) evidence_id: String,
    pub(crate) item_id: Option<String>,
    pub(crate) chunk_id: Option<String>,
    pub(crate) text_hash: Option<String>,
    pub(crate) title: Option<String>,
}

/// Content-addressed manifest of one report file, rooted under the app's
/// `research/artifacts/<job_id>` directory. Only portable relative paths: no
/// local absolute paths ever travel in the envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResearchFileManifestV1 {
    pub(crate) rel_path: String,
    pub(crate) sha256: String,
    pub(crate) size: u64,
}

impl ResearchEnvelopeV1 {
    pub(crate) fn from_json(raw: &str) -> ResearchResult<Self> {
        let mut envelope: Self = serde_json::from_str(raw).map_err(|error| {
            invalid_envelope(format!("failed to deserialize research envelope: {error}"))
        })?;
        envelope.canonicalize();
        envelope.validate()?;
        Ok(envelope)
    }

    pub(crate) fn to_canonical_json(&self) -> ResearchResult<String> {
        let mut envelope = self.clone();
        envelope.canonicalize();
        envelope.validate()?;
        serde_json::to_string(&envelope).map_err(|error| {
            invalid_envelope(format!("failed to serialize research envelope: {error}"))
        })
    }

    pub(crate) fn fingerprint_sha256(&self) -> ResearchResult<String> {
        let canonical = self.to_canonical_json()?;
        Ok(format!("{:x}", Sha256::digest(canonical.as_bytes())))
    }

    pub(crate) fn validate(&self) -> ResearchResult<()> {
        if self.envelope_version != RESEARCH_ENVELOPE_VERSION {
            return Err(ResearchError::new(
                UNSUPPORTED_RESEARCH_ENVELOPE_VERSION,
                format!(
                    "unsupported research envelope version {}; expected {}",
                    self.envelope_version, RESEARCH_ENVELOPE_VERSION
                ),
            ));
        }
        require_non_empty("id", &self.id)?;
        require_non_empty("job.id", &self.job.id)?;
        // PROTOCOL: `payload.id == row_id`. The envelope id is the job id.
        if self.id != self.job.id {
            return Err(invalid_envelope(format!(
                "id {:?} must equal job.id {:?}",
                self.id, self.job.id
            )));
        }
        if !TERMINAL_STATUSES.contains(&self.job.status.as_str()) {
            return Err(invalid_envelope(format!(
                "unsupported research job status {:?}",
                self.job.status
            )));
        }
        if !CLOSE_REASONS.contains(&self.job.close_reason.as_str()) {
            return Err(invalid_envelope(format!(
                "unsupported research job close_reason {:?}",
                self.job.close_reason
            )));
        }
        require_non_empty("job.title", &self.job.title)?;
        require_non_empty("job.question", &self.job.question)?;
        require_non_empty("job.project", &self.job.project)?;
        if self.job.created_at < 0 || self.job.updated_at < 0 {
            return Err(invalid_envelope(format!(
                "job timestamps must be non-negative, found created_at {} updated_at {}",
                self.job.created_at, self.job.updated_at
            )));
        }

        if let Some(report) = &self.report {
            if report.version < 1 {
                return Err(invalid_envelope(format!(
                    "report.version must be positive, found {}",
                    report.version
                )));
            }
            if !report.content_json.is_object() {
                return Err(invalid_envelope(
                    "report.content_json must be a JSON object",
                ));
            }
        }

        let mut evidence_ids = std::collections::HashSet::new();
        for entry in &self.sources {
            require_non_empty("sources[].source_id", &entry.source_id)?;
            require_non_empty("sources[].evidence_id", &entry.evidence_id)?;
            if !evidence_ids.insert(entry.evidence_id.as_str()) {
                return Err(invalid_envelope(format!(
                    "duplicate source evidence id {:?}",
                    entry.evidence_id
                )));
            }
            for (field, value) in [
                ("item_id", &entry.item_id),
                ("chunk_id", &entry.chunk_id),
                ("text_hash", &entry.text_hash),
                ("title", &entry.title),
            ] {
                if let Some(value) = value {
                    if value.trim().is_empty() {
                        return Err(invalid_envelope(format!(
                            "sources[].{field} must not be empty when present"
                        )));
                    }
                }
            }
        }

        if let Some(file) = &self.report_file {
            if file.sha256.len() != 64 || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(invalid_envelope("report_file.sha256 must be 64 hex digits"));
            }
            if !is_safe_relative_path(&file.rel_path) {
                return Err(invalid_envelope(format!(
                    "report_file path {:?} is not a safe relative path",
                    file.rel_path
                )));
            }
        }

        Ok(())
    }

    fn canonicalize(&mut self) {
        if let Some(report) = &mut self.report {
            canonicalize_json(&mut report.content_json);
        }
        self.sources.sort_by(|left, right| {
            (
                &left.source_id,
                &left.evidence_id,
                &left.item_id,
                &left.chunk_id,
                &left.text_hash,
                &left.title,
            )
                .cmp(&(
                    &right.source_id,
                    &right.evidence_id,
                    &right.item_id,
                    &right.chunk_id,
                    &right.text_hash,
                    &right.title,
                ))
        });
    }
}

/// Opens `estado.sqlite` read-only. The snapshot reader never writes the
/// research state database.
pub(crate) fn open_research_state_read_only(path: &Path) -> ResearchResult<Connection> {
    let raw = path
        .to_str()
        .ok_or_else(|| invalid_envelope("research state path is not valid UTF-8"))?;
    Connection::open_with_flags(raw, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| ResearchError::sql("Failed to open research state read-only", error))
}

/// Asserts the job exists, belongs to the Investigations engine and is
/// terminal. The capture side uses this so an active job is never enqueued.
pub(crate) fn require_terminal_research_job(
    state: &Connection,
    job_id: &str,
) -> ResearchResult<()> {
    let status: Option<String> = state
        .query_row(
            "SELECT status FROM jobs WHERE id = ?1 AND modo = ?2",
            rusqlite::params![job_id, RESEARCH_JOB_MODO],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(ResearchError::sql("Failed to read research job", other)),
        })?;
    let Some(status) = status else {
        return Err(ResearchError::new(
            RESEARCH_JOB_NOT_FOUND,
            format!("no research job with id {job_id}"),
        ));
    };
    if !TERMINAL_STATUSES.contains(&status.as_str()) {
        return Err(ResearchError::new(
            RESEARCH_JOB_NOT_TERMINAL,
            format!("research job {job_id} is not terminal (status {status:?})"),
        ));
    }
    Ok(())
}

/// Every terminal Investigations job id in stable order. The seed reads this
/// from a read-only research state connection; active jobs never appear.
pub(crate) fn terminal_research_job_ids(state: &Connection) -> ResearchResult<Vec<String>> {
    let mut stmt = state
        .prepare(
            "SELECT id FROM jobs
              WHERE modo = ?1 AND status IN ('done', 'failed')
              ORDER BY id",
        )
        .map_err(|error| ResearchError::sql("Failed to inspect research jobs", error))?;
    let rows = stmt
        .query_map([RESEARCH_JOB_MODO], |row| row.get::<_, String>(0))
        .map_err(|error| ResearchError::sql("Failed to inspect research jobs", error))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| ResearchError::sql("Failed to inspect research jobs", error))?;
    Ok(rows)
}

struct JobRow {
    status: String,
    close_reason: Option<String>,
    question: String,
    project: String,
    corpus_snapshot_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

/// Captures one coherent terminal job aggregate from `estado.sqlite`. The
/// caller supplies the app's `research/artifacts` root; the optional
/// `report.md` manifest is read (bounded) and hashed from
/// `<artifacts_root>/<job_id>/report.md`.
pub(crate) fn snapshot_job(
    state_db_path: &Path,
    artifacts_root: &Path,
    job_id: &str,
) -> ResearchResult<ResearchEnvelopeV1> {
    if !is_safe_path_component(job_id) {
        return Err(invalid_envelope(format!(
            "job id {job_id:?} is not a safe path component"
        )));
    }
    let state = open_research_state_read_only(state_db_path)?;
    snapshot_job_conn(&state, artifacts_root, job_id)
}

/// Same snapshot from an already open research state connection (read-only in
/// production; fixtures may hold a plain connection).
pub(crate) fn snapshot_job_conn(
    state: &Connection,
    artifacts_root: &Path,
    job_id: &str,
) -> ResearchResult<ResearchEnvelopeV1> {
    require_terminal_research_job(state, job_id)?;
    let job = load_job_row(state, job_id)?;

    let request = load_current_artifact(state, job_id, "request")?;
    let title = match request.map(|(_, content)| content) {
        Some(content) => match content.get("title") {
            None | Some(Value::Null) => job.question.clone(),
            Some(Value::String(title)) if !title.trim().is_empty() => title.clone(),
            Some(Value::String(_)) => job.question.clone(),
            Some(_) => {
                return Err(invalid_envelope(
                    "request artifact title must be a JSON string when present",
                ))
            }
        },
        None => job.question.clone(),
    };

    let report = load_current_artifact(state, job_id, "report")?.map(|(version, content_json)| {
        ResearchReportV1 {
            version,
            content_json,
        }
    });

    let sources = match load_current_artifact(state, job_id, "archive")? {
        Some((_, content)) => {
            let evidence = content
                .get("evidence")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    invalid_envelope("archive artifact evidence must be a JSON array")
                })?;
            source_manifest(evidence)?
        }
        None => Vec::new(),
    };

    let mut envelope = ResearchEnvelopeV1 {
        id: job_id.to_string(),
        envelope_version: RESEARCH_ENVELOPE_VERSION,
        job: ResearchJobSummaryV1 {
            id: job_id.to_string(),
            status: job.status,
            close_reason: job.close_reason.ok_or_else(|| {
                invalid_envelope(format!("terminal job {job_id} has no close_reason"))
            })?,
            title,
            question: job.question,
            project: job.project,
            corpus_snapshot_id: job.corpus_snapshot_id,
            created_at: job.created_at,
            updated_at: job.updated_at,
        },
        report,
        sources,
        report_file: report_file_manifest(artifacts_root, job_id)?,
    };
    envelope.canonicalize();
    envelope.validate()?;
    Ok(envelope)
}

fn load_job_row(state: &Connection, job_id: &str) -> ResearchResult<JobRow> {
    let row = state
        .query_row(
            "SELECT status, close_reason, pregunta, project, corpus_snapshot_id,
                    created_at, updated_at
               FROM jobs WHERE id = ?1 AND modo = ?2",
            rusqlite::params![job_id, RESEARCH_JOB_MODO],
            |row| {
                Ok(JobRow {
                    status: row.get(0)?,
                    close_reason: row.get(1)?,
                    question: row.get(2)?,
                    project: row.get(3)?,
                    corpus_snapshot_id: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => ResearchError::new(
                RESEARCH_JOB_NOT_FOUND,
                format!("no research job with id {job_id}"),
            ),
            other => ResearchError::sql("Failed to read research job summary", other),
        })?;
    if !TERMINAL_STATUSES.contains(&row.status.as_str()) {
        return Err(ResearchError::new(
            RESEARCH_JOB_NOT_TERMINAL,
            format!(
                "research job {job_id} is not terminal (status {:?})",
                row.status
            ),
        ));
    }
    if let Some(reason) = &row.close_reason {
        if !CLOSE_REASONS.contains(&reason.as_str()) {
            return Err(invalid_envelope(format!(
                "research job {job_id} has unsupported close_reason {reason:?}"
            )));
        }
    }
    Ok(row)
}

/// The current (non-obsolete, highest version) artifact of one kind, with its
/// version. `None` when the job never produced one. An artifact without
/// parseable JSON content is malformed required data and rejects the snapshot.
fn load_current_artifact(
    state: &Connection,
    job_id: &str,
    kind: &str,
) -> ResearchResult<Option<(i64, Value)>> {
    let row: Option<(i64, Option<String>)> = state
        .query_row(
            "SELECT version, content_json FROM artifacts
              WHERE job_id = ?1 AND tipo = ?2 AND obsolete = 0
              ORDER BY version DESC, rowid DESC
              LIMIT 1",
            rusqlite::params![job_id, kind],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(ResearchError::sql(
                "Failed to read research artifact",
                other,
            )),
        })?;
    let Some((version, content_json)) = row else {
        return Ok(None);
    };
    let content_json = content_json
        .ok_or_else(|| invalid_envelope(format!("{kind} artifact has no content_json")))?;
    let content: Value = serde_json::from_str(&content_json).map_err(|error| {
        invalid_envelope(format!("{kind} artifact content is not JSON: {error}"))
    })?;
    if !content.is_object() {
        return Err(invalid_envelope(format!(
            "{kind} artifact content must be a JSON object"
        )));
    }
    Ok(Some((version, content)))
}

/// Builds the de-duplicated source manifest from the current archive evidence.
/// Identical evidence ids collapse deterministically; the same id with a
/// different identity is malformed, never silently merged.
fn source_manifest(evidence: &[Value]) -> ResearchResult<Vec<ResearchSourceEntryV1>> {
    let mut entries = Vec::with_capacity(evidence.len());
    for raw in evidence {
        let entry = raw
            .as_object()
            .ok_or_else(|| invalid_envelope("archive evidence entry must be a JSON object"))?;
        let evidence_id = required_string(entry.get("id"), "archive evidence entry id")?;
        let chunk_id_raw = optional_string(entry.get("chunk_id"), "archive evidence chunk_id")?;
        let provenance = optional_string(entry.get("provenance"), "archive evidence provenance")?;
        let source_id = chunk_id_raw.clone().unwrap_or_else(|| evidence_id.clone());
        let chunk_id = chunk_id_raw.or_else(|| {
            match provenance.as_deref() {
                // Corpus chunks fall back to the entry id, like the engine's
                // own citation rule; Zotero/external hits have no chunk.
                None | Some("entropia_chunk") => Some(evidence_id.clone()),
                Some(_) => None,
            }
        });
        entries.push(ResearchSourceEntryV1 {
            source_id,
            evidence_id,
            item_id: optional_string(entry.get("item_id"), "archive evidence item_id")?,
            chunk_id,
            text_hash: optional_string(entry.get("text_hash"), "archive evidence text_hash")?,
            title: optional_string(entry.get("title"), "archive evidence title")?,
        });
    }
    // Deduplicate on the evidence id first so duplicates are adjacent.
    entries.sort_by(|left, right| {
        (
            &left.evidence_id,
            &left.source_id,
            &left.item_id,
            &left.chunk_id,
            &left.text_hash,
            &left.title,
        )
            .cmp(&(
                &right.evidence_id,
                &right.source_id,
                &right.item_id,
                &right.chunk_id,
                &right.text_hash,
                &right.title,
            ))
    });
    let mut deduplicated: Vec<ResearchSourceEntryV1> = Vec::with_capacity(entries.len());
    for entry in entries {
        if let Some(previous) = deduplicated.last() {
            if previous.evidence_id == entry.evidence_id {
                if previous != &entry {
                    return Err(invalid_envelope(format!(
                        "conflicting duplicate archive evidence id {:?}",
                        entry.evidence_id
                    )));
                }
                continue;
            }
        }
        deduplicated.push(entry);
    }
    deduplicated.sort_by(|left, right| {
        (
            &left.source_id,
            &left.evidence_id,
            &left.item_id,
            &left.chunk_id,
            &left.text_hash,
            &left.title,
        )
            .cmp(&(
                &right.source_id,
                &right.evidence_id,
                &right.item_id,
                &right.chunk_id,
                &right.text_hash,
                &right.title,
            ))
    });
    Ok(deduplicated)
}

/// Builds the content-addressed `report.md` manifest from a bounded read.
/// `None` when the engine never rendered the file. A link, a directory, an
/// oversized file or a read that changes under us is a hard error.
pub(crate) fn report_file_manifest(
    artifacts_root: &Path,
    job_id: &str,
) -> ResearchResult<Option<ResearchFileManifestV1>> {
    if !is_safe_path_component(job_id) {
        return Err(invalid_envelope(format!(
            "job id {job_id:?} is not a safe path component"
        )));
    }
    let path = artifacts_root.join(job_id).join(REPORT_FILE_REL_PATH);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ResearchError::new(
                RESEARCH_SNAPSHOT_UNREADABLE,
                format!("cannot inspect {}: {error}", path.display()),
            ))
        }
    };
    if !metadata.file_type().is_file() {
        return Err(ResearchError::new(
            RESEARCH_SNAPSHOT_UNREADABLE,
            format!("{} is not a regular file", path.display()),
        ));
    }
    if metadata.len() > MAX_REPORT_FILE_BYTES {
        return Err(ResearchError::new(
            RESEARCH_SNAPSHOT_UNREADABLE,
            format!(
                "{} exceeds the {} byte report file bound",
                path.display(),
                MAX_REPORT_FILE_BYTES
            ),
        ));
    }
    let (size, sha256) = read_report_file(&path)?;
    Ok(Some(ResearchFileManifestV1 {
        rel_path: REPORT_FILE_REL_PATH.to_string(),
        sha256,
        size,
    }))
}

/// Re-reads the report file (bounded) and verifies its SHA-256 and size
/// against the manifest. The later receive slice verifies every transferred
/// file with this before accepting it.
pub(crate) fn verify_report_file(
    artifacts_root: &Path,
    job_id: &str,
    file: &ResearchFileManifestV1,
) -> ResearchResult<()> {
    if !is_safe_path_component(job_id) {
        return Err(invalid_envelope(format!(
            "job id {job_id:?} is not a safe path component"
        )));
    }
    if !is_safe_relative_path(&file.rel_path) {
        return Err(invalid_envelope(format!(
            "report file path {:?} is not a safe relative path",
            file.rel_path
        )));
    }
    let path = artifacts_root.join(job_id).join(&file.rel_path);
    let (size, sha256) = read_report_file(&path).map_err(|error| {
        ResearchError::new(
            RESEARCH_REPORT_FILE_MISMATCH,
            format!("cannot read {}: {}", path.display(), error.message),
        )
    })?;
    if size != file.size || sha256 != file.sha256 {
        return Err(ResearchError::new(
            RESEARCH_REPORT_FILE_MISMATCH,
            format!(
                "{} does not match the manifest (size {size}, sha256 {sha256})",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// Bounded read: at most [`MAX_REPORT_FILE_BYTES`] bytes plus one, so an
/// oversized file can never be hashed truncated. The size is the bytes
/// actually read; the hash covers exactly those bytes.
fn read_report_file(path: &Path) -> ResearchResult<(u64, String)> {
    let file = File::open(path).map_err(|error| {
        ResearchError::new(
            RESEARCH_SNAPSHOT_UNREADABLE,
            format!("cannot open {}: {error}", path.display()),
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_REPORT_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            ResearchError::new(
                RESEARCH_SNAPSHOT_UNREADABLE,
                format!("cannot read {}: {error}", path.display()),
            )
        })?;
    if bytes.len() as u64 > MAX_REPORT_FILE_BYTES {
        return Err(ResearchError::new(
            RESEARCH_SNAPSHOT_UNREADABLE,
            format!(
                "{} exceeds the {} byte report file bound",
                path.display(),
                MAX_REPORT_FILE_BYTES
            ),
        ));
    }
    Ok((
        bytes.len() as u64,
        format!("{:x}", Sha256::digest(bytes.as_slice())),
    ))
}

/// True for a single safe path component: no separators, no drive colons, no
/// `.`/`..`. Job ids are directory names under `research/artifacts`.
pub(crate) fn is_safe_path_component(component: &str) -> bool {
    !component.is_empty() && !component.contains('/') && is_safe_relative_path(component)
}

/// True for a portable relative path: no absolute prefix, no backslashes, no
/// drive colons, and no empty/`.`/`..` components. Mirrors the writing file
/// layer so no local absolute path can ever travel in an envelope.
pub(crate) fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn required_string(value: Option<&Value>, field: &str) -> ResearchResult<String> {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.clone()),
        _ => Err(invalid_envelope(format!(
            "{field} must be a non-empty string"
        ))),
    }
}

fn optional_string(value: Option<&Value>, field: &str) -> ResearchResult<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(Some(text.clone())),
        Some(_) => Err(invalid_envelope(format!(
            "{field} must be a non-empty string when present"
        ))),
    }
}

fn require_non_empty(field: &str, value: &str) -> ResearchResult<()> {
    if value.trim().is_empty() {
        Err(invalid_envelope(format!("{field} must not be empty")))
    } else {
        Ok(())
    }
}

fn invalid_envelope(message: impl Into<String>) -> ResearchError {
    ResearchError::new(INVALID_RESEARCH_ENVELOPE, message)
}

fn canonicalize_json(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<_> = std::mem::take(object).into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            for (key, mut child) in entries {
                canonicalize_json(&mut child);
                object.insert(key, child);
            }
        }
        Value::Array(values) => {
            for value in values {
                canonicalize_json(value);
            }
        }
        _ => {}
    }
}
