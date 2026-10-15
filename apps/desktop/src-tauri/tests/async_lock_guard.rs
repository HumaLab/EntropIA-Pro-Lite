//! Keeps the shared `ui_conn` off the async thread (A-05b, plan 3.4).
//!
//! Every command in these modules is an `async fn` running on Tokio's
//! runtime. Locking `AppDbState::ui_conn` there stalls a runtime worker — and,
//! since settings now resolve credential-store references after the SQL,
//! possibly behind a slow keyring — while blocking every renderer query. So an
//! `async fn` may touch `ui_conn` only inside a blocking closure
//! (`run_blocking_db_task` or `spawn_blocking`), and it may not drive the
//! settings/credential-store helpers (`get_setting`, `read_secret`, ...)
//! outside one either: those resolve secret references and must run on the
//! blocking pool, after the connection is released.
//!
//! Read as text on purpose: the guarantee is about where a call sits inside a
//! function body, which no type or runtime check observes. The scan mirrors
//! the A-05b audit heuristic — for each `async fn`, `ui_conn.lock()` must
//! appear only inside a blocking closure. Comments and string literals are
//! masked out first, so docs that mention `ui_conn.lock()` do not count.
//!
//! It failed on the pre-A-05b code (llm, transcription, ocr, deps, navegador,
//! nlp, writing, zotero) and passes once those commands delegate to the
//! blocking pool.

use std::path::PathBuf;

/// The command modules the A-05b audit named, plus the readers that run on
/// async code paths (`runtime/manager.rs`, `store_updates.rs`).
const SCANNED_FILES: [&str; 15] = [
    "src/settings.rs",
    "src/transcription/commands.rs",
    "src/llm/commands.rs",
    "src/ocr/commands.rs",
    "src/ocr/mod.rs",
    "src/nlp/commands.rs",
    "src/nlp/embeddings.rs",
    "src/deps/mod.rs",
    "src/deps/mod_lite.rs",
    "src/navegador/commands.rs",
    "src/writing/publish.rs",
    "src/zotero_web.rs",
    "src/runtime/manager.rs",
    "src/store_updates.rs",
    "src/db/commands.rs",
];

/// The closures that move work off the async thread.
const BLOCKING_CALLS: [&str; 2] = ["run_blocking_db_task", "spawn_blocking"];

/// Calls that read or write settings through the credential store: outside a
/// blocking closure they would resolve the keyring on the async thread.
const SETTINGS_CALLS: [&str; 6] = [
    "get_setting(",
    "resolve_api_key_input(",
    "read_secret(",
    "store_secret(",
    "delete_secret(",
    "persist_setting(",
];

fn read(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// One top-level async fn: its name, the byte offset where it starts, and its
/// body.
struct AsyncFn {
    name: String,
    offset: usize,
    body: String,
}

/// Top-level `async fn` bodies, from the declaration line to the first `}`
/// at column 0 (rustfmt keeps top-level closing braces there). Nested items
/// are indented, so they never terminate the body early.
fn async_fns(source: &str) -> Vec<AsyncFn> {
    let lines: Vec<&str> = source.lines().collect();
    let mut line_offsets = Vec::with_capacity(lines.len());
    let mut offset = 0;
    for line in &lines {
        line_offsets.push(offset);
        offset += line.len() + 1;
    }
    let mut functions = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let declaration = lines[index];
        let trimmed = declaration.trim_start();
        let is_async_fn = trimmed.starts_with("async fn ")
            || trimmed.starts_with("pub async fn ")
            || (trimmed.starts_with("pub(") && trimmed.contains(") async fn "));
        // Only top-level items: an indented method belongs to an impl block.
        if is_async_fn && declaration.len() == trimmed.len() {
            let name = trimmed
                .split("async fn ")
                .nth(1)
                .and_then(|rest| rest.split(['(', '<', ' ']).next())
                .unwrap_or("<unnamed>")
                .to_string();
            let mut body = String::new();
            let mut end = index;
            loop {
                body.push_str(lines[end]);
                body.push('\n');
                // A body that opens and closes on the declaration line (an
                // empty `{}`) ends there. Otherwise the body runs to the first
                // `}` at column 0, which rustfmt keeps for top-level items.
                if end == index
                    && declaration.contains('{')
                    && declaration.trim_end().ends_with('}')
                {
                    break;
                }
                end += 1;
                if end >= lines.len() || lines[end] == "}" {
                    body.push_str("}\n");
                    break;
                }
            }
            functions.push(AsyncFn {
                name,
                offset: line_offsets[index],
                body,
            });
            index = end + 1;
            continue;
        }
        index += 1;
    }
    functions
}

