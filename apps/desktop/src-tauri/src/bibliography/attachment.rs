//! Attachment file resolution (E4a-WU1): maps one cataloged Zotero
//! attachment to a local file or to an honest unavailability verdict.
//!
//! The resolver never guesses, never touches the network, and never
//! creates corpus assets: it only answers "read this path" or "here is
//! why there is nothing to read". Extraction (E4a-WU2) and selective OCR
//! (E4b) consume the verdict; the UI renders the reason verbatim.

use std::path::PathBuf;

/// The durable file reference of one `zotero_attachments` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRef {
    pub attachment_id: String,
    pub attachment_key: String,
    pub content_type: Option<String>,
    pub link_mode: Option<String>,
    pub native_path: Option<String>,
    pub filename: Option<String>,
    pub url: Option<String>,
    pub mtime: Option<i64>,
    pub native_version: Option<i64>,
}

/// Where one attachment resolves: a readable file, or a named reason with
/// no file. Reasons are stable machine strings; the UI renders them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentResolution {
    File(PathBuf),
    Unavailable {
        reason: &'static str,
        detail: String,
    },
}

/// Resolves one attachment against the filesystem. `zotero_data_dir` is
/// the optional user-configured Zotero profile directory used only for
/// stored copies (`storage/<key>/<filename>`); linked files resolve
/// through their own absolute path and never through the data dir.
pub fn resolve_attachment_file(
    attachment: &AttachmentRef,
    zotero_data_dir: Option<&str>,
) -> AttachmentResolution {
    // Ground truth first: an existing file wins for every link mode.
    if let Some(path) = attachment
        .native_path
        .as_deref()
        .filter(|path| !path.is_empty())
    {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return AttachmentResolution::File(candidate);
        }
        if matches!(attachment.link_mode.as_deref(), Some("linked_file")) {
            return AttachmentResolution::Unavailable {
                reason: "linked_file_missing",
                detail: format!("linked file moved or unreadable: {path}"),
            };
        }
    }
    match attachment.link_mode.as_deref().map(str::trim) {
        Some("imported_file") => {
            let Some(data_dir) = zotero_data_dir.filter(|dir| !dir.is_empty()) else {
                return AttachmentResolution::Unavailable {
                    reason: "data_dir_not_configured",
                    detail: "stored copies need the Zotero data directory setting".to_string(),
                };
            };
            let filename = attachment.filename.as_deref().unwrap_or_default();
            if attachment.attachment_key.is_empty() || filename.is_empty() {
                return AttachmentResolution::Unavailable {
                    reason: "no_file_reference",
                    detail: "stored copy has no key or filename".to_string(),
                };
            }
            let candidate = PathBuf::from(data_dir)
                .join("storage")
                .join(&attachment.attachment_key)
                .join(filename);
            if candidate.is_file() {
                return AttachmentResolution::File(candidate);
            }
            AttachmentResolution::Unavailable {
                reason: "stored_copy_missing",
                detail: format!("no stored copy at {}", candidate.display()),
            }
        }
        Some("linked_file") => AttachmentResolution::Unavailable {
            reason: "linked_file_missing",
            detail: "linked file has no readable path".to_string(),
        },
        Some("imported_url") | Some("linked_url") | Some(_) | None
            if attachment.native_path.as_deref().is_none_or(str::is_empty) =>
        {
            let empty = attachment.link_mode.is_none()
                && attachment.filename.as_deref().is_none_or(str::is_empty)
                && attachment.url.as_deref().is_none_or(str::is_empty);
            if empty {
                return AttachmentResolution::Unavailable {
                    reason: "no_file_reference",
                    detail: "the attachment catalogs no file reference".to_string(),
                };
            }
            AttachmentResolution::Unavailable {
                reason: "not_a_local_file",
                detail: format!(
                    "link mode {} is not a local file: E4a never fetches over the network",
                    attachment.link_mode.as_deref().unwrap_or("<unset>")
                ),
            }
        }
        _ => AttachmentResolution::Unavailable {
            reason: "not_a_local_file",
            detail: "no readable native path for this link mode".to_string(),
        },
    }
}

