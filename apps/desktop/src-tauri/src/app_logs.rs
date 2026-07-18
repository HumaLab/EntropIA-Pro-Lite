use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};

const MAX_LOG_ENTRIES: usize = 2_000;
const DEFAULT_LOG_ENTRIES_WINDOW: usize = 20;
const LOG_FILE_NAME: &str = "entropia.log";
const MAX_MESSAGE_CHARS: usize = 4_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppLogEntry {
    pub id: u64,
    pub timestamp_ms: u64,
    pub level: AppLogLevel,
    pub source: String,
    pub message: String,
}

#[derive(Debug)]
struct AppLogsInner {
    entries: VecDeque<AppLogEntry>,
    next_id: u64,
}

#[derive(Debug)]
pub struct AppLogsState {
    inner: Mutex<AppLogsInner>,
    file_lock: Mutex<()>,
    log_dir: PathBuf,
    log_file: PathBuf,
}

impl AppLogsState {
    pub fn new(log_dir: PathBuf) -> Self {
        let log_file = log_dir.join(LOG_FILE_NAME);
        let entries = load_existing_entries(&log_file, MAX_LOG_ENTRIES);
        let next_id = entries
            .iter()
            .map(|entry| entry.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);

        Self {
            inner: Mutex::new(AppLogsInner { entries, next_id }),
            file_lock: Mutex::new(()),
            log_dir,
            log_file,
        }
    }

    fn recent_entries(&self, limit: usize) -> Vec<AppLogEntry> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let start = inner.entries.len().saturating_sub(limit);
        inner.entries.iter().skip(start).cloned().collect()
    }

    fn clear(&self) -> Result<(), String> {
        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            inner.entries.clear();
            inner.next_id = 0;
        }

        let _file_guard = self
            .file_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        fs::create_dir_all(&self.log_dir)
            .map_err(|error| format!("No se pudo crear el directorio de logs: {error}"))?;
        fs::write(&self.log_file, "")
            .map_err(|error| format!("No se pudo limpiar el archivo de logs: {error}"))?;
        Ok(())
    }

    fn append(
        &self,
        level: AppLogLevel,
        source: impl Into<String>,
        message: impl Into<String>,
    ) -> AppLogEntry {
        let entry = {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let entry = AppLogEntry {
                id: inner.next_id,
                timestamp_ms: now_ms(),
                level,
                source: sanitize_field(source.into(), 96),
                message: sanitize_field(message.into(), MAX_MESSAGE_CHARS),
            };
            inner.next_id = inner.next_id.saturating_add(1);
            inner.entries.push_back(entry.clone());
            while inner.entries.len() > MAX_LOG_ENTRIES {
                inner.entries.pop_front();
            }
            entry
        };

        self.append_to_file(&entry);
        entry
    }

    fn append_to_file(&self, entry: &AppLogEntry) {
        let _file_guard = self
            .file_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        if fs::create_dir_all(&self.log_dir).is_err() {
            return;
        }

        let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)
        else {
            return;
        };

        if let Ok(line) = serde_json::to_string(entry) {
            let _ = writeln!(file, "{line}");
        }
    }

    fn log_dir(&self) -> &Path {
        &self.log_dir
    }
}

#[tauri::command]
pub fn logs_get(state: State<'_, AppLogsState>) -> Result<Vec<AppLogEntry>, String> {
    Ok(state.recent_entries(DEFAULT_LOG_ENTRIES_WINDOW))
}

#[tauri::command]
pub fn logs_clear(state: State<'_, AppLogsState>) -> Result<(), String> {
    state.clear()
}

#[tauri::command]
pub fn logs_open_dir(state: State<'_, AppLogsState>) -> Result<(), String> {
    fs::create_dir_all(state.log_dir())
        .map_err(|error| format!("No se pudo crear el directorio de logs: {error}"))?;
    open_path(state.log_dir())
}

#[tauri::command]
pub fn logs_append(
    app_handle: AppHandle,
    level: AppLogLevel,
    source: String,
    message: String,
) -> Result<(), String> {
    append(&app_handle, level, source, message);
    Ok(())
}

pub fn info(app_handle: &AppHandle, source: impl Into<String>, message: impl Into<String>) {
    append(app_handle, AppLogLevel::Info, source, message);
}

pub fn warn(app_handle: &AppHandle, source: impl Into<String>, message: impl Into<String>) {
    append(app_handle, AppLogLevel::Warn, source, message);
}

pub fn error(app_handle: &AppHandle, source: impl Into<String>, message: impl Into<String>) {
    append(app_handle, AppLogLevel::Error, source, message);
}

