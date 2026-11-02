//! The local audit trail of what an assistant did (S9, D10).
//!
//! One JSON line per tool call in `<cache>/logs/agent-audit.jsonl`: when, which
//! client, which tool, how it ended, which error code, which object ids and — for
//! search only — how long the query was. Never the query itself or any other free
//! text, never a token: an audit file is exactly the kind of file that gets copied
//! out of a machine, so it must be useless as a leak.
//!
//! Past [`AUDIT_MAX_BYTES`] the file is rotated to a single `.1` sibling, so the
//! trail is bounded without ever growing a directory of old copies. Writing is
//! best effort: a full disk logs a warning and the tool call still answers.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::automation::access::Principal;

/// The audit file, in the logs directory under the cache.
pub const AUDIT_FILE: &str = "agent-audit.jsonl";

/// The one previous file rotation keeps (D10).
pub const ROTATED_FILE: &str = "agent-audit.jsonl.1";

/// Size past which the file rotates (D10).
pub const AUDIT_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// How a call ended, in two words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditStatus {
    Ok,
    Error,
}

/// One line of the audit file. Field names are the contract D10 fixes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLine {
    /// Unix milliseconds.
    pub ts: i64,
    pub client_id: String,
    pub client_name: String,
    pub tool: String,
    pub status: AuditStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// The ids the call touched or returned. Never a title, a path or a query.
    pub object_ids: Vec<String>,
    /// Length of a search query, for search calls only: the query itself never
    /// reaches the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_chars: Option<usize>,
    /// The owner's answer to an assistant's request (`owner_decision` lines
    /// only): `approved`, `failed` (approved, nothing ran) or `rejected`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
}

impl AuditLine {
    /// A call that answered normally.
    pub fn ok(
        caller: &Principal,
        tool: &str,
        object_ids: Vec<String>,
        query_chars: Option<usize>,
    ) -> Self {
        Self {
            ts: now_ms(),
            client_id: caller.client_id.clone(),
            client_name: caller.name.clone(),
            tool: tool.to_string(),
            status: AuditStatus::Ok,
            error_code: None,
            object_ids,
            query_chars,
            decision: None,
        }
    }

    /// The owner answered an assistant's request in the app. Attributed to the
    /// assistant that asked, with the approval id, the id of what it acts on
    /// (the job, the item whose text it replaces, or the collection an import
    /// fills) and the answer.
    pub fn owner_decision(
        client_id: &str,
        client_name: &str,
        approval_id: &str,
        target_id: &str,
        decision: &str,
    ) -> Self {
        Self {
            ts: now_ms(),
            client_id: client_id.to_string(),
            client_name: client_name.to_string(),
            tool: "owner_decision".to_string(),
            status: AuditStatus::Ok,
            error_code: None,
            object_ids: vec![approval_id.to_string(), target_id.to_string()],
            query_chars: None,
            decision: Some(decision.to_string()),
        }
    }

    /// A call the contract refused, carrying its stable code.
    pub fn failed(
        caller: &Principal,
        tool: &str,
        error_code: &str,
        query_chars: Option<usize>,
    ) -> Self {
        Self {
            ts: now_ms(),
            client_id: caller.client_id.clone(),
            client_name: caller.name.clone(),
            tool: tool.to_string(),
            status: AuditStatus::Error,
            error_code: Some(error_code.to_string()),
            object_ids: Vec::new(),
            query_chars,
            decision: None,
        }
    }
}

/// Unix milliseconds, the same clock the app log uses.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Serializes every append in the process. Each tool call builds its own
/// [`AuditLog`], so the lock cannot live in the value: two calls crossing the cap
/// together would otherwise both rotate, and the second rotation would throw
/// away (or fail to find) the file the first one kept.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// The append-only trail of one machine.
#[derive(Debug, Clone)]
pub struct AuditLog {
    dir: PathBuf,
    max_bytes: u64,
}

impl AuditLog {
    /// The trail under `logs_dir` — `<cache>/logs` in the app.
    pub fn new(logs_dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: logs_dir.into(),
            max_bytes: AUDIT_MAX_BYTES,
        }
    }

    /// The same trail with a smaller cap, so a test can rotate without writing
    /// megabytes.
    #[cfg(test)]
    pub fn with_max_bytes(logs_dir: impl Into<PathBuf>, max_bytes: u64) -> Self {
        Self {
            dir: logs_dir.into(),
            max_bytes,
        }
    }

    /// The current audit file.
    pub fn path(&self) -> PathBuf {
        self.dir.join(AUDIT_FILE)
    }

    /// The one previous audit file.
    pub fn rotated_path(&self) -> PathBuf {
        self.dir.join(ROTATED_FILE)
    }

    /// Append one line. A failure is logged and swallowed: the audit trail may
    /// never be the reason an assistant gets no answer (D10).
    pub fn append(&self, line: &AuditLine) {
        if let Err(error) = self.write(line) {
            log::warn!(target: "automation", "could not append the assistant audit line: {error}");
        }
    }

    fn write(&self, line: &AuditLine) -> Result<(), String> {
        // A poisoned lock only means another append panicked; the file is still
        // usable, and the trail is best effort anyway.
        let _guard = WRITE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut bytes = serde_json::to_vec(line)
            .map_err(|error| format!("could not serialize the audit line: {error}"))?;
        bytes.push(b'\n');
        std::fs::create_dir_all(&self.dir).map_err(|error| {
            format!(
                "could not create the audit directory {}: {error}",
                self.dir.display()
            )
        })?;
        let path = self.path();
        let current = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        if current > 0 && current + bytes.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| format!("could not open {}: {error}", path.display()))?;
        file.write_all(&bytes)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    }

    /// Move the current file to the `.1` slot, dropping whatever was there. One
    /// previous file is the whole retention policy (D10).
    fn rotate(&self) -> Result<(), String> {
        let rotated = self.rotated_path();
        if rotated.exists() {
            std::fs::remove_file(&rotated).map_err(|error| {
                format!(
                    "could not replace the previous audit file {}: {error}",
                    rotated.display()
                )
            })?;
        }
        let path = self.path();
        std::fs::rename(&path, &rotated).map_err(|error| {
            format!(
                "could not rotate {} to {}: {error}",
                path.display(),
                rotated.display()
            )
        })
    }
}

