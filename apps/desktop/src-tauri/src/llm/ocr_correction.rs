use regex::Regex;
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

const IMAGE_MARKER_PREFIX: &str = "OCRC_IMAGE_REFERENCE";

static OCR_REGION_MARKDOWN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!\[\]\(\s*([^)]*)\)").expect("valid OCR region Markdown regex"));
static OCR_REGION_VALUE_RE: LazyLock<Regex> = LazyLock::new(|| {
    let numeric = r"[-+]?(?:\d+(?:\.\d*)?|\.\d+)";
    Regex::new(&format!(
        r"(?i)^page\s*=\s*(\d+)\s*,\s*bbox\s*=\s*\[\s*({numeric})\s*,\s*({numeric})\s*,\s*({numeric})\s*,\s*({numeric})\s*\]$"
    ))
    .expect("valid OCR region value regex")
});
static IMAGE_MARKER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<!--\s*OCRC_IMAGE_REFERENCE[^>]*-->").expect("valid image marker regex")
});

#[derive(Clone)]
struct ReferenceSpan {
    start: usize,
    end: usize,
    source: String,
}

struct HtmlScan {
    images: Vec<ReferenceSpan>,
    inert_ranges: Vec<(usize, usize)>,
}

#[derive(Clone)]
struct ProtectedImage {
    start: usize,
    end: usize,
    source: String,
    marker: String,
    preceding_anchor: Option<String>,
    following_anchor: Option<String>,
    standalone: bool,
    left_separator: String,
    right_separator: String,
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn is_valid_ocr_region_value(value: &str) -> bool {
    let Some(captures) = OCR_REGION_VALUE_RE.captures(value.trim()) else {
        return false;
    };
    let Some(page) = captures
        .get(1)
        .and_then(|value| value.as_str().parse::<f64>().ok())
    else {
        return false;
    };
    let [Some(left), Some(top), Some(right), Some(bottom)] = [2, 3, 4, 5].map(|index| {
        captures
            .get(index)
            .and_then(|value| value.as_str().parse::<f64>().ok())
    }) else {
        return false;
    };

    page.is_finite()
        && page >= 0.0
        && page.fract() == 0.0
        && [left, top, right, bottom]
            .iter()
            .all(|value| value.is_finite())
        && left >= 0.0
        && top >= 0.0
        && right > left
        && bottom > top
}

fn html_tag_has_real_quoted_src(tag: &str) -> bool {
    let bytes = tag.as_bytes();
    let mut index = 4;
    while index < bytes.len() {
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'/') {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] == b'>' {
            break;
        }

        let name_start = index;
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && !matches!(bytes[index], b'=' | b'/' | b'>')
        {
            index += 1;
        }
        if name_start == index {
            index += 1;
            continue;
        }
        let name = &tag[name_start..index];
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || !matches!(bytes[index], b'"' | b'\'') {
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>'
            {
                index += 1;
            }
            continue;
        }

        let quote = bytes[index];
        index += 1;
        let value_start = index;
        while index < bytes.len() && bytes[index] != quote {
            index += 1;
        }
        if index >= bytes.len() {
            return false;
        }
        let value = &tag[value_start..index];
        index += 1;
        if name.eq_ignore_ascii_case("src") && !value.trim().is_empty() {
            return true;
        }
    }
    false
}

fn is_html_tag_opener(source: &str, position: usize) -> bool {
    let bytes = source.as_bytes();
    let Some(first) = bytes.get(position + 1).copied() else {
        return false;
    };
    if first.is_ascii_alphabetic() {
        return true;
    }
    match first {
        b'/' | b'?' => bytes.get(position + 2).is_some_and(u8::is_ascii_alphabetic),
        b'!' => bytes
            .get(position + 2)
            .is_some_and(|next| next.is_ascii_alphabetic() || *next == b'['),
        _ => false,
    }
}

fn scan_html(source: &str) -> HtmlScan {
    let bytes = source.as_bytes();
    let mut images = Vec::new();
    let mut inert_ranges = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        if source[cursor..].starts_with("<!--") {
            if let Some(comment_end) = source[cursor + 4..].find("-->") {
                let end = cursor + 4 + comment_end + 3;
                inert_ranges.push((cursor, end));
                cursor = end;
                continue;
            }
            inert_ranges.push((cursor, source.len()));
            break;
        }

        let character = source[cursor..]
            .chars()
            .next()
            .expect("cursor remains on a character boundary");
        if character != '<' {
            cursor += character.len_utf8();
            continue;
        }
        if !is_html_tag_opener(source, cursor) {
            cursor += character.len_utf8();
            continue;
        }

        let mut quote = None;
        let mut end = None;
        for (offset, byte) in bytes[cursor + 1..].iter().copied().enumerate() {
            if let Some(active_quote) = quote {
                if byte == active_quote {
                    quote = None;
                }
            } else if matches!(byte, b'"' | b'\'') {
                quote = Some(byte);
            } else if byte == b'>' {
                end = Some(cursor + 1 + offset + 1);
                break;
            }
        }
        let Some(end) = end else {
            cursor += character.len_utf8();
            continue;
        };
        inert_ranges.push((cursor, end));
        let tag = &source[cursor..end];
        let tag_bytes = tag.as_bytes();
        let is_img = tag_bytes
            .get(1..4)
            .is_some_and(|name| name.eq_ignore_ascii_case(b"img"))
            && tag_bytes
                .get(4)
                .is_some_and(|after| after.is_ascii_whitespace() || matches!(after, b'/' | b'>'));
        if is_img && html_tag_has_real_quoted_src(tag) {
            images.push(ReferenceSpan {
                start: cursor,
                end,
                source: tag.to_string(),
            });
        }
        cursor = end;
    }
    HtmlScan {
        images,
        inert_ranges,
    }
}

fn reference_spans_with_html(source: &str, html: &HtmlScan) -> Vec<ReferenceSpan> {
    let mut spans = Vec::new();
    for captures in OCR_REGION_MARKDOWN_RE.captures_iter(source) {
        let Some(whole) = captures.get(0) else {
            continue;
        };
        let Some(value) = captures.get(1) else {
            continue;
        };
        let overlaps_inert_html = html
            .inert_ranges
            .iter()
            .any(|(start, end)| whole.start() < *end && *start < whole.end());
        if !overlaps_inert_html && is_valid_ocr_region_value(value.as_str()) {
            spans.push(ReferenceSpan {
                start: whole.start(),
                end: whole.end(),
                source: whole.as_str().to_string(),
            });
        }
    }
    spans.extend(html.images.iter().cloned());
    spans.sort_by_key(|span| span.start);

    let mut last_end = 0;
    spans.retain(|span| {
        if span.start < last_end {
            return false;
        }
        last_end = span.end;
        true
    });
    spans
}

