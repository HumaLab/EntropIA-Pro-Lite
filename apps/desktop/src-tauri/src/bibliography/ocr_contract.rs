//! The versioned OCR contract (plan-sync-bibliografia-derivada D3, WU3).
//!
//! A cached OCR page is keyed by `(source_hash, page_number, ocr_contract)`
//! (plan §2.2), so the contract has to name everything that can make the same
//! page come back different: the provider, its model, and the version of the
//! page prompt plus the parser that turns the answer into a page row. Two
//! devices reuse an answer only under the same contract string; a change of
//! provider, model or prompt/parser version misses every stored page.
//!
//! Format: `ocr-v<version>:<provider>:<model>:p<version>` — the leading
//! `ocr-v` keeps the family readable in a stored key, the trailing `p` is the
//! prompt/parser version D3 asks for. Both carry
//! [`OCR_PAGE_CONTRACT_VERSION`].

use sha2::{Digest, Sha256};

/// Version of the page prompt and of the page parser.
///
/// Bump this whenever any of these changes, because the answer to the same
/// page can change with it:
///
/// - the GLM-OCR page request in `ocr/glm_ocr.rs`: the endpoint, the model
///   id ([`crate::ocr::glm_ocr::GLM_OCR_MODEL_ID`]) and the `file` /
///   `start_page_id` / `end_page_id` fields;
/// - the local Paddle model the Pro build runs (`PADDLE_PAGE_MODEL_ID`, the
///   `PP-OCRv5` detection/recognition pair in `ocr/paddle.rs`);
/// - the parser that reads the answer: `glm_response_to_page_texts` and
///   `format_region_text` in `ocr/mod.rs`, plus
///   [`crate::ocr::markup::ocr_markup_to_text`], which `settled_page_row`
///   applies to the stored page text.
///
/// Version 1 is, by definition, the baseline for everything paid before this
/// cache existed (plan §2.3b).
pub const OCR_PAGE_CONTRACT_VERSION: u32 = 1;

/// The local Paddle model the page path runs when this build compiles it: the
/// `PP-OCRv5_mobile_det.mnn` + `latin_PP-OCRv5_mobile_rec_infer.mnn` pair
/// `ocr/paddle.rs` refuses to start without.
#[cfg(feature = "paddle-ocr")]
pub const PADDLE_PAGE_MODEL_ID: &str = "PP-OCRv5_mobile";

/// The contract of the provider configuration the executor is about to use:
/// `ocr-v<version>:<provider>:<model>:p<version>`.
pub fn ocr_contract_for(provider_name: &str, model_id: &str) -> String {
    ocr_contract_for_version(OCR_PAGE_CONTRACT_VERSION, provider_name, model_id)
}

/// [`ocr_contract_for`] with an explicit version. The version is a parameter
/// so a test can prove a bump invalidates hits without moving the constant
/// every device writes with.
pub fn ocr_contract_for_version(version: u32, provider_name: &str, model_id: &str) -> String {
    format!(
        "ocr-v{version}:{}:{}:p{version}",
        field(provider_name),
        field(model_id)
    )
}

/// One contract field: trimmed, with `%` and `:` escaped so a model id can
/// never split the four-field shape. A blank becomes `unknown`, the stand-in a
/// provider that runs no model reports.
fn field(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "unknown".to_string();
    }
    trimmed.replace('%', "%25").replace(':', "%3A")
}

/// A representative GLM-OCR answer: a heading, an HTML table — the provider
/// returns tables as HTML — and inline markup. The parser probe runs it
/// through [`crate::ocr::markup::ocr_markup_to_text`] so a change in the
/// conversion, not just in this comment, moves the fingerprint.
const PARSER_PROBE_MARKUP: &str = concat!(
    "## Título de la página\n\n",
    "<table><thead><tr><th>Col 1</th><th>Col 2</th></tr></thead>",
    "<tbody><tr><td>uno</td><td>dos</td></tr></tbody></table>\n\n",
    "<p>Texto con <b>negrita</b>.</p>"
);

/// Digest of the page prompt and of the parser that reads its answer: the
/// GLM-OCR request shape, its model id and the Markdown the markup parser
/// produces for `PARSER_PROBE_MARKUP`.
///
/// It is the guard behind [`OCR_PAGE_CONTRACT_VERSION`]: the prompt and the
/// parser live in other files, so this digest is what
/// [`PINNED_PAGE_PROMPT_FINGERPRINT`] pins — a change there fails a test that
/// says to bump the version.
pub fn page_prompt_fingerprint() -> String {
    let descriptor = format!(
        "glm-layout-parsing|model={}|fields=file,start_page_id,end_page_id|\
         answer=md_results+layout_details|markup={}",
        crate::ocr::glm_ocr::GLM_OCR_MODEL_ID,
        crate::ocr::markup::ocr_markup_to_text(PARSER_PROBE_MARKUP),
    );
    hex::encode(Sha256::digest(descriptor.as_bytes()))
}

