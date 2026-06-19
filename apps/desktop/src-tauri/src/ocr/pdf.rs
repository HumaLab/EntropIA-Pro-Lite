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
/// Used by the multi-page OCR pipeline to know how many pages to process.
pub fn pdf_page_count(bytes: &[u8]) -> Result<usize, String> {
    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| format!("Failed to load PDF for page count: {e}"))?;
    Ok(document.pages().len().into())
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
            "Page index {} out of bounds (PDF has {} pages)",
            page_index, page_count
        ));
    }

    let page_idx: PdfPageIndex = PdfPageIndex::from(page_index as u16);
    let page = pages
        .get(page_idx)
        .map_err(|e| format!("Failed to get page {page_index} from PDF: {e}"))?;

    // Render at 300 DPI equivalent. A typical letter-size page is 8.5" × 11"
    // which at 300 DPI gives 2550 × 3300 pixels.
    let render_config = PdfRenderConfig::new()
        .set_target_width(2550)
        .rotate_if_landscape(PdfPageRenderRotation::Degrees90, true);

    let bitmap = page
        .render_with_config(&render_config)
        .map_err(|e| format!("Failed to render PDF page {page_index}: {e}"))?;

    // Convert to image::DynamicImage, then encode as PNG
    let dynamic_image = bitmap.as_image();

    let mut png_bytes = Vec::new();
    dynamic_image
        .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode rendered page as PNG: {e}"))?;

    Ok(png_bytes)
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
    if pages.len() == 0 {
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
    use crate::runtime::status::{RuntimeCapability, RuntimeState, RuntimeStatus};
    use std::cell::RefCell;
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
}
