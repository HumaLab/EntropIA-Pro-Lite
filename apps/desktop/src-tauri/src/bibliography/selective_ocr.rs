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
/// publisher called rich is never re-examined by the OCR pass. Garbled
/// text (raw glyph codes from a custom font encoding) grades
/// [`PageQuality::NativeEmpty`]: OCR owns that page and the codes are never
/// merged back in.
pub fn classify_page_text(native_text: Option<&str>) -> PageQuality {
    match native_text {
        None => PageQuality::NativeEmpty,
        Some(text) if text.trim().is_empty() => PageQuality::NativeEmpty,
        Some(text) if crate::ocr::pdf::is_garbled_text(text) => PageQuality::NativeEmpty,
        Some(text) if crate::ocr::pdf::is_quality_text(text) => PageQuality::NativeRich,
        Some(_) => PageQuality::NativeSparse,
    }
}

/// The exact upstream error a GLM-OCR answer with no useful content
/// produces (`ocr::GLM_OCR_EMPTY_RESPONSE_MESSAGE`). In the page path that
/// answer means "this page holds no text", not a page failure.
pub const EMPTY_OCR_PAGE_RESPONSE: &str = crate::ocr::GLM_OCR_EMPTY_RESPONSE_MESSAGE;

/// True when `error` is the GLM-OCR empty response, i.e. the provider
/// answered but the page carries no recognizable text.
pub fn is_empty_ocr_page_response(error: &str) -> bool {
    error.contains(EMPTY_OCR_PAGE_RESPONSE)
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
    fn garbled_pages_route_to_ocr_which_owns_the_page() {
        // Raw glyph codes from a font with a custom encoding and no
        // ToUnicode map ("3FWJTUB" = "Revista"): needs OCR and the native
        // codes must never be kept, so the verdict is NativeEmpty.
        assert_eq!(
            classify_page_text(Some(
                "3FWJTUB %JDJBMJ[BMF 4FQJFNCJ[BDJ %JSJF[B 1VCJPFT 4B[BDJ 4JFOUJGJDP"
            )),
            PageQuality::NativeEmpty
        );
    }

    #[test]
    fn a_glm_empty_response_is_an_empty_page_not_a_failure() {
        assert!(is_empty_ocr_page_response(EMPTY_OCR_PAGE_RESPONSE));
        assert!(is_empty_ocr_page_response(&format!(
            "provider: {EMPTY_OCR_PAGE_RESPONSE}"
        )));
        assert!(!is_empty_ocr_page_response("request timed out after 30s"));
        assert!(!is_empty_ocr_page_response(
            "GLM-OCR no está configurado: cargá una API key"
        ));
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
    fn whole_document_mode_is_for_documents_that_are_mostly_scan() {
        assert!(should_use_pdf_mode(70, 70), "a full scan");
        assert!(should_use_pdf_mode(5, 10), "half the pages is enough");
        assert!(
            !should_use_pdf_mode(1, 10),
            "a native paper with one sparse page"
        );
        assert!(!should_use_pdf_mode(0, 10), "nothing to recognize");
    }

    #[test]
    fn windows_cover_the_document_and_skip_native_stretches() {
        let all: Vec<i64> = (1..=205).collect();
        assert_eq!(
            plan_pdf_windows(&all, &[], 205, 100),
            vec![(1, 100), (101, 200), (201, 205)]
        );
        assert_eq!(plan_pdf_windows(&all[..20], &[], 20, 100), vec![(1, 20)]);
        // Only the last window holds a page that needs OCR.
        assert_eq!(plan_pdf_windows(&[150], &[], 205, 100), vec![(101, 200)]);
        assert!(plan_pdf_windows(&[], &[], 205, 100).is_empty());
    }

    /// §2.4: cached pages are cut out of the document, so a window never
    /// spans one. Each remaining run is chunked from its own start under the
    /// provider's page limit, and a run without a miss is dropped.
    #[test]
    fn cached_pages_split_the_windows_into_miss_runs() {
        // Contiguous hits: only the tail run is paid for.
        let cached: Vec<i64> = (1..=4).collect();
        let misses: Vec<i64> = (5..=10).collect();
        assert_eq!(
            plan_pdf_windows(&misses, &cached, 10, 100),
            vec![(5, 10)],
            "the four cached pages are cut out of the single window"
        );

        // Alternating hits: every miss becomes its own run.
        assert_eq!(
            plan_pdf_windows(&[2, 4, 6, 8, 9, 10], &[1, 3, 5, 7], 10, 100),
            vec![(2, 2), (4, 4), (6, 6), (8, 10)],
            "each miss run is its own request and none crosses a cached page"
        );

        // The provider limit still applies inside a run.
        assert_eq!(
            plan_pdf_windows(&[1, 3, 4, 5, 6, 7], &[2], 7, 2),
            vec![(1, 1), (3, 4), (5, 6), (7, 7)]
        );

        // A run without a miss is dropped even when its neighbours pay.
        assert_eq!(
            plan_pdf_windows(&[6], &[3], 8, 2),
            vec![(6, 7)],
            "the run before the cached page holds no miss"
        );
    }

    /// §2.4: when every page is cached there is no window at all — and a
    /// cached page is never inside a window, boundary pages included.
    #[test]
    fn all_cached_pages_plan_no_window_at_all() {
        let cached: Vec<i64> = (1..=10).collect();
        assert!(plan_pdf_windows(&[], &cached, 10, 100).is_empty());
        assert!(plan_pdf_windows(&[1, 3], &[1, 2, 3], 3, 100).is_empty());
        assert!(plan_pdf_windows(&[1], &[1], 1, 100).is_empty());
        assert_eq!(
            plan_pdf_windows(&[5], &[10], 10, 2),
            vec![(5, 6)],
            "a cached page past the only miss still cuts the run before it"
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

// ── Selective OCR traits (E4b-WU3) ─────────────────────────────────────────
//
// Boundaries the bibliography executor drives. Production implementations
// arrive in E4b-WU4 (pdfium renderer, paddle/GLM selector); tests inject
// fakes so page checkpoints, resume, and no-duplicate integration are
// provable without models, keys, or native libraries.

/// Renders one PDF page (1-based) to PNG bytes for a provider.
pub trait PageRenderer: Send + Sync {
    fn render_page(&self, pdf_bytes: &[u8], page_number: u32) -> Result<Vec<u8>, String>;
    fn name(&self) -> &'static str;
}

/// Recognizes one rendered page. Plain text out; layout stays with the
/// renderer and the provider's own regions (E4d consumes spans).
pub trait PageOcrProvider: Send + Sync {
    fn recognize_page(&self, image_bytes: &[u8]) -> Result<String, String>;

    /// How many PDF pages one whole-document request may carry, or `None`
    /// when the provider only reads rendered page images. Whole-document
    /// mode needs no page rendering, so a scanned PDF costs one request per
    /// window instead of one per page.
    fn pdf_pages_per_request(&self) -> Option<usize> {
        None
    }

    /// Recognizes pages `first_page..=last_page` (1-based, inclusive) of
    /// `pdf_bytes` in one request. Returns exactly one text per page of the
    /// range, in page order (an empty string for a page with no text); any
    /// other shape is an error, never a guess at page boundaries.
    fn recognize_pdf_pages(
        &self,
        _pdf_bytes: &[u8],
        _first_page: u32,
        _last_page: u32,
    ) -> Result<Vec<String>, String> {
        Err("pdf_mode_unsupported: this provider reads page images only".to_string())
    }

    fn name(&self) -> &str;

    /// The model this provider bills against — the second half of the OCR
    /// contract. Providers that run no model (test doubles) keep the
    /// default.
    fn model_id(&self) -> &str {
        "unspecified"
    }

    /// The versioned contract that says which provider configuration
    /// produced a page answer (plan D3):
    /// `ocr-v<version>:<provider>:<model>:p<version>`. Cached pages are
    /// stored under it and reused only when it matches, so a change of
    /// provider, model or prompt/parser version misses them.
    fn ocr_contract(&self) -> String {
        crate::bibliography::ocr_contract::ocr_contract_for(self.name(), self.model_id())
    }
}

/// Whole-document OCR pays off when most of the document needs it (a scan);
/// a mostly-native document with a few sparse pages stays on page images so
/// native pages are never re-recognized.
pub fn should_use_pdf_mode(needs_ocr: usize, total_pages: usize) -> bool {
    needs_ocr > 0 && needs_ocr * 2 >= total_pages
}

/// Splits `1..=total_pages` into runs of consecutive pages that are NOT in
/// `cached_pages`, chunks each run into windows of at most
/// `pages_per_request` pages from the run's own start, and keeps only the
/// windows holding a page that still needs OCR. Windows are 1-based and
/// inclusive.
///
/// A cached page was already paid for (§2.4): a window that contained it would
/// send it to the provider again, so every run stops at one — dropping the
/// page from `needing_pages` alone is not enough. With no cached pages the
/// plan is exactly the one the executor always produced: windows aligned from
/// page 1, windows without a pending page dropped.
pub fn plan_pdf_windows(
    needing_pages: &[i64],
    cached_pages: &[i64],
    total_pages: usize,
    pages_per_request: usize,
) -> Vec<(u32, u32)> {
    let size = pages_per_request.max(1);
    let mut windows = Vec::new();
    let mut first = 1usize;
    while first <= total_pages {
        if cached_pages.contains(&(first as i64)) {
            first += 1;
            continue;
        }
        // The run of consecutive pages the provider may read.
        let mut run_end = first;
        while run_end < total_pages && !cached_pages.contains(&((run_end + 1) as i64)) {
            run_end += 1;
        }
        let mut window_first = first;
        while window_first <= run_end {
            let window_last = (window_first + size - 1).min(run_end);
            let holds_pending_page = needing_pages
                .iter()
                .any(|page| *page >= window_first as i64 && *page <= window_last as i64);
            if holds_pending_page {
                windows.push((window_first as u32, window_last as u32));
            }
            window_first = window_last + 1;
        }
        first = run_end + 1;
    }
    windows
}

/// Classifies a provider failure the way the queue understands it.
/// Transport and overload signals retry with backoff; missing credentials
/// or models park as configuration (the user fixes settings, then
/// resumes); anything else fails the unit with its cause.
pub fn map_page_ocr_error(error: &str) -> crate::processing::scheduler::ExecOutput {
    use crate::processing::scheduler::ExecOutput;
    let lower = error.to_lowercase();
    if lower.contains("429") || lower.contains("rate limit") {
        return ExecOutput::Retryable {
            code: crate::processing::repository::RATE_LIMITED_CODE.to_string(),
            message: error.to_string(),
        };
    }
    for signal in [
        "timeout",
        "timed out",
        "connection",
        "429",
        "rate limit",
        "500",
        "502",
        "503",
        "504",
    ] {
        if lower.contains(signal) {
            return ExecOutput::Retryable {
                code: "provider_transient".to_string(),
                message: error.to_string(),
            };
        }
    }
    if lower.contains("api key")
        || lower.contains("unauthorized")
        || lower.contains("401")
        || lower.contains("403")
        || lower.contains("no paddle")
        || lower.contains("model")
        || lower.contains("not configured")
        || lower.contains("not installed")
    {
        return ExecOutput::Blocked {
            code: "configuration_required_ocr".to_string(),
            message: error.to_string(),
        };
    }
    ExecOutput::Fatal {
        code: "ocr_failed".to_string(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod trait_tests {
    use super::map_page_ocr_error;
    use crate::processing::scheduler::ExecOutput;

    #[test]
    fn a_rate_limit_keeps_its_never_terminal_code_through_the_chunk_wave_mapper() {
        match map_page_ocr_error("OpenRouter embedding API error (429 Too Many Requests): {}") {
            ExecOutput::Retryable { code, .. } => {
                assert_eq!(code, crate::processing::repository::RATE_LIMITED_CODE)
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            map_page_ocr_error("OpenRouter embedding API error (401 Unauthorized): bad key"),
            ExecOutput::Blocked { .. }
        ));
        assert!(matches!(
            map_page_ocr_error("OpenRouter embedding API error (400 Bad Request): bad input"),
            ExecOutput::Fatal { .. }
        ));
    }

    #[test]
    fn ocr_errors_map_to_retry_block_or_fatal() {
        assert!(matches!(
            map_page_ocr_error("request timed out after 30s"),
            ExecOutput::Retryable { .. }
        ));
        assert!(matches!(
            map_page_ocr_error("OpenRouter API error (429): slow down"),
            ExecOutput::Retryable { .. }
        ));
        assert!(matches!(
            map_page_ocr_error("GLM-OCR no está configurado: cargá una API key"),
            ExecOutput::Blocked { .. }
        ));
        assert!(matches!(
            map_page_ocr_error("no paddle models installed"),
            ExecOutput::Blocked { .. }
        ));
        assert!(matches!(
            map_page_ocr_error("splines reticulated unexpectedly"),
            ExecOutput::Fatal { .. }
        ));
    }
}

/// The contract seam between a provider and the derived cache (plan D3): a
/// provider names itself and its model, and the contract derives from both.
#[cfg(test)]
mod contract_tests {
    use super::*;

    struct GlmLikeProvider;

    impl PageOcrProvider for GlmLikeProvider {
        fn recognize_page(&self, _image_bytes: &[u8]) -> Result<String, String> {
            Ok(String::new())
        }

        fn name(&self) -> &str {
            "glm"
        }

        fn model_id(&self) -> &str {
            "glm-ocr"
        }
    }

    /// A provider that runs no model (a test double, or a future local engine
    /// without one) still gets one stable contract, never an empty field.
    struct ModelLessProvider;

    impl PageOcrProvider for ModelLessProvider {
        fn recognize_page(&self, _image_bytes: &[u8]) -> Result<String, String> {
            Ok(String::new())
        }

        fn name(&self) -> &str {
            "fake"
        }
    }

    #[test]
    fn a_provider_contract_is_its_name_and_model_under_the_current_version() {
        assert_eq!(GlmLikeProvider.ocr_contract(), "ocr-v1:glm:glm-ocr:p1");
    }

    #[test]
    fn a_provider_without_a_model_still_gets_a_stable_contract() {
        assert_eq!(ModelLessProvider.model_id(), "unspecified");
        assert_eq!(
            ModelLessProvider.ocr_contract(),
            "ocr-v1:fake:unspecified:p1"
        );
    }
}
