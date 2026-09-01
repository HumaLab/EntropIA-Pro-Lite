//! Post-processing for OCR output with bounding boxes.
//!
//! Applies three transformations to PaddleOCR-style region output:
//! 1. **Column grouping** — assigns column indices based on X-overlap and sorts
//!    left-to-right, then top-to-bottom within each column.
//! 2. **Hyphen merging** — joins lines split with a trailing hyphen across breaks.
//! 3. **Paragraph detection** — inserts paragraph breaks between lines where
//!    a sentence ends and the next starts with an uppercase letter.
//!
//! OCR output without bounding boxes bypasses post-processing entirely.
//!
//! NOTE: Currently unused in production — was part of the layout-aware pipeline
//! that is no longer active in OCRL mode. Kept for potential future re-enablement.

#![allow(dead_code)]

use super::provider::OcrRegion;

/// X-overlap threshold in pixels for column grouping.
///
/// Two regions whose bounding boxes overlap by less than this threshold on the
/// X-axis are considered to be in different columns. 200px works well for
/// typical document layouts with gutters of ~0.5 inches at 300 DPI.
pub(crate) const COLUMN_OVERLAP_THRESHOLD: i32 = 200;

/// Apply all post-processing steps to OCR regions.
///
/// Regions must have bounding boxes (from PaddleOCR). Plain text-only output
/// with `bbox: None` should skip this pipeline entirely.
pub fn postprocess(regions: Vec<OcrRegion>) -> Vec<OcrRegion> {
    let grouped = group_columns(regions);
    let merged = merge_hyphens(grouped);
    detect_paragraphs(merged)
}

/// Group regions into columns by X-axis overlap of their bounding boxes.
///
/// Algorithm:
/// 1. Compute the set of distinct column boundaries by clustering bboxes
///    whose X-ranges overlap by at least `COLUMN_OVERLAP_THRESHOLD` pixels.
/// 2. Assign each region to a column index (0 = leftmost).
/// 3. Sort regions: left-to-right by column, then top-to-bottom by Y within
///    each column.
fn group_columns(mut regions: Vec<OcrRegion>) -> Vec<OcrRegion> {
    // Filter to regions that have bounding boxes
    let has_bbox: Vec<bool> = regions.iter().map(|r| r.bbox.is_some()).collect();
    if !has_bbox.iter().any(|b| *b) {
        // No bounding boxes — nothing to group
        return regions;
    }

    // Collect X-ranges for regions with bboxes
    let x_ranges: Vec<(i32, i32)> = regions
        .iter()
        .enumerate()
        .filter(|(_, r)| r.bbox.is_some())
        .map(|(_, r)| {
            let b = r.bbox.as_ref().unwrap();
            (b.x, b.x + b.width as i32)
        })
        .collect();

    if x_ranges.is_empty() {
        return regions;
    }

    // Sort x_ranges by left edge, keeping track of original indices
    let mut sorted_indices: Vec<usize> = (0..x_ranges.len()).collect();
    sorted_indices.sort_by_key(|&i| x_ranges[i].0);

    // Greedy column grouping:
    // Start with the leftmost region's range as column 0.
    // For each subsequent region (sorted by left edge), check if it overlaps
    // significantly with the current column's X-range. If not, start a new column.
    let mut column_ranges: Vec<(i32, i32)> = vec![]; // (min_x, max_x) per column
    let mut region_to_column: Vec<usize> = vec![0; x_ranges.len()];

    for (sort_pos, &idx) in sorted_indices.iter().enumerate() {
        let (region_left, region_right) = x_ranges[idx];
        let region_width = region_right - region_left;

        // Try to find an existing column this region belongs to
        let mut best_column: Option<usize> = None;
        let mut best_overlap: i32 = 0;

        for (col_idx, (col_left, col_right)) in column_ranges.iter().enumerate() {
            // X-overlap between region and column
            let overlap_start = region_left.max(*col_left);
            let overlap_end = region_right.min(*col_right);
            let overlap = overlap_end - overlap_start;

            if overlap >= COLUMN_OVERLAP_THRESHOLD.min(region_width) && overlap > best_overlap {
                best_overlap = overlap;
                best_column = Some(col_idx);
            }
        }

        let col = match best_column {
            Some(col_idx) => {
                // Extend column range
                let (col_left, col_right) = column_ranges[col_idx];
                column_ranges[col_idx] = (col_left.min(region_left), col_right.max(region_right));
                col_idx
            }
            None => {
                // New column
                column_ranges.push((region_left, region_right));
                column_ranges.len() - 1
            }
        };

        region_to_column[sort_pos] = col;
    }

    // Map sorted_indices → column assignments back to original region indices
    // We need a mapping from the bbox-filtered index to the actual region index
    let bbox_region_indices: Vec<usize> = regions
        .iter()
        .enumerate()
        .filter(|(_, r)| r.bbox.is_some())
        .map(|(i, _)| i)
        .collect();

    for (sort_pos, &sorted_idx) in sorted_indices.iter().enumerate() {
        let actual_region_idx = bbox_region_indices[sorted_idx];
        regions[actual_region_idx].column = Some(region_to_column[sort_pos]);
    }

    // Sort regions: left-to-right by column, then top-to-bottom by Y
    regions.sort_by(|a, b| {
        let col_a = a.column.unwrap_or(0);
        let col_b = b.column.unwrap_or(0);
        match col_a.cmp(&col_b) {
            std::cmp::Ordering::Equal => {
                let y_a = a.bbox.as_ref().map_or(0, |b| b.y);
                let y_b = b.bbox.as_ref().map_or(0, |b| b.y);
                y_a.cmp(&y_b)
            }
            other => other,
        }
    });

    regions
}

