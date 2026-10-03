//! One-off startup sweep of orphaned folders under `<data>/assets/`.
//!
//! Before a pulled asset delete removed its own file, a device that synced
//! kept the files of deleted items forever. This sweep finds them and MOVES
//! them into a recoverable quarantine, once per archive. It never deletes.
//! Losing a user's file is the worst outcome, so the rule is narrow and
//! location-based, not reference-based:
//!
//! * `assets/<collection_id>/<item_id>/...` is an orphan only when that item is
//!   no longer in `items`, or its collection is no longer in `collections`;
//! * `assets/<collection_id>/<file>` (directly under a collection folder) only
//!   when that collection is gone;
//! * anything under a LIVE item's folder is never touched, referenced or not:
//!   originals and superseded edit versions of a live image live there;
//! * any other shape (a file directly under `assets/`, a folder name that is
//!   not a plain id) is kept and logged;
//! * a file an `assets` row, a pending download or a web capture still names is
//!   kept even under a gone item (and logged);
//! * nothing younger than [`MIN_AGE`] moves, links are never entered, and a
//!   path must pass the same inbound validation as a sync path.
//!
//! Orphans go to `<data>/orphaned-assets/<YYYY-MM-DD>/<original relative
//! path>`, never overwriting what is there. The sweep needs the database to
//! answer completely (`assets`, `items`, `collections`, none of them empty
//! beside files on disk), otherwise it touches nothing and stays pending.
//! Completion is recorded in `app_settings` ([`SETTING_KEY`]) only when the run
//! saw everything and nothing failed or was too young.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::Connection;

/// `app_settings` key that records a completed sweep.
pub const SETTING_KEY: &str = "assets_orphan_sweep_v2";

/// Quarantine folder under the data directory.
pub const QUARANTINE_DIR: &str = "orphaned-assets";

/// Nothing younger than this is moved.
pub const MIN_AGE: Duration = Duration::from_secs(60 * 60);

/// Most entries one sweep looks at: a bound, not an expectation.
pub const MAX_ENTRIES: usize = 200_000;

/// Most "kept for a doubt" lines written to stderr.
const MAX_LOGGED_DOUBTS: usize = 20;

/// One file the plan moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    /// Path relative to the data directory, with `/`.
    pub rel: String,
    pub size: u64,
}

/// What a sweep saw and did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Files looked at.
    pub examined: usize,
    /// Files under a live collection and item: never touched.
    pub kept_live: usize,
    /// Files kept because their shape is unexpected or a row still names them.
    pub kept_doubt: usize,
    /// Orphans kept because they are too young.
    pub young: usize,
    /// Links and paths that failed validation.
    pub refused: usize,
    /// Files moved to the quarantine.
    pub moved: usize,
    /// Bytes moved.
    pub moved_bytes: u64,
    /// Moves and reads that failed.
    pub errors: usize,
    /// The database could not answer completely; nothing was touched.
    pub db_skipped: bool,
    /// A table the rule needs is empty beside files; nothing was touched.
    pub empty_table_guard: bool,
    /// Stopped at [`MAX_ENTRIES`].
    pub incomplete: bool,
    /// Relative paths kept for a doubt (bounded sample).
    pub doubts: Vec<String>,
}

/// The decision, before anything moves.
#[derive(Debug, Default)]
pub struct Plan {
    pub moves: Vec<Move>,
    pub report: Report,
}

impl Report {
    /// True when the sweep saw everything and nothing is left for a retry.
    pub fn complete(&self) -> bool {
        !self.db_skipped
            && !self.empty_table_guard
            && !self.incomplete
            && self.young == 0
            && self.errors == 0
    }