/// Replaces string literals, char literals, and comments with spaces so the
/// scan only sees code. Byte offsets are preserved, so a match still reports
/// the original line.
fn mask(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let blank = |out: &mut Vec<u8>, at: usize| {
        if out[at] != b'\n' {
            out[at] = b' ';
        }
    };
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                blank(&mut out, i);
                i += 1;
            }
        } else if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let mut depth = 0;
            while i < bytes.len() {
                if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    blank(&mut out, i);
                    blank(&mut out, i + 1);
                    i += 2;
                } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    blank(&mut out, i);
                    blank(&mut out, i + 1);
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    blank(&mut out, i);
                    i += 1;
                }
            }
        } else if bytes[i] == b'r' || (bytes[i] == b'b' && bytes.get(i + 1) == Some(&b'r')) {
            // Raw string (`r"…"`, `r#"…"#`, `br#"…"#`): skip the prefix hash run.
            let raw_at = if bytes[i] == b'b' { i + 1 } else { i };
            let mut j = raw_at + 1;
            let mut hashes = 0;
            while bytes.get(j) == Some(&b'#') {
                hashes += 1;
                j += 1;
            }
            if bytes.get(j) == Some(&b'"') {
                for at in i..=j {
                    blank(&mut out, at);
                }
                j += 1;
                loop {
                    if bytes.get(j) == Some(&b'"') {
                        let mut close = 0;
                        while bytes.get(j + 1 + close) == Some(&b'#') && close < hashes {
                            close += 1;
                        }
                        if close == hashes {
                            for at in j..j + 1 + hashes {
                                blank(&mut out, at);
                            }
                            i = j + 1 + hashes;
                            break;
                        }
                    }
                    if j >= bytes.len() {
                        i = bytes.len();
                        break;
                    }
                    blank(&mut out, j);
                    j += 1;
                }
                continue;
            }
            i += 1;
        } else if bytes[i] == b'"' {
            blank(&mut out, i);
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    blank(&mut out, i);
                    i += 1;
                    if i < bytes.len() {
                        blank(&mut out, i);
                        i += 1;
                    }
                } else if bytes[i] == b'"' {
                    blank(&mut out, i);
                    i += 1;
                    break;
                } else {
                    blank(&mut out, i);
                    i += 1;
                }
            }
        } else if bytes[i] == b'\'' {
            // A char literal (`'{'`, `'\n'`) is blanked; a lifetime (`'a`) is
            // left alone.
            let is_char = bytes.get(i + 1) == Some(&b'\\')
                || (bytes.get(i + 2) == Some(&b'\'') && bytes.get(i + 1) != Some(&b'\n'));
            if is_char {
                blank(&mut out, i);
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        blank(&mut out, i);
                        i += 1;
                    } else if bytes[i] == b'\'' {
                        blank(&mut out, i);
                        i += 1;
                        break;
                    } else {
                        blank(&mut out, i);
                        i += 1;
                    }
                }
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).expect("masking preserves UTF-8")
}