/// Merge hyphenated line breaks.
///
/// If a region's text ends with `-` and the next region starts with a lowercase
/// letter, the two regions are merged by removing the hyphen and joining the words.
/// For example, "intro-" + "duction" becomes "introduction".
fn merge_hyphens(regions: Vec<OcrRegion>) -> Vec<OcrRegion> {
    let mut result: Vec<OcrRegion> = Vec::with_capacity(regions.len());
    let mut i = 0;

    while i < regions.len() {
        let mut current = regions[i].clone();

        // Merge with subsequent regions while current ends with '-' and next starts lowercase
        while i + 1 < regions.len() {
            let text = current.text.trim_end();
            if !text.ends_with('-') {
                break;
            }

            let next_text = regions[i + 1].text.trim_start();
            let next_starts_lower = next_text.chars().next().is_some_and(|c| c.is_lowercase());

            if next_starts_lower {
                // Merge: remove hyphen and join words.
                // `text` is already `current.text.trim_end()` which ends with '-'.
                // Remove the trailing hyphen to get the root word, then append
                // the next word (which starts lowercase — it's a continuation).
                let root = &text[..text.len() - 1]; // strip trailing '-'
                let next_trimmed = regions[i + 1].text.trim_start();
                current.text = format!("{root}{next_trimmed}");

                // Keep the bounding box of the first region if available
                // (the merged region spans from the first bbox start to the last bbox end)
                if let (Some(ref mut bbox), Some(next_bbox)) =
                    (current.bbox.as_mut(), regions[i + 1].bbox.as_ref())
                {
                    let new_right =
                        (bbox.x + bbox.width as i32).max(next_bbox.x + next_bbox.width as i32);
                    let new_bottom =
                        (bbox.y + bbox.height as i32).max(next_bbox.y + next_bbox.height as i32);
                    bbox.width = (new_right - bbox.x) as u32;
                    bbox.height = (new_bottom - bbox.y) as u32;
                }

                // Take the higher confidence
                current.confidence = current.confidence.max(regions[i + 1].confidence);

                i += 1;
            } else {
                break;
            }
        }

        result.push(current);
        i += 1;
    }

    result
}

