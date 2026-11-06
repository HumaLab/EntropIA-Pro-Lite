//! Manual layout correction: re-recognize ONE layout block after the user
//! moved or resized its box (or drew a new box after it).
//!
//! GLM-OCR's layout pass sometimes misses a box or cuts a paragraph short, and
//! the text under the missing part never reaches the extraction. The viewer
//! lets the user fix the box geometry; this command then:
//!
//! 1. crops the page image to the corrected box,
//! 2. sends only that crop to GLM-OCR,
//! 3. writes the new geometry and content back into the stored layout, keeping
//!    the block's reading-order position (a new box goes right after the block
//!    it was created from, and the later blocks of that page shift by one),
//! 4. splices the recognized text into the extraction at the same position:
//!    it replaces the block's previous text, or, when that text cannot be
//!    found (the block was empty, or the user edited it), anchors on the
//!    nearest neighbor block of the reading order.
//!
//! The GLM-OCR call runs with the database released; the write re-checks that
//! the block still has the geometry the user edited, so a layout that changed
//! meanwhile (a re-OCR finished) is never overwritten with a stale index.

use super::glm_ocr::{GlmOcrClient, GlmOcrResponse};
use crate::db::commands::run_blocking_db_task;
use crate::db::state::AppDbState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

/// Pixels of page context kept around the box, in rendered-image pixels, so
/// glyphs that touch the edge of the box survive the crop.
const CROP_PADDING_PX: u32 = 6;
/// GLM-OCR rejects or misreads tiny images; crops shorter than this on either
/// side are upscaled before upload.
const MIN_CROP_SIDE_PX: u32 = 64;
/// A box smaller than this (in layout pixels) is a slip of the mouse.
const MIN_BOX_SIDE: f64 = 4.0;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LayoutBoxInput {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutBlockEditMode {
    /// The block keeps its position in the reading order; its box and text change.
    Replace,
    /// A new block is created right after the referenced one.
    InsertAfter,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutBlockReocrRequest {
    pub asset_id: String,
    /// Index of the block in the stored `layouts.blocks` JSON array (the
    /// frontend's `layout-block-<index>` id).
    pub block_index: usize,
    /// The block's box as the frontend loaded it; a mismatch means the layout
    /// changed under the user and the edit is refused.
    pub expected_bbox: LayoutBoxInput,
    /// The corrected box, in the block's own `image_width`/`image_height` space.
    pub bbox: LayoutBoxInput,
    pub mode: LayoutBlockEditMode,
    /// The extraction text the user currently sees (it may carry manual
    /// edits not persisted yet); the recognized text is spliced into it.
    pub current_text: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LayoutBlockReocrResult {
    /// The full extraction text after the splice, already persisted.
    pub text_content: String,
    /// The text recognized inside the corrected box.
    pub block_content: String,
    /// Index of the edited (or created) block in the stored blocks array.
    pub block_index: usize,
    /// `false` when no neighbor could be located in the text and the block
    /// text was appended at the end instead.
    pub anchored: bool,
}

// ── Pure helpers ────────────────────────────────────────────────────────────

fn bbox_of(value: &Value) -> Option<LayoutBoxInput> {
    let bbox = value.get("bbox")?;
    Some(LayoutBoxInput {
        x: bbox.get("x")?.as_f64()?,
        y: bbox.get("y")?.as_f64()?,
        width: bbox.get("width")?.as_f64()?,
        height: bbox.get("height")?.as_f64()?,
    })
}

fn same_box(a: &LayoutBoxInput, b: &LayoutBoxInput) -> bool {
    const TOLERANCE: f64 = 0.5;
    (a.x - b.x).abs() <= TOLERANCE
        && (a.y - b.y).abs() <= TOLERANCE
        && (a.width - b.width).abs() <= TOLERANCE
        && (a.height - b.height).abs() <= TOLERANCE
}

fn int_field(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_f64().map(|float| float.round() as i64))
    })
}