fn reference_spans(source: &str) -> Vec<ReferenceSpan> {
    let html = scan_html(source);
    reference_spans_with_html(source, &html)
}

fn last_anchor_word(value: &str) -> Option<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .next_back()
        .map(str::to_string)
}

fn first_anchor_word(value: &str) -> Option<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .find(|word| !word.is_empty())
        .map(str::to_string)
}

fn protected_images(source: &str) -> Vec<ProtectedImage> {
    let spans = reference_spans(source);
    spans
        .iter()
        .enumerate()
        .map(|(index, span)| {
            let mut suffix = 0_usize;
            let marker = loop {
                let suffix_text = if suffix == 0 {
                    String::new()
                } else {
                    format!("_{suffix}")
                };
                let candidate =
                    format!("<!--{IMAGE_MARKER_PREFIX}_{:04}{suffix_text}-->", index + 1);
                if !source.contains(&candidate) {
                    break candidate;
                }
                suffix += 1;
            };
            let previous_end = index
                .checked_sub(1)
                .map(|previous| spans[previous].end)
                .unwrap_or(0);
            let next_start = spans
                .get(index + 1)
                .map(|next| next.start)
                .unwrap_or(source.len());
            let line_start = source[..span.start]
                .rfind('\n')
                .map(|position| position + 1)
                .unwrap_or(0);
            let line_end = source[span.end..]
                .find('\n')
                .map(|position| span.end + position)
                .unwrap_or(source.len());
            let left_separator_len = source[..span.start]
                .chars()
                .rev()
                .take_while(|character| character.is_whitespace())
                .map(char::len_utf8)
                .sum::<usize>();
            let right_separator_len = source[span.end..]
                .chars()
                .take_while(|character| character.is_whitespace())
                .map(char::len_utf8)
                .sum::<usize>();

            ProtectedImage {
                start: span.start,
                end: span.end,
                source: span.source.clone(),
                marker,
                preceding_anchor: last_anchor_word(&source[previous_end..span.start]),
                following_anchor: first_anchor_word(&source[span.end..next_start]),
                standalone: source[line_start..line_end].trim() == span.source,
                left_separator: source[span.start - left_separator_len..span.start].to_string(),
                right_separator: source[span.end..span.end + right_separator_len].to_string(),
            }
        })
        .collect()
}

pub(crate) fn protect_image_references(source: &str) -> String {
    let images = protected_images(source);
    if images.is_empty() {
        return source.to_string();
    }
    let mut protected = String::with_capacity(source.len());
    let mut cursor = 0;
    for image in &images {
        protected.push_str(&source[cursor..image.start]);
        protected.push_str(&image.marker);
        cursor = image.end;
    }
    protected.push_str(&source[cursor..]);
    protected
}

#[derive(Clone, Copy)]
enum SlotIdentity {
    Image(usize),
    Generic,
    Extra,
}

struct CorrectedSlot {
    start: usize,
    end: usize,
    identity: SlotIdentity,
}

fn corrected_reference_slots(
    original: &str,
    corrected: &str,
    images: &[ProtectedImage],
) -> Vec<CorrectedSlot> {
    let preexisting_markers: HashSet<&str> = IMAGE_MARKER_RE
        .find_iter(original)
        .map(|marker| marker.as_str())
        .collect();
    let html = scan_html(corrected);
    let marker_is_nested_inert = |start: usize, end: usize| {
        html.inert_ranges.iter().any(|(inert_start, inert_end)| {
            start < *inert_end && *inert_start < end && (*inert_start != start || *inert_end != end)
        })
    };
    let mut used_images = vec![false; images.len()];
    let mut slots = Vec::new();
    for marker in IMAGE_MARKER_RE.find_iter(corrected) {
        if marker_is_nested_inert(marker.start(), marker.end())
            || preexisting_markers.contains(marker.as_str())
        {
            continue;
        }
        let identity = images
            .iter()
            .position(|image| image.marker == marker.as_str())
            .map_or(SlotIdentity::Generic, |image_index| {
                if used_images[image_index] {
                    SlotIdentity::Extra
                } else {
                    used_images[image_index] = true;
                    SlotIdentity::Image(image_index)
                }
            });
        slots.push(CorrectedSlot {
            start: marker.start(),
            end: marker.end(),
            identity,
        });
    }
    for span in reference_spans_with_html(corrected, &html) {
        let image_index = images.iter().enumerate().find_map(|(index, image)| {
            (!used_images[index] && image.source == span.source).then_some(index)
        });
        let identity = image_index.map_or(SlotIdentity::Extra, |index| {
            used_images[index] = true;
            SlotIdentity::Image(index)
        });
        slots.push(CorrectedSlot {
            start: span.start,
            end: span.end,
            identity,
        });
    }
    slots.sort_by_key(|slot| slot.start);

    let mut last_end = 0;
    slots.retain(|slot| {
        if slot.start < last_end {
            return false;
        }
        last_end = slot.end;
        true
    });
    slots
}

fn is_whole_anchor_match(output: &str, position: usize, anchor: &str) -> bool {
    let before_is_alphanumeric = output[..position]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric);
    let after = position + anchor.len();
    let after_is_alphanumeric = output[after..]
        .chars()
        .next()
        .is_some_and(char::is_alphanumeric);
    !before_is_alphanumeric && !after_is_alphanumeric
}

fn clamp_to_char_boundary(value: &str, offset: usize) -> usize {
    let mut position = offset.min(value.len());
    while position > 0 && !value.is_char_boundary(position) {
        position -= 1;
    }
    position
}

fn avoid_splitting_word(output: &str, position: usize, minimum_position: usize) -> usize {
    let before_is_alphanumeric = output[..position]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric);
    let after_is_alphanumeric = output[position..]
        .chars()
        .next()
        .is_some_and(char::is_alphanumeric);
    if !before_is_alphanumeric || !after_is_alphanumeric {
        return position;
    }

    let word_start = output[..position]
        .char_indices()
        .rev()
        .take_while(|(_, character)| character.is_alphanumeric())
        .map(|(index, _)| index)
        .last()
        .unwrap_or(position);
    if word_start >= minimum_position {
        return word_start;
    }
    position
        + output[position..]
            .chars()
            .take_while(|character| character.is_alphanumeric())
            .map(char::len_utf8)
            .sum::<usize>()
}

