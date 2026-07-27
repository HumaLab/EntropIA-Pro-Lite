/// Tauri IPC commands for OCR operations.
use super::{update_extraction_text, OcrQueue};
use crate::db::state::AppDbState;
use crate::nlp::NlpQueue;
use crate::path_utils::normalize_windows_path_string;
use serde::Serialize;
use tauri::{AppHandle, State};

/// A single rendered PDF page returned by `render_pdf_pages_cmd`.
#[derive(Clone, Serialize)]
pub struct RenderedPage {
    pub page_number: u32,
    pub png_path: String,
}

/// A single-page PDF produced by splitting a multi-page PDF.
///
/// Returned by `split_pdf_pages`. Each entry is one page of the original
/// document, preserved as an independent PDF (no rasterization).
#[derive(Clone, Serialize)]
pub struct SplitPage {
    pub page_number: u32,
    pub pdf_path: String,
}

#[derive(Debug, serde::Serialize)]
pub struct PdfCropResult {
    pub path: String,
    pub size: u64,
}

fn next_pdf_crop_path(path: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(crate::image_edit::next_version_path(path, None))
}

fn write_rendered_pages_with<R, F, W>(
    output_dir: &std::path::Path,
    filename_prefix: &str,
    render_pages: R,
    mut create_writer: F,
) -> Result<Vec<RenderedPage>, String>
where
    R: FnOnce(&mut dyn FnMut(usize, usize, &[u8]) -> Result<(), String>) -> Result<usize, String>,
    F: FnMut(&std::path::Path) -> std::io::Result<W>,
    W: std::io::Write,
{
    let mut rendered_pages = Vec::new();
    let mut write_error = None;
    let traversal = render_pages(&mut |page_idx, page_count, png_data| {
        if page_idx == 0 {
            eprintln!("[render_pdf_pages] Rendering {page_count} pages from PDF");
        }

        let page_number = (page_idx + 1) as u32;
        let filename = format!("{filename_prefix}_page_{page_number}.png");
        let file_path = output_dir.join(filename);
        let write_result = (|| {
            let mut file = create_writer(&file_path)
                .map_err(|e| format!("Failed to create PNG file for page {page_number}: {e}"))?;
            file.write_all(png_data)
                .map_err(|e| format!("Failed to write PNG data for page {page_number}: {e}"))
        })();

        if let Err(error) = write_result {
            write_error = Some(error.clone());
            return Err(error);
        }

        rendered_pages.push(RenderedPage {
            page_number,
            png_path: normalize_windows_path_string(&file_path),
        });
        eprintln!("[render_pdf_pages] Rendered page {page_number}/{page_count}");
        Ok(())
    });

    traversal.map_err(|error| {
        write_error.unwrap_or_else(|| format!("Failed to render PDF pages: {error}"))
    })?;

    Ok(rendered_pages)
}

/// Submit an OCR extraction job to the background worker queue.
///
/// Returns immediately with `Ok("queued")`. The worker will process the job
/// asynchronously and emit `ocr:progress`, `ocr:complete`, or `ocr:error` events.
///
/// # Arguments
/// * `asset_id`   — unique ID of the asset in the database
/// * `asset_path` — absolute filesystem path to the asset file
/// * `asset_type` — `"pdf"` or `"image"`
/// * `mode`       — `"light"` (plain PaddleOCR, default) or `"high"` (PaddleVL → PaddleOCR)
/// * `ocr_queue`  — managed state injected by Tauri
#[tauri::command]
pub async fn extract_text(
    asset_id: String,
    asset_path: String,
    asset_type: String,
    mode: Option<String>,
    app_handle: AppHandle,
    ocr_queue: State<'_, OcrQueue>,
    db: State<'_, AppDbState>,
) -> Result<String, String> {
    let ocr_mode = match mode.as_deref() {
        Some("high") => super::OcrMode::High,
        _ => super::OcrMode::Light, // default to light
    };

    if ocr_mode == super::OcrMode::High {
        let conn = db
            .ui_conn
            .lock()
            .map_err(|e| format!("DB lock poisoned: {e}"))?;
        super::ensure_selected_cloud_key(&conn)?;
    }

    crate::app_logs::info(
        &app_handle,
        "ocr",
        format!(
            "Trabajo OCR encolado: asset_id={}, tipo={}, modo={:?}",
            asset_id, asset_type, ocr_mode
        ),
    );

    let job = super::OcrJob {
        asset_id,
        asset_path,
        asset_type,
        mode: ocr_mode,
    };

    ocr_queue.submit(job)?;
    Ok("queued".to_string())
}

