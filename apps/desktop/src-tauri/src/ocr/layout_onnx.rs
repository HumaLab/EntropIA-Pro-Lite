//! Native ONNX layout detection engine using PP-DocLayout-L.
//!
//! Loads the PP-DocLayout-L ONNX model (PicoDet architecture, Large variant)
//! and performs layout detection on document images. Detected regions are
//! used for reading-order-aware OCR extraction.
//!
//! PP-DocLayout-L (PicoDet) I/O format:
//!   Inputs:
//!     - im_shape: [1, 2] — (height, width) of the original image as float32
//!     - image: [1, 3, 640, 640] — resized + normalized NCHW tensor
//!     - scale_factor: [1, 2] — (scale_y, scale_x) = (orig_h/640, orig_w/640)
//!   Outputs:
//!     - fetch_name_0: [N, 6] where each row is [class_id, score, x1, y1, x2, y2]
//!     - fetch_name_1: [1] (int32) — number of detections N
//!
//! Pipeline: image bytes → decode → resize (640×640) → normalize →
//! ONNX inference (with all 3 inputs) → filter by confidence → map labels →
//! LayoutRegion vec
//!
//! Note: PicoDet applies NMS internally, so we don't need our own NMS.
//! The `im_shape` input is required by L/V3 variants (the smaller S variant
//! doesn't use it); without it, the model fails with "Missing Input" errors.
//!
//! NOTE: Currently unused in production — OCRL mode no longer uses layout detection.
//! Kept for potential future re-enablement.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::provider::{BoundingBox, LayoutCategory, LayoutRegion};
use crate::runtime::{managed_resource_path, RuntimeManager};
use image::GenericImageView;
use ndarray::{Array, Array2, Array4};
use ort::session::Session;
use ort::value::TensorRef;

/// Confidence threshold for detections. Detections below this are discarded.
const CONFIDENCE_THRESHOLD: f32 = 0.50;

/// PP-DocLayout class ID → label mapping (23 classes).
///
/// Verified against the ONNX model output format: [class_id, score, x1, y1, x2, y2].
/// Unknown class IDs default to PlainText.
/// Source: PaddleX PP-DocLayout inference.yml label_list (same for S and L variants).
const LABEL_MAP: &[(usize, &str, LayoutCategory)] = &[
    (0, "doc_title", LayoutCategory::Title),
    (1, "paragraph_title", LayoutCategory::Title),
    (2, "text", LayoutCategory::PlainText),
    (3, "abandoned", LayoutCategory::Abandoned),
    (4, "figure_title", LayoutCategory::Caption),
    (5, "figure_note", LayoutCategory::Footnote),
    (6, "text", LayoutCategory::PlainText),
    (7, "page_header", LayoutCategory::Header),
    (8, "page_footer", LayoutCategory::Footer),
    (9, "table", LayoutCategory::Table),
    (10, "table_caption", LayoutCategory::Caption),
    (11, "table_note", LayoutCategory::Footnote),
    (12, "image", LayoutCategory::Figure),
    (13, "chart", LayoutCategory::Figure),
    (14, "vision_footnote", LayoutCategory::Footnote),
    (15, "formula", LayoutCategory::Abandoned),
    (16, "seal", LayoutCategory::Abandoned),
    (17, "paragraph_title", LayoutCategory::Title),
    (18, "code", LayoutCategory::Code),
    (19, "reference", LayoutCategory::Reference),
    (20, "abstract", LayoutCategory::PlainText),
    (21, "page_number", LayoutCategory::Footer),
    (22, "text", LayoutCategory::PlainText),
];

/// Default model input size. PP-DocLayout-L uses 640×640.
/// (S variant uses 480×480, V3 variant uses 800×800 — we don't ship those.)
const DEFAULT_INPUT_SIZE: u32 = 640;

/// Shared ONNX Runtime initialization.
/// Reuses the same ORT DLL as the NER module via OnceLock.
static ORT_INIT: OnceLock<Result<(), String>> = OnceLock::new();

