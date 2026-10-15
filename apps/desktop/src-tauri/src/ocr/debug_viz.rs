//! Debug visualization helpers for the OCR pipeline.
//!
//! Only compiled into debug builds (`cfg!(debug_assertions)`). Additionally
//! gated by the `ENTROPIA_DEBUG_VIZ` environment variable — set it to any
//! non-empty value to enable debug visualization output. Without the env var,
//! debug visualization is a silent no-op even in debug builds, avoiding
//! unnecessary file I/O and log noise during normal development.
//!
//! When enabled, writes artifacts per processed asset into the workspace root:
//!
//! **Layout detection** (legacy, currently unused):
//!   - `tests_layouts/<asset_id>_<ts>.png` — image with layout bboxes
//!   - `recortes/<asset_id>_<ts>/...` — cropped regions by column/order
//!
//! **OCR line detection** (active):
//!   - `tests_layouts/<asset_id>_<ts>.png` — image with detected text-line
//!     bounding boxes drawn on top, in the style of PaddleOCR's vis_result.

#![allow(dead_code)]

use std::path::PathBuf;

use image::{DynamicImage, Rgba, RgbaImage};
use imageproc::drawing::draw_hollow_rect_mut;
use imageproc::rect::Rect;
use serde_json::json;

use super::glm_ocr::GlmOcrResponse;
use super::provider::{BoundingBox, LayoutCategory, LayoutRegion, OcrRegion};

/// Thickness of the rectangle outline in pixels.
const RECT_THICKNESS: i32 = 4;

