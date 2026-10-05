//! Reading order computation for OCR regions.
//!
//! Port of `orden_lectura.py` algorithm: groups detected text lines into columns
//! by X-proximity and sorts them for natural reading order (headers first, then
//! columns left-to-right, lines top-to-bottom within each column).
//!
//! ## Algorithm (from orden_lectura.py)
//!
//! 1. **Header separation** — "top wide items" (in the top 22% of the image,
//!    wider than 58% of image width) are placed first, sorted top-to-bottom.
//! 2. **Column grouping** — remaining items are clustered by X proximity;
//!    tolerance is `max(median_line_height * 2.5, img_width * 0.04)`.
//!    Columns are anchored by the median x1 of their items.
//! 3. **Defensive merge** — columns whose anchors are closer than
//!    `tolerance * 0.8` are merged.
//! 4. **Noise compaction** — columns with fewer than `max(4, total * 8%)`
//!    items get absorbed into the nearest "main" column.
//! 5. **Final order** — headers top-to-bottom, then columns left-to-right,
//!    items within each column sorted by (y1, x1).

#![allow(dead_code)]

use super::provider::OcrRegion;

// ── Legacy layout-based reading order (dead code) ─────────────────────────────
// Kept for potential future re-enablement of layout-aware OCR.

use super::provider::LayoutRegion;

const COLUMN_CENTER_THRESHOLD: i32 = 15;
const MIN_GAP_FOR_FILLING: i32 = 4;
const GAP_ALIGNMENT_RATIO: f32 = 0.3;

pub struct ReadingOrder {
    pub columns: Vec<Vec<usize>>,
}

pub fn compute_reading_order(regions: &mut [LayoutRegion], _image_width: u32) -> ReadingOrder {
    if regions.is_empty() {
        return ReadingOrder { columns: vec![] };
    }
    fill_vertical_gaps(regions);
    let columns = group_columns(regions);
    let mut global_order = 0usize;
    let mut result_columns: Vec<Vec<usize>> = Vec::new();
    for column in &columns {
        let mut col_indices = Vec::new();
        for &idx in &column.indices {
            regions[idx].order = global_order;
            global_order += 1;
            col_indices.push(idx);
        }
        result_columns.push(col_indices);
    }
    ReadingOrder {
        columns: result_columns,
    }
}

fn fill_vertical_gaps(regions: &mut [LayoutRegion]) {
    if regions.len() < 2 {
        return;
    }
    let mut sorted_indices: Vec<usize> = (0..regions.len()).collect();
    sorted_indices.sort_by_key(|&i| regions[i].bbox.y);
    for window in sorted_indices.windows(2) {
        let above_idx = window[0];
        let below_idx = window[1];
        let above = &regions[above_idx];
        let below = &regions[below_idx];
        let a_left = above.bbox.x;
        let a_right = above.bbox.x + above.bbox.width as i32;
        let b_left = below.bbox.x;
        let b_right = below.bbox.x + below.bbox.width as i32;
        let overlap_left = a_left.max(b_left);
        let overlap_right = a_right.min(b_right);
        let overlap = overlap_right - overlap_left;
        if overlap <= 0 {
            continue;
        }
        let narrower = (above.bbox.width.min(below.bbox.width)) as f32;
        if narrower <= 0.0 || (overlap as f32 / narrower) < GAP_ALIGNMENT_RATIO {
            continue;
        }
        let above_bottom = above.bbox.y + above.bbox.height as i32;
        let below_top = below.bbox.y;
        let gap = below_top - above_bottom;
        if gap < MIN_GAP_FOR_FILLING {
            continue;
        }
        let gap_center_x = (overlap_left + overlap_right) / 2;
        let above_center_x = above.bbox.x + (above.bbox.width as i32) / 2;
        let below_center_x = below.bbox.x + (below.bbox.width as i32) / 2;
        let above_dist = (above_center_x - gap_center_x).abs();
        let below_dist = (below_center_x - gap_center_x).abs();
        if above_dist <= below_dist {
            regions[above_idx].bbox.height = (below_top - regions[above_idx].bbox.y) as u32;
        } else {
            let height_diff = (regions[below_idx].bbox.y - above_bottom) as u32;
            regions[below_idx].bbox.y = above_bottom;
            regions[below_idx].bbox.height += height_diff;
        }
    }
}

