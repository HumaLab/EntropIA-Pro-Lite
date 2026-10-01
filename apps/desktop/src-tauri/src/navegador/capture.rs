//! Turning what the capture script found in a page into a typed draft.
//!
//! The script ([`SCRIPT`]) runs in the page's main world, through the platform's
//! script evaluation (`Webview::eval_with_callback`), not through the page's
//! IPC. Consequences that shape this module:
//! - A hostile page controls its own content and can also tamper with the
//!   globals the script relies on, so what comes back may say anything. Every
//!   field is untrusted data: parsed into a typed struct, size-checked, cleaned
//!   and (for the URL) run through [`url_policy`] again on this side.
//! - The draft is data, never instructions. Nothing here, or in the commands
//!   built on it, writes any of it to the filesystem or the database (T5
//!   persists nothing); `accessed_at` comes from this process's clock, never
//!   from the page.
//!
//! What is hashed: for a page, the SHA-256 of the HTML snapshot's UTF-8 bytes;
//! for a selection, the SHA-256 of the exact quote's UTF-8 bytes. The draft
//! records which one in `hash_of`.
//!
//! The HTML snapshot is the raw `outerHTML`, scripts included. The plan strips
//! active content when a local copy is DISPLAYED, not when it is captured, so
//! the snapshot stays a faithful record of what the page served.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Url;

use super::url_policy::{self, NavigationKind};

/// The capture script: a function expression taking the kind (`"page"` or
/// `"selection"`) and returning a JSON-serialisable object.
pub const SCRIPT: &str = include_str!("capture.js");

/// Plain text of a page or selection, in bytes of UTF-8.
pub const TEXT_MAX_BYTES: usize = 2 * 1024 * 1024;
/// HTML snapshot, in bytes of UTF-8.
pub const HTML_MAX_BYTES: usize = 10 * 1024 * 1024;
/// The whole message from the page. JSON escaping can multiply the content, so
/// this is well above the sum of the two caps above.
pub const RAW_MAX_BYTES: usize = 64 * 1024 * 1024;
/// Characters of context kept on each side of a selection.
pub const CONTEXT_MAX_CHARS: usize = 400;

const TITLE_MAX_CHARS: usize = 500;
const SITE_NAME_MAX_CHARS: usize = 200;
const LANG_MAX_CHARS: usize = 35;
const ERROR_DETAIL_MAX_CHARS: usize = 200;

/// Stable codes the UI maps to messages.
pub mod code {
    pub const NO_SELECTION: &str = "no_selection";
    pub const PDF_DOCUMENT: &str = "pdf_document";
    pub const SCRIPT_FAILED: &str = "script_failed";
    pub const INVALID_RESULT: &str = "invalid_result";
    pub const TOO_LARGE: &str = "too_large";
    pub const BLOCKED_URL: &str = "blocked_url";
    pub const TIMEOUT: &str = "timeout";
    pub const NOT_OPEN: &str = "not_open";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureKind {
    Page,
    Selection,
}

impl CaptureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CaptureKind::Page => "page",
            CaptureKind::Selection => "selection",
        }
    }
}

/// Why a capture produced no draft. `code` is stable; `detail` is for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureError {
    pub code: &'static str,
    pub detail: Option<String>,
}

impl CaptureError {
    pub fn new(code: &'static str) -> Self {
        Self { code, detail: None }
    }

    pub fn with_detail(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: Some(detail.into()),
        }
    }
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{}: {detail}", self.code),
            None => f.write_str(self.code),
        }
    }
}

/// A capture that has not been saved anywhere. Serialised to the main UI; the
/// HTML snapshot stays on this side (it can be 10 MB) and is never sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDraft {
    pub kind: CaptureKind,
    /// Where the page says it is, after policy validation.
    pub final_url: String,
    pub title: Option<String>,
    /// What the site declares as canonical. Untrusted; http(s) only.
    pub canonical_url: Option<String>,
    pub site_name: Option<String>,
    pub lang: Option<String>,
    /// Readable text of the page, or the selected text.
    pub text: String,
    pub quote: Option<String>,
    pub quote_prefix: Option<String>,
    pub quote_suffix: Option<String>,
    #[serde(skip)]
    pub html: Option<String>,
    pub html_bytes: usize,
    /// `"html"` for a page, `"quote"` for a selection.
    pub hash_of: &'static str,
    pub sha256: String,
    pub truncated: bool,
    /// UTC, RFC 3339, from this process's clock.
    pub accessed_at: String,
}

