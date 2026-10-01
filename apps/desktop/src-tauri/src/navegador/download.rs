//! Downloads from the embedded browser: routing, quarantine, verification,
//! cleanup.
//!
//! A page may ask the webview to download a file, and the file is whatever the
//! server sends. EntropIA only stores what it can verify (a PDF); everything
//! else is the person's, not ours. The rules, in the order they apply:
//! - The address passes [`url_policy`] first, exactly like a navigation.
//! - [`route_for`] decides by the suggested file name and the address: a `.pdf`
//!   (or a name that says nothing, like no extension or `.php`) goes to
//!   quarantine to be checked; a name with any other extension goes straight
//!   to the person's download folder ([`Route::UserFolder`]).
//! - Quarantine is a directory under the app's CACHE directory, under a name
//!   this module chooses (`<uuid>.part`). The name the page or server suggests
//!   is used only for display, after [`sanitize_name`]; it never reaches the
//!   quarantine path.
//! - When a quarantined download ends, this module looks only at the path IT
//!   chose (the directory the OS reports back is ignored), and keeps the file
//!   only if it is non-empty, within [`MAX_BYTES`] and starts with `%PDF-`. A
//!   kept file is renamed `<uuid>.pdf` and hashed. A file that is not a PDF is
//!   MOVED to the download folder (never deleted: it is the person's file);
//!   only an empty or over-cap PDF, or a failure, is deleted.
//! - A file routed to the download folder keeps a sanitized version of the
//!   suggested name, never overwrites anything (` (1)`, ` (2)`...), and is left
//!   alone when it ends, even if it turns out to be a PDF (no auto-import).
//!   The path this module chose is the only one it ever looks at.
//! - Nothing is ever opened or executed.
//! - Files left behind by a crash or an abandoned draft are swept when the app
//!   starts ([`sweep_stale`]). Until the persistence phase decides where a
//!   verified PDF lives, quarantine is its only home, so this also removes
//!   verified PDFs older than [`STALE_AFTER`].
//!
//! The webview reports no progress, so a file cannot be stopped when it passes
//! the size cap while it downloads; it is refused and deleted once it ends.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
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
/// Longest extension taken as one (`.tar`, `.torrent`, `.webarchive`).
const EXTENSION_MAX_CHARS: usize = 16;
const FALLBACK_STEM: &str = "download";
const EXTENSION: &str = ".pdf";
const PDF_MAGIC: &[u8] = b"%PDF-";

/// Where a download goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Checked, and kept only if it is a PDF.
    Quarantine,
    /// The person's download folder; EntropIA keeps nothing.
    UserFolder,
}

/// Extensions of files generated on request, whose name says nothing about the
/// type: judged by their bytes like a name without an extension.
const UNINFORMATIVE_EXTENSIONS: &[&str] = &[
    "php",
    "php3",
    "php4",
    "php5",
    "asp",
    "aspx",
    "ashx",
    "axd",
    "jsp",
    "jspx",
    "cgi",
    "cfm",
    "do",
    "action",
    "bin",
    "dat",
    "tmp",
    "download",
    "part",
    "crdownload",
];

/// The extension of `name`, lowercase, when it looks like one: something after
/// the last dot, letters and digits only, and a stem before it.
fn extension_of(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    let plain = !ext.is_empty()
        && ext.chars().count() <= EXTENSION_MAX_CHARS
        && ext.chars().all(|c| c.is_ascii_alphanumeric());
    (plain && !stem.trim().is_empty()).then(|| ext.to_ascii_lowercase())
}

/// The last path segment of `url`. A `blob:<origin>/<id>` address has none of
/// its own (its path is the origin), so it yields nothing.
fn url_file_name(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.next_back()?;
    (!segment.is_empty()).then(|| segment.to_string())
}

/// Decide where a download of `url` suggested as `suggested` goes. The file
/// name wins, then the address; a name that tells nothing (no extension, or a
/// generated page like `.php`) is checked in quarantine, because it may well
/// be a PDF. The decision is only about where the file lands first: a
/// quarantined file that is not a PDF still ends up in the person's folder.
pub fn route_for(suggested: &str, url: &Url) -> Route {
    let judge = |extension: Option<String>| match extension.as_deref() {
        Some("pdf") => Some(Route::Quarantine),
        Some(other) if UNINFORMATIVE_EXTENSIONS.contains(&other) => None,
        Some(_) => Some(Route::UserFolder),
        None => None,
    };
    judge(extension_of(&sanitize_name(suggested)))
        .or_else(|| judge(url_file_name(url).and_then(|name| extension_of(&name))))
        .unwrap_or(Route::Quarantine)
}

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
    /// A verified PDF, held in quarantine.
    Ready,
    /// A file EntropIA does not keep, written to the person's folder.
    Saved,
    Rejected,
    Failed,
}

/// A download, as the main UI sees it. It carries no path into quarantine (the
/// file is `<id>.pdf` there); a file saved to the person's folder carries that
/// folder, which is the person's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDraft {
    pub id: String,
    pub url: String,
    /// For display only, sanitized. `.pdf` is only ever appended to a verified
    /// PDF: a file that turned out to be a zip is not shown as `x.zip.pdf`.
    pub file_name: String,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    /// The folder a `saved` file went to.
    pub saved_to: Option<String>,
    /// UTC, RFC 3339, from this process's clock.
    pub accessed_at: String,
    pub status: DownloadStatus,
    pub reason: Option<&'static str>,
    /// The browser tab whose page started the download (`None` for one a popup
    /// window started). The download outlives a tab switch, and the tab itself
    /// may be closed before it ends: this is only for display.
    pub tab: Option<u32>,
    /// The page the download started from, as it was at that moment. A tab
    /// keeps navigating, so this is a snapshot: it never follows the tab.
    pub page_url: Option<String>,
    pub page_title: Option<String>,
    /// The source that already holds this exact PDF (same sha256), when the
    /// archive says so: the file is not kept in quarantine and cannot be saved
    /// again.
    pub already_saved_in: Option<String>,
}

impl DownloadDraft {
    /// The download was let through and is running.
    pub fn started(pending: &Pending) -> Self {
        Self::for_pending(pending, DownloadStatus::Downloading, None, None, None)
    }

    /// The file is in the person's folder at `path`: not ours to keep, no hash.
    pub fn saved(pending: &Pending, path: &Path, size: Option<u64>) -> Self {
        let mut draft = Self::for_pending(pending, DownloadStatus::Saved, size, None, None);
        if let Some(name) = path.file_name() {
            draft.file_name = name.to_string_lossy().into_owned();
        }
        draft.saved_to = path.parent().map(|dir| dir.to_string_lossy().into_owned());
        draft
    }