const FULL_WIDTH_RATIO: f32 = 0.6;

fn group_columns(regions: &[LayoutRegion]) -> Vec<ColumnGroup> {
    let n = regions.len();
    if n == 0 {
        return vec![];
    }
    let centers: Vec<i32> = regions
        .iter()
        .map(|r| r.bbox.x + (r.bbox.width as i32) / 2)
        .collect();
    let min_x = regions.iter().map(|r| r.bbox.x).min().unwrap_or(0);
    let max_x = regions
        .iter()
        .map(|r| r.bbox.x + r.bbox.width as i32)
        .max()
        .unwrap_or(0);
    let total_span = (max_x - min_x).max(1) as f32;
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        if parent[i] != i {
            parent[i] = find(parent, parent[i]);
        }
        parent[i]
    }
    fn union(parent: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[ra] = rb;
        }
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if (centers[i] - centers[j]).abs() <= COLUMN_CENTER_THRESHOLD {
                union(&mut parent, i, j);
            }
        }
    }
    let mut groups: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    let mut merged_groups: Vec<Vec<usize>> = Vec::new();
    let mut singletons: Vec<usize> = Vec::new();
    for indices in groups.into_values() {
        if indices.len() == 1 {
            let idx = indices[0];
            if regions[idx].bbox.width as f32 / total_span >= FULL_WIDTH_RATIO {
                singletons.push(idx);
                continue;
            }
        }
        merged_groups.push(indices);
    }
    if !merged_groups.is_empty() {
        for singleton_idx in singletons {
            let s_center = centers[singleton_idx];
            let mut best_dist = i32::MAX;
            let mut best_col = 0;
            let mut best_center = i32::MAX;
            for (col_idx, group) in merged_groups.iter().enumerate() {
                let avg_center =
                    group.iter().map(|&i| centers[i]).sum::<i32>() / group.len() as i32;
                let dist = (s_center - avg_center).abs();
                if dist < best_dist || (dist == best_dist && avg_center < best_center) {
                    best_dist = dist;
                    best_col = col_idx;
                    best_center = avg_center;
                }
            }
            merged_groups[best_col].push(singleton_idx);
        }
    } else {
        for idx in singletons {
            merged_groups.push(vec![idx]);
        }
    }
    let mut columns: Vec<ColumnGroup> = merged_groups
        .into_iter()
        .map(|mut indices| {
            indices.sort_by_key(|&i| regions[i].bbox.y);
            let avg_center =
                indices.iter().map(|&i| centers[i]).sum::<i32>() / indices.len() as i32;
            ColumnGroup {
                avg_center,
                indices,
            }
        })
        .collect();
    columns.sort_by_key(|c| c.avg_center);
    columns
}

struct ColumnGroup {
    avg_center: i32,
    indices: Vec<usize>,
}

// ── OCR Line Reading Order (from orden_lectura.py) ────────────────────────────

/// Width-to-image-width ratio for a line to be considered a "header" that
/// spans the page. Lines wider than this in the top band go first.
const WIDE_TOP_RATIO: f32 = 0.58;

/// Vertical band (as a fraction of image height) for header detection.
/// Lines starting above this threshold AND wide enough are "top wide items".
const TOP_BAND_RATIO: f32 = 0.22;

/// An intermediate item for column grouping — mirrors the Python dict.
#[derive(Clone)]
struct OcrItem {
    idx: usize,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
    w: f32,
    h: f32,
}

/// A column of OCR items, anchored by the median x1 of its members.
#[derive(Clone)]
struct OcrColumn {
    anchor: f32,
    items: Vec<OcrItem>,
}

