//! PDF text extraction and page rendering for OCR fallback.
//!
//! Two extraction strategies:
//! 1. **Native text** — `extract_pdf_text()` extracts embedded text via `pdf-extract`.
//!    Fast and accurate for text-based PDFs. Quality-checked with `is_quality_text()`.
//! 2. **Page rendering** — `render_pdf_page_to_image()` renders a PDF page as PNG
//!    bitmap via `pdfium-render`, enabling OCR fallback for scanned/image-based PDFs.
//!
//! Thumbnails:
//! - `render_pdf_thumbnail()` renders the first page at 400px width, suitable for
//!   card previews in the collection view.
//!
//! For multi-page PDFs, `pdf_page_count()` returns the total number of pages,
//! and `render_pdf_page_to_image()` accepts any page index (not just page 0).
//!
//! # Pdfium native library resolution
//!
//! The `pdfium-render` crate requires a native Pdfium shared library (`pdfium.dll`
//! on Windows, `libpdfium.so` on Linux, `libpdfium.dylib` on macOS).
//!
//! Resolution order (3-tier, matching the bundled native-library patterns):
//! 1. **Bundled resource** — `resources/lib/` via Tauri's `BaseDirectory::Resource`
//! 2. **Dev fallback** — `CARGO_MANIFEST_DIR/resources/lib/` (for development)
//! 3. **System library** — OS default search paths (`PATH`, `/usr/lib`, etc.)
//!
//! Call `init_pdfium_path()` once during app startup (from OCR worker or command
//! handler) to cache the resolved path. If never called, falls back to current
//! directory + system library (original pdfium-render behavior).

#[cfg(feature = "local-ml")]
use crate::runtime::{managed_resource_path, RuntimeManager};
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use imageproc::geometric_transformations::{rotate_about_center, Interpolation};
use pdfium_render::prelude::*;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Cached resolved path to the Pdfium native library.
///
/// - `Some(Some(path))` = initialized with a resolved DLL path
/// - `Some(None)` = initialized, but DLL not found in bundled paths (use system library)
/// - `None` = not yet initialized (fall back to CWD + system library)
static PDFIUM_PATH: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

/// GLM-OCR rejects page images larger than 10 decimal megabytes.
pub const MAX_RENDERED_PAGE_IMAGE_BYTES: usize = 10_000_000;

pub(super) trait RenderedPageSource {
    fn page_count(&mut self) -> Result<usize, String>;
    fn render_page(&mut self, index: usize) -> Result<Vec<u8>, String>;
}

struct PdfiumPageSource<'a> {
    pages: &'a PdfPages<'a>,
}

impl RenderedPageSource for PdfiumPageSource<'_> {
    fn page_count(&mut self) -> Result<usize, String> {
        Ok(self.pages.len().into())
    }

    fn render_page(&mut self, index: usize) -> Result<Vec<u8>, String> {
        let page = self
            .pages
            .get(PdfPageIndex::from(index as u16))
            .map_err(|e| format!("Failed to get page {index} from PDF: {e}"))?;

        render_pdf_page(&page, index)
            .map_err(|e| format!("Failed to render PDF page {}: {e}", index + 1))
    }
}

pub(super) fn visit_rendered_pages<S, V>(source: &mut S, mut visitor: V) -> Result<usize, String>
where
    S: RenderedPageSource,
    V: FnMut(usize, usize, &[u8]) -> Result<(), String>,
{
    let page_count = source.page_count()?;

    for page_index in 0..page_count {
        let png = source.render_page(page_index)?;
        if png.len() > MAX_RENDERED_PAGE_IMAGE_BYTES {
            return Err(format!(
                "Rendered PDF page {} image exceeds the {} byte limit",
                page_index + 1,
                MAX_RENDERED_PAGE_IMAGE_BYTES
            ));
        }
        visitor(page_index, page_count, &png)?;
    }

    Ok(page_count)
}

/// Resolve the Pdfium native library path using 3-tier resolution.
///
/// This function MUST be called once during app startup (from the OCR worker or
/// command handler) to cache the DLL path. It is safe to call multiple times —
/// only the first call sets the cached value.
///
/// # Resolution order
/// 1. Tauri resource path: `BaseDirectory::Resource` + `resources/lib/`
/// 2. CARGO_MANIFEST_DIR fallback: `<manifest>/resources/lib/`
/// 3. No bundled path found → falls back to system library at runtime
pub fn init_pdfium_path(app_handle: &tauri::AppHandle) {
    // The managed runtime root only exists when local inference is compiled in.
    // Without `local-ml`, fall through to the dev/system-library lookup below.
    #[cfg(feature = "local-ml")]
    let runtime_root = managed_runtime_root_for_pdfium(app_handle).ok().flatten();
    #[cfg(not(feature = "local-ml"))]
    let runtime_root: Option<PathBuf> = {
        let _ = app_handle;
        None
    };
    let resolved = resolve_pdfium_dll_path_from_roots(
        runtime_root.as_deref(),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
    );

    let cache = PDFIUM_PATH.get_or_init(|| Mutex::new(None));
    let mut cached = cache.lock().expect("pdfium path cache poisoned");
    let should_update = match (&*cached, &resolved) {
        (None, Some(_)) => true,
        (Some(existing), Some(new_path)) => existing != new_path,
        _ => false,
    };

    if should_update {
        *cached = resolved.clone();
    }

    match cached.as_ref() {
        Some(path) => eprintln!(
            "[pdf] ✅ Pdfium native library resolved: {}",
            path.display()
        ),
        None => {
            eprintln!(
                "[pdf] ℹ️ Pdfium no se resolvió desde runtime/resources dev; se intentará la librería del sistema ({})",
                dll_name_display()
            )
        }
    }
}

#[cfg(feature = "local-ml")]
fn managed_runtime_root_for_pdfium(
    app_handle: &tauri::AppHandle,
) -> Result<Option<PathBuf>, String> {
    managed_runtime_root_for_pdfium_with(
        || RuntimeManager::new().ensure_ready_or_bootstrap(app_handle),
        || RuntimeManager::new().hydrated_runtime_root(app_handle),
    )
}