/// [`page_prompt_fingerprint`] as of contract version 1. If the fingerprint
/// test fails, the page prompt or the markup parser moved: bump
/// [`OCR_PAGE_CONTRACT_VERSION`], then update this value in the same change.
pub const PINNED_PAGE_PROMPT_FINGERPRINT: &str =
    "e02fe37d75a123d859b56ff7cae857aaa6e1f50fad487e96bb9397c847ed2e05";

#[cfg(test)]
mod tests {
    use super::*;

    const GLM_CONTRACT: &str = "ocr-v1:glm:glm-ocr:p1";

    /// The cache key must be stable: the same configuration always names the
    /// same contract, or nothing is ever reused.
    #[test]
    fn the_same_provider_and_model_always_produce_the_same_contract() {
        assert_eq!(
            ocr_contract_for("glm", "glm-ocr"),
            ocr_contract_for("glm", "glm-ocr")
        );
        assert_eq!(
            ocr_contract_for("glm", "glm-ocr"),
            ocr_contract_for(" glm ", " glm-ocr ")
        );
    }

    /// A different provider is a different answer: it must miss every stored
    /// page.
    #[test]
    fn a_different_provider_produces_a_different_contract() {
        assert_ne!(
            ocr_contract_for("glm", "glm-ocr"),
            ocr_contract_for("paddle", "glm-ocr")
        );
    }

    /// Same provider, another model: also a different answer.
    #[test]
    fn a_different_model_produces_a_different_contract() {
        assert_ne!(
            ocr_contract_for("glm", "glm-ocr"),
            ocr_contract_for("glm", "glm-ocr-2")
        );
    }

    /// Bumping the version is how a prompt or parser change invalidates the
    /// cache (D3): the contract must differ.
    #[test]
    fn a_bumped_version_produces_a_different_contract() {
        let current = ocr_contract_for("glm", "glm-ocr");
        let bumped = ocr_contract_for_version(OCR_PAGE_CONTRACT_VERSION + 1, "glm", "glm-ocr");
        assert_ne!(current, bumped);
        assert_ne!(
            ocr_contract_for_version(1, "glm", "glm-ocr"),
            ocr_contract_for_version(2, "glm", "glm-ocr")
        );
    }

    /// The contract carries the version constant, so persisted keys can be
    /// told apart by version alone.
    #[test]
    fn the_contract_carries_the_version_constant() {
        assert_eq!(ocr_contract_for("glm", "glm-ocr"), GLM_CONTRACT);
        let contract = ocr_contract_for("glm", "glm-ocr");
        assert!(
            contract.contains(&format!("ocr-v{OCR_PAGE_CONTRACT_VERSION}:")),
            "version missing from {contract}"
        );
        assert!(
            contract.ends_with(&format!(":p{OCR_PAGE_CONTRACT_VERSION}")),
            "prompt/parser version missing from {contract}"
        );
        assert_eq!(contract.split(':').count(), 4, "{contract}");
    }

    /// A colon inside a field must never split the four-field shape: two
    /// different configurations can never collapse into one key.
    #[test]
    fn a_field_with_a_colon_never_splits_the_contract_fields() {
        let split = ocr_contract_for("glm:a", "glm-ocr");
        assert_eq!(split.split(':').count(), 4, "{split}");
        assert_ne!(split, ocr_contract_for("glm", "a:glm-ocr"));
        assert_ne!(split, ocr_contract_for("glm", "glm-ocr"));
    }

    /// The fingerprint only tracks the parser while the probe is markup the
    /// parser actually converts: a plain-text probe would leave the parser
    /// out of the digest entirely.
    #[test]
    fn the_probe_markup_actually_exercises_the_parser() {
        let parsed = crate::ocr::markup::ocr_markup_to_text(PARSER_PROBE_MARKUP);
        assert_ne!(parsed, PARSER_PROBE_MARKUP, "probe not converted: {parsed}");
        assert!(!parsed.contains('<'), "tags survived the parser: {parsed}");
        assert!(!parsed.trim().is_empty(), "probe converted to nothing");
    }

    /// The prompt and the parser live outside this module, so their identity
    /// is pinned: an edit there fails here until the version is bumped.
    #[test]
    fn the_glm_page_prompt_fingerprint_is_pinned_so_a_prompt_change_forces_a_version_bump() {
        assert_eq!(
            page_prompt_fingerprint(),
            PINNED_PAGE_PROMPT_FINGERPRINT,
            "the GLM-OCR page prompt or the markup→text parser changed: bump \
             OCR_PAGE_CONTRACT_VERSION (it invalidates every cached OCR page) and update \
             PINNED_PAGE_PROMPT_FINGERPRINT in the same change"
        );
    }
}