/// Materialize a normalized region from one PDF page as a standalone PDF.
#[tauri::command]
pub async fn crop_pdf(
    path: String,
    page: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    app_handle: tauri::AppHandle,
) -> Result<PdfCropResult, String> {
    if page == 0 {
        return Err("PDF page numbers are 1-based".to_string());
    }

    super::pdf::init_pdfium_path(&app_handle);
    tokio::task::spawn_blocking(move || {
        let source_path = std::path::PathBuf::from(&path);
        let source_bytes =
            std::fs::read(&source_path).map_err(|e| format!("Failed to read PDF file: {e}"))?;
        let cropped_bytes = super::pdf::crop_pdf_to_single_page_bytes(
            &source_bytes,
            (page - 1) as usize,
            x,
            y,
            width,
            height,
        )?;
        let output_path = next_pdf_crop_path(&path);
        std::fs::write(&output_path, &cropped_bytes)
            .map_err(|e| format!("Failed to write cropped PDF: {e}"))?;

        Ok(PdfCropResult {
            path: normalize_windows_path_string(&output_path),
            size: cropped_bytes.len() as u64,
        })
    })
    .await
    .map_err(|e| format!("PDF crop task panicked: {e}"))?
}

#[tauri::command]
pub async fn test_glm_ocr_connection(api_key: String) -> Result<(), String> {
    super::glm_ocr::GlmOcrClient::new(api_key)
        .test_connection()
        .await
}

/// Update the text_content of the latest extraction for an asset.
///
/// This allows users to manually correct OCR output and persist the correction.
/// The original extraction metadata (id, created_at, method, confidence) is preserved.
#[tauri::command]
pub async fn update_extraction_text_cmd(
    asset_id: String,
    text_content: String,
    db: State<'_, AppDbState>,
    _nlp_queue: State<'_, NlpQueue>,
) -> Result<(), String> {
    let conn = db
        .ui_conn
        .lock()
        .map_err(|e| format!("DB lock poisoned: {e}"))?;
    update_extraction_text(&conn, &asset_id, &text_content)?;

    Ok(())
}

/// Generate a thumbnail PNG for the first page of a PDF.
///
/// Returns the filesystem path to the cached thumbnail. The frontend should
/// use `convertFileSrc()` to turn this path into a webview-accessible URL.
///
/// Thumbnails are cached at `{app_data_dir}/thumbnails/{asset_id}.png`.
/// If a cached thumbnail already exists, the cached path is returned immediately
/// without re-rendering.
#[tauri::command]
pub async fn generate_pdf_thumbnail(
    asset_path: String,
    asset_id: String,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    use std::io::Write;
    use tauri::Manager;

    // Ensure Pdfium DLL path is initialized before any PDF operations.
    // This is a no-op if already called by the OCR worker; safe to call multiple times.
    super::pdf::init_pdfium_path(&app_handle);

    // Resolve thumbnails directory
    let app_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;

    let thumb_dir = app_dir.join("thumbnails");
    std::fs::create_dir_all(&thumb_dir)
        .map_err(|e| format!("Failed to create thumbnails directory: {e}"))?;

    let thumb_path = thumb_dir.join(format!("{asset_id}.png"));

    // Return cached thumbnail immediately if it exists
    if thumb_path.exists() {
        return Ok(normalize_windows_path_string(&thumb_path));
    }

    // Read PDF and render thumbnail in a blocking task
    // (pdfium is CPU-intensive and must not block the async runtime)
    let result_path = tokio::task::spawn_blocking(move || {
        let bytes =
            std::fs::read(&asset_path).map_err(|e| format!("Failed to read PDF file: {e}"))?;

        let png_data = super::pdf::render_pdf_thumbnail(&bytes)?;

        // Write thumbnail to disk
        let mut file = std::fs::File::create(&thumb_path)
            .map_err(|e| format!("Failed to create thumbnail file: {e}"))?;
        file.write_all(&png_data)
            .map_err(|e| format!("Failed to write thumbnail data: {e}"))?;

        Ok::<String, String>(normalize_windows_path_string(&thumb_path))
    })
    .await
    .map_err(|e| format!("Thumbnail generation task panicked: {e}"))??;

    Ok(result_path)
}

