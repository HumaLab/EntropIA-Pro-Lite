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
//! Resolution order:
//! 1. **Managed runtime** (Pro, `local-ml`) — `<runtime>/resources/lib/`
//! 2. **Bundled with the app** (macOS and Linux) — `Contents/Frameworks/` in the
//!    .app, `resources/pdfium/` under the .deb's resource dir
//!    ([`bundled_pdfium_candidate_paths`])
//! 3. **Dev fallback** — `CARGO_MANIFEST_DIR/resources/lib/` (for development)
//! 4. **System library** — OS default search paths (`PATH`, `/usr/lib`, etc.)
//!
//! Call `init_pdfium_path()` once during app startup (from OCR worker or command
//! handler) to cache the resolved path. If never called, falls back to current
//! directory + system library (original pdfium-render behavior).
//!
//! The bibliography pipeline resolves differently: [`ensure_pdfium_path_without_runtime`]
//! fills the same cache from the bundled library only — never from the ML
//! runtime — so a background sync cannot trigger a runtime bootstrap
//! (JD4-B-001). Where nothing is bundled the caller logs it and reads with
//! lopdf. The page RENDER may additionally fall back to an already-hydrated
//! managed runtime copy of the library ([`ensure_pdfium_path_with_hydrated_runtime`],
//! JD6-B-001) — still without bootstrapping.
//!
//! The no-bootstrap guarantee covers **PDFium resolution only**. The Pro
//! local Paddle OCR path resolves its model directory through
//! `resolve_paddle_model_dir` → `managed_runtime_root_for_ocr`, which does
//! call `RuntimeManager::ensure_ready_or_bootstrap`: that is pre-existing
//! behavior and outside part A (JD6-A-002).

#[cfg(feature = "local-ml")]
use crate::runtime::{managed_resource_path, RuntimeManager};
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use imageproc::geometric_transformations::{rotate_about_center, Interpolation};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId};
use pdfium_render::prelude::*;
use std::collections::BTreeSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Cached resolved path to the Pdfium native library.
///
/// - `Some(Some(path))` = initialized with a resolved DLL path
/// - `Some(None)` = initialized, but DLL not found in bundled paths (use system library)
/// - `None` = not yet initialized (fall back to CWD + system library)
static PDFIUM_PATH: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

thread_local! {
    /// How many live Pdfium instances THIS thread holds right now. The
    /// `thread_safe` bindings hold one global, non-reentrant lock from bind to
    /// drop, so the batch lifecycle asserts on this: at most one instance at a
    /// time on the reading thread, and none alive while the selective OCR pass
    /// binds its own. The counter is thread-scoped on purpose (JD6-A-007): a
    /// concurrently running test binding its own instance must not move the
    /// assertions of another. Instances are created and dropped on the same
    /// thread (they hold the global lock across their whole life).
    static PDFIUM_INSTANCES_ALIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Successful binds on this thread — the seam that proves PDFium was
    /// actually used instead of silently falling back to lopdf (JD6-A-006).
    static PDFIUM_BIND_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Page numbers this thread asked PDFium to read text from. A page the
    /// bomb-safe bound already refused must never appear here (JD6-A-004).
    static PDFIUM_TEXT_PAGES_REQUESTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// One bound Pdfium, counted while alive. Derefs to [`Pdfium`]; the counter
/// drops when the instance (and the library lock it holds) goes.
pub(crate) struct GuardedPdfium {
    inner: Pdfium,
}

impl GuardedPdfium {
    fn new(inner: Pdfium) -> Self {
        PDFIUM_INSTANCES_ALIVE.with(|count| count.set(count.get() + 1));
        Self { inner }
    }
}

impl std::ops::Deref for GuardedPdfium {
    type Target = Pdfium;

    fn deref(&self) -> &Pdfium {
        &self.inner
    }
}

impl Drop for GuardedPdfium {
    fn drop(&mut self) {
        PDFIUM_INSTANCES_ALIVE.with(|count| count.set(count.get().saturating_sub(1)));
    }
}

/// Live Pdfium instances on this thread. A diagnostic seam: the bibliography
/// reader's batches and the OCR pass's page renders must never overlap. The
/// count is thread-scoped (JD6-A-007) so concurrently running tests cannot
/// move each other's assertions.
pub(crate) fn pdfium_instances_alive() -> usize {
    PDFIUM_INSTANCES_ALIVE.with(std::cell::Cell::get)
}

/// Successful binds on this thread (JD6-A-006): before/after deltas prove a
/// test exercised the real library instead of the lopdf fallback.
pub(crate) fn pdfium_bind_count() -> u64 {
    PDFIUM_BIND_COUNT.with(std::cell::Cell::get)
}

/// Page numbers this thread asked PDFium for (JD6-A-004): a page refused by
/// the bomb-safe bound must never be requested.
pub(crate) fn pdfium_text_pages_requested() -> u64 {
    PDFIUM_TEXT_PAGES_REQUESTED.with(std::cell::Cell::get)
}

/// Whether a Pdfium instance binds at all right now — false means every
/// reader falls back to lopdf.
pub(crate) fn pdfium_loads() -> bool {
    get_pdfium().is_ok()
}

/// One batch of per-page texts (1-based page numbers) through ONE Pdfium
/// instance: `None` marks the pages Pdfium itself could not read (the caller
/// falls back to lopdf for those). The instance — and the global lock it
/// holds — is released before this returns, so a long read neither
/// monopolizes the lock nor survives into `maybe_ocr_pages`, whose page
/// renders bind their own instance (the lock is not reentrant).
pub(crate) const PDFIUM_TEXT_BATCH_PAGES: usize = 20;

pub(crate) fn read_pdfium_page_texts(
    bytes: &[u8],
    page_numbers: &[u32],
) -> Result<Vec<(u32, Option<String>)>, String> {
    PDFIUM_TEXT_PAGES_REQUESTED.with(|count| count.set(count.get() + page_numbers.len() as u64));
    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| format!("Failed to load PDF for per-page text: {e}"))?;
    let pages = document.pages();
    let mut out = Vec::with_capacity(page_numbers.len());
    for &number in page_numbers {
        let index = number.saturating_sub(1);
        let text = if index > u32::from(u16::MAX) {
            None
        } else {
            match pages.get(PdfPageIndex::from(index as u16)) {
                // `all()` clips to the page rect and silently drops text a
                // producer placed outside it — lopdf reads that text, and the
                // reader must never lose what the fallback finds. The full
                // coordinate space keeps every character.
                Ok(page) => page.text().ok().map(|text| text.inside_rect(PdfRect::MAX)),
                Err(_) => None,
            }
        };
        out.push((number, text));
    }
    drop(document);
    drop(pdfium);
    Ok(out)
}

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
    let bundled_resource_dir = bundled_resource_dir(app_handle);
    let resolved = resolve_pdfium_dll_path_from_roots(
        runtime_root.as_deref(),
        bundled_resource_dir.as_deref(),
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

/// The installed app's resource directory, where every bundle carries its
/// Pdfium library. On Windows it is the exe's own directory; Tauri may report it
/// with the `\\?\` prefix, which the library loader does not need.
fn bundled_resource_dir(app_handle: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    app_handle
        .path()
        .resource_dir()
        .ok()
        .map(strip_windows_prefix)
}

fn resolve_pdfium_dll_path_from_roots(
    managed_root: Option<&std::path::Path>,
    bundled_resource_dir: Option<&std::path::Path>,
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

    // The corpus/Pro chain keeps its pre-part-A candidate list and order
    // (JD6-A-005): one bundled layout for the current OS, then the dev
    // candidates. The runtime-free resolver probes the broader host-filtered
    // list instead (`resolve_bundled_pdfium_path`).
    if let Some(resource_dir) = bundled_resource_dir {
        for bundled in bundled_pdfium_candidate_paths(resource_dir, &dll_name) {
            if bundled.exists() {
                return Some(strip_windows_prefix(bundled));
            }
        }
    }

    for dev_path in dev_pdfium_candidate_paths(manifest_dir, dll_name.to_string_lossy().as_ref()) {
        if dev_path.exists() {
            return Some(strip_windows_prefix(dev_path));
        }
    }

    None
}

/// The bundled/dev part of the lookup, with the resource dir injectable: the
/// app's resource dir at runtime, a fake installer layout in the tests. This
/// is the whole of the runtime-free resolver (`ensure_pdfium_path_without_runtime`)
/// used by the bibliography page reader — it must never reach the ML runtime
/// module, pinned by `the_bundled_resolver_never_calls_the_ml_runtime_bootstrap`.
/// It probes [`host_pdfium_candidate_paths`] — the host's own layouts only
/// (JD6-A-005).
fn resolve_bundled_pdfium_path(
    resource_dir: Option<&std::path::Path>,
    manifest_dir: &std::path::Path,
) -> Option<PathBuf> {
    for candidate in host_pdfium_candidate_paths(resource_dir, manifest_dir) {
        if candidate.exists() {
            return Some(strip_windows_prefix(candidate));
        }
    }

    None
}

/// The candidates [`resolve_bundled_pdfium_path`] may probe: the current
/// host's layouts only (JD6-A-005). A `<os>-<arch>` resource subdir for a
/// DIFFERENT host is never probed — a foreign-architecture library must not
/// shadow a host-compatible one (or the system-library fallback behind it).
/// The untagged layout dirs carry the host library's own name in every
/// installer that uses them:
/// - macOS: `Contents/Frameworks/libpdfium.dylib`, beside `Contents/Resources`
///   (`bundle.macOS.frameworks` in tauri.lite.macos.conf.json);
/// - Lite Linux: `resources/pdfium/libpdfium.so` under `/usr/lib/<productName>`
///   (`bundle.resources` in tauri.lite.linux.conf.json);
/// - Pro/Lite Windows installers, the Store MSIX repack and `tauri dev`
///   (whose resource dir is `target/debug`): `resources/lib/<dll>`
///   (`bundle.resources` in tauri.windows.conf.json / tauri.lite.windows.conf.json);
/// - Pro Linux: `resources/lib/linux-x86_64/libpdfium.so`
///   (`bundle.resources` in tauri.linux.conf.json) — the host's own
///   `<os>-<arch>` subdir.
///
/// The dev candidates follow exactly where the pre-part-A code had them and
/// in its order (the flat host path first): `CARGO_MANIFEST_DIR/resources/lib`
/// plus the Linux-only per-arch and runtime-pack extras.
fn host_pdfium_candidate_paths(resource_dir: Option<&Path>, manifest_dir: &Path) -> Vec<PathBuf> {
    let dll_name = Pdfium::pdfium_platform_library_name();
    let mut candidates = Vec::new();

    if let Some(resource_dir) = resource_dir {
        // The .app's Frameworks dir sits beside the Contents/Resources dir
        // Tauri reports; for any other layout the parent probe just misses.
        #[cfg(target_os = "macos")]
        if let Some(contents) = resource_dir.parent() {
            candidates.push(contents.join("Frameworks").join(&dll_name));
        }
        // Lite resources: `resources/pdfium/<dll>`.
        candidates.push(
            resource_dir
                .join("resources")
                .join("pdfium")
                .join(&dll_name),
        );
        // Pro/Lite Windows, the MSIX repack and the dev resource dir:
        // `resources/lib/<dll>`.
        candidates.push(resource_dir.join("resources").join("lib").join(&dll_name));
        // Pro Linux (and any per-arch pack): `resources/lib/<os>-<arch>/<dll>`
        // — the HOST subdir only.
        if let Some(platform) = host_platform_resource_dir() {
            candidates.push(
                resource_dir
                    .join("resources")
                    .join("lib")
                    .join(platform)
                    .join(&dll_name),
            );
        }
    }

    candidates.extend(dev_pdfium_candidate_paths(
        manifest_dir,
        dll_name.to_string_lossy().as_ref(),
    ));
    candidates
}

/// The `<os>-<arch>` resource subdir that matches this host, when the host is
/// one of the layouts the installers ship. Foreign-architecture subdirs are
/// never candidates (JD6-A-005).
fn host_platform_resource_dir() -> Option<&'static str> {
    const PLATFORM_DIRS: &[&str] = &[
        "windows-x86_64",
        "windows-aarch64",
        "linux-x86_64",
        "linux-aarch64",
        "macos-x86_64",
        "macos-aarch64",
    ];
    let current = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    PLATFORM_DIRS.iter().copied().find(|dir| *dir == current)
}

/// Where the installers put the Pdfium library for THIS OS, relative to the
/// resource directory Tauri reports — the shared candidate list of
/// [`init_pdfium_path`] (the corpus/Pro chain). Pinned to its pre-part-A
/// shape by `the_corpus_candidate_list_keeps_its_pre_part_a_shape` (JD6-A-005):
/// - macOS: `Contents/Frameworks/libpdfium.dylib`, beside `Contents/Resources`
///   (`bundle.macOS.frameworks` in tauri.lite.macos.conf.json);
/// - Linux: `resources/pdfium/libpdfium.so` under `/usr/lib/<productName>`
///   (`bundle.resources` in tauri.lite.linux.conf.json);
/// - Windows: `resources\lib\pdfium.dll` beside the exe (`bundle.resources` in
///   tauri.windows.conf.json for NSIS/MSI, and repack-store-msix.ps1 for the
///   Store MSIX). Pro finds its managed runtime copy first; Lite has only this.
fn bundled_pdfium_candidate_paths(resource_dir: &Path, dll_name: &std::ffi::OsStr) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        resource_dir
            .parent()
            .map(|contents| vec![contents.join("Frameworks").join(dll_name)])
            .unwrap_or_default()
    }

    #[cfg(target_os = "linux")]
    {
        vec![resource_dir.join("resources").join("pdfium").join(dll_name)]
    }

    #[cfg(target_os = "windows")]
    {
        vec![resource_dir.join("resources").join("lib").join(dll_name)]
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (resource_dir, dll_name);
        Vec::new()
    }
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

/// Answers from the cache when it already holds a resolved path; otherwise
/// runs `resolve` exactly once and records the decision. A populated cache is
/// never re-resolved: the app setup decides once and every later reader
/// (bibliography page reader, OCR page renders) trusts it.
fn ensure_cached_pdfium_path(
    cache: &Mutex<Option<PathBuf>>,
    resolve: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    let cached = cache.lock().expect("pdfium path cache poisoned");
    if let Some(path) = cached.as_ref() {
        return Some(path.clone());
    }
    drop(cached);
    let resolved = resolve();
    let mut cached = cache.lock().expect("pdfium path cache poisoned");
    if cached.is_none() {
        *cached = resolved.clone();
    }
    resolved
}