/// What the script sends back. Every field optional: it is untrusted.
#[derive(Debug, Deserialize)]
struct RawCapture {
    ok: bool,
    error: Option<String>,
    kind: Option<String>,
    url: Option<String>,
    title: Option<String>,
    #[serde(rename = "canonicalUrl")]
    canonical_url: Option<String>,
    #[serde(rename = "siteName")]
    site_name: Option<String>,
    lang: Option<String>,
    text: Option<String>,
    quote: Option<String>,
    #[serde(rename = "quotePrefix")]
    quote_prefix: Option<String>,
    #[serde(rename = "quoteSuffix")]
    quote_suffix: Option<String>,
    html: Option<String>,
    truncated: Option<bool>,
}

/// Validate the script's result into a draft.
///
/// `navigation_for` says how the page's URL was reached, so a typed `http`
/// page can still be captured while a link-followed one cannot.
pub fn parse_capture(
    raw: &str,
    expected: CaptureKind,
    accessed_at: String,
    navigation_for: impl Fn(&Url) -> NavigationKind,
) -> Result<CaptureDraft, CaptureError> {
    if raw.len() > RAW_MAX_BYTES {
        return Err(CaptureError::with_detail(
            code::TOO_LARGE,
            "the page result",
        ));
    }
    let raw: RawCapture = serde_json::from_str(raw)
        .map_err(|_| CaptureError::with_detail(code::INVALID_RESULT, "not a capture object"))?;

    if !raw.ok {
        return Err(match raw.error.as_deref() {
            Some(code::NO_SELECTION) => CaptureError::new(code::NO_SELECTION),
            Some(code::PDF_DOCUMENT) => CaptureError::new(code::PDF_DOCUMENT),
            Some(other) => match clean_line(other, ERROR_DETAIL_MAX_CHARS) {
                Some(detail) => CaptureError::with_detail(code::SCRIPT_FAILED, detail),
                None => CaptureError::new(code::SCRIPT_FAILED),
            },
            None => CaptureError::new(code::SCRIPT_FAILED),
        });
    }

    if raw.kind.as_deref() != Some(expected.as_str()) {
        return Err(CaptureError::with_detail(
            code::INVALID_RESULT,
            "wrong kind",
        ));
    }
    let url = raw
        .url
        .as_deref()
        .and_then(|u| Url::parse(u).ok())
        .ok_or_else(|| CaptureError::with_detail(code::INVALID_RESULT, "no valid url"))?;
    url_policy::check_url(&url, navigation_for(&url))
        .map_err(|reason| CaptureError::with_detail(code::BLOCKED_URL, reason.to_string()))?;

    let too_large = |what: &str| CaptureError::with_detail(code::TOO_LARGE, what.to_string());
    let text = raw.text.unwrap_or_default();
    if text.len() > TEXT_MAX_BYTES {
        return Err(too_large("text"));
    }
    if raw.html.as_ref().is_some_and(|h| h.len() > HTML_MAX_BYTES) {
        return Err(too_large("html"));
    }
    if raw.quote.as_ref().is_some_and(|q| q.len() > TEXT_MAX_BYTES) {
        return Err(too_large("quote"));
    }

    let selection = expected == CaptureKind::Selection;
    let (text, quote, html, hash_of, sha256, html_bytes) = if selection {
        let quote = raw
            .quote
            .filter(|q| !q.trim().is_empty())
            .ok_or_else(|| CaptureError::new(code::NO_SELECTION))?;
        let sha256 = sha256_hex(quote.as_bytes());
        (quote.clone(), Some(quote), None, "quote", sha256, 0)
    } else {
        let html = raw
            .html
            .filter(|h| !h.is_empty())
            .ok_or_else(|| CaptureError::with_detail(code::INVALID_RESULT, "no html"))?;
        let sha256 = sha256_hex(html.as_bytes());
        let html_bytes = html.len();
        (text, None, Some(html), "html", sha256, html_bytes)
    };

    Ok(CaptureDraft {
        kind: expected,
        final_url: url.to_string(),
        title: raw.title.and_then(|t| clean_line(&t, TITLE_MAX_CHARS)),
        canonical_url: raw.canonical_url.and_then(|u| http_url(&u)),
        site_name: raw
            .site_name
            .and_then(|s| clean_line(&s, SITE_NAME_MAX_CHARS)),
        lang: raw.lang.and_then(|l| clean_line(&l, LANG_MAX_CHARS)),
        text,
        quote,
        quote_prefix: raw
            .quote_prefix
            .filter(|_| selection)
            .map(|p| tail_chars(&p, CONTEXT_MAX_CHARS)),
        quote_suffix: raw
            .quote_suffix
            .filter(|_| selection)
            .map(|s| head_chars(&s, CONTEXT_MAX_CHARS)),
        html,
        html_bytes,
        hash_of,
        sha256,
        truncated: raw.truncated.unwrap_or(false),
        accessed_at,
    })
}

