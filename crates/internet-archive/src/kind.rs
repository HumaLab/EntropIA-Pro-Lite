//! File kinds a user can ask the importer for.
//!
//! The kinds are a filter, not a format list: `Pdf` means any PDF the item
//! offers, `PdfText` only PDFs Internet Archive publishes as `Text PDF` (with
//! an OCR text layer), and `Image` the still-image formats EntropIA can
//! import (JPEG, PNG, TIFF).

/// One user-facing file kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileKind {
    /// Any PDF.
    Pdf,
    /// Only PDFs published as `Text PDF` (OCR text layer).
    PdfText,
    /// JPEG, PNG or TIFF images.
    Image,
}

impl FileKind {
    /// The kind of a PDF-ish file from its Internet Archive `format`:
    /// `Text PDF` (any case) is [`FileKind::PdfText`], every other PDF is
    /// [`FileKind::Pdf`].
    pub fn of_pdf_format(format: Option<&str>) -> FileKind {
        match format {
            Some(f) if f.trim().eq_ignore_ascii_case("Text PDF") => FileKind::PdfText,
            _ => FileKind::Pdf,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FileKind;

    #[test]
    fn only_text_pdf_format_is_pdf_text() {
        assert_eq!(FileKind::of_pdf_format(Some("Text PDF")), FileKind::PdfText);
        assert_eq!(
            FileKind::of_pdf_format(Some(" text pdf ")),
            FileKind::PdfText
        );
        assert_eq!(
            FileKind::of_pdf_format(Some("Additional Text PDF")),
            FileKind::Pdf
        );
        assert_eq!(
            FileKind::of_pdf_format(Some("Image Container PDF")),
            FileKind::Pdf
        );
        assert_eq!(FileKind::of_pdf_format(None), FileKind::Pdf);
    }
}