// Only the local-ml `managed_runtime_root_for_pdfium` wrapper and the unit tests
// call this; in the lean lib build (no local-ml, no tests) it is unreferenced.
#[cfg(any(feature = "local-ml", test))]
fn managed_runtime_root_for_pdfium_with<E, H>(
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

fn resolve_pdfium_dll_path_from_roots(
    managed_root: Option<&std::path::Path>,
    manifest_dir: &std::path::Path,
) -> Option<PathBuf> {
    let dll_name = Pdfium::pdfium_platform_library_name();

    if let Some(root) = managed_root {
        // `managed_resource_path` lives in the local-ml-gated runtime module, but
        // its layout (`<root>/resources/<rel>`) is stable. Inline the same join in
        // the lean build so the bundled-pdfium lookup still resolves there.
        #[cfg(feature = "local-ml")]
        let managed = managed_resource_path(root, "lib").join(&dll_name);
        #[cfg(not(feature = "local-ml"))]
        let managed = root.join("resources").join("lib").join(&dll_name);
        if managed.exists() {
            return Some(managed);
        }
    }

    for dev_path in dev_pdfium_candidate_paths(manifest_dir, dll_name.to_string_lossy().as_ref()) {
        if dev_path.exists() {
            return Some(strip_windows_prefix(dev_path));
        }
    }

    None
}

fn dev_pdfium_candidate_paths(manifest_dir: &Path, dll_name: &str) -> Vec<PathBuf> {
    let base_candidate = manifest_dir.join("resources").join("lib").join(dll_name);

    #[cfg(target_os = "linux")]
    {
        let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        let mut candidates = vec![base_candidate];
        candidates.push(
            manifest_dir
                .join("resources")
                .join("lib")
                .join(&platform)
                .join(dll_name),
        );
        candidates.push(
            manifest_dir
                .join("resources")
                .join("runtime-pack")
                .join(platform)
                .join("resources")
                .join("lib")
                .join(dll_name),
        );
        candidates
    }

    #[cfg(not(target_os = "linux"))]
    {
        vec![base_candidate]
    }
}

/// Strip the Windows `\\?\` UNC prefix from a path if present.
///
/// Tauri's `resolve()` on Windows may return paths with the `\\?\` prefix
/// (extended-length path prefix). Some native libraries and APIs don't handle
/// this prefix correctly, so we strip it for compatibility.
fn strip_windows_prefix(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy().into_owned();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path
    }
}

/// Initialize a Pdfium instance without panicking.
///
/// Uses the cached DLL path if `init_pdfium_path()` was called, otherwise
/// falls back to current directory + system library (original behavior).
///
/// # Errors
/// Returns `Err` with a human-readable message if the Pdfium native
/// library cannot be loaded (missing DLL/so/dylib, wrong architecture, etc.).
fn get_pdfium() -> Result<Pdfium, String> {
    let cached_path = PDFIUM_PATH
        .get()
        .and_then(|cache| cache.lock().ok().and_then(|path| path.clone()));
    let attempted_resolved_path = cached_path.clone();

    let bindings = match cached_path.as_ref() {
        // Initialized with a resolved DLL path — try that first, then system library
        Some(path) => Pdfium::bind_to_library(path).or_else(|path_err| {
            eprintln!(
                "[pdf] Failed to load pdfium from resolved path ({}): {path_err} — trying system library",
                path.display()
            );
            Pdfium::bind_to_system_library()
        }),
        // Initialized but no bundled DLL found — system library only
        None if PDFIUM_PATH.get().is_some() => Pdfium::bind_to_system_library(),
        // Not initialized — fall back to CWD + system library (original pdfium-render behavior)
        None => Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./"))
            .or_else(|_| Pdfium::bind_to_system_library()),
    }
    .map_err(|e| {
        let resolved_path_note = attempted_resolved_path
            .as_ref()
            .map(|path| format!("- Resolved bundled/dev path attempted: {}\n", path.display()))
            .unwrap_or_default();

        format!(
            "Could not load Pdfium native library.\n\
             Error: {e}\n\n\
             Resolution tried:\n\
             {}\
             - Bundled resource: resources/lib/{}\n\
             - Development: CARGO_MANIFEST_DIR/resources/lib/{}\n\
             - Linux dev fallback: CARGO_MANIFEST_DIR/resources/lib/linux-x86_64/{}\n\
             - Runtime-pack dev fallback: CARGO_MANIFEST_DIR/resources/runtime-pack/<platform>/resources/lib/{}\n\
             - System library paths (PATH, /usr/lib, etc.)\n\n\
             Make sure the Pdfium shared library is installed and accessible.\n\
             On Windows, place pdfium.dll in resources/lib/ or install it globally.",
            resolved_path_note,
            dll_name_display(),
            dll_name_display(),
            dll_name_display(),
            dll_name_display(),
        )
    })?;

    Ok(Pdfium::new(bindings))
}

/// Returns the platform-specific Pdfium library filename for error messages.
fn dll_name_display() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "pdfium.dll"
    }
    #[cfg(target_os = "linux")]
    {
        "libpdfium.so"
    }
    #[cfg(target_os = "macos")]
    {
        "libpdfium.dylib"
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        "pdfium"
    }
}

/// Extract text from the native text layer of a PDF byte slice.
/// Returns the raw extracted text or an error message.
pub fn extract_pdf_text(bytes: &[u8]) -> Result<String, String> {
    pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| format!("PDF text extraction failed: {e}"))
}

/// Returns `true` if the text contains at least `MIN_ALPHANUM_CHARS` valid
/// UTF-8 alphanumeric characters. Used to decide whether native PDF text is
/// rich enough or we should fall back to OCR.
pub fn is_quality_text(text: &str) -> bool {
    const MIN_ALPHANUM_CHARS: usize = 50;
    text.chars().filter(|c| c.is_alphanumeric()).count() >= MIN_ALPHANUM_CHARS
}

/// Build a conservative per-page profile for a PDF, synchronously.
///
/// Pro has no Pdfium render actor (unlike Lite); this binds the engine via the
/// same `get_pdfium()` path as `pdf_page_count`/`render_pdf_page_to_image`, then
/// delegates the per-page profiling to `pdf_probe::profile_pdf_with_engine`.
/// Pdfium work is blocking — call from a blocking-safe context.
pub(super) fn profile_pdf_sync(bytes: &[u8]) -> Result<super::pdf_probe::DocumentProfile, String> {
    let pdfium = get_pdfium()?;
    super::pdf_probe::profile_pdf_with_engine(&pdfium, bytes)
}

/// Get the number of pages in a PDF document.
///
/// Used by OCR and editing pipelines to validate page structure.
pub fn pdf_page_count(bytes: &[u8]) -> Result<usize, String> {
    let document = load_lopdf_document(bytes, "page count")?;
    Ok(document.get_pages().len())
}

fn load_lopdf_document(bytes: &[u8], operation: &str) -> Result<lopdf::Document, String> {
    match lopdf::Document::load_mem(bytes) {
        Ok(document) => Ok(document),
        Err(error)
            if error
                .to_string()
                .contains("invalid start value in Prev field") =>
        {
            let repaired = neutralize_invalid_latest_prev(bytes)
                .ok_or_else(|| format!("Failed to load PDF for {operation}: {error}"))?;
            lopdf::Document::load_mem(&repaired).map_err(|retry_error| {
                format!(
                    "Failed to load PDF for {operation} after ignoring invalid Prev pointer: {retry_error}"
                )
            })
        }
        Err(error) => Err(format!("Failed to load PDF for {operation}: {error}")),
    }
}