/// Resolves the Pdfium library the bundle carries — and nothing else. Unlike
/// [`init_pdfium_path`] it never reaches the ML runtime: no background sync
/// and no bibliography extraction may trigger
/// `RuntimeManager::ensure_ready_or_bootstrap` (JD4-B-001). Used by the app
/// setup, before the bibliography page reader, and by
/// `ProductionSelectiveOcr::render_page` (which may follow it with
/// [`ensure_pdfium_path_with_hydrated_runtime`], JD6-B-001).
///
/// The guarantee covers PDFium resolution only: the Pro local Paddle OCR
/// model path still bootstraps the runtime (JD6-A-002, pre-existing).
///
/// Returns the resolved path, or `None` where nothing is bundled (Pro macOS):
/// the caller logs it and reads with lopdf instead.
pub fn ensure_pdfium_path_without_runtime(app_handle: &tauri::AppHandle) -> Option<PathBuf> {
    ensure_pdfium_path_without_runtime_dir(bundled_resource_dir(app_handle).as_deref())
}

/// [`ensure_pdfium_path_without_runtime`] with the resource dir injected: the
/// test seam for the resolver, and the reader's own last resort when no app
/// handle is available (native-only executors, direct reads).
pub(crate) fn ensure_pdfium_path_without_runtime_dir(
    resource_dir: Option<&std::path::Path>,
) -> Option<PathBuf> {
    let cache = PDFIUM_PATH.get_or_init(|| Mutex::new(None));
    ensure_cached_pdfium_path(cache, || {
        let resolved = resolve_bundled_pdfium_path(
            resource_dir,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        );
        match &resolved {
            Some(path) => eprintln!(
                "[pdf] ✅ Pdfium incluida resuelta sin runtime de ML: {}",
                path.display()
            ),
            None => eprintln!(
                "[pdf] ℹ️ Sin Pdfium incluida ({}) ni en el checkout de desarrollo; la lectura por página usa lopdf",
                dll_name_display()
            ),
        }
        resolved
    })
}

/// [`ensure_pdfium_path_without_runtime`] plus the one fallback the page
/// RENDER may take and the readers may not (JD6-B-001): an ALREADY-HYDRATED
/// managed runtime copy of the library, when one is on disk. Like the bundled
/// resolver this never calls `RuntimeManager::ensure_ready_or_bootstrap` — it
/// asks the runtime only for the root it has already hydrated
/// (`hydrated_runtime_root`) and keeps only a library file that exists there.
/// A missing copy is honest absence: the caller logs it and falls back.
pub fn ensure_pdfium_path_with_hydrated_runtime(app_handle: &tauri::AppHandle) -> Option<PathBuf> {
    let cache = PDFIUM_PATH.get_or_init(|| Mutex::new(None));
    ensure_cached_pdfium_path(cache, || {
        let resolved = resolve_bundled_pdfium_path(
            bundled_resource_dir(app_handle).as_deref(),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        );
        // The managed runtime copy only exists when local inference is
        // compiled in; the lean build has only the bundled library.
        #[cfg(feature = "local-ml")]
        let resolved = resolved.or_else(|| existing_managed_pdfium_path(app_handle));
        match &resolved {
            Some(path) => eprintln!(
                "[pdf] ✅ Pdfium resuelta para el render de páginas: {}",
                path.display()
            ),
            None => eprintln!(
                "[pdf] ℹ️ Sin Pdfium incluida ni copia hidratada del runtime ({})",
                dll_name_display()
            ),
        }
        resolved
    })
}

/// The managed-runtime half of the render fallback (JD6-B-001): the library
/// `init_pdfium_path` would use, but only when an already-hydrated managed
/// runtime root answers and the file is already on disk.
#[cfg(feature = "local-ml")]
fn existing_managed_pdfium_path(app_handle: &tauri::AppHandle) -> Option<PathBuf> {
    existing_managed_pdfium_path_with(|| RuntimeManager::new().hydrated_runtime_root(app_handle))
}

/// [`existing_managed_pdfium_path`] with the hydrated-root lookup injected:
/// one test proves a fixture runtime copy is resolved, another that absence
/// stays absence. The injected closure is the ONLY runtime call — nothing in
/// here may bootstrap (JD6-B-001), pinned by the source scan in
/// `the_bundled_resolver_never_calls_the_ml_runtime_bootstrap`.
#[cfg(any(feature = "local-ml", test))]
fn existing_managed_pdfium_path_with<H>(hydrated_runtime_root: H) -> Option<PathBuf>
where
    H: FnOnce() -> Result<Option<PathBuf>, String>,
{
    let root = hydrated_runtime_root().ok().flatten()?;
    let dll_name = Pdfium::pdfium_platform_library_name();
    // Same `<root>/resources/lib` layout `init_pdfium_path` uses (see
    // `resolve_pdfium_dll_path_from_roots`); the join is inlined so the lean
    // build compiles it too.
    #[cfg(feature = "local-ml")]
    let managed = managed_resource_path(&root, "lib").join(&dll_name);
    #[cfg(not(feature = "local-ml"))]
    let managed = root.join("resources").join("lib").join(&dll_name);
    managed.exists().then(|| strip_windows_prefix(managed))
}

/// Initialize a Pdfium instance without panicking.
///
/// Uses the cached DLL path if `init_pdfium_path()` was called, otherwise
/// falls back to current directory + system library (original behavior).
///
/// # Errors
/// Returns `Err` with a human-readable message if the Pdfium native
/// library cannot be loaded (missing DLL/so/dylib, wrong architecture, etc.).
fn get_pdfium() -> Result<GuardedPdfium, String> {
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

    PDFIUM_BIND_COUNT.with(|count| count.set(count.get() + 1));
    Ok(GuardedPdfium::new(Pdfium::new(bindings)))
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

/// Why the progress-reporting whole-document extract
/// ([`extract_pdf_text_with_progress`]) stopped.
#[derive(Debug)]
pub enum ExtractPdfTextError {
    /// The caller's cancel flag fired at a page boundary: the pass stopped
    /// early and no text is kept.
    Cancelled,
    /// The read failed (locked file, parser error or panic): the same
    /// message [`extract_pdf_text`] has always returned.
    Failed(String),
}

/// How one whole-document pass over the parser ended. Text exists only on
/// `Text`; `Cancelled` and the failures carry no partial output.
#[derive(Debug)]
enum ExtractPassOutcome {
    Text(String),
    Cancelled,
    Failed(pdf_extract::OutputError),
    Panicked,
}

/// A delegating [`pdf_extract::OutputDev`] around
/// [`pdf_extract::PlainTextOutput`]: every method forwards unchanged — the
/// text must stay byte-identical to `pdf_extract::extract_text_from_mem` —
/// and `end_page` additionally reports the finished page and checks the
/// cancel flag, the only per-page boundary `output_doc` offers. On cancel it
/// answers an [`pdf_extract::OutputError`] so `output_doc` stops walking
/// pages at once.
struct PageProgressOutput<'a, 'b, 'c> {
    inner: pdf_extract::PlainTextOutput<&'a mut String>,
    page_total: i64,
    pages_done: i64,
    cancel: Option<&'b std::sync::atomic::AtomicBool>,
    on_page: Option<&'c mut dyn FnMut(i64, i64)>,
    cancelled: bool,
}

impl<'a, 'b, 'c> PageProgressOutput<'a, 'b, 'c> {
    fn new(
        writer: &'a mut String,
        page_total: i64,
        cancel: Option<&'b std::sync::atomic::AtomicBool>,
        on_page: Option<&'c mut dyn FnMut(i64, i64)>,
    ) -> Self {
        Self {
            inner: pdf_extract::PlainTextOutput::new(writer),
            page_total,
            pages_done: 0,
            cancel,
            on_page,
            cancelled: false,
        }
    }
}

impl pdf_extract::OutputDev for PageProgressOutput<'_, '_, '_> {
    fn begin_page(
        &mut self,
        page_num: u32,
        media_box: &pdf_extract::MediaBox,
        art_box: Option<(f64, f64, f64, f64)>,
    ) -> Result<(), pdf_extract::OutputError> {
        self.inner.begin_page(page_num, media_box, art_box)
    }

    fn end_page(&mut self) -> Result<(), pdf_extract::OutputError> {
        self.inner.end_page()?;
        self.pages_done += 1;
        if let Some(on_page) = self.on_page.as_mut() {
            on_page(self.pages_done, self.page_total);
        }
        if self
            .cancel
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
        {
            self.cancelled = true;
            return Err(cancelled_output_error());
        }
        Ok(())
    }

    fn output_character(
        &mut self,
        trm: &pdf_extract::Transform,
        width: f64,
        spacing: f64,
        font_size: f64,
        char: &str,
    ) -> Result<(), pdf_extract::OutputError> {
        self.inner
            .output_character(trm, width, spacing, font_size, char)
    }

    fn begin_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        self.inner.begin_word()
    }

    fn end_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        self.inner.end_word()
    }

    fn end_line(&mut self) -> Result<(), pdf_extract::OutputError> {
        self.inner.end_line()
    }

    fn stroke(
        &mut self,
        ctm: &pdf_extract::Transform,
        colorspace: &pdf_extract::ColorSpace,
        color: &[f64],
        path: &pdf_extract::Path,
    ) -> Result<(), pdf_extract::OutputError> {
        self.inner.stroke(ctm, colorspace, color, path)
    }

    fn fill(
        &mut self,
        ctm: &pdf_extract::Transform,
        colorspace: &pdf_extract::ColorSpace,
        color: &[f64],
        path: &pdf_extract::Path,
    ) -> Result<(), pdf_extract::OutputError> {
        self.inner.fill(ctm, colorspace, color, path)
    }
}

/// The sentinel `end_page` answers on cancel: any
/// [`pdf_extract::OutputError`] stops `output_doc`; which outcome it was is
/// read from the wrapper's `cancelled` flag, never parsed back out of the
/// error.
fn cancelled_output_error() -> pdf_extract::OutputError {
    pdf_extract::OutputError::IoError(std::io::Error::new(
        std::io::ErrorKind::Interrupted,
        "text extraction cancelled",
    ))
}

/// pdf-extract 0.7.12 keeps `maybe_decrypt` private; this is that step
/// verbatim: a permissions-only file opens with the empty user password, and
/// a real user password fails with the crate's own hint on stderr.
fn maybe_decrypt_pdf_extract(
    doc: &mut pdf_extract::Document,
) -> Result<(), pdf_extract::OutputError> {
    if !doc.is_encrypted() {
        return Ok(());
    }
    if let Err(error) = doc.decrypt("") {
        if let pdf_extract::Error::Decryption(
            pdf_extract::encryption::DecryptionError::IncorrectPassword,
        ) = error
        {
            eprintln!(
                "Encrypted documents must be decrypted with a password using \
                 {{extract_text|extract_text_from_mem|output_doc}}_encrypted"
            );
        }
        return Err(pdf_extract::OutputError::PdfError(error));
    }
    Ok(())
}

/// Mirrors pdf-extract 0.7.12's `extract_text_from_mem` exactly — same
/// [`pdf_extract::Document::load_mem`], same decrypt step
/// ([`maybe_decrypt_pdf_extract`]), same [`pdf_extract::output_doc`] over
/// [`pdf_extract::PlainTextOutput`] — with [`PageProgressOutput`] around the
/// writer. Progress and cancel ride on `end_page`; the text itself must come
/// out byte-identical. The panic containment callers rely on lives here too.
fn extract_text_from_mem_reported(
    bytes: &[u8],
    cancel: Option<&std::sync::atomic::AtomicBool>,
    on_page: Option<&mut dyn FnMut(i64, i64)>,
) -> ExtractPassOutcome {
    let mut text = String::new();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut doc = match pdf_extract::Document::load_mem(bytes) {
            Ok(doc) => doc,
            Err(error) => {
                return ExtractPassOutcome::Failed(pdf_extract::OutputError::PdfError(error))
            }
        };
        if let Err(error) = maybe_decrypt_pdf_extract(&mut doc) {
            return ExtractPassOutcome::Failed(error);
        }
        let page_total = doc.get_pages().len() as i64;
        let mut output = PageProgressOutput::new(&mut text, page_total, cancel, on_page);
        match pdf_extract::output_doc(&doc, &mut output) {
            Ok(()) => {}
            Err(_) if output.cancelled => return ExtractPassOutcome::Cancelled,
            Err(error) => return ExtractPassOutcome::Failed(error),
        }
        ExtractPassOutcome::Text(std::mem::take(&mut text))
    })) {
        Ok(outcome) => outcome,
        Err(_) => ExtractPassOutcome::Panicked,
    }
}

/// [`extract_pdf_text`] with the reprocess preview's hooks: `on_page`, when
/// there is one, receives `(pages_done, pages_total)` after every finished
/// page of the whole-document parser, and `cancel`, when there is one, is
/// checked at the same boundaries — a flag set mid-extract stops the pass
/// with [`ExtractPdfTextError::Cancelled`] instead of running to the end.
/// With neither hook the read is exactly [`extract_pdf_text`].
pub fn extract_pdf_text_with_progress(
    bytes: &[u8],
    cancel: Option<&std::sync::atomic::AtomicBool>,
    on_page: Option<&mut dyn FnMut(i64, i64)>,
) -> Result<String, ExtractPdfTextError> {
    let bytes = open_with_empty_password(bytes).map_err(ExtractPdfTextError::Failed)?;
    // `pdf-extract` parses the file with its own, stricter lopdf: it reads the
    // header line as UTF-8 and wants `%PDF-` at byte 0.
    let normalized = normalize_pdf_header(&bytes);
    let bytes: &[u8] = normalized.as_deref().unwrap_or(&bytes);
    let text = match extract_text_from_mem_reported(bytes, cancel, on_page) {
        ExtractPassOutcome::Text(text) => text,
        ExtractPassOutcome::Cancelled => return Err(ExtractPdfTextError::Cancelled),
        ExtractPassOutcome::Failed(error) => {
            return Err(ExtractPdfTextError::Failed(format!(
                "PDF text extraction failed: {error}"
            )))
        }
        ExtractPassOutcome::Panicked => {
            return Err(ExtractPdfTextError::Failed(
                UNREADABLE_PDF_TEXT_MESSAGE.to_string(),
            ))
        }
    };
    // `pdf-extract` also fails silently: a page with an inline image
    // ahead of its text comes back as `Ok("")`. That is not "no text
    // layer", so ask the page layer before any caller treats the file as
    // a scan. An honest blank stays the original answer.
    Ok(if text.trim().is_empty() {
        page_layer_text(bytes).unwrap_or(text)
    } else {
        text
    })
}

/// Extract text from the native text layer of a PDF byte slice.
/// Returns the raw extracted text or an error message.
///
/// `pdf-extract` signals unsupported constructs (function type 4 tint
/// transforms, DeviceN spaces, dangling references, fonts without a unicode
/// map) with `panic!` instead of an error, and that panic is contained here,
/// at the narrowest boundary, so every caller — bibliography extraction,
/// corpus import, OCR fallback — receives an ordinary `Err`. Containment
/// needs unwinding: `[profile.release]` must not set `panic = "abort"`.
pub fn extract_pdf_text(bytes: &[u8]) -> Result<String, String> {
    match extract_pdf_text_with_progress(bytes, None, None) {
        Ok(text) => Ok(text),
        Err(ExtractPdfTextError::Failed(message)) => Err(message),
        // Unreachable: no cancel flag travels this path.
        Err(ExtractPdfTextError::Cancelled) => Err("PDF text extraction cancelled".to_string()),
    }
}

