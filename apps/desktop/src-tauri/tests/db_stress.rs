//! Stress test for the archive database Pro and Lite share
//! (`%APPDATA%\com.entropia.shared\entropia.sqlite`).
//!
//! Ignored by default: it seeds tens of thousands of rows and runs for about
//! two minutes. Run it on demand:
//!
//!   cargo test --profile measure --test db_stress -- --ignored --nocapture --test-threads 1
//!
//! Every scenario uses a throwaway temp database built from the real schema
//! fixture — never the user's archive. Each one prints a `[stress]` line with
//! its numbers and asserts only what must never happen (lock errors,
//! corruption, lost commits); latency targets are printed, not asserted, so a
//! slow machine reports instead of failing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, TransactionBehavior};

const SCHEMA_FIXTURE: &str = include_str!("fixtures/schema_full.sql");
const CHILD_ENV: &str = "DB_STRESS_CHILD";
const CHILD_DB_ENV: &str = "DB_STRESS_DB";

const ITEMS: usize = 10_000;
const ASSETS_PER_ITEM: usize = 5;
const CONCURRENT_SECS: u64 = 60;
const TWO_PROCESS_SECS: u64 = 30;
const CRASH_ROUNDS: usize = 20;
/// Pause between worker writes. Real OCR/NLP spend seconds per unit; 10 ms is
/// still ~100 writes/s per worker, far above anything the app produces.
/// `DB_STRESS_PAUSE_MS` overrides it to try a realistic pace.
fn worker_pause() -> Duration {
    let ms = std::env::var("DB_STRESS_PAUSE_MS")
        .ok()
        .and_then(|v| v.parse().ok());
    Duration::from_millis(ms.unwrap_or(10))
}

fn open(path: &Path) -> Connection {
    entropia_desktop_lib::db_open_for_tests(path)
}

fn fresh_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("entropia.sqlite");
    open(&path)
        .execute_batch(SCHEMA_FIXTURE)
        .expect("apply schema fixture");
    (dir, path)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

const WORDS: &[&str] = &[
    "expediente",
    "juzgado",
    "sentencia",
    "acta",
    "legajo",
    "carta",
    "archivo",
    "provincia",
    "municipio",
    "decreto",
    "censo",
    "parroquia",
    "bautismo",
    "matrimonio",
    "defunción",
    "notaría",
    "escritura",
    "testamento",
    "puerto",
    "aduana",
    "ferrocarril",
    "colonia",
    "estancia",
    "cabildo",
];

/// Deterministic pseudo-text so FTS has something realistic to index.
fn text(seed: usize, words: usize) -> String {
    (0..words)
        .map(|i| WORDS[(seed.wrapping_mul(31).wrapping_add(i * 7)) % WORDS.len()])
        .collect::<Vec<_>>()
        .join(" ")
}

/// Seeds `items` items with `ASSETS_PER_ITEM` assets each, one extraction per
/// asset, the FTS row per item and one pending OCR task per asset.
fn seed(conn: &Connection, items: usize) {
    let tx = conn.unchecked_transaction().expect("seed tx");
    tx.execute(
        "INSERT INTO collections (id, name, created_at, updated_at) VALUES ('c1', 'Fondo', 1, 1)",
        [],
    )
    .unwrap();
    {
        let mut item = tx
            .prepare("INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at) VALUES (?1, ?2, 'c1', ?3, ?4, ?4)")
            .unwrap();
        let mut asset = tx
            .prepare("INSERT INTO assets (id, item_id, path, type, size, sort_index, created_at) VALUES (?1, ?2, ?3, 'image', 1000, ?4, ?5)")
            .unwrap();
        let mut extraction = tx
            .prepare("INSERT INTO extractions (id, asset_id, text_content, method, created_at) VALUES (?1, ?2, ?3, 'ocr', ?4)")
            .unwrap();
        let mut fts = tx
            .prepare("INSERT INTO fts_items (rowid, item_id, title, metadata, extracted_text) SELECT rowid, id, title, metadata, ?2 FROM items WHERE id = ?1")
            .unwrap();
        let mut task = tx
            .prepare("INSERT INTO processing_tasks (id, kind, asset_id_snapshot, state, created_at, updated_at) VALUES (?1, 'ocr', ?2, 'pending', 1, 1)")
            .unwrap();
        for i in 0..items {
            let item_id = format!("i{i}");
            item.execute(params![
                item_id,
                format!("Documento {i} {}", text(i, 3)),
                format!("{{\"fecha\":\"19{:02}\"}}", i % 100),
                i as i64
            ])
            .unwrap();
            let mut item_text = String::new();
            for a in 0..ASSETS_PER_ITEM {
                let asset_id = format!("a{i}_{a}");
                let page = text(i + a, 120);
                asset
                    .execute(params![
                        asset_id,
                        item_id,
                        format!("{asset_id}.png"),
                        a as i64,
                        i as i64
                    ])
                    .unwrap();
                extraction
                    .execute(params![format!("e{i}_{a}"), asset_id, page, i as i64])
                    .unwrap();
                task.execute(params![format!("ocr-{asset_id}"), asset_id])
                    .unwrap();
                item_text.push_str(&page);
                item_text.push(' ');
            }
            fts.execute(params![item_id, item_text]).unwrap();
        }
    }
    tx.commit().expect("seed commit");
}

