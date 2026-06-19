pub mod commands;
pub mod glm_ocr;
pub mod postprocess;
pub mod provider;

#[cfg(feature = "paddle-ocr")]
pub mod paddle;

#[cfg(feature = "local-ml")]
pub mod layout_onnx;
pub mod paddle_vl_types;
#[cfg(feature = "paddle-ocr")]
pub mod paddle_vl;
mod pdf;
pub mod pdf_probe;
pub mod reading_order;

// Dev-only visualization helpers for debugging layout detection.
// Compiled in debug builds only; the call site is also gated by cfg!(debug_assertions).
#[cfg(debug_assertions)]
mod debug_viz;

use crate::nlp::{lookup_item_id_for_asset, NlpJob, NlpQueue};
#[cfg(feature = "paddle-ocr")]
use crate::runtime::{managed_resource_path, RuntimeManager};
use base64::Engine;
use glm_ocr::{GlmOcrLayoutDetail, GlmOcrResponse};
// PaddleVlOutput is consumed only by the paddle-gated layout helpers.
#[cfg(feature = "paddle-ocr")]
use paddle_vl_types::PaddleVlOutput;
#[cfg(feature = "paddle-ocr")]
use paddle_vl::{create_paddle_vl_engine_result, PaddleVlEngine};
// Native PDF helpers are used only by the paddle-gated local OCR path. The lean
// GLM-OCR worker sends raw bytes to the remote provider and never touches pdfium.
#[cfg(feature = "paddle-ocr")]
use pdf::{extract_pdf_text, init_pdfium_path, is_quality_text, pdf_page_count};
use provider::LayoutCategory;
// The OcrProvider trait and Arc handle are used only by the paddle worker arm and
// its process_* helpers; the lean GLM worker calls the remote provider directly.
#[cfg(feature = "paddle-ocr")]
use provider::OcrProvider;
use serde::Serialize;
#[cfg(feature = "paddle-ocr")]
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

const OCRH_MODE_LOCAL: &str = "local";
const OCRH_MODE_GLM_OCR: &str = "glm_ocr";
// `auto` mode (try local PaddleVL, fall back to GLM) only exists on the paddle
// High path. The lean worker always routes to GLM, so this constant is dead there.
#[cfg(feature = "paddle-ocr")]
const OCRH_MODE_AUTO: &str = "auto";
const OCRH_SETTING_MODE: &str = "ocrh_mode";
const OCRH_SETTING_GLM_OCR_API_KEY: &str = "glm_ocr_api_key";

#[cfg(feature = "paddle-ocr")]
fn managed_runtime_root_for_ocr(
    app_handle: &AppHandle,
) -> Result<Option<std::path::PathBuf>, String> {
    managed_runtime_root_for_ocr_with(
        || RuntimeManager::new().ensure_ready_or_bootstrap(app_handle),
        || RuntimeManager::new().hydrated_runtime_root(app_handle),
    )
}

#[cfg(any(feature = "paddle-ocr", test))]
fn managed_runtime_root_for_ocr_with<E, H>(
    ensure_ready_or_bootstrap: E,
    hydrated_runtime_root: H,
) -> Result<Option<std::path::PathBuf>, String>
where
    E: FnOnce() -> Result<crate::runtime::status::RuntimeStatus, String>,
    H: FnOnce() -> Result<Option<std::path::PathBuf>, String>,
{
    let status = ensure_ready_or_bootstrap()?;
    if status.state != crate::runtime::status::RuntimeState::Healthy {
        return Ok(None);
    }

    hydrated_runtime_root()
}

// ── Event payloads ──────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct OcrProgressPayload {
    pub asset_id: String,
    pub pct: u8,
    pub stage: String,
}

#[derive(Clone, Serialize)]
pub struct OcrCompletePayload {
    pub asset_id: String,
    pub method: String,
    pub text_length: usize,
    pub text_content: String,
}

#[derive(Clone, Serialize)]
pub struct OcrErrorPayload {
    pub asset_id: String,
    pub error: String,
}

#[derive(Debug, Clone)]
struct ProcessedOcrOutput {
    ocr: provider::OcrOutput,
    layout: Option<LayoutPersistencePayload>,
}

#[derive(Debug, Clone, Serialize)]
struct LayoutPersistencePayload {
    model: String,
    image_width: u32,
    image_height: u32,
    regions: Vec<PersistedLayoutRegion>,
    blocks: Vec<PersistedLayoutBlock>,
}

#[derive(Debug, Clone, Serialize)]
struct PersistedLayoutRegion {
    page: u32,
    image_width: u32,
    image_height: u32,
    category: String,
    bbox: paddle_vl_types::PaddleVlBbox,
    confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
struct PersistedLayoutBlock {
    page: u32,
    image_width: u32,
    image_height: u32,
    label: String,
    content: String,
    bbox: paddle_vl_types::PaddleVlBbox,
    order: i32,
    group_id: i32,
}

// `from_page`/`push_page` build layout payloads from PaddleVL subprocess output;
// only the paddle-gated High OCR path constructs them. The lean GLM worker builds
// `LayoutPersistencePayload` directly from the GLM response instead.
#[cfg(feature = "paddle-ocr")]
impl LayoutPersistencePayload {
    fn from_page(page: u32, output: &PaddleVlOutput) -> Self {
        let mut payload = Self {
            model: output.method.clone(),
            image_width: output.image_width,
            image_height: output.image_height,
            regions: Vec::new(),
            blocks: Vec::new(),
        };
        payload.push_page(page, output);
        payload
    }

