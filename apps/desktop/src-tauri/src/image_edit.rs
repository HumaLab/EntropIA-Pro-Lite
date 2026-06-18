//! Image editing commands: crop, rotate, erase region.
//!
//! All operations write a NEW versioned file (never in-place) to force
//! browser cache invalidation and support undo. The previous file is
//! kept on disk so undo can restore it by pointing the asset path back.

use crate::path_utils::{ensure_within_dir, validate_existing_file};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GenericImageView, ImageBuffer, Pixel, Rgb, Rgba, RgbaImage};
use imageproc::geometric_transformations::{rotate_about_center, Interpolation};
use std::path::{Path, PathBuf};

/// Result of an image edit operation. Returned to the frontend so it can
/// update asset paths and dimensions, and maintain an undo history.
#[derive(serde::Serialize)]
pub struct ImageEditResult {
    /// New path of the edited image (always a new versioned file)
    pub path: String,
    /// Width in pixels after the edit
    pub width: u32,
    /// Height in pixels after the edit
    pub height: u32,
    /// True when the file format changed (e.g. JPEG → PNG for transparency)
    pub format_changed: bool,
    /// Path of the file before the edit (kept on disk for undo)
    pub previous_path: String,
}

/// Generate a new versioned path for an image file.
///
/// Finds the next available version number by checking the filesystem,
/// so undo paths that are still on disk won't be overwritten.
///
/// Examples:
///   `photo.jpg` → `photo_v2.jpg` (if _v2 doesn't exist)
///   `photo_v2.jpg` → `photo_v3.jpg` (if _v3 doesn't exist)
///   `photo_v2.jpg` → `photo_v4.jpg` (if _v3 exists but _v4 doesn't)
fn next_version_path(path: &str, force_extension: Option<&str>) -> String {
    let p = Path::new(path);
    let ext =
        force_extension.unwrap_or_else(|| p.extension().and_then(|e| e.to_str()).unwrap_or(""));
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let dir = p.parent().unwrap_or(Path::new("."));

    // Parse existing version suffix: "photo" → (photo, 2), "photo_v2" → (photo, 3)
    let (base_stem, first_version) = if let Some(idx) = stem.rfind("_v") {
        let suffix = &stem[idx + 2..];
        if let Ok(v) = suffix.parse::<u32>() {
            (&stem[..idx], v + 1)
        } else {
            (stem, 2u32)
        }
    } else {
        (stem, 2u32)
    };

    // Find the next available version number
    let mut version = first_version;
    loop {
        let new_stem = format!("{base_stem}_v{version}");
        let new_filename = if !ext.is_empty() {
            format!("{new_stem}.{ext}")
        } else {
            new_stem
        };
        let new_path = dir.join(new_filename);
        if !new_path.exists() {
            return new_path.to_string_lossy().to_string();
        }
        version += 1;
    }
}

/// Resolve the app data directory used to scope-check asset paths.
fn resolve_app_data_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))
}

/// Validate an image-edit source path at the IPC boundary: it must be an
/// existing file inside the app data directory (imported assets are always
/// copied under `{app_data_dir}/assets/…` by the frontend import flow).
fn validate_source_image_path(path: &str, app_data_dir: &Path) -> Result<(), String> {
    let canonical = validate_existing_file(path)?;
    ensure_within_dir(&canonical, app_data_dir)?;
    Ok(())
}

/// JPEG quality used when re-encoding edited images. `DynamicImage::save`
/// uses the image crate's default of 75, which compounds visible generational
/// loss when edits are chained (each edit re-encodes the previous output).
const JPEG_QUALITY: u8 = 92;

/// Save an image inferring the format from the path, but encoding JPEG
/// targets at [`JPEG_QUALITY`] instead of the encoder default of 75.
fn save_image(img: &DynamicImage, path: &str) -> image::ImageResult<()> {
    let is_jpeg = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg"));
    if is_jpeg {
        let writer = std::io::BufWriter::new(std::fs::File::create(path)?);
        img.write_with_encoder(JpegEncoder::new_with_quality(writer, JPEG_QUALITY))
    } else {
        img.save(path)
    }
}