fn page_of(value: &Value) -> i64 {
    int_field(value, "page")
        .filter(|page| *page >= 1)
        .unwrap_or(1)
}

fn order_of(value: &Value) -> i64 {
    int_field(value, "order").unwrap_or(0)
}

fn bbox_json(bbox: &LayoutBoxInput) -> Value {
    json!({
        "x": bbox.x.round() as i64,
        "y": bbox.y.round() as i64,
        "width": bbox.width.round().max(1.0) as i64,
        "height": bbox.height.round().max(1.0) as i64,
    })
}

/// Clamps a box to its `[0, w] × [0, h]` image and refuses degenerate ones.
fn clamp_box(bbox: &LayoutBoxInput, width: f64, height: f64) -> Result<LayoutBoxInput, String> {
    if ![bbox.x, bbox.y, bbox.width, bbox.height]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err("La caja editada tiene coordenadas inválidas.".to_string());
    }
    let (limit_w, limit_h) = if width > 0.0 && height > 0.0 {
        (width, height)
    } else {
        (f64::MAX, f64::MAX)
    };
    let x1 = bbox.x.clamp(0.0, limit_w);
    let y1 = bbox.y.clamp(0.0, limit_h);
    let x2 = (bbox.x + bbox.width).clamp(0.0, limit_w);
    let y2 = (bbox.y + bbox.height).clamp(0.0, limit_h);
    if x2 - x1 < MIN_BOX_SIDE || y2 - y1 < MIN_BOX_SIDE {
        return Err("La caja editada es demasiado chica para reconocer texto.".to_string());
    }
    Ok(LayoutBoxInput {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    })
}

/// Maps a box from layout space (`layout_w × layout_h`) onto a rendered image
/// of `image_w × image_h`, padded and clamped. Returns `(x, y, w, h)` pixels.
fn crop_rect(
    bbox: &LayoutBoxInput,
    layout_w: f64,
    layout_h: f64,
    image_w: u32,
    image_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    if image_w == 0 || image_h == 0 {
        return None;
    }
    let (scale_x, scale_y) = if layout_w > 0.0 && layout_h > 0.0 {
        (image_w as f64 / layout_w, image_h as f64 / layout_h)
    } else {
        (1.0, 1.0)
    };
    let pad = CROP_PADDING_PX as f64;
    let x1 = (bbox.x * scale_x - pad).floor().clamp(0.0, image_w as f64);
    let y1 = (bbox.y * scale_y - pad).floor().clamp(0.0, image_h as f64);
    let x2 = ((bbox.x + bbox.width) * scale_x + pad)
        .ceil()
        .clamp(0.0, image_w as f64);
    let y2 = ((bbox.y + bbox.height) * scale_y + pad)
        .ceil()
        .clamp(0.0, image_h as f64);
    let width = (x2 - x1) as u32;
    let height = (y2 - y1) as u32;
    (width > 0 && height > 0).then_some((x1 as u32, y1 as u32, width, height))
}

/// The recognized crop as `(block content, extraction text, first label)`.
/// The block keeps the raw contents (as the layout pass stores them); the
/// extraction text prefers GLM's own markdown, exactly like a full-page OCR.
fn crop_response_content(response: &GlmOcrResponse) -> (String, String, Option<String>) {
    let mut details: Vec<(i32, usize, &super::glm_ocr::GlmOcrLayoutDetail)> = response
        .layout_details
        .iter()
        .flatten()
        .enumerate()
        .map(|(position, detail)| (detail.index.unwrap_or(position as i32), position, detail))
        .collect();
    details.sort_by_key(|(index, position, _)| (*index, *position));

    let mut first_label = None;
    let contents: Vec<String> = details
        .iter()
        .filter_map(|(_, _, detail)| {
            let content = detail.content.as_deref().unwrap_or_default().trim();
            if content.is_empty() {
                return None;
            }
            if first_label.is_none() {
                first_label = detail.label.clone();
            }
            Some(content.to_string())
        })
        .collect();
    let block_content = contents.join("\n\n");
    let markdown = response.md_results.trim();
    let text = if markdown.is_empty() {
        block_content.clone()
    } else {
        markdown.to_string()
    };
    (block_content, text, first_label)
}

