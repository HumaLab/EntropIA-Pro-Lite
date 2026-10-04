//! One-time compaction of the archive, taken at app close.
//!
//! The archive is created with `auto_vacuum = NONE`, so pages freed by a big
//! delete (the checkpoint purge, a removed collection) stay in the file as
//! reusable free pages: a 12 GB archive can hold a few GB of data. Only a full
//! `VACUUM` returns that space, and it also is the only moment `auto_vacuum`
//! can change, so the same pass switches the archive to `INCREMENTAL`. From
//! then on `reclaim_free_pages` gives space back in bounded steps at startup
//! and no second `VACUUM` is ever needed.
//!
//! A `VACUUM` of a multi-GB file takes minutes and needs the database to
//! itself, so it belongs at close (see `run_close_sequence` in `lib.rs`), never
//! at startup, and only when it pays off:
//!
//! - the archive is still `auto_vacuum = NONE`;
//! - free pages are a big share of the file AND a big absolute amount;
//! - the volumes involved have the room a `VACUUM` needs (a `VACUUM` writes the
//!   rebuilt database to a temp file and, in WAL mode, again through the WAL);
//! - nobody else holds the database: a TRUNCATE checkpoint that cannot finish
//!   within a short wait means a reader or writer is still attached, and the
//!   compaction is skipped until the next close.
//!
//! Safety. `VACUUM` is one atomic transaction: a process killed in the middle
//! leaves either the old archive or the new one, and WAL recovery on the next
//! open settles which. It also keeps rowids, which matters here because
//! `fts_items.rowid` mirrors `items.rowid` (see the rowid test below).

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::Connection;

use super::open::open_archive_connection;
use crate::processing::repository;

/// `app_settings` key recording the last completed compaction.
pub const SETTING_KEY: &str = "archive_compaction_v1";

/// Thresholds of one compaction decision. [`Policy::default`] is production;
/// tests shrink the numbers instead of building a gigabyte archive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Policy {
    /// Free pages must be more than this share of the file.
    pub min_free_ratio: f64,
    /// ...and more than this many bytes (a small file is not worth a close delay).
    pub min_free_bytes: u64,
    /// Room that must stay free on a volume after the work, on top of what it needs.
    pub disk_margin_bytes: u64,
    /// How long to wait for the database to be free before skipping.
    pub exclusivity_wait: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            min_free_ratio: 0.30,
            min_free_bytes: 200 * 1024 * 1024,
            disk_margin_bytes: 512 * 1024 * 1024,
            exclusivity_wait: Duration::from_secs(2),
        }
    }
}

/// What the page counters say about the archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveStats {
    pub page_size: u64,
    pub page_count: u64,
    pub freelist_count: u64,
    pub auto_vacuum: i64,
}

impl ArchiveStats {
    pub fn total_bytes(&self) -> u64 {
        self.page_size * self.page_count
    }

    pub fn free_bytes(&self) -> u64 {
        self.page_size * self.freelist_count
    }

    pub fn used_bytes(&self) -> u64 {
        self.total_bytes().saturating_sub(self.free_bytes())
    }
}

/// Why a compaction did not run. Never an error: the next close tries again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// `auto_vacuum` is already `FULL`/`INCREMENTAL`: free pages go back by themselves.
    NotNeeded(i64),
    FreeBelowFloor {
        free_bytes: u64,
    },
    FreeBelowRatio {
        free_bytes: u64,
        total_bytes: u64,
    },
    DiskSpaceUnknown,
    NotEnoughDisk {
        needed: u64,
        available: u64,
    },
    /// Another connection still holds the archive.
    InUse,
}

impl SkipReason {
    pub fn describe(&self) -> String {
        match self {
            Self::NotNeeded(mode) => format!("auto_vacuum already {mode}"),
            Self::FreeBelowFloor { free_bytes } => {
                format!("only {} free, below the floor", mib(*free_bytes))
            }
            Self::FreeBelowRatio {
                free_bytes,
                total_bytes,
            } => format!(
                "{} free of {}, below the ratio",
                mib(*free_bytes),
                mib(*total_bytes)
            ),
            Self::DiskSpaceUnknown => "free disk space unknown".to_string(),
            Self::NotEnoughDisk { needed, available } => format!(
                "not enough free disk space ({} needed, {} available)",
                mib(*needed),
                mib(*available)
            ),
            Self::InUse => "the archive is still in use by another connection".to_string(),
        }
    }
}

