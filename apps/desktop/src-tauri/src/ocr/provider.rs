//! OCR provider trait and shared output types.
//!
//! Defines the `OcrProvider` trait that decouples OCR logic from the worker,
//! and the `OcrOutput`, `OcrRegion`, `BoundingBox` structs used by all providers
//! to return structured recognition results.

use serde::Serialize;

/// Bounding box from OCR detection.
///
/// Represents a rectangle in pixel coordinates within the source image.
/// Origin (0,0) is the top-left corner of the image.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BoundingBox {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Single OCR region with optional bounding box and column assignment.
///
/// Each region represents a line or block of text recognized by the OCR engine.
/// The `column` field is assigned during post-processing to indicate which
/// column the region belongs to (0 = leftmost).
#[derive(Debug, Clone, Serialize)]
pub struct OcrRegion {
    pub text: String,
    pub confidence: f32,
    pub bbox: Option<BoundingBox>,
    pub column: Option<usize>,
}

/// Unified OCR output from any provider.
///
/// Contains the full recognized text, structured regions with bounding boxes,
/// and the method used to produce the result (e.g. "paddle" or "paddle_vl").
#[derive(Debug, Clone, Serialize)]
pub struct OcrOutput {
    pub text: String,
    pub regions: Vec<OcrRegion>,
    pub method: String,
}

/// Layout category from document layout detection.
///
/// In the lean build only the GLM-OCR label mapping is reachable, which
/// constructs a subset of these variants (Title/PlainText/Table/Figure). The
/// remaining variants are still matched/serialized and are produced by the
/// paddle layout path, so they stay in the enum — allow them to be "never
/// constructed" in lean rather than cfg-gating individual variants (which would
/// force cfg arms across every exhaustive match on this enum).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum LayoutCategory {
    Title,     // doc_title, paragraph_title → "## " prefix
    PlainText, // text, abstract → as-is
    Table,     // table → "---\n...\n---" wrapper
    Figure,    // image, chart → skip in text output
    Caption,   // figure_title, table_caption → as-is
    Footnote,  // vision_footnote, figure_note, table_note → "Note: " prefix
    Header,    // page_header → skip
    Footer,    // page_footer, page_number → skip
    Code,      // code → code block markers
    Reference, // reference → as-is
    Abandoned, // abandoned, seal, formula → skip
}

/// A single layout region detected by the layout engine.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LayoutRegion {
    pub label: LayoutCategory,
    pub bbox: BoundingBox,
    pub confidence: f32,
    pub order: usize,
}

/// Output from the layout detection engine.
#[derive(Debug, Clone, Serialize)]
pub struct LayoutOutput {
    pub regions: Vec<LayoutRegion>,
    pub image_width: u32,
    pub image_height: u32,
}

/// Provider trait — sync, called inside `spawn_blocking`.
///
/// Each OCR engine implements this trait. The worker holds a `Box<dyn OcrProvider>`
/// and calls `recognize` for each job. Init-time fallback selects which provider
/// to use; once selected, all jobs go through the same provider.
pub trait OcrProvider: Send + Sync {
    /// Recognize text from raw image bytes.
    ///
    /// The bytes can be any format supported by `image::load_from_memory`
    /// (PNG, JPEG, TIFF, etc.). Returns structured output with regions and
    /// bounding boxes, or an error message on failure.
    fn recognize(&self, image_bytes: &[u8]) -> Result<OcrOutput, String>;

    /// Recognize text without applying orientation correction.
    ///
    /// Used for per-region OCR after layout detection, where each crop is
    /// expected to be already upright (inherited from the parent image's
    /// orientation). The orientation classifier produces unreliable results
    /// on small crops because it lacks document-level context.
    ///
    /// Default implementation falls back to `recognize` for providers that
    /// do not expose a separate no-orientation path.
    ///
    /// NOTE: Currently unused in production — OCRL mode no longer crops regions.
    /// Kept for potential future re-enablement.
    #[allow(dead_code)]
    fn recognize_no_ori(&self, image_bytes: &[u8]) -> Result<OcrOutput, String> {
        self.recognize(image_bytes)
    }

    /// Short identifier for the provider (e.g. "paddle").
    ///
    /// Only the paddle worker constructs/queries a provider; the lean GLM worker
    /// calls the remote provider directly, so this is unused there.
    #[allow(dead_code)]
    fn name(&self) -> &str;
}