    fn push_page(&mut self, page: u32, output: &PaddleVlOutput) {
        self.image_width = self.image_width.max(output.image_width);
        self.image_height = self.image_height.max(output.image_height);

        self.regions
            .extend(output.regions.iter().map(|region| PersistedLayoutRegion {
                page,
                image_width: output.image_width,
                image_height: output.image_height,
                category: region.category.clone(),
                bbox: region.bbox.clone(),
                confidence: region.confidence,
            }));

        self.blocks
            .extend(output.blocks.iter().map(|block| PersistedLayoutBlock {
                page,
                image_width: output.image_width,
                image_height: output.image_height,
                label: block.label.clone(),
                content: block.content.clone(),
                bbox: block.bbox.clone(),
                order: block.order,
                group_id: block.group_id,
            }));
    }
}

#[cfg(feature = "paddle-ocr")]
fn ocr_output_from_paddlevl(output: &PaddleVlOutput) -> provider::OcrOutput {
    provider::OcrOutput {
        text: output.text.clone(),
        regions: output
            .regions
            .iter()
            .map(|region| provider::OcrRegion {
                text: String::new(),
                confidence: region.confidence,
                bbox: Some(provider::BoundingBox {
                    x: region.bbox.x,
                    y: region.bbox.y,
                    width: region.bbox.width as u32,
                    height: region.bbox.height as u32,
                }),
                column: None,
            })
            .collect(),
        method: output.method.clone(),
    }
}

fn glm_label_to_layout_category(label: &str) -> Option<LayoutCategory> {
    match label {
        "title" => Some(LayoutCategory::Title),
        "text" => Some(LayoutCategory::PlainText),
        "table" => Some(LayoutCategory::Table),
        "image" => Some(LayoutCategory::Figure),
        "formula" => Some(LayoutCategory::PlainText),
        _ => None,
    }
}

fn strip_html_tags(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut inside_tag = false;

    for ch in value.chars() {
        match ch {
            '<' => inside_tag = true,
            '>' => {
                inside_tag = false;
                result.push(' ');
            }
            _ if !inside_tag => result.push(ch),
            _ => {}
        }
    }

    result
}

fn normalize_glm_text_fragment(value: &str) -> String {
    strip_html_tags(value)
        .replace("<br>", " ")
        .replace("<br/>", " ")
        .replace("<br />", " ")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_lowercase()
}

fn collect_glm_markdown_titles(markdown: &str) -> std::collections::HashSet<String> {
    let mut titles = std::collections::HashSet::new();
    let mut inside_centered_block = false;
    let mut centered_block_has_heading = false;
    let mut centered_lines: Vec<String> = Vec::new();

    let flush_centered_block = |titles: &mut std::collections::HashSet<String>,
                                centered_block_has_heading: bool,
                                centered_lines: &mut Vec<String>| {
        if centered_block_has_heading {
            for line in centered_lines.iter() {
                let normalized = normalize_glm_text_fragment(line);
                if !normalized.is_empty() {
                    titles.insert(normalized);
                }
            }
        }
        centered_lines.clear();
    };

    for raw_line in markdown.lines() {
        let line = raw_line.trim();
        let line_without_html = strip_html_tags(line);
        let normalized_line = line_without_html.trim();

        if line.starts_with("<div") && line.contains("align=\"center\"") {
            inside_centered_block = true;
            centered_block_has_heading = false;
            centered_lines.clear();
            continue;
        }

        if inside_centered_block {
            if line.starts_with("</div") {
                flush_centered_block(&mut titles, centered_block_has_heading, &mut centered_lines);
                inside_centered_block = false;
                centered_block_has_heading = false;
                continue;
            }

            if normalized_line.starts_with('#') {
                centered_block_has_heading = true;
            }

            let cleaned = normalized_line.trim_start_matches('#').trim();
            if !cleaned.is_empty() {
                centered_lines.push(cleaned.to_string());
            }
            continue;
        }

        if normalized_line.starts_with('#') {
            let title = normalized_line.trim_start_matches('#').trim();
            if !title.is_empty() {
                titles.insert(normalize_glm_text_fragment(title));
            }
        }
    }

    if inside_centered_block {
        flush_centered_block(&mut titles, centered_block_has_heading, &mut centered_lines);
    }

    titles
}

fn resolve_glm_effective_label(
    raw_label: &str,
    content: &str,
    markdown_titles: &std::collections::HashSet<String>,
) -> String {
    if raw_label == "text" {
        let normalized_content =
            normalize_glm_text_fragment(content.trim_start_matches('#').trim());
        if !normalized_content.is_empty() && markdown_titles.contains(&normalized_content) {
            return "title".to_string();
        }
        if content.trim_start().starts_with('#') {
            return "title".to_string();
        }
    }

    raw_label.to_string()
}

fn page_dimensions_from_glm_response(response: &GlmOcrResponse, page_index: usize) -> (u32, u32) {
    response
        .data_info
        .as_ref()
        .and_then(|info| info.pages.get(page_index))
        .map(|page| (page.width, page.height))
        .unwrap_or((0, 0))
}

fn normalized_bbox_to_pixels(
    detail: &GlmOcrLayoutDetail,
    fallback_width: u32,
    fallback_height: u32,
) -> Option<paddle_vl_types::PaddleVlBbox> {
    if detail.bbox_2d.len() != 4 {
        return None;
    }

    let width = detail.width.unwrap_or(fallback_width);
    let height = detail.height.unwrap_or(fallback_height);
    if width == 0 || height == 0 {
        return None;
    }

    let raw_x1 = detail.bbox_2d[0];
    let raw_y1 = detail.bbox_2d[1];
    let raw_3 = detail.bbox_2d[2];
    let raw_4 = detail.bbox_2d[3];

    let looks_normalized = [raw_x1, raw_y1, raw_3, raw_4]
        .iter()
        .all(|value| *value >= 0.0 && *value <= 1.0);

    let (x1, y1, x2, y2) = if looks_normalized {
        let norm_x1 = raw_x1.clamp(0.0, 1.0);
        let norm_y1 = raw_y1.clamp(0.0, 1.0);
        let norm_3 = raw_3.clamp(0.0, 1.0);
        let norm_4 = raw_4.clamp(0.0, 1.0);

        let x1 = (norm_x1 * width as f32).round() as i32;
        let y1 = (norm_y1 * height as f32).round() as i32;

        if norm_3 > norm_x1 && norm_4 > norm_y1 {
            (
                x1,
                y1,
                (norm_3 * width as f32).round() as i32,
                (norm_4 * height as f32).round() as i32,
            )
        } else {
            (
                x1,
                y1,
                ((norm_x1 + norm_3).clamp(0.0, 1.0) * width as f32).round() as i32,
                ((norm_y1 + norm_4).clamp(0.0, 1.0) * height as f32).round() as i32,
            )
        }
    } else {
        let x1 = raw_x1.round() as i32;
        let y1 = raw_y1.round() as i32;

        if raw_3 > raw_x1 && raw_4 > raw_y1 {
            (x1, y1, raw_3.round() as i32, raw_4.round() as i32)
        } else {
            (
                x1,
                y1,
                (raw_x1 + raw_3).round() as i32,
                (raw_y1 + raw_4).round() as i32,
            )
        }
    };

    Some(paddle_vl_types::PaddleVlBbox {
        x: x1,
        y: y1,
        width: (x2 - x1).max(0),
        height: (y2 - y1).max(0),
    })
}

fn glm_response_has_useful_content(response: &GlmOcrResponse) -> bool {
    if !response.md_results.trim().is_empty() {
        return true;
    }

    response.layout_details.iter().flatten().any(|detail| {
        let label = detail.label.as_deref().unwrap_or_default();
        let content = detail.content.as_deref().unwrap_or_default().trim();
        !content.is_empty() && matches!(label, "text" | "table" | "formula")
    })
}

fn glm_response_to_processed_output(
    response: &GlmOcrResponse,
    method: &str,
) -> Result<ProcessedOcrOutput, String> {
    let mut blocks = Vec::new();
    let mut regions = Vec::new();
    let mut ocr_regions = Vec::new();
    let mut max_width = 0_u32;
    let mut max_height = 0_u32;
    let markdown_titles = collect_glm_markdown_titles(&response.md_results);

    for (page_idx, page_details) in response.layout_details.iter().enumerate() {
        let page =
            u32::try_from(page_idx + 1).map_err(|_| "GLM-OCR page index overflow".to_string())?;
        let (fallback_width, fallback_height) =
            page_dimensions_from_glm_response(response, page_idx);

        for detail in page_details {
            let raw_label = detail.label.as_deref().unwrap_or("text");
            let content = detail.content.clone().unwrap_or_default();
            let trimmed = content.trim();
            let width = detail.width.unwrap_or(fallback_width);
            let height = detail.height.unwrap_or(fallback_height);
            let bbox = normalized_bbox_to_pixels(detail, fallback_width, fallback_height);
            max_width = max_width.max(width);
            max_height = max_height.max(height);
            let effective_label = resolve_glm_effective_label(raw_label, trimmed, &markdown_titles);

            let Some(mapped_category) = glm_label_to_layout_category(&effective_label) else {
                continue;
            };

            if let (Some(formatted_text), Some(ref bbox)) = (
                format_region_text(&mapped_category, &content),
                bbox.as_ref(),
            ) {
                ocr_regions.push(provider::OcrRegion {
                    text: formatted_text,
                    confidence: 1.0,
                    bbox: Some(provider::BoundingBox {
                        x: bbox.x,
                        y: bbox.y,
                        width: bbox.width as u32,
                        height: bbox.height as u32,
                    }),
                    column: None,
                });
            }

            if let Some(bbox) = bbox {
                let order = detail.index.unwrap_or((blocks.len() + 1) as i32);
                regions.push(PersistedLayoutRegion {
                    page,
                    image_width: width,
                    image_height: height,
                    category: effective_label.clone(),
                    bbox: bbox.clone(),
                    confidence: 1.0,
                });
                blocks.push(PersistedLayoutBlock {
                    page,
                    image_width: width,
                    image_height: height,
                    label: effective_label,
                    content: trimmed.to_string(),
                    bbox,
                    order,
                    group_id: page as i32,
                });
            }
        }
    }

    blocks.sort_by_key(|block| (block.page, block.order));

    let text = if !response.md_results.trim().is_empty() {
        response.md_results.trim().to_string()
    } else {
        blocks
            .iter()
            .filter_map(|block| {
                glm_label_to_layout_category(block.label.as_str())
                    .and_then(|category| format_region_text(&category, &block.content))
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    Ok(ProcessedOcrOutput {
        ocr: provider::OcrOutput {
            text,
            regions: ocr_regions,
            method: method.to_string(),
        },
        layout: (!blocks.is_empty()).then_some(LayoutPersistencePayload {
            model: method.to_string(),
            image_width: max_width,
            image_height: max_height,
            regions,
            blocks,
        }),
    })
}

fn get_ocrh_mode(conn: &rusqlite::Connection) -> String {
    crate::settings::get_setting(conn, OCRH_SETTING_MODE)
        .unwrap_or_else(|| OCRH_MODE_LOCAL.to_string())
        .to_lowercase()
}

fn get_glm_ocr_api_key(conn: &rusqlite::Connection) -> String {
    crate::settings::get_setting(conn, OCRH_SETTING_GLM_OCR_API_KEY)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub(super) fn ensure_selected_cloud_key(conn: &rusqlite::Connection) -> Result<(), String> {
    let mode = get_ocrh_mode(conn);
    if mode == OCRH_MODE_GLM_OCR && get_glm_ocr_api_key(conn).is_empty() {
        return Err(
            "GLM-OCR no está configurado. Andá a Configuración > OCRH y cargá una API key antes de usar OCRH."
                .to_string(),
        );
    }

    Ok(())
}

fn encode_bytes_for_glm_ocr(bytes: &[u8]) -> Result<String, String> {
    let mime = if bytes.starts_with(b"%PDF-") {
        "application/pdf"
    } else {
        match image::guess_format(bytes)
            .map_err(|e| format!("No pude detectar el formato de la imagen para GLM-OCR: {e}"))?
        {
            image::ImageFormat::Png => "image/png",
            image::ImageFormat::Jpeg => "image/jpeg",
            other => {
                return Err(format!(
                    "GLM-OCR sólo acepta PDF, PNG o JPG/JPEG. Formato detectado no soportado: {other:?}"
                ))
            }
        }
    };

    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

async fn process_with_glm_ocr_provider(
    bytes: &[u8],
    asset_id: &str,
    app_handle: &AppHandle,
    api_key: &str,
    method: &str,
) -> Result<ProcessedOcrOutput, String> {
    emit_progress(app_handle, asset_id, 55, "submitting_glm_ocr");
    let payload = encode_bytes_for_glm_ocr(bytes)?;
    let client = glm_ocr::GlmOcrClient::new(api_key.to_string());
    emit_progress(app_handle, asset_id, 75, "waiting_glm_ocr");
    let response = client.parse_file(&payload).await?;

    #[cfg(debug_assertions)]
    {
        let _ = debug_viz::save_glm_ocr_response_debug(&response, method, asset_id);
    }

    if !glm_response_has_useful_content(&response) {
        return Err("GLM-OCR devolvió una respuesta vacía para este asset.".to_string());
    }

    emit_progress(app_handle, asset_id, 92, "parsing_glm_ocr");
    glm_response_to_processed_output(&response, method)
}

#[cfg(feature = "paddle-ocr")]
fn detect_image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::load_from_memory(bytes)
        .map(|img| (img.width(), img.height()))
        .ok()
}

#[cfg(feature = "paddle-ocr")]
fn rescale_paddlevl_bbox(bbox: &mut paddle_vl_types::PaddleVlBbox, scale_x: f64, scale_y: f64) {
    bbox.x = ((bbox.x as f64) * scale_x).round() as i32;
    bbox.y = ((bbox.y as f64) * scale_y).round() as i32;
    bbox.width = ((bbox.width as f64) * scale_x).round() as i32;
    bbox.height = ((bbox.height as f64) * scale_y).round() as i32;
}

#[cfg(feature = "paddle-ocr")]
fn rescale_paddlevl_output_to_dimensions(
    output: &mut PaddleVlOutput,
    target_width: u32,
    target_height: u32,
) {
    if output.image_width == 0 || output.image_height == 0 {
        return;
    }

    if output.image_width == target_width && output.image_height == target_height {
        return;
    }

    let scale_x = target_width as f64 / output.image_width as f64;
    let scale_y = target_height as f64 / output.image_height as f64;

    for region in &mut output.regions {
        rescale_paddlevl_bbox(&mut region.bbox, scale_x, scale_y);
    }

    for block in &mut output.blocks {
        rescale_paddlevl_bbox(&mut block.bbox, scale_x, scale_y);
    }

    output.image_width = target_width;
    output.image_height = target_height;
}

// ── Job & Queue ─────────────────────────────────────────────────────────────

/// A single OCR work unit submitted to the background worker.
pub struct OcrJob {
    pub asset_id: String,
    pub asset_path: String,
    pub asset_type: String, // "pdf" | "image"
    pub mode: OcrMode,
}

/// OCR processing mode.
///
/// - `Light`: Plain lightweight PaddleOCR only — no layout detection, no Python subprocess.
/// - `High`: PaddleOCR-VL Python subprocess only. Slower but layout-aware extraction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum OcrMode {
    #[default]
    Light, // Plain lightweight PaddleOCR
    High, // PaddleOCR-VL only
}

/// Handle for submitting jobs to the background OCR worker.
///
/// Managed as Tauri state — the `extract_text` command grabs this via `State<OcrQueue>`.
pub struct OcrQueue {
    sender: mpsc::Sender<OcrJob>,
}

impl OcrQueue {
    /// Create a new queue and return `(OcrQueue, Receiver)`.
    ///
    /// The caller is responsible for passing the receiver to [`start_worker`].
    pub fn new() -> (Self, mpsc::Receiver<OcrJob>) {
        // Bounded channel — 64 pending jobs should be more than enough for a
        // single-user desktop app. `try_send` will fail gracefully if full.
        let (sender, receiver) = mpsc::channel::<OcrJob>(64);
        (Self { sender }, receiver)
    }

    /// Submit a job to the queue. Returns immediately.
    pub fn submit(&self, job: OcrJob) -> Result<(), String> {
        self.sender
            .try_send(job)
            .map_err(|e| format!("Failed to enqueue OCR job: {e}"))
    }

    /// Spawn the background worker loop on the Tokio runtime.
    ///
    /// The worker:
    /// 1. Opens its own SQLite connection for persisting extractions.
    /// 2. Loads the lightweight PaddleOCR provider.
    /// 3. Keeps PaddleVL lazy; OCRH/high OCR resolves it only when requested and falls back to PaddleOCR.
    /// 4. Drains jobs serially from the receiver.
    /// 5. Saves extracted text to DB, then emits events per job.
    pub fn start_worker(
        db_path: std::path::PathBuf,
        mut receiver: mpsc::Receiver<OcrJob>,
        app_handle: AppHandle,
    ) {
        #[cfg(not(feature = "paddle-ocr"))]
        {
            // Lean (--no-default-features) parity with EntropIA-Lite: there is no
            // local PaddleOCR engine in this build, so OCR is routed to the remote
            // GLM-OCR provider — the same always-compiled chain the paddle High
            // path uses. We mirror Lite's run_job/process_job shape: per job open a
            // dedicated SQLite connection, read the GLM-OCR API key, read the file
            // bytes, pick the method by asset type, call the remote provider, then
            // persist + enqueue NLP follow-ups + emit completion. On any failure we
            // emit a single ocr:error and keep draining (no drain-all).
            tauri::async_runtime::spawn(async move {
                eprintln!("[OCR] EntropIA lean OCR worker ready; GLM-OCR remote only");
                crate::app_logs::info(
                    &app_handle,
                    "ocr",
                    "Motor OCR remoto GLM-OCR listo (build lean sin PaddleOCR liviano)",
                );

                while let Some(job) = receiver.recv().await {
                    let asset_id = job.asset_id.clone();
                    // The lean build always routes to remote GLM-OCR; the OCR mode
                    // is kept on the job for contract parity but not honored here
                    // (mirrors EntropIA-Lite's `let _ = &job.mode;`).
                    let _ = &job.mode;

                    // Per-job rusqlite connection (mirrors the paddle worker's
                    // WAL/foreign-keys pragmas). rusqlite connections must not be
                    // shared across tasks, so each job opens its own.
                    let conn = match rusqlite::Connection::open(&db_path) {
                        Ok(c) => {
                            if let Err(e) =
                                c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
                            {
                                eprintln!("[OCR] Failed to configure DB pragmas: {e}");
                            }
                            c
                        }
                        Err(e) => {
                            eprintln!("[OCR] Failed to open worker DB connection: {e}");
                            let _ = app_handle.emit(
                                "ocr:error",
                                OcrErrorPayload {
                                    asset_id,
                                    error: format!("Failed to open OCR DB connection: {e}"),
                                },
                            );
                            continue;
                        }
                    };

                    let api_key = get_glm_ocr_api_key(&conn);
                    if api_key.is_empty() {
                        crate::app_logs::error(
                            &app_handle,
                            "ocr",
                            "OCR remoto no disponible: falta la API key de GLM-OCR",
                        );
                        let _ = app_handle.emit(
                            "ocr:error",
                            OcrErrorPayload {
                                asset_id,
                                error: "GLM-OCR no está configurado. Andá a Configuración > OCRH y cargá una API key antes de usar OCR."
                                    .to_string(),
                            },
                        );
                        continue;
                    }

                    emit_progress(&app_handle, &asset_id, 25, "reading");
                    let bytes = match tokio::fs::read(&job.asset_path).await {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            let error = format!("Failed to read {}: {e}", job.asset_path);
                            crate::app_logs::error(
                                &app_handle,
                                "ocr",
                                format!("OCR falló: asset_id={asset_id}, error={error}"),
                            );
                            let _ = app_handle.emit(
                                "ocr:error",
                                OcrErrorPayload { asset_id, error },
                            );
                            continue;
                        }
                    };

                    let method = if job.asset_type == "pdf" {
                        "pdf_glm_ocr"
                    } else {
                        "glm_ocr"
                    };

                    let result = process_with_glm_ocr_provider(
                        &bytes,
                        &asset_id,
                        &app_handle,
                        &api_key,
                        method,
                    )
                    .await;

                    match result {
                        Ok(output) => {
                            emit_progress(&app_handle, &asset_id, 100, "done");
                            let method = output.ocr.method.clone();
                            let text_content = output.ocr.text.clone();
                            let save_result =
                                save_extraction(&conn, &asset_id, &text_content, &method)
                                    .and_then(|_| match output.layout.as_ref() {
                                        Some(layout) => save_layout(&conn, &asset_id, layout),
                                        None => delete_layout(&conn, &asset_id),
                                    })
                                    .and_then(|_| lookup_item_id_for_asset(&conn, &asset_id));

                            if let Err(e) = &save_result {
                                eprintln!("[ocr] Failed to save extraction for {asset_id}: {e}");
                                crate::app_logs::error(
                                    &app_handle,
                                    "ocr",
                                    format!("No se pudo guardar extracción de {asset_id}: {e}"),
                                );
                            } else if let Ok(Some(item_id)) = &save_result {
                                let nlp_queue = app_handle.state::<NlpQueue>();
                                // FTS indexing: ensures the new text is searchable immediately.
                                if let Err(e) = nlp_queue.submit(NlpJob::IndexFts {
                                    item_id: item_id.clone(),
                                }) {
                                    eprintln!(
                                        "[nlp] Failed to auto-enqueue IndexFts after OCR save: {e}"
                                    );
                                }
                                // Asset-level embedding keeps similarity in sync for the
                                // specific page/audio chunk that changed.
                                if let Err(e) = nlp_queue.submit(NlpJob::ComputeAssetEmbedding {
                                    item_id: item_id.clone(),
                                    asset_id: asset_id.clone(),
                                }) {
                                    eprintln!(
                                        "[nlp] Failed to auto-enqueue ComputeAssetEmbedding after OCR save: {e}"
                                    );
                                }
                            }

                            let _ = app_handle.emit(
                                "ocr:complete",
                                OcrCompletePayload {
                                    asset_id: asset_id.clone(),
                                    method: method.clone(),
                                    text_length: text_content.len(),
                                    text_content,
                                },
                            );
                            crate::app_logs::info(
                                &app_handle,
                                "ocr",
                                format!("OCR completado: asset_id={asset_id}, método={method}"),
                            );
                        }
                        Err(err) => {
                            crate::app_logs::error(
                                &app_handle,
                                "ocr",
                                format!("OCR falló: asset_id={asset_id}, error={err}"),
                            );
                            let _ = app_handle.emit(
                                "ocr:error",
                                OcrErrorPayload {
                                    asset_id,
                                    error: err,
                                },
                            );
                        }
                    }
                }
            });
            return;
        }

        #[cfg(feature = "paddle-ocr")]
        std::thread::Builder::new()
            .name("ocr-worker".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                // Initialize Pdfium native library path resolution once.
                // This caches the DLL search path for all subsequent PDF operations.
                init_pdfium_path(&app_handle);

                // ── Provider initialization: local OCR is Paddle-only ─────────
                let provider: Arc<dyn OcrProvider> = {
                    #[cfg(feature = "paddle-ocr")]
                    {
                        let model_dir = resolve_paddle_model_dir(&app_handle);
                        match paddle::PaddleOcrProvider::new(model_dir) {
                            Ok(p) => Arc::new(p) as Arc<dyn OcrProvider>,
                            Err(e) => {
                                eprintln!("[OCR] 🚨 PaddleOCR unavailable — draining queue with errors: {e}");
                                crate::app_logs::error(
                                    &app_handle,
                                    "ocr",
                                    format!("PaddleOCR liviano no está disponible: {e}"),
                                );
                                while let Some(job) = receiver.blocking_recv() {
                                    let _ = app_handle.emit(
                                        "ocr:error",
                                        OcrErrorPayload {
                                            asset_id: job.asset_id,
                                            error: format!(
                                                "OCR local no disponible: PaddleOCR liviano no pudo inicializarse ({e})"
                                            ),
                                        },
                                    );
                                }
                                return;
                            }
                        }
                    }

                    #[cfg(not(feature = "paddle-ocr"))]
                    {
                        eprintln!("[OCR] 🚨 PaddleOCR feature disabled — draining queue with errors");
                        crate::app_logs::error(
                            &app_handle,
                            "ocr",
                            "OCR local no disponible: binario compilado sin feature paddle-ocr",
                        );
                        while let Some(job) = receiver.blocking_recv() {
                            let _ = app_handle.emit(
                                "ocr:error",
                                OcrErrorPayload {
                                    asset_id: job.asset_id,
                                    error: "OCR local no disponible: EntropIA fue compilado sin PaddleOCR liviano".to_string(),
                                },
                            );
                        }
                        return;
                    }
                };

                eprintln!("[OCR] Provider ready: {}", provider.name());
                crate::app_logs::info(
                    &app_handle,
                    "ocr",
                    format!("Motor OCR listo: {}", provider.name()),
                );

                let mut paddle_vl_engine: Option<PaddleVlEngine> = None;
                eprintln!("[OCR] High OCR mode is lazy; PaddleOCR-VL will initialize on OCRH jobs");

                // Dedicated DB connection for this worker (avoids open/close per job).
                let conn = match rusqlite::Connection::open(&db_path) {
                    Ok(c) => {
                        if let Err(e) =
                            c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
                        {
                            eprintln!("[OCR] Failed to configure DB pragmas: {e}");
                        }
                        c
                    }
                    Err(e) => {
                        eprintln!("[OCR] Failed to open worker DB connection: {e}");
                        crate::app_logs::error(
                            &app_handle,
                            "ocr",
                            format!("No se pudo abrir conexión DB del worker OCR: {e}"),
                        );
                        while let Some(job) = receiver.blocking_recv() {
                            let _ = app_handle.emit(
                                "ocr:error",
                                OcrErrorPayload {
                                    asset_id: job.asset_id,
                                    error: format!("Failed to open OCR DB connection: {e}"),
                                },
                            );
                        }
                        return;
                    }
                };

                while let Some(job) = receiver.blocking_recv() {
                    let asset_id = job.asset_id.clone();
                    if job.mode == OcrMode::High && paddle_vl_engine.is_none() {
                        eprintln!("[OCRH] Re-probing PaddleOCR-VL before high OCR job {asset_id}");
                        crate::app_logs::info(
                            &app_handle,
                            "ocrh",
                            format!("Re-probando PaddleOCR-VL para asset {asset_id}"),
                        );
                        crate::python_discovery::invalidate_probe_cache_entry("paddle_vl");
                        match create_paddle_vl_engine_result(&app_handle, &db_path) {
                            Ok(engine) => {
                                eprintln!("[OCRH] PaddleOCR-VL became available after re-probe");
                                crate::app_logs::info(
                                    &app_handle,
                                    "ocrh",
                                    "PaddleOCR-VL quedó disponible después de re-probar",
                                );
                                paddle_vl_engine = Some(engine);
                            }
                            Err(error) => {
                                eprintln!("[OCRH] Local PaddleOCR-VL still unavailable after re-probe: {error}");
                                crate::app_logs::warn(
                                    &app_handle,
                                    "ocrh",
                                    format!("PaddleOCR-VL local sigue no disponible: {error}"),
                                );
                            }
                        }
                    }
                    let result = tauri::async_runtime::block_on(process_job(
                        &provider,
                        &conn,
                        &job,
                        &app_handle,
                        paddle_vl_engine.as_ref(),
                    ));

                    match result {
                        Ok(output) => {
                            let method = output.ocr.method.clone();
                            let text_content = output.ocr.text.clone();
                            let save_result = save_extraction(&conn, &asset_id, &text_content, &method)
                                .and_then(|_| match output.layout.as_ref() {
                                    Some(layout) => save_layout(&conn, &asset_id, layout),
                                    None => delete_layout(&conn, &asset_id),
                                })
                                .and_then(|_| lookup_item_id_for_asset(&conn, &asset_id));

                            if let Err(e) = &save_result {
                                eprintln!("[ocr] Failed to save extraction for {asset_id}: {e}");
                                crate::app_logs::error(
                                    &app_handle,
                                    "ocr",
                                    format!("No se pudo guardar extracción de {asset_id}: {e}"),
                                );
                            } else if let Ok(Some(item_id)) = &save_result {
                                let nlp_queue = app_handle.state::<NlpQueue>();
                                if let Err(e) = nlp_queue.submit(NlpJob::ExtractEntitiesForAsset {
                                    item_id: item_id.clone(),
                                    asset_id: asset_id.clone(),
                                }) {
                                    eprintln!(
                                        "[nlp] Failed to auto-enqueue ExtractEntitiesForAsset after OCR save: {e}"
                                    );
                                } else {
                                    eprintln!(
                                        "[nlp] Auto-enqueued ExtractEntitiesForAsset after OCR save: asset_id={}, item_id={}",
                                        asset_id, item_id
                                    );
                                }
                                // FTS indexing: ensures the new text is searchable immediately.
                                if let Err(e) = nlp_queue.submit(NlpJob::IndexFts {
                                    item_id: item_id.clone(),
                                }) {
                                    eprintln!(
                                        "[nlp] Failed to auto-enqueue IndexFts after OCR save: {e}"
                                    );
                                } else {
                                    eprintln!(
                                        "[nlp] Auto-enqueued IndexFts after OCR save: item_id={}",
                                        item_id
                                    );
                                }
                                // Asset-level embedding keeps similarity in sync for the
                                // specific page/audio chunk that changed.
                                if let Err(e) = nlp_queue.submit(NlpJob::ComputeAssetEmbedding {
                                    item_id: item_id.clone(),
                                    asset_id: asset_id.clone(),
                                }) {
                                    eprintln!(
                                        "[nlp] Failed to auto-enqueue ComputeAssetEmbedding after OCR save: {e}"
                                    );
                                } else {
                                    eprintln!(
                                        "[nlp] Auto-enqueued ComputeAssetEmbedding after OCR save: asset_id={}, item_id={}",
                                        asset_id, item_id
                                    );
                                }
                            }

                            let _ = app_handle.emit(
                                "ocr:complete",
                                OcrCompletePayload {
                                    asset_id: asset_id.clone(),
                                    method: method.clone(),
                                    text_length: text_content.len(),
                                    text_content,
                                },
                            );
                            crate::app_logs::info(
                                &app_handle,
                                "ocr",
                                format!("OCR completado: asset_id={asset_id}, método={method}"),
                            );
                        }
                        Err(err) => {
                            crate::app_logs::error(
                                &app_handle,
                                "ocr",
                                format!("OCR falló: asset_id={asset_id}, error={err}"),
                            );
                            let _ = app_handle.emit(
                                "ocr:error",
                                OcrErrorPayload {
                                    asset_id,
                                    error: err,
                                },
                            );
                        }
                    }
                }
            })
            .expect("Failed to spawn OCR worker thread");
    }
}

// ── Model directory resolution ──────────────────────────────────────────────

/// Resolve the PaddleOCR model directory.
///
/// In production (bundled Tauri app), uses `BaseDirectory::Resource`.
/// In dev mode, falls back to `CARGO_MANIFEST_DIR` so models can be loaded
/// from the project's `resources/models/ocr/` directory.
#[cfg(feature = "paddle-ocr")]
fn resolve_paddle_model_dir(app_handle: &AppHandle) -> std::path::PathBuf {
    let runtime_root = managed_runtime_root_for_ocr(app_handle).ok().flatten();
    resolve_paddle_model_dir_from_roots(
        runtime_root.as_deref(),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}

#[cfg(feature = "paddle-ocr")]
fn resolve_paddle_model_dir_from_roots(
    managed_root: Option<&std::path::Path>,
    manifest_dir: &std::path::Path,
) -> std::path::PathBuf {
    if let Some(root) = managed_root {
        let managed = managed_resource_path(root, "models/ocr");
        if managed.exists() {
            return managed;
        }
    }

    let dev_path = manifest_dir.join("resources").join("models").join("ocr");
    if dev_path.exists() {
        return dev_path;
    }

    std::path::PathBuf::from("resources/models/ocr")
}

// ── Persistence ─────────────────────────────────────────────────────────────

/// Upsert an extraction row for the given asset_id.
///
/// Uses SQLite `ON CONFLICT(asset_id) DO UPDATE` semantics.
fn save_extraction(
    conn: &rusqlite::Connection,
    asset_id: &str,
    text_content: &str,
    method: &str,
) -> Result<(), String> {
    // True UPSERT keyed by `asset_id` (requires UNIQUE index on extractions.asset_id).
    // This avoids DELETE+INSERT churn and keeps writes atomic.
    let id = uuid::Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    conn.execute(
        "INSERT INTO extractions(id, asset_id, text_content, method, confidence, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(asset_id) DO UPDATE SET
           text_content = excluded.text_content,
           method = excluded.method,
           confidence = excluded.confidence,
           created_at = excluded.created_at",
        rusqlite::params![id, asset_id, text_content, method, None::<f64>, now],
    )
    .map_err(|e| format!("Failed to upsert extraction: {e}"))?;

    Ok(())
}

fn save_layout(
    conn: &rusqlite::Connection,
    asset_id: &str,
    layout: &LayoutPersistencePayload,
) -> Result<(), String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let regions_json = serde_json::to_string(&layout.regions)
        .map_err(|e| format!("Failed to serialize layout regions: {e}"))?;
    let blocks_json = serde_json::to_string(&layout.blocks)
        .map_err(|e| format!("Failed to serialize layout blocks: {e}"))?;

    conn.execute(
        "INSERT INTO layouts(id, asset_id, regions, blocks, model, image_width, image_height, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(asset_id) DO UPDATE SET
           regions = excluded.regions,
           blocks = excluded.blocks,
           model = excluded.model,
           image_width = excluded.image_width,
           image_height = excluded.image_height,
           created_at = excluded.created_at",
        rusqlite::params![
            id,
            asset_id,
            regions_json,
            blocks_json,
            layout.model,
            layout.image_width,
            layout.image_height,
            now,
        ],
    )
    .map_err(|e| format!("Failed to upsert layout: {e}"))?;

    Ok(())
}

fn delete_layout(conn: &rusqlite::Connection, asset_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM layouts WHERE asset_id = ?1",
        rusqlite::params![asset_id],
    )
    .map_err(|e| format!("Failed to delete stale layout: {e}"))?;
    Ok(())
}

/// Update only the text_content of the latest extraction for an asset.
/// Preserves id, created_at, method, and confidence.
/// Returns `Ok(())` even if no extraction exists (no-op).
fn update_extraction_text(
    conn: &rusqlite::Connection,
    asset_id: &str,
    text_content: &str,
) -> Result<(), String> {
    // Find the latest extraction for this asset
    let mut stmt = conn
        .prepare("SELECT id FROM extractions WHERE asset_id = ?1 ORDER BY created_at DESC LIMIT 1")
        .map_err(|e| format!("Failed to prepare query: {e}"))?;

    let extraction_id: Result<String, _> = stmt.query_row([asset_id], |row| row.get(0));

    drop(stmt); // release borrow before execute

    match extraction_id {
        Ok(id) => {
            conn.execute(
                "UPDATE extractions SET text_content = ?1 WHERE id = ?2",
                rusqlite::params![text_content, id],
            )
            .map_err(|e| format!("Failed to update extraction text: {e}"))?;
            Ok(())
        }
        Err(_) => Ok(()), // no extraction exists — no-op
    }
}

// ── Layout-Aware Text Formatting ──────────────────────────────────────────

/// Format text based on the layout category of the region.
///
/// Returns `None` for categories that should be skipped (Figure, Header, Footer, Abandoned).
/// Returns `Some(formatted_text)` for categories that contribute to the output.
///
/// NOTE: Currently unused in production code (layout-aware pipeline removed from Light mode).
/// Kept for potential future use and tested in unit tests.
#[allow(dead_code)]
fn format_region_text(category: &LayoutCategory, text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    match category {
        LayoutCategory::Title => Some(format!("## {trimmed}")),
        LayoutCategory::PlainText => Some(trimmed.to_string()),
        LayoutCategory::Table => Some(format!("---\n{trimmed}\n---")),
        LayoutCategory::Figure => None, // Skip figures in text output
        LayoutCategory::Caption => Some(trimmed.to_string()),
        LayoutCategory::Footnote => Some(format!("Note: {trimmed}")),
        LayoutCategory::Header => None, // Skip headers (typically noise)
        LayoutCategory::Footer => None, // Skip footers (page numbers, etc.)
        LayoutCategory::Code => Some(format!("```\n{trimmed}\n```")),
        LayoutCategory::Reference => Some(trimmed.to_string()),
        LayoutCategory::Abandoned => None, // Skip abandoned content
    }
}

/// Maximum image dimension (longest side, in pixels) to feed into PaddleVL.
///
/// PaddleOCR-VL is a vision-language model that runs a VLM on the full image.
/// Inference time scales roughly with pixel count — on CPU, a 2200×2575 image
/// (5.67 MP) takes 15+ minutes, while a 1500×1756 image (2.63 MP) takes ~4 min.
///
/// We downscale images larger than this threshold (in either dimension) before
/// passing to PaddleVL. The aspect ratio is preserved. At 1000px longest side,
/// OCR accuracy on typical document images (scanned newspapers, book pages,
/// forms) may start to degrade for very small fonts — monitor results and
/// bump this up to 1500-2000 if needed.
#[cfg(feature = "paddle-ocr")]
const PADDLE_VL_MAX_DIMENSION: u32 = 1000;

/// Maximum total pixel count before triggering downscale.
///
/// Belt-and-suspenders check alongside PADDLE_VL_MAX_DIMENSION. Consistent with
/// MAX_DIMENSION=1000: a 1000×1000 square image is exactly at the limit.
/// Anything larger by area (e.g. 1100×1100 = 1.21 MP) triggers downscale.
/// We trigger downscale if EITHER condition is met.
#[cfg(feature = "paddle-ocr")]
const PADDLE_VL_MAX_PIXELS: u32 = 1_000_000; // 1 megapixel

/// Downscale an image if it exceeds PaddleVL's comfort zone.
///
/// Returns the (possibly reduced) image bytes as PNG. If no downscale is
/// needed (image fits comfortably within PADDLE_VL_MAX_DIMENSION and
/// PADDLE_VL_MAX_PIXELS), returns the original bytes verbatim.
///
/// On decode or re-encode failure, returns the original bytes — we never
/// want to block OCR because of a resize issue.
#[cfg(feature = "paddle-ocr")]
fn maybe_downscale_for_paddlevl(bytes: &[u8]) -> Vec<u8> {
    let img = match image::load_from_memory(bytes) {
        Ok(img) => img,
        Err(e) => {
            eprintln!(
                "[OCRH] Could not decode image for downscale check: {e}. Using original bytes."
            );
            return bytes.to_vec();
        }
    };

    let (w, h) = (img.width(), img.height());
    let total_pixels = (w as u64) * (h as u64);
    let max_dim = w.max(h);

    let exceeds_dim = max_dim > PADDLE_VL_MAX_DIMENSION;
    let exceeds_pixels = total_pixels > PADDLE_VL_MAX_PIXELS as u64;

    if !exceeds_dim && !exceeds_pixels {
        eprintln!(
            "[OCRH] Image size {}x{} ({:.2} MP) OK, no downscale needed",
            w,
            h,
            total_pixels as f64 / 1_000_000.0
        );
        return bytes.to_vec();
    }

    // Compute target size: scale the longest side down to PADDLE_VL_MAX_DIMENSION
    // while preserving aspect ratio. This also addresses the PADDLE_VL_MAX_PIXELS
    // case because reducing the longest side also reduces total pixels.
    let scale = PADDLE_VL_MAX_DIMENSION as f32 / max_dim as f32;
    let new_w = ((w as f32) * scale).round().max(1.0) as u32;
    let new_h = ((h as f32) * scale).round().max(1.0) as u32;

    eprintln!(
        "[OCRH] Downscaling {}x{} ({:.2} MP) → {}x{} ({:.2} MP) for PaddleVL",
        w,
        h,
        total_pixels as f64 / 1_000_000.0,
        new_w,
        new_h,
        (new_w as u64 * new_h as u64) as f64 / 1_000_000.0
    );

    // Triangle filter = good balance of quality vs speed for document images
    let resized = img.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle);