/// Ensure ONNX Runtime is initialized exactly once.
fn ensure_ort_init(model_dir: &Path) -> Result<(), String> {
    ORT_INIT
        .get_or_init(|| initialize_ort(model_dir.to_path_buf()))
        .clone()
}

fn initialize_ort(model_dir: PathBuf) -> Result<(), String> {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        ort::init()
            .commit()
            .map_err(|e| format!("Failed to initialize ORT from ORT_DYLIB_PATH: {e}"))?;
        return Ok(());
    }

    let dylib_path = find_ort_dylib(&model_dir).ok_or_else(|| {
        format!(
            "No ONNX Runtime dynamic library found near {}. Expected onnxruntime.dll or set ORT_DYLIB_PATH.",
            model_dir.display()
        )
    })?;

    ort::init_from(dylib_path.display().to_string())
        .commit()
        .map_err(|e| {
            format!(
                "Failed to initialize ORT from {}: {e}",
                dylib_path.display()
            )
        })?;

    Ok(())
}

fn find_ort_dylib(model_dir: &Path) -> Option<PathBuf> {
    let candidates = runtime_candidates(model_dir);
    candidates.into_iter().find(|path| path.exists())
}

fn runtime_candidates(model_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut push_names = |base: &Path| {
        for name in runtime_file_names() {
            candidates.push(base.join(name));
        }
    };

    push_names(model_dir);
    if let Some(parent) = model_dir.parent() {
        push_names(parent);
        // Also search sibling directories — Windows dev currently keeps the
        // shared onnxruntime.dll handoff under resources/models/ner/.
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    push_names(&entry.path());
                }
            }
        }
    }

    candidates
}

fn runtime_file_names() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["onnxruntime.dll"]
    }
    #[cfg(target_os = "linux")]
    {
        &["libonnxruntime.so", "libonnxruntime.so.1"]
    }
    #[cfg(target_os = "macos")]
    {
        &["libonnxruntime.dylib"]
    }
}

/// Map a class ID (from ONNX output) to a LayoutCategory.
///
/// Uses the LABEL_MAP lookup. Unknown IDs default to PlainText.
fn map_class_id(class_id: usize) -> LayoutCategory {
    LABEL_MAP
        .iter()
        .find(|(id, _, _)| *id == class_id)
        .map(|(_, _, cat)| cat.clone())
        .unwrap_or(LayoutCategory::PlainText)
}

/// Map a string label (from metadata) to a LayoutCategory.
#[allow(dead_code)]
fn map_label(label: &str) -> LayoutCategory {
    match label {
        "doc_title" | "paragraph_title" => LayoutCategory::Title,
        "text" | "abstract" => LayoutCategory::PlainText,
        "table" => LayoutCategory::Table,
        "image" | "chart" => LayoutCategory::Figure,
        "figure_title" | "table_caption" => LayoutCategory::Caption,
        "vision_footnote" | "figure_note" | "table_note" => LayoutCategory::Footnote,
        "page_header" => LayoutCategory::Header,
        "page_footer" | "page_number" => LayoutCategory::Footer,
        "code" => LayoutCategory::Code,
        "reference" => LayoutCategory::Reference,
        "abandoned" | "seal" | "formula" => LayoutCategory::Abandoned,
        _ => LayoutCategory::PlainText,
    }
}

/// `preprocess` output: (input_tensor, im_shape, scale_factor, original_size).
type PreprocessedImage = (Array4<f32>, Array2<f32>, Array2<f32>, (u32, u32));