/// One clean line of untrusted text: control characters and bidirectional
/// overrides gone, runs of whitespace collapsed, at most `max` characters.
/// `None` when nothing is left.
pub(super) fn clean_line(input: &str, max: usize) -> Option<String> {
    let spaced: String = input
        .chars()
        .filter(|c| !is_bidi_control(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let line: String = line.chars().take(max).collect();
    let line = line.trim_end().to_string();
    (!line.is_empty()).then_some(line)
}

/// Characters that reorder how text is displayed, a spoofing vector in titles.
pub(super) fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

fn head_chars(input: &str, max: usize) -> String {
    input.chars().take(max).collect()
}

fn tail_chars(input: &str, max: usize) -> String {
    let count = input.chars().count();
    input.chars().skip(count.saturating_sub(max)).collect()
}

/// An absolute `http(s)` URL, normalised; anything else is dropped.
fn http_url(input: &str) -> Option<String> {
    let url = Url::parse(input.trim()).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

/// The script to run in the page for `kind`.
pub fn script_for(kind: CaptureKind) -> String {
    // `kind` is one of two static strings: a quoted literal is a safe JS string.
    SCRIPT.replacen("'__KIND__'", &format!("\"{}\"", kind.as_str()), 1)
}

/// SHA-256 as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// `secs` since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// The current time, UTC, RFC 3339.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_utc(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const AT: &str = "2026-09-30T12:00:00Z";

    fn navigation(_: &Url) -> NavigationKind {
        NavigationKind::Navigation
    }

    fn page_json() -> serde_json::Value {
        json!({
            "ok": true,
            "kind": "page",
            "url": "https://example.com/article",
            "title": "An article",
            "canonicalUrl": "https://example.com/a",
            "siteName": "Example",
            "lang": "en",
            "text": "Readable text",
            "html": "abc",
            "truncated": false,
        })
    }

    fn selection_json() -> serde_json::Value {
        json!({
            "ok": true,
            "kind": "selection",
            "url": "https://example.com/article",
            "title": "An article",
            "text": "the quote",
            "quote": "the quote",
            "quotePrefix": "before ",
            "quoteSuffix": " after",
            "truncated": false,
        })
    }

    fn page(value: serde_json::Value) -> Result<CaptureDraft, CaptureError> {
        parse_capture(
            &value.to_string(),
            CaptureKind::Page,
            AT.to_string(),
            navigation,
        )
    }

    fn selection(value: serde_json::Value) -> Result<CaptureDraft, CaptureError> {
        parse_capture(
            &value.to_string(),
            CaptureKind::Selection,
            AT.to_string(),
            navigation,
        )
    }

    fn code_of(result: Result<CaptureDraft, CaptureError>) -> &'static str {
        result.unwrap_err().code
    }

    #[test]
    fn a_page_becomes_a_draft_hashed_over_its_html() {
        let draft = page(page_json()).unwrap();
        assert_eq!(draft.kind, CaptureKind::Page);
        assert_eq!(draft.final_url, "https://example.com/article");
        assert_eq!(draft.title.as_deref(), Some("An article"));
        assert_eq!(
            draft.canonical_url.as_deref(),
            Some("https://example.com/a")
        );
        assert_eq!(draft.site_name.as_deref(), Some("Example"));
        assert_eq!(draft.lang.as_deref(), Some("en"));
        assert_eq!(draft.text, "Readable text");
        assert_eq!(draft.html.as_deref(), Some("abc"));
        assert_eq!(draft.html_bytes, 3);
        assert_eq!(draft.hash_of, "html");
        // SHA-256("abc"), the standard test vector.
        assert_eq!(
            draft.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(draft.accessed_at, AT);
        assert!(!draft.truncated);
    }

    #[test]
    fn a_selection_becomes_a_draft_hashed_over_its_quote() {
        let draft = selection(selection_json()).unwrap();
        assert_eq!(draft.kind, CaptureKind::Selection);
        assert_eq!(draft.quote.as_deref(), Some("the quote"));
        assert_eq!(draft.quote_prefix.as_deref(), Some("before "));
        assert_eq!(draft.quote_suffix.as_deref(), Some(" after"));
        assert_eq!(draft.html, None);
        assert_eq!(draft.hash_of, "quote");
        assert_eq!(draft.sha256, sha256_hex("the quote".as_bytes()));
    }

    #[test]
    fn the_time_of_access_is_ours_never_the_pages() {
        let mut value = page_json();
        value["accessedAt"] = json!("1999-01-01T00:00:00Z");
        value["accessed_at"] = json!("1999-01-01T00:00:00Z");
        assert_eq!(page(value).unwrap().accessed_at, AT);
    }

    #[test]
    fn anything_but_a_json_object_is_rejected() {
        for raw in ["null", "[]", "\"text\"", "42", "true", "", "{", "not json"] {
            let result = parse_capture(raw, CaptureKind::Page, AT.to_string(), navigation);
            assert_eq!(code_of(result), code::INVALID_RESULT, "input: {raw:?}");
        }
    }

    #[test]
    fn a_field_of_the_wrong_type_is_rejected() {
        for (field, value) in [
            ("title", json!(5)),
            ("text", json!(["a"])),
            ("html", json!({})),
            ("truncated", json!("yes")),
            ("url", json!(false)),
        ] {
            let mut doc = page_json();
            doc[field] = value;
            assert_eq!(code_of(page(doc)), code::INVALID_RESULT, "field: {field}");
        }
    }

    #[test]
    fn the_script_reporting_failure_maps_to_a_code() {
        let none = json!({"ok": false, "error": "no_selection"});
        assert_eq!(code_of(selection(none)), code::NO_SELECTION);
        let pdf = json!({"ok": false, "error": "pdf_document"});
        assert_eq!(code_of(page(pdf)), code::PDF_DOCUMENT);
        let other = json!({"ok": false, "error": "TypeError: x is not a function"});
        let error = page(other).unwrap_err();
        assert_eq!(error.code, code::SCRIPT_FAILED);
        assert!(error.detail.unwrap().contains("TypeError"));
        let silent = json!({"ok": false});
        assert_eq!(code_of(page(silent)), code::SCRIPT_FAILED);
    }

    #[test]
    fn a_failure_detail_from_the_page_is_bounded() {
        let long = "x".repeat(10_000);
        let error = page(json!({"ok": false, "error": long})).unwrap_err();
        assert!(error.detail.unwrap().chars().count() <= ERROR_DETAIL_MAX_CHARS);
    }

    #[test]
    fn a_result_for_the_other_kind_is_rejected() {
        assert_eq!(code_of(selection(page_json())), code::INVALID_RESULT);
        assert_eq!(code_of(page(selection_json())), code::INVALID_RESULT);
        let mut missing = page_json();
        missing.as_object_mut().unwrap().remove("kind");
        assert_eq!(code_of(page(missing)), code::INVALID_RESULT);
    }

    #[test]
    fn a_page_needs_a_url_that_parses() {
        for url in [json!(null), json!(""), json!("not a url")] {
            let mut doc = page_json();
            doc["url"] = url;
            assert_eq!(code_of(page(doc)), code::INVALID_RESULT);
        }
        let mut gone = page_json();
        gone.as_object_mut().unwrap().remove("url");
        assert_eq!(code_of(page(gone)), code::INVALID_RESULT);
    }

    #[test]
    fn the_reported_url_must_pass_the_url_policy() {
        for url in [
            "file:///etc/passwd",
            "http://169.254.169.254/latest",
            "https://localhost/",
            "javascript:alert(1)",
            "data:text/html,hi",
            // http only counts when it was typed.
            "http://example.com/",
        ] {
            let mut doc = page_json();
            doc["url"] = json!(url);
            assert_eq!(code_of(page(doc)), code::BLOCKED_URL, "url: {url}");
        }
    }

    #[test]
    fn a_typed_http_page_can_be_captured() {
        let mut doc = page_json();
        doc["url"] = json!("http://example.com/");
        let draft = parse_capture(&doc.to_string(), CaptureKind::Page, AT.to_string(), |_| {
            NavigationKind::Typed
        })
        .unwrap();
        assert_eq!(draft.final_url, "http://example.com/");
    }

    #[test]
    fn oversized_content_is_rejected() {
        let mut text = page_json();
        text["text"] = json!("a".repeat(TEXT_MAX_BYTES + 1));
        assert_eq!(code_of(page(text)), code::TOO_LARGE);
        let mut html = page_json();
        html["html"] = json!("a".repeat(HTML_MAX_BYTES + 1));
        assert_eq!(code_of(page(html)), code::TOO_LARGE);
        let mut quote = selection_json();
        quote["quote"] = json!("a".repeat(TEXT_MAX_BYTES + 1));
        assert_eq!(code_of(selection(quote)), code::TOO_LARGE);
        let raw = " ".repeat(RAW_MAX_BYTES + 1);
        assert_eq!(
            code_of(parse_capture(
                &raw,
                CaptureKind::Page,
                AT.into(),
                navigation
            )),
            code::TOO_LARGE
        );
    }

    #[test]
    fn the_limits_count_bytes_not_characters() {
        // Two bytes per character: half the characters already fill the cap.
        let mut doc = page_json();
        doc["text"] = json!("é".repeat(TEXT_MAX_BYTES / 2 + 1));
        assert_eq!(code_of(page(doc)), code::TOO_LARGE);
        let mut edge = page_json();
        edge["text"] = json!("é".repeat(TEXT_MAX_BYTES / 2));
        assert!(page(edge).is_ok());
    }

    #[test]
    fn a_page_without_html_is_rejected() {
        let mut none = page_json();
        none.as_object_mut().unwrap().remove("html");
        assert_eq!(code_of(page(none)), code::INVALID_RESULT);
        let mut empty = page_json();
        empty["html"] = json!("");
        assert_eq!(code_of(page(empty)), code::INVALID_RESULT);
    }

    #[test]
    fn a_page_may_have_no_readable_text() {
        let mut doc = page_json();
        doc["text"] = json!("");
        assert_eq!(page(doc).unwrap().text, "");
        let mut missing = page_json();
        missing.as_object_mut().unwrap().remove("text");
        assert_eq!(page(missing).unwrap().text, "");
    }

    #[test]
    fn an_empty_selection_is_no_selection() {
        for quote in [json!(""), json!("   \n\t"), json!(null)] {
            let mut doc = selection_json();
            doc["quote"] = quote;
            assert_eq!(code_of(selection(doc)), code::NO_SELECTION);
        }
    }

    #[test]
    fn the_quote_is_kept_exactly() {
        let mut doc = selection_json();
        doc["quote"] = json!("  spaced\n quote  ");
        let draft = selection(doc).unwrap();
        assert_eq!(draft.quote.as_deref(), Some("  spaced\n quote  "));
        assert_eq!(draft.text, "  spaced\n quote  ");
        assert_eq!(draft.sha256, sha256_hex("  spaced\n quote  ".as_bytes()));
    }

    #[test]
    fn a_declared_canonical_url_must_be_an_absolute_http_url() {
        for canonical in [
            json!("/relative"),
            json!("javascript:alert(1)"),
            json!("file:///c:/x"),
            json!(""),
            json!(null),
        ] {
            let mut doc = page_json();
            doc["canonicalUrl"] = canonical;
            assert_eq!(page(doc).unwrap().canonical_url, None);
        }
        let mut http = page_json();
        http["canonicalUrl"] = json!("http://example.org/x");
        assert_eq!(
            page(http).unwrap().canonical_url.as_deref(),
            Some("http://example.org/x")
        );
    }

    #[test]
    fn single_line_fields_are_cleaned_and_bounded() {
        let mut doc = page_json();
        doc["title"] = json!("  A\u{0}b\nc\u{202e}\t d  ");
        doc["siteName"] = json!("s".repeat(1_000));
        doc["lang"] = json!("l".repeat(1_000));
        let draft = page(doc).unwrap();
        let title = draft.title.unwrap();
        assert!(!title.chars().any(char::is_control), "{title:?}");
        assert!(title.starts_with("A b c"));
        assert_eq!(
            draft.site_name.unwrap().chars().count(),
            SITE_NAME_MAX_CHARS
        );
        assert_eq!(draft.lang.unwrap().chars().count(), LANG_MAX_CHARS);

        let mut long = page_json();
        long["title"] = json!("t".repeat(5_000));
        assert_eq!(
            page(long).unwrap().title.unwrap().chars().count(),
            TITLE_MAX_CHARS
        );

        let mut blank = page_json();
        blank["title"] = json!("   ");
        assert_eq!(page(blank).unwrap().title, None);
    }

    #[test]
    fn the_context_around_a_selection_keeps_the_part_nearest_the_quote() {
        let mut doc = selection_json();
        doc["quotePrefix"] = json!(format!("{}NEAR", "p".repeat(1_000)));
        doc["quoteSuffix"] = json!(format!("NEAR{}", "s".repeat(1_000)));
        let draft = selection(doc).unwrap();
        let prefix = draft.quote_prefix.unwrap();
        let suffix = draft.quote_suffix.unwrap();
        assert_eq!(prefix.chars().count(), CONTEXT_MAX_CHARS);
        assert!(prefix.ends_with("NEAR"));
        assert_eq!(suffix.chars().count(), CONTEXT_MAX_CHARS);
        assert!(suffix.starts_with("NEAR"));
    }

    #[test]
    fn the_truncated_flag_is_carried_over() {
        let mut doc = page_json();
        doc["truncated"] = json!(true);
        assert!(page(doc).unwrap().truncated);
        let mut missing = page_json();
        missing.as_object_mut().unwrap().remove("truncated");
        assert!(!page(missing).unwrap().truncated);
    }

    #[test]
    fn the_draft_goes_to_the_ui_without_the_html() {
        let value = serde_json::to_value(page(page_json()).unwrap()).unwrap();
        let object = value.as_object().unwrap();
        assert!(!object.contains_key("html"));
        for key in [
            "kind",
            "finalUrl",
            "title",
            "canonicalUrl",
            "siteName",
            "lang",
            "text",
            "htmlBytes",
            "hashOf",
            "sha256",
            "truncated",
            "accessedAt",
        ] {
            assert!(object.contains_key(key), "missing {key}");
        }
        assert_eq!(object["kind"], "page");
    }

    #[test]
    fn the_script_gets_its_kind_and_nothing_else_changes() {
        for kind in [CaptureKind::Page, CaptureKind::Selection] {
            let script = script_for(kind);
            assert!(!script.contains("__KIND__"), "placeholder left in place");
            assert!(script.contains(&format!("var kind = \"{}\"", kind.as_str())));
            assert_eq!(
                script.len() + "'__KIND__'".len(),
                SCRIPT.len() + kind.as_str().len() + 2
            );
        }
    }

    #[test]
    fn errors_print_their_code_and_detail() {
        assert_eq!(CaptureError::new(code::TIMEOUT).to_string(), "timeout");
        assert_eq!(
            CaptureError::with_detail(code::SCRIPT_FAILED, "boom").to_string(),
            "script_failed: boom"
        );
    }

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn rfc3339_covers_the_epoch_leap_days_and_the_far_future() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(rfc3339_utc(1_709_251_199), "2024-02-29T23:59:59Z");
        assert_eq!(rfc3339_utc(4_102_444_800), "2100-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(4_107_542_400), "2100-03-01T00:00:00Z");
    }

    #[test]
    fn the_clock_reads_as_a_utc_timestamp() {
        let now = now_rfc3339();
        assert_eq!(now.len(), 20);
        assert!(now.ends_with('Z') && now.as_bytes()[10] == b'T', "{now}");
        assert!(now.as_str() > "2026-01-01T00:00:00Z");
    }
}