    // Re-encode as PNG (lossless, preserves text sharpness)
    let mut out = Vec::with_capacity(bytes.len() / 2);
    if let Err(e) = resized.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png) {
        eprintln!("[OCRH] Re-encode failed: {e}. Using original bytes.");
        return bytes.to_vec();
    }

    out
}

/// Decide whether the layout detection result is trustworthy, or whether we
/// should bypass per-region OCR and use the provider directly on the full image.
///
/// Heuristics (any triggers bypass):
///   1. Zero regions detected — nothing to work with
///   2. Only 1 region detected — layout model likely failed to segment the page;
///      a single "region" spanning most of the image is basically "OCR the whole
///      thing" anyway, but per-region cropping often loses text at the edges.
///   3. A single region covers > 60% of the image area — the layout model treated
///      the whole page as one block, which means we lose nothing by bypassing it
///      and OCR'ing the full image directly (and we gain the provider's internal
///      post-processing that we'd otherwise miss).
///
/// When bypassed, the caller falls through to `provider.recognize(full_image)`
/// which typically produces much better results on newspaper clippings, forms,
/// and other documents where PP-DocLayout-L gives sparse/wrong regions.
///
/// NOTE: Currently unused in production code (layout-aware pipeline removed from Light mode).
/// Kept for potential future use and tested in unit tests.
#[allow(dead_code)]
fn should_bypass_layout(
    regions: &[provider::LayoutRegion],
    image_width: u32,
    image_height: u32,
) -> bool {
    if regions.len() < 2 {
        if regions.is_empty() {
            eprintln!("[OCRL] Bypassing layout: 0 regions detected");
        } else {
            eprintln!(
                "[OCRL] Bypassing layout: only 1 region detected (insufficient segmentation)"
            );
        }
        return true;
    }

    // Check if any single region dominates the image (> 60% of total area)
    let total_area = (image_width as u64) * (image_height as u64);
    if total_area == 0 {
        return false;
    }
    for region in regions {
        let region_area = (region.bbox.width as u64) * (region.bbox.height as u64);
        let ratio = (region_area as f64) / (total_area as f64);
        if ratio > 0.60 {
            eprintln!(
                "[OCRL] Bypassing layout: region {:?} covers {:.1}% of the image (>60% threshold)",
                region.label,
                ratio * 100.0
            );
            return true;
        }
    }

    false
}

