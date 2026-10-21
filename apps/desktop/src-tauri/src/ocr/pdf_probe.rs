use pdfium_render::prelude::{
    PdfPageObjectCommon, PdfPageObjectType, PdfPageObjectsCommon, Pdfium,
};
use serde::Serialize;

const TEXT_CHARS_THRESHOLD: usize = 40;
const IMAGE_DOMINANT_RATIO: f32 = 0.90;
const FULL_PAGE_IMAGE_RATIO: f32 = 0.95;
const DOCUMENT_DOMINANCE_RATIO: f32 = 0.80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PageKind {
    Native,
    ImageOnly,
    ImageWithOcr,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageProfile {
    pub page_number: u32,
    pub has_text: bool,
    pub text_chars: usize,
    pub image_count: usize,
    pub largest_image_ratio: f32,
    pub full_page_image_like: bool,
    pub kind: PageKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocumentProfile {
    pub pages: Vec<PageProfile>,
    pub native_pages: usize,
    pub image_only_pages: usize,
    pub image_with_ocr_pages: usize,
    pub uncertain_pages: usize,
    pub dominant_kind: PageKind,
    pub mixed: bool,
    pub should_render_as_images: bool,
}

/// Profile every page of a PDF document.
///
/// Pro's Pdfium binding is synchronous (no render actor), so this forwards to
/// `super::pdf::profile_pdf_sync`, which binds the engine via the same
/// `get_pdfium()` path used by `pdf_page_count` and `render_pdf_page_to_image`,
/// then runs the per-page profiling below with that engine. Pdfium work is
/// blocking; call this from a blocking-safe context (`spawn_blocking`).
pub fn profile_pdf_bytes(bytes: &[u8]) -> Result<DocumentProfile, String> {
    super::pdf::profile_pdf_sync(bytes)
}

/// Build the per-page profile with an already-bound Pdfium engine.
///
/// Ported verbatim from EntropIA-Lite's actor-driven profiler: the per-page
/// signals (quality text-char count, image count, largest image area ratio)
/// are identical — only the engine acquisition differs (synchronous in Pro).
pub(super) fn profile_pdf_with_engine(
    pdfium: &Pdfium,
    bytes: &[u8],
) -> Result<DocumentProfile, String> {
    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| format!("Failed to load PDF for profile: {e}"))?;

    let page_count = super::pdf::pdfium_page_count(document.pages().len());
    let mut pages = Vec::with_capacity(page_count);

    for page_index in 0..page_count {
        let page = document
            .pages()
            .get(super::pdf::pdfium_page_index(page_index)?)
            .map_err(|e| format!("Failed to get page {} for profile: {e}", page_index + 1))?;

        let text_chars = page
            .text()
            .map(|text| count_quality_text_chars(&text.to_string()))
            .unwrap_or(0);

        let page_width = page.width().value.max(0.0);
        let page_height = page.height().value.max(0.0);
        let page_area = page_width * page_height;

        let mut image_count = 0usize;
        let mut largest_image_ratio = 0.0f32;

        if page_area > 0.0 {
            for object in page.objects().iter() {
                if object.object_type() != PdfPageObjectType::Image {
                    continue;
                }

                image_count += 1;

                if let Ok(bounds) = object.bounds() {
                    let rect = bounds.to_rect();
                    let image_area = rect.width().value.max(0.0) * rect.height().value.max(0.0);
                    let ratio = (image_area / page_area).clamp(0.0, 1.0);
                    largest_image_ratio = largest_image_ratio.max(ratio);
                }
            }
        }

        pages.push(classify_page(
            (page_index + 1) as u32,
            text_chars,
            image_count,
            largest_image_ratio,
        ));
    }

    Ok(summarize_document(pages))
}

fn count_quality_text_chars(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphanumeric()).count()
}

pub fn classify_page(
    page_number: u32,
    text_chars: usize,
    image_count: usize,
    largest_image_ratio: f32,
) -> PageProfile {
    let has_text = text_chars >= TEXT_CHARS_THRESHOLD;
    let image_dominant = largest_image_ratio >= IMAGE_DOMINANT_RATIO;
    let full_page_image_like = largest_image_ratio >= FULL_PAGE_IMAGE_RATIO;
    let kind = match (has_text, image_dominant) {
        (true, false) => PageKind::Native,
        (false, true) => PageKind::ImageOnly,
        (true, true) => PageKind::ImageWithOcr,
        (false, false) => PageKind::Uncertain,
    };

    PageProfile {
        page_number,
        has_text,
        text_chars,
        image_count,
        largest_image_ratio,
        full_page_image_like,
        kind,
    }
}