/// Crop an image to the specified pixel region.
///
/// Saves the result as a NEW versioned file (never in-place).
/// The original file is kept on disk for undo.
#[tauri::command]
pub async fn crop_image(
    path: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    app_handle: tauri::AppHandle,
) -> Result<ImageEditResult, String> {
    let app_data_dir = resolve_app_data_dir(&app_handle)?;
    validate_source_image_path(&path, &app_data_dir)?;
    tokio::task::spawn_blocking(move || crop_image_file(path, x, y, width, height))
        .await
        .map_err(|e| format!("Image crop task panicked: {e}"))?
}

fn crop_image_file(
    path: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<ImageEditResult, String> {
    let img = image::open(&path).map_err(|e| format!("Failed to open image: {e}"))?;
    let (orig_w, orig_h) = img.dimensions();

    // Clamp crop region to image bounds
    let cx = x.min(orig_w);
    let cy = y.min(orig_h);
    let cw = width.min(orig_w.saturating_sub(cx));
    let ch = height.min(orig_h.saturating_sub(cy));

    if cw == 0 || ch == 0 {
        return Err("Crop region is outside image bounds or has zero dimensions".to_string());
    }

    // crop_imm returns an owned cropped image preserving the source colour type
    let result = img.crop_imm(cx, cy, cw, ch);

    let new_path = next_version_path(&path, None);
    save_image(&result, &new_path).map_err(|e| format!("Failed to save cropped image: {e}"))?;

    Ok(ImageEditResult {
        path: new_path,
        width: cw,
        height: ch,
        format_changed: false,
        previous_path: path,
    })
}

/// Rotate an image 90° in the specified direction.
///
/// Saves the result as a NEW versioned file (never in-place).
/// The original file is kept on disk for undo.
///
/// - `"left"` = 90° counter-clockwise (270° CW)
/// - `"right"` = 90° clockwise
#[tauri::command]
pub async fn rotate_image(
    path: String,
    direction: String,
    app_handle: tauri::AppHandle,
) -> Result<ImageEditResult, String> {
    let app_data_dir = resolve_app_data_dir(&app_handle)?;
    validate_source_image_path(&path, &app_data_dir)?;
    tokio::task::spawn_blocking(move || rotate_image_file(path, direction))
        .await
        .map_err(|e| format!("Image rotation task panicked: {e}"))?
}

fn rotate_image_file(path: String, direction: String) -> Result<ImageEditResult, String> {
    let img = image::open(&path).map_err(|e| format!("Failed to open image: {e}"))?;

    let rotated = match direction.as_str() {
        "left" => img.rotate270(), // 90° counter-clockwise
        "right" => img.rotate90(), // 90° clockwise
        _ => {
            return Err(format!(
                "Invalid direction: '{direction}'. Use 'left' or 'right'."
            ))
        }
    };

    let (w, h) = rotated.dimensions();
    let new_path = next_version_path(&path, None);
    save_image(&rotated, &new_path).map_err(|e| format!("Failed to save rotated image: {e}"))?;

    Ok(ImageEditResult {
        path: new_path,
        width: w,
        height: h,
        format_changed: false,
        previous_path: path,
    })
}

/// Rotate an image by an arbitrary number of degrees.
///
/// Saves the result as a NEW versioned file with an expanded canvas so the
/// corners are not clipped by the rotation. The source format is preserved:
/// JPEG (and other non-alpha formats) fill the exposed corners with opaque white
/// and stay JPEG, keeping the file small and within downstream OCR upload limits;
/// alpha-capable sources (PNG/WebP/GIF) keep transparent corners and stay PNG.
#[tauri::command]
pub async fn rotate_image_degrees(
    path: String,
    degrees: f32,
    app_handle: tauri::AppHandle,
) -> Result<ImageEditResult, String> {
    let app_data_dir = resolve_app_data_dir(&app_handle)?;
    validate_source_image_path(&path, &app_data_dir)?;
    tokio::task::spawn_blocking(move || rotate_image_degrees_file(path, degrees))
        .await
        .map_err(|e| format!("Fine image rotation task panicked: {e}"))?
}

fn rotate_image_degrees_file(path: String, degrees: f32) -> Result<ImageEditResult, String> {
    if !degrees.is_finite() {
        return Err("Rotation degrees must be finite".to_string());
    }
    if degrees.abs() < f32::EPSILON {
        return Err("Rotation degrees must be non-zero".to_string());
    }
    if degrees.abs() > 360.0 {
        return Err("Rotation degrees must be between -360 and 360".to_string());
    }

    let img = image::open(&path).map_err(|e| format!("Failed to open image: {e}"))?;

    // Preserve the source format. Formats without an alpha channel (JPEG, BMP, TIFF)
    // fill the corners exposed by the rotation with opaque white and keep their
    // extension, so a rotated scan stays a small JPEG instead of ballooning into a
    // lossless RGBA PNG (which also blows past downstream OCR upload limits).
    // Alpha-capable sources (PNG/WebP/GIF) keep transparent corners and stay PNG.
    let alpha_capable = Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ["png", "webp", "gif"].iter().any(|f| ext.eq_ignore_ascii_case(f)));
    let background = if alpha_capable {
        Rgba([255, 255, 255, 0]) // transparent
    } else {
        Rgba([255, 255, 255, 255]) // opaque white
    };

    let source = img.to_rgba8();
    let (orig_w, orig_h) = source.dimensions();
    let radians = degrees.to_radians();
    let sin = radians.sin().abs();
    let cos = radians.cos().abs();
    let expanded_w = ((orig_w as f32 * cos) + (orig_h as f32 * sin)).ceil() as u32;
    let expanded_h = ((orig_w as f32 * sin) + (orig_h as f32 * cos)).ceil() as u32;
    let mut canvas = RgbaImage::from_pixel(expanded_w.max(1), expanded_h.max(1), background);
    let offset_x = ((canvas.width() - orig_w) / 2) as i64;
    let offset_y = ((canvas.height() - orig_h) / 2) as i64;
    image::imageops::overlay(&mut canvas, &source, offset_x, offset_y);

    let rotated = rotate_about_center(&canvas, radians, Interpolation::Bilinear, background);
    let (out_w, out_h) = (canvas.width(), canvas.height());

    let new_path = if alpha_capable {
        let new_path = next_version_path(&path, Some("png"));
        DynamicImage::ImageRgba8(rotated)
            .save_with_format(&new_path, image::ImageFormat::Png)
            .map_err(|e| format!("Failed to save fine-rotated image: {e}"))?;
        new_path
    } else {
        // Corners are opaque white: drop the (now-uniform) alpha channel and
        // re-encode in the source format (JPEG at high quality) to keep it small.
        let new_path = next_version_path(&path, None);
        let rgb = DynamicImage::ImageRgba8(rotated).to_rgb8();
        save_image(&DynamicImage::ImageRgb8(rgb), &new_path)
            .map_err(|e| format!("Failed to save fine-rotated image: {e}"))?;
        new_path
    };

    Ok(ImageEditResult {
        path: new_path,
        width: out_w,
        height: out_h,
        format_changed: false,
        previous_path: path,
    })
}