/// Crops a layout-detected region from the image with a fixed 5px margin
/// on all sides (top, bottom, left, right).
///
/// The layout model (PP-DocLayout-L) produces tight bounding boxes that
/// closely match text boundaries, so minimal margin is needed.
///
/// Clamps to image bounds. Returns `None` if the cropped region is too small
/// (< 10px on either axis) — anything smaller cannot produce useful OCR.
///
/// NOTE: Currently unused in production code (layout-aware pipeline removed from Light mode).
/// Kept for potential future use and tested in unit tests.
#[allow(dead_code)]
fn crop_region(
    image: &image::DynamicImage,
    bbox: &provider::BoundingBox,
) -> Option<image::DynamicImage> {
    let (img_w, img_h) = (image.width() as i32, image.height() as i32);
    const MARGIN: i32 = 5;

    let x1 = (bbox.x - MARGIN).max(0);
    let y1 = (bbox.y - MARGIN).max(0);
    let x2 = (bbox.x + bbox.width as i32 + MARGIN).min(img_w);
    let y2 = (bbox.y + bbox.height as i32 + MARGIN).min(img_h);

    let crop_w = (x2 - x1) as u32;
    let crop_h = (y2 - y1) as u32;

    // Skip regions that are too small — cannot produce useful OCR.
    if crop_w < 10 || crop_h < 10 {
        eprintln!(
            "[ocr] Skipping region too small: {}x{} at ({},{})",
            crop_w, crop_h, x1, y1
        );
        return None;
    }

    Some(image.crop_imm(x1 as u32, y1 as u32, crop_w, crop_h))
}