pub fn summarize_document(pages: Vec<PageProfile>) -> DocumentProfile {
    let native_pages = pages
        .iter()
        .filter(|page| page.kind == PageKind::Native)
        .count();
    let image_only_pages = pages
        .iter()
        .filter(|page| page.kind == PageKind::ImageOnly)
        .count();
    let image_with_ocr_pages = pages
        .iter()
        .filter(|page| page.kind == PageKind::ImageWithOcr)
        .count();
    let uncertain_pages = pages
        .iter()
        .filter(|page| page.kind == PageKind::Uncertain)
        .count();

    let total_pages = pages.len();
    let dominant_kind = dominant_kind(
        total_pages,
        native_pages,
        image_only_pages,
        image_with_ocr_pages,
        uncertain_pages,
    );
    let mixed = total_pages == 0
        || pages.iter().any(|page| page.kind != dominant_kind)
        || dominant_kind == PageKind::Uncertain;
    let should_render_as_images =
        !(dominant_kind == PageKind::Native && !mixed && uncertain_pages == 0);

    DocumentProfile {
        pages,
        native_pages,
        image_only_pages,
        image_with_ocr_pages,
        uncertain_pages,
        dominant_kind,
        mixed,
        should_render_as_images,
    }
}

fn dominant_kind(
    total_pages: usize,
    native_pages: usize,
    image_only_pages: usize,
    image_with_ocr_pages: usize,
    uncertain_pages: usize,
) -> PageKind {
    if total_pages == 0 {
        return PageKind::Uncertain;
    }

    let threshold = (total_pages as f32 * DOCUMENT_DOMINANCE_RATIO).ceil() as usize;
    let candidates = [
        (PageKind::Native, native_pages),
        (PageKind::ImageOnly, image_only_pages),
        (PageKind::ImageWithOcr, image_with_ocr_pages),
        (PageKind::Uncertain, uncertain_pages),
    ];

    candidates
        .into_iter()
        .find_map(|(kind, count)| (count >= threshold).then_some(kind))
        .unwrap_or(PageKind::Uncertain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_page_marks_native_only_when_text_is_sufficient_without_dominant_image() {
        let profile = classify_page(1, 40, 0, 0.0);

        assert_eq!(profile.kind, PageKind::Native);
        assert!(profile.has_text);
        assert!(!profile.full_page_image_like);
    }

    #[test]
    fn classify_page_distinguishes_image_only_and_image_with_ocr() {
        assert_eq!(classify_page(1, 0, 1, 0.91).kind, PageKind::ImageOnly);
        assert_eq!(classify_page(1, 40, 1, 0.91).kind, PageKind::ImageWithOcr);
        assert!(classify_page(1, 40, 1, 0.95).full_page_image_like);
    }

    #[test]
    fn summarize_document_defaults_uncertain_or_mixed_documents_to_images() {
        let profile = summarize_document(vec![
            classify_page(1, 40, 0, 0.0),
            classify_page(2, 0, 1, 0.91),
        ]);

        assert_eq!(profile.dominant_kind, PageKind::Uncertain);
        assert!(profile.mixed);
        assert!(profile.should_render_as_images);
    }

    #[test]
    fn summarize_document_keeps_only_confident_native_documents_as_pdf() {
        let profile = summarize_document(vec![
            classify_page(1, 40, 0, 0.0),
            classify_page(2, 80, 1, 0.20),
        ]);

        assert_eq!(profile.dominant_kind, PageKind::Native);
        assert!(!profile.mixed);
        assert!(!profile.should_render_as_images);
    }

    #[test]
    fn summarize_document_converts_image_with_ocr_documents_to_images() {
        let profile = summarize_document(vec![
            classify_page(1, 60, 1, 0.92),
            classify_page(2, 80, 1, 0.96),
        ]);

        assert_eq!(profile.dominant_kind, PageKind::ImageWithOcr);
        assert!(!profile.mixed);
        assert!(profile.should_render_as_images);
    }

    #[test]
    fn summarize_document_treats_empty_document_as_uncertain_image_render() {
        let profile = summarize_document(Vec::new());

        assert!(profile.pages.is_empty());
        assert_eq!(profile.dominant_kind, PageKind::Uncertain);
        assert!(profile.mixed);
        assert!(profile.should_render_as_images);
    }
}