/// Fill a rectangular region of an image buffer with a solid colour.
///
/// Writes contiguous row slices instead of calling bounds-checked
/// `put_pixel` once per pixel. The region must be within bounds.
fn fill_rect<P: Pixel>(
    buf: &mut ImageBuffer<P, Vec<P::Subpixel>>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: P,
) {
    let channels = P::CHANNEL_COUNT as usize;
    let stride = buf.width() as usize * channels;
    let color_channels = color.channels().to_vec();
    let raw: &mut [P::Subpixel] = buf;
    for row in y..y + height {
        let start = row as usize * stride + x as usize * channels;
        let row_slice = &mut raw[start..start + width as usize * channels];
        for px in row_slice.chunks_exact_mut(channels) {
            px.copy_from_slice(&color_channels);
        }
    }
}

/// Erase (fill) a rectangular region of an image with a solid or transparent color.
///
/// Saves the result as a NEW versioned file (never in-place).
/// When `fill` is `"transparent"` and the source format doesn't support alpha
/// (e.g. JPEG), the output is converted to PNG.
/// The original file is kept on disk for undo.
#[tauri::command]
pub async fn erase_region(
    path: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    fill: String,
    app_handle: tauri::AppHandle,
) -> Result<ImageEditResult, String> {
    let app_data_dir = resolve_app_data_dir(&app_handle)?;
    validate_source_image_path(&path, &app_data_dir)?;
    tokio::task::spawn_blocking(move || erase_region_file(path, x, y, width, height, fill))
        .await
        .map_err(|e| format!("Image erase task panicked: {e}"))?
}

