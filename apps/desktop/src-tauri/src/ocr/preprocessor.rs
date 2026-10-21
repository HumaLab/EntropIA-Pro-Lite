/// Image preprocessing pipeline for OCR input.
///
/// Converts a colour image to grayscale and applies adaptive thresholding
/// to produce a binarised image. Currently unused — PaddleOCR handles its own
/// preprocessing internally.
use image::{DynamicImage, GrayImage};
use imageproc::contrast::adaptive_threshold;

/// Block radius used for the adaptive-threshold window.
/// A 15-pixel radius (~31x31 neighbourhood) works well for typical scanned documents.
const THRESHOLD_BLOCK_RADIUS: u32 = 15;

/// Preprocess an image for OCR: grayscale conversion + adaptive threshold.
///
/// Returns a `GrayImage` with binary pixel values (0 or 255).
pub fn preprocess_image(img: DynamicImage) -> GrayImage {
    let gray = img.into_luma8();
    adaptive_threshold(&gray, THRESHOLD_BLOCK_RADIUS, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};

    #[test]
    fn preprocessed_image_preserves_dimensions() {
        // Create a 100x100 test image filled with mid-gray pixels
        let rgb = RgbImage::from_fn(100, 100, |_x, _y| image::Rgb([128, 128, 128]));
        let dynamic = DynamicImage::ImageRgb8(rgb);

        let result = preprocess_image(dynamic);

        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 100);
    }
}