// ── Job Processing ──────────────────────────────────────────────────────────

/// Process a single OCR job using any OcrProvider.
///
/// Returns OCR text plus optional layout persistence payload.
///
/// Layout engine parameter removed — layout-aware Light mode is not used in
/// production. PaddleVL handles layout in High mode.
#[cfg(feature = "paddle-ocr")]
async fn process_job(
    provider: &Arc<dyn OcrProvider>,
    conn: &rusqlite::Connection,
    job: &OcrJob,
    app_handle: &AppHandle,
    paddle_vl_engine: Option<&PaddleVlEngine>,
) -> Result<ProcessedOcrOutput, String> {
    let asset_id = job.asset_id.clone();

    // Stage 1 — reading file (25 %)
    emit_progress(app_handle, &asset_id, 25, "reading");

    let file_bytes = tokio::fs::read(&job.asset_path)
        .await
        .map_err(|e| format!("Failed to read {}: {e}", job.asset_path))?;

    match job.asset_type.as_str() {
        "pdf" => {
            process_pdf(
                provider,
                conn,
                &file_bytes,
                &asset_id,
                app_handle,
                paddle_vl_engine,
                &job.mode,
            )
            .await
        }
        _ => {
            process_image(
                provider,
                conn,
                &file_bytes,
                &asset_id,
                app_handle,
                paddle_vl_engine,
                &job.mode,
            )
            .await
        }
    }
}