fn erase_region_file(
    path: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    fill: String,
) -> Result<ImageEditResult, String> {
    let img = image::open(&path).map_err(|e| format!("Failed to open image: {e}"))?;
    let (orig_w, orig_h) = img.dimensions();

    // Clamp region to image bounds
    let ex = x.min(orig_w);
    let ey = y.min(orig_h);
    let ew = width.min(orig_w.saturating_sub(ex));
    let eh = height.min(orig_h.saturating_sub(ey));

    if ew == 0 || eh == 0 {
        return Err("Erase region is outside image bounds or has zero dimensions".to_string());
    }

    // Determine if format supports alpha channel
    let ext = Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let supports_alpha = !matches!(ext.as_str(), "jpg" | "jpeg");
    let needs_conversion = fill == "transparent" && !supports_alpha;

    // Determine fill colour
    let fill_color: Rgba<u8> = match fill.as_str() {
        "transparent" => Rgba([0, 0, 0, 0]),
        "white" => Rgba([255, 255, 255, 255]),
        "black" => Rgba([0, 0, 0, 255]),
        _ => {
            return Err(format!(
                "Invalid fill: '{fill}'. Use 'transparent', 'white', or 'black'."
            ))
        }
    };

    // Opaque fills on 8-bit RGB/RGBA sources are applied in place, keeping the
    // source colour type. Anything else (transparent fill, other colour types)
    // goes through an RGBA copy so the fill and alpha channel are well-defined.
    let result = match img {
        DynamicImage::ImageRgb8(mut buf) if fill_color[3] == 255 => {
            let rgb_fill = Rgb([fill_color[0], fill_color[1], fill_color[2]]);
            fill_rect(&mut buf, ex, ey, ew, eh, rgb_fill);
            DynamicImage::ImageRgb8(buf)
        }
        DynamicImage::ImageRgba8(mut buf) => {
            fill_rect(&mut buf, ex, ey, ew, eh, fill_color);
            DynamicImage::ImageRgba8(buf)
        }
        other => {
            let mut rgba_img = other.to_rgba8();
            fill_rect(&mut rgba_img, ex, ey, ew, eh, fill_color);
            DynamicImage::ImageRgba8(rgba_img)
        }
    };

    let (w, h) = result.dimensions();

    // Generate versioned path with the appropriate extension
    let forced_ext = if needs_conversion { Some("png") } else { None };
    let new_path = next_version_path(&path, forced_ext);

    if needs_conversion {
        result
            .save_with_format(&new_path, image::ImageFormat::Png)
            .map_err(|e| format!("Failed to save image as PNG: {e}"))?;
    } else {
        save_image(&result, &new_path).map_err(|e| format!("Failed to save erased image: {e}"))?;
    }

    Ok(ImageEditResult {
        path: new_path,
        width: w,
        height: h,
        format_changed: needs_conversion,
        previous_path: path,
    })
}

/// Delete an asset file AND its versioned edit siblings.
///
/// Image edits always write a NEW versioned file (`photo.jpg` → `photo_v2.jpg`
/// → `photo_v3.png` …) and keep the previous versions on disk for undo, so
/// deleting only the asset's current file leaks the older versions forever.
///
/// Conservative family match — only files in the same directory whose name
/// matches exactly `^{base}(_v\d+)?\.{image-ext}$`, where `base` is the given
/// file's stem with any `_vN` suffix stripped and `{image-ext}` is ANY known
/// raster image extension ([`ASSET_IMAGE_EXTENSIONS`]). Matching every image
/// extension — not just the current file's — is required because
/// format-converting edits change the extension across versions (a fine
/// rotation turns `photo.jpg` into `photo_v2.png`, and the DB then points at
/// the `.png`). `photo_final.png` is NOT part of the `photo` family.
///
/// Returns the number of files deleted. The given file may already be gone
/// (its siblings are still cleaned up); a missing directory deletes nothing.
#[tauri::command]
pub async fn delete_asset_files(
    asset_path: String,
    app_handle: tauri::AppHandle,
) -> Result<u32, String> {
    let app_data_dir = resolve_app_data_dir(&app_handle)?;
    tokio::task::spawn_blocking(move || delete_asset_file_family(&asset_path, &app_data_dir))
        .await
        .map_err(|e| format!("Asset deletion task panicked: {e}"))?
}