/// Text of every page through lopdf's per-page decoder, joined in page order.
/// `None` when the file does not load or no page holds any text.
fn page_layer_text(bytes: &[u8]) -> Option<String> {
    let document = load_lopdf_document(bytes, "page text").ok()?;
    let mut pages: Vec<String> = Vec::new();
    for number in document.get_pages().keys() {
        let text: String = document
            .extract_text_chunks_with_limit(
                &[*number],
                crate::bibliography::processing::BIBLIOGRAPHY_PAGE_CONTENT_LIMIT_BYTES,
            )
            .into_iter()
            .filter_map(Result::ok)
            .collect();
        if !text.trim().is_empty() {
            pages.push(text);
        }
    }
    (!pages.is_empty()).then(|| {
        pages.join(
            "

",
        )
    })
}

/// How far into the file the spec lets `%PDF-` sit.
const PDF_HEADER_SEARCH_WINDOW: usize = 1024;

/// Tolerates the header shapes real producers emit and strict parsers refuse:
/// junk before `%PDF-` (the spec allows it within the first 1024 bytes) and
/// binary bytes on the header line itself (`%PDF-1.4` + two high bytes + EOL).
///
/// Leading junk is dropped, because xref offsets count from `%PDF-`. Bad bytes
/// on the header line are blanked in place, so the length and every offset
/// after it stay put. Returns `None` when the header is already clean or no
/// `%PDF-` marker exists in the window, leaving the parser's own error honest.
pub(crate) fn normalize_pdf_header(bytes: &[u8]) -> Option<Vec<u8>> {
    const MARKER: &[u8] = b"%PDF-";
    let window = &bytes[..bytes.len().min(PDF_HEADER_SEARCH_WINDOW)];
    let start = window
        .windows(MARKER.len())
        .position(|candidate| candidate == MARKER)?;
    let body = &bytes[start..];
    let line_len = body
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
        .unwrap_or(body.len());
    let is_clean = |byte: &u8| byte.is_ascii() && !byte.is_ascii_control();
    if start == 0 && body[..line_len].iter().all(is_clean) {
        return None;
    }
    let mut repaired = body.to_vec();
    for byte in &mut repaired[..line_len] {
        if !is_clean(byte) {
            *byte = b' ';
        }
    }
    Some(repaired)
}

/// Opens a PDF that is encrypted but readable with the EMPTY user password.
///
/// "Permissions only" protection (an owner password restricting printing or
/// copying, no user password) is how most journal articles ship: any reader
/// opens them, so refusing them as "password protected" is wrong. Returns the
/// input untouched when it is not encrypted, the same document re-serialised
/// without encryption when the empty password opens it — so every downstream
/// parser (`pdf-extract`, per-page `lopdf`, pdfium) sees plain objects — and
/// [`ENCRYPTED_PDF_MESSAGE`] only when a real user password is required.
pub fn open_with_empty_password(bytes: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>, String> {
    use std::borrow::Cow;
    if !bytes
        .windows(b"/Encrypt".len())
        .any(|window| window == b"/Encrypt")
    {
        return Ok(Cow::Borrowed(bytes));
    }
    // `load_lopdf_document` authenticates with the empty user password while
    // loading and fails with the protected message when that does not open it.
    let mut document = load_lopdf_document(bytes, "decryption")?;
    if document.is_encrypted() {
        // Still carries the Encrypt entry: authentication did not run.
        document
            .decrypt("")
            .map_err(|_| ENCRYPTED_PDF_MESSAGE.to_string())?;
    } else if !document.was_encrypted() {
        // `/Encrypt` appeared only in content (a stream or a string).
        return Ok(Cow::Borrowed(bytes));
    }
    let mut plain = Vec::with_capacity(bytes.len());
    document
        .save_to(&mut plain)
        .map_err(|error| format!("Failed to rewrite PDF without encryption: {error}"))?;
    Ok(Cow::Owned(plain))
}

/// What a user reads when the PDF text parser gives up on a file's structure.
pub const UNREADABLE_PDF_TEXT_MESSAGE: &str =
    "No se pudo leer el texto de este PDF: su estructura interna no es compatible con el lector.";

/// Returns `true` if the text contains at least `MIN_ALPHANUM_CHARS` valid
/// UTF-8 alphanumeric characters and is not garbled (see [`is_garbled_text`]).
/// Used to decide whether native PDF text is rich enough or we should fall
/// back to OCR: raw glyph codes pass the character-count bar but are not
/// text a reader can use, so they route to OCR like a scanned page.
pub fn is_quality_text(text: &str) -> bool {
    const MIN_ALPHANUM_CHARS: usize = 50;
    text.chars().filter(|c| c.is_alphanumeric()).count() >= MIN_ALPHANUM_CHARS
        && !is_garbled_text(text)
}

/// True when the text looks like raw glyph codes instead of language.
///
/// Some PDFs embed fonts with a custom encoding and no ToUnicode map; every
/// parser then returns the raw glyph codes instead of the characters the
/// reader sees ("3FWJTUB" where the page says "Revista"). The codes are
/// deterministic per font — a constant shift, a symbol soup — but they are
/// not language, and letter statistics see that: shifted text is consonant
/// soup in capitals, with digits and symbols glued inside words.
///
/// Deterministic and language-light (Spanish/English/Portuguese primary).
/// Over letters it weighs the uppercase ratio, the vowel ratio (accented
/// vowels included), digits glued to letters, letters in tokens longer than
/// 24 characters, intrusive symbols like `[ @ ¨ ˇ ¡` inside words, and the
/// whitespace ratio. Tuned on two real garbled extractions from a user
/// library plus normal ES/EN academic text, an all-caps title page, short
/// all-caps headings, a table of numbers, references with DOIs/URLs/emails
/// and ligature text — none of which may flag.
pub fn is_garbled_text(text: &str) -> bool {
    let Some(stats) = GarbleStats::of(text) else {
        return false;
    };
    let letters = stats.letters as f64;
    let vowel_ratio = stats.vowel_letters as f64 / letters;
    let upper_ratio = stats.upper_letters as f64 / letters;
    let intrusion_ratio = stats.intrusive_symbols as f64 / letters;
    let glue_ratio = stats.glued_letters as f64 / letters;
    let long_token_ratio = stats.long_token_letters as f64 / letters;
    let whitespace_ratio = stats.whitespace_chars as f64 / stats.total_chars.max(1) as f64;

    // Vowel starvation: real Spanish/English/Portuguese prose never drops
    // this low over a paragraph (shifted glyph codes sit near 0.05-0.15,
    // because the vowel positions are filled by consonant codes).
    if stats.letters >= 24 && vowel_ratio < 0.15 {
        return true;
    }
    // The shift signature: capital consonant soup corroborated by digits
    // glued to letters, intrusive symbols inside words, or runaway tokens.
    if stats.letters >= 24
        && upper_ratio >= 0.60
        && vowel_ratio < 0.25
        && (intrusion_ratio >= 0.04 || glue_ratio >= 0.10 || long_token_ratio >= 0.25)
    {
        return true;
    }
    // Symbol soup around the words, whatever the case.
    if stats.letters >= 24 && intrusion_ratio >= 0.10 && vowel_ratio < 0.35 {
        return true;
    }
    // A page with no word separation at all is not language either.
    if stats.letters >= 40 && whitespace_ratio < 0.04 && vowel_ratio < 0.30 {
        return true;
    }
    // Short fragments (headings, captions) flag only on the full signature:
    // "STRENGTH" is a word, "3FWJTUB" is not.
    if stats.letters < 24
        && upper_ratio >= 0.90
        && vowel_ratio < 0.25
        && (intrusion_ratio >= 0.04 || glue_ratio >= 0.10 || long_token_ratio >= 0.15)
    {
        return true;
    }
    false
}

/// Vowel letters of Spanish/English/Portuguese, accented forms included.
fn is_vowel_letter(lower: char) -> bool {
    matches!(
        lower,
        'a' | 'e'
            | 'i'
            | 'o'
            | 'u'
            | 'á'
            | 'é'
            | 'í'
            | 'ó'
            | 'ú'
            | 'ü'
            | 'à'
            | 'è'
            | 'ì'
            | 'ò'
            | 'ù'
            | 'â'
            | 'ê'
            | 'î'
            | 'ô'
            | 'û'
            | 'ã'
            | 'õ'
            | 'ä'
            | 'ë'
            | 'ï'
            | 'ö'
            | 'ÿ'
            | 'å'
            | 'æ'
            | 'œ'
    )
}

/// Punctuation language actually uses. Anything else glued to a word is
/// evidence of a broken encoding (`[ @ ¨ ˇ ¡` and friends).
const COMMON_PUNCT: &[char] = &[
    '.', ',', ';', ':', '!', '?', '(', ')', '\'', '"', '-', '_', '/', '\\', '%', '&', '*', '+',
    '=', '<', '>', '#', '$', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2013}',
    '\u{2014}', '\u{2026}', '\u{00A7}', '\u{00B0}', '\u{00AB}', '\u{00BB}', '\u{2022}', '\u{00B7}',
];

/// Letter statistics [`is_garbled_text`] scores.
struct GarbleStats {
    letters: usize,
    upper_letters: usize,
    vowel_letters: usize,
    intrusive_symbols: usize,
    glued_letters: usize,
    long_token_letters: usize,
    whitespace_chars: usize,
    total_chars: usize,
}

impl GarbleStats {
    /// Collects the statistics in one pass over the (ligature-expanded) text.
    /// `None` when the text holds fewer than six letters: too little evidence
    /// for any verdict but "not garbled".
    fn of(text: &str) -> Option<Self> {
        const MIN_LETTERS: usize = 6;
        let mut chars: Vec<char> = Vec::with_capacity(text.len());
        for c in text.chars() {
            match c {
                '\u{FB00}' => chars.extend(['f', 'f']),
                '\u{FB01}' => chars.extend(['f', 'i']),
                '\u{FB02}' => chars.extend(['f', 'l']),
                '\u{FB03}' => chars.extend(['f', 'f', 'i']),
                '\u{FB04}' => chars.extend(['f', 'f', 'l']),
                other => chars.push(other),
            }
        }
        let mut stats = Self {
            letters: 0,
            upper_letters: 0,
            vowel_letters: 0,
            intrusive_symbols: 0,
            glued_letters: 0,
            long_token_letters: 0,
            whitespace_chars: 0,
            total_chars: 0,
        };
        // A maximal alphanumeric run with at least one digit and three
        // letters: digits glued into words ("3FWJTUB", "Table2shows").
        let mut run_letters = 0usize;
        let mut run_digits = 0usize;
        for (index, &c) in chars.iter().enumerate() {
            stats.total_chars += 1;
            if c.is_whitespace() {
                stats.whitespace_chars += 1;
            } else if c.is_alphabetic() {
                stats.letters += 1;
                if c.is_uppercase() {
                    stats.upper_letters += 1;
                }
                let mut lower = c.to_lowercase();
                if let (Some(first), None) = (lower.next(), lower.next()) {
                    if is_vowel_letter(first) {
                        stats.vowel_letters += 1;
                    }
                }
            }
            if c.is_alphanumeric() {
                if c.is_alphabetic() {
                    run_letters += 1;
                } else {
                    run_digits += 1;
                }
                continue;
            }
            flush_run(&mut stats, &mut run_letters, &mut run_digits);
            if c.is_whitespace() || COMMON_PUNCT.contains(&c) {
                continue;
            }
            let near_letter = chars
                .get(index.wrapping_sub(1))
                .is_some_and(|before| before.is_alphabetic())
                || chars
                    .get(index + 1)
                    .is_some_and(|after| after.is_alphabetic());
            if near_letter {
                stats.intrusive_symbols += 1;
            }
        }
        flush_run(&mut stats, &mut run_letters, &mut run_digits);
        // Letters sitting in whitespace-free tokens longer than 24 chars.
        for token in text.split_whitespace() {
            let token_letters = token.chars().filter(|c| c.is_alphabetic()).count();
            if token_letters > 24 {
                stats.long_token_letters += token_letters;
            }
        }
        (stats.letters >= MIN_LETTERS).then_some(stats)
    }
}

/// Books one maximal alphanumeric run into the glue counter and resets it.
fn flush_run(stats: &mut GarbleStats, run_letters: &mut usize, run_digits: &mut usize) {
    if *run_digits > 0 && *run_letters >= 3 {
        stats.glued_letters += *run_letters;
    }
    *run_letters = 0;
    *run_digits = 0;
}

/// Version of the bibliography garble detector. Bumped whenever a rule or a
/// threshold changes, so a stored measurement names the rules that produced
/// it (B2, plan-texto-nativo-parte-b 2.2).
pub const BIBLIOGRAPHY_DETECTOR_VERSION: u32 = 1;

/// Which of the two bibliography-garble rules flag a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GarbledBibliographyFlags {
    /// Rule 1, glued words: at least half the Latin letters sit in tokens
    /// whose longest Latin-letter run is longer than 24 characters.
    pub glued_words: bool,
    /// Rule 2, old OCR noise: at least 8 % of the eligible 4+-letter tokens
    /// carry an intrusive `.`, `~` or `·` between two letters.
    pub old_ocr_noise: bool,
}

/// True when the bibliography detector flags the text as garbled (B2). Used
/// to pick OCR candidates among pages whose native text LOOKS rich: the
/// lopdf page layer of old books glues every word together
/// ("ArrozElcultivodelarroz…"), and old recognizers leave dotted noise in
/// text that is otherwise spaced. Sibling of [`is_garbled_text`] (raw glyph
/// codes), not a replacement: the corpus rules of `is_garbled_text` and
/// [`is_quality_text`] are untouched.
///
/// Two independent rules over the eligible tokens (whitespace split minus
/// URL/DOI/email/domain tokens, punctuation stripped at the edges):
///
/// 1. **Glued words.** Needs 80 Latin letters (ASCII, Latin-1 Supplement,
///    Latin Extended-A/B — so spaceless scripts are never judged). Flags
///    when >= 50 % of those letters lie in tokens whose longest Latin-letter
///    run is longer than 24.
/// 2. **Old OCR noise.** Needs 40 eligible tokens of 4+ Latin letters. Flags
///    when >= 8 % of them contain `.`, `~` or `·` between two letters.
///    Hyphens and apostrophes never count, the Catalan geminate `l·l` is
///    not noise, and dotted abbreviations (`U.S.`, `U.S.A.`, `e.g.`, `i.e.`)
///    are not noise either.
pub fn is_garbled_bibliography_text(text: &str) -> bool {
    let flags = garbled_bibliography_flags(text);
    flags.glued_words || flags.old_ocr_noise
}