/// PDF pipeline: try native text first, fall back to page-by-page OCR.
///
/// For text-based PDFs, the native text layer is extracted and quality-checked.
/// If it's insufficient (scanned PDFs, images), every page is rendered and OCR'd,
/// then the results are concatenated with page separators.
#[cfg(feature = "paddle-ocr")]
async fn process_pdf(
    provider: &Arc<dyn OcrProvider>,
    conn: &rusqlite::Connection,
    bytes: &[u8],
    asset_id: &str,
    app_handle: &AppHandle,
    paddle_vl_engine: Option<&PaddleVlEngine>,
    mode: &OcrMode,
) -> Result<ProcessedOcrOutput, String> {
    if mode == &OcrMode::High {
        let ocrh_mode = get_ocrh_mode(conn);
        let glm_ocr_api_key = get_glm_ocr_api_key(conn);

        match ocrh_mode.as_str() {
            OCRH_MODE_GLM_OCR => {
                return process_with_glm_ocr_provider(
                    bytes,
                    asset_id,
                    app_handle,
                    &glm_ocr_api_key,
                    "pdf_glm_ocr",
                )
                .await;
            }
            OCRH_MODE_AUTO if !glm_ocr_api_key.is_empty() => {
                match process_with_glm_ocr_provider(
                    bytes,
                    asset_id,
                    app_handle,
                    &glm_ocr_api_key,
                    "pdf_glm_ocr",
                )
                .await
                {
                    Ok(result) => return Ok(result),
                    Err(error) => {
                        eprintln!("[OCRH] GLM-OCR failed in auto mode for {asset_id}, falling back to local OCRH: {error}");
                    }
                }
            }
            _ => {}
        }
    }

    // Stage 2 — extracting native text (50 %)
    emit_progress(app_handle, asset_id, 50, "extracting_native");

    let bytes_owned = bytes.to_vec();
    let native_text = tokio::task::spawn_blocking(move || extract_pdf_text(&bytes_owned))
        .await
        .map_err(|e| format!("PDF extraction task panicked: {e}"))?;

    match native_text {
        Ok(text) if is_quality_text(&text) => {
            emit_progress(app_handle, asset_id, 100, "done");
            Ok(ProcessedOcrOutput {
                ocr: provider::OcrOutput {
                    text: text.clone(),
                    regions: vec![provider::OcrRegion {
                        text,
                        confidence: 0.0,
                        bbox: None,
                        column: None,
                    }],
                    method: "native".to_string(),
                },
                layout: None,
            })
        }
        _ => {
            // Native text failed quality check — render ALL pages and OCR them.
            eprintln!(
                "[pdf] Native text failed quality check, falling back to multi-page PDF→image→OCR"
            );

            // Get page count in a blocking task (pdfium interaction)
            let pdf_bytes_for_count = bytes.to_vec();
            let page_count =
                tokio::task::spawn_blocking(move || pdf_page_count(&pdf_bytes_for_count))
                    .await
                    .map_err(|e| format!("PDF page count task panicked: {e}"))?
                    .map_err(|e| format!("Failed to get PDF page count: {e}"))?;

            eprintln!("[pdf] Processing {page_count} page(s) via OCR fallback");

            let mut all_text = String::new();
            let mut all_regions: Vec<provider::OcrRegion> = Vec::new();
            let mut layout_payload: Option<LayoutPersistencePayload> = None;
            let mut method_suffix = String::new();

            for page_idx in 0..page_count {
                // Progress: 60% base + (page_idx / page_count) * 35% range
                let pct = 60 + ((page_idx as u8 * 35) / page_count.max(1) as u8);
                emit_progress(
                    app_handle,
                    asset_id,
                    pct.min(95),
                    &format!("ocr_page_{}", page_idx + 1),
                );

                // Render this page
                let pdf_bytes_for_render = bytes.to_vec();
                let page_image = tokio::task::spawn_blocking(move || {
                    pdf::render_pdf_page_to_image(&pdf_bytes_for_render, page_idx)
                })
                .await
                .map_err(|e| format!("PDF render task panicked: {e}"))?
                .map_err(|e| format!("PDF page {} rendering failed: {e}", page_idx + 1))?;

                // OCR this page — mode-aware pipeline
                let provider_clone = Arc::clone(provider);
                let engine_clone = paddle_vl_engine.cloned();
                let mode_clone = mode.clone();
                let original_page_dimensions = detect_image_dimensions(&page_image);
                let app_handle_for_page = app_handle.clone();

                let output = tokio::task::spawn_blocking(move || {
                    match mode_clone {
                        OcrMode::Light => {
                            // Light mode: lightweight PaddleOCR, no layout detection
                            provider_clone
                                .recognize(&page_image)
                                .map(|output| (output, None))
                                .map_err(|e| format!("OCR page {} failed: {e}", page_idx + 1))
                        }
                        OcrMode::High => {
                            // High mode: try PaddleVL, fall back to lightweight PaddleOCR
                            if let Some(engine) = engine_clone {
                                // Downscale large PDF page renders before PaddleVL (same reason as images)
                                let vl_bytes = maybe_downscale_for_paddlevl(&page_image);

                                let temp_path = std::env::temp_dir().join(format!(
                                    "entropia_paddlevl_pdf_{}_{}.png",
                                    page_idx,
                                    uuid::Uuid::new_v4()
                                ));

                                if let Err(e) = std::fs::write(&temp_path, &vl_bytes) {
                                     eprintln!("[OCRH] Failed to write temp file for PaddleVL on PDF page {}: {e}. Falling back to PaddleOCR light.", page_idx + 1);
                                     return provider_clone
                                         .recognize(&page_image)
                                         .map(|output| (output, None))
                                         .map_err(|e| format!("OCR page {} failed: {e}", page_idx + 1));
                                 }

                                let temp_path_str = match temp_path.to_str() {
                                    Some(s) => s.to_string(),
                                    None => {
                                         eprintln!("[OCRH] Invalid temp path for PaddleVL on PDF page {}. Falling back.", page_idx + 1);
                                         return provider_clone
                                             .recognize(&page_image)
                                             .map(|output| (output, None))
                                             .map_err(|e| format!("OCR page {} failed: {e}", page_idx + 1));
                                     }
                                 };

                                let vl_result = engine.detect(&temp_path_str);
                                let _ = std::fs::remove_file(&temp_path);

                                match vl_result {
                                    Ok(mut vl_result) => {
                                        if let Some((original_width, original_height)) = original_page_dimensions {
                                            rescale_paddlevl_output_to_dimensions(
                                                &mut vl_result,
                                                original_width,
                                                original_height,
                                            );
                                        }
                                        Ok((
                                            ocr_output_from_paddlevl(&vl_result),
                                            Some(vl_result),
                                        ))
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "[OCRH] PaddleVL failed for PDF page {}: {e}. Falling back to PaddleOCR light.",
                                            page_idx + 1
                                        );
                                        crate::app_logs::warn(
                                            &app_handle_for_page,
                                            "ocrh",
                                            format!(
                                                "PaddleOCR-VL falló/agotó timeout en PDF página {}; usando PaddleOCR liviano: {e}",
                                                page_idx + 1
                                            ),
                                        );
                                        provider_clone
                                            .recognize(&page_image)
                                            .map(|output| (output, None))
                                            .map_err(|e| format!("OCR page {} failed: {e}", page_idx + 1))
                                    }
                                }
                            } else {
                                eprintln!(
                                    "[OCRH] PaddleVL engine unavailable for PDF page {}. Falling back to PaddleOCR light.",
                                    page_idx + 1
                                );
                                crate::app_logs::warn(
                                    &app_handle_for_page,
                                    "ocrh",
                                    format!(
                                        "PaddleOCR-VL no disponible en PDF página {}; usando PaddleOCR liviano",
                                        page_idx + 1
                                    ),
                                );
                                provider_clone
                                    .recognize(&page_image)
                                    .map(|output| (output, None))
                                    .map_err(|e| format!("OCR page {} failed: {e}", page_idx + 1))
                            }
                        }
                    }
                })
                .await
                .map_err(|e| format!("OCR page {} task panicked: {e}", page_idx + 1))??;

                // Track method for reporting
                let (output, page_layout) = output;

                if let Some(vl_output) = page_layout {
                    let page_num = u32::try_from(page_idx + 1).map_err(|_| {
                        format!("PDF page index {} does not fit into u32", page_idx + 1)
                    })?;
                    if let Some(layout) = layout_payload.as_mut() {
                        layout.push_page(page_num, &vl_output);
                    } else {
                        layout_payload =
                            Some(LayoutPersistencePayload::from_page(page_num, &vl_output));
                    }
                }

                if method_suffix.is_empty() {
                    method_suffix = format!("pdf_{}", output.method);
                }

                // Accumulate results with page separators
                if !all_text.is_empty() {
                    all_text.push_str("\n\n---\n\n"); // Page separator
                }
                all_text.push_str(&output.text);
                all_regions.extend(output.regions);
            }

            let method = if !all_text.is_empty() {
                method_suffix
            } else {
                "pdf_unknown".to_string()
            };

            emit_progress(app_handle, asset_id, 100, "done");
            Ok(ProcessedOcrOutput {
                ocr: provider::OcrOutput {
                    text: all_text,
                    regions: all_regions,
                    method,
                },
                layout: layout_payload,
            })
        }
    }
}

/// Image pipeline: mode-aware OCR with progressive fallback.
///
/// **Light mode** (OCRL): Plain lightweight PaddleOCR on the full image. No layout detection.
/// Fast and simple.
///
/// **High mode** (OCRH): PaddleVL Python subprocess with 900s timeout, then fallback to PaddleOCR.
#[cfg(feature = "paddle-ocr")]
async fn process_image(
    provider: &Arc<dyn OcrProvider>,
    conn: &rusqlite::Connection,
    bytes: &[u8],
    asset_id: &str,
    app_handle: &AppHandle,
    paddle_vl_engine: Option<&PaddleVlEngine>,
    mode: &OcrMode,
) -> Result<ProcessedOcrOutput, String> {
    match mode {
        OcrMode::Light => process_image_light(provider, bytes, asset_id, app_handle).await,
        OcrMode::High => {
            process_image_high(
                provider,
                conn,
                bytes,
                asset_id,
                app_handle,
                paddle_vl_engine,
            )
            .await
        }
    }
}