/// Preprocess an image for PP-DocLayout-L (PicoDet) inference.
///
/// PicoDet uses direct resize + auxiliary inputs (`im_shape`, `scale_factor`).
/// The model handles coordinate mapping internally — we must supply ALL THREE
/// inputs or inference fails with "Missing Input" errors.
///
/// 1. Decode image from bytes
/// 2. Resize to model input size (640×640) — direct resize, no padding
/// 3. Normalize per-channel: /255 then standardize (mean/std per ImageNet)
/// 4. Convert to NCHW ndarray [1, 3, H, W]
/// 5. Build im_shape [orig_h, orig_w] and scale_factor [scale_y, scale_x]
///
/// Returns a tuple of (input_tensor, im_shape, scale_factor, original_size) where:
/// - input_tensor: NCHW float32 array ready for ONNX
/// - im_shape: [1, 2] float32 array containing original (height, width)
/// - scale_factor: [1, 2] float32 array containing (scale_y, scale_x)
/// - original_size: (width, height) of the original image for later coord scaling
fn preprocess(
    image_bytes: &[u8],
    target_h: u32,
    target_w: u32,
) -> Result<PreprocessedImage, String> {
    let img = image::load_from_memory(image_bytes)
        .map_err(|e| format!("Failed to decode image for layout detection: {e}"))?;

    let (orig_w, orig_h) = (img.width(), img.height());

    // Direct resize to model input size (no letterbox padding — PicoDet uses scale_factor)
    let resized = img.resize_exact(target_w, target_h, image::imageops::FilterType::Triangle);

    // ImageNet normalization: mean=[0.485, 0.456, 0.406], std=[0.229, 0.224, 0.225]
    let mean = [0.485f32, 0.456, 0.406];
    let std_dev = [0.229f32, 0.224, 0.225];

    let mut input = Array::zeros((1, 3, target_h as usize, target_w as usize));

    for y in 0..target_h {
        for x in 0..target_w {
            let pixel = resized.get_pixel(x, y);
            let channels = pixel.0;
            for c in 0..3usize {
                let val = channels[c] as f32 / 255.0;
                let normalized = (val - mean[c]) / std_dev[c];
                input[[0, c, y as usize, x as usize]] = normalized;
            }
        }
    }

    // im_shape: original image size as (height, width) — required by L/V3 variants
    let im_shape = Array::from_shape_vec((1, 2), vec![orig_h as f32, orig_w as f32])
        .map_err(|e| format!("Failed to create im_shape tensor: {e}"))?;

    // scale_factor: how to map output coords back to original space
    // Format: [scale_y, scale_x] — PicoDet convention (height first)
    let scale_y = orig_h as f32 / target_h as f32;
    let scale_x = orig_w as f32 / target_w as f32;
    let scale_factor = Array::from_shape_vec((1, 2), vec![scale_y, scale_x])
        .map_err(|e| format!("Failed to create scale_factor tensor: {e}"))?;

    Ok((input, im_shape, scale_factor, (orig_w, orig_h)))
}

/// A raw detection from the ONNX model output.
#[derive(Clone)]
struct RawDetection {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    score: f32,
    class_id: usize,
}

/// Native ONNX layout detection engine.
///
/// Uses PP-DocLayout-L (PicoDet architecture, Large variant) to detect document
/// layout regions. Thread-safe: the ONNX session is wrapped in a Mutex.
pub struct OnnxLayoutEngine {
    session: Mutex<Session>,
    model_input_size: (u32, u32),
}

impl OnnxLayoutEngine {
    /// Create a new layout engine by loading the ONNX model.
    pub fn new(model_path: &Path) -> Result<Self, String> {
        if !model_path.exists() {
            return Err(format!("Layout model not found: {}", model_path.display()));
        }

        let model_dir = model_path
            .parent()
            .ok_or_else(|| {
                format!(
                    "Model path has no parent directory: {}",
                    model_path.display()
                )
            })?
            .to_path_buf();

        ensure_ort_init(&model_dir)?;

        let session = Session::builder()
            .map_err(|e| format!("Failed to create layout session builder: {e}"))?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .map_err(|e| format!("Failed to set layout optimization level: {e}"))?
            .commit_from_file(model_path)
            .map_err(|e| format!("Failed to load layout model {}: {e}", model_path.display()))?;

        // Detect model input size from session metadata.
        // PP-DocLayout-L expects [?, 3, 640, 640] for the "image" input (dynamic batch).
        // If the model reports dynamic dims (-1 or 0), we default to DEFAULT_INPUT_SIZE.
        let model_input_size = {
            let input_info = &session.inputs[0];
            match &input_info.input_type {
                ort::value::ValueType::Tensor { shape, .. } => {
                    let h = shape
                        .get(2)
                        .map(|&d| if d > 0 { d as u32 } else { DEFAULT_INPUT_SIZE })
                        .unwrap_or(DEFAULT_INPUT_SIZE);
                    let w = shape
                        .get(3)
                        .map(|&d| if d > 0 { d as u32 } else { DEFAULT_INPUT_SIZE })
                        .unwrap_or(DEFAULT_INPUT_SIZE);
                    (h, w)
                }
                _ => (DEFAULT_INPUT_SIZE, DEFAULT_INPUT_SIZE),
            }
        };

        eprintln!(
            "[ocr/layout_onnx] ✅ Layout engine initialized (input: {}x{})",
            model_input_size.0, model_input_size.1
        );

        Ok(Self {
            session: Mutex::new(session),
            model_input_size,
        })
    }