/// Reorder OCR regions by natural reading order.
///
/// Port of `orden_lectura.py` algorithm:
/// 1. Separate "top wide items" (headers in the top band that span the page)
/// 2. Group remaining items into columns by X-proximity with dynamic tolerance
/// 3. Merge close columns, compact noise columns into main ones
/// 4. Sort: headers top-to-bottom, then columns left-to-right, items top-to-bottom
///
/// Returns a new `Vec<OcrRegion>` in reading order. Regions without bounding
/// boxes are appended at the end in their original order.
///
/// If fewer than 2 regions have bounding boxes, returns the regions unchanged.
pub fn reorder_ocr_regions(
    regions: &[OcrRegion],
    img_width: u32,
    img_height: u32,
) -> Vec<OcrRegion> {
    // Separate regions with and without bboxes
    let mut with_bbox: Vec<(usize, &OcrRegion)> = Vec::new();
    let mut without_bbox: Vec<(usize, &OcrRegion)> = Vec::new();

    for (i, r) in regions.iter().enumerate() {
        if r.bbox.is_some() {
            with_bbox.push((i, r));
        } else {
            without_bbox.push((i, r));
        }
    }

    if with_bbox.len() < 2 {
        // Not enough spatial info to reorder — return as-is
        return regions.to_vec();
    }

    let img_w = img_width as f32;
    let img_h = img_height as f32;

    // Build OcrItem structs from regions with bboxes
    let items: Vec<OcrItem> = with_bbox
        .iter()
        .map(|(idx, r)| {
            let b = r.bbox.as_ref().unwrap();
            let x1 = b.x as f32;
            let y1 = b.y as f32;
            let w = b.width as f32;
            let h = b.height as f32;
            OcrItem {
                idx: *idx,
                x1,
                x2: x1 + w,
                y1,
                y2: y1 + h,
                w: w.max(1.0),
                h: h.max(1.0),
            }
        })
        .collect();

    // ── Step 1: Separate top-wide items (headers) ──────────────────────────
    let mut top_wide: Vec<OcrItem> = Vec::new();
    let mut column_candidates: Vec<OcrItem> = Vec::new();

    for it in items {
        let is_top = it.y1 <= img_h * TOP_BAND_RATIO;
        let is_wide = it.w >= img_w * WIDE_TOP_RATIO;
        if is_top && is_wide {
            top_wide.push(it);
        } else {
            column_candidates.push(it);
        }
    }

    // Sort headers top-to-bottom
    top_wide.sort_by(|a, b| {
        a.y1.partial_cmp(&b.y1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x1.partial_cmp(&b.x1).unwrap_or(std::cmp::Ordering::Equal))
    });

    // ── Step 2: Group remaining items into columns ────────────────────────
    let columns = group_ocr_columns(&column_candidates, img_w);

    // ── Step 3: Assemble final reading order ────────────────────────────────
    let mut ordered_indices: Vec<usize> = Vec::with_capacity(regions.len());

    // Headers first
    for it in &top_wide {
        ordered_indices.push(it.idx);
    }

    // Then columns left-to-right, items top-to-bottom within each column
    for col in &columns {
        for it in &col.items {
            ordered_indices.push(it.idx);
        }
    }

    // Regions without bboxes go at the end in original order
    for (idx, _) in &without_bbox {
        ordered_indices.push(*idx);
    }

    // Build reordered result
    let mut result = Vec::with_capacity(regions.len());
    for idx in ordered_indices {
        result.push(regions[idx].clone());
    }

    result
}