fn mib(bytes: u64) -> String {
    format!("{} MiB", bytes / (1024 * 1024))
}

/// Result of one attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Compacted {
        before_bytes: u64,
        after_bytes: u64,
        elapsed: Duration,
    },
    Skipped(SkipReason),
}

impl Outcome {
    /// The one line the app log gets.
    pub fn summary(&self) -> String {
        match self {
            Self::Compacted {
                before_bytes,
                after_bytes,
                elapsed,
            } => format!(
                "Archivo compactado: {} -> {} en {:.1} s (auto_vacuum = INCREMENTAL)",
                mib(*before_bytes),
                mib(*after_bytes),
                elapsed.as_secs_f64()
            ),
            Self::Skipped(reason) => format!("Compactación omitida: {}", reason.describe()),
        }
    }
}

/// Reads the counters the decision is made from.
pub fn read_stats(conn: &Connection) -> Result<ArchiveStats, String> {
    let pragma = |name: &str| -> Result<i64, String> {
        conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
            .map_err(|e| format!("Failed to read {name}: {e}"))
    };
    Ok(ArchiveStats {
        page_size: pragma("page_size")?.max(0) as u64,
        page_count: pragma("page_count")?.max(0) as u64,
        freelist_count: pragma("freelist_count")?.max(0) as u64,
        auto_vacuum: pragma("auto_vacuum")?,
    })
}

/// Free space, in bytes, on the volume holding `path`; `None` when the OS
/// cannot say (the compaction is then skipped rather than guessed).
pub fn available_disk_space(path: &Path) -> Option<u64> {
    fs2::available_space(path).ok()
}

/// Bytes the archive's own volume must have: a `VACUUM` in WAL mode writes the
/// rebuilt content through the WAL while the old file is still in place, so it
/// needs about the live data again; the owner's rule (the whole file's size) is
/// the floor when that is larger.
fn required_archive_volume_bytes(stats: &ArchiveStats, policy: &Policy) -> u64 {
    stats
        .total_bytes()
        .max(stats.used_bytes().saturating_mul(2))
        .saturating_add(policy.disk_margin_bytes)
}

/// The temp volume holds the rebuilt copy while it is built (the live data).
fn required_temp_volume_bytes(stats: &ArchiveStats, policy: &Policy) -> u64 {
    stats.used_bytes().saturating_add(policy.disk_margin_bytes)
}

/// The decision, pure: counters plus free space of the archive and temp volumes.
pub fn assess(
    stats: &ArchiveStats,
    policy: &Policy,
    archive_volume_free: Option<u64>,
    temp_volume_free: Option<u64>,
) -> Result<(), SkipReason> {
    if stats.auto_vacuum != 0 {
        return Err(SkipReason::NotNeeded(stats.auto_vacuum));
    }
    let free_bytes = stats.free_bytes();
    if free_bytes <= policy.min_free_bytes {
        return Err(SkipReason::FreeBelowFloor { free_bytes });
    }
    let total_bytes = stats.total_bytes();
    if (free_bytes as f64) <= policy.min_free_ratio * total_bytes as f64 {
        return Err(SkipReason::FreeBelowRatio {
            free_bytes,
            total_bytes,
        });
    }
    let (Some(archive_free), Some(temp_free)) = (archive_volume_free, temp_volume_free) else {
        return Err(SkipReason::DiskSpaceUnknown);
    };
    let needed = required_archive_volume_bytes(stats, policy);
    if archive_free < needed {
        return Err(SkipReason::NotEnoughDisk {
            needed,
            available: archive_free,
        });
    }
    let needed = required_temp_volume_bytes(stats, policy);
    if temp_free < needed {
        return Err(SkipReason::NotEnoughDisk {
            needed,
            available: temp_free,
        });
    }
    Ok(())
}

fn is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

/// `wal_checkpoint(TRUNCATE)` waits for every reader and writer to leave. It
/// reports `busy = 1` (or fails busy) when one did not within the wait, which
/// is exactly "someone else holds the archive".
fn checkpoint_truncate(conn: &Connection) -> Result<bool, rusqlite::Error> {
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
    Ok(busy == 0)
}

fn file_bytes(db_path: &Path) -> u64 {
    std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0)
}