fn neutralize_invalid_latest_prev(bytes: &[u8]) -> Option<Vec<u8>> {
    let startxref = bytes
        .windows(b"startxref".len())
        .rposition(|window| window == b"startxref")?;
    let xref_offset = std::str::from_utf8(&bytes[startxref + b"startxref".len()..])
        .ok()?
        .split_whitespace()
        .next()?
        .parse::<usize>()
        .ok()?;
    if xref_offset >= startxref {
        return None;
    }

    let latest_xref = &bytes[xref_offset..startxref];
    let prev_offset = latest_xref
        .windows(b"/Prev".len())
        .rposition(|window| window == b"/Prev")?;
    let value_start = prev_offset
        + b"/Prev".len()
        + latest_xref[prev_offset + b"/Prev".len()..]
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
    let value_len = latest_xref[value_start..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if value_len == 0 {
        return None;
    }

    let token_start = xref_offset + prev_offset;
    let token_end = xref_offset + value_start + value_len;
    let mut repaired = bytes.to_vec();
    repaired[token_start..token_end].fill(b' ');
    Some(repaired)
}

/// Split a PDF into one single-page PDF per page, preserving the original page
/// content without rasterizing or recompressing it.
///
/// Each returned tuple is `(page_number, pdf_bytes)` with 1-based page numbers.
/// The import flow uses this to decompose a multi-page PDF into one PDF asset
/// per page (each sent directly to GLM-OCR as a PDF), keeping the original
/// document as the parent asset. Parsing and serialization are pure Rust but
/// still blocking, so call this from a blocking-safe context.
pub fn split_pdf_to_single_page_bytes(bytes: &[u8]) -> Result<Vec<(u32, Vec<u8>)>, String> {
    let source = load_lopdf_document(bytes, "splitting")?;
    let page_numbers = source.get_pages().keys().copied().collect::<Vec<_>>();
    if page_numbers.is_empty() {
        return Err("Cannot split a PDF without pages".to_string());
    }

    let mut pages = Vec::with_capacity(page_numbers.len());
    for (index, page_number) in page_numbers.iter().copied().enumerate() {
        let mut single = source.clone();
        let pages_to_delete = page_numbers
            .iter()
            .copied()
            .filter(|candidate| *candidate != page_number)
            .collect::<Vec<_>>();
        single.delete_pages(&pages_to_delete);
        single.prune_objects();
        single.renumber_objects();

        let mut pdf_bytes = Vec::new();
        single
            .save_to(&mut pdf_bytes)
            .map_err(|e| format!("Failed to save single-page PDF for page {}: {e}", index + 1))?;
        pages.push((index as u32 + 1, pdf_bytes));
    }

    Ok(pages)
}

fn normalized_crop_bounds(
    image_width: u32,
    image_height: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(u32, u32, u32, u32), String> {
    if image_width == 0 || image_height == 0 {
        return Err("Cannot crop an empty PDF page".to_string());
    }
    if ![x, y, width, height].iter().all(|value| value.is_finite()) {
        return Err("PDF crop coordinates must be finite".to_string());
    }
    const NORMALIZED_EPSILON: f64 = 1e-9;
    if x < 0.0
        || y < 0.0
        || width <= 0.0
        || height <= 0.0
        || x + width > 1.0 + NORMALIZED_EPSILON
        || y + height > 1.0 + NORMALIZED_EPSILON
    {
        return Err("PDF crop coordinates must define a non-empty normalized region".to_string());
    }

    let left = (x * f64::from(image_width)).floor() as u32;
    let top = (y * f64::from(image_height)).floor() as u32;
    let right = ((x + width) * f64::from(image_width)).ceil() as u32;
    let bottom = ((y + height) * f64::from(image_height)).ceil() as u32;
    let crop_width = right.min(image_width).saturating_sub(left);
    let crop_height = bottom.min(image_height).saturating_sub(top);

    if crop_width == 0 || crop_height == 0 {
        return Err("PDF crop region is smaller than one rendered pixel".to_string());
    }

    Ok((left, top, crop_width, crop_height))
}

#[derive(Clone, Copy, Debug)]
pub struct NormalizedPdfRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub enum PdfPageEdit {
    Crop(NormalizedPdfRegion),
    Erase(NormalizedPdfRegion),
    Rotate,
}

fn rotate_pdf_edit_image(image: DynamicImage, degrees: f32) -> Result<RgbaImage, String> {
    if !degrees.is_finite() {
        return Err("PDF rotation degrees must be finite".to_string());
    }

    let normalized = degrees.rem_euclid(360.0);
    if normalized.abs() < f32::EPSILON || (360.0 - normalized).abs() < f32::EPSILON {
        return Ok(image.to_rgba8());
    }
    if (normalized - 90.0).abs() < f32::EPSILON {
        return Ok(image.rotate90().to_rgba8());
    }
    if (normalized - 180.0).abs() < f32::EPSILON {
        return Ok(image.rotate180().to_rgba8());
    }
    if (normalized - 270.0).abs() < f32::EPSILON {
        return Ok(image.rotate270().to_rgba8());
    }

    let source = image.to_rgba8();
    let (source_width, source_height) = source.dimensions();
    let radians = degrees.to_radians();
    let sin = radians.sin().abs();
    let cos = radians.cos().abs();
    let expanded_width = ((source_width as f32 * cos) + (source_height as f32 * sin)).ceil() as u32;
    let expanded_height =
        ((source_width as f32 * sin) + (source_height as f32 * cos)).ceil() as u32;
    let background = Rgba([255, 255, 255, 255]);
    let mut canvas =
        RgbaImage::from_pixel(expanded_width.max(1), expanded_height.max(1), background);
    let offset_x = i64::from((canvas.width() - source_width) / 2);
    let offset_y = i64::from((canvas.height() - source_height) / 2);
    image::imageops::overlay(&mut canvas, &source, offset_x, offset_y);

    Ok(rotate_about_center(
        &canvas,
        radians,
        Interpolation::Bilinear,
        background,
    ))
}

pub fn rotate_pdf_page_quarter_turns_to_bytes(
    bytes: &[u8],
    page_index: usize,
    degrees: i32,
) -> Result<Vec<u8>, String> {
    let mut document = load_lopdf_document(bytes, "rotating")?;
    let pages = document.get_pages();
    let page_id = pages.values().nth(page_index).copied().ok_or_else(|| {
        format!(
            "Page index {page_index} out of bounds (PDF has {} pages)",
            pages.len()
        )
    })?;
    let mut current_id = page_id;
    let mut visited = std::collections::HashSet::new();
    let current_rotation = loop {
        if !visited.insert(current_id) {
            break 0;
        }
        let dictionary = document
            .get_dictionary(current_id)
            .map_err(|error| format!("Failed to resolve PDF page rotation: {error}"))?;
        if let Ok(rotation) = dictionary.get(b"Rotate").and_then(lopdf::Object::as_i64) {
            break rotation;
        }
        match dictionary
            .get(b"Parent")
            .and_then(lopdf::Object::as_reference)
        {
            Ok(parent_id) if parent_id != current_id => current_id = parent_id,
            _ => break 0,
        }
    };
    let page = document
        .get_object_mut(page_id)
        .map_err(|error| format!("Failed to load PDF page for rotation: {error}"))?
        .as_dict_mut()
        .map_err(|error| format!("Failed to access PDF page dictionary: {error}"))?;
    page.set(
        "Rotate",
        lopdf::Object::Integer((current_rotation + i64::from(degrees)).rem_euclid(360)),
    );

    let mut output = Vec::new();
    document
        .save_to(&mut output)
        .map_err(|error| format!("Failed to save rotated PDF: {error}"))?;
    Ok(output)
}

fn erase_pdf_image_region(
    image: &mut RgbaImage,
    region: NormalizedPdfRegion,
) -> Result<(), String> {
    let (left, top, width, height) = normalized_crop_bounds(
        image.width(),
        image.height(),
        region.x,
        region.y,
        region.width,
        region.height,
    )?;
    for row in top..top + height {
        for column in left..left + width {
            image.put_pixel(column, row, Rgba([255, 255, 255, 255]));
        }
    }
    Ok(())
}

fn apply_pdf_page_edit(
    rendered: DynamicImage,
    rotation_degrees: f32,
    existing_crop: Option<NormalizedPdfRegion>,
    existing_erasures: &[NormalizedPdfRegion],
    edit: PdfPageEdit,
) -> Result<RgbaImage, String> {
    let mut source = rendered.to_rgba8();
    for erasure in existing_erasures {
        erase_pdf_image_region(&mut source, *erasure)?;
    }

    if let Some(region) = existing_crop {
        let (left, top, width, height) = normalized_crop_bounds(
            source.width(),
            source.height(),
            region.x,
            region.y,
            region.width,
            region.height,
        )?;
        source = image::imageops::crop_imm(&source, left, top, width, height).to_image();
    }

    let mut edited = rotate_pdf_edit_image(DynamicImage::ImageRgba8(source), rotation_degrees)?;
    if let PdfPageEdit::Erase(region) = edit {
        erase_pdf_image_region(&mut edited, region)?;
    }
    if let PdfPageEdit::Crop(region) = edit {
        let (left, top, width, height) = normalized_crop_bounds(
            edited.width(),
            edited.height(),
            region.x,
            region.y,
            region.width,
            region.height,
        )?;
        edited = image::imageops::crop_imm(&edited, left, top, width, height).to_image();
    }

    Ok(edited)
}

/// Materialize the current PDF viewport and one edit as a standalone PDF page.
/// Rotation and prior erasures are baked into the pixels before the new edit,
/// matching the versioned image-edit pipeline used by the frontend history.
pub fn edit_pdf_page_to_single_page_bytes(
    bytes: &[u8],
    page_index: usize,
    rotation_degrees: f32,
    existing_crop: Option<NormalizedPdfRegion>,
    existing_erasures: &[NormalizedPdfRegion],
    edit: PdfPageEdit,
) -> Result<Vec<u8>, String> {
    let (rendered, page_width_pt, page_height_pt) = {
        let pdfium = get_pdfium()?;
        let document = pdfium
            .load_pdf_from_byte_slice(bytes, None)
            .map_err(|e| format!("Failed to load PDF for editing: {e}"))?;
        let pages = document.pages();
        let page_count: usize = pages.len().into();
        if page_index >= page_count {
            return Err(format!(
                "Page index {page_index} out of bounds (PDF has {page_count} pages)"
            ));
        }
        let page = pages
            .get(PdfPageIndex::from(page_index as u16))
            .map_err(|e| format!("Failed to get page {page_index} from PDF: {e}"))?;
        let rotation = page.rotation().unwrap_or(PdfPageRenderRotation::None);
        let (page_width, page_height) = match rotation {
            PdfPageRenderRotation::Degrees90 | PdfPageRenderRotation::Degrees270 => {
                (page.height().value, page.width().value)
            }
            _ => (page.width().value, page.height().value),
        };
        (
            render_pdf_page_image(&page, page_index, false)?,
            page_width,
            page_height,
        )
    };

    let source_width = rendered.width();
    let source_height = rendered.height();
    let edited = apply_pdf_page_edit(
        rendered,
        rotation_degrees,
        existing_crop,
        existing_erasures,
        edit,
    )?;

    let points_per_pixel_x = page_width_pt / source_width as f32;
    let points_per_pixel_y = page_height_pt / source_height as f32;
    let points_per_pixel = (points_per_pixel_x + points_per_pixel_y) / 2.0;
    let output_width = PdfPoints::new(edited.width() as f32 * points_per_pixel);
    let output_height = PdfPoints::new(edited.height() as f32 * points_per_pixel);

    let pdfium = get_pdfium()?;
    let mut derived = pdfium
        .create_new_pdf()
        .map_err(|e| format!("Failed to create edited PDF: {e}"))?;
    {
        let mut page = derived
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::new_custom(output_width, output_height))
            .map_err(|e| format!("Failed to create edited PDF page: {e}"))?;
        page.objects_mut()
            .create_image_object(
                PdfPoints::new(0.0),
                PdfPoints::new(0.0),
                &DynamicImage::ImageRgba8(edited),
                Some(output_width),
                Some(output_height),
            )
            .map_err(|e| format!("Failed to embed edited PDF page image: {e}"))?;
    }

    derived
        .save_to_bytes()
        .map_err(|e| format!("Failed to save edited PDF: {e}"))
}

/// Materialize one normalized page region as a standalone image-backed PDF.
///
/// The derived page intentionally has no inherited text layer. A CropBox-only
/// edit can leave out-of-crop text visible to native PDF extraction, while this
/// representation guarantees that every OCR provider sees only the crop.
///
/// The derived page keeps the source page's point-per-pixel mapping: its size
/// is the crop region scaled by `crop_pixels / rendered_pixels` against the
/// source page's size in points. A hardcoded DPI would shrink non-letter pages
/// (the crop render is a fixed 2550px wide, so the effective DPI varies with
/// the page size), leaving the crop visually reduced inside a canvas that no
/// longer matches the selected region — and misrepresenting its physical size
/// to downstream OCR renderers.
pub fn crop_pdf_to_single_page_bytes(
    bytes: &[u8],
    page_index: usize,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<Vec<u8>, String> {
    edit_pdf_page_to_single_page_bytes(
        bytes,
        page_index,
        0.0,
        None,
        &[],
        PdfPageEdit::Crop(NormalizedPdfRegion {
            x,
            y,
            width,
            height,
        }),
    )
}

/// Render a single PDF page to PNG bytes, suitable for OCR processing.
///
/// Uses `pdfium-render` to rasterize the page at 300 DPI equivalent
/// (target width ~2550px for letter-size). Returns raw PNG bytes that
/// can be fed directly to `OcrProvider::recognize()`.
///
/// # Arguments
/// * `bytes` — Raw PDF file bytes
/// * `page_index` — Zero-based page index (0 = first page)
///
/// # Errors
/// Returns `Err` if:
/// - Pdfium fails to initialize
/// - PDF cannot be loaded
/// - Page index is out of bounds
/// - Rendering or encoding fails
pub fn render_pdf_page_to_image(bytes: &[u8], page_index: usize) -> Result<Vec<u8>, String> {
    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| format!("Failed to load PDF: {e}"))?;

    let pages = document.pages();
    let page_count: usize = pages.len().into();

    if page_index >= page_count {
        return Err(format!(
            "Page index {page_index} out of bounds (PDF has {page_count} pages)"
        ));
    }

    let page_idx: PdfPageIndex = PdfPageIndex::from(page_index as u16);
    let page = pages
        .get(page_idx)
        .map_err(|e| format!("Failed to get page {page_index} from PDF: {e}"))?;

    render_pdf_page(&page, page_index)
}

/// Visit all PDF pages from one loaded Pdfium document.
///
/// Pdfium documents retain the source bytes internally, so this must complete
/// before the document is dropped. Pdfium work is blocking — call from a
/// blocking-safe context. The explicit Pdfium/document load intentionally
/// stays outside the shared traversal so only one document is loaded.
pub fn render_pdf_pages_with<V>(bytes: &[u8], visitor: V) -> Result<usize, String>
where
    V: FnMut(usize, usize, &[u8]) -> Result<(), String>,
{
    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| format!("Failed to load PDF: {e}"))?;
    let mut source = PdfiumPageSource {
        pages: document.pages(),
    };

    visit_rendered_pages(&mut source, visitor)
}