    /// One line for the app log.
    pub fn summary(&self) -> String {
        let mut line = format!(
            "assets orphan sweep: {} file(s) examined, {} kept under live items; moved {} \
             orphan file(s) ({} bytes) to {QUARANTINE_DIR}/; kept {} for a doubt, {} young, \
             {} refused; {} error(s)",
            self.examined,
            self.kept_live,
            self.moved,
            self.moved_bytes,
            self.kept_doubt,
            self.young,
            self.refused,
            self.errors
        );
        if self.db_skipped {
            line.push_str("; skipped (database not ready)");
        }
        if self.empty_table_guard {
            line.push_str("; skipped (a table is empty but files are present)");
        }
        if self.incomplete {
            line.push_str("; stopped at the entry limit");
        }
        if self.complete() {
            line.push_str("; done");
        } else {
            line.push_str("; will run again at the next start");
        }
        line
    }
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |_| Ok(()),
    )
    .is_ok()
}

fn column_of(conn: &Connection, sql: &str) -> Option<Vec<String>> {
    let mut statement = conn.prepare(sql).ok()?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .ok()?;
    rows.collect::<Result<Vec<_>, _>>().ok()
}

/// What the database says exists.
struct Known {
    /// Relative keys (lowercase, `/`) of every file a row names.
    keys: HashSet<String>,
    /// Stored values that could not be reduced to a relative key: a file whose
    /// key ends them is kept.
    unresolved: Vec<String>,
    /// Lowercase ids.
    collections: HashSet<String>,
    items: HashSet<String>,
    asset_rows: usize,
}

impl Known {
    /// `None` when the database cannot answer completely. Never partial.
    fn read(conn: &Connection, data_dir: &Path) -> Option<Self> {
        for table in ["assets", "items", "collections"] {
            if !table_exists(conn, table) {
                return None;
            }
        }
        let paths = column_of(conn, "SELECT path FROM assets WHERE path IS NOT NULL")?;
        let asset_rows = paths.len();
        let collections = column_of(conn, "SELECT id FROM collections")?
            .into_iter()
            .map(|id| id.to_lowercase())
            .collect();
        let items = column_of(conn, "SELECT id FROM items")?
            .into_iter()
            .map(|id| id.to_lowercase())
            .collect();
        let mut extra = Vec::new();
        for (table, sql) in [
            (
                "sync_pending_blobs",
                "SELECT rel_path FROM sync_pending_blobs WHERE rel_path IS NOT NULL",
            ),
            (
                "sync_web_pending_blobs",
                "SELECT rel_path FROM sync_web_pending_blobs WHERE rel_path IS NOT NULL",
            ),
            (
                "web_captures",
                "SELECT rel_path FROM web_captures WHERE rel_path IS NOT NULL
                 UNION
                 SELECT text_rel_path FROM web_captures WHERE text_rel_path IS NOT NULL",
            ),
        ] {
            if table_exists(conn, table) {
                extra.extend(column_of(conn, sql)?);
            }
        }

        let mut keys = HashSet::new();
        let mut unresolved = Vec::new();
        for stored in paths.iter().chain(extra.iter()) {
            let norm = stored.trim().replace('\\', "/");
            if norm.is_empty() {
                continue;
            }
            let rel = if Path::new(&norm).is_absolute() || norm.as_bytes().get(1) == Some(&b':') {
                match crate::path_utils::derive_rel_path(&norm, data_dir) {
                    Ok(rel) => rel,
                    Err(_) => {
                        unresolved.push(norm.to_lowercase());
                        continue;
                    }
                }
            } else {
                norm
            };
            keys.insert(rel.to_lowercase());
        }
        Some(Self {
            keys,
            unresolved,
            collections,
            items,
            asset_rows,
        })
    }

    fn names(&self, key: &str) -> bool {
        self.keys.contains(key) || self.unresolved.iter().any(|stored| stored.ends_with(key))
    }
}

