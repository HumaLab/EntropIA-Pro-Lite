//! The startup sweep of `<data>/web-captures/`: what a crash or a failed delete
//! can leave behind, removed without ever putting the archive at risk.
//!
//! Four rules, each applied only to what is old enough that no save can still
//! be working on it ([`MIN_AGE`]):
//! 1. temporary files (`.<name>.tmp`) from an interrupted save;
//! 2. files no `web_captures` row points at (`rel_path` or `text_rel_path`):
//!    a crash between the rename and the commit;
//! 3. folders left empty;
//! 4. folders of a source that no longer exists (a delete whose files would not
//!    go).
//!
//! Rules 2 and 4 need the database. If it cannot answer (the tables are not
//! there yet because the renderer has not migrated, or a query fails), they are
//! skipped: a failed read must never read as "nothing is referenced". Rules 1
//! and 3 do not depend on it.
//!
//! The sweep only ever looks inside `web-captures/`, only at folders whose name
//! is a plain id, and never enters a symlink or junction (see
//! [`super::capture_files`]).

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use rusqlite::Connection;

use super::capture_files::{self, Located};
use super::save::DIR;

/// Nothing younger than this is touched.
pub const MIN_AGE: Duration = Duration::from_secs(60 * 60);

/// Most folders and files one sweep looks at: a bound, not an expectation.
pub const MAX_ENTRIES: usize = 50_000;

/// What a sweep did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Temporary files removed.
    pub temp: usize,
    /// Unreferenced files removed.
    pub orphans: usize,
    /// Empty source folders removed.
    pub empty_dirs: usize,
    /// Folders of vanished sources removed.
    pub ghost_dirs: usize,
    /// Things that were not entered or touched because they are links, or
    /// resolve outside the root.
    pub refused: usize,
    /// Removals and reads that failed (the sweep goes on).
    pub errors: usize,
    /// Rules 2 and 4 did not run because the database could not say.
    pub db_rules_skipped: bool,
    /// The sweep stopped at [`MAX_ENTRIES`]; the rest waits for the next start.
    pub incomplete: bool,
}

impl Report {
    /// One line for the log.
    pub fn summary(&self) -> String {
        let mut line = format!(
            "web-captures sweep: {} temporary file(s), {} orphan file(s), {} empty folder(s), \
             {} folder(s) of deleted sources removed; {} refused, {} error(s)",
            self.temp, self.orphans, self.empty_dirs, self.ghost_dirs, self.refused, self.errors
        );
        if self.db_rules_skipped {
            line.push_str("; orphan and deleted-source rules skipped (database not ready)");
        }
        if self.incomplete {
            line.push_str("; stopped at the entry limit");
        }
        line
    }
}

/// What the database says exists.
struct Known {
    /// Relative keys of every file a capture row points at.
    keys: HashSet<String>,
    /// Ids of every source row.
    sources: HashSet<String>,
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |_| Ok(()),
    )
    .is_ok()
}

fn column_of(conn: &Connection, sql: &str) -> Option<HashSet<String>> {
    let mut statement = conn.prepare(sql).ok()?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .ok()?;
    rows.collect::<Result<HashSet<_>, _>>().ok()
}

impl Known {
    /// `None` when the database cannot answer completely: the tables are
    /// missing or a query fails. Never a partial answer.
    fn read(conn: &Connection) -> Option<Self> {
        if !table_exists(conn, "web_sources") || !table_exists(conn, "web_captures") {
            return None;
        }
        let keys = column_of(
            conn,
            "SELECT rel_path FROM web_captures WHERE rel_path IS NOT NULL
             UNION
             SELECT text_rel_path FROM web_captures WHERE text_rel_path IS NOT NULL",
        )?
        .into_iter()
        // Keys are stored with forward slashes; be safe against another spelling.
        .map(|key| key.replace('\\', "/"))
        .collect();
        let sources = column_of(conn, "SELECT id FROM web_sources")?;
        Some(Self { keys, sources })
    }
}

fn is_old(meta: &fs::Metadata, now: SystemTime, min_age: Duration) -> bool {
    meta.modified()
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age > min_age)
}