#[derive(Default)]
struct Latencies(Vec<Duration>);

impl Latencies {
    fn push(&mut self, d: Duration) {
        self.0.push(d);
    }
    fn summary(&mut self) -> String {
        if self.0.is_empty() {
            return "n=0".into();
        }
        self.0.sort();
        let pct = |p: f64| self.0[((self.0.len() - 1) as f64 * p) as usize].as_millis();
        format!(
            "n={} p50={}ms p95={}ms max={}ms",
            self.0.len(),
            pct(0.50),
            pct(0.95),
            self.0.last().unwrap().as_millis()
        )
    }
    fn p95(&mut self) -> Duration {
        self.0.sort();
        self.0
            .get(((self.0.len().max(1) - 1) as f64 * 0.95) as usize)
            .copied()
            .unwrap_or_default()
    }
}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy) | Some(rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// Claims one pending OCR task like `repository::claim_next` does:
/// BEGIN IMMEDIATE, pick, mark running, commit. Then "finishes" it.
fn worker_claim_and_finish(conn: &Connection, session: &str) -> rusqlite::Result<bool> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let picked: Option<String> = conn
        .query_row(
            "SELECT id FROM processing_tasks WHERE state = 'pending' ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok();
    let Some(id) = picked else {
        conn.execute_batch("COMMIT")?;
        return Ok(false);
    };
    conn.execute(
        "UPDATE processing_tasks SET state = 'running', owner_session = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, session, now_ms()],
    )?;
    conn.execute_batch("COMMIT")?;
    conn.execute(
        "UPDATE processing_tasks SET state = 'succeeded', updated_at = ?2 WHERE id = ?1",
        params![id, now_ms()],
    )?;
    Ok(true)
}

/// Mirrors `db_execute_transaction` (db/commands.rs): an IMMEDIATE transaction
/// that reads first and then writes.
/// `DB_STRESS_UI_TX=deferred` brings back the old DEFERRED shape, which fails
/// instantly with `database is locked` while a worker is mid-write.
fn ui_deferred_edit(conn: &mut Connection, n: u64) -> rusqlite::Result<()> {
    let behavior = match std::env::var("DB_STRESS_UI_TX").as_deref() {
        Ok("deferred") => TransactionBehavior::Deferred,
        _ => TransactionBehavior::Immediate,
    };
    let tx = conn.transaction_with_behavior(behavior)?;
    // Every scenario seeds at least 1 000 items.
    let item = format!("i{}", n as usize % 1_000);
    let _: i64 = tx.query_row(
        "SELECT COUNT(*) FROM notes WHERE item_id = ?1",
        [&item],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO notes (id, item_id, content, created_at, updated_at) VALUES (?1, ?2, 'nota', ?3, ?3)",
        params![format!("ui-note-{n}"), item, now_ms()],
    )?;
    tx.execute(
        "UPDATE items SET title = title, updated_at = ?2 WHERE id = ?1",
        params![item, now_ms()],
    )?;
    tx.commit()
}

fn integrity_ok(conn: &Connection) -> String {
    conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap()
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Volume: interactive reads on a large archive
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "stress: run on demand, see module docs"]
fn stress_1_volume_reads_stay_interactive() {
    let (_dir, path) = fresh_db();
    let conn = open(&path);
    let started = Instant::now();
    seed(&conn, ITEMS);
    let seeded = started.elapsed();
    let size_mb = std::fs::metadata(&path).unwrap().len() as f64 / 1_048_576.0;

    let mut list = Latencies::default();
    let mut search = Latencies::default();
    let mut detail = Latencies::default();
    for i in 0..50 {
        let t = Instant::now();
        let mut stmt = conn
            .prepare_cached("SELECT id, title, updated_at FROM items WHERE collection_id = 'c1' ORDER BY updated_at DESC LIMIT 50 OFFSET ?1")
            .unwrap();
        let rows: Vec<String> = stmt
            .query_map([i * 50], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows.len(), 50);
        list.push(t.elapsed());

        let t = Instant::now();
        let term = WORDS[i as usize % WORDS.len()];
        let mut stmt = conn
            .prepare_cached(
                "SELECT item_id FROM fts_items WHERE fts_items MATCH ?1 ORDER BY rank LIMIT 50",
            )
            .unwrap();
        let hits = stmt
            .query_map([term], |r| r.get::<_, String>(0))
            .unwrap()
            .count();
        assert!(hits > 0, "FTS must find '{term}'");
        search.push(t.elapsed());

        let t = Instant::now();
        let item = format!("i{}", (i * 197) % ITEMS as i64);
        let mut stmt = conn
            .prepare_cached("SELECT a.id, e.text_content FROM assets a LEFT JOIN extractions e ON e.asset_id = a.id WHERE a.item_id = ?1 ORDER BY a.sort_index")
            .unwrap();
        let pages = stmt
            .query_map([item], |r| r.get::<_, String>(0))
            .unwrap()
            .count();
        assert_eq!(pages, ASSETS_PER_ITEM);
        detail.push(t.elapsed());
    }

    println!(
        "[stress 1 volume] {ITEMS} items / {} assets seeded in {:.1}s, file {size_mb:.0} MB",
        ITEMS * ASSETS_PER_ITEM,
        seeded.as_secs_f64()
    );
    println!(
        "[stress 1 volume] list page:   {}  (target p95 < 200ms)",
        list.summary()
    );
    println!(
        "[stress 1 volume] fts search:  {}  (target p95 < 200ms)",
        search.summary()
    );
    println!(
        "[stress 1 volume] item detail: {}  (target p95 < 200ms)",
        detail.summary()
    );
    assert_eq!(integrity_ok(&conn), "ok");
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. UI + background workers in one process
// ─────────────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "stress: run on demand, see module docs"]
fn stress_2_ui_and_workers_share_the_archive() {
    let (_dir, path) = fresh_db();
    seed(&open(&path), ITEMS);

    let stop = Arc::new(AtomicBool::new(false));
    let busy = Arc::new(AtomicU64::new(0));
    let other_errors = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();
    // Longest single write transaction any worker held, to tell a long lock
    // hold from a long queue. `DB_STRESS_SKIP=queue,text,bulk` drops a worker
    // kind to see which one slows the UI.
    let max_hold_ms = Arc::new(AtomicU64::new(0));
    let skip = std::env::var("DB_STRESS_SKIP").unwrap_or_default();
    let count = |kind: &str, n: usize| if skip.contains(kind) { 0 } else { n };

    // 3 queue workers (OCR / embedding shape).
    for w in 0..count("queue", 3) {
        let (path, stop, busy, other) = (
            path.clone(),
            stop.clone(),
            busy.clone(),
            other_errors.clone(),
        );
        handles.push(std::thread::spawn(move || {
            let conn = open(&path);
            let mut done = 0u64;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(worker_pause());
                match worker_claim_and_finish(&conn, &format!("w{w}")) {
                    Ok(_) => done += 1,
                    Err(e) => {
                        let _ = conn.execute_batch("ROLLBACK");
                        if is_busy(&e) {
                            busy.fetch_add(1, Ordering::Relaxed)
                        } else {
                            other.fetch_add(1, Ordering::Relaxed)
                        };
                    }
                }
            }
            ("queue", done)
        }));
    }
    // 2 text writers (NLP / FTS reindex shape): rewrite an extraction + its FTS row.
    for w in 0..count("text", 2) {
        let (path, stop, busy, other, max_hold) = (
            path.clone(),
            stop.clone(),
            busy.clone(),
            other_errors.clone(),
            max_hold_ms.clone(),
        );
        handles.push(std::thread::spawn(move || {
            let mut conn = open(&path);
            let mut n = w as usize;
            let mut done = 0u64;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(worker_pause());
                let item = format!("i{}", n % ITEMS);
                let result = (|| -> rusqlite::Result<()> {
                    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    let held = Instant::now();
                    tx.execute(
                        "UPDATE extractions SET text_content = ?2 WHERE id = ?1",
                        params![format!("e{}_0", n % ITEMS), text(n, 120)],
                    )?;
                    tx.execute("DELETE FROM fts_items WHERE item_id = ?1", [&item])?;
                    tx.execute(
                        "INSERT INTO fts_items (rowid, item_id, title, metadata, extracted_text) SELECT rowid, id, title, metadata, ?2 FROM items WHERE id = ?1",
                        params![item, text(n, 600)],
                    )?;
                    tx.commit()?;
                    max_hold.fetch_max(held.elapsed().as_millis() as u64, Ordering::Relaxed);
                    Ok(())
                })();
                match result {
                    Ok(()) => done += 1,
                    Err(e) => if is_busy(&e) { busy.fetch_add(1, Ordering::Relaxed); } else { other.fetch_add(1, Ordering::Relaxed); },
                }
                n += 2;
            }
            ("text", done)
        }));
    }
    // 1 bulk reader (sync / export shape): long read transactions.
    if count("bulk", 1) == 1 {
        let (path, stop) = (path.clone(), stop.clone());
        handles.push(std::thread::spawn(move || {
            let conn = open(&path);
            let mut done = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let n: i64 = conn
                    .query_row("SELECT COUNT(*) FROM (SELECT e.text_content FROM extractions e JOIN assets a ON a.id = e.asset_id)", [], |r| r.get(0))
                    .unwrap();
                assert!(n > 0);
                done += 1;
            }
            ("bulk-read", done)
        }));
    }

    // The UI: alternates the deferred multi-statement transaction with single
    // writes, at a human-ish pace.
    let mut ui = open(&path);
    let mut ui_lat = Latencies::default();
    let (mut ui_tx_lat, mut ui_single_lat) = (Latencies::default(), Latencies::default());
    let wal = path.with_extension("sqlite-wal");
    let mut wal_max = 0u64;
    let (mut ui_busy, mut ui_other) = (0u64, Vec::<String>::new());
    let deadline = Instant::now() + Duration::from_secs(CONCURRENT_SECS);
    let mut n = 0u64;
    while Instant::now() < deadline {
        let t = Instant::now();
        let result = if n % 2 == 0 {
            ui_deferred_edit(&mut ui, n)
        } else {
            ui.execute(
                "UPDATE items SET metadata = ?2, updated_at = ?3 WHERE id = ?1",
                params![
                    format!("i{}", n as usize % ITEMS),
                    format!("{{\"v\":{n}}}"),
                    now_ms()
                ],
            )
            .map(|_| ())
        };
        match result {
            Ok(()) => {
                ui_lat.push(t.elapsed());
                if n % 2 == 0 {
                    ui_tx_lat.push(t.elapsed())
                } else {
                    ui_single_lat.push(t.elapsed())
                }
            }
            Err(e) if is_busy(&e) => {
                ui_busy += 1;
                println!(
                    "[stress 2 concurrent] UI busy after {}ms: {e:?}",
                    t.elapsed().as_millis()
                );
            }
            Err(e) => ui_other.push(e.to_string()),
        }
        n += 1;
        wal_max = wal_max.max(std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0));
        std::thread::sleep(Duration::from_millis(20));
    }
    stop.store(true, Ordering::Relaxed);
    let mut summary = Vec::new();
    for h in handles {
        let (kind, done) = h.join().expect("worker panicked");
        summary.push(format!("{kind}={done}"));
    }

    let conn = open(&path);
    println!(
        "[stress 2 concurrent] {CONCURRENT_SECS}s, workers done: {}",
        summary.join(" ")
    );
    println!(
        "[stress 2 concurrent] UI writes: {}  busy={ui_busy} other={}  (target p95 < 250ms, busy = 0)",
        ui_lat.summary(),
        ui_other.len()
    );
    println!(
        "[stress 2 concurrent] UI tx: {}  UI single: {}",
        ui_tx_lat.summary(),
        ui_single_lat.summary()
    );
    println!(
        "[stress 2 concurrent] skip='{skip}' longest text-writer lock hold={}ms, WAL peak {:.0} MB",
        max_hold_ms.load(Ordering::Relaxed),
        wal_max as f64 / 1_048_576.0
    );
    println!(
        "[stress 2 concurrent] worker busy errors={} other errors={}",
        busy.load(Ordering::Relaxed),
        other_errors.load(Ordering::Relaxed)
    );
    if ui_lat.p95() > Duration::from_millis(250) {
        println!("[stress 2 concurrent] WARNING: UI p95 above target");
    }
    assert!(ui_other.is_empty(), "UI hit non-busy errors: {ui_other:?}");
    assert_eq!(ui_busy, 0, "UI writes failed with database-busy");
    assert_eq!(
        busy.load(Ordering::Relaxed),
        0,
        "workers failed with database-busy"
    );
    assert_eq!(
        other_errors.load(Ordering::Relaxed),
        0,
        "workers hit other errors"
    );
    assert_eq!(integrity_ok(&conn), "ok");
}