fn render_pdf_page(page: &PdfPage<'_>, page_index: usize) -> Result<Vec<u8>, String> {
    let image = render_pdf_page_image(page, page_index, true)?;
    encode_png_with_max_size(&image, MAX_RENDERED_PAGE_IMAGE_BYTES)
}

fn render_pdf_page_image(
    page: &PdfPage<'_>,
    page_index: usize,
    rotate_landscape: bool,
) -> Result<DynamicImage, String> {
    // Render at 300 DPI equivalent. A typical letter-size page is 8.5" × 11"
    // which at 300 DPI gives 2550 × 3300 pixels.
    let mut render_config = PdfRenderConfig::new().set_target_width(2550);
    if rotate_landscape {
        render_config = render_config.rotate_if_landscape(PdfPageRenderRotation::Degrees90, true);
    }
    let bitmap = page
        .render_with_config(&render_config)
        .map_err(|e| format!("Failed to render PDF page {page_index}: {e}"))?;

    Ok(bitmap.as_image())
}

fn encode_png_with_max_size(image: &DynamicImage, max_size: usize) -> Result<Vec<u8>, String> {
    let mut candidate = image.clone();

    loop {
        let mut png_bytes = Vec::new();
        candidate
            .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
            .map_err(|e| format!("Failed to encode rendered page as PNG: {e}"))?;

        if png_bytes.len() <= max_size {
            return Ok(png_bytes);
        }

        let (width, height) = candidate.dimensions();
        if width == 1 && height == 1 {
            return Err(format!(
                "Rendered PDF page image exceeds the {max_size} byte limit even at 1x1 pixels"
            ));
        }

        candidate = candidate.resize(
            (width / 2).max(1),
            (height / 2).max(1),
            image::imageops::FilterType::Triangle,
        );
    }
}