/// A temporary file of an interrupted save: `.<name>.tmp`.
fn is_temp(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(".tmp")
}

/// Sweep `<data_dir>/web-captures/`. `now` and `min_age` are injected so tests
/// are exact.
pub fn sweep(data_dir: &Path, conn: &Connection, now: SystemTime, min_age: Duration) -> Report {
    let mut report = Report::default();
    let root = capture_files::root(data_dir);
    match fs::symlink_metadata(&root) {
        Ok(meta) if meta.file_type().is_symlink() => {
            report.refused += 1;
            return report;
        }
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            report.refused += 1;
            return report;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return report,
        Err(_) => {
            report.errors += 1;
            return report;
        }
    }

    let known = Known::read(conn);
    report.db_rules_skipped = known.is_none();

    let Ok(entries) = fs::read_dir(&root) else {
        report.errors += 1;
        return report;
    };
    let mut examined = 0usize;
    for entry in entries.flatten() {
        examined += 1;
        if examined > MAX_ENTRIES {
            report.incomplete = true;
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // Only folders named like a source id are ours; anything else stays.
        if !capture_files::valid_source_id(&name) {
            continue;
        }
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {}
            Ok(kind) if kind.is_symlink() => {
                report.refused += 1;
                continue;
            }
            _ => continue,
        }
        match capture_files::locate_source_dir(data_dir, &name) {
            Located::Dir(dir) => sweep_source(
                &dir,
                &name,
                known.as_ref(),
                now,
                min_age,
                &mut examined,
                &mut report,
            ),
            Located::Refused(_) => report.refused += 1,
            Located::Missing => {}
        }
    }
    report
}

