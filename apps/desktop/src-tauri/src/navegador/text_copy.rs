//! Copying a page or a selection capture into a collection.
//!
//! Those captures hold text and no file the corpus can open, so a copy is a PDF
//! rendered from the text ([`super::text_pdf`]): a provenance header and then
//! the page's readable text, or the exact quote with its context. The corpus
//! imports it like any PDF and can read its text layer without OCR.
//!
//! Everything is decided here, from the rows: the renderer names a capture and
//! never supplies a path, text or provenance. The PDF is written to a temporary
//! file under `<data>/web-captures/_copy/` (a name no source id can have, so the
//! startup sweep never takes it for a source folder); the import copies it into
//! `assets/` and it is not needed again. Files in that folder older than
//! [`STALE_AFTER`] are removed whenever a new one is rendered.
//!
//! A selection's sha256 is of the quote, so it is checked again before the quote
//! is vouched for (`file_changed`). A page's sha256 is of the HTML, which is not
//! what the copy shows; there the check is that the text it renders still exists
//! (`file_missing`, `no_text`).

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::capture_files;
use super::sources::{self, code, CopyTicket, WebCaptureProvenance};
use super::text_pdf::{self, Block, Document};

/// Folder, inside `<data>/web-captures/`, where rendered copies wait for the
/// import to take them.
pub const COPY_DIR: &str = "_copy";

/// A rendered copy this old is no longer waiting for an import.
pub const STALE_AFTER: Duration = Duration::from_secs(60 * 60);

/// `provenance.rendering` of a copy rendered from a capture's text.
pub const RENDERING: &str = "text-pdf";

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Row {
    source_id: String,
    original_url: String,
    final_url: String,
    source_title: Option<String>,
    capture_title: Option<String>,
    accessed_at: String,
    sha256: String,
    kind: String,
    text: Option<String>,
    text_rel_path: Option<String>,
    quote_prefix: Option<String>,
    quote_suffix: Option<String>,
}

fn read_row(conn: &Connection, capture_id: &str) -> Result<Row, String> {
    conn.query_row(
        "SELECT s.id, s.original_url, s.final_url, s.title, c.title, c.accessed_at, c.sha256,
                c.kind, c.text, c.text_rel_path, c.quote_prefix, c.quote_suffix
         FROM web_captures c JOIN web_sources s ON s.id = c.web_source_id
         WHERE c.id = ?1",
        [capture_id],
        |row| {
            Ok(Row {
                source_id: row.get(0)?,
                original_url: row.get(1)?,
                final_url: row.get(2)?,
                source_title: row.get(3)?,
                capture_title: row.get(4)?,
                accessed_at: row.get(5)?,
                sha256: row.get(6)?,
                kind: row.get(7)?,
                text: row.get(8)?,
                text_rel_path: row.get(9)?,
                quote_prefix: row.get(10)?,
                quote_suffix: row.get(11)?,
            })
        },
    )
    .map_err(|error| format!("{}: {error}", code::DB_ERROR))
}

fn missing() -> String {
    format!("{}: the captured text is not on disk", code::FILE_MISSING)
}

/// The capture's text as bytes: the row's, or its file's when it was too large
/// for the row.
fn text_bytes(row: &Row, data_dir: &Path) -> Result<Vec<u8>, String> {
    if let Some(text) = &row.text {
        return Ok(text.clone().into_bytes());
    }
    let Some(key) = &row.text_rel_path else {
        return Err(format!("{}: the capture holds no text", code::NO_TEXT));
    };
    let file = sources::capture_file(data_dir, &row.source_id, key).ok_or_else(missing)?;
    fs::read(file).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            missing()
        } else {
            format!("{}: {error}", code::DB_ERROR)
        }
    })
}

fn non_blank(text: Option<String>) -> Option<String> {
    text.filter(|text| !text.trim().is_empty())
}

