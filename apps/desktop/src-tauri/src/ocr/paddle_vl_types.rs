//! Pure data types for PaddleOCR-VL results.
//!
//! Always-compiled (no runtime/process/python_discovery deps) so the shared
//! persisted-layout structs and the GLM-OCR remote path can construct/consume
//! them in BOTH build variants. The engine impl lives in `paddle_vl.rs` and is
//! gated behind the `paddle-ocr` feature; this module is its always-on data half
//! (mirrors the P1 `runtime::bootstrap_types` split).

use serde::{Deserialize, Serialize};

/// Parsed result from the paddle_vl.py script.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // fields deserialized from Python but not all read in Rust
pub struct PaddleVlOutput {
    pub text: String,
    pub method: String,
    pub blocks: Vec<PaddleVlBlock>,
    pub regions: Vec<PaddleVlRegion>,
    pub image_width: u32,
    pub image_height: u32,
    /// The device that was actually used by the Python subprocess.
    /// May differ from the requested device if GPU init failed and fell back to CPU.
    #[serde(default)]
    pub actual_device: Option<String>,
}

/// A single block from PaddleOCR-VL with text content.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // fields deserialized from Python but not all read in Rust
pub struct PaddleVlBlock {
    pub label: String,
    pub content: String,
    pub bbox: PaddleVlBbox,
    pub order: i32,
    pub group_id: i32,
}

/// Bounding box in the format returned by paddle_vl.py.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaddleVlBbox {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A layout region from PaddleOCR-VL detection.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)] // fields deserialized from Python but not all read in Rust
pub struct PaddleVlRegion {
    pub category: String,
    pub bbox: PaddleVlBbox,
    pub confidence: f32,
}

/// One layout box from the layout-only `layout_v3.py` script: no content,
/// recognition happens per box with the lightweight OCR engine.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LayoutV3Box {
    pub label: String,
    pub category: String,
    pub bbox: PaddleVlBbox,
    pub confidence: f32,
    pub order: i32,
}

/// Parsed result from the layout-only `layout_v3.py` script.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LayoutV3Output {
    pub method: String,
    pub image_width: u32,
    pub image_height: u32,
    /// The device that was actually used by the Python subprocess.
    #[serde(default)]
    pub actual_device: Option<String>,
    pub boxes: Vec<LayoutV3Box>,
}