fn sweep_source(
    dir: &Path,
    name: &str,
    known: Option<&Known>,
    now: SystemTime,
    min_age: Duration,
    examined: &mut usize,
    report: &mut Report,
) {
    let Ok(dir_meta) = fs::metadata(dir) else {
        report.errors += 1;
        return;
    };
    // Judged before anything is removed: removing a file touches the folder.
    let dir_old = is_old(&dir_meta, now, min_age);
    let source_gone = known.is_some_and(|known| !known.sources.contains(name));

    let Ok(entries) = fs::read_dir(dir) else {
        report.errors += 1;
        return;
    };
    let mut files = Vec::new();
    let mut other = 0usize;
    for entry in entries.flatten() {
        *examined += 1;
        if *examined > MAX_ENTRIES {
            report.incomplete = true;
            return;
        }
        match entry.file_type() {
            Ok(kind) if kind.is_file() => files.push(entry),
            Ok(kind) if kind.is_symlink() => {
                report.refused += 1;
                other += 1;
            }
            _ => other += 1,
        }
    }

    let old_enough = |entry: &fs::DirEntry| {
        entry
            .metadata()
            .is_ok_and(|meta| is_old(&meta, now, min_age))
    };

    // Rule 4: the source is gone and nothing in its folder is recent.
    if source_gone && dir_old && other == 0 && files.iter().all(old_enough) {
        let removal = capture_files::remove_dir_contents(dir);
        if removal.dir_removed {
            report.ghost_dirs += 1;
        }
        report.errors += removal.left;
        return;
    }

    let mut remaining = other;
    for entry in files {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let temp = is_temp(&file_name);
        // Rule 1 needs no database; rule 2 needs it to have answered.
        let orphan = !temp
            && known.is_some_and(|known| {
                let key = format!("{DIR}/{name}/{file_name}");
                !known.keys.contains(&key)
            });
        if (temp || orphan) && old_enough(&entry) {
            if fs::remove_file(entry.path()).is_ok() {
                if temp {
                    report.temp += 1;
                } else {
                    report.orphans += 1;
                }
                continue;
            }
            report.errors += 1;
        }
        remaining += 1;
    }

    // Rule 3: nothing left, and the folder is not new.
    if remaining == 0 && dir_old && fs::remove_dir(dir).is_ok() {
        report.empty_dirs += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navegador::capture_files::root;
    use crate::sync::test_support::new_app_schema_db;
    use std::fs;
    use std::path::PathBuf;

    const OLD: Duration = Duration::from_secs(3 * 3600);
    const FRESH: Duration = Duration::from_secs(10 * 60);

    struct Env {
        data: tempfile::TempDir,
        conn: Connection,
    }

    impl Env {
        fn new() -> Self {
            Self {
                data: tempfile::tempdir().unwrap(),
                conn: new_app_schema_db(),
            }
        }

        fn dir(&self, id: &str) -> PathBuf {
            let dir = root(self.data.path()).join(id);
            fs::create_dir_all(&dir).unwrap();
            dir
        }

        /// A source folder with its `web_sources` row.
        fn source(&self, id: &str) -> PathBuf {
            self.conn
                .execute(
                    "INSERT INTO web_sources
                       (id, original_url, final_url, first_accessed_at, created_at, updated_at)
                     VALUES (?1, 'https://e.com/', 'https://e.com/', 'x', 1, 1)",
                    [id],
                )
                .unwrap();
            self.dir(id)
        }

        fn capture(&self, source: &str, id: &str, rel: Option<&str>, text_rel: Option<&str>) {
            self.conn
                .execute(
                    "INSERT INTO web_captures
                       (id, web_source_id, accessed_at, final_url, kind, mime_type,
                        rel_path, text_rel_path, sha256, hash_of, size_bytes, created_at)
                     VALUES (?1, ?2, 'x', 'https://e.com/', 'page', 'text/html',
                             ?3, ?4, 'abc', 'html', 1, 1)",
                    rusqlite::params![id, source, rel, text_rel],
                )
                .unwrap();
        }

        /// A file at `<data>/<rel>` last modified `age` ago.
        fn file(&self, rel: &str, age: Duration) -> PathBuf {
            let path = self.data.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"x").unwrap();
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(SystemTime::now() - age).unwrap();
            path
        }

        fn sweep(&self) -> Report {
            sweep(self.data.path(), &self.conn, SystemTime::now(), MIN_AGE)
        }

        /// Sweep as if two hours had passed, so folders created a moment ago
        /// count as old (a folder's own age cannot be set portably).
        fn sweep_later(&self) -> Report {
            let later = SystemTime::now() + Duration::from_secs(2 * 3600);
            sweep(self.data.path(), &self.conn, later, MIN_AGE)
        }
    }

    /// A directory link (symlink, or a junction on Windows), if this machine
    /// lets the test make one.
    fn link_dir(link: &Path, target: &Path) -> bool {
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_dir(target, link).is_ok() {
                return true;
            }
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .map(|out| out.status.success())
                .unwrap_or(false)
        }
        #[cfg(not(windows))]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
    }

    #[test]
    fn an_old_temp_file_goes_and_a_fresh_one_stays() {
        let env = Env::new();
        env.source("src-1");
        env.capture(
            "src-1",
            "cap-1",
            Some("web-captures/src-1/cap-1.html"),
            None,
        );
        env.file("web-captures/src-1/cap-1.html", OLD);
        let old = env.file("web-captures/src-1/.cap-2.html.tmp", OLD);
        let fresh = env.file("web-captures/src-1/.cap-3.html.tmp", FRESH);

        let report = env.sweep();

        assert_eq!(report.temp, 1);
        assert!(!old.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn an_old_unreferenced_file_goes_and_a_referenced_one_stays() {
        let env = Env::new();
        env.source("src-1");
        env.capture(
            "src-1",
            "cap-1",
            Some("web-captures/src-1/cap-1.html"),
            Some("web-captures/src-1/cap-1.txt"),
        );
        let page = env.file("web-captures/src-1/cap-1.html", OLD);
        let text = env.file("web-captures/src-1/cap-1.txt", OLD);
        let orphan = env.file("web-captures/src-1/cap-9.html", OLD);
        let young_orphan = env.file("web-captures/src-1/cap-8.html", FRESH);

        let report = env.sweep();

        assert_eq!(report.orphans, 1);
        assert!(page.exists() && text.exists(), "referenced files stay");
        assert!(!orphan.exists());
        assert!(young_orphan.exists(), "a save may still be committing it");
    }

    #[test]
    fn a_file_is_referenced_by_its_key_in_the_source_it_sits_in() {
        let env = Env::new();
        env.source("src-1");
        env.source("src-2");
        // The row points into src-2, so the same name in src-1 is not it.
        env.capture(
            "src-2",
            "cap-1",
            Some("web-captures/src-2/cap-1.html"),
            None,
        );
        let kept = env.file("web-captures/src-2/cap-1.html", OLD);
        let stray = env.file("web-captures/src-1/cap-1.html", OLD);

        env.sweep();

        assert!(kept.exists());
        assert!(!stray.exists());
    }

    #[test]
    fn an_old_empty_source_folder_goes_and_a_fresh_one_stays() {
        let env = Env::new();
        let empty = env.source("src-empty");

        let fresh = env.sweep();
        assert_eq!(fresh.empty_dirs, 0);
        assert!(empty.exists(), "a save may be about to fill it");

        let later = env.sweep_later();
        assert_eq!(later.empty_dirs, 1);
        assert!(!empty.exists());
    }

    #[test]
    fn a_folder_left_empty_by_the_sweep_goes_in_the_same_pass() {
        let env = Env::new();
        env.source("src-1");
        let orphan = env.file("web-captures/src-1/cap-9.html", OLD);

        let report = env.sweep_later();

        assert_eq!(report.orphans, 1);
        assert_eq!(report.empty_dirs, 1);
        assert!(!orphan.exists());
        assert!(!root(env.data.path()).join("src-1").exists());
    }

    #[test]
    fn the_folder_of_a_deleted_source_goes_when_everything_in_it_is_old() {
        let env = Env::new();
        let a = env.file("web-captures/gone-1/cap-1.html", OLD);
        let b = env.file("web-captures/gone-1/cap-1.txt", OLD);

        let report = env.sweep_later();

        assert_eq!(report.ghost_dirs, 1);
        assert!(!a.exists() && !b.exists());
        assert!(!root(env.data.path()).join("gone-1").exists());
    }

    #[test]
    fn the_folder_of_a_deleted_source_stays_while_anything_in_it_is_fresh() {
        let env = Env::new();
        let old = env.file("web-captures/gone-1/cap-1.html", OLD);
        let fresh = env.file("web-captures/gone-1/cap-2.html", FRESH);

        let report = env.sweep();

        assert_eq!(report.ghost_dirs, 0);
        assert!(fresh.exists());
        // The old file is not referenced either, so it is an orphan on its own.
        assert!(!old.exists());
    }

    #[test]
    fn a_source_that_exists_keeps_its_referenced_files_however_old() {
        let env = Env::new();
        env.source("src-1");
        env.capture("src-1", "cap-1", Some("web-captures/src-1/cap-1.pdf"), None);
        let pdf = env.file("web-captures/src-1/cap-1.pdf", OLD);

        let report = env.sweep_later();

        assert_eq!(report, Report::default());
        assert!(pdf.exists());
    }

    #[test]
    fn nothing_to_sweep_is_not_an_error_and_creates_nothing() {
        let env = Env::new();
        assert_eq!(env.sweep(), Report::default());
        assert!(!root(env.data.path()).exists());
    }

    #[test]
    fn without_the_tables_only_the_database_free_rules_run() {
        let data = tempfile::tempdir().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        let env = Env { data, conn };
        let orphan = env.file("web-captures/src-1/cap-1.html", OLD);
        let tmp = env.file("web-captures/src-1/.cap-2.html.tmp", OLD);
        let ghost_file = env.file("web-captures/src-2/cap-3.html", OLD);

        let report = env.sweep_later();

        assert!(report.db_rules_skipped);
        assert_eq!(report.temp, 1);
        assert!(!tmp.exists());
        assert!(orphan.exists(), "an unreadable database never means orphan");
        assert!(ghost_file.exists());
        assert_eq!(report.orphans + report.ghost_dirs, 0);
    }

    #[test]
    fn a_failing_query_skips_the_database_rules_too() {
        let env = Env::new();
        // The table exists but not with the column the sweep reads.
        env.conn
            .execute_batch(
                "DROP TABLE web_captures;
                 CREATE TABLE web_captures (id TEXT PRIMARY KEY);",
            )
            .unwrap();
        let orphan = env.file("web-captures/src-1/cap-1.html", OLD);

        let report = env.sweep_later();

        assert!(report.db_rules_skipped);
        assert!(orphan.exists());
    }

    #[test]
    fn only_folders_named_like_ids_are_swept_and_loose_files_are_left() {
        let env = Env::new();
        let odd_dir = env.file("web-captures/not a source/x.html", OLD);
        let dotted = env.file("web-captures/a.b/x.html", OLD);
        let loose = env.file("web-captures/notes.txt", OLD);

        env.sweep_later();

        assert!(odd_dir.exists() && dotted.exists() && loose.exists());
    }

    #[test]
    fn the_folder_of_rendered_copies_is_not_taken_for_an_orphan_source() {
        let env = Env::new();
        let rendered = env.file("web-captures/_copy/c1-1-0.pdf", OLD);

        env.sweep_later();

        assert!(rendered.exists());
    }

    #[test]
    fn nothing_outside_web_captures_is_touched() {
        let env = Env::new();
        let asset = env.file("assets/old.bin", OLD);
        let sibling = env.file("web-captures-extra/src-1/x.html", OLD);
        let db = env.file("entropia.sqlite", OLD);

        env.sweep_later();

        assert!(asset.exists() && sibling.exists() && db.exists());
    }

    #[test]
    fn a_source_folder_that_is_a_link_is_never_entered() {
        let env = Env::new();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("precious.html");
        fs::write(&victim, b"x").unwrap();
        let old = SystemTime::now() - OLD;
        fs::File::options()
            .write(true)
            .open(&victim)
            .unwrap()
            .set_modified(old)
            .unwrap();
        fs::create_dir_all(root(env.data.path())).unwrap();
        let link = root(env.data.path()).join("src-1");
        if !link_dir(&link, outside.path()) {
            eprintln!("skipped: this machine cannot create a directory link");
            return;
        }

        let report = env.sweep_later();

        assert!(victim.exists(), "a file behind a link was deleted");
        assert!(report.refused >= 1);
        assert_eq!(report.orphans + report.temp + report.ghost_dirs, 0);
    }

    #[test]
    fn a_root_that_is_a_link_is_refused_whole() {
        let env = Env::new();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("src-1").join("x.html");
        fs::create_dir_all(victim.parent().unwrap()).unwrap();
        fs::write(&victim, b"x").unwrap();
        if !link_dir(&root(env.data.path()), outside.path()) {
            eprintln!("skipped: this machine cannot create a directory link");
            return;
        }

        let report = env.sweep_later();

        assert!(victim.exists());
        assert_eq!(report.refused, 1);
    }

    #[test]
    fn the_app_starts_the_sweep_at_setup() {
        // The sweep only helps if something runs it: this reads the setup code.
        let lib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
        let body = fs::read_to_string(lib).unwrap();
        assert!(body.contains("navegador::sweep_captures("));
    }

    #[test]
    fn the_summary_says_what_was_removed_and_what_was_skipped() {
        let report = Report {
            temp: 1,
            orphans: 2,
            empty_dirs: 3,
            ghost_dirs: 4,
            refused: 5,
            errors: 6,
            db_rules_skipped: true,
            incomplete: true,
        };
        let line = report.summary();
        assert!(line.starts_with("web-captures sweep: 1 temporary"));
        assert!(line.contains("2 orphan") && line.contains("3 empty") && line.contains("4 folder"));
        assert!(line.contains("5 refused") && line.contains("6 error"));
        assert!(line.contains("skipped") && line.contains("entry limit"));
        let quiet = Report::default().summary();
        assert!(!quiet.contains("skipped") && !quiet.contains("entry limit"));
    }
}