/// Light mode: plain lightweight PaddleOCR — no layout detection.
///
/// Runs the provider's `recognize()` directly on the full image. Fast and simple.
/// Layout-aware processing is available via High mode (PaddleVL).
#[cfg(feature = "paddle-ocr")]
async fn process_image_light(
    provider: &Arc<dyn OcrProvider>,
    bytes: &[u8],
    asset_id: &str,
    app_handle: &AppHandle,
) -> Result<ProcessedOcrOutput, String> {
    emit_progress(app_handle, asset_id, 50, "ocr_inference");

    // Plain OCR — no layout detection, just run the provider on the full image
    let provider_clone = Arc::clone(provider);
    let bytes_owned = bytes.to_vec();

    let mut output = tokio::task::spawn_blocking(move || provider_clone.recognize(&bytes_owned))
        .await
        .map_err(|e| format!("OCR task panicked: {e}"))?
        .map_err(|e| format!("OCR inference failed: {e}"))?;

    // Reorder regions by reading order (columns left-to-right, top-to-bottom)
    // This matches the algorithm in orden_lectura.py
    if output.regions.len() >= 2 && output.regions.iter().any(|r| r.bbox.is_some()) {
        // Decode image to get dimensions for reading order computation
        if let Ok(img) = image::load_from_memory(bytes) {
            let (img_w, img_h) = (img.width(), img.height());
            output.regions = reading_order::reorder_ocr_regions(&output.regions, img_w, img_h);
            // Rebuild text from reordered regions
            output.text = output
                .regions
                .iter()
                .map(|r| r.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
        }
        // If image decode fails, keep original order — don't fail OCR for this
    }

    // Dev-only: save debug visualization of detected OCR lines
    #[cfg(debug_assertions)]
    {
        if !output.regions.is_empty() {
            let method = output.method.clone();
            let regions_debug = output.regions.clone();
            let bytes_debug = bytes.to_vec();
            let aid = asset_id.to_string();
            // Best-effort — don't fail OCR if debug viz fails
            let _ = debug_viz::save_ocr_lines_debug(&bytes_debug, &regions_debug, &method, &aid);
        }
    }

    emit_progress(app_handle, asset_id, 100, "done");
    Ok(ProcessedOcrOutput {
        ocr: output,
        layout: None,
    })
}

/// High mode: PaddleVL Python subprocess first, lightweight PaddleOCR fallback.
///
/// Runs PaddleOCR-VL (layout + OCR in one pass) via Python subprocess.
/// Falls back to PaddleOCR light if PaddleVL is unavailable, fails, or times out.
#[cfg(feature = "paddle-ocr")]
async fn process_image_high(
    provider: &Arc<dyn OcrProvider>,
    conn: &rusqlite::Connection,
    bytes: &[u8],
    asset_id: &str,
    app_handle: &AppHandle,
    paddle_vl_engine: Option<&PaddleVlEngine>,
) -> Result<ProcessedOcrOutput, String> {
    let ocrh_mode = get_ocrh_mode(conn);
    let glm_ocr_api_key = get_glm_ocr_api_key(conn);

    match ocrh_mode.as_str() {
        OCRH_MODE_GLM_OCR => {
            return process_with_glm_ocr_provider(
                bytes,
                asset_id,
                app_handle,
                &glm_ocr_api_key,
                "glm_ocr",
            )
            .await;
        }
        OCRH_MODE_AUTO if !glm_ocr_api_key.is_empty() => {
            match process_with_glm_ocr_provider(
                bytes,
                asset_id,
                app_handle,
                &glm_ocr_api_key,
                "glm_ocr",
            )
            .await
            {
                Ok(result) => return Ok(result),
                Err(error) => {
                    eprintln!("[OCRH] GLM-OCR failed in auto mode for {asset_id}, falling back to local OCRH: {error}");
                }
            }
        }
        _ => {}
    }

    emit_progress(app_handle, asset_id, 50, "ocr_inference");

    // Try PaddleVL (Python subprocess) if available
    if let Some(engine) = paddle_vl_engine {
        emit_progress(app_handle, asset_id, 55, "paddlevl_detection");

        let engine_clone = engine.clone();
        let provider_clone = Arc::clone(provider);
        let bytes_owned = bytes.to_vec();
        let asset_id_owned = asset_id.to_string();
        let original_image_dimensions = detect_image_dimensions(bytes);
        let app_handle_for_high = app_handle.clone();

        let output = tokio::task::spawn_blocking(move || {
            // Downscale large images before feeding to PaddleVL — inference time
            // scales with pixel count, and CPUs can take 10x longer on 2200x2575
            // images vs 1500x1756. The resized image is still sharp enough for
            // accurate OCR on typical document scans.
            let vl_bytes = maybe_downscale_for_paddlevl(&bytes_owned);

            // Write bytes to a temp file for PaddleVL subprocess
            let temp_path = std::env::temp_dir()
                .join(format!("entropia_paddlevl_{}.png", uuid::Uuid::new_v4()));

            if let Err(e) = std::fs::write(&temp_path, &vl_bytes) {
                eprintln!(
                    "[OCRH] Failed to write temp file for PaddleVL for {asset_id_owned}: {e}. \
                     Falling back to PaddleOCR light."
                );
                return provider_clone
                    .recognize(&bytes_owned)
                    .map(|ocr| ProcessedOcrOutput { ocr, layout: None })
                    .map_err(|e| format!("OCR inference failed: {e}"));
            }

            let temp_path_str = match temp_path.to_str() {
                Some(s) => s.to_string(),
                None => {
                    eprintln!(
                        "[OCRH] Invalid temp path for PaddleVL for {asset_id_owned}. \
                         Falling back to PaddleOCR light."
                    );
                    return provider_clone
                        .recognize(&bytes_owned)
                        .map(|ocr| ProcessedOcrOutput { ocr, layout: None })
                        .map_err(|e| format!("OCR inference failed: {e}"));
                }
            };

            // Run PaddleVL detection via Python subprocess
            let vl_result = engine_clone.detect(&temp_path_str);
            let _ = std::fs::remove_file(&temp_path); // best-effort cleanup

            match vl_result {
                Ok(mut vl_output) => {
                    if let Some((original_width, original_height)) = original_image_dimensions {
                        rescale_paddlevl_output_to_dimensions(
                            &mut vl_output,
                            original_width,
                            original_height,
                        );
                    }
                    eprintln!(
                        "[OCRH] PaddleVL detected {} blocks for {asset_id_owned}",
                        vl_output.blocks.len()
                    );
                    Ok(ProcessedOcrOutput {
                        ocr: ocr_output_from_paddlevl(&vl_output),
                        layout: Some(LayoutPersistencePayload::from_page(1, &vl_output)),
                    })
                }
                Err(e) => {
                    eprintln!(
                        "[OCRH] PaddleVL failed for {asset_id_owned}: {e}. \
                         Falling back to PaddleOCR light."
                    );
                    crate::app_logs::warn(
                        &app_handle_for_high,
                        "ocrh",
                        format!(
                            "PaddleOCR-VL falló/agotó timeout en {asset_id_owned}; usando PaddleOCR liviano: {e}"
                        ),
                    );
                    provider_clone
                        .recognize(&bytes_owned)
                        .map(|ocr| ProcessedOcrOutput { ocr, layout: None })
                        .map_err(|e| format!("OCR inference failed: {e}"))
                }
            }
        })
        .await
        .map_err(|e| format!("OCR task panicked: {e}"))??;

        emit_progress(app_handle, asset_id, 100, "done");
        return Ok(output);
    }

    eprintln!(
        "[OCRH] PaddleVL engine unavailable for {asset_id}. Falling back to PaddleOCR light."
    );
    crate::app_logs::warn(
        app_handle,
        "ocrh",
        format!("PaddleOCR-VL no disponible para {asset_id}; usando PaddleOCR liviano"),
    );
    process_image_light(provider, bytes, asset_id, app_handle).await
}

/// Emit an `ocr:progress` event to the frontend.
fn emit_progress(app_handle: &AppHandle, asset_id: &str, pct: u8, stage: &str) {
    let _ = app_handle.emit(
        "ocr:progress",
        OcrProgressPayload {
            asset_id: asset_id.to_string(),
            pct,
            stage: stage.to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::paddle_vl_types::{PaddleVlBbox, PaddleVlBlock, PaddleVlOutput, PaddleVlRegion};
    use crate::ocr::provider::{BoundingBox, LayoutCategory};
    use crate::runtime::status::{RuntimeCapability, RuntimeState, RuntimeStatus};
    use std::cell::RefCell;
    use std::path::PathBuf;

    #[test]
    fn test_format_region_text_title() {
        let result = format_region_text(&LayoutCategory::Title, "Introduction");
        assert_eq!(result, Some("## Introduction".to_string()));
    }

    #[test]
    fn test_format_region_text_plain_text() {
        let result = format_region_text(&LayoutCategory::PlainText, "Some body text");
        assert_eq!(result, Some("Some body text".to_string()));
    }

    #[test]
    fn test_format_region_text_table() {
        let result = format_region_text(&LayoutCategory::Table, "col1 | col2\na | b");
        assert_eq!(result, Some("---\ncol1 | col2\na | b\n---".to_string()));
    }

    #[test]
    fn test_format_region_text_figure_skipped() {
        let result = format_region_text(&LayoutCategory::Figure, "image description");
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_region_text_caption() {
        let result = format_region_text(&LayoutCategory::Caption, "Figure 1: Diagram");
        assert_eq!(result, Some("Figure 1: Diagram".to_string()));
    }

    #[test]
    fn test_format_region_text_footnote() {
        let result = format_region_text(&LayoutCategory::Footnote, "See reference 1");
        assert_eq!(result, Some("Note: See reference 1".to_string()));
    }

    #[test]
    fn test_format_region_text_header_skipped() {
        let result = format_region_text(&LayoutCategory::Header, "Page 1");
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_region_text_footer_skipped() {
        let result = format_region_text(&LayoutCategory::Footer, "Page 1");
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_region_text_code() {
        let result = format_region_text(&LayoutCategory::Code, "fn main() {}");
        assert_eq!(result, Some("```\nfn main() {}\n```".to_string()));
    }

    #[test]
    fn test_format_region_text_reference() {
        let result = format_region_text(&LayoutCategory::Reference, "[1] Smith 2024");
        assert_eq!(result, Some("[1] Smith 2024".to_string()));
    }

    #[test]
    fn test_format_region_text_abandoned_skipped() {
        let result = format_region_text(&LayoutCategory::Abandoned, "seal content");
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_region_text_empty_skipped() {
        let result = format_region_text(&LayoutCategory::PlainText, "   ");
        assert_eq!(result, None);
    }

    #[test]
    fn test_crop_region_basic() {
        // Create a 200x200 white image
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            200,
            200,
            image::Rgba([255, 255, 255, 255]),
        ));

        let bbox = BoundingBox {
            x: 50,
            y: 50,
            width: 100,
            height: 100,
        };
        let cropped = crop_region(&img, &bbox);

        assert!(cropped.is_some(), "Crop should succeed for valid bbox");
        let cropped = cropped.unwrap();
        // Should be larger than 100x100 due to 15% padding (now 30px on each side)
        // Total expected: 100 + 30 + 30 = 160 (or clamped to image bounds)
        assert!(
            cropped.width() >= 100,
            "Cropped width should be at least 100, got {}",
            cropped.width()
        );
        assert!(
            cropped.height() >= 100,
            "Cropped height should be at least 100, got {}",
            cropped.height()
        );
    }

    #[test]
    fn test_crop_region_at_edge() {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            200,
            200,
            image::Rgba([255, 255, 255, 255]),
        ));

        // Region near the top-left corner — padding should be clamped
        let bbox = BoundingBox {
            x: 0,
            y: 0,
            width: 50,
            height: 50,
        };
        let cropped = crop_region(&img, &bbox);

        assert!(cropped.is_some(), "Crop at edge should succeed");
    }

    #[test]
    fn test_crop_region_too_small() {
        // A region that after 5px margin on each side is still < 10px
        // should be skipped — too small for useful OCR.
        let _tiny_img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            15,
            15,
            image::Rgba([255, 255, 255, 255]),
        ));
        // 3x3 region + 5px margin each side in a 15x15 image:
        //   x1 = max(5-5, 0) = 0, x2 = min(5+3+5, 15) = 13 → width = 13
        // That's >= 10, so we need an even smaller image:
        let micro_img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([255, 255, 255, 255]),
        ));
        // 2x2 region + 5px margin clamped to 8x8 image:
        //   x1 = max(3-5, 0) = 0, x2 = min(3+2+5, 8) = 8 → width = 8
        //   8 < 10 → skipped
        let bbox = BoundingBox {
            x: 3,
            y: 3,
            width: 2,
            height: 2,
        };
        let cropped = crop_region(&micro_img, &bbox);

        assert!(
            cropped.is_none(),
            "Region that crops to <10px after margin should be skipped"
        );
    }

    #[test]
    fn test_crop_region_margin_is_5px() {
        // Verify the margin is exactly 5px on each side.
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            500,
            500,
            image::Rgba([255, 255, 255, 255]),
        ));

        // 50x50 region at center — 5px margin each side → 60x60 crop
        let bbox = BoundingBox {
            x: 200,
            y: 200,
            width: 50,
            height: 50,
        };
        let cropped = crop_region(&img, &bbox).expect("crop should succeed");

        assert_eq!(cropped.width(), 60, "50px + 5px + 5px = 60px width");
        assert_eq!(cropped.height(), 60, "50px + 5px + 5px = 60px height");
    }

    // Exercises the paddle-only layout payload builder (`from_page`).
    #[cfg(feature = "paddle-ocr")]
    #[test]
    fn test_layout_payload_tracks_page_metadata_and_blocks() {
        let output = PaddleVlOutput {
            text: "hola".to_string(),
            method: "paddle_vl".to_string(),
            blocks: vec![PaddleVlBlock {
                label: "title".to_string(),
                content: "Portada".to_string(),
                bbox: PaddleVlBbox {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40,
                },
                order: 0,
                group_id: 1,
            }],
            regions: vec![PaddleVlRegion {
                category: "title".to_string(),
                bbox: PaddleVlBbox {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40,
                },
                confidence: 0.98,
            }],
            image_width: 1200,
            image_height: 1800,
            actual_device: None,
        };

        let payload = LayoutPersistencePayload::from_page(2, &output);

        assert_eq!(payload.model, "paddle_vl");
        assert_eq!(payload.blocks.len(), 1);
        assert_eq!(payload.regions.len(), 1);
        assert_eq!(payload.blocks[0].page, 2);
        assert_eq!(payload.blocks[0].image_width, 1200);
        assert_eq!(payload.regions[0].page, 2);
        assert_eq!(payload.regions[0].image_height, 1800);
    }

    // Exercises the paddle-only bbox rescaling helper.
    #[cfg(feature = "paddle-ocr")]
    #[test]
    fn test_rescale_paddlevl_output_to_original_dimensions() {
        let mut output = PaddleVlOutput {
            text: "hola".to_string(),
            method: "paddle_vl".to_string(),
            blocks: vec![PaddleVlBlock {
                label: "title".to_string(),
                content: "Portada".to_string(),
                bbox: PaddleVlBbox {
                    x: 100,
                    y: 50,
                    width: 200,
                    height: 80,
                },
                order: 0,
                group_id: 1,
            }],
            regions: vec![PaddleVlRegion {
                category: "title".to_string(),
                bbox: PaddleVlBbox {
                    x: 100,
                    y: 50,
                    width: 200,
                    height: 80,
                },
                confidence: 0.98,
            }],
            image_width: 1000,
            image_height: 334,
            actual_device: None,
        };

        rescale_paddlevl_output_to_dimensions(&mut output, 2425, 809);

        assert_eq!(output.image_width, 2425);
        assert_eq!(output.image_height, 809);
        assert_eq!(output.blocks[0].bbox.x, 242);
        assert_eq!(output.blocks[0].bbox.y, 121);
        assert_eq!(output.blocks[0].bbox.width, 485);
        assert_eq!(output.blocks[0].bbox.height, 194);
        assert_eq!(output.regions[0].bbox.x, 242);
        assert_eq!(output.regions[0].bbox.height, 194);
    }

    #[test]
    fn test_save_layout_upserts_and_delete_layout_clears_row() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE assets (id TEXT PRIMARY KEY);
             INSERT INTO assets(id) VALUES ('asset-1');
             CREATE TABLE layouts (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                regions TEXT NOT NULL,
                blocks TEXT NOT NULL,
                model TEXT NOT NULL,
                image_width INTEGER NOT NULL,
                image_height INTEGER NOT NULL,
                created_at INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX idx_layouts_asset_id_unique ON layouts(asset_id);",
        )
        .expect("schema");

        let payload = LayoutPersistencePayload {
            model: "paddle_vl".to_string(),
            image_width: 900,
            image_height: 1400,
            regions: vec![PersistedLayoutRegion {
                page: 1,
                image_width: 900,
                image_height: 1400,
                category: "plain_text".to_string(),
                bbox: PaddleVlBbox {
                    x: 1,
                    y: 2,
                    width: 3,
                    height: 4,
                },
                confidence: 0.9,
            }],
            blocks: vec![PersistedLayoutBlock {
                page: 1,
                image_width: 900,
                image_height: 1400,
                label: "plain_text".to_string(),
                content: "texto".to_string(),
                bbox: PaddleVlBbox {
                    x: 1,
                    y: 2,
                    width: 3,
                    height: 4,
                },
                order: 0,
                group_id: 0,
            }],
        };

        save_layout(&conn, "asset-1", &payload).expect("first upsert");
        save_layout(&conn, "asset-1", &payload).expect("second upsert");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM layouts WHERE asset_id = ?1",
                ["asset-1"],
                |row| row.get(0),
            )
            .expect("count row");
        assert_eq!(count, 1, "layout upsert should keep one row per asset");

        let blocks_json: String = conn
            .query_row(
                "SELECT blocks FROM layouts WHERE asset_id = ?1",
                ["asset-1"],
                |row| row.get(0),
            )
            .expect("blocks json");
        assert!(blocks_json.contains("texto"));

        delete_layout(&conn, "asset-1").expect("delete layout");
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM layouts WHERE asset_id = ?1",
                ["asset-1"],
                |row| row.get(0),
            )
            .expect("remaining row count");
        assert_eq!(remaining, 0);
    }

    #[test]

    fn test_glm_response_to_processed_output_uses_markdown_and_maps_layout() {
        let response = GlmOcrResponse {
            id: Some("task-1".to_string()),
            created: Some(1),
            model: Some("GLM-OCR".to_string()),
            md_results: "# Título\n\nTexto".to_string(),
            layout_details: vec![vec![
                GlmOcrLayoutDetail {
                    index: Some(1),
                    label: Some("text".to_string()),
                    bbox_2d: vec![0.1, 0.2, 0.5, 0.4],
                    content: Some("Título".to_string()),
                    height: Some(1000),
                    width: Some(800),
                },
                GlmOcrLayoutDetail {
                    index: Some(2),
                    label: Some("table".to_string()),
                    bbox_2d: vec![0.2, 0.5, 0.8, 0.9],
                    content: Some("<table><tr><td>a</td></tr></table>".to_string()),
                    height: Some(1000),
                    width: Some(800),
                },
            ]],
            data_info: None,
            request_id: Some("req-1".to_string()),
        };

        let output = glm_response_to_processed_output(&response, "glm_ocr").expect("glm output");

        assert_eq!(output.ocr.text, "# Título\n\nTexto");
        assert_eq!(output.ocr.method, "glm_ocr");
        assert_eq!(output.layout.as_ref().expect("layout").blocks.len(), 2);
        assert_eq!(
            output.layout.as_ref().expect("layout").blocks[0].label,
            "title"
        );
        assert_eq!(
            output.layout.as_ref().expect("layout").blocks[1].label,
            "table"
        );
        assert_eq!(output.layout.as_ref().expect("layout").image_width, 800);
        assert_eq!(output.layout.as_ref().expect("layout").image_height, 1000);
    }

    #[test]
    fn test_glm_response_promotes_markdown_heading_inside_wrapped_html() {
        let response = GlmOcrResponse {
            id: None,
            created: None,
            model: None,
            md_results: "<div align=\"center\">\n\n# MAR DEL PLATA: Lucha en la industria del pescado\n\n</div>".to_string(),
            layout_details: vec![vec![GlmOcrLayoutDetail {
                index: Some(1),
                label: Some("text".to_string()),
                bbox_2d: vec![0.1, 0.1, 0.4, 0.1],
                content: Some("MAR DEL PLATA: Lucha en la industria del pescado".to_string()),
                height: Some(1000),
                width: Some(800),
            }]],
            data_info: None,
            request_id: None,
        };

        let output = glm_response_to_processed_output(&response, "glm_ocr").expect("glm output");
        assert_eq!(
            output.layout.as_ref().expect("layout").blocks[0].label,
            "title"
        );
        assert_eq!(
            output.layout.as_ref().expect("layout").regions[0].category,
            "title"
        );
    }

    #[test]
    fn test_glm_response_promotes_multiline_centered_heading_lines_to_title() {
        let response = GlmOcrResponse {
            id: None,
            created: None,
            model: None,
            md_results: "<div align=\"center\">\n\n# Trabajadores del Pescado\n\nRechazan una Impugnación\n\n</div>".to_string(),
            layout_details: vec![vec![
                GlmOcrLayoutDetail {
                    index: Some(1),
                    label: Some("text".to_string()),
                    bbox_2d: vec![0.1, 0.1, 0.4, 0.1],
                    content: Some("Trabajadores del Pescado".to_string()),
                    height: Some(1000),
                    width: Some(800),
                },
                GlmOcrLayoutDetail {
                    index: Some(2),
                    label: Some("text".to_string()),
                    bbox_2d: vec![0.1, 0.22, 0.45, 0.1],
                    content: Some("Rechazan una Impugnación".to_string()),
                    height: Some(1000),
                    width: Some(800),
                },
            ]],
            data_info: None,
            request_id: None,
        };

        let output = glm_response_to_processed_output(&response, "glm_ocr").expect("glm output");
        let layout = output.layout.as_ref().expect("layout");
        assert_eq!(layout.blocks[0].label, "title");
        assert_eq!(layout.blocks[1].label, "title");
        assert_eq!(layout.regions[0].category, "title");
        assert_eq!(layout.regions[1].category, "title");
    }

    #[test]
    fn test_glm_response_promotes_heading_when_html_precedes_markdown_on_same_line() {
        let response = GlmOcrResponse {
            id: None,
            created: None,
            model: None,
            md_results: "<div align=\"center\"> ## Trabajadores del Pescado </div>".to_string(),
            layout_details: vec![vec![GlmOcrLayoutDetail {
                index: Some(1),
                label: Some("text".to_string()),
                bbox_2d: vec![663.0, 521.0, 1323.0, 592.0],
                content: Some("## Trabajadores del Pescado".to_string()),
                height: Some(4950),
                width: Some(3825),
            }]],
            data_info: None,
            request_id: None,
        };

        let output = glm_response_to_processed_output(&response, "glm_ocr").expect("glm output");
        let layout = output.layout.as_ref().expect("layout");
        assert_eq!(layout.blocks[0].label, "title");
        assert_eq!(layout.regions[0].category, "title");
    }

    #[test]
    fn test_glm_bbox_conversion_supports_xywh_shape() {
        let bbox = normalized_bbox_to_pixels(
            &GlmOcrLayoutDetail {
                index: Some(1),
                label: Some("text".to_string()),
                bbox_2d: vec![0.1, 0.2, 0.05, 0.04],
                content: Some("Texto".to_string()),
                height: Some(1000),
                width: Some(800),
            },
            800,
            1000,
        )
        .expect("bbox");

        assert_eq!(bbox.x, 80);
        assert_eq!(bbox.y, 200);
        assert_eq!(bbox.width, 40);
        assert_eq!(bbox.height, 40);
    }

    #[test]
    fn test_glm_bbox_conversion_supports_absolute_xywh_shape() {
        let bbox = normalized_bbox_to_pixels(
            &GlmOcrLayoutDetail {
                index: Some(1),
                label: Some("text".to_string()),
                bbox_2d: vec![1726.0, 2880.0, 640.0, 180.0],
                content: Some("Texto".to_string()),
                height: Some(3508),
                width: Some(2480),
            },
            2480,
            3508,
        )
        .expect("bbox");

        assert_eq!(bbox.x, 1726);
        assert_eq!(bbox.y, 2880);
        assert_eq!(bbox.width, 640);
        assert_eq!(bbox.height, 180);
    }

    #[test]
    fn test_glm_response_has_useful_content_detects_empty_responses() {
        let empty = GlmOcrResponse {
            id: None,
            created: None,
            model: None,
            md_results: "   ".to_string(),
            layout_details: vec![vec![GlmOcrLayoutDetail {
                index: Some(1),
                label: Some("image".to_string()),
                bbox_2d: vec![0.0, 0.0, 1.0, 1.0],
                content: Some("https://example.com/image.png".to_string()),
                height: Some(10),
                width: Some(10),
            }]],
            data_info: None,
            request_id: None,
        };

        let useful = GlmOcrResponse {
            id: None,
            created: None,
            model: None,
            md_results: " ".to_string(),
            layout_details: vec![vec![GlmOcrLayoutDetail {
                index: Some(1),
                label: Some("text".to_string()),
                bbox_2d: vec![0.0, 0.0, 1.0, 1.0],
                content: Some("hola".to_string()),
                height: Some(10),
                width: Some(10),
            }]],
            data_info: None,
            request_id: None,
        };

        assert!(!glm_response_has_useful_content(&empty));
        assert!(glm_response_has_useful_content(&useful));
    }

    #[cfg(feature = "paddle-ocr")]
    #[test]
    fn resolve_paddle_model_dir_prefers_managed_runtime_assets() {
        let runtime_dir = tempfile::tempdir().expect("runtime dir");
        let manifest_dir = tempfile::tempdir().expect("manifest dir");
        let managed_models = runtime_dir
            .path()
            .join("resources")
            .join("models")
            .join("ocr");
        std::fs::create_dir_all(&managed_models).expect("create model dir");

        let resolved =
            resolve_paddle_model_dir_from_roots(Some(runtime_dir.path()), manifest_dir.path());

        assert_eq!(resolved, managed_models);
    }

    #[test]
    fn managed_runtime_root_for_ocr_bootstraps_before_resolving_assets() {
        let calls = RefCell::new(Vec::new());
        let expected = PathBuf::from("/tmp/runtime-ready");

        let resolved = managed_runtime_root_for_ocr_with(
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
    fn managed_runtime_root_for_ocr_stays_blocked_when_bootstrap_cannot_prepare_runtime() {
        let calls = RefCell::new(Vec::new());

        let resolved = managed_runtime_root_for_ocr_with(
            || {
                calls.borrow_mut().push("ensure_ready");
                Ok(RuntimeStatus {
                    state: RuntimeState::BlockedSourceUnavailable,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: false,
                    summary: "No hay una fuente confiable disponible para bootstrap".to_string(),
                    blocked_capabilities: vec![RuntimeCapability::Ocr],
                    details: vec!["manifest remoto no publicado".to_string()],
                    guidance: vec!["Reintentá cuando exista una fuente firmada".to_string()],
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
        .expect("blocked runtime should not raise transport errors");

        assert_eq!(resolved, None);
        assert_eq!(calls.into_inner(), vec!["ensure_ready"]);
    }
}