    /// The download ended: verified, refused after the fact, or failed.
    pub fn finished(pending: &Pending, outcome: Result<Verified, &'static str>) -> Self {
        match outcome {
            Ok(verified) => {
                let mut draft = Self::for_pending(
                    pending,
                    DownloadStatus::Ready,
                    Some(verified.size),
                    Some(verified.sha256),
                    None,
                );
                // Only a verified PDF gets to be called one.
                draft.file_name = sanitize_file_name(&pending.file_name);
                draft
            }
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
            file_name: sanitize_name(&suggested.to_string_lossy()),
            size: None,
            sha256: None,
            saved_to: None,
            accessed_at: super::capture::now_rfc3339(),
            status: DownloadStatus::Rejected,
            reason: Some(why),
            tab: None,
            page_url: None,
            page_title: None,
            already_saved_in: None,
        }
    }

    /// The same draft, saying the archive already holds these bytes in `source_id`.
    pub fn with_already_saved_in(mut self, source_id: impl Into<String>) -> Self {
        self.already_saved_in = Some(source_id.into());
        self
    }

    /// The same draft, saying which page asked for the download.
    pub fn with_page(mut self, url: Option<&str>, title: Option<&str>) -> Self {
        self.page_url = url.map(str::to_string);
        self.page_title = title.and_then(|t| super::capture::clean_line(t, PAGE_TITLE_MAX));
        self
    }

    /// The same draft, saying which tab's page asked for the download.
    pub fn with_tab(mut self, tab: Option<u32>) -> Self {
        self.tab = tab;
        self
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
            saved_to: None,
            accessed_at: pending.accessed_at.clone(),
            status,
            reason,
            tab: pending.tab,
            page_url: pending.page_url.clone(),
            page_title: pending.page_title.clone(),
            already_saved_in: None,
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

/// A name that is safe to write in the person's folder and to show: like
/// [`sanitize_file_name`] (no directories, none of the characters Windows
/// forbids, no reserved device names, at most 120 characters, no dots or spaces
/// at the edges) but it keeps whatever extension the name has and adds none.
pub fn sanitize_name(raw: &str) -> String {
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
    let name = trim_edges(&cleaned);
    if name.is_empty() {
        return FALLBACK_STEM.to_string();
    }
    let first_part = name.split('.').next().unwrap_or("").trim_end();
    let name = if is_reserved_device_name(first_part) {
        format!("_{name}")
    } else {
        name.to_string()
    };
    cut_keeping_extension(&name)
}

/// At most [`FILE_NAME_MAX_CHARS`] characters, cutting the stem so a plausible
/// extension survives.
fn cut_keeping_extension(name: &str) -> String {
    if name.chars().count() <= FILE_NAME_MAX_CHARS {
        return name.to_string();
    }
    if let Some((stem, ext)) = name.rsplit_once('.') {
        let ext_chars = ext.chars().count();
        if !stem.is_empty() && ext_chars <= EXTENSION_MAX_CHARS {
            let room = FILE_NAME_MAX_CHARS - ext_chars - 1;
            let stem: String = stem.chars().take(room).collect();
            let stem = trim_edges(&stem);
            let stem = if stem.is_empty() { FALLBACK_STEM } else { stem };
            return format!("{stem}.{ext}");
        }
    }
    let cut: String = name.chars().take(FILE_NAME_MAX_CHARS).collect();
    trim_edges(&cut).to_string()
}

/// `name`, or `name (1)`, `name (2)`... before the extension, the first one
/// `taken` says is free. Never returns a name `taken` reports as used.
pub fn unique_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.chars().count() <= EXTENSION_MAX_CHARS => {
            (stem, format!(".{ext}"))
        }
        _ => (name, String::new()),
    };
    let numbered = |counter: &dyn std::fmt::Display| {
        // Cut the stem, not the number: the suffix has to survive the limit.
        let suffix = format!(" ({counter})");
        let room = FILE_NAME_MAX_CHARS.saturating_sub(ext.chars().count() + suffix.chars().count());
        let stem: String = stem.chars().take(room).collect();
        format!("{}{suffix}{ext}", trim_edges(&stem))
    };
    for n in 1..=9999u32 {
        let candidate = numbered(&n);
        if !taken(&candidate) {
            return candidate;
        }
    }
    // Ten thousand copies of one name: stop counting.
    numbered(&uuid::Uuid::new_v4())
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

/// The folder non-PDF downloads go to: the one the person chose if it is still
/// a directory, else the system's Downloads folder if that is one, else none.
pub fn resolve_folder(
    chosen: Option<&Path>,
    default: Option<PathBuf>,
    is_dir: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    chosen
        .map(Path::to_path_buf)
        .filter(|dir| is_dir(dir))
        .or_else(|| default.filter(|dir| is_dir(dir)))
}

/// Check a folder the person picked before it is remembered: text, absolute,
/// and an existing directory. Returns the path as given.
pub fn validate_folder(input: &str) -> Result<PathBuf, &'static str> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(folder_error::INVALID);
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(folder_error::NOT_ABSOLUTE);
    }
    match fs::metadata(&path) {
        Ok(meta) if meta.is_dir() => Ok(path),
        Ok(_) => Err(folder_error::NOT_A_FOLDER),
        Err(_) => Err(folder_error::MISSING),
    }
}

/// Why a chosen folder was refused; shown to the person as is.
pub mod folder_error {
    pub const INVALID: &str = "That is not a valid folder";
    pub const NOT_ABSOLUTE: &str = "The folder must be a full path";
    pub const NOT_A_FOLDER: &str = "That path is not a folder";
    pub const MISSING: &str = "That folder does not exist";
}

/// `<cache>/navegador/downloads`.
pub fn quarantine_dir(cache: &Path) -> PathBuf {
    cache.join("navegador").join("downloads")
}

/// Ids are ours (UUIDs); refusing anything else keeps a name from ever
/// climbing out of the quarantine directory.
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn part_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.part"))
}

pub fn pdf_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.pdf"))
}

/// What became of a quarantined download that ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A PDF that passed every check, renamed `<id>.pdf` in quarantine.
    Verified(Verified),
    /// Not a PDF, so not ours to keep: moved to the person's folder.
    Saved { path: PathBuf, size: u64 },
}