// ─────────────────────────────────────────────────────────────────────────────
// 3 + 4. Two processes on one archive, and a process killed mid-write
// ─────────────────────────────────────────────────────────────────────────────

/// Child mode. `write:<secs>` writes notes for that long and exits 0 on a
/// clean run (1 on any error); `crash` loops open write transactions until
/// killed.
fn child_main(mode: &str, path: &Path) -> ! {
    let mut conn = open(path);
    let pid = std::process::id();
    if let Some(secs) = mode.strip_prefix("write:") {
        let deadline = Instant::now() + Duration::from_secs(secs.parse().unwrap());
        let mut n = 0u64;
        while Instant::now() < deadline {
            if let Err(e) = ui_deferred_edit(&mut conn, n + 1_000_000 * pid as u64) {
                eprintln!("child write failed: {e}");
                std::process::exit(1);
            }
            n += 1;
        }
        println!("{n}");
        std::process::exit(0);
    }
    // crash: big transactions so the kill lands inside one most of the time.
    let mut n = 0u64;
    loop {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        for _ in 0..200 {
            tx.execute(
                "INSERT INTO notes (id, item_id, content, created_at, updated_at) VALUES (?1, 'i1', ?2, 1, 1)",
                params![format!("crash-{pid}-{n}"), text(n as usize, 200)],
            )
            .unwrap();
            n += 1;
        }
        tx.commit().unwrap();
    }
}

