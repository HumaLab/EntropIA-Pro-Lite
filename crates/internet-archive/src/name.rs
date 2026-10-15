//! Safe display name for a remote Internet Archive file.
//!
//! The importer concatenates this name into the stored path, so it must not
//! carry directories, Windows-invalid characters, or reserved device names.
//! The rules mirror `safeStem` in `apps/desktop/src/lib/navegador-copy.ts`.

/// Longest stored file name, including `.pdf`. Matches the web-capture helper.
const FILE_NAME_MAX: usize = 100;
const EXTENSION: &str = ".pdf";
const FALLBACK: &str = "PDF";

/// The name passed as `overrides.fileName`. Always ends in `.pdf`.
///
/// Uses the last segment of `remote_name` (Archive names often include
/// folders). When that segment sanitizes to empty, falls back to `identifier`,
/// then to `PDF`.
pub fn display_file_name(remote_name: &str, identifier: &str) -> String {
    let segment = remote_name.rsplit(['/', '\\']).next().unwrap_or("");
    let stem = strip_pdf_extension(&sanitize_stem(segment));
    let stem = if stem.is_empty() {
        let from_id = strip_pdf_extension(&sanitize_stem(identifier));
        if from_id.is_empty() {
            FALLBACK.to_string()
        } else {
            from_id
        }
    } else {
        stem
    };
    format!("{stem}{EXTENSION}")
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
        .take(FILE_NAME_MAX - EXTENSION.len() - 1)
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
}
