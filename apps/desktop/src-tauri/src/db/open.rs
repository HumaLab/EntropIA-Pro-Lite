//! The one place that knows how to open a connection to the archive.
//!
//! Every connection to `entropia.sqlite` carries the same three settings, and
//! they are not decorations:
//!
//! - **WAL** lets a reader work while a writer writes. Without it every read
//!   blocks behind every write.
//! - **Foreign keys** are off by default in SQLite. The schema declares them, so
//!   a connection that does not enable them silently accepts orphan rows.
//! - **`busy_timeout`** is the one that was missing. Without it SQLite gives up
//!   the instant another connection holds the write lock, and the caller sees
//!   `database is locked` rather than waiting the moment it takes to clear.
//!
//! That last one is why this module exists. The settings used to be repeated at
//! nine call sites, and the repetitions had drifted: `sync` and `geo` waited,
//! while the UI connection, both OCR workers, transcription, the LLM worker and
//! the embedding scheduler did not. Twenty-four further sites opened the archive
//! and configured nothing at all. So an embedding job writing during a long
//! import failed outright — not because the archive was unavailable, but because
//! nobody had told SQLite it was allowed to wait.
//!
//! Repeating a list is how a list drifts. Opening the archive happens here now,
//! and a test in this file holds every other module to that.

use std::path::Path;

use rusqlite::Connection;

/// How long a connection waits for the write lock before reporting it busy.
///
/// Long enough to cover an ordinary contended write — an import committing a
/// batch while an embedding job wants in — and short enough that a genuinely
/// stuck writer still surfaces instead of hanging the app forever.
pub const BUSY_TIMEOUT_MS: u32 = 15_000;

/// Opens a connection to the archive with the settings every caller needs.
pub fn open_archive_connection(db_path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(db_path)
        .map_err(|e| format!("Failed to open {}: {e}", db_path.display()))?;
    conn.execute_batch(&format!(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout={BUSY_TIMEOUT_MS};"
    ))
    .map_err(|e| format!("Failed to configure {}: {e}", db_path.display()))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn busy_timeout_of(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("read busy_timeout")
    }

    #[test]
    fn an_opened_connection_is_allowed_to_wait_for_the_write_lock() {
        // The defect this module exists for: without busy_timeout SQLite reports
        // `database is locked` the instant another connection holds the write
        // lock, so a background job dies during an ordinary import instead of
        // waiting the moment it takes to clear.
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = open_archive_connection(&dir.path().join("entropia.sqlite")).expect("open");

        assert_eq!(busy_timeout_of(&conn), i64::from(BUSY_TIMEOUT_MS));
    }

    #[test]
    fn an_opened_connection_enforces_foreign_keys() {
        // SQLite defaults these OFF. The schema declares them, so a connection
        // that forgets silently accepts orphan rows.
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = open_archive_connection(&dir.path().join("entropia.sqlite")).expect("open");

        let enabled: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("read foreign_keys");
        assert_eq!(enabled, 1);
    }

    #[test]
    fn a_second_writer_waits_instead_of_failing_immediately() {
        // Two connections, one holding the write lock. The second must block for
        // its timeout rather than return instantly — that difference is the
        // whole point, and it is observable as elapsed time.
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("entropia.sqlite");

        let holder = open_archive_connection(&db_path).expect("open holder");
        holder
            .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY);")
            .expect("create table");

        let waiter = open_archive_connection(&db_path).expect("open waiter");
        // A short timeout keeps the test fast while still proving it waits.
        waiter
            .busy_timeout(std::time::Duration::from_millis(400))
            .expect("set short timeout");

        let held = holder.unchecked_transaction().expect("begin");
        held.execute("INSERT INTO t(id) VALUES (1)", [])
            .expect("write inside the held transaction");

        // A real write, not a DEFERRED `transaction()`: only an actual write
        // reaches for the lock the holder is sitting on.
        let started = std::time::Instant::now();
        let blocked = waiter.execute("INSERT INTO t(id) VALUES (2)", []);
        let elapsed = started.elapsed();

        assert!(blocked.is_err(), "the write lock is held, so this cannot win");
        assert!(
            elapsed >= std::time::Duration::from_millis(300),
            "it gave up after {elapsed:?} instead of waiting for the lock"
        );
    }

    #[test]
    fn no_other_module_opens_the_archive_by_hand() {
        // The defect this module was created for was not one missing pragma.
        // Nine connections each repeated the pragma list — and that is exactly
        // how a list drifts: `sync` and `geo` had waited for the write lock for
        // years while the UI, the OCR workers, transcription, the LLM worker and
        // the embedding scheduler had not. Nobody could see the difference,
        // because the knowledge lived in nine places.
        //
        // Twenty-four MORE opened the archive and configured nothing at all, and
        // a guard that only looked for a hand-written pragma walked straight past
        // every one of them. So the rule is not "remember the pragma". The rule
        // is that opening the archive happens here, and only here.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        collect_offenders(&src, &mut offenders);
        offenders.sort();

        assert!(
            offenders.is_empty(),
            "these open or configure an archive connection themselves instead of              calling open_archive_connection: {offenders:#?}"
        );
    }

    /// Files allowed to open a connection directly, and why.
    ///
    /// `rag/baseline.rs` opens a throwaway working copy it creates with
    /// `Backup::new` — a different file from the archive, with no other writer
    /// to contend with.
    const DIRECT_OPEN_ALLOWED: &[&str] = &["baseline.rs"];

    fn collect_offenders(dir: &std::path::Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("read src dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_offenders(&path, found);
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) if name.ends_with(".rs") => name.to_string(),
                _ => continue,
            };
            if name == "open.rs" || DIRECT_OPEN_ALLOWED.contains(&name.as_str()) {
                continue;
            }

            let body = std::fs::read_to_string(&path).unwrap_or_default();
            let runtime = runtime_portion(&body);

            if runtime.contains("journal_mode=WAL") || runtime.contains("journal_mode = WAL") {
                found.push(format!("{} (writes the pragmas by hand)", path.display()));
            }
            // `Connection::open_in_memory(` does not contain `Connection::open(`
            // — an underscore sits where the parenthesis would be — so throwaway
            // in-memory databases never match this.
            if runtime.contains("Connection::open(") {
                found.push(format!("{} (opens a connection directly)", path.display()));
            }
        }
    }

    /// The part of a file that ships — everything before its `mod tests`.
    ///
    /// Test code opens throwaway databases on purpose and is not the subject of
    /// this rule.
    fn runtime_portion(body: &str) -> &str {
        match body.find("#[cfg(test)]") {
            Some(at) => match body[at..].find("mod tests") {
                Some(_) => &body[..at],
                None => body,
            },
            None => body,
        }
    }
}