/// Byte ranges of the argument lists of every `identifier(...)` call, from the
/// opening `(` to just after the matching `)`. Runs on masked text.
fn call_argument_spans(masked: &str, identifier: &str) -> Vec<(usize, usize)> {
    let bytes = masked.as_bytes();
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(offset) = masked[from..].find(identifier) {
        let at = from + offset;
        let before_ok = at == 0 || !is_ident_byte(bytes[at - 1]);
        let end = at + identifier.len();
        let after_ok = end >= bytes.len() || !is_ident_byte(bytes[end]);
        from = at + identifier.len();
        if !before_ok || !after_ok {
            continue;
        }
        let mut j = skip_spaces(bytes, end);
        if bytes.get(j) != Some(&b'(') {
            continue;
        }
        let mut depth = 0;
        while j < bytes.len() {
            match bytes[j] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        spans.push((at, j + 1));
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
    }
    spans
}

fn skip_spaces(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        at += 1;
    }
    at
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn line_of(source: &str, offset: usize) -> usize {
    source.as_bytes()[..offset]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// `ui_conn.lock()` (whitespace allowed) outside every blocking span.
fn ui_conn_locks_outside_blocking(masked: &str, body_offset: usize) -> Vec<usize> {
    let bytes = masked.as_bytes();
    let blocking = BLOCKING_CALLS
        .iter()
        .flat_map(|call| call_argument_spans(masked, call))
        .collect::<Vec<_>>();
    let mut offenders = Vec::new();
    let mut from = 0;
    while let Some(offset) = masked[from..].find("ui_conn") {
        let at = from + offset;
        from = at + "ui_conn".len();
        let before_ok = at == 0 || !is_ident_byte(bytes[at - 1]);
        let after_ok = from >= bytes.len() || !is_ident_byte(bytes[from]);
        if !before_ok || !after_ok {
            continue;
        }
        let after_conn = skip_spaces(bytes, from);
        if !masked[after_conn..].starts_with(".lock") {
            continue;
        }
        if !blocking
            .iter()
            .any(|(start, end)| *start <= at && at < *end)
        {
            offenders.push(body_offset + at);
        }
    }
    offenders
}

/// Settings/credential-store calls outside every blocking span.
fn settings_calls_outside_blocking(masked: &str, body_offset: usize) -> Vec<(usize, String)> {
    let blocking = BLOCKING_CALLS
        .iter()
        .flat_map(|call| call_argument_spans(masked, call))
        .collect::<Vec<_>>();
    let mut offenders = Vec::new();
    for call in SETTINGS_CALLS {
        let mut from = 0;
        while let Some(offset) = masked[from..].find(call) {
            let at = from + offset;
            from = at + call.len();
            if !blocking
                .iter()
                .any(|(start, end)| *start <= at && at < *end)
            {
                offenders.push((body_offset + at, call.to_string()));
            }
        }
    }
    offenders
}

fn scan() -> Vec<String> {
    let mut violations = Vec::new();
    for file in SCANNED_FILES {
        let source = read(file);
        let masked = mask(&source);
        for function in async_fns(&source) {
            let body_start = function.offset;
            let body_masked = &masked[body_start..body_start + function.body.len()];
            for offset in ui_conn_locks_outside_blocking(body_masked, body_start) {
                violations.push(format!(
                    "{file}:{} `{}` locks ui_conn outside run_blocking_db_task/spawn_blocking",
                    line_of(&source, offset),
                    function.name
                ));
            }
            for (offset, call) in settings_calls_outside_blocking(body_masked, body_start) {
                violations.push(format!(
                    "{file}:{} `{}` calls {call} outside run_blocking_db_task/spawn_blocking",
                    line_of(&source, offset),
                    function.name
                ));
            }
        }
    }
    violations
}

#[test]
fn async_functions_keep_ui_conn_and_settings_io_on_the_blocking_pool() {
    let violations = scan();
    assert!(
        violations.is_empty(),
        "async fns must delegate ui_conn and credential-store work to the blocking pool:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn the_scan_sees_the_command_modules_and_their_functions() {
    // A regression guard for the scan itself: if the files are renamed or the
    // async-fn detection stops matching, the test above would silently pass
    // over nothing.
    let mut async_fns_seen = 0;
    let mut modules_with_async_fns = 0;
    for file in SCANNED_FILES {
        let source = read(file);
        let functions = async_fns(&source);
        if !functions.is_empty() {
            modules_with_async_fns += 1;
        }
        async_fns_seen += functions.len();
    }
    assert!(
        async_fns_seen > 40,
        "the scan found only {async_fns_seen} async fns across {modules_with_async_fns} modules; \
         did the file list or the declaration detection drift?"
    );
    assert!(
        modules_with_async_fns >= 8,
        "only {modules_with_async_fns} scanned modules declare async fns"
    );
}
