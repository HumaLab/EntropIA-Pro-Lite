//! Safe display name for a remote Internet Archive file.
//!
//! The importer concatenates this name into the stored path, so it must not
//! carry directories, Windows-invalid characters, or reserved device names.
//! The rules mirror `safeStem` in `apps/desktop/src/lib/navegador-copy.ts`.

/// Longest stored file name, including the extension. Matches the
/// web-capture helper.
const FILE_NAME_MAX: usize = 100;
/// Room kept for the longest extension (`.pdf`, `.jpg`, `.png`, `.tif`).
const EXTENSION_LEN: usize = 4;

/// The name passed as `overrides.fileName` for a PDF. Always ends in `.pdf`.
///
/// Uses the last segment of `remote_name` (Archive names often include
/// folders). When that segment sanitizes to empty, falls back to `identifier`,
/// then to `PDF`.
pub fn display_file_name(remote_name: &str, identifier: &str) -> String {
    display_file_name_for(remote_name, identifier, "pdf")
}

/// [`display_file_name`] for any staging `extension` (`pdf`, `jpg`, `png`,
/// `tif`): the remote extension is replaced by `.<extension>` and the last
/// fallback is `PDF` or `Image`.
pub fn display_file_name_for(remote_name: &str, identifier: &str, extension: &str) -> String {
    let segment = remote_name.rsplit(['/', '\\']).next().unwrap_or("");
    // PDFs strip only `.pdf`/`.lcpdf`, exactly as before images existed.
    let strip = |stem: &str| {
        if extension == "pdf" {
            strip_pdf_extension(stem)
        } else {
            strip_known_extension(stem)
        }
    };
    let stem = strip(&sanitize_stem(segment));
    let stem = if stem.is_empty() {
        let from_id = strip(&sanitize_stem(identifier));
        if from_id.is_empty() {
            if extension == "pdf" { "PDF" } else { "Image" }.to_string()
        } else {
            from_id
        }
    } else {
        stem
    };
    format!("{stem}.{extension}")
}

fn sanitize_stem(text: &str) -> String {
    let mut cleaned = String::new();
    for ch in text.chars() {
        if ch.is_control() || is_format(ch) {
            if ch.is_whitespace() {
                cleaned.push(' ');
            }
            continue;
        }
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            continue;
        }
        cleaned.push(ch);
    }
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|c: char| c.is_whitespace() || c == '.');
    let mut stem: String = trimmed
        .chars()
        .take(FILE_NAME_MAX - EXTENSION_LEN - 1)
        .collect();
    while stem.ends_with([' ', '.']) {
        stem.pop();
    }
    if stem.is_empty() {
        return String::new();
    }
    let device = stem.split('.').next().unwrap_or("");
    if is_reserved_device(device) {
        format!("_{stem}")
    } else {
        stem
    }
}

/// Strip a trailing `.pdf`/`.lcpdf` or importable image extension.
fn strip_known_extension(stem: &str) -> String {
    let lower = stem.to_ascii_lowercase();
    for ext in [".jpeg", ".jpg", ".png", ".tiff", ".tif"] {
        if lower.ends_with(ext) {
            let kept = stem[..stem.len() - ext.len()].trim_end_matches([' ', '.']);
            return kept.to_string();
        }
    }
    strip_pdf_extension(stem)
}

fn strip_pdf_extension(stem: &str) -> String {
    let lower = stem.to_ascii_lowercase();
    if lower.ends_with(".pdf") || lower.ends_with(".lcpdf") {
        let cut = if lower.ends_with(".lcpdf") { 6 } else { 4 };
        stem[..stem.len() - cut].to_string()
    } else {
        stem.to_string()
    }
}

fn is_format(ch: char) -> bool {
    matches!(ch, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}')
}

fn is_reserved_device(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_plain_name_and_adds_pdf() {
        assert_eq!(display_file_name("scans/vol1.pdf", "id"), "vol1.pdf");
    }

    #[test]
    fn strips_traversal_and_invalid_characters() {
        assert_eq!(display_file_name("a/../b<c>:\"d.pdf", "id"), "bcd.pdf");
        assert_eq!(display_file_name("notes?.pdf", "id"), "notes.pdf");
        assert_eq!(display_file_name("a\\b.pdf", "id"), "b.pdf");
    }

    #[test]
    fn reserved_windows_names_get_a_prefix() {
        assert_eq!(display_file_name("NUL.pdf", "id"), "_NUL.pdf");
        assert_eq!(display_file_name("con.txt", "id"), "_con.txt.pdf");
    }

    #[test]
    fn empty_segment_falls_back_to_the_identifier() {
        assert_eq!(display_file_name("scans/", "item-1"), "item-1.pdf");
        assert_eq!(display_file_name("...", ""), "PDF.pdf");
    }

    #[test]
    fn truncates_to_the_file_name_cap() {
        let long = "a".repeat(200);
        let name = display_file_name(&long, "id");
        assert!(name.ends_with(".pdf"));
        assert!(name.len() <= FILE_NAME_MAX);
    }

    #[test]
    fn drops_trailing_dots_and_spaces() {
        assert_eq!(display_file_name("  report... ", "id"), "report.pdf");
    }

    #[test]
    fn pdf_names_keep_image_looking_suffixes_as_before() {
        assert_eq!(display_file_name("scan.tif", "id"), "scan.tif.pdf");
        assert_eq!(display_file_name("photo.jpg", "id"), "photo.jpg.pdf");
        assert_eq!(display_file_name("???", "foo.png"), "foo.png.pdf");
    }

    #[test]
    fn images_keep_their_own_extension() {
        assert_eq!(
            display_file_name_for("photos/Plate 1.jpeg", "id", "jpg"),
            "Plate 1.jpg"
        );
        assert_eq!(display_file_name_for("map.TIF", "id", "tif"), "map.tif");
        assert_eq!(display_file_name_for("scan.png", "id", "png"), "scan.png");
        assert_eq!(display_file_name_for("???", "", "png"), "Image.png");
        assert_eq!(display_file_name_for("book.pdf", "id", "pdf"), "book.pdf");
    }
}
