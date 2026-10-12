//! Ratchet against per-client `User-Agent` literals in backend source.
//!
//! Every outgoing HTTP client identifies itself with the shared
//! `crate::USER_AGENT` const, whose version comes from Cargo. The clients used
//! to carry their own `"EntropIA-Desktop/0.1 (historical-research-app)"`
//! literal, which drifted from the app version and would drift again for every
//! future bump. This test freezes that migration: any new `.user_agent("...")`
//! literal fails the build until it uses `crate::USER_AGENT` instead.
//!
//! The one deliberate exception is `src/sync/http.rs`, which sends
//! `"EntropIA-Desktop-Sync/1.0"`: that is a sync protocol identifier the
//! server may rely on, not a client version string.
//!
//! The check is textual, like `tests/eprintln_ratchet.rs`: every `*.rs` file
//! under `src/` is scanned line by line, and `#[cfg(test)]` modules and
//! comments count too, so never mention the forbidden literal in a comment.

use std::fs;
use std::path::{Path, PathBuf};

/// Source files the ratchet covers; mirrors `apps/desktop/src-tauri/src`.
const SOURCE_DIR: &str = "src";
/// The only file allowed to hardcode a `User-Agent`.
const SYNC_HTTP: &str = "sync/http.rs";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

#[test]
fn no_client_hardcodes_a_user_agent() {
    let root = manifest_dir().join(SOURCE_DIR);
    let mut files = Vec::new();
    collect_rust_sources(&root, &mut files);

    let mut violations = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .expect("source file lives under src/")
            .to_string_lossy()
            .replace('\\', "/");
        if relative == SYNC_HTTP {
            continue;
        }
        let contents = fs::read_to_string(&file).expect("read source file");
        for (index, line) in contents.lines().enumerate() {
            if line.contains(".user_agent(\"") {
                violations.push(format!(
                    "{relative}:{}: use crate::USER_AGENT instead of a hardcoded literal",
                    index + 1
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "the user-agent ratchet was violated:\n{}",
        violations.join("\n")
    );
}