/// Reads one cataloged attachment into its file reference. `None` when
/// the attachment row does not exist.
pub fn attachment_ref_for(
    conn: &rusqlite::Connection,
    attachment_id: &str,
) -> Result<Option<AttachmentRef>, String> {
    conn.query_row(
        "SELECT id, attachment_key, content_type, link_mode, native_path, filename, url,
                mtime, native_version
         FROM zotero_attachments WHERE id = ?1",
        [attachment_id],
        |row| {
            Ok(AttachmentRef {
                attachment_id: row.get(0)?,
                attachment_key: row.get(1)?,
                content_type: row.get(2)?,
                link_mode: row.get(3)?,
                native_path: row.get(4)?,
                filename: row.get(5)?,
                url: row.get(6)?,
                mtime: row.get(7)?,
                native_version: row.get(8)?,
            })
        },
    )
    .map(Some)
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(format!(
            "Failed to read attachment {attachment_id}: {other}"
        )),
    })
}

/// Canonical, plain spelling of an existing path. Zotero stores
/// `native_path` with forward slashes (`C:/Users/...`); canonicalizing gives
/// the OS spelling, and the Windows verbatim prefix is dropped because the
/// asset URL and every viewer expect the ordinary form.
pub fn plain_canonical(path: &std::path::Path) -> std::io::Result<PathBuf> {
    let canonical = std::fs::canonicalize(path)?;
    let text = canonical.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return Ok(PathBuf::from(format!(r"\\{rest}")));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return Ok(PathBuf::from(rest));
    }
    Ok(canonical)
}

/// What the in-app viewer can show for one original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalKind {
    /// A PDF file, served to the webview through a one-file asset grant.
    Pdf,
    /// An HTML snapshot, shown from the text the catalog stores: no file.
    Html,
}

/// Whether the attachment is an HTML snapshot, by its catalog metadata alone.
pub fn is_html_snapshot(attachment: &AttachmentRef) -> bool {
    let typed = attachment
        .content_type
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    if let Some(value) = typed {
        return value.starts_with("text/html") || value.starts_with("application/xhtml");
    }
    attachment
        .filename
        .as_deref()
        .map(str::to_ascii_lowercase)
        .is_some_and(|name| name.ends_with(".html") || name.ends_with(".htm"))
}