    /// Detect layout regions in an image.
    ///
    /// Full pipeline: decode → preprocess → ONNX inference → filter → label mapping.
    /// Returns layout regions in original image coordinates.
    ///
    /// PP-DocLayout-L (PicoDet) output format:
    ///   Output 0: [N, 6] where each row is [class_id, score, x1, y1, x2, y2]
    ///   Output 1: [1] (int32) — number of detections N
    ///
    /// Coordinates may be in model input space (640×640) or already in original
    /// image space depending on how the model was exported. We auto-detect via
    /// a heuristic (max_coord > model_input_size → already original) and scale
    /// accordingly.
    pub fn detect(&self, image_bytes: &[u8]) -> Result<Vec<LayoutRegion>, String> {
        let (target_h, target_w) = self.model_input_size;
        eprintln!(
            "[ocr/layout_onnx] detect(): input_size=({target_h},{target_w}), image_bytes_len={}",
            image_bytes.len()
        );

        // 1. Preprocess: decode, resize, normalize, build all 3 input tensors
        let (input_tensor, im_shape, scale_factor, (orig_w, orig_h)) =
            preprocess(image_bytes, target_h, target_w)?;

        // 2. Run ONNX inference with ALL THREE inputs: im_shape + image + scale_factor
        //
        // CRITICAL: the order and naming matter. PP-DocLayout-L's ONNX graph expects
        // inputs by NAME, not position. We use ort::inputs! with explicit names to
        // avoid "Missing Input" errors if the ORT runtime changes input order.
        let (num_dets, detections_data) = {
            let mut session = self
                .session
                .lock()
                .map_err(|_| "Layout session mutex poisoned")?;

            // Get output names before inference (they're session metadata)
            let output_names: Vec<String> =
                session.outputs.iter().map(|o| o.name.clone()).collect();

            // Get input names so we can bind tensors by name
            let input_names: Vec<String> = session.inputs.iter().map(|i| i.name.clone()).collect();
            eprintln!("[ocr/layout_onnx] Model expects inputs: {input_names:?}");

            let image_ref = TensorRef::from_array_view(&input_tensor)
                .map_err(|e| format!("Failed to create image input tensor: {e}"))?;
            let im_shape_ref = TensorRef::from_array_view(&im_shape)
                .map_err(|e| format!("Failed to create im_shape tensor: {e}"))?;
            let scale_factor_ref = TensorRef::from_array_view(&scale_factor)
                .map_err(|e| format!("Failed to create scale_factor tensor: {e}"))?;

            // Bind inputs by name (ort::inputs! supports ("name", tensor) pairs)
            let outputs = session
                .run(ort::inputs![
                    "im_shape" => im_shape_ref,
                    "image" => image_ref,
                    "scale_factor" => scale_factor_ref,
                ])
                .map_err(|e| format!("Layout ONNX inference failed: {e}"))?;

            // Extract output data into owned arrays before dropping the lock
            // Output 0: [N, 6] — detections as [class_id, score, x1, y1, x2, y2]
            // Output 1: [1] (int32) — number of detections
            let mut num_dets: usize = 0;
            let mut dets_array: Option<ndarray::ArrayD<f32>> = None;

            for i in 0..outputs.len() {
                let name = &output_names[i];
                // Output 1 (count) is int32 — handle separately
                if i == 1 && outputs.len() == 2 {
                    let count_arr = outputs[i]
                        .try_extract_array::<i32>()
                        .map_err(|e| format!("Failed to extract detection count: {e}"))?;
                    num_dets = count_arr.first().copied().unwrap_or(0) as usize;
                    eprintln!("[ocr/layout_onnx] Output 1 ({name}): num_dets={num_dets}");
                } else {
                    let arr = outputs[i].try_extract_array::<f32>().map_err(|e| {
                        format!("Failed to extract layout output tensor {i} ({name}): {e}")
                    })?;
                    eprintln!(
                        "[ocr/layout_onnx] Output {i} ({name}): shape={:?}",
                        arr.shape()
                    );
                    dets_array = Some(arr.to_owned());
                }
            }

            (num_dets, dets_array)
        }; // session lock dropped here

        // 3. Parse PicoDet output: [N, 6] with [class_id, score, x1, y1, x2, y2]
        let dets =
            detections_data.ok_or_else(|| "No detection output tensor from model".to_string())?;
        let mut detections = Vec::new();

        eprintln!(
            "[ocr/layout_onnx] Output shape: {:?}, num_dets from count output: {}",
            dets.shape(),
            num_dets
        );

        // The output may be [N, 6] or could have a batch dimension [1, N, 6]
        let dets_2d: Vec<[f32; 6]> = if dets.shape().len() == 3 && dets.shape()[2] == 6 {
            // [1, N, 6] — squeeze batch dimension
            (0..dets.shape()[1])
                .map(|i| {
                    let row: [f32; 6] = [
                        dets[[0, i, 0]],
                        dets[[0, i, 1]],
                        dets[[0, i, 2]],
                        dets[[0, i, 3]],
                        dets[[0, i, 4]],
                        dets[[0, i, 5]],
                    ];
                    row
                })
                .collect()
        } else if dets.shape().len() == 2 && dets.shape()[1] == 6 {
            // [N, 6] — already 2D
            let n = dets.shape()[0].min(num_dets.max(dets.shape()[0]));
            (0..n)
                .map(|i| {
                    let row: [f32; 6] = [
                        dets[[i, 0]],
                        dets[[i, 1]],
                        dets[[i, 2]],
                        dets[[i, 3]],
                        dets[[i, 4]],
                        dets[[i, 5]],
                    ];
                    row
                })
                .collect()
        } else {
            // Fallback: try to reshape and parse
            eprintln!(
                "[ocr/layout_onnx] Unexpected detection shape: {:?}, attempting fallback",
                dets.shape()
            );
            let total = dets.len();
            if total >= 6 && total % 6 == 0 {
                let n = total / 6;
                (0..n)
                    .map(|i| {
                        let row: [f32; 6] = [
                            dets[[i * 6]],
                            dets[[i * 6 + 1]],
                            dets[[i * 6 + 2]],
                            dets[[i * 6 + 3]],
                            dets[[i * 6 + 4]],
                            dets[[i * 6 + 5]],
                        ];
                        row
                    })
                    .collect()
            } else {
                return Err(format!(
                    "Cannot parse detection output with shape {:?}",
                    dets.shape()
                ));
            }
        };

        // Use num_dets from output if available, otherwise use parsed rows
        let effective_dets = if num_dets > 0 && num_dets < dets_2d.len() {
            num_dets
        } else {
            dets_2d.len()
        };

        for row in dets_2d.iter().take(effective_dets) {
            // PicoDet format: [class_id, score, x1, y1, x2, y2]
            let class_id = row[0] as usize;
            let score = row[1];
            let x1 = row[2];
            let y1 = row[3];
            let x2 = row[4];
            let y2 = row[5];

            if score >= CONFIDENCE_THRESHOLD && x2 > x1 && y2 > y1 {
                detections.push(RawDetection {
                    x1,
                    y1,
                    x2,
                    y2,
                    score,
                    class_id,
                });
            }
        }

        eprintln!(
            "[ocr/layout_onnx] Parsed {} detections (raw: {}, filtered by confidence >= {})",
            detections.len(),
            effective_dets,
            CONFIDENCE_THRESHOLD
        );

        // 4. Scale boxes from model input space back to original image coordinates.
        //
        // CRITICAL: PicoDet with the `scale_factor` input tensor performs coordinate
        // remapping INTERNALLY. The output coordinates are ALREADY in the original
        // image space — NO additional scaling is needed.
        //
        // We log the raw output coordinates and the original image dimensions to
        // verify this assumption empirically. If raw coords are within [0, orig_w/h],
        // they are already in original space (correct). If they are within [0, target_w/h]
        // (i.e. ≤480), the model is NOT applying scale_factor and we'd need to scale.
        if let Some(first) = detections.first() {
            eprintln!(
                "[ocr/layout_onnx] First raw detection: ({:.1},{:.1})-({:.1},{:.1}) | orig_image: {}x{} | model_input: {}x{}",
                first.x1, first.y1, first.x2, first.y2, orig_w, orig_h, target_w, target_h
            );
            // Heuristic: if max coord exceeds model input size, coords are likely already in original space
            let max_coord = first.x2.max(first.y2);
            let likely_original_space = max_coord > target_w as f32 || max_coord > target_h as f32;
            eprintln!(
                "[ocr/layout_onnx] max_coord={:.1} → coords likely in {} space",
                max_coord,
                if likely_original_space {
                    "ORIGINAL"
                } else {
                    "MODEL_INPUT (need scaling)"
                }
            );
        }

        // Detect coordinate space: if any coord exceeds the model input size, the model
        // already applied scale_factor and we must NOT scale again. Otherwise, scale.
        let needs_scaling = !detections
            .iter()
            .any(|d| d.x2 > target_w as f32 || d.y2 > target_h as f32);

        let scale_y = if needs_scaling {
            orig_h as f32 / target_h as f32
        } else {
            1.0
        };
        let scale_x = if needs_scaling {
            orig_w as f32 / target_w as f32
        } else {
            1.0
        };

        eprintln!(
            "[ocr/layout_onnx] Coordinate handling: needs_scaling={needs_scaling}, scale_x={scale_x:.3}, scale_y={scale_y:.3}"
        );

        let regions: Vec<LayoutRegion> = detections
            .into_iter()
            .enumerate()
            .map(|(i, det)| {
                // Scale coordinates to original image space (or no-op if already there)
                let x1_orig = det.x1 * scale_x;
                let y1_orig = det.y1 * scale_y;
                let x2_orig = det.x2 * scale_x;
                let y2_orig = det.y2 * scale_y;

                // Clamp to image bounds
                let x1 = x1_orig.max(0.0);
                let y1 = y1_orig.max(0.0);
                let x2 = x2_orig.min(orig_w as f32);
                let y2 = y2_orig.min(orig_h as f32);

                let bbox = BoundingBox {
                    x: x1.round() as i32,
                    y: y1.round() as i32,
                    width: (x2 - x1).round().max(1.0) as u32,
                    height: (y2 - y1).round().max(1.0) as u32,
                };

                LayoutRegion {
                    label: map_class_id(det.class_id),
                    bbox,
                    confidence: det.score,
                    order: i, // temporary order; compute_reading_order will assign final
                }
            })
            .collect();

        Ok(regions)
    }
}