/// Contents of the blocks before (nearest first) and after (nearest first)
/// `index` in reading order (page, then order), skipping empty ones.
fn reading_order_neighbors(blocks: &[Value], index: usize) -> (Vec<String>, Vec<String>) {
    let mut ordered: Vec<usize> = (0..blocks.len()).collect();
    ordered.sort_by_key(|&i| (page_of(&blocks[i]), order_of(&blocks[i]), i));
    let position = ordered.iter().position(|&i| i == index).unwrap_or(0);
    let content = |i: &usize| {
        blocks[*i]
            .get("content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    let before = ordered[..position]
        .iter()
        .rev()
        .filter_map(content)
        .collect();
    let after = ordered[position + 1..].iter().filter_map(content).collect();
    (before, after)
}

/// Splices `new_text` into `text` at the block's place in the reading order.
///
/// - `old` found → replaced (the occurrence after the nearest previous
///   neighbor when the text repeats);
/// - otherwise inserted after the nearest previous neighbor found, or before
///   the nearest next neighbor found;
/// - nothing to anchor on → appended (the second value is then `false`).
pub(crate) fn splice_block_text(
    text: &str,
    old: Option<&str>,
    before: &[String],
    after: &[String],
    new_text: &str,
) -> (String, bool) {
    let new_text = new_text.trim();
    let previous_end = before
        .iter()
        .find_map(|anchor| text.find(anchor.as_str()).map(|start| start + anchor.len()));

    if let Some(old) = old.map(str::trim).filter(|old| !old.is_empty()) {
        let start = previous_end
            .and_then(|from| text[from..].find(old).map(|offset| from + offset))
            .or_else(|| text.find(old));
        if let Some(start) = start {
            let mut spliced = String::with_capacity(text.len() + new_text.len());
            spliced.push_str(&text[..start]);
            spliced.push_str(new_text);
            spliced.push_str(&text[start + old.len()..]);
            return (spliced, true);
        }
    }

    if let Some(end) = previous_end {
        let mut spliced = String::with_capacity(text.len() + new_text.len() + 2);
        spliced.push_str(&text[..end]);
        spliced.push_str("\n\n");
        spliced.push_str(new_text);
        spliced.push_str(&text[end..]);
        return (spliced, true);
    }

    if let Some(start) = after.iter().find_map(|anchor| text.find(anchor.as_str())) {
        let mut spliced = String::with_capacity(text.len() + new_text.len() + 2);
        spliced.push_str(&text[..start]);
        spliced.push_str(new_text);
        spliced.push_str("\n\n");
        spliced.push_str(&text[start..]);
        return (spliced, true);
    }

    if text.trim().is_empty() {
        return (new_text.to_string(), true);
    }
    (format!("{}\n\n{new_text}", text.trim_end()), false)
}

/// Writes the edit into the stored blocks/regions arrays. Returns the index
/// of the edited or created block.
fn apply_block_edit(
    blocks: &mut Vec<Value>,
    regions: &mut [Value],
    index: usize,
    mode: LayoutBlockEditMode,
    bbox: &LayoutBoxInput,
    content: &str,
    detected_label: Option<&str>,
) -> Result<usize, String> {
    let source = blocks
        .get(index)
        .cloned()
        .ok_or_else(|| "El bloque de layout ya no existe.".to_string())?;
    let page = page_of(&source);
    match mode {
        LayoutBlockEditMode::Replace => {
            let old_bbox = bbox_of(&source);
            let label = source.get("label").cloned().unwrap_or(json!("text"));
            let block = &mut blocks[index];
            block["bbox"] = bbox_json(bbox);
            block["content"] = json!(content);
            // The region drawn for this block (GLM writes one per block, with
            // the same box) follows the new geometry.
            if let Some(old_bbox) = old_bbox {
                if let Some(region) = regions.iter_mut().find(|region| {
                    page_of(region) == page
                        && bbox_of(region)
                            .is_some_and(|region_box| same_box(&region_box, &old_bbox))
                        && region.get("category") == Some(&label)
                }) {
                    region["bbox"] = bbox_json(bbox);
                }
            }
            Ok(index)
        }
        LayoutBlockEditMode::InsertAfter => {
            let order = order_of(&source);
            for block in blocks.iter_mut() {
                if page_of(block) == page && order_of(block) > order {
                    block["order"] = json!(order_of(block) + 1);
                }
            }
            let label = detected_label
                .filter(|label| !label.trim().is_empty())
                .unwrap_or("text");
            let mut created = json!({
                "page": page,
                "label": label,
                "content": content,
                "bbox": bbox_json(bbox),
                "order": order + 1,
                "group_id": source.get("group_id").cloned().unwrap_or(json!(page)),
            });
            for key in ["image_width", "image_height"] {
                if let Some(value) = source.get(key) {
                    created[key] = value.clone();
                }
            }
            blocks.insert(index + 1, created);
            Ok(index + 1)
        }
    }
}

fn push_region_for_created_block(regions: &mut Vec<Value>, block: &Value) {
    let mut region = json!({
        "page": page_of(block),
        "category": block.get("label").cloned().unwrap_or(json!("text")),
        "bbox": block.get("bbox").cloned().unwrap_or(Value::Null),
        "confidence": 1.0,
    });
    for key in ["image_width", "image_height"] {
        if let Some(value) = block.get(key) {
            region[key] = value.clone();
        }
    }
    regions.push(region);
}

fn parse_json_array(raw: &str, field: &str) -> Result<Vec<Value>, String> {
    serde_json::from_str::<Vec<Value>>(raw)
        .map_err(|e| format!("No pude leer layouts.{field}: {e}"))
}

fn encode_png(image: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| format!("No pude codificar el recorte: {e}"))?;
    Ok(bytes)
}

/// Crops `page` to the box and returns PNG bytes ready for GLM-OCR.
fn crop_to_png(
    page: &image::DynamicImage,
    bbox: &LayoutBoxInput,
    layout_w: f64,
    layout_h: f64,
) -> Result<Vec<u8>, String> {
    let (x, y, width, height) = crop_rect(bbox, layout_w, layout_h, page.width(), page.height())
        .ok_or_else(|| "La caja editada queda fuera de la página.".to_string())?;
    let mut crop = page.crop_imm(x, y, width, height);
    let shortest = crop.width().min(crop.height());
    if shortest < MIN_CROP_SIDE_PX {
        let factor = (MIN_CROP_SIDE_PX as f64 / shortest.max(1) as f64).ceil() as u32;
        crop = crop.resize(
            crop.width() * factor,
            crop.height() * factor,
            image::imageops::FilterType::Lanczos3,
        );
    }
    encode_png(&crop)
}

// ── Command ─────────────────────────────────────────────────────────────────

struct LoadedBlock {
    asset_path: String,
    asset_type: String,
    page: i64,
    layout_w: f64,
    layout_h: f64,
    api_key: String,
}

fn load_block(
    db: &AppDbState,
    request: &LayoutBlockReocrRequest,
) -> Result<(LoadedBlock, String), String> {
    let api_key = crate::settings::read_settings_unlocked(
        &db.ui_conn,
        &[super::OCRH_SETTING_GLM_OCR_API_KEY],
        &crate::settings::KeyringSecretStore,
    )?
    .into_iter()
    .next()
    .flatten()
    .unwrap_or_default()
    .trim()
    .to_string();
    if api_key.is_empty() {
        return Err(
            "GLM-OCR no está configurado. Andá a Configuración > OCRH y cargá una API key para reconocer cajas del layout."
                .to_string(),
        );
    }

    let conn = db
        .ui_conn
        .lock()
        .map_err(|e| format!("DB lock poisoned: {e}"))?;
    let (stored_path, asset_type): (String, String) = conn
        .query_row(
            "SELECT path, type FROM assets WHERE id = ?1",
            [&request.asset_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("No encontré el asset {}: {e}", request.asset_id))?;
    let (blocks_json, layout_w, layout_h): (String, i64, i64) = conn
        .query_row(
            "SELECT blocks, image_width, image_height FROM layouts WHERE asset_id = ?1",
            [&request.asset_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| format!("El asset no tiene layout guardado: {e}"))?;
    drop(conn);

    let blocks = parse_json_array(&blocks_json, "blocks")?;
    let block = blocks
        .get(request.block_index)
        .ok_or_else(|| "El bloque de layout ya no existe; recargá el layout.".to_string())?;
    ensure_expected_bbox(block, &request.expected_bbox)?;
    let layout_w = int_field(block, "image_width").unwrap_or(layout_w) as f64;
    let layout_h = int_field(block, "image_height").unwrap_or(layout_h) as f64;
    Ok((
        LoadedBlock {
            asset_path: String::new(),
            asset_type,
            page: page_of(block),
            layout_w,
            layout_h,
            api_key,
        },
        stored_path,
    ))
}

fn ensure_expected_bbox(block: &Value, expected: &LayoutBoxInput) -> Result<(), String> {
    match bbox_of(block) {
        Some(current) if same_box(&current, expected) => Ok(()),
        _ => Err(
            "El layout cambió mientras lo editabas; recargá el layout y volvé a intentar."
                .to_string(),
        ),
    }
}

fn render_page(loaded: &LoadedBlock) -> Result<image::DynamicImage, String> {
    let bytes = std::fs::read(&loaded.asset_path)
        .map_err(|e| format!("Failed to read {}: {e}", loaded.asset_path))?;
    if loaded.asset_type == "pdf" || bytes.starts_with(b"%PDF-") {
        let page_index = usize::try_from(loaded.page - 1).unwrap_or(0);
        super::pdf::render_pdf_page_to_unrotated_image(&bytes, page_index)
    } else {
        image::load_from_memory(&bytes).map_err(|e| format!("No pude abrir la imagen: {e}"))
    }
}

/// Writes the new layout and extraction text in one transaction.
fn persist_edit(
    db: &AppDbState,
    request: &LayoutBlockReocrRequest,
    bbox: &LayoutBoxInput,
    block_content: &str,
    text_piece: &str,
    detected_label: Option<&str>,
) -> Result<LayoutBlockReocrResult, String> {
    let conn = db
        .ui_conn
        .lock()
        .map_err(|e| format!("DB lock poisoned: {e}"))?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin layout edit: {e}"))?;
    let (regions_json, blocks_json): (String, String) = tx
        .query_row(
            "SELECT regions, blocks FROM layouts WHERE asset_id = ?1",
            [&request.asset_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("El asset no tiene layout guardado: {e}"))?;
    let mut blocks = parse_json_array(&blocks_json, "blocks")?;
    let mut regions = parse_json_array(&regions_json, "regions")?;
    let block = blocks
        .get(request.block_index)
        .ok_or_else(|| "El bloque de layout ya no existe; recargá el layout.".to_string())?;
    ensure_expected_bbox(block, &request.expected_bbox)?;

    let old_content = match request.mode {
        LayoutBlockEditMode::Replace => block
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string),
        LayoutBlockEditMode::InsertAfter => None,
    };
    let (mut before, after) = reading_order_neighbors(&blocks, request.block_index);
    if request.mode == LayoutBlockEditMode::InsertAfter {
        // The new block goes right after the source block: the source block
        // itself is the nearest previous anchor.
        if let Some(own) = block
            .get("content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            before.insert(0, own.to_string());
        }
    }
    let (text_content, anchored) = splice_block_text(
        &request.current_text,
        old_content.as_deref(),
        &before,
        &after,
        text_piece,
    );

    let edited_index = apply_block_edit(
        &mut blocks,
        &mut regions,
        request.block_index,
        request.mode,
        bbox,
        block_content,
        detected_label,
    )?;
    if request.mode == LayoutBlockEditMode::InsertAfter {
        let created = blocks[edited_index].clone();
        push_region_for_created_block(&mut regions, &created);
    }

    let blocks_json = serde_json::to_string(&blocks)
        .map_err(|e| format!("Failed to serialize layout blocks: {e}"))?;
    let regions_json = serde_json::to_string(&regions)
        .map_err(|e| format!("Failed to serialize layout regions: {e}"))?;
    tx.execute(
        "UPDATE layouts SET blocks = ?1, regions = ?2 WHERE asset_id = ?3",
        rusqlite::params![blocks_json, regions_json, request.asset_id],
    )
    .map_err(|e| format!("Failed to update layout: {e}"))?;
    super::update_extraction_text(&tx, &request.asset_id, &text_content)?;
    tx.commit()
        .map_err(|e| format!("Failed to commit layout edit: {e}"))?;

    Ok(LayoutBlockReocrResult {
        text_content,
        block_content: block_content.to_string(),
        block_index: edited_index,
        anchored,
    })
}

/// Re-recognizes one layout block inside a user-corrected box with GLM-OCR,
/// keeping its place in the reading order. See the module docs.
#[tauri::command]
pub async fn reocr_layout_block(
    request: LayoutBlockReocrRequest,
    app: AppHandle,
    db: State<'_, AppDbState>,
) -> Result<LayoutBlockReocrResult, String> {
    let db = db.inner().clone();

    let read_db = db.clone();
    let read_request = request.clone();
    let (mut loaded, stored_path) =
        run_blocking_db_task(move || load_block(&read_db, &read_request)).await?;
    loaded.asset_path = crate::path_utils::resolve_asset_path_at_boundary(&stored_path, &app)?;
    let bbox = clamp_box(&request.bbox, loaded.layout_w, loaded.layout_h)?;

    let render_app = app.clone();
    let crop_png = tauri::async_runtime::spawn_blocking(move || {
        if loaded.asset_type == "pdf" {
            super::pdf::init_pdfium_path(&render_app);
        }
        let page = render_page(&loaded)?;
        let png = crop_to_png(&page, &bbox, loaded.layout_w, loaded.layout_h)?;
        Ok::<_, String>((png, loaded.api_key))
    })
    .await
    .map_err(|e| format!("Layout crop task failed: {e}"))?;
    let (png, api_key) = crop_png?;

    let payload = super::encode_bytes_for_glm_ocr(&png)?;
    let response = GlmOcrClient::new(api_key).parse_file(&payload).await?;
    let (block_content, text_piece, detected_label) = crop_response_content(&response);
    if block_content.trim().is_empty() {
        return Err("GLM-OCR no encontró texto dentro de la caja editada.".to_string());
    }

    run_blocking_db_task(move || {
        persist_edit(
            &db,
            &request,
            &bbox,
            &block_content,
            &text_piece,
            detected_label.as_deref(),
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(x: f64, y: f64, width: f64, height: f64) -> LayoutBoxInput {
        LayoutBoxInput {
            x,
            y,
            width,
            height,
        }
    }

    fn block(page: i64, order: i64, content: &str, bbox: (i64, i64, i64, i64)) -> Value {
        json!({
            "page": page,
            "image_width": 1000,
            "image_height": 2000,
            "label": "text",
            "content": content,
            "bbox": {"x": bbox.0, "y": bbox.1, "width": bbox.2, "height": bbox.3},
            "order": order,
            "group_id": page,
        })
    }

    #[test]
    fn replacing_a_block_keeps_its_place_in_the_text() {
        let text = "Uno.\n\nDos cortado\n\nTres.";
        let (spliced, anchored) = splice_block_text(
            text,
            Some("Dos cortado"),
            &["Uno.".to_string()],
            &["Tres.".to_string()],
            "Dos cortado y su final.",
        );
        assert!(anchored);
        assert_eq!(spliced, "Uno.\n\nDos cortado y su final.\n\nTres.");
    }

    #[test]
    fn a_repeated_block_text_is_replaced_after_its_previous_neighbor() {
        let text = "Nota\n\nA\n\nNota\n\nB";
        let (spliced, _) =
            splice_block_text(text, Some("Nota"), &["A".to_string()], &[], "Nota completa");
        assert_eq!(spliced, "Nota\n\nA\n\nNota completa\n\nB");
    }

    #[test]
    fn a_block_whose_text_was_edited_is_inserted_after_the_previous_neighbor() {
        let text = "Uno.\n\nTres.";
        let (spliced, anchored) = splice_block_text(
            text,
            Some("texto que ya no está"),
            &["Uno.".to_string()],
            &["Tres.".to_string()],
            "Dos.",
        );
        assert!(anchored);
        assert_eq!(spliced, "Uno.\n\nDos.\n\nTres.");
    }

    #[test]
    fn a_first_block_is_inserted_before_the_next_neighbor() {
        let (spliced, anchored) =
            splice_block_text("Dos.\n\nTres.", None, &[], &["Dos.".to_string()], "Uno.");
        assert!(anchored);
        assert_eq!(spliced, "Uno.\n\nDos.\n\nTres.");
    }

    #[test]
    fn without_anchors_the_text_is_appended_and_reported() {
        let (spliced, anchored) = splice_block_text("Otra cosa", None, &[], &[], "Nuevo");
        assert!(!anchored);
        assert_eq!(spliced, "Otra cosa\n\nNuevo");
        let (spliced, anchored) = splice_block_text("", None, &[], &[], "Nuevo");
        assert!(anchored);
        assert_eq!(spliced, "Nuevo");
    }

    #[test]
    fn neighbors_follow_page_then_order_and_skip_empty_blocks() {
        let blocks = vec![
            block(1, 2, "B", (0, 0, 1, 1)),
            block(1, 1, "A", (0, 0, 1, 1)),
            block(1, 3, "", (0, 0, 1, 1)),
            block(2, 1, "D", (0, 0, 1, 1)),
            block(1, 4, "C", (0, 0, 1, 1)),
        ];
        let (before, after) = reading_order_neighbors(&blocks, 0);
        assert_eq!(before, vec!["A".to_string()]);
        assert_eq!(after, vec!["C".to_string(), "D".to_string()]);
    }

    #[test]
    fn replace_updates_the_block_and_its_region_only() {
        let mut blocks = vec![
            block(1, 1, "A", (10, 10, 100, 50)),
            block(1, 2, "B", (10, 80, 100, 50)),
        ];
        let mut regions = vec![
            json!({"page": 1, "category": "text", "bbox": {"x": 10, "y": 10, "width": 100, "height": 50}}),
            json!({"page": 1, "category": "text", "bbox": {"x": 10, "y": 80, "width": 100, "height": 50}}),
        ];
        let index = apply_block_edit(
            &mut blocks,
            &mut regions,
            1,
            LayoutBlockEditMode::Replace,
            &bx(10.0, 80.0, 100.0, 120.4),
            "B completo",
            Some("title"),
        )
        .expect("edit");
        assert_eq!(index, 1);
        assert_eq!(blocks[1]["content"], "B completo");
        assert_eq!(blocks[1]["label"], "text", "replace keeps the label");
        assert_eq!(blocks[1]["order"], 2, "replace keeps the reading order");
        assert_eq!(blocks[1]["bbox"]["height"], 120);
        assert_eq!(regions[1]["bbox"]["height"], 120);
        assert_eq!(regions[0]["bbox"]["height"], 50);
    }

    #[test]
    fn insert_after_shifts_the_later_blocks_of_the_same_page() {
        let mut blocks = vec![
            block(1, 1, "A", (10, 10, 100, 50)),
            block(1, 2, "C", (10, 200, 100, 50)),
            block(2, 1, "D", (10, 10, 100, 50)),
        ];
        let mut regions = Vec::new();
        let index = apply_block_edit(
            &mut blocks,
            &mut regions,
            0,
            LayoutBlockEditMode::InsertAfter,
            &bx(10.0, 70.0, 100.0, 100.0),
            "B",
            Some("text"),
        )
        .expect("insert");
        assert_eq!(index, 1);
        let orders: Vec<(String, i64, i64)> = blocks
            .iter()
            .map(|b| {
                (
                    b["content"].as_str().unwrap().to_string(),
                    page_of(b),
                    order_of(b),
                )
            })
            .collect();
        assert_eq!(
            orders,
            vec![
                ("A".to_string(), 1, 1),
                ("B".to_string(), 1, 2),
                ("C".to_string(), 1, 3),
                ("D".to_string(), 2, 1),
            ]
        );
        assert_eq!(blocks[1]["image_width"], 1000);
        push_region_for_created_block(&mut regions, &blocks[1]);
        assert_eq!(regions[0]["bbox"]["y"], 70);
    }

    #[test]
    fn crop_rect_scales_from_layout_space_and_pads() {
        // Layout space 1000×2000, rendered page 2000×4000 (×2).
        let rect =
            crop_rect(&bx(100.0, 200.0, 300.0, 50.0), 1000.0, 2000.0, 2000, 4000).expect("rect");
        let pad = CROP_PADDING_PX;
        assert_eq!(rect, (200 - pad, 400 - pad, 600 + 2 * pad, 100 + 2 * pad));
        // Clamped to the image.
        let rect =
            crop_rect(&bx(990.0, 1990.0, 50.0, 50.0), 1000.0, 2000.0, 1000, 2000).expect("rect");
        assert_eq!(rect.0 + rect.2, 1000);
        assert_eq!(rect.1 + rect.3, 2000);
    }

    #[test]
    fn clamp_box_refuses_degenerate_or_invalid_boxes() {
        assert!(clamp_box(&bx(0.0, 0.0, 2.0, 50.0), 100.0, 100.0).is_err());
        assert!(clamp_box(&bx(f64::NAN, 0.0, 20.0, 50.0), 100.0, 100.0).is_err());
        let clamped = clamp_box(&bx(-10.0, 90.0, 50.0, 50.0), 100.0, 100.0).expect("clamped");
        assert_eq!(clamped, bx(0.0, 90.0, 40.0, 10.0));
    }

    #[test]
    fn crop_response_joins_contents_in_index_order() {
        let response: GlmOcrResponse = serde_json::from_str(
            r##"{"md_results":"","layout_details":[[
                {"index":2,"label":"text","content":"segundo"},
                {"index":1,"label":"title","content":"# primero"},
                {"index":3,"label":"image","content":""}
            ]]}"##,
        )
        .expect("response");
        let (block, text, label) = crop_response_content(&response);
        assert_eq!(block, "# primero\n\nsegundo");
        assert_eq!(text, block);
        assert_eq!(label.as_deref(), Some("title"));
    }

    #[test]
    fn crop_to_png_upscales_tiny_crops() {
        let page = image::DynamicImage::new_rgb8(400, 400);
        let png = crop_to_png(&page, &bx(10.0, 10.0, 100.0, 8.0), 400.0, 400.0).expect("png");
        let decoded = image::load_from_memory(&png).expect("decode");
        assert!(decoded.height() >= MIN_CROP_SIDE_PX);
    }
}