/// Validates a resolved file before the app serves it: an existing regular
/// file, a PDF by catalog content type (or extension when untyped) and by its
/// `%PDF-` signature. Returns the canonical plain path to grant, or a
/// `(reason, detail)` pair the reader shows verbatim. Never opens a directory
/// and never launches anything.
pub fn validate_pdf_original(
    path: &std::path::Path,
    content_type: Option<&str>,
) -> Result<PathBuf, (String, String)> {
    let canonical = plain_canonical(path).map_err(|error| {
        (
            "original_unreadable".to_string(),
            format!("{}: {error}", path.display()),
        )
    })?;
    if !canonical.is_file() {
        return Err((
            "not_a_file".to_string(),
            format!("not a regular file: {}", canonical.display()),
        ));
    }
    let typed = content_type
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let pdf_by_type = match typed.as_deref() {
        Some(value) => value == "application/pdf",
        None => canonical
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf")),
    };
    if !pdf_by_type {
        return Err((
            "not_a_pdf".to_string(),
            format!("not a PDF by content type: {}", canonical.display()),
        ));
    }
    let mut head = [0u8; 5];
    let signature_ok = std::fs::File::open(&canonical)
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut head))
        .is_ok()
        && &head == b"%PDF-";
    if !signature_ok {
        return Err((
            "not_a_pdf".to_string(),
            format!("no PDF signature: {}", canonical.display()),
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_copy(dir: &tempfile::TempDir, key: &str, filename: &str, bytes: &[u8]) -> String {
        let path = dir.path().join("storage").join(key).join(filename);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("storage dirs");
        std::fs::write(&path, bytes).expect("stored copy");
        path.to_string_lossy().to_string()
    }

    fn attachment(link_mode: &str) -> AttachmentRef {
        AttachmentRef {
            attachment_id: "att-1".to_string(),
            attachment_key: "ABCDEF12".to_string(),
            content_type: Some("application/pdf".to_string()),
            mtime: Some(1_700_000_000),
            native_version: Some(3),
            link_mode: Some(link_mode.to_string()),
            native_path: None,
            filename: Some("paper.pdf".to_string()),
            url: None,
        }
    }

    #[test]
    fn existing_native_path_wins_regardless_of_link_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = stored_copy(&dir, "OTHERKEY", "paper.pdf", b"%PDF-1.4 fake");
        for mode in [
            "linked_file",
            "imported_file",
            "imported_url",
            "linked_url",
            "mystery",
        ] {
            let mut att = attachment(mode);
            att.native_path = Some(path.clone());
            assert_eq!(
                resolve_attachment_file(&att, None),
                AttachmentResolution::File(PathBuf::from(&path)),
                "an existing file is ground truth for link mode {mode}"
            );
        }
    }

    #[test]
    fn linked_file_resolves_through_its_own_path_or_reports_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("paper.pdf");
        std::fs::write(&path, b"%PDF-1.4 fake").expect("linked file");
        let mut att = attachment("linked_file");
        att.native_path = Some(path.to_string_lossy().to_string());
        assert_eq!(
            resolve_attachment_file(&att, None),
            AttachmentResolution::File(path.clone())
        );

        let mut missing = attachment("linked_file");
        missing.native_path = Some(dir.path().join("gone.pdf").to_string_lossy().to_string());
        assert!(
            matches!(
                resolve_attachment_file(&missing, None),
                AttachmentResolution::Unavailable {
                    reason: "linked_file_missing",
                    ..
                }
            ),
            "a moved linked file reports missing instead of guessing"
        );
    }

    #[test]
    fn stored_copies_resolve_under_the_configured_data_dir_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        stored_copy(&dir, "ABCDEF12", "paper.pdf", b"%PDF-1.4 fake");
        let data_dir = dir.path().to_string_lossy().to_string();

        let att = attachment("imported_file");
        assert_eq!(
            resolve_attachment_file(&att, Some(&data_dir)),
            AttachmentResolution::File(
                dir.path()
                    .join("storage")
                    .join("ABCDEF12")
                    .join("paper.pdf")
            ),
            "a stored copy resolves under storage/<key>/<filename>"
        );
        assert!(
            matches!(
                resolve_attachment_file(&att, None),
                AttachmentResolution::Unavailable {
                    reason: "data_dir_not_configured",
                    ..
                }
            ),
            "without a data dir there is nothing honest to try"
        );

        let mut no_copy = attachment("imported_file");
        no_copy.attachment_key = "NOPEKEY1".to_string();
        assert!(
            matches!(
                resolve_attachment_file(&no_copy, Some(&data_dir)),
                AttachmentResolution::Unavailable {
                    reason: "stored_copy_missing",
                    ..
                }
            ),
            "a missing stored copy reports missing instead of scanning"
        );
    }

    #[test]
    fn validate_pdf_original_refuses_directories_and_missing_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(validate_pdf_original(&dir.path().join("gone.pdf"), None).is_err());
        let error = validate_pdf_original(dir.path(), Some("application/pdf"))
            .expect_err("a directory is not an original");
        assert_eq!(error.0, "not_a_file");
    }

    #[test]
    fn urls_and_unknown_modes_are_not_files() {
        let mut url = attachment("linked_url");
        url.url = Some("https://example.invalid/paper".to_string());
        assert!(
            matches!(
                resolve_attachment_file(&url, None),
                AttachmentResolution::Unavailable {
                    reason: "not_a_local_file",
                    ..
                }
            ),
            "web links never resolve to a file: no network fetch in E4a"
        );
        let mut mystery = attachment("mystery-mode");
        mystery.native_path = None;
        assert!(
            matches!(
                resolve_attachment_file(&mystery, None),
                AttachmentResolution::Unavailable {
                    reason: "not_a_local_file",
                    ..
                }
            ),
            "unknown modes fail closed"
        );
        assert!(
            matches!(
                resolve_attachment_file(
                    &AttachmentRef {
                        attachment_id: "att-0".to_string(),
                        attachment_key: "".to_string(),
                        content_type: None,
                        mtime: None,
                        native_version: None,
                        link_mode: None,
                        native_path: None,
                        filename: None,
                        url: None,
                    },
                    None
                ),
                AttachmentResolution::Unavailable {
                    reason: "no_file_reference",
                    ..
                }
            ),
            "an empty reference fails closed"
        );
    }
}