/// Create a layout engine from the Tauri app handle.
///
/// Uses 3-tier path resolution (same as PaddleVL):
/// 1. BaseDirectory::Resource (production)
/// 2. CARGO_MANIFEST_DIR/resources/models/ocr/ (dev)
///
/// Returns None if the model file is not found (graceful degradation).
pub fn create_layout_engine(app_handle: &tauri::AppHandle) -> Option<OnnxLayoutEngine> {
    let runtime_root = managed_runtime_root_for_layout(app_handle).ok().flatten();

    if let Some(path) = resolve_layout_model_path_from_roots(
        runtime_root.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    ) {
        eprintln!(
            "[ocr/layout_onnx] Found layout model at: {}",
            path.display()
        );
        match OnnxLayoutEngine::new(&path) {
            Ok(engine) => return Some(engine),
            Err(e) => {
                eprintln!("[ocr/layout_onnx] ❌ Failed to load model from resolved path: {e}");
            }
        }
    }

    eprintln!("[ocr/layout_onnx] ⚠️ Layout model not found — native layout detection unavailable");
    None
}

fn managed_runtime_root_for_layout(
    app_handle: &tauri::AppHandle,
) -> Result<Option<PathBuf>, String> {
    managed_runtime_root_for_layout_with(
        || RuntimeManager::new().ensure_ready_or_bootstrap(app_handle),
        || RuntimeManager::new().hydrated_runtime_root(app_handle),
    )
}