/// An id-shaped folder name: the only kind the rule reasons about.
fn is_plain_id(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

enum Place {
    /// Under a live collection and item (or the live collection's own files
    /// are handled as `Unexpected`).
    Live,
    /// Provably dead by its location.
    Orphan,
    /// A shape the rule does not cover.
    Unexpected,
}

/// Classify a file by its `assets/<collection>/<item>/...` location.
fn place_of(known: &Known, rel: &str) -> Place {
    let parts: Vec<&str> = rel.split('/').collect();
    match parts.len() {
        // `assets/<file>`
        0..=2 => Place::Unexpected,
        // `assets/<collection>/<file>`
        3 => {
            // A live collection's own loose files are not covered by the rule.
            if !is_plain_id(parts[1]) || known.collections.contains(&parts[1].to_lowercase()) {
                Place::Unexpected
            } else {
                Place::Orphan
            }
        }
        _ => {
            if !is_plain_id(parts[1]) || !is_plain_id(parts[2]) {
                return Place::Unexpected;
            }
            let collection_live = known.collections.contains(&parts[1].to_lowercase());
            let item_live = known.items.contains(&parts[2].to_lowercase());
            if collection_live && item_live {
                Place::Live
            } else {
                Place::Orphan
            }
        }
    }
}

fn is_old(meta: &fs::Metadata, now: SystemTime, min_age: Duration) -> bool {
    meta.modified()
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age > min_age)
}

fn doubt(report: &mut Report, rel: String) {
    report.kept_doubt += 1;
    if report.doubts.len() < MAX_LOGGED_DOUBTS {
        report.doubts.push(rel);
    }
}

/// Decide what would move. Touches nothing. `now` and `min_age` are injected so
/// tests are exact.
pub fn plan(data_dir: &Path, conn: &Connection, now: SystemTime, min_age: Duration) -> Plan {
    let mut plan = Plan::default();
    let assets_root = data_dir.join("assets");
    match fs::symlink_metadata(&assets_root) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            plan.report.refused += 1;
            return plan;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return plan,
        Err(_) => {
            plan.report.errors += 1;
            return plan;
        }
    }
    let Some(known) = Known::read(conn, data_dir) else {
        plan.report.db_skipped = true;
        return plan;
    };

    let report = &mut plan.report;
    let mut stack: Vec<(PathBuf, String)> = vec![(assets_root.clone(), "assets".to_string())];
    let mut files_seen = 0usize;
    'walk: while let Some((dir, rel_dir)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            report.errors += 1;
            continue;
        };
        for entry in entries.flatten() {
            report.examined += 1;
            if report.examined > MAX_ENTRIES {
                report.incomplete = true;
                break 'walk;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                report.refused += 1;
                doubt(report, format!("{rel_dir}/<non-UTF-8 name>"));
                continue;
            };
            let rel = format!("{rel_dir}/{name}");
            // `DirEntry::file_type` does not follow links.
            let Ok(kind) = entry.file_type() else {
                report.errors += 1;
                continue;
            };
            if kind.is_symlink() {
                report.refused += 1;
                doubt(report, rel);
                continue;
            }
            if kind.is_dir() {
                stack.push((entry.path(), rel));
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            files_seen += 1;
            // In-flight downloads belong to the sync blob cleanup.
            if name.ends_with(".part") {
                continue;
            }
            match place_of(&known, &rel) {
                Place::Live => report.kept_live += 1,
                Place::Unexpected => doubt(report, rel),
                Place::Orphan => {
                    if known.names(&rel.to_lowercase()) {
                        doubt(report, rel);
                        continue;
                    }
                    let Ok(meta) = entry.metadata() else {
                        report.errors += 1;
                        continue;
                    };
                    if !is_old(&meta, now, min_age) {
                        report.young += 1;
                        continue;
                    }
                    plan.moves.push(Move {
                        rel,
                        size: meta.len(),
                    });
                }
            }
        }
    }

    // A table with no rows beside files on disk is a database that is not
    // there yet (or was restored empty), not an archive of orphans.
    let empty = known.asset_rows == 0 || known.items.is_empty() || known.collections.is_empty();
    if empty && files_seen > 0 {
        plan.report.empty_table_guard = true;
        plan.moves.clear();
    }
    if plan.report.incomplete {
        plan.moves.clear();
    }
    plan
}

