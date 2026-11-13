//! File kinds a user can ask the importer for.
//!
//! The kinds are a filter, not a format list: `Pdf` means any PDF the item
//! offers, `PdfText` only PDFs Internet Archive publishes as `Text PDF` (with
//! an OCR text layer), and `Image` the still-image formats EntropIA can
//! import (JPEG, PNG, TIFF). GIF is left out: EntropIA does not import it.

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

    /// The kind of any listed file: PDF-ish files (see [`is_pdf_ish`]) keep
    /// their PDF kind, files with an importable image extension or format
    /// are [`FileKind::Image`], anything else falls back to [`FileKind::Pdf`].
    pub fn of_file(name: &str, format: Option<&str>) -> FileKind {
        if !is_pdf_ish(name, format) && image_extension(name, format).is_some() {
            FileKind::Image
        } else {
            FileKind::of_pdf_format(format)
        }
    }
}

/// A file is PDF-ish when `format` mentions `pdf` (any case) or the name
/// ends in `.pdf` or `.lcpdf` (any case).
pub(crate) fn is_pdf_ish(name: &str, format: Option<&str>) -> bool {
    if format.is_some_and(|f| f.to_ascii_lowercase().contains("pdf")) {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".pdf") || lower.ends_with(".lcpdf")
}

/// Normalized extension (`jpg`, `png`, `tif`) of an importable image. A
/// declared format decides alone and must be exactly `JPEG`, `PNG` or
/// `TIFF` (any case): `Item Tile` (`__ia_thumb.jpg`), `JPEG Thumb` or
/// `JPEG 2000` are not images EntropIA imports. Without a format, the
/// name's extension decides.
pub fn image_extension(name: &str, format: Option<&str>) -> Option<&'static str> {
    if let Some(format) = format.map(str::trim).filter(|f| !f.is_empty()) {
        return match format.to_ascii_lowercase().as_str() {
            "jpeg" => Some("jpg"),
            "png" => Some("png"),
            "tiff" => Some("tif"),
            _ => None,
        };
    }
    let lower = name.to_ascii_lowercase();
    match lower.rsplit_once('.').map(|(_, ext)| ext) {
        Some("jpg" | "jpeg") => Some("jpg"),
        Some("png") => Some("png"),
        Some("tif" | "tiff") => Some("tif"),
        _ => None,
    }
}

/// Staging file extension for a listed file: its image extension for
/// images, `pdf` for everything else. The extension decides how EntropIA
/// classifies the imported asset.
pub fn staging_extension(name: &str, format: Option<&str>) -> &'static str {
    match FileKind::of_file(name, format) {
        FileKind::Image => image_extension(name, format).unwrap_or("pdf"),
        FileKind::Pdf | FileKind::PdfText => "pdf",
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

    #[test]
    fn files_are_images_by_extension_or_format_and_pdfs_win() {
        assert_eq!(FileKind::of_file("a.jpg", Some("JPEG")), FileKind::Image);
        assert_eq!(FileKind::of_file("a.JPEG", None), FileKind::Image);
        assert_eq!(FileKind::of_file("a.png", None), FileKind::Image);
        assert_eq!(FileKind::of_file("a.tiff", None), FileKind::Image);
        assert_eq!(FileKind::of_file("scan", Some("TIFF")), FileKind::Image);
        assert_eq!(
            FileKind::of_file("a.pdf", Some("Text PDF")),
            FileKind::PdfText
        );
        assert_eq!(FileKind::of_file("a.pdf", Some("JPEG")), FileKind::Pdf);
        assert_eq!(FileKind::of_file("a.gif", Some("GIF")), FileKind::Pdf);
    }

    #[test]
    fn staging_extension_follows_the_kind() {
        assert_eq!(super::staging_extension("a.pdf", Some("Text PDF")), "pdf");
        assert_eq!(super::staging_extension("a.jpeg", None), "jpg");
        assert_eq!(super::staging_extension("a.PNG", None), "png");
        assert_eq!(super::staging_extension("a.tif", None), "tif");
        assert_eq!(super::staging_extension("scan", Some("JPEG")), "jpg");
        assert_eq!(super::image_extension("a.gif", Some("GIF")), None);
        assert_eq!(super::image_extension("a.jp2", Some("JPEG 2000")), None);
        assert_eq!(
            super::image_extension("__ia_thumb.jpg", Some("Item Tile")),
            None
        );
        assert_eq!(
            super::image_extension("a_thumb.jpg", Some("JPEG Thumb")),
            None
        );
        assert_eq!(super::image_extension("a.png", Some("JPEG")), Some("jpg"));
    }
}