/// The per-rule verdict behind [`is_garbled_bibliography_text`]: the
/// measurement report shows which rule flagged each page.
pub fn garbled_bibliography_flags(text: &str) -> GarbledBibliographyFlags {
    let tokens = eligible_bibliography_tokens(text);
    GarbledBibliographyFlags {
        glued_words: glued_words_flag(&tokens),
        old_ocr_noise: old_ocr_noise_flag(&tokens),
    }
}

/// Rule 1 needs this many Latin letters before it judges a page: below it
/// there is no evidence either way (spaceless scripts never reach it).
const BIBLIOGRAPHY_GLUED_MIN_LETTERS: usize = 80;
/// A Latin-letter run longer than this is a glued word.
const BIBLIOGRAPHY_GLUED_RUN: usize = 24;
/// Rule 2 needs this many eligible tokens of 4+ Latin letters.
const BIBLIOGRAPHY_NOISE_MIN_TOKENS: usize = 40;
/// A token enters rule 2's judgement from this many Latin letters.
const BIBLIOGRAPHY_NOISE_MIN_TOKEN_LETTERS: usize = 4;

/// The letters rule 1 counts: ASCII, Latin-1 Supplement and Latin
/// Extended-A/B. `×` (U+00D7) and `÷` (U+00F7) sit inside Latin-1 but are
/// operators, not letters.
fn is_latin_letter(c: char) -> bool {
    c.is_ascii_alphabetic()
        || matches!(
            c,
            '\u{00C0}'..='\u{00D6}' | '\u{00D8}'..='\u{00F6}' | '\u{00F8}'..='\u{00FF}'
        )
        || matches!(c, '\u{0100}'..='\u{024F}')
}

/// Punctuation a producer hangs on token edges; stripped before the token
/// is classified, so "(U.S.A)," and "«riego»." keep their identity.
const BIBLIOGRAPHY_EDGE_PUNCT: &[char] = &[
    '.', ',', ';', ':', '!', '?', '(', ')', '[', ']', '{', '}', '\'', '"', '-', '_', '`', '~', '*',
    '+', '=', '<', '>', '/', '\\', '|', '@', '#', '$', '%', '^', '&', '\u{2018}', '\u{2019}',
    '\u{201C}', '\u{201D}', '\u{00AB}', '\u{00BB}', '\u{2013}', '\u{2014}', '\u{2026}',
];