/// Generate or retrieve a cached bounded thumbnail for an image asset.
///
/// The path hash is part of the filename so edited assets that receive a new
/// file path do not reuse stale thumbnails for the same asset ID.
#[tauri::command]
pub async fn generate_image_thumbnail(
    asset_path: String,
    asset_id: String,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    use image::ImageFormat;
    use sha2::{Digest, Sha256};
    use std::io::Cursor;
    use tauri::Manager;

    let app_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;

    let thumb_dir = app_dir.join("thumbnails");
    std::fs::create_dir_all(&thumb_dir)
        .map_err(|e| format!("Failed to create thumbnails directory: {e}"))?;

    let path_hash = Sha256::digest(asset_path.as_bytes());
    let thumb_name = format!("image-{asset_id}-{path_hash:x}.png");
    let thumb_path = thumb_dir.join(thumb_name);

    if thumb_path.exists() {
        return Ok(normalize_windows_path_string(&thumb_path));
    }

    let result_path = tokio::task::spawn_blocking(move || {
        let image = image::ImageReader::open(&asset_path)
            .map_err(|e| format!("Failed to open image file: {e}"))?
            .with_guessed_format()
            .map_err(|e| format!("Failed to detect image format: {e}"))?
            .decode()
            .map_err(|e| format!("Failed to decode image file: {e}"))?;

        let thumbnail = image.thumbnail(400, 400);
        let mut png_data = Vec::new();
        thumbnail
            .write_to(&mut Cursor::new(&mut png_data), ImageFormat::Png)
            .map_err(|e| format!("Failed to encode image thumbnail: {e}"))?;

        std::fs::write(&thumb_path, png_data)
            .map_err(|e| format!("Failed to write image thumbnail: {e}"))?;

        Ok::<String, String>(normalize_windows_path_string(&thumb_path))
    })
    .await
    .map_err(|e| format!("Image thumbnail generation task panicked: {e}"))??;

    Ok(result_path)
}

/// Delete cached image thumbnails for an asset. Best-effort cleanup when assets
/// are removed; stale path-version thumbnails are harmless if removal fails.
#[tauri::command]
pub async fn delete_image_thumbnail(
    asset_id: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    use tauri::Manager;

    let app_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;

    let thumb_dir = app_dir.join("thumbnails");
    if !thumb_dir.exists() {
        return Ok(());
    }

    let prefix = format!("image-{asset_id}-");
    for entry in std::fs::read_dir(&thumb_dir)
        .map_err(|e| format!("Failed to read thumbnails directory: {e}"))?
    {
        let entry = entry.map_err(|e| format!("Failed to read thumbnail entry: {e}"))?;
        let filename = entry.file_name();
        let filename = filename.to_string_lossy();
        if filename.starts_with(&prefix) && filename.ends_with(".png") {
            std::fs::remove_file(entry.path())
                .map_err(|e| format!("Failed to delete image thumbnail: {e}"))?;
        }
    }

    Ok(())
}