/// Strip a trailing `_v{digits}` version suffix from a file stem.
/// Mirrors the suffix produced by [`next_version_path`].
fn strip_version_suffix(stem: &str) -> &str {
    if let Some(idx) = stem.rfind("_v") {
        let suffix = &stem[idx + 2..];
        if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
            return &stem[..idx];
        }
    }
    stem
}

/// Raster image extensions the app can ingest (frontend import flow, see
/// `SUPPORTED_IMAGES` in `apps/desktop/src/lib/file-import.ts`) or produce
/// via the `image` crate. Used to match an asset's whole version family,
/// whatever format each version was saved in.
const ASSET_IMAGE_EXTENSIONS: &[&str] =
    &["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff", "gif"];

/// True when `file_name` matches exactly `^{base}(_v\d+)?\.{image-ext}$`,
/// where `{image-ext}` is any extension in [`ASSET_IMAGE_EXTENSIONS`].
/// Extensions compare case-insensitively; stems compare exactly so families
/// with shared prefixes (`photo` vs `photo_final`) never overlap. The match
/// deliberately spans ALL known image extensions: format-converting edits
/// (e.g. fine rotation forces PNG output) leave older versions on disk with
/// a different extension than the asset's current file.
fn file_belongs_to_asset_family(file_name: &str, base: &str) -> bool {
    let candidate = Path::new(file_name);
    let Some(stem) = candidate.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let Some(ext) = candidate.extension().and_then(|e| e.to_str()) else {
        return false;
    };

    if !ASSET_IMAGE_EXTENSIONS
        .iter()
        .any(|known| ext.eq_ignore_ascii_case(known))
    {
        return false;
    }

    if stem == base {
        return true;
    }

    let Some(version) = stem
        .strip_prefix(base)
        .and_then(|rest| rest.strip_prefix("_v"))
    else {
        return false;
    };
    !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit())
}