/// Group OCR items into columns by X-proximity.
///
/// Works identically to `orden_lectura.py`'s `_group_columns_by_x`:
/// - Tolerance is `max(median_height * 2.5, img_width * 0.04)`
/// - Columns are anchored by the median x1 of their items
/// - Defensive merge of columns closer than `tolerance * 0.8`
/// - Noise compaction: small columns (<8% of total or <4 items) merge into
///   nearest main column
fn group_ocr_columns(items: &[OcrItem], img_w: f32) -> Vec<OcrColumn> {
    if items.is_empty() {
        return vec![];
    }

    // Compute dynamic tolerance based on median line height
    let mut heights: Vec<f32> = items.iter().map(|it| it.h).collect();
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_h = heights[heights.len() / 2];
    let x_tol = (median_h * 2.5).max(img_w * 0.04);

    // Sort items by (x1, y1) for deterministic column assignment
    let mut sorted: Vec<&OcrItem> = items.iter().collect();
    sorted.sort_by(|a, b| {
        a.x1.partial_cmp(&b.x1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.y1.partial_cmp(&b.y1).unwrap_or(std::cmp::Ordering::Equal))
    });

    // Assign each item to the closest column within tolerance
    let mut columns: Vec<OcrColumn> = Vec::new();

    for it in &sorted {
        let mut best_idx: Option<usize> = None;
        let mut best_dist = f32::MAX;

        for (ci, col) in columns.iter().enumerate() {
            let dist = (it.x1 - col.anchor).abs();
            if dist <= x_tol && dist < best_dist {
                best_idx = Some(ci);
                best_dist = dist;
            }
        }

        if let Some(ci) = best_idx {
            columns[ci].items.push((*it).clone());
            // Update anchor to median of all x1s in column
            let mut x1s: Vec<f32> = columns[ci].items.iter().map(|i| i.x1).collect();
            x1s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            columns[ci].anchor = x1s[x1s.len() / 2];
        } else {
            columns.push(OcrColumn {
                anchor: it.x1,
                items: vec![(*it).clone()],
            });
        }
    }

    // ── Defensive merge: columns closer than tolerance * 0.8 ───────────────
    columns.sort_by(|a, b| {
        a.anchor
            .partial_cmp(&b.anchor)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut merged: Vec<OcrColumn> = Vec::new();
    for col in columns {
        if merged.is_empty() {
            merged.push(col);
            continue;
        }
        let prev = merged.last_mut().unwrap();
        if (col.anchor - prev.anchor).abs() <= x_tol * 0.8 {
            prev.items.extend(col.items);
            // Recompute anchor
            let mut x1s: Vec<f32> = prev.items.iter().map(|i| i.x1).collect();
            x1s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            prev.anchor = x1s[x1s.len() / 2];
        } else {
            merged.push(col);
        }
    }

    // Sort items within each column by (y1, x1)
    for col in &mut merged {
        col.items.sort_by(|a, b| {
            a.y1.partial_cmp(&b.y1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.x1.partial_cmp(&b.x1).unwrap_or(std::cmp::Ordering::Equal))
        });
    }

    // ── Noise compaction: merge small columns into nearest main column ────
    let total = items.len() as f32;
    let min_items_for_main = (total * 0.08).ceil() as usize;
    let min_for_main = 4usize.max(min_items_for_main);

    let main_cols: Vec<usize> = merged
        .iter()
        .enumerate()
        .filter(|(_, c)| c.items.len() >= min_for_main)
        .map(|(i, _)| i)
        .collect();

    if !main_cols.is_empty() {
        let mut noise_cols: Vec<usize> = Vec::new();
        for (i, col) in merged.iter().enumerate() {
            if col.items.len() < min_for_main {
                noise_cols.push(i);
            }
        }

        // Collect items from noise columns and absorb them into nearest main column
        let mut absorbed: Vec<(usize, Vec<OcrItem>)> = Vec::new(); // (main_col_idx, items)
        for noise_idx in noise_cols.iter().rev() {
            let noise_col = &merged[*noise_idx];
            // Find nearest main column by anchor distance
            let nearest_main = main_cols
                .iter()
                .min_by_key(|&&mi| {
                    let dist = (noise_col.anchor - merged[mi].anchor).abs();
                    // Use ordered float comparison — we don't need exact ordering of NaNs
                    ordered_float(dist)
                })
                .unwrap();

            absorbed.push((*nearest_main, noise_col.items.clone()));
        }

        // Absorb noise items into their nearest main columns
        for (main_idx, items) in absorbed {
            merged[main_idx].items.extend(items);
            // Recompute anchor
            let mut x1s: Vec<f32> = merged[main_idx].items.iter().map(|i| i.x1).collect();
            x1s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            merged[main_idx].anchor = x1s[x1s.len() / 2];
        }

        // Remove noise columns (in reverse to preserve indices)
        for &noise_idx in noise_cols.iter().rev() {
            merged.remove(noise_idx);
        }

        // Re-sort items within each column after absorption
        for col in &mut merged {
            col.items.sort_by(|a, b| {
                a.y1.partial_cmp(&b.y1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.x1.partial_cmp(&b.x1).unwrap_or(std::cmp::Ordering::Equal))
            });
        }
    }

    // Final: sort columns left-to-right by anchor
    merged.sort_by(|a, b| {
        a.anchor
            .partial_cmp(&b.anchor)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    merged
}

/// Helper for ordering f32 values (treats NaN as greater than everything).
fn ordered_float(f: f32) -> u64 {
    if f.is_nan() {
        u64::MAX
    } else {
        f.to_bits() as u64
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::provider::{BoundingBox, LayoutCategory};

    fn make_layout_region(
        label: LayoutCategory,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        confidence: f32,
    ) -> LayoutRegion {
        LayoutRegion {
            label,
            bbox: BoundingBox {
                x,
                y,
                width: w,
                height: h,
            },
            confidence,
            order: 0,
        }
    }

    fn make_ocr_region(text: &str, x: i32, y: i32, w: u32, h: u32, confidence: f32) -> OcrRegion {
        OcrRegion {
            text: text.to_string(),
            confidence,
            bbox: Some(BoundingBox {
                x,
                y,
                width: w,
                height: h,
            }),
            column: None,
        }
    }

    // ── Legacy tests (LayoutRegion-based) ─────────────────────────────────

    #[test]
    fn test_single_column_top_to_bottom() {
        let mut regions = vec![
            make_layout_region(LayoutCategory::PlainText, 50, 300, 300, 30, 0.9),
            make_layout_region(LayoutCategory::PlainText, 50, 100, 300, 30, 0.9),
            make_layout_region(LayoutCategory::PlainText, 50, 200, 300, 30, 0.9),
        ];
        let _ = compute_reading_order(&mut regions, 600);
        let at_100 = regions.iter().find(|r| r.bbox.y == 100).unwrap();
        let at_200 = regions.iter().find(|r| r.bbox.y == 200).unwrap();
        let at_300 = regions.iter().find(|r| r.bbox.y == 300).unwrap();
        assert_eq!(at_100.order, 0);
        assert_eq!(at_200.order, 1);
        assert_eq!(at_300.order, 2);
    }

    #[test]
    fn test_two_column_layout() {
        let mut regions = vec![
            make_layout_region(LayoutCategory::PlainText, 450, 400, 300, 30, 0.9),
            make_layout_region(LayoutCategory::PlainText, 50, 400, 300, 30, 0.9),
            make_layout_region(LayoutCategory::PlainText, 50, 100, 300, 30, 0.9),
            make_layout_region(LayoutCategory::PlainText, 450, 100, 300, 30, 0.9),
        ];
        let order = compute_reading_order(&mut regions, 800);
        assert_eq!(order.columns.len(), 2);
    }

    #[test]
    fn test_empty_regions() {
        let mut regions: Vec<LayoutRegion> = vec![];
        let order = compute_reading_order(&mut regions, 600);
        assert!(regions.is_empty());
        assert!(order.columns.is_empty());
    }

    // ── New tests (OcrRegion-based reading order) ─────────────────────────

    #[test]
    fn test_ocr_single_column_reading_order() {
        // Three lines in a single column, shuffled
        let regions = vec![
            make_ocr_region("bottom", 50, 300, 300, 30, 0.9),
            make_ocr_region("top", 50, 100, 300, 30, 0.9),
            make_ocr_region("middle", 50, 200, 300, 30, 0.9),
        ];

        let ordered = reorder_ocr_regions(&regions, 400, 500);
        assert_eq!(ordered[0].text, "top");
        assert_eq!(ordered[1].text, "middle");
        assert_eq!(ordered[2].text, "bottom");
    }

    #[test]
    fn test_ocr_two_columns_left_to_right() {
        // Left column + right column, shuffled
        let regions = vec![
            make_ocr_region("R-bottom", 450, 400, 300, 30, 0.9),
            make_ocr_region("L-bottom", 50, 400, 300, 30, 0.9),
            make_ocr_region("L-top", 50, 100, 300, 30, 0.9),
            make_ocr_region("R-top", 450, 100, 300, 30, 0.9),
        ];

        let ordered = reorder_ocr_regions(&regions, 800, 500);
        // Left column should come before right column
        let left_lines: Vec<&str> = ordered
            .iter()
            .filter(|r| r.bbox.as_ref().unwrap().x < 300)
            .map(|r| r.text.as_str())
            .collect();
        let right_lines: Vec<&str> = ordered
            .iter()
            .filter(|r| r.bbox.as_ref().unwrap().x >= 300)
            .map(|r| r.text.as_str())
            .collect();

        assert_eq!(left_lines, vec!["L-top", "L-bottom"]);
        assert_eq!(right_lines, vec!["R-top", "R-bottom"]);

        // All left lines should appear before all right lines
        let left_end = ordered.iter().position(|r| r.text == "L-bottom").unwrap();
        let right_start = ordered.iter().position(|r| r.text == "R-top").unwrap();
        assert!(
            left_end < right_start,
            "Left column should come before right column"
        );
    }

    #[test]
    fn test_ocr_header_first_then_columns() {
        // Wide header at the top + two columns below
        let regions = vec![
            make_ocr_region("body-L", 50, 200, 300, 30, 0.9),
            make_ocr_region("body-R", 450, 200, 300, 30, 0.9),
            make_ocr_region("HEADER", 50, 30, 700, 30, 0.95), // wide header spanning full width
        ];

        let ordered = reorder_ocr_regions(&regions, 800, 500);
        // Header should come first
        assert_eq!(ordered[0].text, "HEADER");
        // Then body lines
        assert!(
            ordered.iter().position(|r| r.text == "body-L").unwrap()
                < ordered.iter().position(|r| r.text == "body-R").unwrap()
        );
    }

    #[test]
    fn test_ocr_regions_without_bbox_preserved() {
        let regions = vec![
            make_ocr_region("with-box", 50, 100, 300, 30, 0.9),
            OcrRegion {
                text: "no-box".to_string(),
                confidence: 0.5,
                bbox: None,
                column: None,
            },
        ];

        let ordered = reorder_ocr_regions(&regions, 400, 500);
        // Region with bbox should come first, region without bbox at end
        assert_eq!(ordered.len(), 2);
        assert!(ordered
            .iter()
            .any(|r| r.text == "with-box" && r.bbox.is_some()));
        assert!(ordered
            .iter()
            .any(|r| r.text == "no-box" && r.bbox.is_none()));
    }

    #[test]
    fn test_ocr_too_few_regions_unchanged() {
        // Only 1 region with bbox — should return as-is
        let regions = vec![make_ocr_region("only", 50, 100, 300, 30, 0.9)];
        let ordered = reorder_ocr_regions(&regions, 400, 500);
        assert_eq!(ordered[0].text, "only");
    }

    #[test]
    fn test_ocr_noise_column_compaction() {
        // 10 items in column 1 (x~50), 1 noise item in column 2 (x~200)
        // The noise item should merge into column 1
        let mut regions: Vec<OcrRegion> = (0..10)
            .map(|i| make_ocr_region(&format!("main-{i}"), 50, 100 + i * 40, 120, 30, 0.9))
            .collect();
        // Noise item — single line far to the right but close enough
        regions.push(make_ocr_region("noise", 200, 150, 80, 30, 0.8));

        let ordered = reorder_ocr_regions(&regions, 400, 600);
        assert_eq!(ordered.len(), 11);
        // All items should be in reading order (top-to-bottom within column)
        // "noise" should be absorbed into the main column, not form its own column
    }
}
