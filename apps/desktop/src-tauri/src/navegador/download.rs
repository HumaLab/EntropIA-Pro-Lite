//! Downloads from the embedded browser: quarantine, verification, cleanup.
//!
//! A page may ask the webview to download a file, and the file is whatever the
//! server sends. The rules, in the order they apply:
//! - The address passes [`url_policy`] first, exactly like a navigation.
//! - The file goes to a quarantine directory under the app's CACHE directory,
//!   never the person's Downloads folder, under a name this module chooses
//!   (`<uuid>.part`). The name the page or server suggests is used only for
//!   display, after [`sanitize_file_name`]; it never reaches a path.
//! - When the download ends, this module looks only at the path IT chose (the
//!   directory the OS reports back is ignored), and keeps the file only if it
//!   is non-empty, within [`MAX_BYTES`] and starts with `%PDF-`. Anything else
//!   is deleted. A kept file is renamed `<uuid>.pdf` and hashed.
//! - Nothing is ever opened or executed.
//! - Files left behind by a crash or an abandoned draft are swept when the app
//!   starts ([`sweep_stale`]). Until the persistence phase decides where a
//!   verified PDF lives, quarantine is its only home, so this also removes
//!   verified PDFs older than [`STALE_AFTER`].
//!
//! The webview reports no progress, so a file cannot be stopped when it passes
//! the size cap while it downloads; it is refused and deleted once it ends.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde::Serialize;
use tauri::Url;

/// Largest PDF kept. The sync server's blob limit is 200 MB; this stays well
/// under it.
pub const MAX_BYTES: u64 = 100 * 1024 * 1024;
/// Downloads allowed to run at once.
pub const MAX_IN_FLIGHT: usize = 4;
/// Quarantined files older than this are removed at startup.
pub const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

const FILE_NAME_MAX_CHARS: usize = 120;
const FALLBACK_STEM: &str = "download";
const EXTENSION: &str = ".pdf";
const PDF_MAGIC: &[u8] = b"%PDF-";

/// Stable codes the UI maps to messages.
pub mod reason {
    pub const NOT_PDF: &str = "not_pdf";
    pub const TOO_LARGE: &str = "too_large";
    pub const EMPTY: &str = "empty";
    pub const INTERRUPTED: &str = "interrupted";
    pub const IO_ERROR: &str = "io_error";
    pub const TOO_MANY: &str = "too_many";
    pub const BLOCKED: &str = "blocked";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Downloading,
    Ready,
    Rejected,
    Failed,
}

/// A download that is not kept anywhere yet. Serialised to the main UI; it
/// carries no filesystem path (the file is `<id>.pdf` in quarantine).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDraft {
    pub id: String,
    pub url: String,
    /// For display only, sanitized.
    pub file_name: String,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    /// UTC, RFC 3339, from this process's clock.
    pub accessed_at: String,
    pub status: DownloadStatus,
    pub reason: Option<&'static str>,
}

impl DownloadDraft {
    /// The download was let through and is running.
    pub fn started(pending: &Pending) -> Self {
        Self::for_pending(pending, DownloadStatus::Downloading, None, None, None)
    }

    /// The download ended: verified, refused after the fact, or failed.
    pub fn finished(pending: &Pending, outcome: Result<Verified, &'static str>) -> Self {
        match outcome {
            Ok(verified) => Self::for_pending(
                pending,
                DownloadStatus::Ready,
                Some(verified.size),
                Some(verified.sha256),
                None,
            ),
            Err(why) => {
                // The file itself was wrong, or the machine failed us.
                let status = if matches!(why, reason::IO_ERROR | reason::INTERRUPTED) {
                    DownloadStatus::Failed
                } else {
                    DownloadStatus::Rejected
                };
                Self::for_pending(pending, status, None, None, Some(why))
            }
        }
    }

    /// The download was never let through.
    pub fn refused(url: &Url, suggested: &Path, why: &'static str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            url: url.to_string(),
            file_name: sanitize_file_name(&suggested.to_string_lossy()),
            size: None,
            sha256: None,
            accessed_at: super::capture::now_rfc3339(),
            status: DownloadStatus::Rejected,
            reason: Some(why),
        }
    }

    fn for_pending(
        pending: &Pending,
        status: DownloadStatus,
        size: Option<u64>,
        sha256: Option<String>,
        reason: Option<&'static str>,
    ) -> Self {
        Self {
            id: pending.id.clone(),
            url: pending.url.to_string(),
            file_name: pending.file_name.clone(),
            size,
            sha256,
            accessed_at: pending.accessed_at.clone(),
            status,
            reason,
        }
    }
}