fn delete_asset_file_family(asset_path: &str, app_data_dir: &Path) -> Result<u32, String> {
    if asset_path.trim().is_empty() {
        return Err("Asset path must not be empty".to_string());
    }

    let canonical = ensure_within_dir(asset_path, app_data_dir)?;
    if canonical.exists() && !canonical.is_file() {
        return Err(format!("Asset path is not a file: {asset_path}"));
    }

    let Some(parent) = canonical.parent() else {
        return Err(format!("Asset path has no parent directory: {asset_path}"));
    };
    let stem = canonical
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("Asset path has no file name: {asset_path}"))?;
    let base = strip_version_suffix(stem);

    if !parent.is_dir() {
        return Ok(0);
    }

    let mut deleted = 0u32;
    for entry in
        std::fs::read_dir(parent).map_err(|e| format!("Failed to read asset directory: {e}"))?
    {
        let entry = entry.map_err(|e| format!("Failed to read asset directory entry: {e}"))?;
        let entry_path = entry.path();
        if !entry_path.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_belongs_to_asset_family(file_name, base) {
            std::fs::remove_file(&entry_path).map_err(|e| {
                format!(
                    "Failed to delete asset file '{}': {e}",
                    entry_path.display()
                )
            })?;
            deleted += 1;
        }
    }

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, ImageFormat};

    #[test]
    fn rotate_image_degrees_keeps_jpeg_source_as_jpeg() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_path = dir.path().join("sample.jpg");
        let img = ImageBuffer::from_pixel(10, 4, Rgba([255, 0, 0, 255]));
        DynamicImage::ImageRgba8(img)
            .save_with_format(&source_path, ImageFormat::Jpeg)
            .expect("save source");

        let result = rotate_image_degrees_file(source_path.to_string_lossy().to_string(), 45.0)
            .expect("fine rotate image");

        assert_eq!(result.previous_path, source_path.to_string_lossy());
        // A JPEG source must stay JPEG (white corners), not balloon into a lossless PNG.
        assert!(result.path.ends_with("sample_v2.jpg"));
        assert!(Path::new(&result.path).exists());
        assert!(!result.format_changed);
        assert!(result.width >= 10);
        assert!(result.height > 4);
        // The rotated output decodes as an opaque RGB JPEG (no alpha channel).
        let out = image::open(&result.path).expect("open rotated output");
        assert!(out.color().channel_count() <= 3 || !out.color().has_alpha());
    }

    #[test]
    fn rotate_image_degrees_keeps_png_source_as_png() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_path = dir.path().join("sample.png");
        let img = ImageBuffer::from_pixel(10, 4, Rgba([0, 0, 255, 255]));
        DynamicImage::ImageRgba8(img)
            .save_with_format(&source_path, ImageFormat::Png)
            .expect("save source");

        let result = rotate_image_degrees_file(source_path.to_string_lossy().to_string(), 30.0)
            .expect("fine rotate image");

        // Alpha-capable sources keep transparent corners and stay PNG.
        assert!(result.path.ends_with("sample_v2.png"));
        assert!(Path::new(&result.path).exists());
        assert!(!result.format_changed);
    }

    #[test]
    fn rotate_image_degrees_rejects_invalid_degrees() {
        assert!(rotate_image_degrees_file("missing.png".to_string(), 0.0).is_err());
        assert!(rotate_image_degrees_file("missing.png".to_string(), f32::NAN).is_err());
        assert!(rotate_image_degrees_file("missing.png".to_string(), 361.0).is_err());
    }

    #[test]
    fn crop_image_file_writes_cropped_versioned_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_path = dir.path().join("sample.png");
        let img = ImageBuffer::from_pixel(10, 4, Rgba([255, 0, 0, 255]));
        DynamicImage::ImageRgba8(img)
            .save_with_format(&source_path, ImageFormat::Png)
            .expect("save source");

        let result = crop_image_file(source_path.to_string_lossy().to_string(), 2, 1, 4, 2)
            .expect("crop image");

        assert!(result.path.ends_with("sample_v2.png"));
        assert_eq!((result.width, result.height), (4, 2));
        assert!(!result.format_changed);
        let cropped = image::open(&result.path).expect("open cropped");
        assert_eq!(cropped.dimensions(), (4, 2));
        assert_eq!(cropped.get_pixel(0, 0), Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn crop_image_file_reencodes_jpeg_sources_as_jpeg() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_path = dir.path().join("sample.jpg");
        let img = ImageBuffer::from_pixel(16, 16, Rgba([0, 128, 255, 255]));
        DynamicImage::ImageRgba8(img)
            .save_with_format(&source_path, ImageFormat::Jpeg)
            .expect("save source");

        let result = crop_image_file(source_path.to_string_lossy().to_string(), 0, 0, 8, 8)
            .expect("crop image");

        assert!(result.path.ends_with("sample_v2.jpg"));
        assert!(!result.format_changed);
        let cropped = image::open(&result.path).expect("open cropped");
        assert_eq!(cropped.dimensions(), (8, 8));
    }

    #[test]
    fn erase_region_file_fills_white_without_rgba_promotion() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_path = dir.path().join("sample.png");
        let img = ImageBuffer::from_pixel(8, 8, Rgb([10, 20, 30]));
        DynamicImage::ImageRgb8(img)
            .save_with_format(&source_path, ImageFormat::Png)
            .expect("save source");

        let result = erase_region_file(
            source_path.to_string_lossy().to_string(),
            2,
            2,
            4,
            4,
            "white".to_string(),
        )
        .expect("erase region");

        assert!(!result.format_changed);
        let erased = image::open(&result.path).expect("open erased");
        // Opaque fills keep the source colour type (no RGBA promotion)
        assert!(matches!(erased, DynamicImage::ImageRgb8(_)));
        assert_eq!(erased.get_pixel(3, 3), Rgba([255, 255, 255, 255]));
        assert_eq!(erased.get_pixel(0, 0), Rgba([10, 20, 30, 255]));
    }

    #[test]
    fn erase_region_file_transparent_fill_converts_jpeg_to_png() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_path = dir.path().join("sample.jpg");
        let img = ImageBuffer::from_pixel(8, 8, Rgba([200, 50, 50, 255]));
        DynamicImage::ImageRgba8(img)
            .save_with_format(&source_path, ImageFormat::Jpeg)
            .expect("save source");

        let result = erase_region_file(
            source_path.to_string_lossy().to_string(),
            0,
            0,
            4,
            4,
            "transparent".to_string(),
        )
        .expect("erase region");

        assert!(result.format_changed);
        assert!(result.path.ends_with("sample_v2.png"));
        let erased = image::open(&result.path).expect("open erased");
        assert_eq!(erased.get_pixel(1, 1)[3], 0);
    }

    #[test]
    fn validate_source_image_path_accepts_files_inside_app_data_dir() {
        let app_data = tempfile::tempdir().expect("tempdir");
        let item_dir = app_data.path().join("assets").join("col-1").join("item-1");
        std::fs::create_dir_all(&item_dir).expect("create item dir");
        let file_path = item_dir.join("photo.png");
        std::fs::write(&file_path, b"data").expect("write file");

        assert!(validate_source_image_path(&file_path.to_string_lossy(), app_data.path()).is_ok());
    }

    #[test]
    fn validate_source_image_path_rejects_missing_outside_and_directories() {
        let app_data = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("tempdir outside");
        let outside_file = outside.path().join("photo.png");
        std::fs::write(&outside_file, b"data").expect("write outside file");
        let inside_dir = app_data.path().join("assets");
        std::fs::create_dir_all(&inside_dir).expect("create inside dir");

        assert!(validate_source_image_path("", app_data.path()).is_err());
        assert!(validate_source_image_path(
            &app_data.path().join("missing.png").to_string_lossy(),
            app_data.path()
        )
        .is_err());
        assert!(
            validate_source_image_path(&outside_file.to_string_lossy(), app_data.path()).is_err()
        );
        assert!(
            validate_source_image_path(&inside_dir.to_string_lossy(), app_data.path()).is_err()
        );
    }

    #[test]
    fn strip_version_suffix_strips_only_numeric_versions() {
        assert_eq!(strip_version_suffix("photo"), "photo");
        assert_eq!(strip_version_suffix("photo_v2"), "photo");
        assert_eq!(strip_version_suffix("photo_v12"), "photo");
        assert_eq!(strip_version_suffix("photo_v2_final"), "photo_v2_final");
        assert_eq!(strip_version_suffix("photo_vX"), "photo_vX");
        assert_eq!(strip_version_suffix("photo_v"), "photo_v");
    }

    #[test]
    fn file_belongs_to_asset_family_matches_exact_family_only() {
        // Family members: base and _vN versions, in ANY known image format
        // (format-converting edits change the extension across versions).
        assert!(file_belongs_to_asset_family("photo.jpg", "photo"));
        assert!(file_belongs_to_asset_family("photo_v2.jpg", "photo"));
        assert!(file_belongs_to_asset_family("photo_v3.png", "photo"));
        assert!(file_belongs_to_asset_family("photo_v10.PNG", "photo"));
        assert!(file_belongs_to_asset_family("photo.webp", "photo"));
        assert!(file_belongs_to_asset_family("photo.jpeg", "photo"));
        assert!(file_belongs_to_asset_family("photo_v2.TIFF", "photo"));

        // Near-miss prefixes must NOT match.
        assert!(!file_belongs_to_asset_family("photo_final.png", "photo"));
        assert!(!file_belongs_to_asset_family("photography.jpg", "photo"));
        assert!(!file_belongs_to_asset_family("photo_v2x.jpg", "photo"));
        assert!(!file_belongs_to_asset_family("photo_vX.jpg", "photo"));

        // Non-image extensions must NOT match.
        assert!(!file_belongs_to_asset_family("photo.pdf", "photo"));
        assert!(!file_belongs_to_asset_family("photo.json", "photo"));
        assert!(!file_belongs_to_asset_family("photo", "photo"));
    }

    #[test]
    fn delete_asset_file_family_removes_versioned_siblings_only() {
        let app_data = tempfile::tempdir().expect("tempdir");
        let item_dir = app_data.path().join("assets").join("col-1").join("item-1");
        std::fs::create_dir_all(&item_dir).expect("create item dir");

        let family = [
            "photo.jpg",
            "photo_v2.jpg",
            "photo_v3.jpg",
            "photo_v4.png",
            "photo.webp",
        ];
        let survivors = [
            "photo_final.png",
            "photography.jpg",
            "photo_v2x.jpg",
            "photo.pdf",
            "other.png",
        ];
        for name in family.iter().chain(survivors.iter()) {
            std::fs::write(item_dir.join(name), b"data").expect("write file");
        }

        let deleted = delete_asset_file_family(
            &item_dir.join("photo_v3.jpg").to_string_lossy(),
            app_data.path(),
        )
        .expect("delete family");

        assert_eq!(deleted, family.len() as u32);
        for name in family {
            assert!(!item_dir.join(name).exists(), "{name} should be deleted");
        }
        for name in survivors {
            assert!(item_dir.join(name).exists(), "{name} should survive");
        }
    }

    #[test]
    fn delete_asset_file_family_removes_original_format_after_conversion() {
        // Regression: a fine rotation converts photo.jpg → photo_v2.png and
        // the DB path moves to the .png. Deleting via the CURRENT .png path
        // must still remove the original-format .jpg (the largest file).
        let app_data = tempfile::tempdir().expect("tempdir");
        let item_dir = app_data.path().join("assets").join("col-1").join("item-1");
        std::fs::create_dir_all(&item_dir).expect("create item dir");
        std::fs::write(item_dir.join("photo.jpg"), b"data").expect("write original");
        std::fs::write(item_dir.join("photo_v2.png"), b"data").expect("write converted v2");
        std::fs::write(item_dir.join("photo_final.png"), b"data").expect("write near miss");
        std::fs::write(item_dir.join("photography.jpg"), b"data").expect("write near miss");

        let deleted = delete_asset_file_family(
            &item_dir.join("photo_v2.png").to_string_lossy(),
            app_data.path(),
        )
        .expect("delete family");

        assert_eq!(deleted, 2);
        assert!(
            !item_dir.join("photo.jpg").exists(),
            "original-format file must not leak after a format-converting edit"
        );
        assert!(!item_dir.join("photo_v2.png").exists());
        assert!(item_dir.join("photo_final.png").exists());
        assert!(item_dir.join("photography.jpg").exists());
    }

    #[test]
    fn delete_asset_file_family_cleans_siblings_when_given_file_is_gone() {
        let app_data = tempfile::tempdir().expect("tempdir");
        let item_dir = app_data.path().join("assets").join("col-1").join("item-1");
        std::fs::create_dir_all(&item_dir).expect("create item dir");
        std::fs::write(item_dir.join("scan.png"), b"data").expect("write base");
        std::fs::write(item_dir.join("scan_v2.png"), b"data").expect("write v2");

        // The DB-referenced version was already removed; siblings remain.
        let deleted = delete_asset_file_family(
            &item_dir.join("scan_v3.png").to_string_lossy(),
            app_data.path(),
        )
        .expect("delete family");

        assert_eq!(deleted, 2);
        assert!(!item_dir.join("scan.png").exists());
        assert!(!item_dir.join("scan_v2.png").exists());
    }

    #[test]
    fn delete_asset_file_family_rejects_paths_outside_app_data_dir() {
        let app_data = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("tempdir outside");
        let outside_file = outside.path().join("photo.jpg");
        std::fs::write(&outside_file, b"data").expect("write outside file");

        let result = delete_asset_file_family(&outside_file.to_string_lossy(), app_data.path());

        assert!(result.is_err());
        assert!(outside_file.exists());
    }

    #[test]
    fn delete_asset_file_family_rejects_empty_and_handles_missing_dir() {
        let app_data = tempfile::tempdir().expect("tempdir");

        assert!(delete_asset_file_family("", app_data.path()).is_err());

        let missing = app_data
            .path()
            .join("assets")
            .join("ghost")
            .join("photo.jpg");
        let deleted = delete_asset_file_family(&missing.to_string_lossy(), app_data.path())
            .expect("missing dir should be a no-op");
        assert_eq!(deleted, 0);
    }
}
