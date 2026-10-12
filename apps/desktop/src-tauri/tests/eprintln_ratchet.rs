//! Ratchet against new `eprintln!` calls in backend source (plan §5.4).
//!
//! The backend is migrating its diagnostics from `eprintln!` to the `log`
//! facade in `src/logging.rs`, one module at a time. This test freezes the
//! current population in `tests/fixtures/eprintln_baseline.txt` so a migration
//! unit can lower a file's count but no one can add a new raw `eprintln!`
//! without the guard noticing.
//!
//! Counting rule: raw occurrences of the token `eprintln!` in every `*.rs`
//! file under `src/`, counted as text. `#[cfg(test)]` modules are NOT skipped:
//! skipping them would require parsing Rust (braces live inside string
//! literals) and counting all is deterministic. Comments count too, so never
//! mention the token in a source comment unless the count is intended.
//!
//! The one occurrence left in `src/app_logs.rs` is deliberate and permanent:
//! it is the stderr echo of an entry the user's app log just persisted, so the
//! baseline is expected to settle at exactly `app_logs.rs 1` once every other
//! module routes its diagnostics through the facade. `println!` is never
//! counted: it is the test-harness channel (`--nocapture`), not a raw stderr
//! write.
//!
//! - A file above its baseline → failure (a new `eprintln!` was added).
//! - A file with `eprintln!` and no baseline entry → failure (new use).
//! - A file below its baseline → pass, with a hint to refresh the baseline.
//! - `regenerate_baseline` (ignored by default) rewrites the fixture.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Source files the ratchet covers; mirrors `apps/desktop/src-tauri/src`.
const SOURCE_DIR: &str = "src";
/// Checked-in per-file limits, `path count` per line (`#` lines are comments).
const BASELINE_FILE: &str = "tests/fixtures/eprintln_baseline.txt";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source_root() -> PathBuf {
    manifest_dir().join(SOURCE_DIR)
}

fn baseline_path() -> PathBuf {
    manifest_dir().join(BASELINE_FILE)
}

/// Collects every `*.rs` file under `dir` into `files`.
fn collect_rust_sources(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source directory") {
        let path = entry.expect("read source entry").path();
        if path.is_dir() {
            collect_rust_sources(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

/// `eprintln!` occurrences in `contents`, counted as raw text.
fn count_eprintln(contents: &str) -> usize {
    contents.matches("eprintln!").count()
}

/// Current `eprintln!` population: `src`-relative path → count, for files with
/// at least one occurrence. Paths use `/` separators on every platform.
fn current_counts() -> BTreeMap<String, usize> {
    let root = source_root();
    let mut files = Vec::new();
    collect_rust_sources(&root, &mut files);

    let mut counts = BTreeMap::new();
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .expect("source file lives under src/")
            .to_string_lossy()
            .replace('\\', "/");
        let contents = fs::read_to_string(&file).expect("read source file");
        let count = count_eprintln(&contents);
        if count > 0 {
            counts.insert(relative, count);
        }
    }
    counts
}

/// Parses the checked-in baseline. A missing file yields an empty map so the
/// test fails loudly with every current use listed.
fn read_baseline() -> BTreeMap<String, usize> {
    let contents = fs::read_to_string(baseline_path()).unwrap_or_default();
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (path, count) = line
                .rsplit_once(' ')
                .expect("baseline lines are `path count`");
            (
                path.to_string(),
                count.trim().parse().expect("baseline count is a number"),
            )
        })
        .collect()
}

#[test]
fn no_source_file_exceeds_the_eprintln_baseline() {
    let current = current_counts();
    let baseline = read_baseline();

    let mut violations = Vec::new();
    let mut lowered = Vec::new();

    for (path, count) in &current {
        match baseline.get(path) {
            None => violations.push(format!(
                "{path}: {count} occurrence(s), no baseline entry — a new file uses eprintln!"
            )),
            Some(limit) if count > limit => violations.push(format!(
                "{path}: {count} occurrence(s), baseline {limit} — new eprintln! call(s)"
            )),
            Some(limit) if count < limit => lowered.push(format!(
                "{path}: {count} occurrence(s), baseline {limit} — lowered, refresh the baseline"
            )),
            Some(_) => {}
        }
    }
    for path in baseline.keys() {
        if !current.contains_key(path) {
            lowered.push(format!(
                "{path}: no occurrences left — remove it from the baseline"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "the eprintln! ratchet was violated:\n{}\n\nIf the new eprintln! is intentional, migrate it to the log \
         facade (src/logging.rs) or update {BASELINE_FILE} deliberately.",
        violations.join("\n")
    );

    if !lowered.is_empty() {
        println!(
            "[eprintln-ratchet] counts moved below the baseline; run \
             `cargo test --test eprintln_ratchet -- --ignored` to refresh {BASELINE_FILE}:\n{}",
            lowered.join("\n")
        );
    }
}

/// Rewrites the fixture from the current tree. Ignored by default: refreshing
/// the baseline is a deliberate review decision, never a side effect of tests.
#[test]
#[ignore = "rewrites tests/fixtures/eprintln_baseline.txt from the current tree"]
fn regenerate_baseline() {
    let mut text = String::from(
        "# eprintln! occurrences per backend source file (plan §5.4 ratchet).\n\
         # Format: `path count`, path relative to src/, one line per file with occurrences.\n\
         # Regenerate deliberately with: cargo test --test eprintln_ratchet -- --ignored\n",
    );
    for (path, count) in current_counts() {
        text.push_str(&path);
        text.push(' ');
        text.push_str(&count.to_string());
        text.push('\n');
    }
    fs::write(baseline_path(), text).expect("write eprintln baseline");
}
