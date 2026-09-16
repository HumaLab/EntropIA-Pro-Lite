//! CSL rendering for the manuscript's bibliography (plan-editor.md §11.5, §11.6).
//!
//! The engine is `hayagriva`, chosen in §27.3 on licensing: it is MIT OR
//! Apache-2.0, where citeproc-js is CPAL-1.0 OR AGPL-1.0 and is reached even
//! through the MIT-labelled `@citation-js/plugin-csl`. Spike S3 confirmed it
//! covers clusters, locators, style switching and cited-only bibliographies,
//! and that it reads Zotero's CSL-JSON with no conversion layer at all.
//!
//! What it does not cover is recorded here rather than discovered later:
//!
//! - **Per-item affixes.** No field for them; [`affix`] injects them after
//!   rendering, with a case per style shape.
//! - **Author suppression.** `CitePurpose::Year` drops the locator, so it is
//!   not CSL's "everything except the author". S3 left three options open for
//!   the user to choose between, so nothing is implemented here that would
//!   pre-empt that decision.
//! - **Intra-cluster reordering.** The style controls the order and clearing
//!   the style's sort does not restore the author's. S3 notes this may be
//!   correct CSL behaviour rather than a defect.

pub mod affix;