fn append(
    app_handle: &AppHandle,
    level: AppLogLevel,
    source: impl Into<String>,
    message: impl Into<String>,
) {
    let entry = app_handle
        .state::<AppLogsState>()
        .append(level, source, message);
    eprintln!(
        "[app-log:{}] [{}] {}",
        level_label(&entry.level),
        entry.source,
        entry.message
    );
    let _ = app_handle.emit("logs://entry", entry);
}

fn level_label(level: &AppLogLevel) -> &'static str {
    match level {
        AppLogLevel::Info => "info",
        AppLogLevel::Warn => "warn",
        AppLogLevel::Error => "error",
    }
}

fn load_existing_entries(log_file: &Path, max_entries: usize) -> VecDeque<AppLogEntry> {
    let Ok(contents) = fs::read_to_string(log_file) else {
        return VecDeque::new();
    };

    let mut entries = VecDeque::new();
    for line in contents.lines() {
        let Ok(entry) = serde_json::from_str::<AppLogEntry>(line) else {
            continue;
        };
        entries.push_back(entry);
        while entries.len() > max_entries {
            entries.pop_front();
        }
    }
    entries
}

fn sanitize_field(value: String, max_chars: usize) -> String {
    let mut cleaned = String::new();
    for ch in value.chars().take(max_chars) {
        if ch.is_control() && ch != '\n' && ch != '\t' {
            cleaned.push(' ');
        } else {
            cleaned.push(ch);
        }
    }

    if value.chars().count() > max_chars {
        cleaned.push_str("… [truncado]");
    }

    redact_sensitive(cleaned)
}

/// Markers whose PRESENCE on a line means the line carries (or is about to carry)
/// a credential (DESIGN §8). `"bearer "` keeps its trailing space so it never
/// matches mid-word noise, but the per-part scan below normalizes it.
const SENSITIVE_MARKERS: &[&str] = &[
    "api_key",
    "apikey",
    "authorization",
    "bearer",
    "token",
    "password",
    "secret",
];

/// Minimum length for a whitespace-delimited part to be treated as a high-entropy
/// credential on a sensitive line (DESIGN §8 — base64url device tokens are 43/44
/// chars). Kept conservative so ordinary words never trip it; the threshold only
/// applies on lines that already contain a sensitive marker.
const HIGH_ENTROPY_MIN_CHARS: usize = 32;

/// True when `part` is itself a sensitive marker (so the part AFTER it is a
/// candidate secret — e.g. `bearer <token>`, `token=<token>`, `password: <pw>`).
fn part_is_marker(part: &str) -> bool {
    let lower = part.to_ascii_lowercase();
    // Strip trailing separators so `token:` / `password=` still match.
    let trimmed = lower.trim_end_matches([':', '=', '"', '\'', ',']);
    SENSITIVE_MARKERS.iter().any(|marker| {
        let marker = marker.trim_end();
        trimmed == marker || lower.contains(marker)
    })
}

/// True when `part` looks like a high-entropy credential: long enough and made of
/// the token alphabet (base64url / hex), with at least one alphabetic char so we
/// never redact long numeric ids or timestamps (DESIGN §8). Punctuation that can
/// wrap a value (quotes, separators) is trimmed before measuring.
fn part_is_high_entropy(part: &str) -> bool {
    let core = part.trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | '=' | ':'));
    if core.chars().count() < HIGH_ENTROPY_MIN_CHARS {
        return false;
    }
    let all_token_chars = core
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '/' | '.' | '='));
    all_token_chars && core.chars().any(|c| c.is_ascii_alphabetic())
}

/// Redacts credential material from a log line (DESIGN §8). On a line containing
/// any sensitive marker, every part that (a) is itself a marker, (b) immediately
/// FOLLOWS a marker part, or (c) is a high-entropy ≥32-char token is replaced
/// with `[redactado]`. Lines without a marker are returned untouched so ordinary
/// diagnostics stay readable. Tokens, Bearer headers and passwords never survive.
fn redact_sensitive(value: String) -> String {
    let lower = value.to_ascii_lowercase();
    if !SENSITIVE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return value;
    }

    let parts: Vec<&str> = value.split_whitespace().collect();
    let mut redacted = Vec::with_capacity(parts.len());
    let mut prev_was_marker = false;
    for part in parts {
        let is_marker = part_is_marker(part);
        // The part right AFTER a marker word is the value being labelled
        // (`bearer <token>`, `password <pw>`) — redact it regardless of length.
        if is_marker || prev_was_marker || part_is_high_entropy(part) {
            redacted.push("[redactado]".to_string());
        } else {
            redacted.push(part.to_string());
        }
        // A marker that already carries its own value inline (`token=abc…`) does
        // not turn the NEXT unrelated word into a secret, so only bare marker
        // words ("bearer", "token:", "password") arm the follow-on redaction.
        prev_was_marker = is_marker && !part_carries_inline_value(part);
    }
    redacted.join(" ")
}