fn insert_missing_reference(
    output: &mut String,
    image: &ProtectedImage,
    minimum_position: usize,
) -> usize {
    let after_preceding = image.preceding_anchor.as_ref().and_then(|anchor| {
        output.rmatch_indices(anchor).find_map(|(position, _)| {
            let anchor_end = position + anchor.len();
            if anchor_end < minimum_position || !is_whole_anchor_match(output, position, anchor) {
                return None;
            }
            if image.standalone {
                Some(
                    output[anchor_end..]
                        .find('\n')
                        .map(|line_end| anchor_end + line_end)
                        .unwrap_or(output.len()),
                )
            } else {
                Some(anchor_end)
            }
        })
    });
    let before_following = image.following_anchor.as_ref().and_then(|anchor| {
        output[minimum_position..]
            .match_indices(anchor)
            .find_map(|(relative, _)| {
                let anchor_position = minimum_position + relative;
                if !is_whole_anchor_match(output, anchor_position, anchor) {
                    return None;
                }
                if image.standalone {
                    Some(
                        output[..anchor_position]
                            .rfind('\n')
                            .map(|line_start| line_start + 1)
                            .unwrap_or(0)
                            .max(minimum_position),
                    )
                } else {
                    Some(anchor_position)
                }
            })
    });
    enum Placement {
        BeforeFollowing,
        AfterPreceding,
        OriginalOffset,
    }
    let (mut position, placement) = if image.standalone {
        after_preceding
            .map(|position| (position, Placement::AfterPreceding))
            .or_else(|| before_following.map(|position| (position, Placement::BeforeFollowing)))
    } else {
        before_following
            .map(|position| (position, Placement::BeforeFollowing))
            .or_else(|| after_preceding.map(|position| (position, Placement::AfterPreceding)))
    }
    .unwrap_or_else(|| {
        let minimum_boundary = clamp_to_char_boundary(output, minimum_position);
        let original_offset = clamp_to_char_boundary(output, image.start).max(minimum_boundary);
        let exact_adjacency = image.left_separator.is_empty()
            && image.right_separator.is_empty()
            && image
                .preceding_anchor
                .as_ref()
                .is_some_and(|anchor| output[..original_offset].ends_with(anchor))
            && image
                .following_anchor
                .as_ref()
                .is_some_and(|anchor| output[original_offset..].starts_with(anchor));
        let position = if exact_adjacency {
            original_offset
        } else {
            avoid_splitting_word(output, original_offset, minimum_boundary)
        };
        (position, Placement::OriginalOffset)
    });

    let (left_reused, right_reused) = match placement {
        Placement::BeforeFollowing => {
            let right_reused = image.right_separator.is_empty()
                || output[..position].ends_with(&image.right_separator);
            if right_reused && !image.right_separator.is_empty() {
                position -= image.right_separator.len();
            }
            let left_reused = image.left_separator.is_empty()
                || output[..position].ends_with(&image.left_separator);
            (left_reused, right_reused)
        }
        Placement::AfterPreceding => {
            let left_reused = image.left_separator.is_empty()
                || output[position..].starts_with(&image.left_separator);
            if left_reused && !image.left_separator.is_empty() {
                position += image.left_separator.len();
            }
            let right_reused = image.right_separator.is_empty()
                || output[position..].starts_with(&image.right_separator);
            (left_reused, right_reused)
        }
        Placement::OriginalOffset => (
            image.left_separator.is_empty() || output[..position].ends_with(&image.left_separator),
            image.right_separator.is_empty()
                || output[position..].starts_with(&image.right_separator),
        ),
    };

    let mut insertion = String::with_capacity(
        image.left_separator.len() + image.source.len() + image.right_separator.len(),
    );
    if !left_reused {
        insertion.push_str(&image.left_separator);
    }
    insertion.push_str(&image.source);
    if !right_reused {
        insertion.push_str(&image.right_separator);
    }
    let reference_end = position
        + insertion
            .find(&image.source)
            .expect("insertion contains reference")
        + image.source.len();
    output.insert_str(position, &insertion);
    reference_end
}

fn replay_surviving_slot(
    output: &mut String,
    start: usize,
    end: usize,
    image: &ProtectedImage,
    replacement_source: &str,
) {
    let left_whitespace = output[..start]
        .chars()
        .rev()
        .take_while(|character| character.is_whitespace())
        .map(char::len_utf8)
        .sum::<usize>();
    let right_whitespace = output[end..]
        .chars()
        .take_while(|character| character.is_whitespace())
        .map(char::len_utf8)
        .sum::<usize>();
    let replace_start = start - left_whitespace;
    let replace_end = end + right_whitespace;
    let mut replacement = String::with_capacity(
        image.left_separator.len() + replacement_source.len() + image.right_separator.len(),
    );
    replacement.push_str(&image.left_separator);
    replacement.push_str(replacement_source);
    replacement.push_str(&image.right_separator);
    output.replace_range(replace_start..replace_end, &replacement);
}