/// Delete a cached PDF thumbnail for an asset.
///
/// Called when a PDF asset is deleted to clean up the thumbnail cache.
/// Returns `Ok(())` even if the file doesn't exist (ENOENT is OK).
#[tauri::command]
pub async fn delete_pdf_thumbnail(
    asset_id: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    use tauri::Manager;

    let app_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;

    let thumb_path = app_dir.join("thumbnails").join(format!("{asset_id}.png"));

    if thumb_path.exists() {
        std::fs::remove_file(&thumb_path)
            .map_err(|e| format!("Failed to delete thumbnail: {e}"))?;
    }

    Ok(())
}

/// Check whether a PDF is scanned (image-only) by testing if its native text
/// layer passes quality checks.
///
/// Returns `true` if the PDF has insufficient native text (likely scanned/image-only)
/// and should be split into per-page image assets during import.
#[tauri::command]
pub async fn is_scanned_pdf(
    asset_path: String,
    app_handle: tauri::AppHandle,
) -> Result<bool, String> {
    // Ensure Pdfium is initialized
    super::pdf::init_pdfium_path(&app_handle);

    let bytes = tokio::task::spawn_blocking(move || std::fs::read(&asset_path))
        .await
        .map_err(|e| format!("Failed to read PDF file: {e}"))?
        .map_err(|e| format!("Failed to read PDF file: {e}"))?;

    let is_scanned = tokio::task::spawn_blocking(move || {
        // Try native text extraction first
        let native_text = super::pdf::extract_pdf_text(&bytes);
        match native_text {
            Ok(text) => !super::pdf::is_quality_text(&text),
            Err(_) => true, // If extraction itself fails, treat as scanned
        }
    })
    .await
    .map_err(|e| format!("PDF check task panicked: {e}"))?;

    Ok(is_scanned)
}

/// Build a conservative per-page profile for a PDF.
///
/// Only confidently native documents should stay as PDF; mixed, uncertain,
/// image-only, and image-with-OCR documents should be rendered as image pages.
/// The frontend invokes this as `probe_pdf` with `{ assetPath }` and expects a
/// `DocumentProfile` (see `apps/desktop/src/lib/file-import.ts`).
#[tauri::command]
pub async fn probe_pdf(
    asset_path: String,
    app_handle: tauri::AppHandle,
) -> Result<super::pdf_probe::DocumentProfile, String> {
    // Ensure Pdfium DLL path is initialized before any PDF operations.
    super::pdf::init_pdfium_path(&app_handle);

    let bytes = tokio::task::spawn_blocking(move || std::fs::read(&asset_path))
        .await
        .map_err(|e| format!("Failed to read PDF file: {e}"))?
        .map_err(|e| format!("Failed to read PDF file: {e}"))?;

    // Pdfium work is blocking; profile off the async runtime.
    tokio::task::spawn_blocking(move || super::pdf_probe::profile_pdf_bytes(&bytes))
        .await
        .map_err(|e| format!("PDF profile task panicked: {e}"))?
}

/// Render all pages of a PDF as PNG images and save them to disk.
///
/// Used by the frontend import flow to convert scanned PDFs into per-page
/// image assets. Each page is rendered at 300 DPI (target width 2550px),
/// saved as a PNG file in the specified output directory.
///
/// # Arguments
/// * `pdf_path` — Absolute filesystem path to the source PDF file
/// * `output_dir` — Directory where PNG files will be saved (created if missing)
/// * `filename_prefix` — Prefix for output filenames (e.g., "document" → "document_page_1.png")
///
/// # Returns
/// A list of `RenderedPage` objects with page numbers and absolute file paths.
#[tauri::command]
pub async fn render_pdf_pages(
    pdf_path: String,
    output_dir: String,
    filename_prefix: String,
    app_handle: tauri::AppHandle,
) -> Result<Vec<RenderedPage>, String> {
    // Ensure Pdfium is initialized
    super::pdf::init_pdfium_path(&app_handle);

    // Create output directory if it doesn't exist
    let out_dir = std::path::PathBuf::from(&output_dir);
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("Failed to create output directory: {e}"))?;

    // Read PDF and render pages in a blocking task.
    let pages = tokio::task::spawn_blocking(move || {
        let bytes =
            std::fs::read(&pdf_path).map_err(|e| format!("Failed to read PDF file: {e}"))?;

        // Check if PDF has quality text (skip rendering if it's a text PDF)
        // This is a safety check — the frontend should only call this for scanned PDFs
        let native_text = super::pdf::extract_pdf_text(&bytes);
        if let Ok(ref text) = native_text {
            if super::pdf::is_quality_text(text) {
                return Err(
                    "PDF has quality native text — not a scanned PDF. Use as PDF asset instead."
                        .to_string(),
                );
            }
        }

        write_rendered_pages_with(
            &out_dir,
            &filename_prefix,
            |visitor| super::pdf::render_pdf_pages_with(&bytes, visitor),
            |path| std::fs::File::create(path),
        )
    })
    .await
    .map_err(|e| format!("PDF render task panicked: {e}"))??;

    Ok(pages)
}