fn starts_with_ascii_ignore_case(token: &str, prefix: &str) -> bool {
    token.len() >= prefix.len()
        && token.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Whether the token is a URL, DOI, email or domain (with or without a
/// path): those are references, not words, and their long dotted runs must
/// never move either rule. Everything else is classified on the
/// punctuation-stripped token.
fn is_reference_token(token: &str) -> bool {
    if starts_with_ascii_ignore_case(token, "http://")
        || starts_with_ascii_ignore_case(token, "https://")
        || starts_with_ascii_ignore_case(token, "www.")
        || starts_with_ascii_ignore_case(token, "doi:")
    {
        return true;
    }
    if token.as_bytes().contains(&b'@') {
        return true;
    }
    // A DOI registrant prefix: `10.<digits>/…`.
    if starts_with_ascii_ignore_case(token, "10.") {
        let rest = &token[3..];
        if let Some((prefix, _)) = rest.split_once('/') {
            if prefix.len() >= 4 && prefix.bytes().all(|b| b.is_ascii_digit()) {
                return true;
            }
        }
    }
    // A domain, path suffix or not: `something.tld` where the TLD is one
    // of the usual ones or any two letters.
    let host = token.split('/').next().unwrap_or(token);
    if let Some(dot) = host.rfind('.') {
        if dot > 0 && dot + 1 < host.len() {
            let tld = &host[dot + 1..];
            if tld.bytes().all(|b| b.is_ascii_alphabetic()) {
                let tld_lower = tld.to_ascii_lowercase();
                if tld.len() == 2
                    || matches!(
                        tld_lower.as_str(),
                        "com" | "org" | "net" | "edu" | "gov" | "io"
                    )
                {
                    return true;
                }
            }
        }
    }
    false
}

/// The tokens both rules judge: whitespace split, edge punctuation
/// stripped, references dropped.
fn eligible_bibliography_tokens(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .map(|raw| raw.trim_matches(|c: char| BIBLIOGRAPHY_EDGE_PUNCT.contains(&c)))
        .filter(|token| !token.is_empty() && !is_reference_token(token))
        .collect()
}

/// Rule 1, glued words: at least half the Latin letters sit in tokens whose
/// longest Latin-letter run is longer than 24 — the shape of a page layer
/// that never saw a space. Exact integer arithmetic at the 50 % line.
fn glued_words_flag(tokens: &[&str]) -> bool {
    let mut total = 0usize;
    let mut glued = 0usize;
    for token in tokens {
        let letters = token.chars().filter(|c| is_latin_letter(*c)).count();
        total += letters;
        let mut run = 0usize;
        let mut longest = 0usize;
        for c in token.chars() {
            if is_latin_letter(c) {
                run += 1;
                longest = longest.max(run);
            } else {
                run = 0;
            }
        }
        if longest > BIBLIOGRAPHY_GLUED_RUN {
            glued += letters;
        }
    }
    total >= BIBLIOGRAPHY_GLUED_MIN_LETTERS && glued * 2 >= total
}

/// Rule 2, old OCR noise: at least 8 % of the judged tokens carry an
/// intrusive `.`, `~` or `·` between two letters. Exact integer arithmetic
/// at the 8 % line (4 of 50 flags; 3 of 50 does not).
fn old_ocr_noise_flag(tokens: &[&str]) -> bool {
    let mut judged = 0usize;
    let mut noisy = 0usize;
    for token in tokens {
        if token.chars().filter(|c| is_latin_letter(*c)).count()
            < BIBLIOGRAPHY_NOISE_MIN_TOKEN_LETTERS
        {
            continue;
        }
        judged += 1;
        if token_has_old_ocr_noise(token) {
            noisy += 1;
        }
    }
    judged >= BIBLIOGRAPHY_NOISE_MIN_TOKENS && noisy * 100 >= judged * 8
}

/// Whether the token carries one non-excluded noise occurrence. Hyphens
/// and apostrophes are not in the separator set at all: real spelling
/// ("state-of-the-art", "l'adquisició") never counts.
fn token_has_old_ocr_noise(token: &str) -> bool {
    let chars: Vec<char> = token.chars().collect();
    for index in 1..chars.len().saturating_sub(1) {
        let sep = chars[index];
        if !matches!(sep, '.' | '~' | '\u{00B7}') {
            continue;
        }
        let before = chars[index - 1];
        let after = chars[index + 1];
        if !(is_latin_letter(before) && is_latin_letter(after)) {
            continue;
        }
        // The Catalan geminate "l·l" ("paral·lel", case-insensitive).
        if sep == '\u{00B7}'
            && before.eq_ignore_ascii_case(&'l')
            && after.eq_ignore_ascii_case(&'l')
        {
            continue;
        }
        // Dotted abbreviations: "U.S.", "U.S.A.", "N.A.T.O." — and the
        // lowercase scholarly pair "e.g." / "i.e." wherever it appears.
        if sep == '.' {
            if before.is_uppercase() && after.is_uppercase() {
                continue;
            }
            if matches!(
                (before.to_ascii_lowercase(), after.to_ascii_lowercase()),
                ('e', 'g') | ('i', 'e')
            ) {
                continue;
            }
        }
        return true;
    }
    false
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

/// The sentence a user reads when a PDF cannot be opened because it is locked.
///
/// It names the cause and the way out. The message it replaced — a complaint
/// about a document without pages — sent people looking at the file for damage
/// that was not there.
/// Shared with the bibliography extractor so locked files name the lock
/// and the way out in both pipelines.
pub const ENCRYPTED_PDF_MESSAGE: &str =
    "El PDF está protegido con contraseña y no se puede leer. Quitale la protección y volvé a importarlo.";

/// True when the document opened but stayed locked.
///
/// An encrypted PDF parses fine: the structure is readable, the object streams
/// are not. `get_pages()` then comes back empty and every caller downstream
/// blames the file for having no pages. Asking the document whether it is still
/// encrypted separates "locked" from "genuinely empty", which are different
/// problems with different answers.
fn is_locked(document: &lopdf::Document) -> bool {
    document.is_encrypted() && document.get_pages().is_empty()
}

fn load_lopdf_document(bytes: &[u8], operation: &str) -> Result<lopdf::Document, String> {
    match lopdf::Document::load_mem(bytes) {
        Ok(document) if is_locked(&document) => Err(ENCRYPTED_PDF_MESSAGE.to_string()),
        Ok(document) => Ok(document),
        // Matched by text because the library reports it as a plain message.
        // That is fragile on purpose-built strings: upgrading lopdf renamed this
        // exact failure from "invalid start value in Prev field" to "failed
        // parsing cross reference table", and the repair silently stopped
        // running. Both spellings are accepted so an upgrade cannot quietly
        // disable the recovery again.
        Err(error)
            if {
                let text = error.to_string();
                text.contains("invalid start value in Prev field")
                    || text.contains("failed parsing cross reference table")
            } =>
        {
            let repaired = neutralize_invalid_latest_prev(bytes)
                .ok_or_else(|| format!("Failed to load PDF for {operation}: {error}"))?;
            let document = lopdf::Document::load_mem(&repaired).map_err(|retry_error| {
                format!(
                    "Failed to load PDF for {operation} after ignoring invalid Prev pointer: {retry_error}"
                )
            })?;
            if is_locked(&document) {
                return Err(ENCRYPTED_PDF_MESSAGE.to_string());
            }
            Ok(document)
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

    let page_ids = source.get_pages();
    let mut pages = Vec::with_capacity(page_numbers.len());
    for (index, page_number) in page_numbers.iter().copied().enumerate() {
        let page_id = *page_ids
            .get(&page_number)
            .ok_or_else(|| format!("Missing page {page_number} while splitting"))?;

        let mut single = extract_single_page(&source, page_id)
            .map_err(|e| format!("Failed to extract page {}: {e}", index + 1))?;

        let mut pdf_bytes = Vec::new();
        single
            .save_to(&mut pdf_bytes)
            .map_err(|e| format!("Failed to save single-page PDF for page {}: {e}", index + 1))?;
        pages.push((index as u32 + 1, pdf_bytes));
    }

    Ok(pages)
}

/// Attributes a page inherits from the page tree when it does not carry its own.
///
/// The extracted page loses its ancestors, so whatever it was inheriting has to
/// travel with it or the page renders wrong — a missing `MediaBox` alone changes
/// the page size.
const INHERITABLE_PAGE_KEYS: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// Builds a one-page document holding only what that page actually references.
///
/// The previous approach cloned the whole document once per page and then pruned
/// and renumbered every object in it, so the work grew with pages × objects: a
/// 518-page archival scan pinned a core for minutes. Copying only the reachable
/// subtree makes each page cost what that page weighs.
fn extract_single_page(source: &lopdf::Document, page_id: ObjectId) -> Result<Document, String> {
    let mut page_dict = source
        .get_dictionary(page_id)
        .map_err(|e| format!("page dictionary: {e}"))?
        .clone();

    // Resolve inheritance BEFORE dropping the parent chain, then cut it: with
    // `/Parent` still in place the traversal below would climb back into the
    // page tree and drag every sibling page along — which is the very cost this
    // function exists to avoid.
    for key in INHERITABLE_PAGE_KEYS {
        if !page_dict.has(key) {
            if let Some(value) = inherited_page_attribute(source, page_id, key) {
                page_dict.set(key.to_vec(), value);
            }
        }
    }
    page_dict.remove(b"Parent");

    let mut target = Document::with_version(source.version.clone());
    let mut copied = BTreeSet::new();
    copy_referenced_objects(source, &mut target, &page_dict, &mut copied)?;
    // Copied objects keep their source ids, but `add_object` numbers new ones
    // from `max_id`, which is still 0. Without this the page, page tree and
    // catalog below take ids 1-3 and overwrite whatever was copied there —
    // often a content stream, which leaves that page blank.
    target.max_id = copied.last().map_or(0, |(id, _)| *id);

    let new_page_id = target.add_object(Object::Dictionary(page_dict));
    let pages_id = target.add_object(dictionary! {
        "Type" => "Pages",
        "Count" => 1_i64,
        "Kids" => vec![new_page_id.into()],
    });
    // The page must point back at the tree it now belongs to.
    if let Ok(dictionary) = target.get_dictionary_mut(new_page_id) {
        dictionary.set("Parent", pages_id);
    }
    let catalog_id = target.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    target.trailer.set("Root", catalog_id);

    Ok(target)
}

/// Walks up `/Parent` looking for an attribute the page did not define itself.
fn inherited_page_attribute(
    source: &lopdf::Document,
    page_id: ObjectId,
    key: &[u8],
) -> Option<Object> {
    let mut current = page_id;
    // Bounded so a malformed file with a cyclic parent chain cannot spin here.
    for _ in 0..32 {
        let dictionary = source.get_dictionary(current).ok()?;
        if let Ok(value) = dictionary.get(key) {
            return Some(value.clone());
        }
        current = match dictionary.get(b"Parent").and_then(Object::as_reference) {
            Ok(parent) => parent,
            Err(_) => return None,
        };
    }
    None
}

/// Copies every object the value transitively references, preserving object ids.
///
/// Ids are kept as they are in the source so the references inside the copied
/// objects stay valid without a renumbering pass.
fn copy_referenced_objects(
    source: &lopdf::Document,
    target: &mut Document,
    value: &Dictionary,
    copied: &mut BTreeSet<ObjectId>,
) -> Result<(), String> {
    let mut pending: Vec<Object> = value.iter().map(|(_, object)| object.clone()).collect();

    while let Some(object) = pending.pop() {
        match object {
            Object::Reference(id) => {
                if !copied.insert(id) {
                    continue;
                }
                let referenced = match source.get_object(id) {
                    Ok(referenced) => referenced.clone(),
                    // A dangling reference is the source's problem, not a reason
                    // to fail the whole split: the page still renders without it.
                    Err(_) => continue,
                };
                pending.push(referenced.clone());
                target.objects.insert(id, referenced);
            }
            Object::Array(items) => pending.extend(items),
            Object::Dictionary(dictionary) => {
                pending.extend(dictionary.iter().map(|(_, object)| object.clone()))
            }
            Object::Stream(stream) => {
                pending.extend(stream.dict.iter().map(|(_, object)| object.clone()))
            }
            _ => {}
        }
    }

    Ok(())
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

    /// A PDF carrying only an OWNER password, AES-128 (`/V 4 /R 4 /AESV2`) —
    /// byte-for-byte the encryption the reported archival scans use. It opens
    /// with an empty user password: the restriction is on printing or copying,
    /// not on reading.
    ///
    /// The first attempt at this fixture used RC4 and passed before any fix
    /// existed, because that is the one scheme the previous lopdf could already
    /// read. A fixture that cannot fail is not a test.
    const OWNER_PASSWORD_PDF: &[u8] =
        include_bytes!("../../tests/fixtures/pdf-aes128-owner-password.pdf");

    /// A PDF carrying a real USER password: it cannot be read without it.
    const USER_PASSWORD_PDF: &[u8] =
        include_bytes!("../../tests/fixtures/pdf-aes128-user-password.pdf");

    /// 816 bytes minimised from real library PDFs: a page whose colour space
    /// is a `Separation` with a PostScript-calculator (`/FunctionType 4`) tint
    /// transform. `pdf-extract` 0.7 answers it with `panic!("unhandled
    /// function type 4")` instead of an error.
    const TYPE4_TINT_PDF: &[u8] =
        include_bytes!("../../tests/fixtures/pdf-type4-tint-transform.pdf");

    #[test]
    fn extract_pdf_text_turns_a_parser_panic_into_an_error() {
        // The same parser panics on other malformed inputs (DeviceN spaces,
        // dangling references, fonts without a unicode map). A panic must
        // never cross this boundary: callers get an honest Err they can
        // route, not an unwinding thread.
        let error = extract_pdf_text(TYPE4_TINT_PDF).expect_err("the parser panics on this file");

        assert_eq!(error, UNREADABLE_PDF_TEXT_MESSAGE);
    }

    /// Text PDFs with an owner password and an EMPTY user password — the
    /// "permissions only" protection journal articles ship with.
    const RC4_EMPTY_USER_TEXT_PDF: &[u8] =
        include_bytes!("../../tests/fixtures/pdf-rc4-128-empty-user-text.pdf");
    const AES_EMPTY_USER_TEXT_PDF: &[u8] =
        include_bytes!("../../tests/fixtures/pdf-aes128-empty-user-text.pdf");

    #[test]
    fn extract_pdf_text_reads_permissions_only_pdfs() {
        for (name, pdf) in [
            ("rc4", RC4_EMPTY_USER_TEXT_PDF),
            ("aes", AES_EMPTY_USER_TEXT_PDF),
        ] {
            let text = extract_pdf_text(pdf).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(text.contains("Permissions only"), "{name}: {text:?}");
        }
    }

    /// Dry run over real files: `ENTROPIA_PDF_SAMPLES_DIR=<dir> cargo test
    /// dry_run_encrypted_samples -- --ignored --nocapture`. Prints, per file,
    /// the decrypted whole-document text and the per-page lopdf text, plus how
    /// many pages fall below the selective-OCR threshold.
    #[test]
    #[ignore = "reads PDFs from ENTROPIA_PDF_SAMPLES_DIR"]
    fn dry_run_encrypted_samples() {
        let dir = std::env::var("ENTROPIA_PDF_SAMPLES_DIR").expect("set ENTROPIA_PDF_SAMPLES_DIR");
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .expect("dir")
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "pdf"))
            .collect();
        files.sort();
        for path in files {
            let bytes = std::fs::read(&path).expect("read");
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let plain = match open_with_empty_password(&bytes) {
                Ok(plain) => plain,
                Err(error) => {
                    println!("{name}: OPEN FAILED: {error}");
                    continue;
                }
            };
            let document = lopdf::Document::load_mem(&plain).expect("plain parses");
            let pages: Vec<u32> = document.get_pages().keys().copied().collect();
            let mut per_page_chars = 0usize;
            let mut thin_pages = 0usize;
            for page in &pages {
                let text = document.extract_text(&[*page]).unwrap_or_default();
                per_page_chars += text.chars().count();
                if text.chars().filter(|c| c.is_alphanumeric()).count() < 50 {
                    thin_pages += 1;
                }
            }
            let whole = extract_pdf_text(&bytes);
            println!(
                "{name}: pages={} per_page_chars={per_page_chars} thin_pages={thin_pages} whole={}",
                pages.len(),
                match &whole {
                    Ok(text) => format!("{} chars", text.chars().count()),
                    Err(error) => format!("ERR {error}"),
                }
            );
        }
    }

    /// A readable text PDF whose header line carries two binary bytes before
    /// the line break, the shape some producers emit. Same length as the
    /// original, so every xref offset stays valid.
    fn pdf_with_binary_header_line() -> Vec<u8> {
        let plain = open_with_empty_password(AES_EMPTY_USER_TEXT_PDF)
            .expect("opens")
            .into_owned();
        assert!(plain.starts_with(b"%PDF-1.3\n%"), "fixture header shape");
        let mut bytes = plain;
        bytes[8] = 0xbe;
        bytes[9] = 0xad;
        bytes
    }

    #[test]
    fn extract_pdf_text_reads_a_pdf_whose_header_line_has_binary_bytes() {
        let bytes = pdf_with_binary_header_line();

        let text = extract_pdf_text(&bytes).expect("a binary header line is tolerated");

        assert!(text.contains("Permissions only"), "{text:?}");
    }

    #[test]
    fn extract_pdf_text_reads_a_pdf_with_junk_before_the_header() {
        let mut bytes = b"junk\r\n".to_vec();
        bytes.extend_from_slice(&pdf_with_binary_header_line());

        // Offsets are relative to `%PDF-`, so the junk is stripped, not kept.
        let text = extract_pdf_text(&bytes).expect("leading junk is tolerated");

        assert!(text.contains("Permissions only"), "{text:?}");
    }

    #[test]
    fn load_lopdf_document_opens_a_pdf_whose_header_line_has_binary_bytes() {
        let bytes = pdf_with_binary_header_line();

        let document = load_lopdf_document(&bytes, "page count").expect("opens");

        assert_eq!(document.get_pages().len(), 1);
    }

    #[test]
    fn extract_pdf_text_keeps_an_honest_error_for_a_file_that_is_not_a_pdf() {
        let error = extract_pdf_text(b"<html>not a pdf at all</html>").expect_err("not a pdf");

        assert!(error.contains("PDF"), "{error}");
    }

    /// RC4-40 permissions-only PDF whose text sits after an inline image
    /// (`BI ... ID <binary> EI`), in a subset TrueType font with a
    /// `Differences` encoding — the shape of a scanned-then-OCR'd report.
    const RC4_40_INLINE_IMAGE_TEXT_PDF: &[u8] =
        include_bytes!("../../tests/fixtures/pdf-rc4-40-inline-image-text.pdf");

    /// `pdf-extract` answers `Ok("")` for a page with an inline image before
    /// its text: no error, no panic, no text. The fixture only guards the fix
    /// while that stays true.
    #[test]
    fn the_inline_image_fixture_still_defeats_the_whole_document_parser() {
        let plain = open_with_empty_password(RC4_40_INLINE_IMAGE_TEXT_PDF).expect("opens");
        let text = pdf_extract::extract_text_from_mem(&plain).expect("no error");
        assert!(text.trim().is_empty(), "pdf-extract read {text:?}");
    }

    #[test]
    fn extract_pdf_text_falls_back_to_the_page_layer_when_the_parser_returns_nothing() {
        let text = extract_pdf_text(RC4_40_INLINE_IMAGE_TEXT_PDF).expect("reads");
        assert!(
            text.contains("Informe sociolaboral del Partido de General Pueyrredon"),
            "{text:?}"
        );
    }

    #[test]
    fn extract_pdf_text_keeps_the_protected_error_for_a_real_user_password() {
        let error = extract_pdf_text(USER_PASSWORD_PDF).expect_err("locked");
        assert_eq!(error, ENCRYPTED_PDF_MESSAGE);
    }

    /// A multi-page PDF with real text per page — the shape the byte-identity
    /// assertions need: several `end_page` boundaries and stable output.
    fn text_pdf_pages(count: u32) -> Vec<u8> {
        use lopdf::Stream;
        let mut document = Document::with_version("1.5");
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let pages_id = document.new_object_id();
        let page_ids = (0..count)
            .map(|index| {
                let content = document.add_object(Stream::new(
                    dictionary! {},
                    format!("BT /F1 12 Tf 20 100 Td (Page {index} text) Tj ET").into_bytes(),
                ));
                document.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                    "Resources" => resources_id,
                    "Contents" => content,
                })
            })
            .collect::<Vec<_>>();
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Count" => count as i64,
                "Kids" => page_ids.iter().map(|id| (*id).into()).collect::<Vec<Object>>(),
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("serialize text PDF");
        bytes
    }

    /// The bytes the whole-document pass actually consumes: decrypted and
    /// header-normalized exactly as [`extract_pdf_text`] prepares them.
    fn preprocessed_pdf_bytes(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
        let plain = open_with_empty_password(bytes).expect("opens");
        match normalize_pdf_header(&plain) {
            Some(normalized) => std::borrow::Cow::Owned(normalized),
            None => plain,
        }
    }

    /// T4: the progress pass must not move one byte of the persisted text —
    /// same input, same output as `pdf_extract::extract_text_from_mem`, hook
    /// or no hook. The fixtures cover the multi-page and encrypted shapes.
    #[test]
    fn the_progress_pass_matches_pdf_extract_byte_for_byte() {
        let multi = text_pdf_pages(4);
        for (name, raw) in [
            ("owner", OWNER_PASSWORD_PDF.to_vec()),
            ("rc4", RC4_EMPTY_USER_TEXT_PDF.to_vec()),
            ("aes", AES_EMPTY_USER_TEXT_PDF.to_vec()),
            ("inline-image", RC4_40_INLINE_IMAGE_TEXT_PDF.to_vec()),
            ("multi-page", multi),
        ] {
            let plain = preprocessed_pdf_bytes(&raw);
            let expected = pdf_extract::extract_text_from_mem(&plain)
                .unwrap_or_else(|error| panic!("{name}: pdf-extract failed: {error}"));
            let mut pages: Vec<(i64, i64)> = Vec::new();
            let outcome = extract_text_from_mem_reported(
                &plain,
                None,
                Some(&mut |done: i64, total: i64| pages.push((done, total))),
            );
            let text = match outcome {
                ExtractPassOutcome::Text(text) => text,
                other => panic!("{name}: the progress pass failed: {other:?}"),
            };
            assert_eq!(text, expected, "{name}: the text must stay byte-identical");
        }
    }

    #[test]
    fn the_progress_pass_reports_one_unit_per_page() {
        let pdf = text_pdf_pages(4);
        let mut pages: Vec<(i64, i64)> = Vec::new();
        let outcome = extract_text_from_mem_reported(
            &pdf,
            None,
            Some(&mut |done: i64, total: i64| pages.push((done, total))),
        );

        assert!(matches!(outcome, ExtractPassOutcome::Text(_)));
        assert_eq!(pages, vec![(1, 4), (2, 4), (3, 4), (4, 4)]);
    }

    #[test]
    fn the_progress_pass_stops_on_cancel_at_the_next_page_boundary() {
        let pdf = text_pdf_pages(4);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut pages: Vec<i64> = Vec::new();
        let outcome = extract_text_from_mem_reported(
            &pdf,
            Some(&cancel),
            Some(&mut |done: i64, _total: i64| {
                pages.push(done);
                if done == 1 {
                    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }),
        );

        assert!(
            matches!(&outcome, ExtractPassOutcome::Cancelled),
            "the flag set inside the pass stops it: {outcome:?}"
        );
        assert_eq!(
            pages,
            vec![1],
            "the pass stops at the first page boundary after the flag: {pages:?}"
        );
    }

    #[test]
    fn the_progress_pass_contains_the_parser_panic_like_the_plain_path() {
        let outcome = extract_text_from_mem_reported(TYPE4_TINT_PDF, None, None);

        assert!(
            matches!(outcome, ExtractPassOutcome::Panicked),
            "a parser panic must not cross the boundary"
        );
    }

    #[test]
    fn open_with_empty_password_hands_back_plain_bytes_for_permissions_only_pdfs() {
        for pdf in [RC4_EMPTY_USER_TEXT_PDF, AES_EMPTY_USER_TEXT_PDF] {
            let plain = open_with_empty_password(pdf).expect("opens");
            assert!(
                !plain.windows(b"/Encrypt".len()).any(|w| w == b"/Encrypt"),
                "the handed-back bytes must carry no encryption"
            );
            let document = lopdf::Document::load_mem(&plain).expect("parses");
            assert!(!document.is_encrypted());
            assert_eq!(document.get_pages().len(), 1);
        }
        let borrowed = open_with_empty_password(TYPE4_TINT_PDF).expect("plain pdf");
        assert!(matches!(borrowed, std::borrow::Cow::Borrowed(_)));
        assert_eq!(
            open_with_empty_password(USER_PASSWORD_PDF).expect_err("locked"),
            ENCRYPTED_PDF_MESSAGE
        );
    }

    #[test]
    fn load_lopdf_document_opens_a_pdf_with_an_owner_password_only() {
        // Archival scans routinely restrict printing while opening freely for
        // reading. Refusing those is refusing most digitised archives — and it
        // did not even fail loudly: the document parsed, reported zero pages,
        // and every caller blamed the file for being empty.
        let document = load_lopdf_document(OWNER_PASSWORD_PDF, "splitting").expect("opens");

        assert_eq!(
            document.get_pages().len(),
            2,
            "an owner-password PDF must yield its pages"
        );
    }

    #[test]
    fn splitting_an_owner_password_pdf_produces_its_pages() {
        let pages = split_pdf_to_single_page_bytes(OWNER_PASSWORD_PDF).expect("splits");
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn a_user_password_pdf_reports_encryption_not_missing_pages() {
        // The reported defect: eighteen archival files failed with a message
        // about absent pages. They were neither empty nor broken — they were
        // encrypted, and the message sent the user looking at the wrong thing.
        let error = split_pdf_to_single_page_bytes(USER_PASSWORD_PDF).expect_err("cannot split");

        assert!(
            error.contains("protegido") && error.contains("contraseña"),
            "the message must name encryption: {error}"
        );
        assert!(
            !error.contains("without pages"),
            "the misleading message must not appear: {error}"
        );
    }

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

    /// Builds a PDF with `count` pages, each with its own content stream, so the
    /// per-page object graph is realistic rather than a shared blank page.
    fn many_page_pdf_bytes(count: u32) -> Vec<u8> {
        use lopdf::Stream;
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let page_ids = (0..count)
            .map(|index| {
                let content = document.add_object(Stream::new(
                    dictionary! {},
                    format!("BT /F1 12 Tf 20 100 Td (Page {index}) Tj ET").into_bytes(),
                ));
                document.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                    "Contents" => content,
                })
            })
            .collect::<Vec<_>>();
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Count" => count as i64,
                "Kids" => page_ids.iter().map(|id| (*id).into()).collect::<Vec<Object>>(),
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        document
            .save_to(&mut bytes)
            .expect("save many-page fixture");
        bytes
    }

    #[test]
    fn splitting_keeps_every_page_own_content() {
        // The new page, page tree and catalog must not take the ids of copied
        // objects: this fixture keeps the first page's content in object 2.
        let pages = split_pdf_to_single_page_bytes(&many_page_pdf_bytes(3)).expect("splits");

        assert_eq!(pages.len(), 3);
        for (number, bytes) in pages {
            let single = Document::load_mem(&bytes).expect("parse split page");
            let page_id = *single.get_pages().get(&1).expect("one page");
            let content = single.get_page_content(page_id);
            assert_eq!(
                // get_page_content ends each stream with a newline.
                String::from_utf8_lossy(&content).trim_end(),
                format!("BT /F1 12 Tf 20 100 Td (Page {}) Tj ET", number - 1),
                "page {number} lost its content"
            );
        }
    }

    #[test]
    fn splitting_stays_linear_in_the_number_of_pages() {
        // The reported case: a 518-page, 63 MB archival scan pinned a core for
        // minutes. Splitting cloned the WHOLE document once per page and then
        // pruned and renumbered every object in it, so the work grew with
        // pages × objects.
        //
        // The ceiling is deliberately loose — this catches a quadratic blowup,
        // not a few milliseconds of drift.
        let source = many_page_pdf_bytes(200);

        let started = std::time::Instant::now();
        let pages = split_pdf_to_single_page_bytes(&source).expect("split");
        let elapsed = started.elapsed();

        assert_eq!(pages.len(), 200);
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "splitting 200 pages took {elapsed:?}; that is quadratic behaviour, not slowness"
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
            resolve_pdfium_dll_path_from_roots(Some(runtime_dir.path()), None, manifest_dir.path());

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

        let resolved = resolve_pdfium_dll_path_from_roots(None, None, manifest_dir.path());

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

        let resolved = resolve_pdfium_dll_path_from_roots(None, None, manifest_dir.path());

        #[cfg(target_os = "linux")]
        assert_eq!(resolved, Some(runtime_pack));
        #[cfg(not(target_os = "linux"))]
        assert_eq!(resolved, None);
    }

    /// Where the bundler puts Pdfium relative to the resource dir Tauri reports:
    /// `Contents/Frameworks/` next to `Contents/Resources/` in the macOS .app
    /// (`bundle.macOS.frameworks`), `resources/pdfium/` under
    /// `/usr/lib/<productName>/` in the Linux .deb (`bundle.resources`), and
    /// `resources/lib/` beside the exe on Windows (`bundle.resources` in
    /// tauri.windows.conf.json, and the Store MSIX repack).
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn bundled_pdfium_fixture(resource_dir: &Path) -> PathBuf {
        let name = Pdfium::pdfium_platform_library_name();
        #[cfg(target_os = "macos")]
        let lib = resource_dir
            .parent()
            .expect("Contents")
            .join("Frameworks")
            .join(name);
        #[cfg(target_os = "linux")]
        let lib = resource_dir.join("resources").join("pdfium").join(name);
        #[cfg(target_os = "windows")]
        let lib = resource_dir.join("resources").join("lib").join(name);
        std::fs::create_dir_all(lib.parent().expect("parent")).expect("mkdir");
        std::fs::write(&lib, b"pdfium").expect("write");
        lib
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    #[test]
    fn resolve_pdfium_finds_the_library_bundled_with_the_installed_app() {
        let install = tempdir().expect("install dir");
        let manifest_dir = tempdir().expect("manifest dir");
        #[cfg(target_os = "macos")]
        let resource_dir = install
            .path()
            .join("EntropIA Lite.app")
            .join("Contents")
            .join("Resources");
        #[cfg(target_os = "linux")]
        let resource_dir = install.path().join("usr").join("lib").join("entropia-lite");
        // Windows reports the exe's own directory: the NSIS/MSI install dir, or
        // the package root of the Store MSIX.
        #[cfg(target_os = "windows")]
        let resource_dir = install.path().join("EntropIA Lite");
        std::fs::create_dir_all(&resource_dir).expect("mkdir resources");
        let bundled = bundled_pdfium_fixture(&resource_dir);

        let resolved =
            resolve_pdfium_dll_path_from_roots(None, Some(&resource_dir), manifest_dir.path());

        assert_eq!(resolved, Some(bundled));
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    #[test]
    fn resolve_pdfium_prefers_the_bundled_library_over_a_dev_checkout() {
        let install = tempdir().expect("install dir");
        let manifest_dir = tempdir().expect("manifest dir");
        let resource_dir = install.path().join("Contents").join("Resources");
        std::fs::create_dir_all(&resource_dir).expect("mkdir resources");
        let bundled = bundled_pdfium_fixture(&resource_dir);
        let dev = manifest_dir
            .path()
            .join("resources")
            .join("lib")
            .join(Pdfium::pdfium_platform_library_name());
        std::fs::create_dir_all(dev.parent().expect("parent")).expect("mkdir");
        std::fs::write(&dev, b"pdfium").expect("write");

        let resolved =
            resolve_pdfium_dll_path_from_roots(None, Some(&resource_dir), manifest_dir.path());

        assert_eq!(resolved, Some(bundled));
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    #[test]
    fn resolve_pdfium_keeps_the_managed_runtime_ahead_of_the_bundle() {
        let runtime_dir = tempdir().expect("runtime dir");
        let install = tempdir().expect("install dir");
        let manifest_dir = tempdir().expect("manifest dir");
        let managed = runtime_dir
            .path()
            .join("resources")
            .join("lib")
            .join(Pdfium::pdfium_platform_library_name());
        std::fs::create_dir_all(managed.parent().expect("parent")).expect("mkdir");
        std::fs::write(&managed, b"pdfium").expect("write");
        let resource_dir = install.path().join("Contents").join("Resources");
        std::fs::create_dir_all(&resource_dir).expect("mkdir resources");
        bundled_pdfium_fixture(&resource_dir);

        let resolved = resolve_pdfium_dll_path_from_roots(
            Some(runtime_dir.path()),
            Some(&resource_dir),
            manifest_dir.path(),
        );

        assert_eq!(resolved, Some(managed));
    }

    // ── A1: bundled resolution without the ML runtime ────────────────────

    /// The shared candidate list of the corpus/Pro chain (`init_pdfium_path`
    /// → `resolve_pdfium_dll_path_from_roots` → [`bundled_pdfium_candidate_paths`])
    /// must keep exactly its pre-part-A shape and order (JD6-A-005): one
    /// bundled layout for the current OS. The broader installer probing lives
    /// in the runtime-free resolver's own [`host_pdfium_candidate_paths`].
    #[test]
    fn the_corpus_candidate_list_keeps_its_pre_part_a_shape() {
        let dll = Pdfium::pdfium_platform_library_name();

        // macOS bundles put the library in `Contents/Frameworks`, beside the
        // `Contents/Resources` dir Tauri reports.
        #[cfg(target_os = "macos")]
        {
            let contents = PathBuf::from("install-root")
                .join("EntropIA Lite.app")
                .join("Contents");
            let mac_resource_dir = contents.join("Resources");
            assert_eq!(
                bundled_pdfium_candidate_paths(&mac_resource_dir, &dll),
                vec![contents.join("Frameworks").join(&dll)],
                "the macOS corpus list is the Frameworks layout, unchanged"
            );
        }
        // Lite Linux deb: `resources/pdfium/<dll>` under `/usr/lib/<productName>`.
        #[cfg(target_os = "linux")]
        {
            let flat_resource_dir = PathBuf::from("install-root");
            assert_eq!(
                bundled_pdfium_candidate_paths(&flat_resource_dir, &dll),
                vec![flat_resource_dir
                    .join("resources")
                    .join("pdfium")
                    .join(&dll)],
                "the Linux corpus list is the Lite layout, unchanged"
            );
        }
        // Pro + Lite Windows installers and the Store MSIX repack:
        // `resources/lib/<dll>` beside the exe.
        #[cfg(target_os = "windows")]
        {
            let flat_resource_dir = PathBuf::from("install-root");
            assert_eq!(
                bundled_pdfium_candidate_paths(&flat_resource_dir, &dll),
                vec![flat_resource_dir.join("resources").join("lib").join(&dll)],
                "the Windows corpus list is the resources/lib layout, unchanged"
            );
        }
    }

    /// The runtime-free resolver's own candidate list covers the layouts the
    /// installers ship for THIS host (JD5-B-005) — untagged dirs every host
    /// probes (`resources/pdfium/<dll>` Lite, `resources/lib/<dll>` Pro
    /// Windows + `tauri dev`), the host's own `<os>-<arch>` subdir (Pro
    /// Linux), and the macOS Frameworks dir beside `Contents/Resources`.
    #[test]
    fn the_bundled_resolver_covers_the_host_layouts() {
        let dll = Pdfium::pdfium_platform_library_name();
        let contents = PathBuf::from("install-root")
            .join("EntropIA Lite.app")
            .join("Contents");
        let resource_dir = contents.join("Resources");
        let candidates = host_pdfium_candidate_paths(Some(&resource_dir), Path::new("manifest"));

        // Lite: `resources/pdfium/<dll>`.
        assert!(
            candidates.contains(&resource_dir.join("resources").join("pdfium").join(&dll)),
            "Lite layout resources/pdfium missing from {candidates:?}"
        );
        // Pro + Lite Windows installers and `tauri dev` (where the resource
        // dir is `target/debug`): `resources/lib/<dll>`.
        assert!(
            candidates.contains(&resource_dir.join("resources").join("lib").join(&dll)),
            "Pro Windows / dev layout resources/lib missing from {candidates:?}"
        );
        // Pro Linux deb: `resources/lib/<host os>-<arch>/<dll>` — the host's
        // own subdir only.
        if let Some(platform) = host_platform_resource_dir() {
            assert!(
                candidates.contains(
                    &resource_dir
                        .join("resources")
                        .join("lib")
                        .join(platform)
                        .join(&dll)
                ),
                "Pro Linux layout resources/lib/{platform} missing from {candidates:?}"
            );
        }
        // macOS bundles put the library in `Contents/Frameworks`, beside the
        // `Contents/Resources` dir Tauri reports.
        #[cfg(target_os = "macos")]
        assert!(
            candidates.contains(&contents.join("Frameworks").join(&dll)),
            "macOS frameworks layout missing from {candidates:?}"
        );
    }

    /// One fake installer layout per host variant, injected as the resource
    /// dir: the resolver must return the library this host's variant ships.
    /// Nothing real is loaded — the fake files only need to exist.
    #[test]
    fn the_bundled_resolver_finds_the_host_layout_from_an_injected_resource_dir() {
        let dll = Pdfium::pdfium_platform_library_name();
        let manifest_dir = tempdir().expect("manifest dir");

        // Lite: resources/pdfium/<dll> under the resource dir.
        let lite = tempdir().expect("lite install");
        let lite_lib = lite.path().join("resources").join("pdfium").join(&dll);
        std::fs::create_dir_all(lite_lib.parent().expect("parent")).expect("mkdir");
        std::fs::write(&lite_lib, b"fake").expect("write");
        assert_eq!(
            resolve_bundled_pdfium_path(Some(lite.path()), manifest_dir.path()),
            Some(strip_windows_prefix(lite_lib)),
            "Lite layout"
        );

        // Pro Windows / dev (target/debug/resources/lib): resources/lib/<dll>.
        let windows = tempdir().expect("windows install");
        let windows_lib = windows.path().join("resources").join("lib").join(&dll);
        std::fs::create_dir_all(windows_lib.parent().expect("parent")).expect("mkdir");
        std::fs::write(&windows_lib, b"fake").expect("write");
        assert_eq!(
            resolve_bundled_pdfium_path(Some(windows.path()), manifest_dir.path()),
            Some(strip_windows_prefix(windows_lib)),
            "Pro Windows / dev layout"
        );

        // Pro Linux: resources/lib/<host os>-<arch>/<dll>.
        if let Some(platform) = host_platform_resource_dir() {
            let linux = tempdir().expect("per-arch install");
            let linux_lib = linux
                .path()
                .join("resources")
                .join("lib")
                .join(platform)
                .join(&dll);
            std::fs::create_dir_all(linux_lib.parent().expect("parent")).expect("mkdir");
            std::fs::write(&linux_lib, b"fake").expect("write");
            assert_eq!(
                resolve_bundled_pdfium_path(Some(linux.path()), manifest_dir.path()),
                Some(strip_windows_prefix(linux_lib)),
                "Pro Linux layout"
            );
        }

        // Dev checkout: CARGO_MANIFEST_DIR/resources/lib/<dll>.
        let dev = tempdir().expect("dev checkout");
        let dev_lib = dev.path().join("resources").join("lib").join(&dll);
        std::fs::create_dir_all(dev_lib.parent().expect("parent")).expect("mkdir");
        std::fs::write(&dev_lib, b"fake").expect("write");
        assert_eq!(
            resolve_bundled_pdfium_path(None, dev.path()),
            Some(strip_windows_prefix(dev_lib)),
            "dev checkout layout"
        );
    }

    /// JD6-A-005: the runtime-free resolver must not shadow host-compatible
    /// libraries with foreign-architecture ones. In a mixed layout — an
    /// x86_64 platform-subdir lib beside the host's own — the host lib wins;
    /// where only the foreign lib exists the resolver reports absence (and
    /// the system library behind it), never the foreign path.
    #[test]
    fn the_bundled_resolver_prefers_the_host_lib_over_an_x86_64_one() {
        let dll = Pdfium::pdfium_platform_library_name();
        let manifest_dir = tempdir().expect("manifest dir");
        let host_platform = host_platform_resource_dir().expect("a host the installers ship");
        // A foreign platform dir that says x86_64: literally `linux-x86_64`
        // where the host is not that (the repo ships one), else
        // `windows-x86_64`.
        let foreign_platform = if host_platform == "linux-x86_64" {
            "windows-x86_64"
        } else {
            "linux-x86_64"
        };

        let mixed = tempdir().expect("mixed layout");
        let host_lib = mixed
            .path()
            .join("resources")
            .join("lib")
            .join(host_platform)
            .join(&dll);
        let foreign_lib = mixed
            .path()
            .join("resources")
            .join("lib")
            .join(foreign_platform)
            .join(&dll);
        for lib in [&host_lib, &foreign_lib] {
            std::fs::create_dir_all(lib.parent().expect("parent")).expect("mkdir");
            std::fs::write(lib, b"fake").expect("write");
        }
        assert_eq!(
            resolve_bundled_pdfium_path(Some(mixed.path()), manifest_dir.path()),
            Some(strip_windows_prefix(host_lib)),
            "the host-architecture lib must win over the foreign x86_64 one"
        );

        let foreign_only = tempdir().expect("foreign-only layout");
        let foreign_only_lib = foreign_only
            .path()
            .join("resources")
            .join("lib")
            .join(foreign_platform)
            .join(&dll);
        std::fs::create_dir_all(foreign_only_lib.parent().expect("parent")).expect("mkdir");
        std::fs::write(&foreign_only_lib, b"fake").expect("write");
        assert_eq!(
            resolve_bundled_pdfium_path(Some(foreign_only.path()), manifest_dir.path()),
            None,
            "a foreign-architecture lib alone must not resolve: it would shadow the system library"
        );
    }

    /// Nothing bundled (Pro macOS ships no Pdfium): the resolver reports the
    /// absence — the reader then falls back to lopdf and logs it.
    #[test]
    fn the_resolver_reports_absence_when_nothing_is_bundled() {
        let install = tempdir().expect("install dir");
        let manifest_dir = tempdir().expect("manifest dir");
        let resource_dir = install.path().join("Contents").join("Resources");
        std::fs::create_dir_all(&resource_dir).expect("mkdir");

        assert_eq!(
            resolve_bundled_pdfium_path(Some(&resource_dir), manifest_dir.path()),
            None,
            "an empty install must report absence, not invent a path"
        );
        assert_eq!(
            resolve_bundled_pdfium_path(None, manifest_dir.path()),
            None,
            "an empty dev checkout must report absence too"
        );
    }

    /// JD6-B-001: where nothing is bundled the page render may fall back to
    /// an already-hydrated managed runtime copy of the library — and only
    /// that: the injected lookup is the single runtime call, the library must
    /// already be on disk, and nothing may bootstrap.
    #[test]
    fn the_page_render_falls_back_to_an_existing_managed_runtime_copy_without_bootstrapping() {
        let runtime_root = tempdir().expect("runtime root");
        let managed = runtime_root
            .path()
            .join("resources")
            .join("lib")
            .join(Pdfium::pdfium_platform_library_name());
        std::fs::create_dir_all(managed.parent().expect("parent")).expect("mkdir");
        std::fs::write(&managed, b"fake").expect("write");

        let calls = RefCell::new(Vec::new());
        let resolved = existing_managed_pdfium_path_with(|| {
            calls.borrow_mut().push("hydrated_runtime_root");
            Ok(Some(runtime_root.path().to_path_buf()))
        });
        assert_eq!(resolved, Some(strip_windows_prefix(managed)));
        assert_eq!(
            calls.borrow().as_slice(),
            &["hydrated_runtime_root"],
            "the hydrated root is the only runtime call — no bootstrap (JD6-B-001)"
        );

        // A runtime root without the library on disk: absence, not a path.
        let empty_root = tempdir().expect("empty runtime root");
        assert_eq!(
            existing_managed_pdfium_path_with(|| Ok(Some(empty_root.path().to_path_buf()))),
            None,
            "a runtime root without the library is absence"
        );
        assert_eq!(
            existing_managed_pdfium_path_with(|| Ok(None)),
            None,
            "no hydrated runtime at all is absence"
        );
        assert_eq!(
            existing_managed_pdfium_path_with(|| Err("runtime lookup failed".to_string())),
            None,
            "a runtime lookup error is absence, never a failure of the render path"
        );
    }

    /// The runtime-free resolver must never reach the ML runtime bootstrap:
    /// no background sync may trigger `RuntimeManager::ensure_ready_or_bootstrap`
    /// (JD4-B-001). Enforced by a source scan of the resolver chains.
    #[test]
    fn the_bundled_resolver_never_calls_the_ml_runtime_bootstrap() {
        let source =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ocr/pdf.rs"))
                .expect("read pdf.rs");

        // The bundled resolver is strict: it does not reference the ML
        // runtime module at all.
        for fn_name in [
            "ensure_pdfium_path_without_runtime(",
            "ensure_pdfium_path_without_runtime_dir(",
            "ensure_cached_pdfium_path(",
            "resolve_bundled_pdfium_path(",
            "host_pdfium_candidate_paths(",
        ] {
            let body = rust_fn_body(&source, fn_name)
                .unwrap_or_else(|| panic!("{fn_name} must exist in ocr/pdf.rs"));
            for forbidden in ["RuntimeManager", "ensure_ready_or_bootstrap"] {
                assert!(
                    !body.contains(forbidden),
                    "{fn_name} must not reference {forbidden}:\n{body}"
                );
            }
        }
        // The page-render fallback may consult the ML runtime for an
        // already-hydrated copy, but never bootstrap (JD6-B-001).
        for fn_name in [
            "ensure_pdfium_path_with_hydrated_runtime(",
            "existing_managed_pdfium_path(",
            // No `(`: this one is generic (`<H>(...)`), and `rust_fn_body`
            // matches the signature prefix.
            "existing_managed_pdfium_path_with",
        ] {
            let body = rust_fn_body(&source, fn_name)
                .unwrap_or_else(|| panic!("{fn_name} must exist in ocr/pdf.rs"));
            assert!(
                !body.contains("ensure_ready_or_bootstrap"),
                "{fn_name} must not bootstrap the ML runtime:\n{body}"
            );
        }

        // The bibliography call sites must use the runtime-free resolver: the
        // selective OCR page renderer keeps its old pdfium path only through
        // the new resolver (JD5-A-001), with the hydrated-runtime fallback
        // (JD6-B-001) and never the runtime-bootstrapping one; app setup
        // resolves once at boot.
        let processing = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/bibliography/processing.rs"
        ))
        .expect("read processing.rs");
        let renderer = rust_fn_body(&processing, "render_page(&self")
            .expect("ProductionSelectiveOcr::render_page must exist");
        assert!(
            renderer.contains("ensure_pdfium_path_without_runtime"),
            "render_page must resolve pdfium through the runtime-free resolver:\n{renderer}"
        );
        assert!(
            renderer.contains("ensure_pdfium_path_with_hydrated_runtime"),
            "render_page may fall back to the hydrated managed runtime copy (JD6-B-001):\n{renderer}"
        );
        assert!(
            !renderer.contains("init_pdfium_path"),
            "render_page must not reach the runtime-bootstrapping resolver:\n{renderer}"
        );
        assert!(
            !renderer.contains("ensure_ready_or_bootstrap"),
            "render_page must never bootstrap the ML runtime:\n{renderer}"
        );

        let lib = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
            .expect("read lib.rs");
        assert!(
            lib.contains("ensure_pdfium_path_without_runtime"),
            "app setup must resolve the bundled pdfium library at startup"
        );
    }

    /// A populated cache is answered as-is: the resolution runs once (app
    /// setup), and every later reader — the bibliography page reader, each
    /// rendered OCR page — trusts the cached decision without re-resolving.
    #[test]
    fn a_populated_pdfium_cache_is_never_resolved_again() {
        let cached_path = PathBuf::from("bundled").join("pdfium");
        let cache = Mutex::new(Some(cached_path.clone()));

        let resolved =
            ensure_cached_pdfium_path(&cache, || panic!("a populated cache must not re-resolve"));

        assert_eq!(resolved, Some(cached_path));
    }

    /// A cold cache resolves exactly once once a path is found and records
    /// it; where nothing exists the answer is honest absence, and the reader
    /// falls back.
    #[test]
    fn a_cold_pdfium_cache_resolves_once_and_reports_absence() {
        let calls = std::cell::Cell::new(0usize);
        let cache: Mutex<Option<PathBuf>> = Mutex::new(None);
        let found = PathBuf::from("bundled").join("pdfium");

        let first = ensure_cached_pdfium_path(&cache, || {
            calls.set(calls.get() + 1);
            Some(found.clone())
        });
        let second = ensure_cached_pdfium_path(&cache, || {
            calls.set(calls.get() + 1);
            Some(found.clone())
        });

        assert_eq!(first, Some(found.clone()));
        assert_eq!(second, Some(found), "the cached decision is answered");
        assert_eq!(calls.get(), 1, "the second read must not re-resolve");

        // Nothing bundled anywhere: absence is the answer, not an invented path.
        let empty: Mutex<Option<PathBuf>> = Mutex::new(None);
        assert_eq!(
            ensure_cached_pdfium_path(&empty, || None),
            None,
            "absence is the honest answer"
        );
    }

    /// Extracts the body of a top-level `fn` by brace depth. Good enough for
    /// rustfmt'd sources with no unbalanced braces in literals.
    fn rust_fn_body(source: &str, fn_signature: &str) -> Option<String> {
        let start = source.find(&format!("fn {fn_signature}"))?;
        let mut depth = 0usize;
        let mut opened = false;
        for (offset, ch) in source[start..].char_indices() {
            match ch {
                '{' => {
                    opened = true;
                    depth += 1;
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    if opened && depth == 0 {
                        return Some(source[start..start + offset + 1].to_string());
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Loads the real library a bundle ships and renders a page with it. Opt-in:
    /// CI points `ENTROPIA_PDFIUM_PROBE_LIB` at the pinned download
    /// (apps/desktop/scripts/fetch-pdfium.sh) on every platform it bundles for.
    #[test]
    fn pinned_pdfium_library_renders_a_pdf_page() {
        let Ok(lib) = std::env::var("ENTROPIA_PDFIUM_PROBE_LIB") else {
            eprintln!("ENTROPIA_PDFIUM_PROBE_LIB unset; skipping");
            return;
        };
        let bindings = Pdfium::bind_to_library(&lib).expect("bind pinned pdfium");
        let pdfium = Pdfium::new(bindings);
        let mut document = pdfium.create_new_pdf().expect("new pdf");
        document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::a4())
            .expect("page 1");
        document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::a4())
            .expect("page 2");
        let bytes = document.save_to_bytes().expect("save");
        drop(document);
        let loaded = pdfium.load_pdf_from_byte_slice(&bytes, None).expect("load");
        assert_eq!(loaded.pages().len(), 2);
        let page = loaded.pages().get(0).expect("page 0");
        let bitmap = page
            .render_with_config(&PdfRenderConfig::new().set_target_width(200))
            .expect("render");
        assert_eq!(bitmap.width(), 200);
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

    // ── Garbled-text detector: custom-encoded fonts without ToUnicode ─────
    //
    // Two real garbled extractions from a user library: one font shifted by
    // a constant -31 ("3FWJTUB" = "Revista", "-VDIBZPSHBOJ[BDJO" =
    // "Luchayorganización"), one producing symbol soup
    // ("@QDK@BHlMDMSQDBK@RDNAQDQ@XONKgSHB@"). Both pages were classified
    // rich and embedded. The samples below are the garbled page texts,
    // with the quoted fragments verbatim.
    const GARBLED_SHIFTED_PAGE: &str = "3FWJTUB %JDJBMJ[BMF 4FQJFNCJ[BDJ %JSJF[B 1VCJPFT 4B[BDJ 4JFOUJGJDP -VDIBZPSHBOJ[BDJO 6PMJBM 1SJNFSB 4FSJF 5SBKBCPKP %FQBSBNFOUP %F 1TZDPMPHJ[BDJ";
    const GARBLED_SYMBOL_PAGE: &str = "@QDK@BHlMDMSQDBK@RDNAQDQ@XONKgSHB@ wKl@RDN@QDK@BHlMDMSQDBK gSHB@RDNAQDQ@XONK wDMSQDBK@RDN@BHlMDMSQDB";

    #[test]
    fn the_reported_garbled_pages_are_detected() {
        assert!(
            is_garbled_text(GARBLED_SHIFTED_PAGE),
            "the -31-shift page must be garbled: {GARBLED_SHIFTED_PAGE}"
        );
        assert!(
            is_garbled_text(GARBLED_SYMBOL_PAGE),
            "the symbol-soup page must be garbled: {GARBLED_SYMBOL_PAGE}"
        );
    }

    #[test]
    fn the_literal_garbled_fragments_are_detected() {
        assert!(is_garbled_text("3FWJTUB"));
        assert!(is_garbled_text("-VDIBZPSHBOJ[BDJO"));
        assert!(is_garbled_text("@QDK@BHlMDMSQDBK@RDNAQDQ@XONKgSHB@"));
    }

    #[test]
    fn normal_spanish_and_english_academic_text_is_not_garbled() {
        let spanish = "El presente trabajo analiza el efecto de la intervencion educativa sobre el rendimiento academico de los estudiantes de secundaria. Se utilizo un diseno cuasiexperimental con una muestra de 240 participantes distribuidos en dos grupos: control y experimental. Los resultados muestran mejoras significativas en comprension lectora (p < 0,01) y en motivacion escolar.";
        let english = "This paper presents a controlled study of bilingual reading comprehension across two instructional conditions. Participants completed standardized vocabulary and fluency measures before and after a twelve week intervention. Mixed effects models revealed a reliable main effect of condition, with no interaction involving prior proficiency (beta = 0.42, SE = 0.08).";
        assert!(!is_garbled_text(spanish), "{spanish}");
        assert!(!is_garbled_text(english), "{english}");
    }

    #[test]
    fn all_caps_titles_and_short_headings_are_not_garbled() {
        let title_page = "REVISTA DE PSICOLOGIA APLICADA A LA EDUCACION\nVOLUMEN XII - NUMERO 3 - SEPTIEMBRE 2019\nUNIVERSIDAD NACIONAL AUTONOMA DE MEXICO\nFACULTAD DE CIENCIAS SOCIALES";
        assert!(!is_garbled_text(title_page), "{title_page}");
        assert!(!is_garbled_text("STRENGTH AND LIMITS"));
        assert!(!is_garbled_text("RESULTADOS"));
    }

    #[test]
    fn tables_of_numbers_are_not_garbled() {
        let table = "Tabla 3\n12 45.6 789\n33 21.0 456\n17 90.2 321";
        assert!(!is_garbled_text(table), "{table}");
        assert!(!is_garbled_text("10.1016/j.bbr.2019.109315"));
    }

    #[test]
    fn references_with_dois_urls_and_emails_are_not_garbled() {
        let references = "Garcia, J. R., and Perez, M. (2019). Aprendizaje automatico en contextos educativos. Revista de Educacion, 45(2), 123-145. https://doi.org/10.1016/j.edurev.2019.05.003\nSmith, A. B., and Jones, C. D. (2021). Bilingual reading comprehension revisited. Journal of Applied Linguistics, 33(4), 512-530. https://doi.org/10.1080/02664623.2021.1876543\nCorrespondence: a.garcia@csic.es; see also https://www.sciencedirect.com/science/article/pii/S0149763419305291";
        assert!(!is_garbled_text(references), "{references}");
    }

    #[test]
    fn ligature_rich_text_is_not_garbled() {
        let ligatures = "The final flow of fluid in fluctuating fields (fluorine compounds) shows consistent effects; fissure patterns and flame dynamics were analysed with the aeolian model.";
        assert!(!is_garbled_text(ligatures), "{ligatures}");
    }

    #[test]
    fn garbled_text_is_never_quality_text() {
        assert!(!is_quality_text(GARBLED_SHIFTED_PAGE));
        assert!(!is_quality_text(GARBLED_SYMBOL_PAGE));
    }

    // ── Bibliography garble detector (B2, plan-texto-nativo 3.4) ──────────
    //
    // Two rules, both pure text statistics. Rule 1 (glued words) catches
    // the lopdf page layer that never saw a space; rule 2 (old OCR noise)
    // catches the dotted artifacts of old recognizers in SPACED text. The
    // p. 309 fixtures are verbatim from one user library (Abulafia 1950,
    // stored row and PDFium read); the rest pin every false-positive shape
    // the plan names. `is_garbled_text`/`is_quality_text` above are a
    // different detector for raw glyph codes and must not change.

    /// Verbatim prefix of the stored `bibliographic_page_texts` row for
    /// Abulafia 1950, p. 309 (353 chars): lopdf glued the words.
    const GARBLED_BIBLIOGRAPHY_STORED_P309: &str =
        "ArrozElcultivodelarrozaligualqueeldelg\u{ed}.r'asoL haadquiridounincrementoe\
        xtraordinarioara\u{ed}zdela\u{fa}ltimaguerr-amundial;no a.l canaandolaproduc\
        ci\u{f3}nnacionalparacubrirlasnecesidadesdelconsumoin-terno,deb.\u{ed}.endo \
        r-ecur-r\u{ed}.r-ae\u{e9}l.laLmpor-tac\u{ed}.\u{f3}nparacu-br-\u{ed}.rlosd\
        \u{e9}ficitequeseproducen.Lasuperfi~iemediacultiv\u{e9}ldaconarrozparaelquin\
        quenio1939/40-1943/44fu\u{e9}de38.234Ha.";

    /// Verbatim 600-char excerpt of PDFium's read of the same page: the
    /// words are spaced again, but the old OCR layer leaves dotted noise
    /// behind ("g \u{ed}.r'as oL", "superfi~ie", "1943/4~\u{b7}").
    const GARBLED_BIBLIOGRAPHY_PDFIUM_P309: &str =
        "Arroz\u{a}El cultivo del arroz al igual que el del g \u{ed}.r'as oL\
        \u{a}ha adquirido un incremento extraordinario a ra\u{ed}z de\u{a}la \
        \u{fa}ltima gue rr-a mundial; no a.l canaando la producci\u{f3}n\u{a}n\
        acional para cubrir las necesidades del consumo in\u{2}terno, de b.\
        \u{ed}.endo r-ecur-r\u{ed}.r-ae \u{e9}l. la Lmpor-t ac \u{ed}.\u{f3}n \
        para cu\u{2}br-\u{ed}.r los d\u{e9}ficite que se producen.\u{a}La supe\
        rfi~ie media cultiv\u{e9}lda con arroz para el\u{a}quinquenio 1939/40 \
        - 1943/44 fu\u{e9} de 38.234 Ha ., y p_-\u{a}r a la cose cha 1943/44 d\
        e 52.272 hect\u{e1}reas; el rendi\u{2}miento medio del quinquenio indi\
        cado fu\u{e9} de 3.122 ki\u{2}logramos por hect\u{e1}rea, siendo del d\
        e le c os e cha 1943/4~\u{b7},\u{a}de 3.";

    /// Old OCR noise in text that still has its spaces: dotted and tilde
    /// artifacts inside words, on a real Spanish page shape.
    const OLD_OCR_NOISE_PARAGRAPH: &str = "Arroz El cultivo del arroz al igual que el del girasol ha adquirido un incremento extraordinario a raiz de la ultima guerra mundial; no al.cana.ndo la pro.ducc.ion nacional para cubrir las necesidades del consumo interno, deb.\u{ed}.endo r.ecu.rrir a la impor.tac.ion para cubrir los deficit que se producen. La superfi~ie media cultiv.ada con arroz para el quinquenio indicado fue de 38.234 Ha., y para la cosecha de 52.272 hectareas; el rendi.mien.to medio del quinquenio fue de 3.122 kilogramos por hectarea, siendodel de la cosecha 4~\u{b7}, de 3.340 kg. La produccion media fue de 104.230 toneladas, correspondiendo a la cosecha maxima de 161.000 toneladas. Ese volumen con alguna variacion se mantiene para las campafias siguientes, donde se ve que la campana siguientellega a 139,6 mil toneladas; en la 1945/46 el 157,9 mil toneladas; en la 1946/47 a 160,6 miltoneladas";

    #[test]
    fn the_stored_p309_page_is_flagged_by_the_glued_words_rule() {
        let flags = garbled_bibliography_flags(GARBLED_BIBLIOGRAPHY_STORED_P309);
        assert!(
            flags.glued_words,
            "rule 1 must flag the stored p. 309 text: {GARBLED_BIBLIOGRAPHY_STORED_P309}"
        );
        assert!(
            !flags.old_ocr_noise,
            "rule 2 is not what flags the stored p. 309 text"
        );
        assert!(is_garbled_bibliography_text(
            GARBLED_BIBLIOGRAPHY_STORED_P309
        ));
    }

    #[test]
    fn the_pdfium_read_of_p309_is_flagged_by_the_old_ocr_noise_rule() {
        // The plan's contract on the real pair: rule 1 flags the stored
        // glued text, rule 2 flags the same page once PDFium spaces it.
        let flags = garbled_bibliography_flags(GARBLED_BIBLIOGRAPHY_PDFIUM_P309);
        assert!(
            flags.old_ocr_noise,
            "rule 2 must flag the PDFium read of p. 309"
        );
        assert!(!flags.glued_words, "PDFium already spaced the words");
        assert!(is_garbled_bibliography_text(
            GARBLED_BIBLIOGRAPHY_PDFIUM_P309
        ));
    }

    #[test]
    fn old_ocr_noise_is_flagged_in_text_that_still_has_spaces() {
        let flags = garbled_bibliography_flags(OLD_OCR_NOISE_PARAGRAPH);
        assert!(
            flags.old_ocr_noise,
            "rule 2 must flag the dotted old-OCR paragraph"
        );
        assert!(!flags.glued_words, "the words here are not glued");
        assert!(is_garbled_bibliography_text(OLD_OCR_NOISE_PARAGRAPH));
    }

    #[test]
    fn clean_english_prose_with_compounds_and_contractions_is_not_flagged() {
        // Hyphens and apostrophes never count as noise, and no word run is
        // anywhere near 24 letters.
        let prose = "This state-of-the-art review examines how bilingual readers don't just translate words; they negotiate meaning across two lexical systems. Well-established findings -- e.g. the interaction between vocabulary size and reading fluency -- hold across the L1 and the L2, and it's been shown repeatedly that a reader's self-efficacy matters as much as raw comprehension. Section 3.2 discusses the trade-off between speed and accuracy; readers who slow down at clause boundaries make fewer inference errors, but they also report higher fatigue. The counter-argument, i.e. that speed drills damage comprehension, isn't supported by the controlled studies surveyed here. Overall, the evidence points to a balanced approach: extensive reading, targeted vocabulary work, and metacognitive strategy training, each contributing independently to the outcome measures.";
        assert!(!is_garbled_bibliography_text(prose), "{prose}");
    }

    #[test]
    fn reference_lists_with_links_dois_and_domains_are_not_flagged() {
        // Without the URL/DOI/email/domain drop the long link tokens would
        // drag rule 2 up past its threshold (the dotted host names).
        let references = "Garcia, J. R., and Perez, M. (2019). Aprendizaje automatico en contextos educativos. Revista de Educacion, 45(2), 123-145. https://doi.org/10.1016/j.edurev.2019.05.003\nSmith, A. B., and Jones, C. D. (2021). Bilingual reading comprehension revisited. Journal of Applied Linguistics, 33(4), 512-530. https://doi.org/10.1080/02664623.2021.1876543\nKumar, R. (2018). Archives and readers. Available at www.jstor.org/stable/26612345 (accessed 4 May 2022). doi:10.1086/26612345. Correspondence: a.garcia@csic.es.\nFurther data at https://dataverse.harvard.edu/dataset.xhtml?persistentId=hdl:1902.1/12345 and mirrored on en.wikipedia.org/wiki/Reading_comprehension; see also the repository at github.com/open-science/reading-corpus (documentation under docs/).";
        assert!(!is_garbled_bibliography_text(references), "{references}");
    }

    #[test]
    fn markdown_tables_are_not_flagged() {
        let table = "| Region | Yield (kg/ha) | Source |\n| --- | --- | --- |\n| Delta | 3.122 | jstor.org |\n| Coast | 3.340 | doi.org |\n| Valley | 2.980 | nara.gov |\n\nThe table above reports the survey averages; the row for Valley combines two seasons, 1939/40 and 1943/44, so the figures are not directly comparable with the others.";
        assert!(!is_garbled_bibliography_text(table), "{table}");
    }

    #[test]
    fn cjk_prose_without_spaces_is_not_flagged() {
        // Spaceless scripts have almost no Latin letters, so rule 1's
        // 80-letter floor keeps the detector away from them.
        let cjk = "\u{6c34}\u{7a3b}\u{683d}\u{57f9}\u{7684}\u{5386}\u{53f2}\u{53ef}\u{4ee5}\u{8ffd}\u{6eaf}\u{5230}\u{6570}\u{5343}\u{5e74}\u{524d}\u{3002}\u{957f}\u{6c5f}\u{4e2d}\u{4e0b}\u{6e38}\u{5730}\u{533a}\u{7684}\u{8003}\u{53e4}\u{8bc1}\u{636e}\u{8868}\u{660e}\u{ff0c}\u{53e4}\u{4eba}\u{5df2}\u{7ecf}\u{638c}\u{63e1}\u{4e86}\u{704c}\u{6e89}\u{4e0e}\u{80b2}\u{79cd}\u{6280}\u{672f}\u{3002}\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{6587}\u{732e}\u{306b}\u{3088}\u{308c}\u{3070}\u{3001}\u{6c34}\u{7530}\u{306e}\u{7ba1}\u{7406}\u{306b}\u{306f}\u{7d30}\u{5fc3}\u{306e}\u{6ce8}\u{610f}\u{304c}\u{6c42}\u{3081}\u{3089}\u{308c}\u{308b}\u{3002}modern varieties \u{3068} traditional landraces \u{306f}\u{7523}\u{91cf}\u{3001}\u{6297}\u{75c5}\u{6027}\u{3068}\u{53e3}\u{611f}\u{306b}\u{660e}\u{697a}\u{306a}\u{5dee}\u{7570}\u{304c}\u{3042}\u{308b}\u{3002}";
        assert!(!is_garbled_bibliography_text(cjk), "{cjk}");
    }

    #[test]
    fn catalan_geminate_l_dot_l_is_not_flagged() {
        // Every l·l here is a Catalan geminate, not OCR noise; without that
        // exclusion this paragraph would flag rule 2.
        let catalan = "La confer\u{e8}ncia paral\u{b7}lela va presentar un excel\u{b7}lent treball sobre el paral\u{b7}lelisme cultural entre les comunitats costaneres. Els autors van argumentar que la traducci\u{f3} no \u{e9}s una operaci\u{f3} mec\u{e0}nica, sin\u{f3} un proc\u{e9}s de negociaci\u{f3} entre lleng\u{fc}es. La seva an\u{e0}lisi del paral\u{b7}lelisme textual va mostrar com els lectors catalans resolen les ambig\u{fc}itats l\u{e8}xiques amb estrat\u{e8}gies diferents de les dels castellans. Aquesta conclusi\u{f3} refor\u{e7}a la hip\u{f2}tesi inicial i obre noves l\u{ed}nies de recerca sobre l'adquisici\u{f3} l\u{e8}xica en entorns bilingues, on el contacte ling\u{fc}\u{ed}stic \u{e9}s constant i la interfer\u{e8}ncia es veu en paraules com col\u{b7}laboraci\u{f3}, il\u{b7}lustraci\u{f3}, paral\u{b7}lelament, excel\u{b7}lentment i col\u{b7}lecci\u{f3}.";
        assert!(!is_garbled_bibliography_text(catalan), "{catalan}");
    }

    #[test]
    fn dotted_abbreviations_are_not_flagged() {
        // Uppercase dotted abbreviations and e.g./i.e. never count as noise;
        // without those exclusions this page would flag rule 2.
        let abbreviations = "The N.A.T.O. council met in the U.S.A. to review the joint programme, e.g. the coastal survey, i.e. the 1952 transect, and the U.S. Navy archive kept in the capital. Delegations from the U.S.A., the N.A.T.O. offices and the E.U. commission attended. Notes were taken by the O.A.S. secretariat and the O.E.C.D. observers, who visited the U.S.A. again in June. The final report, circulated to the N.A.T.O. archives and the U.S. Navy library, summarises the programme, e.g. the survey results, i.e. the 1952 transect data. The U.S.A. delegation praised the O.E.C.D. review and the E.U. commission's contribution to the N.A.T.O. meeting held in the U.S.A. that autumn.";
        assert!(
            !is_garbled_bibliography_text(abbreviations),
            "{abbreviations}"
        );
    }

    #[test]
    fn each_rule_needs_its_minimum_evidence() {
        // Rule 1 below 80 Latin letters: silence.
        let short_glued = "ArrozElcultivodelarrozaligualqueeldelgirasoL";
        assert!(!garbled_bibliography_flags(short_glued).glued_words);
        // Rule 2 below 40 eligible tokens of 4+ letters: silence.
        let sparse_noise =
            "deb.\u{ed}.endo superfi~ie cultiv.ada r.ecu.rrir impor.tac.ion defi.cit cober.tura";
        assert!(!garbled_bibliography_flags(sparse_noise).old_ocr_noise);
    }

    #[test]
    fn the_rules_flag_exactly_at_their_thresholds() {
        // Rule 1: exactly half the Latin letters in a >24-letter run flags;
        // one letter less does not.
        let suffix = ["ab"; 25].join(" "); // 50 letters in short words
        assert!(garbled_bibliography_flags(&format!("{} {suffix}", "a".repeat(50))).glued_words);
        assert!(!garbled_bibliography_flags(&format!("{} {suffix} x", "a".repeat(49))).glued_words);
        // Rule 2: exactly 8 % noisy tokens flags; one noisy token less does
        // not (4/50 vs 3/50).
        let clean = (0..46)
            .map(|n| format!("palabra{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        let noisy = ["pq.rstu"; 4].join(" ");
        assert!(
            garbled_bibliography_flags(&format!("{noisy} {clean}")).old_ocr_noise,
            "4 of 50 is exactly 8 %"
        );
        let clean = (0..47)
            .map(|n| format!("palabra{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        let noisy = ["pq.rstu"; 3].join(" ");
        assert!(
            !garbled_bibliography_flags(&format!("{noisy} {clean}")).old_ocr_noise,
            "3 of 50 is below 8 %"
        );
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