/// Cheap, read-only: would a compaction run now? Lets the close sequence decide
/// whether to show the notice and stop the background workers at all.
pub fn is_worthwhile(db_path: &Path, policy: &Policy) -> Result<Result<(), SkipReason>, String> {
    let conn = open_archive_connection(db_path)?;
    let stats = read_stats(&conn)?;
    Ok(assess(
        &stats,
        policy,
        db_path.parent().and_then(available_disk_space),
        available_disk_space(&std::env::temp_dir()),
    ))
}

/// Compacts the archive and switches it to `auto_vacuum = INCREMENTAL`.
///
/// `volume_free` answers "free bytes on the volume holding this path"; it is a
/// parameter so tests can simulate an unknown or a full disk.
pub fn compact_archive(
    db_path: &Path,
    policy: &Policy,
    volume_free: &dyn Fn(&Path) -> Option<u64>,
) -> Result<Outcome, String> {
    let conn = open_archive_connection(db_path)?;
    let stats = read_stats(&conn)?;
    let archive_dir = db_path.parent().unwrap_or(Path::new("."));
    if let Err(reason) = assess(
        &stats,
        policy,
        volume_free(archive_dir),
        volume_free(&std::env::temp_dir()),
    ) {
        return Ok(Outcome::Skipped(reason));
    }

    // Exclusivity: a short wait instead of the 15 s every other connection gets.
    conn.busy_timeout(policy.exclusivity_wait)
        .map_err(|e| format!("Failed to set busy_timeout: {e}"))?;
    match checkpoint_truncate(&conn) {
        Ok(true) => {}
        Ok(false) => return Ok(Outcome::Skipped(SkipReason::InUse)),
        Err(error) if is_busy(&error) => return Ok(Outcome::Skipped(SkipReason::InUse)),
        Err(error) => return Err(format!("Failed to checkpoint before compaction: {error}")),
    }

    let before_bytes = file_bytes(db_path);
    let started = Instant::now();
    // `auto_vacuum` only changes through a full VACUUM, so it is set right
    // before it. Nothing is persisted until the VACUUM commits.
    conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL")
        .map_err(|e| format!("Failed to set auto_vacuum: {e}"))?;
    match conn.execute_batch("VACUUM") {
        Ok(()) => {}
        Err(error) if is_busy(&error) => return Ok(Outcome::Skipped(SkipReason::InUse)),
        Err(error) => return Err(format!("VACUUM failed: {error}")),
    }
    // Hand the WAL's pages back and shrink the file; the VACUUM is already
    // durable, so a failure here only leaves a bigger WAL for the next open.
    if let Err(error) = checkpoint_truncate(&conn) {
        eprintln!("[compact] post-VACUUM checkpoint failed: {error}");
    }

    let mode: i64 = conn
        .query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
        .map_err(|e| format!("Failed to verify auto_vacuum: {e}"))?;
    if mode != 2 {
        return Err(format!(
            "VACUUM finished but auto_vacuum is {mode}, not INCREMENTAL"
        ));
    }
    let after_bytes = file_bytes(db_path);
    let elapsed = started.elapsed();
    record_compaction(&conn, before_bytes, after_bytes);
    Ok(Outcome::Compacted {
        before_bytes,
        after_bytes,
        elapsed,
    })
}

/// Best effort: a missing marker only costs a cheap re-assessment next close.
fn record_compaction(conn: &Connection, before_bytes: u64, after_bytes: u64) {
    let value = format!(
        "{{\"at\":{},\"before_bytes\":{before_bytes},\"after_bytes\":{after_bytes}}}",
        repository::now_ms()
    );
    let written = conn
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        )
        .and_then(|()| crate::settings::set_setting(conn, SETTING_KEY, &value));
    if let Err(error) = written {
        eprintln!("[compact] could not record the compaction: {error}");
    }
}