/// A PDF that passed every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub size: u64,
    pub sha256: String,
}

/// A name that is safe to show and to store: no directories, no characters
/// Windows forbids, no reserved device names, at most 120 characters, and
/// always ending in `.pdf` (only verified PDFs are kept).
pub fn sanitize_file_name(raw: &str) -> String {
    // The last component, whichever separator the sender used.
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !super::capture::is_bidi_control(*c))
        .map(|c| {
            if c.is_control() || "<>:\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();

    let stem = match cleaned.len().checked_sub(EXTENSION.len()) {
        Some(at)
            if cleaned.is_char_boundary(at) && cleaned[at..].eq_ignore_ascii_case(EXTENSION) =>
        {
            &cleaned[..at]
        }
        _ => cleaned.as_str(),
    };
    let stem = trim_edges(stem);
    if stem.is_empty() {
        return format!("{FALLBACK_STEM}{EXTENSION}");
    }

    let first_part = stem.split('.').next().unwrap_or("").trim_end();
    let stem = if is_reserved_device_name(first_part) {
        format!("_{stem}")
    } else {
        stem.to_string()
    };
    let room = FILE_NAME_MAX_CHARS - EXTENSION.len();
    let stem: String = stem.chars().take(room).collect();
    format!("{}{EXTENSION}", trim_edges(&stem))
}

fn trim_edges(text: &str) -> &str {
    text.trim_matches(|c: char| c == '.' || c.is_whitespace())
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9` and `LPT1`-`LPT9`, which Windows
/// treats as devices whatever the extension.
fn is_reserved_device_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

/// Whether `head` (the first bytes of a file) is a PDF header.
pub fn is_pdf(head: &[u8]) -> bool {
    head.starts_with(PDF_MAGIC)
}

/// `<cache>/navegador/downloads`.
pub fn quarantine_dir(cache: &Path) -> PathBuf {
    cache.join("navegador").join("downloads")
}

/// Ids are ours (UUIDs); refusing anything else keeps a name from ever
/// climbing out of the quarantine directory.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn part_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.part"))
}

pub fn pdf_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.pdf"))
}

/// Check the finished download `<id>.part` in `dir` and, if it is a good PDF,
/// rename it `<id>.pdf`. On any failure the file is deleted.
pub fn finalize(dir: &Path, id: &str) -> Result<Verified, &'static str> {
    finalize_with_cap(dir, id, MAX_BYTES)
}

fn finalize_with_cap(dir: &Path, id: &str, max: u64) -> Result<Verified, &'static str> {
    if !valid_id(id) {
        return Err(reason::IO_ERROR);
    }
    let part = part_path(dir, id);
    let outcome = verify_part(&part, max).and_then(|verified| {
        fs::rename(&part, pdf_path(dir, id)).map_err(|_| reason::IO_ERROR)?;
        Ok(verified)
    });
    if outcome.is_err() {
        let _ = fs::remove_file(&part);
    }
    outcome
}

fn verify_part(part: &Path, max: u64) -> Result<Verified, &'static str> {
    use sha2::{Digest, Sha256};

    let mut file = fs::File::open(part).map_err(|_| reason::IO_ERROR)?;
    let length = file.metadata().map_err(|_| reason::IO_ERROR)?.len();
    if length == 0 {
        return Err(reason::EMPTY);
    }
    if length > max {
        return Err(reason::TOO_LARGE);
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut seen: u64 = 0;
    let mut head = Vec::with_capacity(PDF_MAGIC.len());
    loop {
        let read = file.read(&mut buffer).map_err(|_| reason::IO_ERROR)?;
        if read == 0 {
            break;
        }
        if head.len() < PDF_MAGIC.len() {
            let want = PDF_MAGIC.len() - head.len();
            head.extend_from_slice(&buffer[..read.min(want)]);
            if head.len() == PDF_MAGIC.len() && !is_pdf(&head) {
                return Err(reason::NOT_PDF);
            }
        }
        seen += read as u64;
        // The file may still be growing: the cap holds for what was read.
        if seen > max {
            return Err(reason::TOO_LARGE);
        }
        hasher.update(&buffer[..read]);
    }
    if !is_pdf(&head) {
        return Err(reason::NOT_PDF);
    }
    Ok(Verified {
        size: seen,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

/// Delete `<id>.part` after a failed or cancelled download.
pub fn discard_part(dir: &Path, id: &str) {
    if valid_id(id) {
        let _ = fs::remove_file(part_path(dir, id));
    }
}

/// Remove regular files in `dir` last modified more than `max_age` before
/// `now`. Returns how many were removed. A missing directory is not an error.
pub fn sweep_stale(dir: &Path, now: SystemTime, max_age: Duration) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter(|entry| {
            entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > max_age)
        })
        .filter(|entry| fs::remove_file(entry.path()).is_ok())
        .count()
}

/// A download that has been let through and has not ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: String,
    pub url: Url,
    pub file_name: String,
    /// When the download was requested: UTC, RFC 3339, this process's clock.
    pub accessed_at: String,
}

/// The downloads in flight, in the order they started.
#[derive(Default)]
pub struct Registry {
    pending: Mutex<Vec<Pending>>,
}

impl Registry {
    /// Register a download of `url` whose suggested file is `suggested`.
    pub fn begin(&self, url: &Url, suggested: &Path) -> Result<Pending, &'static str> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.len() >= MAX_IN_FLIGHT {
            return Err(reason::TOO_MANY);
        }
        let entry = Pending {
            id: uuid::Uuid::new_v4().to_string(),
            url: url.clone(),
            file_name: sanitize_file_name(&suggested.to_string_lossy()),
            accessed_at: super::capture::now_rfc3339(),
        };
        pending.push(entry.clone());
        Ok(entry)
    }

    /// The download whose quarantine file is `path` (matched on the file name
    /// alone: the directory the OS reports is not trusted).
    pub fn take_by_path(&self, path: &Path) -> Option<Pending> {
        let name = path.file_name()?.to_str()?;
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let at = pending
            .iter()
            .position(|p| format!("{}.part", p.id).eq_ignore_ascii_case(name))?;
        Some(pending.remove(at))
    }

    /// The oldest download of `url`, for a failure that reports no path.
    pub fn take_by_url(&self, url: &Url) -> Option<Pending> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let at = pending.iter().position(|p| p.url == *url)?;
        Some(pending.remove(at))
    }
}

#[cfg(test)]
mod tests {
    use super::super::capture::sha256_hex;
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    // --- names -------------------------------------------------------------

    #[test]
    fn an_ordinary_name_is_kept() {
        assert_eq!(sanitize_file_name("report.pdf"), "report.pdf");
        assert_eq!(
            sanitize_file_name("Informe año 2024.PDF"),
            "Informe año 2024.pdf"
        );
        assert_eq!(sanitize_file_name("report.v2.pdf"), "report.v2.pdf");
    }

    #[test]
    fn directories_are_dropped_whatever_the_separator() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd.pdf");
        assert_eq!(sanitize_file_name("..\\..\\windows\\evil.pdf"), "evil.pdf");
        assert_eq!(sanitize_file_name("C:\\Users\\x\\a.pdf"), "a.pdf");
        assert_eq!(sanitize_file_name("/abs/olute.pdf"), "olute.pdf");
        assert_eq!(sanitize_file_name("a/b\\c/d.pdf"), "d.pdf");
    }

    #[test]
    fn characters_windows_forbids_are_replaced() {
        assert_eq!(
            sanitize_file_name("a<b>c:d\"e|f?g*h.pdf"),
            "a_b_c_d_e_f_g_h.pdf"
        );
        assert_eq!(
            sanitize_file_name("tab\there\u{0}nul.pdf"),
            "tab_here_nul.pdf"
        );
    }

    #[test]
    fn bidi_overrides_are_removed() {
        assert_eq!(sanitize_file_name("gpj.\u{202e}fdp"), "gpj.fdp.pdf");
        assert_eq!(sanitize_file_name("a\u{2066}b.pdf"), "ab.pdf");
    }

    #[test]
    fn reserved_windows_device_names_are_prefixed() {
        for name in [
            "CON",
            "con.pdf",
            "PRN.pdf",
            "aux",
            "NUL.txt",
            "COM1.pdf",
            "com9",
            "LPT1.pdf",
            "lpt9",
            "CON.tar.gz",
        ] {
            let clean = sanitize_file_name(name);
            assert!(clean.starts_with('_'), "{name} -> {clean}");
            assert!(clean.ends_with(".pdf"), "{name} -> {clean}");
        }
        // Names that only look similar are left alone.
        assert_eq!(sanitize_file_name("console.pdf"), "console.pdf");
        assert_eq!(sanitize_file_name("COM10.pdf"), "COM10.pdf");
        assert_eq!(sanitize_file_name("com0.pdf"), "com0.pdf");
    }

    #[test]
    fn dots_and_spaces_at_the_edges_go() {
        assert_eq!(sanitize_file_name("  spaced name .pdf"), "spaced name.pdf");
        assert_eq!(sanitize_file_name("...hidden.pdf"), "hidden.pdf");
        assert_eq!(sanitize_file_name("trailing.dots..."), "trailing.dots.pdf");
    }

    #[test]
    fn nothing_usable_becomes_the_fallback() {
        for name in ["", "   ", "..", ".", ".pdf", "...", "/", "\\"] {
            assert_eq!(sanitize_file_name(name), "download.pdf", "input: {name:?}");
        }
    }

    #[test]
    fn a_name_that_is_not_a_pdf_name_gets_the_extension() {
        assert_eq!(sanitize_file_name("report"), "report.pdf");
        assert_eq!(sanitize_file_name("report.bin"), "report.bin.pdf");
        assert_eq!(sanitize_file_name("report.pdf.exe"), "report.pdf.exe.pdf");
    }

    #[test]
    fn a_long_name_is_cut_and_keeps_its_extension() {
        let clean = sanitize_file_name(&format!("{}.pdf", "a".repeat(400)));
        assert_eq!(clean.chars().count(), 120);
        assert!(clean.ends_with(".pdf"));
        let wide = sanitize_file_name(&format!("{}.pdf", "ñ".repeat(400)));
        assert_eq!(wide.chars().count(), 120);
        let emoji = sanitize_file_name(&format!("{}.pdf", "😀".repeat(400)));
        assert_eq!(emoji.chars().count(), 120);
        assert!(emoji.ends_with(".pdf"));
    }

    #[test]
    fn a_name_cut_short_never_ends_in_a_dot_or_space() {
        let raw = format!("{}. b.pdf", "a".repeat(115));
        let clean = sanitize_file_name(&raw);
        let stem = clean.strip_suffix(".pdf").unwrap();
        assert!(!stem.ends_with('.') && !stem.ends_with(' '), "{clean:?}");
    }

    // --- content ------------------------------------------------------------

    #[test]
    fn only_a_pdf_header_at_the_very_start_counts() {
        assert!(is_pdf(b"%PDF-1.7\n"));
        assert!(is_pdf(b"%PDF-"));
        assert!(!is_pdf(b"%PDF"));
        assert!(!is_pdf(b" %PDF-1.7"));
        assert!(!is_pdf(b"\n%PDF-1.7"));
        assert!(!is_pdf(b"<html>%PDF-"));
        assert!(!is_pdf(b"%pdf-1.7"));
        assert!(!is_pdf(b""));
        assert!(!is_pdf(b"PK\x03\x04"));
    }

    // --- paths --------------------------------------------------------------

    #[test]
    fn quarantine_lives_under_the_cache_directory() {
        let cache = Path::new("cache-root");
        assert_eq!(
            quarantine_dir(cache),
            Path::new("cache-root").join("navegador").join("downloads")
        );
        let dir = Path::new("q");
        assert_eq!(part_path(dir, "abc"), Path::new("q").join("abc.part"));
        assert_eq!(pdf_path(dir, "abc"), Path::new("q").join("abc.pdf"));
    }

    // --- finalize -----------------------------------------------------------

    fn write_part(dir: &Path, id: &str, bytes: &[u8]) {
        let mut file = File::create(part_path(dir, id)).unwrap();
        file.write_all(bytes).unwrap();
    }

    #[test]
    fn a_good_pdf_is_hashed_and_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let body = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n%%EOF\n";
        write_part(dir.path(), "id1", body);
        let verified = finalize(dir.path(), "id1").unwrap();
        assert_eq!(verified.size, body.len() as u64);
        assert_eq!(verified.sha256, sha256_hex(body));
        assert!(!part_path(dir.path(), "id1").exists());
        assert_eq!(fs::read(pdf_path(dir.path(), "id1")).unwrap(), body);
    }

    #[test]
    fn a_file_that_is_not_a_pdf_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        write_part(dir.path(), "id1", b"<html><script>alert(1)</script></html>");
        assert_eq!(finalize(dir.path(), "id1"), Err(reason::NOT_PDF));
        assert!(!part_path(dir.path(), "id1").exists());
        assert!(!pdf_path(dir.path(), "id1").exists());
    }

    #[test]
    fn a_pdf_renamed_from_something_else_is_still_judged_by_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        write_part(dir.path(), "id1", b"MZ\x90\x00 this is an executable");
        assert_eq!(finalize(dir.path(), "id1"), Err(reason::NOT_PDF));
    }

    #[test]
    fn an_empty_file_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        write_part(dir.path(), "id1", b"");
        assert_eq!(finalize(dir.path(), "id1"), Err(reason::EMPTY));
        assert!(!part_path(dir.path(), "id1").exists());
    }

    #[test]
    fn a_file_over_the_cap_is_deleted_unread() {
        let dir = tempfile::tempdir().unwrap();
        write_part(dir.path(), "id1", b"%PDF-1.7 and then some more bytes");
        assert_eq!(
            finalize_with_cap(dir.path(), "id1", 10),
            Err(reason::TOO_LARGE)
        );
        assert!(!part_path(dir.path(), "id1").exists());
    }

    #[test]
    fn a_file_exactly_at_the_cap_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let body = b"%PDF-1.7 exactly";
        write_part(dir.path(), "id1", body);
        assert!(finalize_with_cap(dir.path(), "id1", body.len() as u64).is_ok());
    }

    #[test]
    fn a_missing_file_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(finalize(dir.path(), "nothing"), Err(reason::IO_ERROR));
    }

    #[test]
    fn a_five_byte_header_alone_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        write_part(dir.path(), "id1", b"%PDF-");
        assert!(finalize(dir.path(), "id1").is_ok());
    }

    #[test]
    fn a_file_shorter_than_the_header_is_not_a_pdf() {
        let dir = tempfile::tempdir().unwrap();
        write_part(dir.path(), "id1", b"%PD");
        assert_eq!(finalize(dir.path(), "id1"), Err(reason::NOT_PDF));
    }

    #[test]
    fn an_id_cannot_reach_outside_the_quarantine_directory() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.part");
        fs::write(&outside, b"%PDF-1.7").unwrap();
        let inner = dir.path().join("q");
        fs::create_dir(&inner).unwrap();
        for id in ["../outside", "..\\outside", "a/../../outside", ""] {
            assert_eq!(finalize(&inner, id), Err(reason::IO_ERROR), "id: {id:?}");
            discard_part(&inner, id);
        }
        assert!(
            outside.exists(),
            "a hostile id deleted or moved a file outside"
        );
    }

    #[test]
    fn discarding_a_part_removes_it_and_tolerates_absence() {
        let dir = tempfile::tempdir().unwrap();
        write_part(dir.path(), "id1", b"partial");
        discard_part(dir.path(), "id1");
        assert!(!part_path(dir.path(), "id1").exists());
        discard_part(dir.path(), "id1");
    }

    // --- sweep --------------------------------------------------------------

    fn age(path: &Path, by: Duration) {
        let file = File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - by).unwrap();
    }

    #[test]
    fn stale_files_are_swept_and_fresh_ones_stay() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("old.part"), b"x").unwrap();
        fs::write(dir.path().join("old.pdf"), b"x").unwrap();
        fs::write(dir.path().join("fresh.pdf"), b"x").unwrap();
        age(&dir.path().join("old.part"), Duration::from_secs(48 * 3600));
        age(&dir.path().join("old.pdf"), Duration::from_secs(25 * 3600));
        age(&dir.path().join("fresh.pdf"), Duration::from_secs(3600));
        let removed = sweep_stale(dir.path(), SystemTime::now(), STALE_AFTER);
        assert_eq!(removed, 2);
        assert!(!dir.path().join("old.part").exists());
        assert!(!dir.path().join("old.pdf").exists());
        assert!(dir.path().join("fresh.pdf").exists());
    }

    #[test]
    fn the_sweep_leaves_directories_alone() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("inner.pdf"), b"x").unwrap();
        age(&sub.join("inner.pdf"), Duration::from_secs(72 * 3600));
        assert_eq!(sweep_stale(dir.path(), SystemTime::now(), STALE_AFTER), 0);
        assert!(sub.join("inner.pdf").exists());
    }

    #[test]
    fn sweeping_a_missing_directory_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("never-created");
        assert_eq!(sweep_stale(&gone, SystemTime::now(), STALE_AFTER), 0);
    }

    // --- registry -----------------------------------------------------------

    #[test]
    fn a_download_is_registered_with_a_clean_display_name() {
        let registry = Registry::default();
        let pending = registry
            .begin(
                &url("https://example.com/p.pdf"),
                Path::new("C:\\Users\\x\\Downloads\\..\\p<1>.pdf"),
            )
            .unwrap();
        assert_eq!(pending.file_name, "p_1_.pdf");
        assert_eq!(pending.url, url("https://example.com/p.pdf"));
        assert!(!pending.id.is_empty());
    }

    #[test]
    fn every_download_gets_its_own_id() {
        let registry = Registry::default();
        let a = registry
            .begin(&url("https://a.test/1"), Path::new("a"))
            .unwrap();
        let b = registry
            .begin(&url("https://a.test/1"), Path::new("a"))
            .unwrap();
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn only_a_few_downloads_run_at_once() {
        let registry = Registry::default();
        let mut held = Vec::new();
        for _ in 0..MAX_IN_FLIGHT {
            held.push(
                registry
                    .begin(&url("https://a.test/x"), Path::new("x"))
                    .unwrap(),
            );
        }
        assert_eq!(
            registry.begin(&url("https://a.test/x"), Path::new("x")),
            Err(reason::TOO_MANY)
        );
        // One ends: there is room again.
        registry.take_by_url(&url("https://a.test/x")).unwrap();
        assert!(registry
            .begin(&url("https://a.test/x"), Path::new("x"))
            .is_ok());
    }

    #[test]
    fn a_finished_download_is_found_by_its_file_name_alone() {
        let registry = Registry::default();
        let pending = registry
            .begin(&url("https://a.test/x"), Path::new("x"))
            .unwrap();
        let elsewhere = Path::new("Z:\\somewhere\\else").join(format!("{}.part", pending.id));
        assert_eq!(registry.take_by_path(&elsewhere), Some(pending));
    }

    #[test]
    fn an_unknown_path_finds_nothing() {
        let registry = Registry::default();
        registry
            .begin(&url("https://a.test/x"), Path::new("x"))
            .unwrap();
        assert_eq!(registry.take_by_path(Path::new("q/not-ours.part")), None);
        assert_eq!(registry.take_by_path(Path::new("")), None);
    }

    #[test]
    fn a_download_is_taken_once() {
        let registry = Registry::default();
        let pending = registry
            .begin(&url("https://a.test/x"), Path::new("x"))
            .unwrap();
        let path = part_path(Path::new("q"), &pending.id);
        assert!(registry.take_by_path(&path).is_some());
        assert!(registry.take_by_path(&path).is_none());
        assert!(registry.take_by_url(&url("https://a.test/x")).is_none());
    }

    #[test]
    fn a_failure_without_a_path_takes_the_oldest_download_of_that_address() {
        let registry = Registry::default();
        let first = registry
            .begin(&url("https://a.test/x"), Path::new("x"))
            .unwrap();
        let other = registry
            .begin(&url("https://a.test/y"), Path::new("y"))
            .unwrap();
        let second = registry
            .begin(&url("https://a.test/x"), Path::new("x"))
            .unwrap();
        assert_eq!(registry.take_by_url(&url("https://a.test/x")), Some(first));
        assert_eq!(registry.take_by_url(&url("https://a.test/x")), Some(second));
        assert_eq!(registry.take_by_url(&url("https://a.test/y")), Some(other));
    }

    // --- draft --------------------------------------------------------------

    fn pending() -> Pending {
        Registry::default()
            .begin(&url("https://a.test/p.pdf"), Path::new("p.pdf"))
            .unwrap()
    }

    #[test]
    fn a_registered_download_carries_the_time_it_was_requested() {
        let at = pending().accessed_at;
        assert_eq!(at.len(), 20, "{at}");
        assert!(at.ends_with('Z'));
    }

    #[test]
    fn a_download_that_started_has_neither_size_nor_hash() {
        let p = pending();
        let draft = DownloadDraft::started(&p);
        assert_eq!(draft.id, p.id);
        assert_eq!(draft.url, "https://a.test/p.pdf");
        assert_eq!(draft.file_name, "p.pdf");
        assert_eq!(draft.accessed_at, p.accessed_at);
        assert_eq!(draft.status, DownloadStatus::Downloading);
        assert_eq!((draft.size, draft.sha256, draft.reason), (None, None, None));
    }

    #[test]
    fn a_verified_pdf_is_ready_with_its_size_and_hash() {
        let p = pending();
        let draft = DownloadDraft::finished(
            &p,
            Ok(Verified {
                size: 42,
                sha256: "ab".repeat(32),
            }),
        );
        assert_eq!(draft.status, DownloadStatus::Ready);
        assert_eq!(draft.size, Some(42));
        assert_eq!(draft.sha256, Some("ab".repeat(32)));
        assert_eq!(draft.reason, None);
        assert_eq!(draft.accessed_at, p.accessed_at);
    }

    #[test]
    fn a_file_the_checks_refuse_is_rejected_and_an_io_failure_is_failed() {
        let p = pending();
        for why in [reason::NOT_PDF, reason::TOO_LARGE, reason::EMPTY] {
            let draft = DownloadDraft::finished(&p, Err(why));
            assert_eq!(draft.status, DownloadStatus::Rejected, "{why}");
            assert_eq!(draft.reason, Some(why));
            assert_eq!((draft.size, draft.sha256), (None, None));
        }
        for why in [reason::IO_ERROR, reason::INTERRUPTED] {
            let draft = DownloadDraft::finished(&p, Err(why));
            assert_eq!(draft.status, DownloadStatus::Failed, "{why}");
            assert_eq!(draft.reason, Some(why));
        }
    }

    #[test]
    fn a_refused_download_is_rejected_with_a_sanitized_name() {
        let draft = DownloadDraft::refused(
            &url("http://169.254.169.254/x"),
            Path::new(r"..\..\evil<.pdf"),
            reason::BLOCKED,
        );
        assert_eq!(draft.status, DownloadStatus::Rejected);
        assert_eq!(draft.reason, Some(reason::BLOCKED));
        assert_eq!(draft.file_name, "evil_.pdf");
        assert!(!draft.id.is_empty());
        assert_eq!(draft.accessed_at.len(), 20);
    }

    #[test]
    fn the_draft_reaches_the_ui_without_a_path() {
        let draft = DownloadDraft {
            id: "id1".into(),
            url: "https://a.test/x.pdf".into(),
            file_name: "x.pdf".into(),
            size: Some(10),
            sha256: Some("ab".repeat(32)),
            accessed_at: "2026-09-30T12:00:00Z".into(),
            status: DownloadStatus::Ready,
            reason: None,
        };
        let value = serde_json::to_value(draft).unwrap();
        let object = value.as_object().unwrap();
        for key in [
            "id",
            "url",
            "fileName",
            "size",
            "sha256",
            "accessedAt",
            "status",
            "reason",
        ] {
            assert!(object.contains_key(key), "missing {key}");
        }
        assert_eq!(
            object.len(),
            8,
            "an unexpected field would leak: {object:?}"
        );
        assert_eq!(object["status"], "ready");
        let rejected = serde_json::to_value(DownloadStatus::Rejected).unwrap();
        assert_eq!(rejected, "rejected");
    }
}