/// Check the finished download `<id>.part` in `dir` and, if it is a good PDF,
/// rename it `<id>.pdf`. On any failure the file is deleted.
pub fn finalize(dir: &Path, id: &str) -> Result<Verified, &'static str> {
    match finalize_or_release(dir, id, None, "")? {
        Outcome::Verified(verified) => Ok(verified),
        Outcome::Saved { .. } => unreachable!("nothing is released without a folder"),
    }
}

/// Like [`finalize`], but a file that is not a PDF is moved to `folder` under
/// `name` (made unique there) instead of being deleted, when there is a folder.
pub fn finalize_or_release(
    dir: &Path,
    id: &str,
    folder: Option<&Path>,
    name: &str,
) -> Result<Outcome, &'static str> {
    finalize_with_cap(dir, id, MAX_BYTES, folder, name)
}

fn finalize_with_cap(
    dir: &Path,
    id: &str,
    max: u64,
    folder: Option<&Path>,
    name: &str,
) -> Result<Outcome, &'static str> {
    if !valid_id(id) {
        return Err(reason::IO_ERROR);
    }
    let part = part_path(dir, id);
    let outcome = match verify_part(&part, max) {
        Ok(verified) => fs::rename(&part, pdf_path(dir, id))
            .map(|()| Outcome::Verified(verified))
            .map_err(|_| reason::IO_ERROR),
        Err(reason::NOT_PDF) if folder.is_some() => {
            release_to_folder(&part, folder.unwrap_or(dir), name)
                .map(|(path, size)| Outcome::Saved { path, size })
        }
        Err(why) => Err(why),
    };
    if outcome.is_err() {
        let _ = fs::remove_file(&part);
    }
    outcome
}

/// Move `part` into `folder` as `name`, or `name (1)`... if that is taken. The
/// name is claimed with an exclusive create first, so two files never end up
/// on the same path; nothing already there is touched. Across volumes (the
/// cache and the Downloads folder need not share one) the move is a copy and a
/// delete. On Windows a copy keeps the file's alternate data streams (that is
/// where the Mark-of-the-Web lives), a rename within a volume keeps them too.
fn release_to_folder(
    part: &Path,
    folder: &Path,
    name: &str,
) -> Result<(PathBuf, u64), &'static str> {
    use std::io::ErrorKind;

    let size = fs::metadata(part).map_err(|_| reason::IO_ERROR)?.len();
    let name = sanitize_name(name);
    for _ in 0..32 {
        let unique = unique_name(&name, |candidate| folder.join(candidate).exists());
        let target = folder.join(&unique);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(claimed) => {
                drop(claimed);
                let moved = fs::rename(part, &target)
                    .or_else(|_| fs::copy(part, &target).and_then(|_| fs::remove_file(part)));
                if moved.is_err() {
                    let _ = fs::remove_file(&target);
                    return Err(reason::IO_ERROR);
                }
                return Ok((target, size));
            }
            // Someone took the name between the check and the claim: next one.
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(reason::IO_ERROR),
        }
    }
    Err(reason::IO_ERROR)
}

fn verify_part(part: &Path, max: u64) -> Result<Verified, &'static str> {
    use sha2::{Digest, Sha256};

    let mut file = fs::File::open(part).map_err(|_| reason::IO_ERROR)?;
    let length = file.metadata().map_err(|_| reason::IO_ERROR)?.len();
    if length == 0 {
        return Err(reason::EMPTY);
    }
    // The type first: the size cap is for PDFs, and a big file that is not one
    // is the person's to keep, not "too large".
    let mut header = [0u8; PDF_MAGIC.len()];
    let mut got = 0;
    while got < header.len() {
        let read = file
            .read(&mut header[got..])
            .map_err(|_| reason::IO_ERROR)?;
        if read == 0 {
            break;
        }
        got += read;
    }
    if !is_pdf(&header[..got]) {
        return Err(reason::NOT_PDF);
    }
    if length > max {
        return Err(reason::TOO_LARGE);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| reason::IO_ERROR)?;
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
    /// Sanitized suggestion for a quarantined download; the final, unique name
    /// for one written to the person's folder.
    pub file_name: String,
    /// Where a download routed to the person's folder is being written.
    pub saved: Option<PathBuf>,
    /// When the download was requested: UTC, RFC 3339, this process's clock.
    pub accessed_at: String,
    /// The browser tab whose page started it, once [`Registry::set_tab`] says.
    pub tab: Option<u32>,
    /// The page that started it, as it was then (address and cleaned title).
    pub page_url: Option<String>,
    pub page_title: Option<String>,
}

/// Longest page title kept with a download, in characters.
pub const PAGE_TITLE_MAX: usize = 200;

/// The downloads in flight, in the order they started.
#[derive(Default)]
pub struct Registry {
    pending: Mutex<Vec<Pending>>,
}