fn spawn_child(mode: &str, path: &Path, test_name: &str) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            test_name,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(CHILD_ENV, mode)
        .env(CHILD_DB_ENV, path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn child")
}

fn maybe_child() {
    if let Ok(mode) = std::env::var(CHILD_ENV) {
        child_main(&mode, Path::new(&std::env::var(CHILD_DB_ENV).unwrap()));
    }
}

#[test]
#[ignore = "stress: run on demand, see module docs"]
fn stress_3_two_processes_write_the_same_archive() {
    maybe_child();
    let (_dir, path) = fresh_db();
    seed(&open(&path), 1_000);

    let children: Vec<_> = (0..2)
        .map(|_| {
            spawn_child(
                &format!("write:{TWO_PROCESS_SECS}"),
                &path,
                "stress_3_two_processes_write_the_same_archive",
            )
        })
        .collect();
    let mut total = 0u64;
    for child in children {
        let out = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "child failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        // libtest prints "test <name> ... " on the same line; the count is the last word.
        let n: u64 = stdout
            .lines()
            .find_map(|l| l.split_whitespace().last()?.parse().ok())
            .expect("child count");
        total += n;
    }
    let conn = open(&path);
    let notes: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM notes WHERE id LIKE 'ui-note-%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!(
        "[stress 3 two-process] {TWO_PROCESS_SECS}s, 2 processes committed {total} transactions, {notes} notes on disk, {:.0} tx/s",
        total as f64 / TWO_PROCESS_SECS as f64
    );
    assert_eq!(notes, total, "every committed transaction must be on disk");
    assert_eq!(integrity_ok(&conn), "ok");
}