fn managed_runtime_root_for_layout_with<E, H>(
    ensure_ready_or_bootstrap: E,
    hydrated_runtime_root: H,
) -> Result<Option<PathBuf>, String>
where
    E: FnOnce() -> Result<crate::runtime::status::RuntimeStatus, String>,
    H: FnOnce() -> Result<Option<PathBuf>, String>,
{
    let status = ensure_ready_or_bootstrap()?;
    if status.state != crate::runtime::status::RuntimeState::Healthy {
        return Ok(None);
    }

    hydrated_runtime_root()
}

fn resolve_layout_model_path_from_roots(
    managed_root: Option<&Path>,
    manifest_dir: &Path,
) -> Option<PathBuf> {
    const MODEL_FILENAME: &str = "PP-DocLayout-L.onnx";

    if let Some(root) = managed_root {
        let managed = managed_resource_path(root, "models/ocr").join(MODEL_FILENAME);
        if managed.exists() {
            return Some(managed);
        }
    }

    let dev_path = manifest_dir
        .join("resources")
        .join("models")
        .join("ocr")
        .join(MODEL_FILENAME);
    if dev_path.exists() {
        return Some(dev_path);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::provider::LayoutCategory;
    use crate::runtime::status::{RuntimeCapability, RuntimeState, RuntimeStatus};
    use std::cell::RefCell;
    use tempfile::tempdir;

    #[test]
    fn test_map_label_known_labels() {
        assert_eq!(map_label("doc_title"), LayoutCategory::Title);
        assert_eq!(map_label("paragraph_title"), LayoutCategory::Title);
        assert_eq!(map_label("text"), LayoutCategory::PlainText);
        assert_eq!(map_label("abstract"), LayoutCategory::PlainText);
        assert_eq!(map_label("table"), LayoutCategory::Table);
        assert_eq!(map_label("image"), LayoutCategory::Figure);
        assert_eq!(map_label("chart"), LayoutCategory::Figure);
        assert_eq!(map_label("figure_title"), LayoutCategory::Caption);
        assert_eq!(map_label("table_caption"), LayoutCategory::Caption);
        assert_eq!(map_label("vision_footnote"), LayoutCategory::Footnote);
        assert_eq!(map_label("figure_note"), LayoutCategory::Footnote);
        assert_eq!(map_label("table_note"), LayoutCategory::Footnote);
        assert_eq!(map_label("page_header"), LayoutCategory::Header);
        assert_eq!(map_label("page_footer"), LayoutCategory::Footer);
        assert_eq!(map_label("page_number"), LayoutCategory::Footer);
        assert_eq!(map_label("code"), LayoutCategory::Code);
        assert_eq!(map_label("reference"), LayoutCategory::Reference);
        assert_eq!(map_label("abandoned"), LayoutCategory::Abandoned);
        assert_eq!(map_label("seal"), LayoutCategory::Abandoned);
        assert_eq!(map_label("formula"), LayoutCategory::Abandoned);
    }

    #[test]
    fn test_map_label_unknown_defaults_to_plain_text() {
        assert_eq!(map_label("unknown_label"), LayoutCategory::PlainText);
        assert_eq!(map_label("random_stuff"), LayoutCategory::PlainText);
        assert_eq!(map_label(""), LayoutCategory::PlainText);
    }

    #[test]
    fn test_map_class_id_known_ids() {
        assert_eq!(map_class_id(0), LayoutCategory::Title);
        assert_eq!(map_class_id(1), LayoutCategory::Title);
        assert_eq!(map_class_id(2), LayoutCategory::PlainText);
        assert_eq!(map_class_id(9), LayoutCategory::Table);
        assert_eq!(map_class_id(12), LayoutCategory::Figure);
        assert_eq!(map_class_id(14), LayoutCategory::Footnote);
        assert_eq!(map_class_id(7), LayoutCategory::Header);
        assert_eq!(map_class_id(8), LayoutCategory::Footer);
        assert_eq!(map_class_id(18), LayoutCategory::Code);
        assert_eq!(map_class_id(19), LayoutCategory::Reference);
        assert_eq!(map_class_id(3), LayoutCategory::Abandoned);
    }

    #[test]
    fn test_map_class_id_unknown_defaults_to_plain_text() {
        assert_eq!(map_class_id(999), LayoutCategory::PlainText);
        assert_eq!(map_class_id(50), LayoutCategory::PlainText);
        assert_eq!(map_class_id(100), LayoutCategory::PlainText);
    }

    #[test]
    fn resolve_layout_model_prefers_managed_runtime_assets() {
        let runtime_dir = tempdir().expect("runtime dir");
        let manifest_dir = tempdir().expect("manifest dir");
        let managed_model = runtime_dir
            .path()
            .join("resources")
            .join("models")
            .join("ocr")
            .join("PP-DocLayout-L.onnx");
        std::fs::create_dir_all(managed_model.parent().expect("model parent"))
            .expect("create model dir");
        std::fs::write(&managed_model, b"onnx").expect("write model");

        let resolved =
            resolve_layout_model_path_from_roots(Some(runtime_dir.path()), manifest_dir.path());

        assert_eq!(resolved, Some(managed_model));
    }

    #[test]
    fn layout_runtime_resolution_bootstraps_before_using_managed_model() {
        let calls = RefCell::new(Vec::new());
        let expected = PathBuf::from("/tmp/runtime-ready");

        let resolved = managed_runtime_root_for_layout_with(
            || {
                calls.borrow_mut().push("ensure_ready");
                Ok(RuntimeStatus {
                    state: RuntimeState::Healthy,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: true,
                    summary: "Runtime listo".to_string(),
                    blocked_capabilities: vec![],
                    details: vec![],
                    guidance: vec![],
                    bootstrap_eligible: false,
                    bootstrap_required: false,
                    active_operation: None,
                })
            },
            || {
                calls.borrow_mut().push("hydrated_root");
                Ok(Some(expected.clone()))
            },
        )
        .expect("runtime resolution should succeed");

        assert_eq!(resolved, Some(expected));
        assert_eq!(calls.into_inner(), vec!["ensure_ready", "hydrated_root"]);
    }

    #[test]
    fn layout_runtime_resolution_respects_blocked_bootstrap_status() {
        let calls = RefCell::new(Vec::new());

        let resolved = managed_runtime_root_for_layout_with(
            || {
                calls.borrow_mut().push("ensure_ready");
                Ok(RuntimeStatus {
                    state: RuntimeState::BlockedSourceUnavailable,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: false,
                    summary: "Bootstrap bloqueado".to_string(),
                    blocked_capabilities: vec![RuntimeCapability::Ocr],
                    details: vec!["manifest remoto no publicado".to_string()],
                    guidance: vec!["Reintentá".to_string()],
                    bootstrap_eligible: false,
                    bootstrap_required: true,
                    active_operation: None,
                })
            },
            || {
                calls.borrow_mut().push("hydrated_root");
                Ok(Some(PathBuf::from("/tmp/stale-runtime")))
            },
        )
        .expect("blocked bootstrap should degrade gracefully");

        assert_eq!(resolved, None);
        assert_eq!(calls.into_inner(), vec!["ensure_ready"]);
    }
}