/// One line written for every call, wherever the caller lives.
pub fn default_log(cache_dir: &Path) -> AuditLog {
    AuditLog::new(cache_dir.join("logs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(tool: &str, size: usize) -> AuditLine {
        let caller = Principal {
            client_id: "cl_test".to_string(),
            name: "Claude Code".to_string(),
            scopes: vec![crate::automation::access::Scope::Read],
        };
        AuditLine::ok(&caller, tool, vec!["x".repeat(size)], None)
    }

    fn read_lines(path: &Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("each audit line is JSON"))
            .collect()
    }

    /// Concurrent appends across the cap must rotate once, not once per
    /// caller: before the shared lock two callers could both see the file over
    /// the cap and the second rotation threw away the lines the first kept.
    #[test]
    fn concurrent_appends_across_the_cap_rotate_once_and_lose_nothing() {
        const WRITERS: usize = 48;
        for _round in 0..20 {
            let dir = tempfile::tempdir().unwrap();
            let one = serde_json::to_vec(&line("search_items", 8)).unwrap().len() as u64 + 1;
            // Room for just over half the lines: exactly one rotation keeps all.
            let log = AuditLog::with_max_bytes(dir.path(), one * (WRITERS as u64 / 2 + 1));
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(WRITERS));
            let handles: Vec<_> = (0..WRITERS)
                .map(|_| {
                    let log = log.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        log.write(&line("search_items", 8)).unwrap();
                    })
                })
                .collect();
            for handle in handles {
                handle.join().unwrap();
            }
            let kept = read_lines(&log.path()).len() + read_lines(&log.rotated_path()).len();
            assert_eq!(kept, WRITERS, "every line survives a single rotation");
        }
    }

    #[test]
    fn a_line_carries_the_contract_fields_and_no_free_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = AuditLog::new(dir.path());
        let caller = Principal {
            client_id: "cl_test".to_string(),
            name: "Claude Code".to_string(),
            scopes: vec![crate::automation::access::Scope::Read],
        };
        log.append(&AuditLine::ok(&caller, "search_items", vec![], Some(14)));
        log.append(&AuditLine::failed(&caller, "items_get", "not_found", None));
        let lines = read_lines(&log.path());
        assert_eq!(lines.len(), 2);
        let mut keys: Vec<&str> = lines[0]
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "clientId",
                "clientName",
                "objectIds",
                "queryChars",
                "status",
                "tool",
                "ts"
            ]
        );
        assert_eq!(lines[0]["status"], "ok");
        assert_eq!(lines[0]["queryChars"], 14);
        assert_eq!(lines[1]["status"], "error");
        assert_eq!(lines[1]["errorCode"], "not_found");
        assert!(lines[1].get("queryChars").is_none(), "absent, not null");
        assert!(lines[0]["ts"].as_i64().is_some_and(|ts| ts > 0));
    }

    #[test]
    fn rotation_keeps_one_previous_file_and_drops_the_older_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Each line is well over 100 bytes; the cap fits a couple of them.
        let log = AuditLog::with_max_bytes(dir.path(), 300);
        for index in 0..12 {
            log.append(&line(&format!("tool_{index}"), 8));
        }
        let rotated = log.rotated_path();
        assert!(rotated.exists(), "the previous file was kept");
        let current = std::fs::metadata(log.path()).expect("current file").len();
        assert!(
            current <= 300,
            "the current file never grows past the cap: {current}"
        );
        let current_lines = read_lines(&log.path());
        let rotated_lines = read_lines(&rotated);
        assert!(!current_lines.is_empty());
        assert!(!rotated_lines.is_empty());
        let last = current_lines.last().expect("last line").clone();
        assert_eq!(last["tool"], "tool_11", "the newest call stays current");
        assert!(
            !rotated_lines.iter().any(|line| line["tool"] == "tool_11"),
            "and is not duplicated into the rotated file"
        );
        assert!(
            !rotated_lines.iter().any(|line| line["tool"] == "tool_0"),
            "only one previous file is kept, so the oldest lines are gone"
        );
    }
}