#[test]
#[ignore = "stress: run on demand, see module docs"]
fn stress_4_killed_mid_write_leaves_a_sound_archive() {
    maybe_child();
    let (_dir, path) = fresh_db();
    seed(&open(&path), 1_000);

    let mut reopen = Latencies::default();
    for round in 0..CRASH_ROUNDS {
        let mut child = spawn_child(
            "crash",
            &path,
            "stress_4_killed_mid_write_leaves_a_sound_archive",
        );
        // Vary the kill point so it lands at different spots of the write.
        std::thread::sleep(Duration::from_millis(300 + (round as u64 * 37) % 400));
        child.kill().expect("kill child");
        child.wait().unwrap();

        let t = Instant::now();
        let conn = open(&path);
        let check = integrity_ok(&conn);
        reopen.push(t.elapsed());
        assert_eq!(check, "ok", "round {round}: archive damaged after kill");
        // Committed batches are whole: notes come in blocks of 200.
        let crash_notes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM notes WHERE id LIKE 'crash-%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            crash_notes % 200,
            0,
            "round {round}: a half-written transaction survived"
        );
    }
    let conn = open(&path);
    let crash_notes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM notes WHERE id LIKE 'crash-%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!(
        "[stress 4 crash] {CRASH_ROUNDS} kills mid-write, integrity ok every time, {crash_notes} notes kept, reopen+check {}",
        reopen.summary()
    );
}