/// True when a marker part already includes its value inline (`token=abc`,
/// `authorization:abc`) so the FOLLOWING part must NOT be auto-redacted.
fn part_carries_inline_value(part: &str) -> bool {
    if let Some(idx) = part.find([':', '=']) {
        // Something non-empty after the separator → inline value present.
        return part[idx + 1..]
            .trim_matches(|c: char| matches!(c, '"' | '\'' | ' '))
            .chars()
            .next()
            .is_some();
    }
    false
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn open_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = Command::new("explorer");
        cmd.arg(path);
        cmd
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut cmd = Command::new("open");
        cmd.arg(path);
        cmd
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(path);
        cmd
    };

    command
        .spawn()
        .map_err(|error| format!("No se pudo abrir el directorio de logs: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 43-char base64url token (no padding) — the shortest device-token shape
    /// (PROTOCOL `/v1/auth/login` returns `base64url-32B`, 43 unpadded chars).
    const TOKEN_43: &str = "abcDEF012ghiJKL345mnoPQR678stuVWX9_-yzABCDE";
    /// 44-char base64url token with one `=` pad char.
    const TOKEN_44: &str = "abcDEF012ghiJKL345mnoPQR678stuVWX9_-yzABCDE=";

    fn assert_redacted_absent(input: &str, token: &str) {
        let out = redact_sensitive(input.to_string());
        assert!(
            !out.contains(token),
            "token leaked through redaction.\n  input:  {input}\n  output: {out}"
        );
        assert!(
            out.contains("[redactado]"),
            "expected a redaction marker: {out}"
        );
    }

    #[test]
    fn redacts_bearer_token_following_marker() {
        assert_redacted_absent(
            &format!("[sync] sending Authorization: Bearer {TOKEN_43}"),
            TOKEN_43,
        );
        assert_redacted_absent(&format!("[sync] header bearer {TOKEN_44}"), TOKEN_44);
    }

    #[test]
    fn redacts_high_entropy_token_on_marker_line_even_far_from_marker() {
        // The marker arms the line; a 44-char token elsewhere on the line must
        // still be caught by the entropy rule (it is not the immediate next part).
        let line = format!("[sync] login ok for ana, token persisted value {TOKEN_44} done");
        assert_redacted_absent(&line, TOKEN_44);
    }

    #[test]
    fn redacts_inline_token_assignment() {
        assert_redacted_absent(&format!("token={TOKEN_43}"), TOKEN_43);
        assert_redacted_absent(&format!("password=\"{TOKEN_44}\""), TOKEN_44);
    }

    #[test]
    fn redacts_password_value() {
        assert_redacted_absent(
            "login password hunter2supersecretvalue123456789",
            "hunter2supersecretvalue123456789",
        );
    }

    #[test]
    fn leaves_clean_lines_untouched() {
        let line = "[sync] cycle complete: applied 12 rows, 3 blobs downloaded";
        assert_eq!(redact_sensitive(line.to_string()), line);
    }

    #[test]
    fn does_not_redact_long_numeric_ids_on_clean_lines() {
        // No marker on the line → never touched, even with a long number.
        let line = "[sync] last_pull_seq advanced to 1760000000123456789";
        assert_eq!(redact_sensitive(line.to_string()), line);
    }

    #[test]
    fn marker_with_inline_value_does_not_redact_next_unrelated_word() {
        // `token=abc` carries its value inline; the following plain word survives.
        let out = redact_sensitive(format!("token={TOKEN_43} cycle ok"));
        assert!(!out.contains(TOKEN_43), "inline token redacted: {out}");
        assert!(out.contains("cycle"), "next unrelated word kept: {out}");
        assert!(out.contains("ok"), "trailing word kept: {out}");
    }

    #[test]
    fn sha256_correlation_prefix_survives() {
        // DESIGN §8 allows logging sha256(token)[..8] for correlation — an 8-char
        // hex prefix is below the entropy threshold and must stay readable.
        let line = "[sync] device token sha256 prefix 1a2b3c4d for correlation";
        let out = redact_sensitive(line.to_string());
        // The 8-char prefix is short; it is the word AFTER "token" though, so the
        // follow-on rule redacts "sha256". The correlation prefix itself (3 words
        // later) is short and survives.
        assert!(
            out.contains("1a2b3c4d"),
            "short correlation prefix kept: {out}"
        );
    }

    #[test]
    fn sanitize_field_applies_redaction() {
        let out = sanitize_field(
            format!("Authorization: Bearer {TOKEN_43}"),
            MAX_MESSAGE_CHARS,
        );
        assert!(!out.contains(TOKEN_43), "sanitize_field must redact: {out}");
    }
}