/// Split a multi-page PDF into one single-page PDF file per page.
///
/// Each page is preserved as an independent PDF — no rasterization, no
/// recompression. The frontend uses this at import time to decompose a
/// multi-page PDF into one PDF asset per page, keeping the original as the
/// parent. Page order is preserved (1-based `page_number`).
///
/// # Arguments
/// * `pdf_path` - Absolute path to the source PDF.
/// * `output_dir` - Directory where single-page PDFs will be written.
/// * `filename_prefix` - Filename prefix (`"doc"` → `doc_page_1.pdf`).
///
/// # Returns
/// A list of `SplitPage` with page numbers and absolute file paths.
#[tauri::command]
pub async fn split_pdf_pages(
    pdf_path: String,
    output_dir: String,
    filename_prefix: String,
    app_handle: tauri::AppHandle,
) -> Result<Vec<SplitPage>, String> {
    // Ensure Pdfium is initialized before any PDF operations.
    super::pdf::init_pdfium_path(&app_handle);

    let out_dir = std::path::PathBuf::from(&output_dir);
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("Failed to create output directory: {e}"))?;

    let pages = tokio::task::spawn_blocking(move || -> Result<Vec<SplitPage>, String> {
        let bytes =
            std::fs::read(&pdf_path).map_err(|e| format!("Failed to read PDF file: {e}"))?;

        let split = super::pdf::split_pdf_to_single_page_bytes(&bytes)?;

        let mut rendered = Vec::with_capacity(split.len());
        for (page_number, pdf_bytes) in split {
            let filename = format!("{filename_prefix}_page_{page_number}.pdf");
            let file_path = out_dir.join(filename);
            std::fs::write(&file_path, &pdf_bytes).map_err(|e| {
                format!("Failed to write single-page PDF for page {page_number}: {e}")
            })?;
            rendered.push(SplitPage {
                page_number,
                pdf_path: normalize_windows_path_string(&file_path),
            });
        }

        Ok(rendered)
    })
    .await
    .map_err(|e| format!("PDF split task panicked: {e}"))??;

    Ok(pages)
}