/// Render the first page of a PDF to PNG bytes at thumbnail resolution (400px wide).
///
/// Intended for collection-view card previews. The output is a compact PNG
/// suitable for use as an `<img>` src via `convertFileSrc`.
///
/// Uses `pdfium-render` with a target width of 400px (roughly 50 DPI equivalent),
/// yielding small files that load fast in the UI.
pub fn render_pdf_thumbnail(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.is_empty() {
        return Err("PDF bytes are empty".to_string());
    }

    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| format!("Failed to load PDF for thumbnail: {e}"))?;

    let pages = document.pages();
    if pages.is_empty() {
        return Err("PDF has no pages".to_string());
    }

    let page = pages
        .get(PdfPageIndex::from(0u16))
        .map_err(|e| format!("Failed to get first page from PDF: {e}"))?;

    let render_config = PdfRenderConfig::new()
        .set_target_width(400)
        .rotate_if_landscape(PdfPageRenderRotation::Degrees90, true);

    let bitmap = page
        .render_with_config(&render_config)
        .map_err(|e| format!("Failed to render PDF thumbnail: {e}"))?;

    let dynamic_image = bitmap.as_image();

    let mut png_bytes = Vec::new();
    dynamic_image
        .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode thumbnail as PNG: {e}"))?;

    Ok(png_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Document, Object};

    fn two_page_pdf_bytes() -> Vec<u8> {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let page_ids = (0..2)
            .map(|index| {
                document.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), (595 + index).into(), 842.into()],
                })
            })
            .collect::<Vec<_>>();
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
                "Count" => 2,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("serialize fixture PDF");
        bytes
    }

    fn pdf_with_invalid_prev() -> Vec<u8> {
        let mut document = Document::load_mem(&two_page_pdf_bytes()).expect("parse fixture PDF");
        document.trailer.set("Prev", 9_999_999);
        let mut bytes = Vec::new();
        document
            .save_to(&mut bytes)
            .expect("serialize invalid Prev fixture");
        bytes
    }

    #[test]
    fn tolerates_an_invalid_prev_pointer_when_the_main_xref_is_readable() {
        let source = pdf_with_invalid_prev();

        assert_eq!(pdf_page_count(&source).expect("recover page count"), 2);
        assert_eq!(
            split_pdf_to_single_page_bytes(&source)
                .expect("recover split")
                .len(),
            2
        );
    }

    #[test]
    fn splits_each_pdf_page_without_a_native_pdfium_library() {
        let source = two_page_pdf_bytes();
        assert_eq!(pdf_page_count(&source).expect("source page count"), 2);
        let pages = split_pdf_to_single_page_bytes(&source).expect("split PDF");

        assert_eq!(pages.len(), 2);
        for (expected_page, (page_number, bytes)) in pages.iter().enumerate() {
            assert_eq!(*page_number, expected_page as u32 + 1);
            let page = Document::load_mem(bytes).expect("parse split page");
            assert_eq!(page.get_pages().len(), 1);
            assert_eq!(pdf_page_count(bytes).expect("split page count"), 1);
            let page_id = *page.get_pages().values().next().expect("page id");
            let media_box = page
                .get_dictionary(page_id)
                .expect("page dictionary")
                .get(b"MediaBox")
                .expect("media box")
                .as_array()
                .expect("media box array");
            assert_eq!(media_box[2], Object::Integer(595 + expected_page as i64));
        }
    }
    use crate::runtime::status::{RuntimeCapability, RuntimeState, RuntimeStatus};
    use image::{Rgba, RgbaImage};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn use_dev_pdfium() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("lib")
            .join(Pdfium::pdfium_platform_library_name());
        let cache = PDFIUM_PATH.get_or_init(|| Mutex::new(None));
        *cache.lock().expect("pdfium path cache") = Some(path);
    }

    fn one_page_pdf_bytes(width_pt: i64, height_pt: i64) -> Vec<u8> {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), width_pt.into(), height_pt.into()],
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(page_id)],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("serialize fixture PDF");
        bytes
    }

    fn media_box_size(cropped_bytes: &[u8]) -> (f32, f32) {
        let out = Document::load_mem(cropped_bytes).expect("parse cropped PDF");
        assert_eq!(out.get_pages().len(), 1, "derived PDF must have one page");
        let page_id = *out.get_pages().values().next().expect("page id");
        let media_box = out
            .get_dictionary(page_id)
            .expect("page dict")
            .get(b"MediaBox")
            .expect("media box")
            .as_array()
            .expect("media box array");
        let values = media_box
            .iter()
            .map(|o| {
                o.as_float()
                    .unwrap_or_else(|_| o.as_i64().unwrap_or(0) as f32)
            })
            .collect::<Vec<_>>();
        (values[2], values[3])
    }

    #[test]
    fn cropped_pdf_page_keeps_source_page_scale_for_any_page_size() {
        use pdfium_render::prelude::{PdfRenderConfig, Pdfium};

        let dll = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("lib")
            .join(Pdfium::pdfium_platform_library_name());
        if !dll.exists() {
            eprintln!("[pdf] pdfium native library not available — skipping crop geometry test");
            return;
        }
        use_dev_pdfium();

        // Newspaper-sized source pages (e.g. 17in x 22in at 72pt/in). The crop
        // render is a fixed 2550px wide, so a hardcoded 300 DPI would shrink the
        // derived page below the real crop size; the derived page must instead
        // keep the source page's point-per-pixel mapping so the crop becomes the
        // whole new canvas at the same visual scale.
        for (source_w, source_h) in [(595, 842), (1224, 1584), (850, 1150)] {
            let source = one_page_pdf_bytes(source_w, source_h);
            let cropped_bytes =
                crop_pdf_to_single_page_bytes(&source, 0, 0.25, 0.25, 0.5, 0.5).expect("crop PDF");
            let (derived_w, derived_h) = media_box_size(&cropped_bytes);

            let render_h = (2550.0 * source_h as f32 / source_w as f32).round() as u32;
            let crop_w_px = ((0.75f64 * 2550.0).ceil() - (0.25f64 * 2550.0).floor()) as f32;
            let crop_h_px =
                ((0.75f64 * render_h as f64).ceil() - (0.25f64 * render_h as f64).floor()) as f32;
            let expected_w = crop_w_px / 2550.0 * source_w as f32;
            let expected_h = crop_h_px / render_h as f32 * source_h as f32;

            assert!(
                (derived_w - expected_w).abs() < 2.0,
                "derived width {derived_w}pt must match the crop region at source scale {expected_w}pt (source {source_w}x{source_h}pt)"
            );
            assert!(
                (derived_h - expected_h).abs() < 2.0,
                "derived height {derived_h}pt must match the crop region at source scale {expected_h}pt (source {source_w}x{source_h}pt)"
            );

            // Re-rendering the derived page for OCR must never lose resolution:
            // at the 2550px target the render is >= the crop's native pixels and
            // preserves the derived page's aspect ratio.
            let pdfium = Pdfium::new(Pdfium::bind_to_library(&dll).expect("bind pdfium"));
            let crop_doc = pdfium
                .load_pdf_from_byte_slice(&cropped_bytes, None)
                .expect("load cropped");
            let crop_page = crop_doc.pages().get(0).expect("crop page");
            let crop_render = crop_page
                .render_with_config(&PdfRenderConfig::new().set_target_width(2550i32))
                .expect("render crop");
            assert_eq!(crop_render.width(), 2550);
            assert!(
                (crop_render.width() as f32 / crop_render.height() as f32 - derived_w / derived_h)
                    .abs()
                    < 0.02,
                "derived render aspect must match its MediaBox"
            );
        }
    }
    use tempfile::tempdir;

    #[test]
    fn resolve_pdfium_prefers_managed_runtime_lib_dir() {
        let runtime_dir = tempdir().expect("runtime dir");
        let manifest_dir = tempdir().expect("manifest dir");
        let managed_dll = runtime_dir
            .path()
            .join("resources")
            .join("lib")
            .join(Pdfium::pdfium_platform_library_name());
        std::fs::create_dir_all(managed_dll.parent().expect("lib parent")).expect("create lib dir");
        std::fs::write(&managed_dll, b"pdfium").expect("write dll");

        let resolved =
            resolve_pdfium_dll_path_from_roots(Some(runtime_dir.path()), manifest_dir.path());

        assert_eq!(resolved, Some(managed_dll));
    }

    #[test]
    fn resolve_pdfium_finds_linux_arch_specific_dev_resource() {
        let manifest_dir = tempdir().expect("manifest dir");
        let arch_specific = manifest_dir
            .path()
            .join("resources")
            .join("lib")
            .join("linux-x86_64")
            .join(Pdfium::pdfium_platform_library_name());
        std::fs::create_dir_all(arch_specific.parent().expect("parent")).expect("mkdir");
        std::fs::write(&arch_specific, b"pdfium").expect("write");

        let resolved = resolve_pdfium_dll_path_from_roots(None, manifest_dir.path());

        #[cfg(target_os = "linux")]
        assert_eq!(resolved, Some(arch_specific));
        #[cfg(not(target_os = "linux"))]
        assert_eq!(resolved, None);
    }

    #[test]
    fn resolve_pdfium_finds_runtime_pack_dev_resource_on_linux() {
        let manifest_dir = tempdir().expect("manifest dir");
        let runtime_pack = manifest_dir
            .path()
            .join("resources")
            .join("runtime-pack")
            .join("linux-x86_64")
            .join("resources")
            .join("lib")
            .join(Pdfium::pdfium_platform_library_name());
        std::fs::create_dir_all(runtime_pack.parent().expect("parent")).expect("mkdir");
        std::fs::write(&runtime_pack, b"pdfium").expect("write");

        let resolved = resolve_pdfium_dll_path_from_roots(None, manifest_dir.path());

        #[cfg(target_os = "linux")]
        assert_eq!(resolved, Some(runtime_pack));
        #[cfg(not(target_os = "linux"))]
        assert_eq!(resolved, None);
    }

    #[test]
    fn empty_text_is_not_quality() {
        assert!(!is_quality_text(""));
    }

    #[test]
    fn short_garbled_text_is_not_quality() {
        let garbled = "!@#$%^&*()_+-=[]{}|;':\",./<>? abc 123";
        assert!(!is_quality_text(garbled));
    }

    #[test]
    fn normal_text_is_quality() {
        let text = "This is a perfectly normal paragraph of text that contains well over fifty alphanumeric characters and should pass the quality heuristic with ease.";
        assert!(is_quality_text(text));
    }

    /// get_pdfium() must never panic — it should return Err when the native
    /// library is unavailable. This test runs in CI where pdfium.dll is often
    /// absent, so it exercises the unhappy path.
    #[test]
    fn get_pdfium_returns_error_without_native_library() {
        // If pdfium is installed, this will succeed — that's fine, we only
        // assert that it doesn't panic. If it's not installed, it must return Err.
        let result = get_pdfium();
        // Either outcome is acceptable; the important thing is NO PANIC.
        // When the library is missing, the error message must mention Pdfium.
        if let Err(msg) = &result {
            assert!(
                msg.contains("Pdfium") || msg.contains("pdfium"),
                "Error message should reference the Pdfium library, got: {msg}"
            );
        }
    }

    /// pdf_page_count requires the pdfium native library which may not be
    /// available in unit test environments. Marked as ignored.
    #[test]
    #[ignore]
    fn pdf_page_count_invalid_bytes() {
        // Invalid PDF bytes should return an error, not panic
        let result = pdf_page_count(b"not a pdf");
        assert!(result.is_err(), "Expected error for invalid PDF bytes");
    }

    /// render_pdf_thumbnail requires the pdfium native library which may not be
    /// available in unit test environments. Marked as ignored.
    #[test]
    #[ignore]
    fn render_pdf_thumbnail_invalid_bytes() {
        // Invalid PDF bytes should return an error, not panic
        let result = render_pdf_thumbnail(b"not a pdf");
        assert!(
            result.is_err(),
            "Expected error for invalid PDF bytes in thumbnail"
        );
    }

    #[test]
    fn render_pdf_thumbnail_empty_bytes() {
        // Empty bytes should return an error (no pdfium needed for this check)
        let result = render_pdf_thumbnail(b"");
        assert!(result.is_err(), "Expected error for empty PDF bytes");
    }

    #[test]
    fn test_strip_windows_prefix() {
        // No prefix — should return unchanged
        let path = PathBuf::from(r"C:\Users\test\file.dll");
        assert_eq!(strip_windows_prefix(path.clone()), path);

        // With prefix — should strip it
        let prefixed = PathBuf::from(r"\\?\C:\Users\test\file.dll");
        let stripped = strip_windows_prefix(prefixed);
        assert_eq!(stripped, PathBuf::from(r"C:\Users\test\file.dll"));

        // Empty path — should be fine
        let empty = PathBuf::from("");
        assert_eq!(strip_windows_prefix(empty.clone()), empty);
    }

    #[test]
    fn test_dll_name_display() {
        // Just verify it returns a non-empty string
        let name = dll_name_display();
        assert!(
            !name.is_empty(),
            "dll_name_display should return a non-empty string"
        );
        assert!(
            name.contains("pdfium") || name.contains("Pdfium"),
            "dll_name_display should contain 'pdfium', got: {name}"
        );
    }

    #[test]
    fn pdfium_runtime_resolution_bootstraps_before_managed_lib_lookup() {
        let calls = RefCell::new(Vec::new());
        let expected = PathBuf::from("/tmp/runtime-ready");

        let resolved = managed_runtime_root_for_pdfium_with(
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
    fn pdfium_runtime_resolution_respects_blocked_bootstrap_status() {
        let calls = RefCell::new(Vec::new());

        let resolved = managed_runtime_root_for_pdfium_with(
            || {
                calls.borrow_mut().push("ensure_ready");
                Ok(RuntimeStatus {
                    state: RuntimeState::BlockedOffline,
                    pack_version: Some("2026.05.0".to_string()),
                    repair_needed: false,
                    repair_available: false,
                    summary: "Bootstrap offline".to_string(),
                    blocked_capabilities: vec![RuntimeCapability::Ocr],
                    details: vec!["offline".to_string()],
                    guidance: vec!["Reintentá".to_string()],
                    bootstrap_eligible: true,
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

    #[test]
    fn page_image_encoding_downscales_to_the_requested_limit() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(128, 128, |x, y| {
            let value = ((x.wrapping_mul(31) ^ y.wrapping_mul(17)) & 0xff) as u8;
            Rgba([value, value.wrapping_add(79), value.wrapping_add(151), 255])
        }));

        let encoded = encode_png_with_max_size(&image, 1024).expect("bounded PNG");
        let decoded = image::load_from_memory(&encoded).expect("decode bounded PNG");

        assert!(encoded.len() <= 1024);
        assert!(decoded.width() < image.width());
        assert_eq!(decoded.width() * 128, decoded.height() * 128);
    }

    #[test]
    fn page_image_encoding_fails_when_no_valid_size_exists() {
        let image = DynamicImage::ImageRgba8(RgbaImage::new(1, 1));

        let error = encode_png_with_max_size(&image, 0).expect_err("zero-byte limit must fail");

        assert!(error.contains("0 byte limit even at 1x1 pixels"));
    }

    #[test]
    fn normalized_crop_bounds_rebase_the_visible_region_to_its_own_dimensions() {
        assert_eq!(
            normalized_crop_bounds(1000, 2000, 0.2, 0.25, 0.5, 0.4).expect("valid crop"),
            (200, 500, 500, 800)
        );
    }

    #[test]
    fn normalized_crop_bounds_reject_regions_outside_the_page() {
        let error = normalized_crop_bounds(1000, 2000, 0.8, 0.1, 0.3, 0.5)
            .expect_err("out-of-page crop must fail");

        assert!(error.contains("normalized region"));
    }

    #[test]
    fn pdf_edit_rotation_uses_the_viewport_orientation() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 2, Rgba([10, 20, 30, 255])));

        let rotated = rotate_pdf_edit_image(image, 90.0).expect("quarter turn");

        assert_eq!(rotated.dimensions(), (2, 4));
    }

    #[test]
    fn quarter_turn_pdf_rotation_preserves_the_page_dictionary() {
        let source = one_page_pdf_bytes(595, 842);

        let rotated =
            rotate_pdf_page_quarter_turns_to_bytes(&source, 0, 450).expect("lossless quarter turn");
        let document = Document::load_mem(&rotated).expect("parse rotated PDF");
        let page_id = *document.get_pages().values().next().expect("page id");
        let page = document.get_dictionary(page_id).expect("page dictionary");

        assert_eq!(page.get(b"Rotate").expect("rotation"), &Object::Integer(90));
        assert_eq!(
            page.get(b"MediaBox").expect("media box"),
            &Object::Array(vec![0.into(), 0.into(), 595.into(), 842.into()])
        );
    }

    #[test]
    fn quarter_turn_pdf_rotation_composes_with_inherited_rotation() {
        let mut source = Document::load_mem(&one_page_pdf_bytes(595, 842)).expect("parse PDF");
        let page_id = *source.get_pages().values().next().expect("page id");
        let parent_id = source
            .get_dictionary(page_id)
            .expect("page dictionary")
            .get(b"Parent")
            .and_then(Object::as_reference)
            .expect("page parent");
        source
            .get_object_mut(parent_id)
            .expect("pages object")
            .as_dict_mut()
            .expect("pages dictionary")
            .set("Rotate", Object::Integer(90));
        let mut source_bytes = Vec::new();
        source.save_to(&mut source_bytes).expect("serialize PDF");

        let rotated = rotate_pdf_page_quarter_turns_to_bytes(&source_bytes, 0, 90)
            .expect("composed quarter turn");
        let document = Document::load_mem(&rotated).expect("parse rotated PDF");
        let page_id = *document.get_pages().values().next().expect("page id");

        assert_eq!(
            document
                .get_dictionary(page_id)
                .expect("page dictionary")
                .get(b"Rotate")
                .expect("page rotation"),
            &Object::Integer(180)
        );
    }

    #[test]
    fn pdf_edit_erasure_uses_normalized_viewport_coordinates() {
        let mut image = RgbaImage::from_pixel(4, 4, Rgba([10, 20, 30, 255]));

        erase_pdf_image_region(
            &mut image,
            NormalizedPdfRegion {
                x: 0.25,
                y: 0.5,
                width: 0.5,
                height: 0.25,
            },
        )
        .expect("erase region");

        assert_eq!(*image.get_pixel(0, 2), Rgba([10, 20, 30, 255]));
        assert_eq!(*image.get_pixel(1, 2), Rgba([255, 255, 255, 255]));
        assert_eq!(*image.get_pixel(2, 2), Rgba([255, 255, 255, 255]));
        assert_eq!(*image.get_pixel(3, 2), Rgba([10, 20, 30, 255]));
    }

    #[test]
    fn pdf_edit_composes_existing_crop_rotation_and_new_viewport_crop() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 4, Rgba([10, 20, 30, 255])));

        let edited = apply_pdf_page_edit(
            image,
            90.0,
            Some(NormalizedPdfRegion {
                x: 0.25,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            }),
            &[],
            PdfPageEdit::Crop(NormalizedPdfRegion {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            }),
        )
        .expect("composed PDF edit");

        assert_eq!(edited.dimensions(), (2, 4));
    }

    #[test]
    fn rendered_page_image_limit_is_ten_decimal_megabytes() {
        assert_eq!(MAX_RENDERED_PAGE_IMAGE_BYTES, 10_000_000);
    }

    struct FakeRenderedPageSource {
        pages: Vec<Vec<u8>>,
        count_calls: usize,
        render_calls: Vec<usize>,
        completed_pages: Rc<RefCell<Vec<usize>>>,
    }

    impl FakeRenderedPageSource {
        fn new(pages: Vec<Vec<u8>>) -> Self {
            Self {
                pages,
                count_calls: 0,
                render_calls: Vec::new(),
                completed_pages: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl RenderedPageSource for FakeRenderedPageSource {
        fn page_count(&mut self) -> Result<usize, String> {
            self.count_calls += 1;
            Ok(self.pages.len())
        }

        fn render_page(&mut self, index: usize) -> Result<Vec<u8>, String> {
            if index > 0 && self.completed_pages.borrow().last() != Some(&(index - 1)) {
                return Err(format!(
                    "page {} rendered before page {} was consumed",
                    index,
                    index - 1
                ));
            }

            self.render_calls.push(index);
            Ok(self.pages[index].clone())
        }
    }

    #[test]
    fn visit_rendered_pages_counts_once_and_consumes_each_page_before_rendering_the_next() {
        let mut source = FakeRenderedPageSource::new(vec![vec![0], vec![1], vec![2]]);
        let mut observed = Vec::new();
        let completed_pages = Rc::clone(&source.completed_pages);

        let count = visit_rendered_pages(&mut source, |index, page_count, png| {
            observed.push((index, page_count, png[0]));
            completed_pages.borrow_mut().push(index);
            Ok(())
        })
        .expect("traversal succeeds");

        assert_eq!(count, 3);
        assert_eq!(source.count_calls, 1);
        assert_eq!(source.render_calls, vec![0, 1, 2]);
        assert_eq!(observed, vec![(0, 3, 0), (1, 3, 1), (2, 3, 2)]);
    }

    #[test]
    fn visit_rendered_pages_returns_zero_without_invoking_the_visitor_for_an_empty_source() {
        let mut source = FakeRenderedPageSource::new(vec![]);
        let mut visitor_calls = 0;

        let count = visit_rendered_pages(&mut source, |_, _, _| {
            visitor_calls += 1;
            Ok(())
        })
        .expect("empty traversal succeeds");

        assert_eq!(count, 0);
        assert_eq!(source.count_calls, 1);
        assert!(source.render_calls.is_empty());
        assert_eq!(visitor_calls, 0);
    }

    #[test]
    fn visit_rendered_pages_stops_before_rendering_later_pages_when_the_visitor_fails() {
        let mut source = FakeRenderedPageSource::new(vec![vec![0], vec![1], vec![2]]);
        let completed_pages = Rc::clone(&source.completed_pages);

        let error = visit_rendered_pages(&mut source, |index, _, _| {
            completed_pages.borrow_mut().push(index);
            if index == 1 {
                Err("visitor failed at page 2".to_string())
            } else {
                Ok(())
            }
        })
        .expect_err("visitor error is returned");

        assert_eq!(error, "visitor failed at page 2");
        assert_eq!(source.render_calls, vec![0, 1]);
    }

    #[test]
    fn visit_rendered_pages_delivers_a_payload_at_the_exact_byte_limit() {
        let mut source = FakeRenderedPageSource::new(vec![vec![7; MAX_RENDERED_PAGE_IMAGE_BYTES]]);
        let mut delivered_len = 0;

        let count = visit_rendered_pages(&mut source, |_, _, png| {
            delivered_len = png.len();
            Ok(())
        })
        .expect("boundary payload is delivered");

        assert_eq!(count, 1);
        assert_eq!(delivered_len, MAX_RENDERED_PAGE_IMAGE_BYTES);
    }

    #[test]
    fn visit_rendered_pages_rejects_an_oversized_payload_before_invoking_the_visitor() {
        let mut source =
            FakeRenderedPageSource::new(vec![vec![7; MAX_RENDERED_PAGE_IMAGE_BYTES + 1]]);
        let mut visitor_calls = 0;

        let error = visit_rendered_pages(&mut source, |_, _, _| {
            visitor_calls += 1;
            Ok(())
        })
        .expect_err("oversized payload is rejected");

        assert_eq!(
            error,
            format!(
                "Rendered PDF page 1 image exceeds the {MAX_RENDERED_PAGE_IMAGE_BYTES} byte limit"
            )
        );
        assert_eq!(visitor_calls, 0);
    }

    type RenderPdfPagesWithSignature =
        fn(&[u8], fn(usize, usize, &[u8]) -> Result<(), String>) -> Result<usize, String>;

    #[test]
    fn render_pdf_pages_with_exposes_the_borrowed_visitor_wrapper() {
        let _: RenderPdfPagesWithSignature = render_pdf_pages_with;
    }
}