/// Detect paragraph boundaries.
///
/// A paragraph break is inserted when:
/// - The previous region's text ends with a sentence-ending punctuation mark (`.`, `!`, `?`)
/// - The current region's text starts with an uppercase letter
///
/// The paragraph break is represented as an extra newline (`\n`) appended to
/// the preceding region's text, so that the final assembled text has blank-line
/// separators between paragraphs.
fn detect_paragraphs(mut regions: Vec<OcrRegion>) -> Vec<OcrRegion> {
    if regions.len() <= 1 {
        return regions;
    }

    for i in 1..regions.len() {
        let prev_ends_with_sentence = regions[i - 1]
            .text
            .trim_end()
            .chars()
            .last()
            .is_some_and(|c| c == '.' || c == '!' || c == '?');

        let curr_starts_upper = regions[i]
            .text
            .trim_start()
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase());

        if prev_ends_with_sentence && curr_starts_upper {
            // Append an extra newline to mark paragraph break
            regions[i - 1].text.push('\n');
        }
    }

    regions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::provider::BoundingBox;

    fn make_region(text: &str, x: i32, y: i32, w: u32, h: u32) -> OcrRegion {
        OcrRegion {
            text: text.to_string(),
            confidence: 0.9,
            bbox: Some(BoundingBox {
                x,
                y,
                width: w,
                height: h,
            }),
            column: None,
        }
    }

    fn make_region_no_bbox(text: &str) -> OcrRegion {
        OcrRegion {
            text: text.to_string(),
            confidence: 0.9,
            bbox: None,
            column: None,
        }
    }

    #[test]
    fn test_group_columns_two_column_layout() {
        // Simulate a two-column document layout:
        // Left column (x=50): two paragraphs top-to-bottom
        // Right column (x=450): two paragraphs top-to-bottom
        let regions = vec![
            // Right column, bottom (should sort to position 3)
            make_region("Right bottom", 450, 400, 300, 30),
            // Left column, bottom (should sort to position 1)
            make_region("Left bottom", 50, 400, 300, 30),
            // Left column, top (should sort to position 0)
            make_region("Left top", 50, 100, 300, 30),
            // Right column, top (should sort to position 2)
            make_region("Right top", 450, 100, 300, 30),
        ];

        let result = group_columns(regions);

        // Verify column assignments
        assert_eq!(result[0].text, "Left top");
        assert_eq!(result[0].column, Some(0));
        assert_eq!(result[1].text, "Left bottom");
        assert_eq!(result[1].column, Some(0));
        assert_eq!(result[2].text, "Right top");
        assert_eq!(result[2].column, Some(1));
        assert_eq!(result[3].text, "Right bottom");
        assert_eq!(result[3].column, Some(1));
    }

    #[test]
    fn test_group_columns_single_column() {
        // All regions overlap on X-axis — single column
        let regions = vec![
            make_region("Bottom", 50, 300, 300, 30),
            make_region("Top", 60, 100, 280, 30),
        ];

        let result = group_columns(regions);

        assert_eq!(result[0].text, "Top");
        assert_eq!(result[1].text, "Bottom");
        // Both should be in column 0
        assert_eq!(result[0].column, Some(0));
        assert_eq!(result[1].column, Some(0));
    }

    #[test]
    fn test_group_columns_no_bboxes() {
        // Regions without bboxes should pass through unchanged
        let regions = vec![
            make_region_no_bbox("First line"),
            make_region_no_bbox("Second line"),
        ];

        let result = group_columns(regions);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].column, None);
        assert_eq!(result[1].column, None);
    }

    #[test]
    fn test_merge_hyphens_basic() {
        let regions = vec![
            OcrRegion {
                text: "intro-".to_string(),
                confidence: 0.9,
                bbox: Some(BoundingBox {
                    x: 50,
                    y: 100,
                    width: 100,
                    height: 20,
                }),
                column: Some(0),
            },
            OcrRegion {
                text: "duction".to_string(),
                confidence: 0.85,
                bbox: Some(BoundingBox {
                    x: 50,
                    y: 130,
                    width: 80,
                    height: 20,
                }),
                column: Some(0),
            },
            OcrRegion {
                text: "of text.".to_string(),
                confidence: 0.88,
                bbox: Some(BoundingBox {
                    x: 50,
                    y: 160,
                    width: 90,
                    height: 20,
                }),
                column: Some(0),
            },
        ];

        let result = merge_hyphens(regions);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].text, "introduction");
        assert_eq!(result[1].text, "of text.");
    }

    #[test]
    fn test_merge_hyphens_no_merge_uppercase() {
        // Should NOT merge if next line starts with uppercase
        let regions = vec![
            OcrRegion {
                text: "self-".to_string(),
                confidence: 0.9,
                bbox: None,
                column: None,
            },
            OcrRegion {
                text: "Important concept".to_string(),
                confidence: 0.85,
                bbox: None,
                column: None,
            },
        ];

        let result = merge_hyphens(regions);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].text, "self-");
    }

    #[test]
    fn test_merge_hyphens_trailing_whitespace() {
        // Hyphen with trailing whitespace should still merge
        let regions = vec![
            OcrRegion {
                text: "well- ".to_string(),
                confidence: 0.9,
                bbox: None,
                column: None,
            },
            OcrRegion {
                text: "known".to_string(),
                confidence: 0.85,
                bbox: None,
                column: None,
            },
        ];

        let result = merge_hyphens(regions);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "wellknown");
    }

    #[test]
    fn test_detect_paragraphs_sentence_break() {
        let regions = vec![
            OcrRegion {
                text: "This is the first paragraph.".to_string(),
                confidence: 0.9,
                bbox: None,
                column: None,
            },
            OcrRegion {
                text: "This is the second paragraph.".to_string(),
                confidence: 0.85,
                bbox: None,
                column: None,
            },
        ];

        let result = detect_paragraphs(regions);
        assert_eq!(result.len(), 2);
        // First region should have an extra newline
        assert!(result[0].text.ends_with('\n'));
        assert_eq!(result[1].text, "This is the second paragraph.");
    }

    #[test]
    fn test_detect_paragraphs_no_break_lowercase() {
        // Should NOT insert paragraph break if next line starts with lowercase
        let regions = vec![
            OcrRegion {
                text: "This sentence continues".to_string(),
                confidence: 0.9,
                bbox: None,
                column: None,
            },
            OcrRegion {
                text: "on the next line.".to_string(),
                confidence: 0.85,
                bbox: None,
                column: None,
            },
        ];

        let result = detect_paragraphs(regions);
        assert_eq!(result[0].text, "This sentence continues");
    }

    #[test]
    fn test_detect_paragraphs_exclamation() {
        let regions = vec![
            OcrRegion {
                text: "Watch out!".to_string(),
                confidence: 0.9,
                bbox: None,
                column: None,
            },
            OcrRegion {
                text: "Danger ahead.".to_string(),
                confidence: 0.85,
                bbox: None,
                column: None,
            },
        ];

        let result = detect_paragraphs(regions);
        assert!(result[0].text.ends_with('\n'));
    }

    #[test]
    fn test_detect_paragraphs_question() {
        let regions = vec![
            OcrRegion {
                text: "What happened?".to_string(),
                confidence: 0.9,
                bbox: None,
                column: None,
            },
            OcrRegion {
                text: "Nobody knows.".to_string(),
                confidence: 0.85,
                bbox: None,
                column: None,
            },
        ];

        let result = detect_paragraphs(regions);
        assert!(result[0].text.ends_with('\n'));
    }

    #[test]
    fn test_detect_paragraphs_single_region() {
        let regions = vec![OcrRegion {
            text: "Only one region.".to_string(),
            confidence: 0.9,
            bbox: None,
            column: None,
        }];

        let result = detect_paragraphs(regions);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "Only one region.");
    }

    #[test]
    fn test_postprocess_full_pipeline() {
        // End-to-end: two columns, hyphenated word, paragraph break
        let regions = vec![
            make_region("Left para one. ", 50, 100, 300, 30),
            make_region("Left para two ", 50, 140, 300, 30),
            make_region("starts here.", 50, 170, 290, 30), // lowercase start after prev line
            make_region("intro-", 50, 250, 100, 30),
            make_region("duction to AI.", 50, 280, 300, 30),
            make_region("Right col first.", 450, 100, 300, 30),
            make_region("Right col second.", 450, 140, 300, 30),
        ];

        let result = postprocess(regions);

        // Verify column assignments (0 for left, 1 for right)
        let left_regions: Vec<_> = result.iter().filter(|r| r.column == Some(0)).collect();
        let right_regions: Vec<_> = result.iter().filter(|r| r.column == Some(1)).collect();

        assert!(!left_regions.is_empty(), "Should have left column regions");
        assert!(
            !right_regions.is_empty(),
            "Should have right column regions"
        );

        // Verify hyphen merge happened: "intro-" + "duction" → "introduction"
        let merged_texts: Vec<&str> = result.iter().map(|r| r.text.as_str()).collect();
        assert!(
            merged_texts.iter().any(|t| t.contains("introduction")),
            "Expected merged hyphen word 'introduction', got: {merged_texts:?}"
        );
    }
}