/// First free destination: `<quarantine>/<date>/<rel>`, then `.dup1`, `.dup2`...
fn free_destination(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut dest = base.to_path_buf();
    for component in rel.split('/') {
        dest.push(component);
    }
    if fs::symlink_metadata(&dest).is_err() {
        return Some(dest);
    }
    for n in 1..1000 {
        let mut name = dest.file_name()?.to_os_string();
        name.push(format!(".dup{n}"));
        let candidate = dest.with_file_name(name);
        if fs::symlink_metadata(&candidate).is_err() {
            return Some(candidate);
        }
    }
    None
}

/// Moves the plan's files into `<data>/orphaned-assets/<date>/`. Nothing is
/// ever deleted or overwritten.
pub fn apply(data_dir: &Path, plan: &mut Plan, date: &str) {
    let assets_root = data_dir.join("assets");
    let quarantine = data_dir.join(QUARANTINE_DIR);
    if let Ok(meta) = fs::symlink_metadata(&quarantine) {
        if !meta.is_dir() {
            // A link or a file where the quarantine belongs: touch nothing.
            plan.report.errors += plan.moves.len();
            return;
        }
    }
    let base = quarantine.join(date);
    let mut parents: Vec<PathBuf> = Vec::new();
    for item in &plan.moves {
        // The same validation a synced path goes through, then the canonical
        // check against the assets root and a last look at what is on disk.
        let Ok(source) = crate::sync::apply::validate_inbound_rel_path(&item.rel, data_dir) else {
            plan.report.refused += 1;
            continue;
        };
        if crate::path_utils::ensure_within_dir(&source, &assets_root).is_err()
            || !fs::symlink_metadata(&source).is_ok_and(|meta| meta.is_file())
        {
            plan.report.refused += 1;
            continue;
        }
        let Some(dest) = free_destination(&base, &item.rel) else {
            plan.report.errors += 1;
            continue;
        };
        let moved = dest
            .parent()
            .map(fs::create_dir_all)
            .unwrap_or(Ok(()))
            .and_then(|()| fs::rename(&source, &dest));
        match moved {
            Ok(()) => {
                plan.report.moved += 1;
                plan.report.moved_bytes += item.size;
                if let Some(parent) = source.parent() {
                    parents.push(parent.to_path_buf());
                }
            }
            Err(error) => {
                eprintln!("[assets] could not quarantine {}: {error}", item.rel);
                plan.report.errors += 1;
            }
        }
    }
    // Folders the moves left empty go, never the root and never one that still
    // holds anything.
    for start in parents {
        let mut dir = Some(start.as_path());
        while let Some(d) = dir {
            if d == assets_root || !d.starts_with(&assets_root) || fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
    }
}

/// `YYYY-MM-DD` (UTC) of a point in time.
pub fn ymd(time: SystemTime) -> String {
    let days = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86_400) as i64)
        .unwrap_or(0);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

/// Plan and apply in one go, without recording anything.
pub fn sweep(data_dir: &Path, conn: &Connection, now: SystemTime, min_age: Duration) -> Report {
    let mut plan = plan(data_dir, conn, now, min_age);
    apply(data_dir, &mut plan, &ymd(now));
    plan.report
}

/// Runs the sweep unless this archive already completed it, and records the
/// completion. Returns the line for the log.
pub fn run_once(data_dir: &Path, conn: &Connection, now: SystemTime, min_age: Duration) -> String {
    if !table_exists(conn, "app_settings") {
        return "assets orphan sweep skipped: settings not ready".to_string();
    }
    let done: Option<String> = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [SETTING_KEY],
            |row| row.get(0),
        )
        .ok();
    if done.is_some() {
        return "assets orphan sweep: already done for this archive".to_string();
    }
    let report = sweep(data_dir, conn, now, min_age);
    for rel in &report.doubts {
        eprintln!("[assets] kept for a doubt: {rel}");
    }
    let mut line = report.summary();
    if report.complete() {
        if let Err(error) = crate::settings::set_setting(conn, SETTING_KEY, "done") {
            line.push_str(&format!("; could not record completion: {error}"));
        }
    }
    line
}

