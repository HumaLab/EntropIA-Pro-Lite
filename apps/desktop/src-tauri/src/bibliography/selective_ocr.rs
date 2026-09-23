//! Selective OCR decisions (E4b-WU1): pure, measurable, dependency-free.
//!
//! Given per-page native texts, this module decides which pages need OCR
//! and reports coverage the UI can render ("3 of 12 pages need OCR").
//! It never touches files, models, or the network: extraction (E4a-WU2)
//! supplies the texts, the capability probe gates the provider before any
//! content is sent (E4b-WU3 runs the decision).

/// Quality of one page's native text layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageQuality {
    /// Rich enough to stand alone: never OCRed.
    NativeRich,
    /// Some text, not enough: OCR supplements, native text is kept.
    NativeSparse,
    /// No usable text: OCR owns the page.
    NativeEmpty,
    /// The page could not be read at all (encrypted, corrupt, render
    /// failure): recorded explicitly, never silently skipped.
    Unreadable,
}

/// Classifies one page's native text: the same richness bar the
/// extraction publisher uses (`ocr::pdf::is_quality_text`), so a page the
/// publisher called rich is never re-examined by the OCR pass.
pub fn classify_page_text(native_text: Option<&str>) -> PageQuality {
    match native_text {
        None => PageQuality::NativeEmpty,
        Some(text) if text.trim().is_empty() => PageQuality::NativeEmpty,
        Some(text) if crate::ocr::pdf::is_quality_text(text) => PageQuality::NativeRich,
        Some(_) => PageQuality::NativeSparse,
    }
}

/// Coverage of one document: totals the UI renders and the executor
/// converges on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrCoverage {
    pub total_pages: usize,
    pub native_rich: usize,
    pub needs_ocr: usize,
    pub unreadable: usize,
}

/// Tallies per-page qualities into coverage. Sparse and empty pages
/// both need OCR (sparse supplements, empty owns); unreadable pages are
/// counted apart so the UI never presents them as pending work.
pub fn tally_coverage(qualities: &[PageQuality]) -> OcrCoverage {
    let mut coverage = OcrCoverage {
        total_pages: qualities.len(),
        native_rich: 0,
        needs_ocr: 0,
        unreadable: 0,
    };
    for quality in qualities {
        match quality {
            PageQuality::NativeRich => coverage.native_rich += 1,
            PageQuality::NativeSparse | PageQuality::NativeEmpty => coverage.needs_ocr += 1,
            PageQuality::Unreadable => coverage.unreadable += 1,
        }
    }
    coverage
}

/// OCR granularity the bibliography executor supports. Page-level is the
/// only honest granularity: whole-document OCR would re-recognize native
/// pages and duplicate text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrGranularity {
    Page,
}

/// What the executor can do before any content is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrCapability {
    pub available: bool,
    pub granularity: Option<OcrGranularity>,
    pub provider_name: Option<String>,
    pub reason: String,
}

/// Pure capability combination: a renderer plus a named provider yields
/// page-granular OCR; anything missing fails closed with the reason.
pub fn probe_page_ocr_capability(
    renderer_available: bool,
    provider_name: Option<&str>,
) -> OcrCapability {
    match (renderer_available, provider_name) {
        (true, Some(provider)) => OcrCapability {
            available: true,
            granularity: Some(OcrGranularity::Page),
            provider_name: Some(provider.to_string()),
            reason: format!("page renderer and {provider} provider ready"),
        },
        (false, _) => OcrCapability {
            available: false,
            granularity: None,
            provider_name: None,
            reason: "no page renderer available: cannot turn PDF pages into provider input"
                .to_string(),
        },
        (true, None) => OcrCapability {
            available: false,
            granularity: None,
            provider_name: None,
            reason: "no OCR provider configured".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_text_never_needs_ocr_sparse_and_empty_do() {
        assert_eq!(
            classify_page_text(Some(
                "Contenido nativo con peso semantico suficiente para superar el umbral"
            )),
            PageQuality::NativeRich
        );
        assert_eq!(classify_page_text(Some("ok")), PageQuality::NativeSparse);
        assert_eq!(classify_page_text(Some("   ")), PageQuality::NativeEmpty);
        assert_eq!(classify_page_text(None), PageQuality::NativeEmpty);
    }

    #[test]
    fn coverage_tallies_a_mixed_document() {
        let qualities = [
            PageQuality::NativeRich,
            PageQuality::NativeSparse,
            PageQuality::NativeEmpty,
            PageQuality::Unreadable,
            PageQuality::NativeRich,
        ];
        assert_eq!(
            tally_coverage(&qualities),
            OcrCoverage {
                total_pages: 5,
                native_rich: 2,
                needs_ocr: 2,
                unreadable: 1,
            }
        );
        assert_eq!(
            tally_coverage(&[]),
            OcrCoverage {
                total_pages: 0,
                native_rich: 0,
                needs_ocr: 0,
                unreadable: 0,
            }
        );
    }

    #[test]
    fn capability_requires_both_renderer_and_provider() {
        let both = probe_page_ocr_capability(true, Some("paddle"));
        assert_eq!(
            both,
            OcrCapability {
                available: true,
                granularity: Some(OcrGranularity::Page),
                provider_name: Some("paddle".to_string()),
                reason: "page renderer and paddle provider ready".to_string(),
            }
        );
        for (renderer, provider) in [(false, Some("paddle")), (true, None), (false, None)] {
            let capability = probe_page_ocr_capability(renderer, provider);
            assert!(
                !capability.available && capability.granularity.is_none(),
                "missing renderer or provider must fail closed: {capability:?}"
            );
        }
    }
}