/// Check if debug visualization is enabled.
///
/// Requires BOTH debug build AND `ENTROPIA_DEBUG_VIZ` env var set to a non-empty value.
/// This prevents debug visualization from running unconditionally in dev builds,
/// which creates unnecessary files and log noise for normal development.
fn is_debug_viz_enabled() -> bool {
    std::env::var("ENTROPIA_DEBUG_VIZ")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Inner corner offset (small solid square) to make region INDEX visible
/// without needing a font. The square is drawn in the region's color at the
/// top-left corner of the bbox, with side = `CORNER_SIZE`.
const CORNER_SIZE: u32 = 20;

/// Distinct colors for up to 8 columns. If more columns exist, they wrap.
const COLUMN_COLORS: &[[u8; 4]] = &[
    [220, 30, 30, 255],   // red
    [30, 160, 60, 255],   // green
    [30, 80, 200, 255],   // blue
    [200, 120, 30, 255],  // orange
    [160, 60, 180, 255],  // purple
    [30, 180, 180, 255],  // teal
    [180, 100, 30, 255],  // brown
    [240, 100, 180, 255], // pink
];

/// Map a layout category to a distinctive RGBA color (fallback when columns
/// are not available).
fn category_color(category: &LayoutCategory) -> Rgba<u8> {
    match category {
        LayoutCategory::Title => Rgba([220, 30, 30, 255]), // bright red
        LayoutCategory::PlainText => Rgba([30, 160, 60, 255]), // green
        LayoutCategory::Table => Rgba([30, 80, 200, 255]), // blue
        LayoutCategory::Figure => Rgba([200, 120, 30, 255]), // orange
        LayoutCategory::Caption => Rgba([160, 60, 180, 255]), // purple
        LayoutCategory::Footnote => Rgba([200, 200, 30, 255]), // yellow
        LayoutCategory::Header => Rgba([130, 130, 130, 255]), // gray
        LayoutCategory::Footer => Rgba([90, 90, 90, 255]), // dark gray
        LayoutCategory::Code => Rgba([30, 180, 180, 255]), // teal
        LayoutCategory::Reference => Rgba([180, 100, 30, 255]), // brown
        LayoutCategory::Abandoned => Rgba([240, 100, 180, 255]), // pink
    }
}

/// Assign a column index to each region based on their `order` field.
///
/// Regions are sorted by `order`, then grouped into columns by detecting
/// gaps in Y-coordinates that indicate a new row. Within each row, regions
/// are sorted by X to determine column assignment.
///
/// Returns a Vec of (region_index, column_index) pairs.
fn assign_column_indices(regions: &[LayoutRegion]) -> Vec<(usize, usize)> {
    if regions.is_empty() {
        return vec![];
    }

    // Sort by order (reading order)
    let mut indexed: Vec<(usize, usize)> = regions
        .iter()
        .enumerate()
        .map(|(i, r)| (i, r.order))
        .collect();
    indexed.sort_by_key(|&(_, order)| order);

    // Detect rows: a new row starts when Y jumps significantly
    // Use a simple heuristic: if Y difference > 50% of median region height, new row
    let heights: Vec<i32> = regions.iter().map(|r| r.bbox.height as i32).collect();
    let median_height = {
        let mut h = heights.clone();
        h.sort();
        h[h.len() / 2]
    };
    let row_threshold = (median_height as f32 * 0.5) as i32;

    // Group into rows by Y proximity
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for &(idx, _order) in &indexed {
        let y = regions[idx].bbox.y;
        let placed = rows.iter_mut().find(|row| {
            let first_y = regions[row[0]].bbox.y;
            (y - first_y).abs() <= row_threshold
        });

        if let Some(row) = placed {
            row.push(idx);
        } else {
            rows.push(vec![idx]);
        }
    }

    // Sort each row by X (left to right)
    for row in &mut rows {
        row.sort_by_key(|&idx| regions[idx].bbox.x);
    }

    // Assign column indices: first region in each row gets col 0, second gets col 1, etc.
    // Then merge columns across rows based on X-center proximity
    let mut col_assignments: Vec<(usize, usize)> = Vec::new();
    let mut col_centers: Vec<i32> = Vec::new(); // average X-center per column

    for row in &rows {
        for (local_col, &idx) in row.iter().enumerate() {
            let center = regions[idx].bbox.x + (regions[idx].bbox.width as i32) / 2;

            // Find best matching global column
            let mut best_col = local_col;
            let mut best_dist = i32::MAX;

            for (global_col, &existing_center) in col_centers.iter().enumerate() {
                let dist = (center - existing_center).abs();
                if dist < best_dist {
                    best_dist = dist;
                    best_col = global_col;
                }
            }

            // If no close column exists, create a new one
            if best_dist > 100 {
                best_col = col_centers.len();
                col_centers.push(center);
            } else {
                // Update the column center average
                col_centers[best_col] = (col_centers[best_col] + center) / 2;
            }

            col_assignments.push((idx, best_col));
        }
    }

    // Sort by region index for consistent lookup
    col_assignments.sort_by_key(|&(idx, _)| idx);

    col_assignments
}

/// Resolve the workspace root (3 levels up from `CARGO_MANIFEST_DIR`).
fn workspace_root() -> Option<PathBuf> {
    let manifest = option_env!("CARGO_MANIFEST_DIR")?;
    let mut path = PathBuf::from(manifest);
    for _ in 0..3 {
        if !path.pop() {
            return None;
        }
    }
    Some(path)
}

/// Current time in milliseconds since the epoch.
fn timestamp_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Draw a thick rectangle outline by drawing N concentric rectangles.
fn draw_thick_rect(image: &mut RgbaImage, rect: Rect, color: Rgba<u8>, thickness: i32) {
    for offset in 0..thickness {
        let x = rect.left() - offset;
        let y = rect.top() - offset;
        let w = (rect.width() as i32) + 2 * offset;
        let h = (rect.height() as i32) + 2 * offset;

        if w <= 0 || h <= 0 {
            continue;
        }

        let img_w = image.width() as i32;
        let img_h = image.height() as i32;
        if x >= img_w || y >= img_h || x + w <= 0 || y + h <= 0 {
            continue;
        }

        let clamped = Rect::at(x.max(0), y.max(0)).of_size(
            (w.min(img_w - x.max(0))).max(1) as u32,
            (h.min(img_h - y.max(0))).max(1) as u32,
        );
        draw_hollow_rect_mut(image, clamped, color);
    }
}

/// Draw a small solid color-coded square at the top-left corner of the bbox.
fn draw_corner_marker(image: &mut RgbaImage, bbox: &BoundingBox, color: Rgba<u8>) {
    let x0 = bbox.x.max(0) as u32;
    let y0 = bbox.y.max(0) as u32;
    let size = CORNER_SIZE.min(bbox.width).min(bbox.height);
    if size == 0 {
        return;
    }

    let img_w = image.width();
    let img_h = image.height();
    for dy in 0..size {
        for dx in 0..size {
            let x = x0 + dx;
            let y = y0 + dy;
            if x < img_w && y < img_h {
                image.put_pixel(x, y, color);
            }
        }
    }
}

/// Save a debug visualization of the layout detection result **after** the
/// heuristic pipeline (gap filling, column grouping, reading order).
///
/// Draws all bboxes on a copy of the original image, color-coded by column
/// (not by category), and writes it to `<workspace>/tests_layouts/<asset_id>_<ts>.png`.
/// Also writes each crop into
/// `<workspace>/recortes/<asset_id>_<ts>/col<col>_<order>_<Category>_conf<NN>.png`.
pub fn save_layout_debug(
    original_bytes: &[u8],
    decoded_image: &DynamicImage,
    regions: &[LayoutRegion],
    crops: &[(usize, Vec<u8>)],
    asset_id: &str,
) -> Result<(), String> {
    // Gate behind ENTROPIA_DEBUG_VIZ env var — silent no-op if not set
    if !is_debug_viz_enabled() {
        return Ok(());
    }

    let root = match workspace_root() {
        Some(r) => r,
        None => {
            log::debug!(target: "debug_viz", "CARGO_MANIFEST_DIR unavailable — skipping debug viz");
            return Ok(());
        }
    };

    let ts = timestamp_ms();
    let layouts_dir = root.join("tests_layouts");
    let crops_dir = root.join("recortes").join(format!("{asset_id}_{ts}"));

    if let Err(e) = std::fs::create_dir_all(&layouts_dir) {
        log::debug!(
            target: "debug_viz",
            "Failed to create {}: {e}",
            layouts_dir.display()
        );
        return Ok(());
    }
    if let Err(e) = std::fs::create_dir_all(&crops_dir) {
        log::debug!(
            target: "debug_viz",
            "Failed to create {}: {e}",
            crops_dir.display()
        );
        return Ok(());
    }

    // ── Assign column indices based on reading order ─────────────────────
    let col_map = assign_column_indices(regions);
    let col_lookup: std::collections::HashMap<usize, usize> = col_map.iter().cloned().collect();
    let num_columns = col_map
        .iter()
        .map(|&(_, c)| c)
        .max()
        .map(|c| c + 1)
        .unwrap_or(1);

    // ── Overlay image with bboxes (color-coded by column) ────────────────
    let mut overlay = decoded_image.to_rgba8();

    // Draw bboxes and build column legend (no per-region log dump)
    let mut column_legend = String::new();
    for col_idx in 0..num_columns {
        let color = COLUMN_COLORS.get(col_idx % COLUMN_COLORS.len()).unwrap();
        if !column_legend.is_empty() {
            column_legend.push_str(", ");
        }
        column_legend.push_str(&format!(
            "col{}=RGBA({},{},{},{})",
            col_idx, color[0], color[1], color[2], color[3]
        ));
    }
    log::debug!(
        target: "debug_viz",
        "Layout for {asset_id}: {} regions, {} columns ({})",
        regions.len(),
        num_columns,
        column_legend
    );

    // Sort regions by reading order for display
    let mut ordered_indices: Vec<usize> = (0..regions.len()).collect();
    ordered_indices.sort_by_key(|&i| regions[i].order);

    for &idx in &ordered_indices {
        let region = &regions[idx];
        let col_idx = col_lookup.get(&idx).copied().unwrap_or(0);
        let color = Rgba(*COLUMN_COLORS.get(col_idx % COLUMN_COLORS.len()).unwrap());

        if region.bbox.width == 0 || region.bbox.height == 0 {
            continue;
        }

        let rect =
            Rect::at(region.bbox.x, region.bbox.y).of_size(region.bbox.width, region.bbox.height);
        draw_thick_rect(&mut overlay, rect, color, RECT_THICKNESS);
        draw_corner_marker(&mut overlay, &region.bbox, color);
    }

    let short_id: String = asset_id.chars().take(8).collect();
    let overlay_path = layouts_dir.join(format!("{short_id}_{ts}.png"));
    if let Err(e) = overlay.save(&overlay_path) {
        log::debug!(
            target: "debug_viz",
            "Failed to write overlay {}: {e}",
            overlay_path.display()
        );
    } else {
        log::debug!(
            target: "debug_viz",
            "✅ Overlay saved: {}",
            overlay_path.display()
        );
    }

    // ── Crops (organized by column and reading order) ────────────────────
    for (region_idx, crop_bytes) in crops {
        let region = match regions.get(*region_idx) {
            Some(r) => r,
            None => continue,
        };
        let col_idx = col_lookup.get(region_idx).copied().unwrap_or(0);
        let label = format!("{:?}", region.label);
        let conf_pct = (region.confidence * 100.0).round() as u32;
        let filename = format!(
            "col{}_order{:02}_{}_conf{:02}.png",
            col_idx, region.order, label, conf_pct
        );
        let crop_path = crops_dir.join(&filename);

        if let Err(e) = std::fs::write(&crop_path, crop_bytes) {
            log::debug!(
                target: "debug_viz",
                "Failed to write crop {}: {e}",
                crop_path.display()
            );
        }
    }

    if !crops.is_empty() {
        log::debug!(
            target: "debug_viz",
            "✅ Saved {} crops to: {}",
            crops.len(),
            crops_dir.display()
        );
    }

    let _ = original_bytes.len();

    Ok(())
}

// ── OCR Line Detection Visualization ────────────────────────────────────────

/// Color for OCR line detection bounding boxes (green, matching PaddleOCR style).
const OCR_LINE_COLOR: Rgba<u8> = Rgba([0, 180, 60, 255]);

/// Thickness of the line detection box outline in pixels.
const OCR_LINE_THICKNESS: i32 = 2;

/// Corner marker size for OCR line boxes (smaller than layout — text lines are smaller).
const OCR_CORNER_SIZE: u32 = 8;

/// Save a debug visualization of OCR line detection results.
///
/// Draws all detected text-line bounding boxes on a copy of the original image
/// and writes it to `<workspace>/tests_layouts/<short_id>_<ts>.png`.
///
/// This is the PaddleOCR equivalent of `vis_result.jpg` — each detected text
/// line gets a colored bounding box with a small confidence marker in the
/// top-left corner. Produces a single overlay image (no crops).
///
/// Only active in debug builds. In release, this function is not called.
pub fn save_ocr_lines_debug(
    original_bytes: &[u8],
    regions: &[OcrRegion],
    method: &str,
    asset_id: &str,
) -> Result<(), String> {
    // Gate behind ENTROPIA_DEBUG_VIZ env var — silent no-op if not set
    if !is_debug_viz_enabled() {
        return Ok(());
    }

    let root = match workspace_root() {
        Some(r) => r,
        None => {
            log::debug!(target: "debug_viz", "CARGO_MANIFEST_DIR unavailable — skipping OCR lines debug");
            return Ok(());
        }
    };

    // Decode image from bytes
    let img = match image::load_from_memory(original_bytes) {
        Ok(img) => img,
        Err(e) => {
            log::debug!(target: "debug_viz", "Failed to decode image for OCR lines overlay: {e}");
            return Ok(());
        }
    };

    let ts = timestamp_ms();
    let layouts_dir = root.join("tests_layouts");

    if let Err(e) = std::fs::create_dir_all(&layouts_dir) {
        log::debug!(
            target: "debug_viz",
            "Failed to create {}: {e}",
            layouts_dir.display()
        );
        return Ok(());
    }

    // Draw bounding boxes on a copy of the image
    let mut overlay = img.to_rgba8();

    let lines_with_bbox: Vec<_> = regions.iter().filter(|r| r.bbox.is_some()).collect();

    log::debug!(
        target: "debug_viz",
        "─── OCR line detection for {asset_id}: {} lines with bboxes (method={method}) ───",
        lines_with_bbox.len()
    );

    for region in lines_with_bbox.iter() {
        let bbox = region.bbox.as_ref().unwrap();

        if bbox.width == 0 || bbox.height == 0 {
            continue;
        }

        // Clamp bbox to image bounds
        let x = bbox.x.max(0) as u32;
        let y = bbox.y.max(0) as u32;
        let w = bbox.width.min(overlay.width().saturating_sub(x));
        let h = bbox.height.min(overlay.height().saturating_sub(y));

        if w == 0 || h == 0 {
            continue;
        }

        let rect = Rect::at(x as i32, y as i32).of_size(w, h);
        draw_thick_rect(&mut overlay, rect, OCR_LINE_COLOR, OCR_LINE_THICKNESS);

        // Small confidence marker in the top-left corner
        let corner_bbox = BoundingBox {
            x: bbox.x,
            y: bbox.y,
            width: bbox.width,
            height: bbox.height,
        };
        draw_corner_marker_sized(&mut overlay, &corner_bbox, OCR_LINE_COLOR, OCR_CORNER_SIZE);
    }

    // Summary log only — no per-line text dump (reduces noise in normal flow)
    if !lines_with_bbox.is_empty() {
        log::debug!(
            target: "debug_viz",
            "  Drew {} bounding boxes on overlay image",
            lines_with_bbox.len()
        );
    }

    // Save overlay image
    let short_id: String = asset_id.chars().take(8).collect();
    let overlay_path = layouts_dir.join(format!("{short_id}_{ts}.png"));
    if let Err(e) = overlay.save(&overlay_path) {
        log::debug!(
            target: "debug_viz",
            "Failed to write overlay {}: {e}",
            overlay_path.display()
        );
    } else {
        log::debug!(
            target: "debug_viz",
            "✅ OCR lines overlay saved: {}",
            overlay_path.display()
        );
    }

    let _ = original_bytes.len();

    Ok(())
}

/// Save a pretty-printed JSON snapshot of the real GLM-OCR response.
///
/// Uses the same dev-only guard as the OCR overlay artifacts:
/// debug builds + `ENTROPIA_DEBUG_VIZ` enabled. The snapshot intentionally
/// stores only the response payload and lightweight capture metadata.
pub fn save_glm_ocr_response_debug(
    response: &GlmOcrResponse,
    method: &str,
    asset_id: &str,
) -> Result<(), String> {
    if !is_debug_viz_enabled() {
        return Ok(());
    }

    let root = match workspace_root() {
        Some(r) => r,
        None => {
            log::debug!(
                target: "debug_viz",
                "CARGO_MANIFEST_DIR unavailable — skipping GLM-OCR response dump"
            );
            return Ok(());
        }
    };

    let ts = timestamp_ms();
    let layouts_dir = root.join("tests_layouts");

    if let Err(e) = std::fs::create_dir_all(&layouts_dir) {
        log::debug!(
            target: "debug_viz",
            "Failed to create {}: {e}",
            layouts_dir.display()
        );
        return Ok(());
    }

    let short_id: String = asset_id.chars().take(32).collect();
    let dump_path = layouts_dir.join(format!("glm_ocr_response_{short_id}_{ts}.json"));
    let total_layout_items: usize = response.layout_details.iter().map(|page| page.len()).sum();

    let payload = json!({
        "asset_id": asset_id,
        "method": method,
        "captured_at_unix_ms": ts,
        "summary": {
            "md_results_length": response.md_results.len(),
            "layout_pages": response.layout_details.len(),
            "layout_items": total_layout_items,
        },
        "response": response,
    });

    let json = match serde_json::to_string_pretty(&payload) {
        Ok(value) => value,
        Err(e) => {
            log::debug!(target: "debug_viz", "Failed to serialize GLM-OCR response for {asset_id}: {e}");
            return Ok(());
        }
    };

    match std::fs::write(&dump_path, json) {
        Ok(_) => {
            log::debug!(
                target: "debug_viz",
                "✅ GLM-OCR response dump saved: {}",
                dump_path.display()
            );
        }
        Err(e) => {
            log::debug!(
                target: "debug_viz",
                "Failed to write GLM-OCR response dump {}: {e}",
                dump_path.display()
            );
        }
    }

    Ok(())
}

/// Draw a small solid color-coded square at the top-left corner of a bbox.
/// Like `draw_corner_marker` but with configurable size (smaller for OCR lines).
fn draw_corner_marker_sized(image: &mut RgbaImage, bbox: &BoundingBox, color: Rgba<u8>, size: u32) {
    let x0 = bbox.x.max(0) as u32;
    let y0 = bbox.y.max(0) as u32;
    let s = size.min(bbox.width).min(bbox.height);
    if s == 0 {
        return;
    }

    let img_w = image.width();
    let img_h = image.height();
    for dy in 0..s {
        for dx in 0..s {
            let px = x0 + dx;
            let py = y0 + dy;
            if px < img_w && py < img_h {
                image.put_pixel(px, py, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_color_is_stable() {
        // Regression guard — colors must not change silently between versions
        let c = category_color(&LayoutCategory::Title);
        assert_eq!(c, Rgba([220, 30, 30, 255]));
        let c = category_color(&LayoutCategory::PlainText);
        assert_eq!(c, Rgba([30, 160, 60, 255]));
    }

    #[test]
    fn workspace_root_resolves() {
        // In `cargo test` we always have CARGO_MANIFEST_DIR set
        let root = workspace_root();
        assert!(root.is_some(), "workspace_root() should resolve in tests");
        let root = root.unwrap();
        // The workspace root should contain `apps/desktop/src-tauri`
        let expected_child = root.join("apps").join("desktop").join("src-tauri");
        assert!(
            expected_child.exists(),
            "expected workspace root to contain {}, but got {}",
            expected_child.display(),
            root.display()
        );
    }

    #[test]
    fn draw_thick_rect_clamps_without_panic() {
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        // Rectangle partially outside the image — must not panic
        let rect = Rect::at(-20, -20).of_size(50, 50);
        draw_thick_rect(&mut img, rect, Rgba([255, 0, 0, 255]), 5);
        // If we got here without panicking, the test passes
    }
}