/// Pages one startup returns to the OS at most (1 GB at 4 KB pages).
pub const RECLAIM_MAX_PAGES: i64 = 262_144;
/// Pause between incremental steps so other writers get the lock.
pub const RECLAIM_PAUSE: Duration = Duration::from_millis(50);

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    fn stats(total_mb: u64, free_mb: u64, auto_vacuum: i64) -> ArchiveStats {
        ArchiveStats {
            page_size: 4096,
            page_count: total_mb * MB / 4096,
            freelist_count: free_mb * MB / 4096,
            auto_vacuum,
        }
    }

    const PLENTY: Option<u64> = Some(u64::MAX / 2);

    #[test]
    fn a_mostly_free_archive_with_room_is_worth_compacting() {
        let policy = Policy::default();
        assert_eq!(
            assess(&stats(12_000, 11_000, 0), &policy, PLENTY, PLENTY),
            Ok(())
        );
    }

    #[test]
    fn free_space_under_the_absolute_floor_is_not_worth_a_slow_close() {
        let policy = Policy::default();
        // 90% free but only 100 MB: below the 200 MB floor.
        let verdict = assess(&stats(110, 100, 0), &policy, PLENTY, PLENTY);
        assert!(matches!(verdict, Err(SkipReason::FreeBelowFloor { .. })));
    }

    #[test]
    fn free_space_under_the_ratio_is_not_worth_it_however_large() {
        let policy = Policy::default();
        // 2 GB free but of 10 GB: 20%, below 30%.
        let verdict = assess(&stats(10_000, 2_000, 0), &policy, PLENTY, PLENTY);
        assert!(matches!(verdict, Err(SkipReason::FreeBelowRatio { .. })));
        // Exactly at 30% is still "not more than".
        let verdict = assess(&stats(10_000, 3_000, 0), &policy, PLENTY, PLENTY);
        assert!(matches!(verdict, Err(SkipReason::FreeBelowRatio { .. })));
    }

    #[test]
    fn unknown_disk_space_skips_instead_of_guessing() {
        let policy = Policy::default();
        let s = stats(12_000, 11_000, 0);
        assert_eq!(
            assess(&s, &policy, None, PLENTY),
            Err(SkipReason::DiskSpaceUnknown)
        );
        assert_eq!(
            assess(&s, &policy, PLENTY, None),
            Err(SkipReason::DiskSpaceUnknown)
        );
    }

    #[test]
    fn a_volume_without_the_room_a_vacuum_needs_skips() {
        let policy = Policy::default();
        let s = stats(12_000, 11_000, 0);
        // The archive volume must hold at least the file's size plus the margin.
        let needed = 12_000 * MB + policy.disk_margin_bytes;
        let verdict = assess(&s, &policy, Some(needed - 1), PLENTY);
        assert!(matches!(verdict, Err(SkipReason::NotEnoughDisk { .. })));
        assert_eq!(assess(&s, &policy, Some(needed), PLENTY), Ok(()));
        // The temp volume holds the rebuilt copy: the live data plus the margin.
        let temp_needed = 1_000 * MB + policy.disk_margin_bytes;
        let verdict = assess(&s, &policy, PLENTY, Some(temp_needed - 1));
        assert!(matches!(verdict, Err(SkipReason::NotEnoughDisk { .. })));
    }

    #[test]
    fn a_mostly_live_archive_needs_room_for_the_wal_copy_too() {
        let policy = Policy::default();
        // 69% live: the WAL copy of the live data (1.38x the file) is the bound.
        let s = stats(1_000, 310, 0);
        let needed = 1_380 * MB + policy.disk_margin_bytes;
        assert!(matches!(
            assess(&s, &policy, Some(needed - MB), PLENTY),
            Err(SkipReason::NotEnoughDisk { .. })
        ));
        assert_eq!(assess(&s, &policy, Some(needed), PLENTY), Ok(()));
    }

    #[test]
    fn an_archive_that_already_reclaims_by_itself_is_left_alone() {
        let policy = Policy::default();
        for mode in [1, 2] {
            assert_eq!(
                assess(&stats(12_000, 11_000, mode), &policy, PLENTY, PLENTY),
                Err(SkipReason::NotNeeded(mode))
            );
        }
    }

    // ---- real archives -------------------------------------------------

    /// Thresholds a test archive of a few MB can meet.
    fn small_policy() -> Policy {
        Policy {
            min_free_ratio: 0.30,
            min_free_bytes: 256 * 1024,
            disk_margin_bytes: 0,
            exclusivity_wait: Duration::from_millis(150),
        }
    }

    fn plenty(_: &Path) -> Option<u64> {
        Some(u64::MAX / 2)
    }

    /// An archive in the shape the app creates (auto_vacuum NONE, WAL) holding
    /// `items` (TEXT primary key, so rowids are implicit) with a contentless FTS
    /// table keyed by `items.rowid`, plus a blob table that gets emptied.
    fn build_archive() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("entropia.sqlite");
        let conn = open_archive_connection(&path).expect("open");
        conn.execute_batch(
            "CREATE TABLE items (id TEXT PRIMARY KEY, title TEXT NOT NULL);
             CREATE VIRTUAL TABLE fts_items USING fts5(title, content='');
             CREATE TABLE blobs (id INTEGER PRIMARY KEY, payload BLOB);
             CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .expect("schema");
        for index in 0..40 {
            conn.execute(
                "INSERT INTO items (id, title) VALUES (?1, ?2)",
                rusqlite::params![format!("item-{index}"), format!("titulo {index} archivo")],
            )
            .unwrap();
        }
        // Make rowids sparse so a renumbering would be visible.
        conn.execute("DELETE FROM items WHERE rowid % 3 = 0", [])
            .unwrap();
        conn.execute(
            "INSERT INTO fts_items(rowid, title) SELECT rowid, title FROM items",
            [],
        )
        .unwrap();
        for _ in 0..128 {
            conn.execute("INSERT INTO blobs (payload) VALUES (zeroblob(65536))", [])
                .unwrap();
        }
        conn.execute("DELETE FROM blobs", []).unwrap();
        (dir, path)
    }

    fn auto_vacuum_of(path: &Path) -> i64 {
        let conn = open_archive_connection(path).expect("open");
        conn.query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
            .unwrap()
    }

    fn reclaim(path: &Path, max_pages: i64) -> Option<i64> {
        let conn = open_archive_connection(path).expect("open");
        repository::reclaim_free_pages(&conn, max_pages, Duration::ZERO).expect("reclaim")
    }

    fn on_disk_size(path: &Path) -> u64 {
        let conn = open_archive_connection(path).expect("open");
        let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        std::fs::metadata(path).unwrap().len()
    }

    fn item_rowids(path: &Path) -> Vec<(i64, String)> {
        let conn = open_archive_connection(path).expect("open");
        let mut stmt = conn
            .prepare("SELECT rowid, id FROM items ORDER BY rowid")
            .unwrap();
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows
    }

    #[test]
    fn compacting_at_close_shrinks_the_file_and_switches_to_incremental() {
        let (_dir, path) = build_archive();
        assert_eq!(auto_vacuum_of(&path), 0, "the archive starts as NONE");
        let before = on_disk_size(&path);
        assert!(before > 4 * MB, "the fixture really holds free pages");

        let outcome = compact_archive(&path, &small_policy(), &plenty).expect("compaction runs");

        let Outcome::Compacted {
            before_bytes,
            after_bytes,
            ..
        } = outcome
        else {
            panic!("expected a compaction, got {outcome:?}");
        };
        assert!(
            after_bytes < before_bytes / 4,
            "{before_bytes} -> {after_bytes}"
        );
        assert_eq!(auto_vacuum_of(&path), 2, "the switch took effect");
        assert!(on_disk_size(&path) < before / 4);
        let conn = open_archive_connection(&path).unwrap();
        let marker: String = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [SETTING_KEY],
                |row| row.get(0),
            )
            .expect("the compaction is recorded");
        assert!(marker.contains("after_bytes"));
    }

    #[test]
    fn compaction_keeps_every_rowid_the_fts_index_points_at() {
        let (_dir, path) = build_archive();
        let before = item_rowids(&path);
        assert!(before.len() > 10);

        compact_archive(&path, &small_policy(), &plenty).expect("compaction runs");

        assert_eq!(item_rowids(&path), before, "VACUUM renumbered items");
        // The contentless FTS still resolves to the same items.
        let conn = open_archive_connection(&path).unwrap();
        let joined: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM fts_items f JOIN items i ON i.rowid = f.rowid
                 WHERE fts_items MATCH 'archivo'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(joined as usize, before.len());
    }

    #[test]
    fn a_compacted_archive_is_not_compacted_again() {
        let (_dir, path) = build_archive();
        compact_archive(&path, &small_policy(), &plenty).unwrap();
        let again = compact_archive(&path, &small_policy(), &plenty).unwrap();
        assert_eq!(again, Outcome::Skipped(SkipReason::NotNeeded(2)));
    }

    #[test]
    fn unknown_disk_space_leaves_the_archive_untouched() {
        let (_dir, path) = build_archive();
        let before = on_disk_size(&path);
        let outcome = compact_archive(&path, &small_policy(), &|_| None).unwrap();
        assert_eq!(outcome, Outcome::Skipped(SkipReason::DiskSpaceUnknown));
        assert_eq!(auto_vacuum_of(&path), 0);
        assert_eq!(on_disk_size(&path), before);
    }

    #[test]
    fn it_skips_while_another_connection_holds_the_write_lock() {
        let (_dir, path) = build_archive();
        let before = on_disk_size(&path);
        // A writer that is mid-transaction, as a background worker would be.
        let holder = open_archive_connection(&path).unwrap();
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        holder
            .execute("INSERT INTO app_settings VALUES ('x', 'y')", [])
            .unwrap();

        let started = Instant::now();
        let outcome = compact_archive(&path, &small_policy(), &plenty).expect("never an error");
        let waited = started.elapsed();

        assert_eq!(outcome, Outcome::Skipped(SkipReason::InUse));
        assert!(
            waited < Duration::from_secs(5),
            "bounded wait, got {waited:?}"
        );
        holder.execute_batch("ROLLBACK").unwrap();
        assert_eq!(auto_vacuum_of(&path), 0, "nothing changed");
        assert_eq!(on_disk_size(&path), before);
    }

    #[test]
    fn it_skips_while_a_reader_still_holds_an_old_snapshot() {
        let (_dir, path) = build_archive();
        let reader = open_archive_connection(&path).unwrap();
        reader.execute_batch("BEGIN").unwrap();
        let _: i64 = reader
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .unwrap();
        // A write after the reader's snapshot: the WAL can no longer be fully
        // checkpointed while that reader stays open.
        let writer = open_archive_connection(&path).unwrap();
        writer
            .execute("INSERT INTO app_settings VALUES ('later', 'write')", [])
            .unwrap();

        let outcome = compact_archive(&path, &small_policy(), &plenty).expect("never an error");

        assert_eq!(outcome, Outcome::Skipped(SkipReason::InUse));
        assert_eq!(auto_vacuum_of(&path), 0);
        drop(reader);
    }

    #[test]
    fn the_worthwhile_probe_is_read_only_and_follows_the_same_rules() {
        let (_dir, path) = build_archive();
        let before = on_disk_size(&path);
        // Production thresholds: a few-MB fixture is under the 200 MB floor.
        let verdict = is_worthwhile(&path, &Policy::default()).unwrap();
        assert!(matches!(verdict, Err(SkipReason::FreeBelowFloor { .. })));
        let verdict = is_worthwhile(&path, &small_policy()).unwrap();
        assert_eq!(verdict, Ok(()));
        assert_eq!(auto_vacuum_of(&path), 0, "the probe changes nothing");
        assert_eq!(on_disk_size(&path), before);
    }

    #[test]
    fn after_compaction_free_pages_return_gradually_and_bounded() {
        let (_dir, path) = build_archive();
        compact_archive(&path, &small_policy(), &plenty).unwrap();
        // Free pages appear again (a big delete after the compaction).
        {
            let conn = open_archive_connection(&path).unwrap();
            for _ in 0..256 {
                conn.execute("INSERT INTO blobs (payload) VALUES (zeroblob(65536))", [])
                    .unwrap();
            }
            conn.execute("DELETE FROM blobs", []).unwrap();
        }
        let conn = open_archive_connection(&path).unwrap();
        let free_before = read_stats(&conn).unwrap().freelist_count;
        assert!(free_before > 1_000);
        drop(conn);

        let released = reclaim(&path, 100).expect("the archive is INCREMENTAL now");
        assert_eq!(released, 100, "bounded by max_pages");
        let conn = open_archive_connection(&path).unwrap();
        assert_eq!(read_stats(&conn).unwrap().freelist_count, free_before - 100);
        drop(conn);

        let rest = reclaim(&path, i64::MAX).unwrap();
        assert_eq!(rest as u64, free_before - 100);
    }

    #[test]
    fn gradual_reclamation_waits_for_the_compaction() {
        let (_dir, path) = build_archive();
        assert_eq!(
            reclaim(&path, 100),
            None,
            "an auto_vacuum NONE archive is never vacuumed by the startup hook"
        );
    }
}