/// Return the number of pages in a PDF file.
///
/// The import flow uses this to decide whether a PDF needs to be split into
/// per-page assets (multi-page) or kept as a single asset (single-page).
#[tauri::command]
pub async fn count_pdf_pages(
    asset_path: String,
    app_handle: tauri::AppHandle,
) -> Result<u32, String> {
    super::pdf::init_pdfium_path(&app_handle);

    let bytes = tokio::task::spawn_blocking(move || std::fs::read(&asset_path))
        .await
        .map_err(|e| format!("PDF page count task panicked: {e}"))?
        .map_err(|e| format!("Failed to read PDF file: {e}"))?;

    tokio::task::spawn_blocking(move || super::pdf::pdf_page_count(&bytes))
        .await
        .map_err(|e| format!("PDF page count task panicked: {e}"))?
        .map(|count| u32::try_from(count).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::pdf::{visit_rendered_pages, RenderedPageSource};
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn pdf_crop_uses_short_incrementing_version_names() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("acta.pdf");
        std::fs::write(&source, b"source").expect("write source");

        let first = next_pdf_crop_path(&source.to_string_lossy());
        assert_eq!(
            first.file_name().and_then(|name| name.to_str()),
            Some("acta_v2.pdf")
        );

        std::fs::write(&first, b"crop").expect("write first crop");
        let second = next_pdf_crop_path(&first.to_string_lossy());
        assert_eq!(
            second.file_name().and_then(|name| name.to_str()),
            Some("acta_v3.pdf")
        );
    }

    struct FakeRenderedPageSource {
        pages: Vec<Vec<u8>>,
        rendered_indexes: Vec<usize>,
        expected_prior_output: Option<PathBuf>,
    }

    impl RenderedPageSource for FakeRenderedPageSource {
        fn page_count(&mut self) -> Result<usize, String> {
            Ok(self.pages.len())
        }

        fn render_page(&mut self, index: usize) -> Result<Vec<u8>, String> {
            if let Some(path) = self.expected_prior_output.as_ref().filter(|_| index == 1) {
                assert_eq!(
                    std::fs::read(path).expect("page 1 written before page 2 renders"),
                    [1]
                );
            }
            self.rendered_indexes.push(index);
            Ok(self.pages[index].clone())
        }
    }

    enum TestWriter {
        File(std::fs::File),
        PrefixThenFail(PrefixThenFailWriter),
    }

    enum CreateFailureTestWriter {
        File(std::fs::File),
    }

    impl std::io::Write for CreateFailureTestWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            match self {
                Self::File(file) => std::io::Write::write(file, buffer),
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            match self {
                Self::File(file) => std::io::Write::flush(file),
            }
        }
    }

    impl std::io::Write for TestWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            match self {
                Self::File(file) => std::io::Write::write(file, buffer),
                Self::PrefixThenFail(writer) => writer.write(buffer),
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            match self {
                Self::File(file) => std::io::Write::flush(file),
                Self::PrefixThenFail(writer) => writer.flush(),
            }
        }
    }

    struct PrefixThenFailWriter {
        file: std::fs::File,
        prefix_len: usize,
        wrote_prefix: bool,
    }

    impl std::io::Write for PrefixThenFailWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            if self.wrote_prefix {
                return Err(std::io::Error::other("synthetic write failure"));
            }

            let prefix_len = self.prefix_len.min(buffer.len());
            std::io::Write::write_all(&mut self.file, &buffer[..prefix_len])?;
            self.wrote_prefix = true;
            Ok(prefix_len)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            std::io::Write::flush(&mut self.file)
        }
    }

    #[test]
    fn write_rendered_pages_writes_each_page_immediately_with_one_based_names_and_metadata() {
        let output = tempdir().expect("output dir");
        let mut source = FakeRenderedPageSource {
            pages: vec![vec![1], vec![2]],
            rendered_indexes: Vec::new(),
            expected_prior_output: Some(output.path().join("scan_page_1.png")),
        };

        let pages = write_rendered_pages_with(
            output.path(),
            "scan",
            |visitor| visit_rendered_pages(&mut source, visitor),
            |path| std::fs::File::create(path),
        )
        .expect("pages are written");

        assert_eq!(source.rendered_indexes, vec![0, 1]);
        assert_eq!(
            std::fs::read(output.path().join("scan_page_1.png")).expect("page 1"),
            [1]
        );
        assert_eq!(
            std::fs::read(output.path().join("scan_page_2.png")).expect("page 2"),
            [2]
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].page_number, 1);
        assert_eq!(pages[1].page_number, 2);
        assert_eq!(
            pages[0].png_path,
            normalize_windows_path_string(&output.path().join("scan_page_1.png"))
        );
        assert_eq!(
            pages[1].png_path,
            normalize_windows_path_string(&output.path().join("scan_page_2.png"))
        );
    }

    #[test]
    fn write_rendered_pages_keeps_source_errors_mapped_to_the_existing_command_error() {
        struct FailingSource;

        impl RenderedPageSource for FailingSource {
            fn page_count(&mut self) -> Result<usize, String> {
                Err("source failed".to_string())
            }

            fn render_page(&mut self, _: usize) -> Result<Vec<u8>, String> {
                unreachable!("count failure stops traversal")
            }
        }

        let output = tempdir().expect("output dir");
        let mut source = FailingSource;

        let result = write_rendered_pages_with(
            output.path(),
            "scan",
            |visitor| visit_rendered_pages(&mut source, visitor),
            |path| std::fs::File::create(path),
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("source failure is returned"),
        };

        assert_eq!(error, "Failed to render PDF pages: source failed");
    }

    #[test]
    fn write_rendered_pages_keeps_prior_output_and_stops_after_a_partial_write_failure() {
        let output = tempdir().expect("output dir");
        let mut source = FakeRenderedPageSource {
            pages: vec![vec![1], vec![2, 3, 4], vec![5]],
            rendered_indexes: Vec::new(),
            expected_prior_output: Some(output.path().join("scan_page_1.png")),
        };

        let result = write_rendered_pages_with(
            output.path(),
            "scan",
            |visitor| visit_rendered_pages(&mut source, visitor),
            |path| {
                if path.file_name().and_then(|name| name.to_str()) == Some("scan_page_2.png") {
                    Ok(TestWriter::PrefixThenFail(PrefixThenFailWriter {
                        file: std::fs::File::create(path)?,
                        prefix_len: 2,
                        wrote_prefix: false,
                    }))
                } else {
                    Ok(TestWriter::File(std::fs::File::create(path)?))
                }
            },
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("partial write failure is returned"),
        };

        assert_eq!(
            error,
            "Failed to write PNG data for page 2: synthetic write failure"
        );
        assert_eq!(source.rendered_indexes, vec![0, 1]);
        assert_eq!(
            std::fs::read(output.path().join("scan_page_1.png")).expect("page 1"),
            [1]
        );
        assert_eq!(
            std::fs::read(output.path().join("scan_page_2.png")).expect("page 2"),
            [2, 3]
        );
        assert!(!output.path().join("scan_page_3.png").exists());
    }

    #[test]
    fn rendered_page_serializes_to_the_established_ipc_shape() {
        let page = RenderedPage {
            page_number: 7,
            png_path: "C:/output/scan_page_7.png".to_string(),
        };

        let serialized = serde_json::to_value(page).expect("RenderedPage serializes");

        assert_eq!(
            serialized,
            serde_json::json!({
                "page_number": 7,
                "png_path": "C:/output/scan_page_7.png",
            })
        );
    }

    #[test]
    fn write_rendered_pages_stops_after_a_page_two_create_failure() {
        let output = tempdir().expect("output dir");
        let mut source = FakeRenderedPageSource {
            pages: vec![vec![1], vec![2], vec![3]],
            rendered_indexes: Vec::new(),
            expected_prior_output: Some(output.path().join("scan_page_1.png")),
        };

        let result = write_rendered_pages_with(
            output.path(),
            "scan",
            |visitor| visit_rendered_pages(&mut source, visitor),
            |path| {
                if path.file_name().and_then(|name| name.to_str()) == Some("scan_page_2.png") {
                    Err::<CreateFailureTestWriter, _>(std::io::Error::other(
                        "synthetic create failure",
                    ))
                } else {
                    Ok(CreateFailureTestWriter::File(std::fs::File::create(path)?))
                }
            },
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("create failure is returned"),
        };

        assert_eq!(
            error,
            "Failed to create PNG file for page 2: synthetic create failure"
        );
        assert_eq!(source.rendered_indexes, vec![0, 1]);
        assert_eq!(
            std::fs::read(output.path().join("scan_page_1.png")).expect("page 1"),
            [1]
        );
        assert!(!output.path().join("scan_page_2.png").exists());
        assert!(!output.path().join("scan_page_3.png").exists());
    }
}