impl Registry {
    /// Register a download of `url` whose suggested file is `suggested`, going
    /// to quarantine.
    pub fn begin(&self, url: &Url, suggested: &Path) -> Result<Pending, &'static str> {
        self.register(url, suggested, None, |_| false)
    }

    /// Register a download that goes to `folder` under a name no file there and
    /// no download in flight has. `exists` says whether a path is taken on disk.
    pub fn begin_in_folder(
        &self,
        url: &Url,
        suggested: &Path,
        folder: &Path,
        exists: impl Fn(&Path) -> bool,
    ) -> Result<Pending, &'static str> {
        self.register(url, suggested, Some(folder), exists)
    }

    fn register(
        &self,
        url: &Url,
        suggested: &Path,
        folder: Option<&Path>,
        exists: impl Fn(&Path) -> bool,
    ) -> Result<Pending, &'static str> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.len() >= MAX_IN_FLIGHT {
            return Err(reason::TOO_MANY);
        }
        let mut file_name = sanitize_name(&suggested.to_string_lossy());
        let mut saved = None;
        if let Some(folder) = folder {
            file_name = unique_name(&file_name, |candidate| {
                exists(&folder.join(candidate))
                    || pending.iter().any(|p| {
                        p.saved
                            .as_deref()
                            .and_then(Path::file_name)
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.eq_ignore_ascii_case(candidate))
                            && p.saved.as_deref().and_then(Path::parent) == Some(folder)
                    })
            });
            saved = Some(folder.join(&file_name));
        }
        let entry = Pending {
            id: uuid::Uuid::new_v4().to_string(),
            url: url.clone(),
            file_name,
            saved,
            accessed_at: super::capture::now_rfc3339(),
            tab: None,
            page_url: None,
            page_title: None,
        };
        pending.push(entry.clone());
        Ok(entry)
    }

    /// Record which tab started the download `id`. False when it is not in
    /// flight.
    pub fn set_tab(&self, id: &str, tab: Option<u32>) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        match pending.iter_mut().find(|p| p.id == id) {
            Some(entry) => {
                entry.tab = tab;
                true
            }
            None => false,
        }
    }

    /// Record the page that started download `id`, as it is right now. The
    /// title comes from a web page, so it is cleaned and bounded. False when the
    /// download is not in flight.
    pub fn set_page(&self, id: &str, url: Option<&str>, title: Option<&str>) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        match pending.iter_mut().find(|p| p.id == id) {
            Some(entry) => {
                entry.page_url = url.map(str::to_string);
                entry.page_title =
                    title.and_then(|t| super::capture::clean_line(t, PAGE_TITLE_MAX));
                true
            }
            None => false,
        }
    }

    /// A download in flight, as it is now, without taking it out.
    pub fn peek(&self, id: &str) -> Option<Pending> {
        let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.iter().find(|p| p.id == id).cloned()
    }

    /// The download whose file is `path`: its quarantine file, or the file it
    /// was routed to in the person's folder (matched on the file name alone:
    /// the directory the OS reports is not trusted, and acting on the file is
    /// left to the path this module stored).
    pub fn take_by_path(&self, path: &Path) -> Option<Pending> {
        let name = path.file_name()?.to_str()?;
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let at = pending.iter().position(|p| match &p.saved {
            None => format!("{}.part", p.id).eq_ignore_ascii_case(name),
            Some(saved) => saved
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(name)),
        })?;
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
            finalize_with_cap(dir.path(), "id1", 10, None, ""),
            Err(reason::TOO_LARGE)
        );
        assert!(!part_path(dir.path(), "id1").exists());
    }

    #[test]
    fn a_file_exactly_at_the_cap_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let body = b"%PDF-1.7 exactly";
        write_part(dir.path(), "id1", body);
        assert!(finalize_with_cap(dir.path(), "id1", body.len() as u64, None, "").is_ok());
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

    // --- which tab started it -------------------------------------------------

    #[test]
    fn a_download_belongs_to_no_tab_until_one_is_set() {
        let registry = Registry::default();
        let pending = registry
            .begin(&url("https://a.test/x"), Path::new("x"))
            .unwrap();
        assert_eq!(pending.tab, None);
        assert_eq!(
            registry.take_by_url(&url("https://a.test/x")).unwrap().tab,
            None
        );
    }

    #[test]
    fn the_tab_set_on_a_download_comes_back_when_it_ends_however_it_is_matched() {
        let registry = Registry::default();
        let by_url = registry
            .begin(&url("https://a.test/1"), Path::new("a"))
            .unwrap();
        let by_path = registry
            .begin(&url("https://a.test/2"), Path::new("b"))
            .unwrap();
        assert!(registry.set_tab(&by_url.id, Some(3)));
        assert!(registry.set_tab(&by_path.id, Some(4)));
        let part = PathBuf::from(format!("{}.part", by_path.id));
        assert_eq!(registry.take_by_path(&part).unwrap().tab, Some(4));
        assert_eq!(
            registry.take_by_url(&url("https://a.test/1")).unwrap().tab,
            Some(3)
        );
    }

    #[test]
    fn setting_the_tab_of_an_unknown_download_changes_nothing() {
        let registry = Registry::default();
        let pending = registry
            .begin(&url("https://a.test/1"), Path::new("a"))
            .unwrap();
        assert!(!registry.set_tab("no-such-id", Some(1)));
        assert_eq!(registry.take_by_url(&pending.url).unwrap().tab, None);
    }

    #[test]
    fn every_kind_of_draft_keeps_the_tab_of_its_download() {
        let registry = Registry::default();
        let pending = registry
            .begin(&url("https://a.test/p.pdf"), Path::new("p.pdf"))
            .unwrap();
        registry.set_tab(&pending.id, Some(2));
        let pending = registry.take_by_url(&pending.url).unwrap();
        assert_eq!(DownloadDraft::started(&pending).tab, Some(2));
        assert_eq!(
            DownloadDraft::finished(&pending, Err(reason::INTERRUPTED)).tab,
            Some(2)
        );
        let verified = Verified {
            size: 3,
            sha256: "a".repeat(64),
        };
        assert_eq!(DownloadDraft::finished(&pending, Ok(verified)).tab, Some(2));
        let saved = DownloadDraft::saved(&pending, Path::new("/d/x.zip"), Some(1));
        assert_eq!(saved.tab, Some(2));
    }

    #[test]
    fn a_refused_download_can_say_which_tab_asked() {
        let refused =
            DownloadDraft::refused(&url("https://a.test/x"), Path::new("x"), reason::BLOCKED);
        assert_eq!(refused.tab, None);
        assert_eq!(refused.with_tab(Some(4)).tab, Some(4));
    }

    // --- the page it came from, as it was when the download started ------------

    #[test]
    fn the_page_a_download_came_from_is_kept_as_it_was_and_comes_back_when_it_ends() {
        let registry = Registry::default();
        let a = registry
            .begin(&url("https://a.test/1.pdf"), Path::new("1.pdf"))
            .unwrap();
        let b = registry
            .begin(&url("https://a.test/2.pdf"), Path::new("2.pdf"))
            .unwrap();
        assert!(registry.set_page(&a.id, Some("https://a.test/one"), Some("Article one")));
        assert!(registry.set_page(&b.id, Some("https://a.test/two"), Some("Article two")));
        let b = registry.take_by_url(&url("https://a.test/2.pdf")).unwrap();
        assert_eq!(b.page_title.as_deref(), Some("Article two"));
        assert_eq!(b.page_url.as_deref(), Some("https://a.test/two"));
        let a = registry
            .take_by_path(&PathBuf::from(format!("{}.part", a.id)))
            .unwrap();
        assert_eq!(a.page_title.as_deref(), Some("Article one"));
    }

    #[test]
    fn the_page_title_is_cleaned_and_bounded_like_any_text_from_a_page() {
        let registry = Registry::default();
        let p = registry
            .begin(&url("https://a.test/x"), Path::new("x"))
            .unwrap();
        let hostile = format!("  Bad\u{202e}\n\ttitle {}", "w".repeat(500));
        registry.set_page(&p.id, None, Some(&hostile));
        let title = registry
            .take_by_url(&p.url)
            .unwrap()
            .page_title
            .expect("a title");
        assert!(title.starts_with("Bad title w"), "{title}");
        assert!(!title.contains('\u{202e}') && !title.contains('\n'));
        assert!(title.chars().count() <= PAGE_TITLE_MAX);
    }

    #[test]
    fn a_blank_title_or_an_unknown_download_leaves_nothing_behind() {
        let registry = Registry::default();
        let p = registry
            .begin(&url("https://a.test/x"), Path::new("x"))
            .unwrap();
        assert!(!registry.set_page("nope", Some("https://a.test/"), Some("T")));
        registry.set_page(&p.id, Some("https://a.test/"), Some("   \u{202e} "));
        let p = registry.take_by_url(&p.url).unwrap();
        assert_eq!(p.page_title, None);
    }

    #[test]
    fn every_draft_keeps_the_page_it_came_from() {
        let registry = Registry::default();
        let p = registry
            .begin(&url("https://a.test/p.pdf"), Path::new("p.pdf"))
            .unwrap();
        registry.set_page(&p.id, Some("https://a.test/one"), Some("Article one"));
        let p = registry.take_by_url(&p.url).unwrap();
        let verified = Verified {
            size: 3,
            sha256: "a".repeat(64),
        };
        let drafts = [
            DownloadDraft::started(&p),
            DownloadDraft::finished(&p, Ok(verified)),
            DownloadDraft::finished(&p, Err(reason::INTERRUPTED)),
            DownloadDraft::saved(&p, Path::new("/d/x.zip"), Some(1)),
        ];
        for draft in drafts {
            assert_eq!(draft.page_title.as_deref(), Some("Article one"));
            assert_eq!(draft.page_url.as_deref(), Some("https://a.test/one"));
        }
    }

    #[test]
    fn a_refused_download_can_carry_the_page_too() {
        let refused =
            DownloadDraft::refused(&url("https://a.test/x"), Path::new("x"), reason::BLOCKED);
        assert_eq!(
            (refused.page_url.clone(), refused.page_title.clone()),
            (None, None)
        );
        let refused = refused.with_page(Some("https://a.test/one"), Some("Article one"));
        assert_eq!(refused.page_title.as_deref(), Some("Article one"));
        assert_eq!(refused.page_url.as_deref(), Some("https://a.test/one"));
    }

    #[test]
    fn peeking_at_a_download_shows_its_current_state_and_leaves_it_in_flight() {
        let registry = Registry::default();
        let p = registry
            .begin(&url("https://a.test/x"), Path::new("x"))
            .unwrap();
        registry.set_tab(&p.id, Some(3));
        registry.set_page(&p.id, Some("https://a.test/one"), Some("One"));
        let seen = registry.peek(&p.id).unwrap();
        assert_eq!(seen.tab, Some(3));
        assert_eq!(seen.page_title.as_deref(), Some("One"));
        assert!(registry.take_by_url(&p.url).is_some(), "still in flight");
        assert!(registry.peek("nope").is_none());
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
            saved_to: None,
            accessed_at: "2026-09-30T12:00:00Z".into(),
            status: DownloadStatus::Ready,
            reason: None,
            tab: Some(2),
            page_url: Some("https://a.test/one".into()),
            page_title: Some("Article one".into()),
            already_saved_in: Some("src-1".into()),
        };
        let value = serde_json::to_value(draft).unwrap();
        let object = value.as_object().unwrap();
        for key in [
            "id",
            "url",
            "fileName",
            "size",
            "sha256",
            "savedTo",
            "accessedAt",
            "status",
            "reason",
            "tab",
            "pageUrl",
            "pageTitle",
            "alreadySavedIn",
        ] {
            assert!(object.contains_key(key), "missing {key}");
        }
        assert_eq!(object["tab"], 2);
        assert_eq!(object["alreadySavedIn"], "src-1");
        assert_eq!(
            object.len(),
            13,
            "an unexpected field would leak: {object:?}"
        );
        assert_eq!(object["status"], "ready");
        let rejected = serde_json::to_value(DownloadStatus::Rejected).unwrap();
        assert_eq!(rejected, "rejected");
    }

    #[test]
    fn a_draft_says_which_source_already_holds_its_file_only_when_told() {
        let pending = Pending {
            id: "id1".into(),
            url: Url::parse("https://a.test/x.pdf").unwrap(),
            file_name: "x.pdf".into(),
            saved: None,
            accessed_at: "2026-09-30T12:00:00Z".into(),
            tab: None,
            page_url: None,
            page_title: None,
        };
        let verified = Verified {
            size: 10,
            sha256: "ab".repeat(32),
        };

        let plain = DownloadDraft::finished(&pending, Ok(verified.clone()));
        let known = DownloadDraft::finished(&pending, Ok(verified)).with_already_saved_in("src-1");

        assert_eq!(plain.already_saved_in, None);
        assert_eq!(known.already_saved_in.as_deref(), Some("src-1"));
        assert_eq!(known.status, DownloadStatus::Ready);
    }

    // --- routing ------------------------------------------------------------

    #[test]
    fn a_pdf_name_goes_to_quarantine() {
        for name in ["paper.pdf", "Paper.PDF", "report.v2.pdf"] {
            assert_eq!(
                route_for(name, &url("https://a.test/whatever.zip")),
                Route::Quarantine,
                "{name}"
            );
        }
    }

    #[test]
    fn any_other_extension_goes_to_the_person_s_folder() {
        for name in [
            "data.zip",
            "photo.JPG",
            "archive.tar.gz",
            "setup.exe",
            "notes.txt",
            "song.mp3",
            "sheet.xlsx",
            "page.html",
        ] {
            assert_eq!(
                route_for(name, &url("https://a.test/paper.pdf")),
                Route::UserFolder,
                "{name}: the file name wins over the address"
            );
        }
    }

    #[test]
    fn a_name_that_says_nothing_falls_back_to_the_address() {
        // No extension in the name: the address decides.
        assert_eq!(
            route_for("download", &url("https://a.test/files/paper.pdf?dl=1")),
            Route::Quarantine
        );
        assert_eq!(
            route_for("download", &url("https://a.test/files/backup.zip")),
            Route::UserFolder
        );
        // A generated page: same.
        assert_eq!(
            route_for("get.php", &url("https://a.test/dl/file.zip")),
            Route::UserFolder
        );
    }

    #[test]
    fn a_name_and_an_address_that_say_nothing_are_checked_in_quarantine() {
        for (name, address) in [
            ("download", "https://a.test/"),
            ("download", "https://a.test/get"),
            ("get.php", "https://a.test/get.php?id=3"),
            ("blob", "https://a.test/x.aspx"),
            ("data.bin", "https://a.test/stream"),
            (".hidden", "https://a.test/"),
            ("", "https://a.test/x"),
        ] {
            assert_eq!(
                route_for(name, &url(address)),
                Route::Quarantine,
                "{name} {address}"
            );
        }
    }

    #[test]
    fn a_blob_address_has_no_file_name_of_its_own() {
        assert_eq!(
            route_for(
                "download",
                &url("blob:https://github.com/8f6d3c1e-2b0a-4c53-9a44-0d1f5e2b7c19")
            ),
            Route::Quarantine
        );
        assert_eq!(
            route_for(
                "file.pdf",
                &url("blob:https://github.com/8f6d3c1e-2b0a-4c53-9a44-0d1f5e2b7c19")
            ),
            Route::Quarantine
        );
        assert_eq!(
            route_for(
                "file.zip",
                &url("blob:https://github.com/8f6d3c1e-2b0a-4c53-9a44-0d1f5e2b7c19")
            ),
            Route::UserFolder
        );
    }

    #[test]
    fn a_hostile_name_is_judged_after_it_is_cleaned() {
        // The directory part is gone before the extension is read.
        assert_eq!(
            route_for("..\\..\\evil.zip", &url("https://a.test/")),
            Route::UserFolder
        );
        assert_eq!(
            route_for("a/b/c.pdf", &url("https://a.test/")),
            Route::Quarantine
        );
    }

    // --- generic names --------------------------------------------------------

    #[test]
    fn sanitize_name_keeps_the_extension_and_adds_none() {
        assert_eq!(sanitize_name("data.zip"), "data.zip");
        assert_eq!(sanitize_name("archive.tar.gz"), "archive.tar.gz");
        assert_eq!(sanitize_name("photo"), "photo");
        assert_eq!(
            sanitize_name("YOLO-object-detection-master.zip"),
            "YOLO-object-detection-master.zip"
        );
        assert_eq!(sanitize_name("Informe año.PDF"), "Informe año.PDF");
    }

    #[test]
    fn sanitize_name_is_as_strict_as_the_pdf_one() {
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_name("C:\\Users\\x\\a.zip"), "a.zip");
        assert_eq!(sanitize_name("a<b>c:d\"e|f?g*h.zip"), "a_b_c_d_e_f_g_h.zip");
        assert_eq!(sanitize_name("gpj.\u{202e}fdp"), "gpj.fdp");
        assert_eq!(sanitize_name("  spaced .zip "), "spaced .zip");
        assert_eq!(sanitize_name("...hidden.zip"), "hidden.zip");
        for name in ["CON", "nul.txt", "COM1.zip", "lpt9"] {
            assert!(sanitize_name(name).starts_with('_'), "{name}");
        }
        assert_eq!(sanitize_name("console.zip"), "console.zip");
        for name in ["", "   ", "..", ".", "...", "/", "\\"] {
            assert_eq!(sanitize_name(name), "download", "input: {name:?}");
        }
    }

    #[test]
    fn a_long_generic_name_is_cut_and_keeps_its_extension() {
        let clean = sanitize_name(&format!("{}.zip", "a".repeat(400)));
        assert_eq!(clean.chars().count(), 120);
        assert!(clean.ends_with(".zip"));
        let wide = sanitize_name(&format!("{}.tar.gz", "ñ".repeat(400)));
        assert_eq!(wide.chars().count(), 120);
        assert!(wide.ends_with(".gz"));
        // Nothing that looks like an extension: cut plainly.
        let plain = sanitize_name(&"b".repeat(400));
        assert_eq!(plain.chars().count(), 120);
    }

    #[test]
    fn a_cut_name_never_ends_in_a_dot_or_space_before_its_extension() {
        let raw = format!("{}. b.zip", "a".repeat(115));
        let clean = sanitize_name(&raw);
        let stem = clean.strip_suffix(".zip").unwrap();
        assert!(!stem.ends_with('.') && !stem.ends_with(' '), "{clean:?}");
    }

    #[test]
    fn a_verified_pdf_name_is_the_only_one_that_gets_pdf_added() {
        assert_eq!(sanitize_file_name("report.zip"), "report.zip.pdf");
        assert_eq!(sanitize_name("report.zip"), "report.zip");
    }

    // --- unique names -----------------------------------------------------------

    #[test]
    fn a_free_name_is_kept() {
        assert_eq!(unique_name("a.zip", |_| false), "a.zip");
    }

    #[test]
    fn a_taken_name_gets_a_number_before_the_extension() {
        let taken = ["a.zip", "a (1).zip"];
        assert_eq!(unique_name("a.zip", |n| taken.contains(&n)), "a (2).zip");
        assert_eq!(unique_name("notes", |n| n == "notes"), "notes (1)");
        assert_eq!(unique_name("x.tar.gz", |n| n == "x.tar.gz"), "x.tar (1).gz");
    }

    #[test]
    fn a_numbered_name_stays_within_the_length_limit() {
        let long = sanitize_name(&format!("{}.zip", "a".repeat(400)));
        let unique = unique_name(&long, |n| n == long);
        assert!(unique.chars().count() <= 120, "{unique}");
        assert!(unique.ends_with(" (1).zip"), "{unique}");
    }

    #[test]
    fn counting_gives_up_on_a_name_taken_ten_thousand_times() {
        let unique = unique_name("a.zip", |n| n == "a.zip" || n.contains(" ("));
        // Everything numbered is "taken", so it falls back to something unique.
        assert!(unique.starts_with("a ("), "{unique}");
    }

    // --- folders --------------------------------------------------------------------

    #[test]
    fn the_chosen_folder_wins_while_it_is_a_folder() {
        let chosen = PathBuf::from("D:/Docs");
        let default = PathBuf::from("C:/Users/x/Downloads");
        assert_eq!(
            resolve_folder(Some(&chosen), Some(default.clone()), |_| true),
            Some(chosen.clone())
        );
        // It was deleted since: the default.
        assert_eq!(
            resolve_folder(Some(&chosen), Some(default.clone()), |p| p == default),
            Some(default.clone())
        );
        assert_eq!(
            resolve_folder(None, Some(default.clone()), |_| true),
            Some(default.clone())
        );
        assert_eq!(resolve_folder(None, Some(default), |_| false), None);
        assert_eq!(resolve_folder(None, None, |_| true), None);
    }

    #[test]
    fn a_folder_must_exist_and_be_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file.txt");
        fs::write(&file, b"x").unwrap();
        assert_eq!(
            validate_folder(&dir.path().to_string_lossy()),
            Ok(dir.path().to_path_buf())
        );
        assert_eq!(
            validate_folder(&format!("  {}  ", dir.path().to_string_lossy())),
            Ok(dir.path().to_path_buf())
        );
        assert_eq!(
            validate_folder(&file.to_string_lossy()),
            Err(folder_error::NOT_A_FOLDER)
        );
        assert_eq!(
            validate_folder(&dir.path().join("gone").to_string_lossy()),
            Err(folder_error::MISSING)
        );
    }

    #[test]
    fn a_folder_must_be_a_full_path() {
        for input in ["", "   ", "Downloads", "..", "./x", "x/y"] {
            assert!(validate_folder(input).is_err(), "{input:?}");
        }
        assert_eq!(validate_folder("bad\u{0}path"), Err(folder_error::INVALID));
        assert_eq!(
            validate_folder("Downloads"),
            Err(folder_error::NOT_ABSOLUTE)
        );
    }

    // --- finalize or release ---------------------------------------------------------

    struct Dirs {
        quarantine: tempfile::TempDir,
        folder: tempfile::TempDir,
    }

    fn dirs() -> Dirs {
        Dirs {
            quarantine: tempfile::tempdir().unwrap(),
            folder: tempfile::tempdir().unwrap(),
        }
    }

    #[test]
    fn a_pdf_is_verified_even_when_there_is_a_folder() {
        let d = dirs();
        write_part(d.quarantine.path(), "id1", b"%PDF-1.7\n%%EOF\n");
        let outcome =
            finalize_or_release(d.quarantine.path(), "id1", Some(d.folder.path()), "x.pdf");
        assert!(matches!(outcome, Ok(Outcome::Verified(_))), "{outcome:?}");
        assert!(pdf_path(d.quarantine.path(), "id1").exists());
        assert_eq!(fs::read_dir(d.folder.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_zip_is_moved_to_the_folder_not_deleted() {
        let d = dirs();
        let body = b"PK\x03\x04 a zip file";
        write_part(d.quarantine.path(), "id1", body);
        let outcome = finalize_or_release(
            d.quarantine.path(),
            "id1",
            Some(d.folder.path()),
            "YOLO-object-detection-master.zip",
        )
        .unwrap();
        let Outcome::Saved { path, size } = outcome else {
            panic!("expected Saved");
        };
        assert_eq!(
            path,
            d.folder.path().join("YOLO-object-detection-master.zip")
        );
        assert_eq!(size, body.len() as u64);
        assert_eq!(fs::read(&path).unwrap(), body);
        assert!(!part_path(d.quarantine.path(), "id1").exists());
        assert!(!pdf_path(d.quarantine.path(), "id1").exists());
    }

    #[test]
    fn a_moved_file_never_overwrites_one_that_is_there() {
        let d = dirs();
        fs::write(d.folder.path().join("data.zip"), b"mine").unwrap();
        fs::write(d.folder.path().join("data (1).zip"), b"mine too").unwrap();
        write_part(d.quarantine.path(), "id1", b"<html>not a pdf</html>");
        let Outcome::Saved { path, .. } = finalize_or_release(
            d.quarantine.path(),
            "id1",
            Some(d.folder.path()),
            "data.zip",
        )
        .unwrap() else {
            panic!("expected Saved");
        };
        assert_eq!(path, d.folder.path().join("data (2).zip"));
        assert_eq!(fs::read(d.folder.path().join("data.zip")).unwrap(), b"mine");
        assert_eq!(
            fs::read(d.folder.path().join("data (1).zip")).unwrap(),
            b"mine too"
        );
    }

    #[test]
    fn a_name_from_the_page_cannot_climb_out_of_the_folder() {
        let d = dirs();
        write_part(d.quarantine.path(), "id1", b"not a pdf");
        let Outcome::Saved { path, .. } = finalize_or_release(
            d.quarantine.path(),
            "id1",
            Some(d.folder.path()),
            "..\\..\\evil.exe",
        )
        .unwrap() else {
            panic!("expected Saved");
        };
        assert_eq!(path.parent().unwrap(), d.folder.path());
        assert_eq!(path.file_name().unwrap(), "evil.exe");
    }

    #[test]
    fn a_big_file_that_is_not_a_pdf_is_saved_not_too_large() {
        let d = dirs();
        write_part(
            d.quarantine.path(),
            "id1",
            b"PK\x03\x04 and a lot more bytes",
        );
        let outcome = finalize_with_cap(
            d.quarantine.path(),
            "id1",
            10,
            Some(d.folder.path()),
            "big.zip",
        );
        assert!(matches!(outcome, Ok(Outcome::Saved { .. })), "{outcome:?}");
        assert!(d.folder.path().join("big.zip").exists());
    }

    #[test]
    fn a_big_pdf_is_still_too_large_and_deleted() {
        let d = dirs();
        write_part(
            d.quarantine.path(),
            "id1",
            b"%PDF-1.7 and then some more bytes",
        );
        let outcome = finalize_with_cap(
            d.quarantine.path(),
            "id1",
            10,
            Some(d.folder.path()),
            "big.pdf",
        );
        assert_eq!(outcome, Err(reason::TOO_LARGE));
        assert!(!part_path(d.quarantine.path(), "id1").exists());
        assert_eq!(fs::read_dir(d.folder.path()).unwrap().count(), 0);
    }

    #[test]
    fn an_empty_file_is_deleted_not_saved() {
        let d = dirs();
        write_part(d.quarantine.path(), "id1", b"");
        assert_eq!(
            finalize_or_release(d.quarantine.path(), "id1", Some(d.folder.path()), "a.zip"),
            Err(reason::EMPTY)
        );
        assert_eq!(fs::read_dir(d.folder.path()).unwrap().count(), 0);
    }

    #[test]
    fn without_a_folder_a_file_that_is_not_a_pdf_is_still_refused_and_deleted() {
        let d = dirs();
        write_part(d.quarantine.path(), "id1", b"PK\x03\x04");
        assert_eq!(
            finalize_or_release(d.quarantine.path(), "id1", None, "a.zip"),
            Err(reason::NOT_PDF)
        );
        assert!(!part_path(d.quarantine.path(), "id1").exists());
    }

    #[test]
    fn a_folder_that_vanished_fails_and_leaves_no_part_behind() {
        let d = dirs();
        write_part(d.quarantine.path(), "id1", b"PK\x03\x04");
        let gone = d.folder.path().join("gone");
        assert_eq!(
            finalize_or_release(d.quarantine.path(), "id1", Some(&gone), "a.zip"),
            Err(reason::IO_ERROR)
        );
        assert!(!part_path(d.quarantine.path(), "id1").exists());
    }

    #[test]
    fn a_hostile_id_is_not_released_either() {
        let d = dirs();
        let outside = d.quarantine.path().join("outside.part");
        fs::write(&outside, b"not a pdf").unwrap();
        let inner = d.quarantine.path().join("q");
        fs::create_dir(&inner).unwrap();
        assert_eq!(
            finalize_or_release(&inner, "../outside", Some(d.folder.path()), "a.zip"),
            Err(reason::IO_ERROR)
        );
        assert!(outside.exists());
        assert_eq!(fs::read_dir(d.folder.path()).unwrap().count(), 0);
    }

    // --- registry, download folder -----------------------------------------------------

    #[test]
    fn a_download_to_the_folder_gets_its_final_unique_name() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("data.zip"), b"x").unwrap();
        let registry = Registry::default();
        let pending = registry
            .begin_in_folder(
                &url("https://a.test/data.zip"),
                Path::new("C:\\Users\\x\\Downloads\\data.zip"),
                dir.path(),
                |p| p.exists(),
            )
            .unwrap();
        assert_eq!(pending.file_name, "data (1).zip");
        assert_eq!(pending.saved, Some(dir.path().join("data (1).zip")));
    }

    #[test]
    fn two_downloads_in_flight_never_share_a_name() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Registry::default();
        let begin = || {
            registry
                .begin_in_folder(
                    &url("https://a.test/a"),
                    Path::new("a.zip"),
                    dir.path(),
                    |p| p.exists(),
                )
                .unwrap()
        };
        let names: Vec<_> = (0..3).map(|_| begin().file_name).collect();
        assert_eq!(names, ["a.zip", "a (1).zip", "a (2).zip"]);
    }

    #[test]
    fn folder_downloads_count_against_the_same_limit() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Registry::default();
        for _ in 0..MAX_IN_FLIGHT {
            registry
                .begin_in_folder(
                    &url("https://a.test/a"),
                    Path::new("a.zip"),
                    dir.path(),
                    |_| false,
                )
                .unwrap();
        }
        assert_eq!(
            registry.begin_in_folder(
                &url("https://a.test/a"),
                Path::new("a.zip"),
                dir.path(),
                |_| false
            ),
            Err(reason::TOO_MANY)
        );
        assert_eq!(
            registry.begin(&url("https://a.test/a"), Path::new("a.pdf")),
            Err(reason::TOO_MANY)
        );
    }

    #[test]
    fn a_finished_folder_download_is_found_by_its_file_name_alone() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Registry::default();
        let pending = registry
            .begin_in_folder(
                &url("https://a.test/a"),
                Path::new("Data.ZIP"),
                dir.path(),
                |_| false,
            )
            .unwrap();
        // The OS reports another directory and another case: same file name.
        assert_eq!(
            registry.take_by_path(Path::new("Z:\\elsewhere\\data.zip")),
            Some(pending)
        );
    }

    #[test]
    fn a_quarantine_file_name_does_not_match_a_folder_download_or_the_reverse() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Registry::default();
        let quarantined = registry
            .begin(&url("https://a.test/a"), Path::new("a.pdf"))
            .unwrap();
        let saved = registry
            .begin_in_folder(
                &url("https://a.test/b"),
                Path::new("b.zip"),
                dir.path(),
                |_| false,
            )
            .unwrap();
        assert_eq!(
            registry.take_by_path(Path::new("b.zip")),
            Some(saved),
            "the folder download is found by the name it was given"
        );
        assert_eq!(registry.take_by_path(Path::new("a.pdf")), None);
        assert_eq!(
            registry.take_by_path(Path::new(&format!("{}.part", quarantined.id))),
            Some(quarantined)
        );
    }

    // --- drafts of the new kinds ----------------------------------------------------------

    #[test]
    fn a_rejected_zip_keeps_its_own_name_and_never_gets_pdf() {
        let zip = Registry::default()
            .begin(
                &url("https://a.test/x"),
                Path::new("YOLO-object-detection-master.zip"),
            )
            .unwrap();
        assert_eq!(zip.file_name, "YOLO-object-detection-master.zip");
        for outcome in [Err(reason::NOT_PDF), Err(reason::IO_ERROR)] {
            let draft = DownloadDraft::finished(&zip, outcome);
            assert_eq!(draft.file_name, "YOLO-object-detection-master.zip");
        }
        assert_eq!(
            DownloadDraft::started(&zip).file_name,
            "YOLO-object-detection-master.zip"
        );
    }

    #[test]
    fn a_verified_pdf_is_shown_with_pdf_even_if_the_server_named_it_otherwise() {
        let pending = Registry::default()
            .begin(&url("https://a.test/x"), Path::new("report"))
            .unwrap();
        let draft = DownloadDraft::finished(
            &pending,
            Ok(Verified {
                size: 5,
                sha256: "ab".repeat(32),
            }),
        );
        assert_eq!(draft.file_name, "report.pdf");
        let named = Registry::default()
            .begin(&url("https://a.test/x"), Path::new("paper.pdf"))
            .unwrap();
        let draft = DownloadDraft::finished(
            &named,
            Ok(Verified {
                size: 5,
                sha256: "ab".repeat(32),
            }),
        );
        assert_eq!(draft.file_name, "paper.pdf");
    }

    #[test]
    fn a_saved_file_says_where_and_carries_no_hash() {
        let dir = tempfile::tempdir().unwrap();
        let pending = Registry::default()
            .begin_in_folder(
                &url("https://a.test/x"),
                Path::new("data.zip"),
                dir.path(),
                |_| false,
            )
            .unwrap();
        let path = dir.path().join("data.zip");
        let draft = DownloadDraft::saved(&pending, &path, Some(2048));
        assert_eq!(draft.status, DownloadStatus::Saved);
        assert_eq!(draft.file_name, "data.zip");
        assert_eq!(
            draft.saved_to.as_deref(),
            Some(dir.path().to_string_lossy().as_ref())
        );
        assert_eq!(
            (draft.size, draft.sha256.clone(), draft.reason),
            (Some(2048), None, None)
        );
        assert_eq!(draft.accessed_at, pending.accessed_at);
        let json = serde_json::to_value(&draft).unwrap();
        assert_eq!(json["status"], "saved");
        assert!(json["savedTo"].is_string());
    }

    #[test]
    fn a_quarantined_draft_has_no_folder() {
        let p = pending();
        assert_eq!(DownloadDraft::started(&p).saved_to, None);
        assert_eq!(
            DownloadDraft::finished(&p, Err(reason::NOT_PDF)).saved_to,
            None
        );
    }
}