/// The folder for rendered copies, created if need be. A link anywhere on the
/// way is refused: nothing is written through one.
fn copy_dir(data_dir: &Path) -> Result<std::path::PathBuf, String> {
    let root = capture_files::root(data_dir);
    let dir = root.join(COPY_DIR);
    fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", code::DB_ERROR))?;
    for folder in [&root, &dir] {
        let plain = fs::symlink_metadata(folder)
            .map(|meta| meta.is_dir() && !meta.file_type().is_symlink())
            .unwrap_or(false);
        if !plain {
            return Err(format!(
                "{}: the captures folder is not a plain folder",
                code::DB_ERROR
            ));
        }
    }
    Ok(dir)
}

/// Remove rendered copies nobody is waiting for. Only regular `.pdf` files
/// directly in `dir`; anything that cannot be removed is left.
fn purge_stale(dir: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_pdf = path.extension().is_some_and(|ext| ext == "pdf");
        let Ok(meta) = entry.metadata() else { continue };
        let old = meta
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_AFTER);
        if is_pdf && meta.is_file() && old {
            let _ = fs::remove_file(path);
        }
    }
}

/// Everything a copy of the page or selection capture `capture_id` needs: the
/// PDF rendered from its text, written to a temporary file, and the provenance
/// read from the rows. Errors use the codes of [`sources::code`], plus `no_text`.
pub fn copy_ticket(
    conn: &Connection,
    data_dir: &Path,
    capture_id: &str,
) -> Result<CopyTicket, String> {
    let row = read_row(conn, capture_id)?;
    let selection = match row.kind.as_str() {
        "page" => false,
        "selection" => true,
        _ => return Err(format!("{}: the capture is not a PDF", code::NOT_A_PDF)),
    };

    let bytes = text_bytes(&row, data_dir)?;
    if selection {
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != row.sha256 {
            return Err(format!(
                "{}: the saved quote is not the one that was verified",
                code::FILE_CHANGED
            ));
        }
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if text.trim().is_empty() {
        return Err(format!("{}: the capture holds no text", code::NO_TEXT));
    }

    let page_title = non_blank(row.capture_title.clone()).or(non_blank(row.source_title.clone()));
    let title = page_title.clone().unwrap_or_else(|| row.final_url.clone());
    let mut meta = vec![
        (
            "Tipo de captura".to_string(),
            if selection {
                "Selección (cita textual)"
            } else {
                "Página (texto legible)"
            }
            .to_string(),
        ),
        ("URL".to_string(), row.final_url.clone()),
    ];
    if row.original_url != row.final_url {
        meta.push(("URL original".to_string(), row.original_url.clone()));
    }
    meta.push(("Consultada (UTC)".to_string(), row.accessed_at.clone()));
    meta.push((
        if selection {
            "SHA-256 (cita)"
        } else {
            "SHA-256 (HTML)"
        }
        .to_string(),
        row.sha256.clone(),
    ));
    meta.push((
        "Copia".to_string(),
        "Texto de la captura generado por EntropIA; no reproduce el diseño ni las imágenes \
         de la página."
            .to_string(),
    ));

    let body = if selection {
        let mut blocks = Vec::new();
        if let Some(before) = non_blank(row.quote_prefix.clone()) {
            blocks.push(Block::Context(before.trim_start().to_string()));
        }
        blocks.push(Block::Quote(text));
        if let Some(after) = non_blank(row.quote_suffix.clone()) {
            blocks.push(Block::Context(after.trim_end().to_string()));
        }
        blocks
    } else {
        vec![Block::Text(text)]
    };

    let rendered = text_pdf::render(&Document { title, meta, body })?;

    let dir = copy_dir(data_dir)?;
    purge_stale(&dir, SystemTime::now());
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|age| age.as_nanos())
        .unwrap_or(0);
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = dir.join(format!("{capture_id}-{nanos}-{serial}.pdf"));
    fs::write(&file, &rendered.bytes).map_err(|error| format!("{}: {error}", code::DB_ERROR))?;

    Ok(CopyTicket {
        path: file.to_string_lossy().into_owned(),
        provenance: WebCaptureProvenance {
            source_id: row.source_id,
            capture_id: capture_id.to_string(),
            original_url: row.original_url,
            final_url: row.final_url,
            page_title,
            accessed_at: row.accessed_at,
            sha256: row.sha256,
            capture_kind: Some(row.kind),
            rendering: Some(RENDERING.to_string()),
        },
    })
}