/// Background thread: opens its own connection, logs one line, and can fail in
/// any way without touching startup or the archive.
pub fn spawn(app: tauri::AppHandle, data_dir: PathBuf, db_path: PathBuf) {
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match crate::db::open::open_archive_connection(&db_path) {
                Ok(conn) => run_once(&data_dir, &conn, SystemTime::now(), MIN_AGE),
                Err(error) => format!("assets orphan sweep skipped: {error}"),
            }
        }));
        let line =
            outcome.unwrap_or_else(|_| "assets orphan sweep stopped by an error".to_string());
        eprintln!("[assets] {line}");
        crate::app_logs::info(&app, "assets", line);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn put(dir: &Path, rel: &str) -> PathBuf {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"bytes").unwrap();
        path
    }

    /// Collections `c1`, items `i1`, and one asset row per given path.
    fn db(paths: &[&str]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE assets (id TEXT PRIMARY KEY, path TEXT NOT NULL);
             CREATE TABLE collections (id TEXT PRIMARY KEY);
             CREATE TABLE items (id TEXT PRIMARY KEY);
             CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE sync_pending_blobs (asset_id TEXT PRIMARY KEY, rel_path TEXT NOT NULL);
             INSERT INTO collections(id) VALUES ('c1');
             INSERT INTO items(id) VALUES ('i1');",
        )
        .unwrap();
        for (i, path) in paths.iter().enumerate() {
            conn.execute(
                "INSERT INTO assets(id, path) VALUES (?1, ?2)",
                rusqlite::params![format!("a{i}"), path],
            )
            .unwrap();
        }
        conn
    }

    /// A clock far enough ahead that every file written now is old.
    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(3 * 60 * 60)
    }

    fn quarantined(dir: &Path, rel: &str) -> bool {
        let day = ymd(later());
        dir.join(QUARANTINE_DIR).join(day).join(rel).exists()
    }

    #[test]
    fn the_owners_shape_keeps_everything_under_a_live_item_and_moves_a_gone_item() {
        let dir = tempfile::tempdir().unwrap();
        // Live item: row points at _o_v3.png; original and v2 are unreferenced.
        let o = put(dir.path(), "assets/c1/i1/photo_o.jpg");
        let v2 = put(dir.path(), "assets/c1/i1/photo_o_v2.png");
        let v3 = put(dir.path(), "assets/c1/i1/photo_o_v3.png");
        let page = put(dir.path(), "assets/c1/i1/doc_page_5.pdf");
        let page2 = put(dir.path(), "assets/c1/i1/doc_page_5_v2.pdf");
        let stray = put(dir.path(), "assets/c1/i1/sub/anything.bin");
        // Gone item under a live collection.
        let gone = put(dir.path(), "assets/c1/dead-item/old.jpg");
        let conn = db(&["assets/c1/i1/photo_o_v3.png"]);

        let report = sweep(dir.path(), &conn, later(), MIN_AGE);

        for kept in [&o, &v2, &v3, &page, &page2, &stray] {
            assert!(kept.exists(), "{} must stay", kept.display());
        }
        assert!(!gone.exists());
        assert!(quarantined(dir.path(), "assets/c1/dead-item/old.jpg"));
        assert!(!dir.path().join("assets/c1/dead-item").exists());
        assert_eq!(report.moved, 1);
        assert_eq!(report.moved_bytes, 5);
        assert_eq!(report.kept_live, 6);
        assert!(report.complete());
    }

    #[test]
    fn a_gone_collection_moves_everything_below_it() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "assets/c1/i1/keep.png");
        let a = put(dir.path(), "assets/dead-col/i9/a.png");
        let b = put(dir.path(), "assets/dead-col/loose.png");
        let conn = db(&["assets/c1/i1/keep.png"]);

        let report = sweep(dir.path(), &conn, later(), MIN_AGE);

        assert!(!a.exists() && !b.exists());
        assert!(quarantined(dir.path(), "assets/dead-col/i9/a.png"));
        assert!(quarantined(dir.path(), "assets/dead-col/loose.png"));
        assert_eq!(report.moved, 2);
    }

    #[test]
    fn unexpected_shapes_are_kept_and_logged() {
        let dir = tempfile::tempdir().unwrap();
        let top = put(dir.path(), "assets/loose.png");
        let live_col_file = put(dir.path(), "assets/c1/loose.png");
        let odd = put(dir.path(), "assets/we ird/i9/x.png");
        put(dir.path(), "assets/c1/i1/keep.png");
        let conn = db(&["assets/c1/i1/keep.png"]);

        let report = sweep(dir.path(), &conn, later(), MIN_AGE);

        assert!(top.exists() && live_col_file.exists() && odd.exists());
        assert_eq!(report.moved, 0);
        assert_eq!(report.kept_doubt, 3);
    }

    #[test]
    fn a_file_a_row_still_names_stays_even_under_a_gone_item() {
        let dir = tempfile::tempdir().unwrap();
        let named = put(dir.path(), "assets/c1/dead/named.png");
        let pending = put(dir.path(), "assets/c1/dead/incoming.png");
        let legacy = put(dir.path(), "assets/c1/dead/legacy.png");
        let conn = db(&[
            "assets/c1/dead/named.png",
            &legacy.to_string_lossy(),
            "assets/c1/i1/x.png",
        ]);
        conn.execute(
            "INSERT INTO sync_pending_blobs(asset_id, rel_path) VALUES ('p', 'assets/c1/dead/incoming.png')",
            [],
        )
        .unwrap();

        let report = sweep(dir.path(), &conn, later(), MIN_AGE);

        assert!(named.exists() && pending.exists() && legacy.exists());
        assert_eq!(report.moved, 0);
    }

    #[test]
    fn keeps_young_orphans_and_stays_pending() {
        let dir = tempfile::tempdir().unwrap();
        let young = put(dir.path(), "assets/c1/dead/new.png");
        let conn = db(&["assets/c1/i1/other.png"]);

        let report = sweep(dir.path(), &conn, SystemTime::now(), MIN_AGE);

        assert_eq!(report.young, 1);
        assert!(young.exists());
        assert!(!report.complete());
    }

    #[test]
    fn never_overwrites_in_the_quarantine() {
        let dir = tempfile::tempdir().unwrap();
        let first = put(dir.path(), "assets/c1/dead/a.png");
        let conn = db(&["assets/c1/i1/x.png"]);
        sweep(dir.path(), &conn, later(), MIN_AGE);
        assert!(!first.exists());
        put(dir.path(), "assets/c1/dead/a.png");

        sweep(dir.path(), &conn, later(), MIN_AGE);

        let day = ymd(later());
        let base = dir
            .path()
            .join(QUARANTINE_DIR)
            .join(day)
            .join("assets/c1/dead");
        assert!(base.join("a.png").exists());
        assert!(
            base.join("a.png.dup1").exists(),
            "the second copy got its own name"
        );
    }

    #[test]
    fn never_touches_what_is_outside_assets_and_leaves_part_files() {
        let dir = tempfile::tempdir().unwrap();
        let outside = put(dir.path(), "web-captures/dead/page.html");
        let writing = put(dir.path(), "writing-images/abc.png");
        let part = put(dir.path(), "assets/c1/dead/big.mp4.part");
        let conn = db(&["assets/c1/i1/x.png"]);

        sweep(dir.path(), &conn, later(), MIN_AGE);

        assert!(outside.exists() && writing.exists() && part.exists());
    }

    #[test]
    fn does_nothing_when_the_database_cannot_answer_or_a_table_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = put(dir.path(), "assets/c1/dead/x.png");
        let bare = Connection::open_in_memory().unwrap();
        let report = sweep(dir.path(), &bare, later(), MIN_AGE);
        assert!(report.db_skipped && !report.complete());

        let empty = db(&[]);
        let report = sweep(dir.path(), &empty, later(), MIN_AGE);
        assert!(report.empty_table_guard && !report.complete());

        let no_items = db(&["assets/c1/i1/x.png"]);
        no_items.execute("DELETE FROM items", []).unwrap();
        let report = sweep(dir.path(), &no_items, later(), MIN_AGE);
        assert!(report.empty_table_guard);
        assert!(file.exists());
    }

    #[test]
    fn a_missing_assets_folder_is_a_clean_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db(&["assets/x.png"]);
        let report = sweep(dir.path(), &conn, later(), MIN_AGE);
        assert_eq!(report, Report::default());
        assert!(report.complete());
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_or_moves_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = put(outside.path(), "secret.txt");
        fs::create_dir_all(dir.path().join("assets/c1/dead")).unwrap();
        std::os::unix::fs::symlink(&secret, dir.path().join("assets/c1/dead/link.txt")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("assets/dirlink")).unwrap();
        let conn = db(&["assets/c1/i1/other.png"]);

        let report = sweep(dir.path(), &conn, later(), MIN_AGE);

        assert!(secret.exists());
        assert_eq!(report.refused, 2);
    }

    #[test]
    fn runs_once_per_archive_and_records_it() {
        let dir = tempfile::tempdir().unwrap();
        let orphan = put(dir.path(), "assets/c1/dead/gone.png");
        let conn = db(&["assets/c1/i1/keep.png"]);

        let first = run_once(dir.path(), &conn, later(), MIN_AGE);
        assert!(first.contains("moved 1 orphan file(s)") && first.ends_with("done"));
        assert!(!orphan.exists());

        let again = put(dir.path(), "assets/c1/dead2/later.png");
        let second = run_once(dir.path(), &conn, later(), MIN_AGE);
        assert!(second.contains("already done"));
        assert!(again.exists(), "the second run touches nothing");
    }

    #[test]
    fn an_incomplete_run_is_not_recorded() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "assets/c1/dead/young.png");
        let conn = db(&["assets/c1/i1/keep.png"]);

        let line = run_once(dir.path(), &conn, SystemTime::now(), MIN_AGE);

        assert!(line.contains("will run again"));
        let recorded: i64 = conn
            .query_row("SELECT COUNT(*) FROM app_settings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(recorded, 0);
    }

    #[test]
    fn ymd_is_the_utc_calendar_date() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_791_000_000);
        assert_eq!(ymd(t), "2026-10-03");
        assert_eq!(ymd(SystemTime::UNIX_EPOCH), "1970-01-01");
    }

    /// Dry run against a real archive, read-only: prints the plan and moves
    /// nothing. `cargo test --lib dry_run_real_archive -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dry_run_real_archive() {
        let data = std::env::var("ENTROPIA_REAL_DATA_DIR")
            .unwrap_or_else(|_| r"C:\Users\agusn\AppData\Roaming\com.entropia.shared".to_string());
        let data = PathBuf::from(data);
        let db_path = data.join("entropia.sqlite");
        let uri = format!(
            "file:{}?mode=ro",
            db_path.to_string_lossy().replace('\\', "/")
        );
        let conn = Connection::open_with_flags(
            &uri,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .expect("open read-only");
        let plan = plan(&data, &conn, SystemTime::now(), MIN_AGE);
        println!("{}", plan.report.summary());
        println!("planned moves: {}", plan.moves.len());
        let bytes: u64 = plan.moves.iter().map(|m| m.size).sum();
        println!("planned bytes: {bytes}");
        for m in &plan.moves {
            println!("MOVE {} ({} bytes)", m.rel, m.size);
        }
        for d in &plan.report.doubts {
            println!("DOUBT {d}");
        }
    }
}