fn restore_image_references(original: &str, corrected: &str) -> String {
    let images = protected_images(original);
    if images.is_empty() {
        return corrected.to_string();
    }

    let slots = corrected_reference_slots(original, corrected, &images);
    let mut assignments = vec![None; slots.len()];
    let mut used_images = vec![false; images.len()];
    for (slot_index, slot) in slots.iter().enumerate() {
        if let SlotIdentity::Image(image_index) = slot.identity {
            assignments[slot_index] = Some(image_index);
            used_images[image_index] = true;
        }
    }
    for (slot_index, slot) in slots.iter().enumerate() {
        if !matches!(slot.identity, SlotIdentity::Generic) {
            continue;
        }
        if let Some(image_index) = used_images.iter().position(|used| !*used) {
            assignments[slot_index] = Some(image_index);
            used_images[image_index] = true;
        }
    }
    let mut positional_images: Vec<usize> = assignments.iter().flatten().copied().collect();
    if positional_images.windows(2).any(|pair| pair[0] > pair[1]) {
        positional_images.sort_unstable();
        let mut ordered = positional_images.into_iter();
        for assignment in &mut assignments {
            if assignment.is_some() {
                *assignment = ordered.next();
            }
        }
    }

    let restoration_tokens: Vec<String> = images
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let mut suffix = 0_usize;
            loop {
                let token = format!(
                    "\u{e000}{IMAGE_MARKER_PREFIX}_RESTORED_{:04}_{suffix}\u{e001}",
                    index + 1
                );
                if !original.contains(&token) && !corrected.contains(&token) {
                    break token;
                }
                suffix += 1;
            }
        })
        .collect();
    let mut restored = corrected.to_string();
    for slot_index in (0..slots.len()).rev() {
        let slot = &slots[slot_index];
        if let Some(image_index) = assignments[slot_index] {
            replay_surviving_slot(
                &mut restored,
                slot.start,
                slot.end,
                &images[image_index],
                &restoration_tokens[image_index],
            );
        } else {
            restored.replace_range(slot.start..slot.end, "");
        }
    }

    let mut minimum_position = 0;
    for (image_index, image) in images.iter().enumerate() {
        if used_images[image_index] {
            minimum_position = restored
                .find(&restoration_tokens[image_index])
                .map(|position| position + restoration_tokens[image_index].len())
                .unwrap_or(minimum_position);
        } else {
            minimum_position = insert_missing_reference(&mut restored, image, minimum_position);
        }
    }
    for (image_index, image) in images.iter().enumerate() {
        if !used_images[image_index] {
            continue;
        }
        if let Some(position) = restored.find(&restoration_tokens[image_index]) {
            restored.replace_range(
                position..position + restoration_tokens[image_index].len(),
                &image.source,
            );
        }
    }
    restored
}

pub(crate) fn ensure_schema(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ocr_correction_backups (
            asset_id TEXT PRIMARY KEY,
            original_text_content TEXT NOT NULL,
            created_at INTEGER NOT NULL
         );",
    )
    .map_err(|error| format!("Failed to ensure OCR correction backup schema: {error}"))
}

pub(crate) const STALE_OCR_CORRECTION_ERROR: &str =
    "stale OCR correction: extraction changed during generation";

pub(crate) fn commit_asset_correction_if_current(
    conn: &rusqlite::Connection,
    asset_id: &str,
    model_output: &str,
    expected_extraction: &str,
) -> Result<String, String> {
    commit_asset_correction_impl(conn, asset_id, model_output, Some(expected_extraction))
}

