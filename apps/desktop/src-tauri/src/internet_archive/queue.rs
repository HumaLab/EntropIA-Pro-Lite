//! Durable queue of "import every result" batches.
//!
//! The dialog owns the queue's meaning (which records are pending, which job
//! holds the current batch); this module only keeps one JSON object on disk
//! at `<job store root>/queue.json`, written atomically, so an unfinished
//! queue survives closing the dialog or the app. The file is not a job
//! folder, so job recovery and listing skip it.

use std::fs;
use std::io::Write as _;
use std::path::Path;

use tauri::State;

use super::IaState;

/// File name inside the job store root.
pub(crate) const QUEUE_FILE: &str = "queue.json";

/// Upper bound for the stored queue (10,000 records fit well below it).
pub(crate) const MAX_QUEUE_BYTES: usize = 4 * 1024 * 1024;

/// The stored queue, or `None` when there is none or it cannot be read as
/// a JSON object (a damaged file is left in place, never parsed further).
pub(crate) fn load_queue(root: &Path) -> Result<Option<String>, String> {
    let content = match fs::read_to_string(root.join(QUEUE_FILE)) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("io: local storage error".to_string()),
    };
    Ok(is_bounded_object(&content).then_some(content))
}

/// Replace the stored queue with `content`, which must be a JSON object of
/// at most [`MAX_QUEUE_BYTES`]. Written through a temporary file and renamed.
pub(crate) fn save_queue(root: &Path, content: &str) -> Result<(), String> {
    if !is_bounded_object(content) {
        return Err("invalid_request: the queue must be a JSON object".to_string());
    }
    let write = || -> std::io::Result<()> {
        fs::create_dir_all(root)?;
        let tmp = root.join(format!("{QUEUE_FILE}.tmp"));
        let mut file = fs::File::create(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, root.join(QUEUE_FILE))
    };
    write().map_err(|_| "io: local storage error".to_string())
}

/// Remove the stored queue; succeeds when there is none.
pub(crate) fn clear_queue(root: &Path) -> Result<(), String> {
    match fs::remove_file(root.join(QUEUE_FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("io: local storage error".to_string()),
    }
}

fn is_bounded_object(content: &str) -> bool {
    content.len() <= MAX_QUEUE_BYTES
        && serde_json::from_str::<serde_json::Value>(content).is_ok_and(|value| value.is_object())
}

/// The pending "import every result" queue, if any.
#[tauri::command]
pub fn ia_queue_load(ia: State<'_, IaState>) -> Result<Option<String>, String> {
    load_queue(ia.store.root())
}

/// Store the queue (a JSON object) so it can be resumed later.
#[tauri::command]
pub fn ia_queue_save(content: String, ia: State<'_, IaState>) -> Result<(), String> {
    save_queue(ia.store.root(), &content)
}

/// Forget the queue (finished or discarded).
#[tauri::command]
pub fn ia_queue_clear(ia: State<'_, IaState>) -> Result<(), String> {
    clear_queue(ia.store.root())
}

#[cfg(test)]
mod tests {
    use super::*;
    use internet_archive::store::JobStore;

    fn root() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("internet-archive");
        (dir, root)
    }

    #[test]
    fn a_saved_queue_loads_back_unchanged() {
        let (_dir, root) = root();
        assert_eq!(load_queue(&root).unwrap(), None);
        let content = r#"{"version":1,"records":[{"identifier":"a","state":"pending"}]}"#;
        save_queue(&root, content).unwrap();
        assert_eq!(load_queue(&root).unwrap().as_deref(), Some(content));
        let newer = r#"{"version":1,"records":[]}"#;
        save_queue(&root, newer).unwrap();
        assert_eq!(load_queue(&root).unwrap().as_deref(), Some(newer));
        assert!(!root.join("queue.json.tmp").exists());
    }

    #[test]
    fn save_rejects_anything_but_a_bounded_json_object() {
        let (_dir, root) = root();
        for bad in ["", "[]", "\"text\"", "{not json"] {
            let err = save_queue(&root, bad).expect_err(bad);
            assert!(err.starts_with("invalid_request"), "{bad}: {err}");
        }
        let huge = format!("{{\"pad\":\"{}\"}}", "x".repeat(MAX_QUEUE_BYTES));
        assert!(save_queue(&root, &huge)
            .expect_err("too big")
            .starts_with("invalid_request"));
        assert_eq!(load_queue(&root).unwrap(), None);
    }

    #[test]
    fn a_damaged_queue_file_reads_as_none() {
        let (_dir, root) = root();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(QUEUE_FILE), b"{half").unwrap();
        assert_eq!(load_queue(&root).unwrap(), None);
    }

    #[test]
    fn clear_removes_the_queue_and_is_idempotent() {
        let (_dir, root) = root();
        clear_queue(&root).unwrap();
        save_queue(&root, "{}").unwrap();
        clear_queue(&root).unwrap();
        assert_eq!(load_queue(&root).unwrap(), None);
        clear_queue(&root).unwrap();
    }

    #[test]
    fn job_recovery_leaves_the_queue_file_alone() {
        let (_dir, root) = root();
        save_queue(&root, r#"{"version":1}"#).unwrap();
        let store = JobStore::new(root.clone());
        let _ = store.recover_all(1_700_000_000);
        assert_eq!(
            load_queue(&root).unwrap().as_deref(),
            Some(r#"{"version":1}"#)
        );
    }
}