fn commit_asset_correction_impl(
    conn: &rusqlite::Connection,
    asset_id: &str,
    model_output: &str,
    expected_extraction: Option<&str>,
) -> Result<String, String> {
    if model_output.trim().is_empty() {
        return Err("OCR correction model output is empty or whitespace-only".to_string());
    }

    ensure_schema(conn)?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("Failed to begin OCR correction transaction: {error}"))?;

    let existing_backup: Option<String> = tx
        .query_row(
            "SELECT original_text_content FROM ocr_correction_backups WHERE asset_id = ?1",
            [asset_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed to read OCR correction backup: {error}"))?;

    let current_extraction: Option<String> = tx
        .query_row(
            "SELECT text_content FROM extractions
             WHERE asset_id = ?1
             ORDER BY created_at DESC
             LIMIT 1",
            [asset_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed to read OCR extraction: {error}"))?;
    let current_extraction = current_extraction
        .ok_or_else(|| format!("No extraction available for OCR correction on asset {asset_id}"))?;
    if expected_extraction.is_some_and(|expected| expected != current_extraction.as_str()) {
        return Err(STALE_OCR_CORRECTION_ERROR.to_string());
    }
    let original = existing_backup.as_deref().unwrap_or(&current_extraction);
    let corrected = restore_image_references(original, model_output);
    let now = now_millis();

    tx.execute(
        "INSERT OR IGNORE INTO ocr_correction_backups(
            asset_id, original_text_content, created_at
         ) VALUES (?1, ?2, ?3)",
        params![asset_id, original, now],
    )
    .map_err(|error| format!("Failed to save original OCR backup: {error}"))?;

    let updated = tx
        .execute(
            "UPDATE extractions SET text_content = ?1 WHERE asset_id = ?2",
            params![corrected, asset_id],
        )
        .map_err(|error| format!("Failed to replace OCR extraction: {error}"))?;
    if updated == 0 {
        return Err(format!(
            "No extraction available for OCR correction on asset {asset_id}"
        ));
    }

    let result_id = format!("llr-asset-{asset_id}-correct_ocr");
    tx.execute(
        "INSERT INTO llm_results(id, target_id, target_type, job_type, result, created_at)
         VALUES (?1, ?2, 'asset', 'correct_ocr', ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            target_id = excluded.target_id,
            target_type = excluded.target_type,
            job_type = excluded.job_type,
            result = excluded.result,
            created_at = excluded.created_at",
        params![result_id, asset_id, corrected, now],
    )
    .map_err(|error| format!("Failed to persist OCR correction result: {error}"))?;

    tx.commit()
        .map_err(|error| format!("Failed to commit OCR correction transaction: {error}"))?;
    Ok(corrected)
}

pub(crate) fn can_restore_original(
    conn: &rusqlite::Connection,
    asset_id: &str,
) -> Result<bool, String> {
    ensure_schema(conn)?;
    conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM ocr_correction_backups WHERE asset_id = ?1
         )",
        [asset_id],
        |row| row.get(0),
    )
    .map_err(|error| format!("Failed to inspect OCR correction backup: {error}"))
}

pub(crate) fn restore_original(
    conn: &rusqlite::Connection,
    asset_id: &str,
) -> Result<String, String> {
    ensure_schema(conn)?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("Failed to begin OCR restore transaction: {error}"))?;
    let original: Option<String> = tx
        .query_row(
            "SELECT original_text_content FROM ocr_correction_backups WHERE asset_id = ?1",
            [asset_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed to read original OCR backup: {error}"))?;
    let original =
        original.ok_or_else(|| format!("No original OCR backup exists for asset {asset_id}"))?;

    let updated = tx
        .execute(
            "UPDATE extractions SET text_content = ?1 WHERE asset_id = ?2",
            params![original, asset_id],
        )
        .map_err(|error| format!("Failed to restore original OCR extraction: {error}"))?;
    if updated == 0 {
        return Err(format!(
            "No extraction available while restoring OCR asset {asset_id}"
        ));
    }

    tx.execute(
        "DELETE FROM llm_results
         WHERE target_id = ?1
           AND (target_type = 'asset' OR target_type = 'unknown')
           AND job_type = 'correct_ocr'",
        [asset_id],
    )
    .map_err(|error| format!("Failed to delete corrected OCR result: {error}"))?;
    tx.execute(
        "DELETE FROM ocr_correction_backups WHERE asset_id = ?1",
        [asset_id],
    )
    .map_err(|error| format!("Failed to delete restored OCR backup: {error}"))?;
    tx.commit()
        .map_err(|error| format!("Failed to commit OCR restore transaction: {error}"))?;
    Ok(original)
}

/// Clear OCRC state for `asset_id` using whatever transaction context the
/// caller already owns. When a transaction is open on `conn`, both DELETEs
/// participate in it (SQLite is already in an explicit transaction, so
/// statements join it). When none is open, each statement autocommits as
/// before. Never begins or commits a transaction of its own.
pub(crate) fn clear_asset_state(conn: &rusqlite::Connection, asset_id: &str) -> Result<(), String> {
    ensure_schema(conn)?;
    conn.execute(
        "DELETE FROM llm_results
         WHERE target_id = ?1
           AND (target_type = 'asset' OR target_type = 'unknown')
           AND job_type = 'correct_ocr'",
        [asset_id],
    )
    .map_err(|error| format!("Failed to clear corrected OCR result: {error}"))?;
    conn.execute(
        "DELETE FROM ocr_correction_backups WHERE asset_id = ?1",
        [asset_id],
    )
    .map_err(|error| format!("Failed to clear original OCR backup: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    const MARKDOWN_IMAGE: &str = "![](page=0,bbox=[10,20,30,40])";
    const HTML_IMAGE: &str = r#"<img src="data:image/png;base64,AAAA" alt="scan">"#;

    fn correction_db(original: &str) -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory correction db");
        conn.execute_batch(
            "CREATE TABLE extractions (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL UNIQUE,
                text_content TEXT NOT NULL,
                method TEXT NOT NULL,
                confidence REAL,
                created_at INTEGER NOT NULL
             );
             CREATE TABLE llm_results (
                id TEXT PRIMARY KEY,
                target_id TEXT NOT NULL,
                target_type TEXT NOT NULL,
                job_type TEXT NOT NULL,
                result TEXT NOT NULL,
                created_at INTEGER NOT NULL
             );",
        )
        .expect("correction schema");
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, method, created_at)
             VALUES ('ext-asset-1', 'asset-1', ?1, 'ocr', 1)",
            [original],
        )
        .expect("original extraction");
        conn
    }

    fn extraction_text(conn: &Connection) -> String {
        conn.query_row(
            "SELECT text_content FROM extractions WHERE asset_id = 'asset-1'",
            [],
            |row| row.get(0),
        )
        .expect("extraction text")
    }

    #[test]
    fn protects_and_restores_standalone_markdown_and_html_images_exactly_once() {
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto\n\n{HTML_IMAGE}\n\nFinal");

        let protected = protect_image_references(&original);
        assert!(!protected.contains(MARKDOWN_IMAGE));
        assert!(!protected.contains(HTML_IMAGE));
        assert_eq!(protected.matches("OCRC_IMAGE_REFERENCE").count(), 2);

        let restored = restore_image_references(&original, &protected);
        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 1);
        assert_eq!(restored.matches(HTML_IMAGE).count(), 1);
        assert!(restored.find(MARKDOWN_IMAGE).unwrap() < restored.find(HTML_IMAGE).unwrap());
    }

    #[test]
    fn missing_markers_fall_back_after_the_nearest_preserved_anchor_in_original_order() {
        let second = "![](page=0,bbox=[50,60,70,80])";
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\n{second}\n\nTexto original");
        let corrected_without_markers = "# Documento\n\nTexto corregido";

        let restored = restore_image_references(&original, corrected_without_markers);

        let heading = restored.find("# Documento").unwrap();
        let first = restored.find(MARKDOWN_IMAGE).unwrap();
        let second = restored.find(second).unwrap();
        let text = restored.find("Texto corregido").unwrap();
        assert!(heading < first && first < second && second < text);
    }

    #[test]
    fn existing_exact_images_are_not_duplicated_when_the_model_drops_the_marker() {
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto");
        let corrected = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto corregido");

        let restored = restore_image_references(&original, &corrected);

        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 1);
    }

    #[test]
    fn invalid_non_empty_alt_lookalike_does_not_replace_the_original_reference() {
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto");
        let lookalike = "![Descripción cambiada](page=0,bbox=[10,20,30,40])";
        let corrected = format!("# Documento\n\n{lookalike}\n\nTexto corregido");

        let restored = restore_image_references(&original, &corrected);

        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 1);
        assert_eq!(restored.matches(lookalike).count(), 1);
    }

    #[test]
    fn duplicate_original_sources_are_restored_by_occurrence() {
        let original =
            format!("# Documento\n\n{MARKDOWN_IMAGE}\n\n{MARKDOWN_IMAGE}\n\nTexto original");
        let protected = protect_image_references(&original);
        let corrected = protected.replacen("<!--OCRC_IMAGE_REFERENCE_0001-->", "", 1);

        let restored = restore_image_references(&original, &corrected);

        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 2);
    }

    #[test]
    fn a_missing_sibling_is_restored_after_an_already_retained_image() {
        let second = "![](page=0,bbox=[50,60,70,80])";
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\n{second}\n\nTexto original");
        let corrected = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto corregido");

        let restored = restore_image_references(&original, &corrected);

        let first_index = restored.find(MARKDOWN_IMAGE).unwrap();
        let second_index = restored.find(second).unwrap();
        assert!(first_index < second_index);
    }

    #[test]
    fn inline_reference_contract_protects_every_position_without_removing_surrounding_text() {
        let second = "![](page=1,bbox=[50,60,70,80])";
        let original = format!(
            "{MARKDOWN_IMAGE} inicio\nAntes {HTML_IMAGE} después\nFinal {second}\nMúltiples {MARKDOWN_IMAGE} + {HTML_IMAGE} + {MARKDOWN_IMAGE}"
        );

        let protected = protect_image_references(&original);

        assert_eq!(
            protected,
            "<!--OCRC_IMAGE_REFERENCE_0001--> inicio\nAntes <!--OCRC_IMAGE_REFERENCE_0002--> después\nFinal <!--OCRC_IMAGE_REFERENCE_0003-->\nMúltiples <!--OCRC_IMAGE_REFERENCE_0004--> + <!--OCRC_IMAGE_REFERENCE_0005--> + <!--OCRC_IMAGE_REFERENCE_0006-->"
        );
    }

    #[test]
    fn inline_reference_contract_restores_reordered_removed_and_altered_tokens_in_order() {
        let second = "![](page=1,bbox=[50,60,70,80])";
        let original = format!("Inicio {MARKDOWN_IMAGE} centro {HTML_IMAGE} cierre {second}");
        let corrected = "Inicio corregido <!--OCRC_IMAGE_REFERENCE_0003--> centro cambiado <!--OCRC_IMAGE_REFERENCE_ALTERED_0002--> cierre reescrito <!--OCRC_IMAGE_REFERENCE_0001-->";

        let restored = restore_image_references(&original, corrected);
        let first = restored.find(MARKDOWN_IMAGE).unwrap();
        let html = restored.find(HTML_IMAGE).unwrap();
        let second = restored.find(second).unwrap();

        assert!(first < html && html < second);
        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 1);
        assert_eq!(restored.matches(HTML_IMAGE).count(), 1);
        assert_eq!(restored.matches("Inicio corregido").count(), 1);
        assert_eq!(restored.matches("centro cambiado").count(), 1);
        assert_eq!(restored.matches("cierre reescrito").count(), 1);
    }

    #[test]
    fn inline_reference_contract_recovers_every_removed_marker_without_duplicating_text() {
        let original = format!(
            "Cabecera\nPrimero {MARKDOWN_IMAGE} medio {HTML_IMAGE}\nDuplicado {MARKDOWN_IMAGE}\nFinal"
        );
        let corrected = "Cabecera corregida\nPrimero ajustado medio ajustado\nDuplicado ajustado\nFinal corregido";

        let restored = restore_image_references(&original, corrected);
        let first = restored.find(MARKDOWN_IMAGE).unwrap();
        let html = restored.find(HTML_IMAGE).unwrap();
        let duplicate = restored.rfind(MARKDOWN_IMAGE).unwrap();

        assert!(first < html && html < duplicate);
        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 2);
        assert_eq!(restored.matches(HTML_IMAGE).count(), 1);
        for text in [
            "Cabecera corregida",
            "Primero ajustado",
            "medio ajustado",
            "Duplicado ajustado",
            "Final corregido",
        ] {
            assert_eq!(restored.matches(text).count(), 1, "duplicated text: {text}");
        }
    }

    #[test]
    fn inline_reference_contract_ignores_malformed_and_non_ocr_lookalikes() {
        let original = [
            "![alt](page=0,bbox=[1,2,3,4])",
            "![](page=0,bbox=[1,2,3])",
            "![](page=-1,bbox=[1,2,3,4])",
            "![](page=0,bbox=[3,2,1,4])",
            "![](https://example.com/image.png)",
            r#"<img alt="missing source">"#,
            r#"<img src=data:image/png;base64,AAAA>"#,
            r#"<img src="">"#,
        ]
        .join("\n");

        assert_eq!(protect_image_references(&original), original);
        assert_eq!(
            restore_image_references(&original, "Texto corregido"),
            "Texto corregido"
        );
    }

    #[test]
    fn inline_review_contract_missing_inline_reference_precedes_its_following_anchor() {
        let original = format!("Antes {MARKDOWN_IMAGE} después");
        let corrected = "Antes corregido después";

        let restored = restore_image_references(&original, corrected);
        let reference = restored.find(MARKDOWN_IMAGE).unwrap();
        let following = restored.find("después").unwrap();

        assert!(reference < following);
        assert_eq!(restored.matches("Antes corregido").count(), 1);
        assert_eq!(restored.matches("después").count(), 1);
        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 1);
    }

    #[test]
    fn inline_review_contract_html_requires_a_real_quoted_src_attribute() {
        let lookalikes = [
            r#"<img data-src="lazy.png">"#,
            r#"<img alt='literal src="fake.png" text'>"#,
        ]
        .join("\n");

        assert_eq!(protect_image_references(&lookalikes), lookalikes);
        assert_eq!(
            restore_image_references(&lookalikes, "Texto corregido"),
            "Texto corregido"
        );
    }

    #[test]
    fn inline_review_contract_preserves_preexisting_marker_like_comments_exactly() {
        let note = "<!--OCRC_IMAGE_REFERENCE_NOTE-->";
        let original = format!("{note}\nAntes {MARKDOWN_IMAGE} después");

        let protected = protect_image_references(&original);
        assert_eq!(
            protected,
            format!("{note}\nAntes <!--OCRC_IMAGE_REFERENCE_0001--> después")
        );

        let restored = restore_image_references(&original, &protected);

        assert_eq!(restored, original);
        assert_eq!(restored.matches(note).count(), 1);
        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 1);
    }

    #[test]
    fn inline_round2_contract_continues_past_multibyte_non_image_tags() {
        let prefix = r#"Prefacio <abé> <section data-x="1">texto</section>"#;
        let original = format!("{prefix} {HTML_IMAGE}");

        let protected = protect_image_references(&original);
        assert_eq!(
            protected,
            format!("{prefix} <!--OCRC_IMAGE_REFERENCE_0001-->")
        );
        assert_eq!(restore_image_references(&original, &protected), original);
    }

    #[test]
    fn inline_round2_contract_ignores_img_text_in_comments_and_attribute_values() {
        let comment = r#"<!-- ejemplo <img src="comment.png"> -->"#;
        let attribute = r#"<div data-template='<img src="attribute.png">'>Texto</div>"#;
        let original = format!("{comment}\n{attribute}\n{HTML_IMAGE}");

        let protected = protect_image_references(&original);
        assert_eq!(
            protected,
            format!("{comment}\n{attribute}\n<!--OCRC_IMAGE_REFERENCE_0001-->")
        );
        assert_eq!(restore_image_references(&original, &protected), original);
    }

    #[test]
    fn inline_round2_contract_anchor_words_never_match_inside_larger_words() {
        let original = format!("cat {MARKDOWN_IMAGE}");
        let corrected = "concatenate permanece";

        let restored = restore_image_references(&original, corrected);
        let word_start = restored.find("concatenate").unwrap();
        let word_end = word_start + "concatenate".len();
        let reference = restored.find(MARKDOWN_IMAGE).unwrap();

        assert_eq!(restored.matches("concatenate permanece").count(), 1);
        assert!(reference < word_start || reference >= word_end);
        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 1);
    }

    #[test]
    fn inline_round2_contract_restores_exact_original_inline_separators() {
        for original in [
            format!("A{MARKDOWN_IMAGE}B"),
            format!("A {MARKDOWN_IMAGE} B"),
            format!("A\n{MARKDOWN_IMAGE}\nB"),
        ] {
            let protected = protect_image_references(&original);
            let corrected = protected.replace("<!--OCRC_IMAGE_REFERENCE_0001-->", "");

            assert_eq!(restore_image_references(&original, &corrected), original);
        }
    }

    #[test]
    fn inline_final_contract_clamps_original_offset_to_utf8_boundary_without_splitting_text() {
        let original = format!("A{MARKDOWN_IMAGE}B");

        for corrected in ["éX", "🙂X", "漢X"] {
            let restored = restore_image_references(&original, corrected);

            assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 1);
            assert_eq!(
                restored.matches(corrected).count(),
                1,
                "corrected multibyte text was split: {restored:?}"
            );
        }
    }

    #[test]
    fn inline_final_contract_literal_less_than_does_not_swallow_later_real_image() {
        let original = format!("2 < 3\n{HTML_IMAGE}");

        let protected = protect_image_references(&original);

        assert_eq!(protected, "2 < 3\n<!--OCRC_IMAGE_REFERENCE_0001-->");
        assert_eq!(restore_image_references(&original, &protected), original);
    }

    #[test]
    fn inline_slot_contract_ignores_markdown_inside_html_comments_and_attributes() {
        let comment = format!("<!-- plantilla {MARKDOWN_IMAGE} -->");
        let attribute =
            format!(r#"<div data-template="antes {MARKDOWN_IMAGE} después">Texto</div>"#);
        let original = format!("{comment}\n{attribute}\nFuera {MARKDOWN_IMAGE}");

        let protected = protect_image_references(&original);

        assert_eq!(
            protected,
            format!("{comment}\n{attribute}\nFuera <!--OCRC_IMAGE_REFERENCE_0001-->")
        );
        assert_eq!(restore_image_references(&original, &protected), original);
    }

    #[test]
    fn inline_slot_contract_replays_original_placement_around_surviving_markers() {
        let standalone_original = format!("A\n{MARKDOWN_IMAGE}\nB");
        let moved_inline = "A movido <!--OCRC_IMAGE_REFERENCE_0001--> B movido";
        let standalone_restored = restore_image_references(&standalone_original, moved_inline);
        assert!(
            standalone_restored
                .lines()
                .any(|line| line == MARKDOWN_IMAGE),
            "standalone reference was left inline: {standalone_restored:?}"
        );
        assert_eq!(standalone_restored.matches("A movido").count(), 1);
        assert_eq!(standalone_restored.matches("B movido").count(), 1);

        for (original, corrected) in [
            (
                format!("A {MARKDOWN_IMAGE} B"),
                "A<!--OCRC_IMAGE_REFERENCE_0001-->B".to_string(),
            ),
            (
                format!("A{MARKDOWN_IMAGE}B"),
                "A  <!--OCRC_IMAGE_REFERENCE_0001-->  B".to_string(),
            ),
        ] {
            assert_eq!(restore_image_references(&original, &corrected), original);
        }
    }

    #[test]
    fn inline_identity_contract_binds_surviving_marker_to_its_original_occurrence() {
        let original = format!("Inicio {MARKDOWN_IMAGE} entre {MARKDOWN_IMAGE} final");
        let corrected =
            "Inicio corregido entre corregido <!--OCRC_IMAGE_REFERENCE_0002--> final corregido";

        let restored = restore_image_references(&original, corrected);
        let occurrences: Vec<usize> = restored
            .match_indices(MARKDOWN_IMAGE)
            .map(|(position, _)| position)
            .collect();
        let intervening = restored.find("entre corregido").unwrap();

        assert_eq!(occurrences.len(), 2);
        assert!(occurrences[0] < intervening);
        assert!(intervening < occurrences[1]);
        assert_eq!(restored.matches("Inicio corregido").count(), 1);
        assert_eq!(restored.matches("entre corregido").count(), 1);
        assert_eq!(restored.matches("final corregido").count(), 1);
    }

    #[test]
    fn inline_identity_contract_ignores_marker_comments_inside_corrected_inert_html() {
        let marker = "<!--OCRC_IMAGE_REFERENCE_0001-->";
        let comment = format!("<!-- plantilla {marker} fin -->");
        let attribute = format!(r#"<div data-template="{marker}">Texto</div>"#);
        let original = format!("Antes {MARKDOWN_IMAGE} después");
        let corrected = format!("Antes corregido\n{comment}\n{attribute}\ndespués");

        let restored = restore_image_references(&original, &corrected);
        let attribute_end = restored.find(&attribute).unwrap() + attribute.len();
        let reference = restored.find(MARKDOWN_IMAGE).unwrap();
        let following = restored.find("después").unwrap();

        assert!(reference > attribute_end);
        assert!(reference < following);
        assert_eq!(restored.matches(&comment).count(), 1);
        assert_eq!(restored.matches(&attribute).count(), 1);
        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 1);
        assert_eq!(restored.matches("Antes corregido").count(), 1);
    }

    #[test]
    fn commit_is_atomic_and_restore_is_one_level() {
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto original");
        let conn = correction_db(&original);
        let protected = protect_image_references(&original);
        let model_output = protected.replace("Texto original", "Texto corregido");

        let corrected = commit_asset_correction_if_current(
            &conn,
            "asset-1",
            &model_output,
            original.as_str(),
        )
        .expect("commit correction");

        assert_eq!(corrected.matches(MARKDOWN_IMAGE).count(), 1);
        assert_eq!(extraction_text(&conn), corrected);
        assert!(can_restore_original(&conn, "asset-1").unwrap());
        let stored_original: String = conn
            .query_row(
                "SELECT original_text_content FROM ocr_correction_backups WHERE asset_id = 'asset-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_original, original);
        let stored_result: String = conn
            .query_row(
                "SELECT result FROM llm_results WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_result, corrected);

        let restored = restore_original(&conn, "asset-1").expect("restore original");
        assert_eq!(restored, original);
        assert_eq!(extraction_text(&conn), original);
        assert!(!can_restore_original(&conn, "asset-1").unwrap());
        assert!(restore_original(&conn, "asset-1").is_err());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM llm_results WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn ocr_correction_contract_rejects_whitespace_before_transaction_and_preserves_fresh_state() {
        let original = "Texto original";
        let conn = correction_db(original);
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();

        let error =
            commit_asset_correction_if_current(&conn, "asset-1", " \n\t ", original).unwrap_err();
        let normalized_error = error.to_ascii_lowercase();
        assert!(
            normalized_error.contains("empty") || normalized_error.contains("whitespace"),
            "unexpected whitespace-output error: {error}"
        );

        conn.execute_batch("ROLLBACK").unwrap();
        ensure_schema(&conn).unwrap();
        assert_eq!(extraction_text(&conn), original);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM ocr_correction_backups WHERE asset_id = 'asset-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM llm_results
                 WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn ocr_correction_contract_whitespace_preserves_existing_backup_extraction_and_result() {
        let conn = correction_db("Texto original");
        let corrected = commit_asset_correction_if_current(
            &conn,
            "asset-1",
            "Texto corregido",
            "Texto original",
        )
        .unwrap();
        let extraction_before = extraction_text(&conn);
        let backup_before: (String, i64) = conn
            .query_row(
                "SELECT original_text_content, created_at
                 FROM ocr_correction_backups WHERE asset_id = 'asset-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let result_before: (String, i64) = conn
            .query_row(
                "SELECT result, created_at FROM llm_results
                 WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        let error = commit_asset_correction_if_current(
            &conn,
            "asset-1",
            "\r\n  ",
            corrected.as_str(),
        )
        .unwrap_err();
        let normalized_error = error.to_ascii_lowercase();
        assert!(
            normalized_error.contains("empty") || normalized_error.contains("whitespace"),
            "unexpected whitespace-output error: {error}"
        );

        assert_eq!(extraction_text(&conn), extraction_before);
        assert_eq!(extraction_before, corrected);
        assert_eq!(
            conn.query_row(
                "SELECT original_text_content, created_at
                 FROM ocr_correction_backups WHERE asset_id = 'asset-1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
            backup_before
        );
        assert_eq!(
            conn.query_row(
                "SELECT result, created_at FROM llm_results
                 WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
            result_before
        );
    }

    #[test]
    fn ocr_correction_contract_restore_rolls_back_all_state_when_backup_delete_fails() {
        let conn = correction_db("Texto original");
        let corrected = commit_asset_correction_if_current(
            &conn,
            "asset-1",
            "Texto corregido",
            "Texto original",
        )
        .unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_ocr_backup_delete
             BEFORE DELETE ON ocr_correction_backups
             BEGIN
               SELECT RAISE(ABORT, 'forced backup delete failure');
             END;",
        )
        .unwrap();

        let error = restore_original(&conn, "asset-1").unwrap_err();

        assert!(error.contains("backup"));
        assert_eq!(extraction_text(&conn), corrected);
        assert!(can_restore_original(&conn, "asset-1").unwrap());
        assert_eq!(
            conn.query_row(
                "SELECT result FROM llm_results
                 WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            corrected
        );
    }

    #[test]
    fn concurrency_contract_rejects_stale_correction_without_touching_any_ocrc_state() {
        let conn = correction_db("Texto original");
        let expected_extraction = commit_asset_correction_if_current(
            &conn,
            "asset-1",
            "Corrección previa",
            "Texto original",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO llm_results(id, target_id, target_type, job_type, result, created_at)
             VALUES ('other-result', 'asset-2', 'asset', 'summarize', 'Otro resultado', 5)",
            [],
        )
        .unwrap();
        let backup_before: (String, i64) = conn
            .query_row(
                "SELECT original_text_content, created_at
                 FROM ocr_correction_backups WHERE asset_id = 'asset-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let correction_before: (String, i64) = conn
            .query_row(
                "SELECT result, created_at FROM llm_results
                 WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        conn.execute(
            "UPDATE extractions
             SET text_content = 'Edición manual más reciente'
             WHERE asset_id = 'asset-1'",
            [],
        )
        .unwrap();

        let error = commit_asset_correction_if_current(
            &conn,
            "asset-1",
            "Resultado OCRC obsoleto",
            &expected_extraction,
        )
        .unwrap_err();
        let normalized_error = error.to_ascii_lowercase();

        assert!(
            normalized_error.contains("stale")
                || normalized_error.contains("concurrent")
                || normalized_error.contains("changed"),
            "unexpected stale-correction error: {error}"
        );
        assert_eq!(extraction_text(&conn), "Edición manual más reciente");
        assert_eq!(
            conn.query_row(
                "SELECT original_text_content, created_at
                 FROM ocr_correction_backups WHERE asset_id = 'asset-1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
            backup_before
        );
        assert_eq!(
            conn.query_row(
                "SELECT result, created_at FROM llm_results
                 WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
            correction_before
        );
        assert_eq!(
            conn.query_row(
                "SELECT result FROM llm_results WHERE id = 'other-result'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "Otro resultado"
        );
    }

    #[test]
    fn second_correction_keeps_the_first_original_backup() {
        let original = "# Documento\n\nTexto original";
        let conn = correction_db(original);

        commit_asset_correction_if_current(&conn, "asset-1", "Primera corrección", &original)
            .unwrap();
        commit_asset_correction_if_current(
            &conn,
            "asset-1",
            "Segunda corrección",
            "Primera corrección",
        )
        .unwrap();

        let restored = restore_original(&conn, "asset-1").unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn missing_extraction_or_backup_never_creates_partial_version_state() {
        let conn = correction_db("Texto original");

        let error =
            commit_asset_correction_if_current(&conn, "missing", "corregido", "").unwrap_err();
        assert!(error.contains("extraction"));
        assert!(!can_restore_original(&conn, "missing").unwrap());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM llm_results WHERE target_id = 'missing'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        assert!(restore_original(&conn, "asset-1").is_err());
    }

    #[test]
    fn clearing_state_removes_backup_and_correction_without_touching_fresh_extraction() {
        let conn = correction_db("Texto original");
        commit_asset_correction_if_current(&conn, "asset-1", "Texto corregido", "Texto original")
            .unwrap();
        conn.execute(
            "UPDATE extractions SET text_content = 'OCR fresco' WHERE asset_id = 'asset-1'",
            [],
        )
        .unwrap();

        clear_asset_state(&conn, "asset-1").unwrap();

        assert_eq!(extraction_text(&conn), "OCR fresco");
        assert!(!can_restore_original(&conn, "asset-1").unwrap());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM llm_results WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
    }
}
