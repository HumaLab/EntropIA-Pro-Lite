//! Single entry point for reading an image with its EXIF orientation applied.
//!
//! The WebView draws a tagged photo upright and reports every coordinate in
//! that upright system. The `image` crate does not: applying the orientation
//! is opt-in. Everything that decodes an asset image goes through this module
//! so both sides agree on which way is up.
//!
//! The file on disk is never modified: the orientation is applied on read.

use crate::image_edit::JPEG_QUALITY;
use image::codecs::jpeg::JpegEncoder;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::io::{BufRead, Cursor, Seek};
use std::path::Path;

/// `assets.type` of a file the WebView draws as an image.
const IMAGE_ASSET_TYPE: &str = "image";
/// How a PDF starts, whatever the asset row says about it.
const PDF_HEADER: &[u8] = b"%PDF-";

/// An image file opened by path. The extension gives the format and the
/// content overrides it when it says otherwise.
fn open_reader(
    path: &Path,
) -> Result<ImageReader<std::io::BufReader<std::fs::File>>, std::io::Error> {
    ImageReader::open(path)?.with_guessed_format()
}

/// An in-memory image. The format comes from the content alone.
fn bytes_reader(bytes: &[u8]) -> Result<ImageReader<Cursor<&[u8]>>, std::io::Error> {
    ImageReader::new(Cursor::new(bytes)).with_guessed_format()
}

/// The orientation a decoder reports. A tag that cannot be read counts as
/// "no transform": whatever is wrong with the file, the pixel decode says so.
fn orientation_of(decoder: &mut impl ImageDecoder) -> Orientation {
    decoder.orientation().unwrap_or(Orientation::NoTransforms)
}

/// The shared core: decode, then apply the orientation the file declares.
/// Returns the orientation that was applied.
fn decode_upright<R: BufRead + Seek>(
    reader: ImageReader<R>,
) -> image::ImageResult<(DynamicImage, Orientation)> {
    let mut decoder = reader.into_decoder()?;
    let orientation = orientation_of(&mut decoder);
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    Ok((img, orientation))
}

/// Read only the orientation, without decoding the pixels.
fn probe<R: BufRead + Seek>(reader: ImageReader<R>) -> Orientation {
    reader
        .into_decoder()
        .map(|mut decoder| orientation_of(&mut decoder))
        .unwrap_or(Orientation::NoTransforms)
}

/// Decode the image at `path` with its EXIF orientation applied.
pub(crate) fn decode_oriented(path: impl AsRef<Path>) -> Result<DynamicImage, String> {
    let reader = open_reader(path.as_ref()).map_err(|e| format!("Failed to open image: {e}"))?;
    decode_upright(reader)
        .map(|(img, _)| img)
        .map_err(|e| format!("Failed to open image: {e}"))
}

/// Decode an in-memory image with its EXIF orientation applied. The format is
/// guessed from the content.
pub(crate) fn decode_oriented_bytes(bytes: &[u8]) -> Result<DynamicImage, String> {
    let reader = bytes_reader(bytes).map_err(|e| format!("Failed to decode image: {e}"))?;
    decode_upright(reader)
        .map(|(img, _)| img)
        .map_err(|e| format!("Failed to decode image: {e}"))
}

/// EXIF orientation of the image at `path`, `1..=8`. `1` when the file carries
/// no tag or cannot be read.
#[allow(dead_code)]
pub(crate) fn exif_orientation(path: impl AsRef<Path>) -> u8 {
    open_reader(path.as_ref())
        .map(probe)
        .unwrap_or(Orientation::NoTransforms)
        .to_exif()
}

/// EXIF orientation (`1..=8`) and pixel size of the image at `path` as a person
/// sees it: `(orientation, width, height)`, with the two sides swapped for the
/// orientations that lay the picture on its side (5 to 8). Only the header is
/// read, no pixel is decoded. `None` when the file is not an image the crate
/// can open.
pub(crate) fn upright_header(path: impl AsRef<Path>) -> Option<(u8, u32, u32)> {
    let mut decoder = open_reader(path.as_ref()).ok()?.into_decoder().ok()?;
    let orientation = orientation_of(&mut decoder).to_exif();
    let (stored_width, stored_height) = decoder.dimensions();
    Some(if orientation >= 5 {
        (orientation, stored_height, stored_width)
    } else {
        (orientation, stored_width, stored_height)
    })
}

/// EXIF orientation of an in-memory image, `1..=8`. `1` when the bytes carry no
/// tag or cannot be read.
pub(crate) fn exif_orientation_bytes(bytes: &[u8]) -> u8 {
    bytes_reader(bytes)
        .map(probe)
        .unwrap_or(Orientation::NoTransforms)
        .to_exif()
}

/// Bytes of the same image, upright and without an orientation tag.
///
/// An image with no tag, or with a tag that says it is already upright, comes
/// back as the very same bytes: nothing is decoded and nothing is re-encoded.
/// So do bytes that are not an image, for the consumer to reject as before.
///
/// A tagged image is decoded, turned upright and encoded again: as JPEG at
/// [`JPEG_QUALITY`] when the source is a JPEG, as PNG in every other case.
/// Neither encoder writes the tag back.
pub(crate) fn oriented_bytes(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if exif_orientation_bytes(&bytes) == 1 {
        return Ok(bytes);
    }

    let reader = bytes_reader(&bytes).map_err(|e| format!("Failed to decode image: {e}"))?;
    let source_is_jpeg = reader.format() == Some(ImageFormat::Jpeg);
    let (img, _) = decode_upright(reader).map_err(|e| format!("Failed to decode image: {e}"))?;

    let mut out = Cursor::new(Vec::new());
    if source_is_jpeg {
        img.write_with_encoder(JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY))
    } else {
        img.write_to(&mut out, ImageFormat::Png)
    }
    .map_err(|e| format!("Failed to encode upright image: {e}"))?;
    Ok(out.into_inner())
}

/// The bytes of an asset file as every OCR, layout and LLM consumer receives
/// them: an image asset comes out upright (see [`oriented_bytes`]), anything
/// else comes out as the very same bytes.
///
/// This is the single ingress: it runs once, where the file is read, and the
/// engines downstream never see an orientation tag. `asset_type` is the
/// `assets.type` column. A PDF stored under an image asset is recognised by
/// its header and left alone, like every other non-image.
///
/// `raw` is the file exactly as read from disk. Whoever needs to identify the
/// source file (its `sha256`) does so over `raw`, before calling this: the
/// bytes returned for a tagged photo are a re-encoding, not the file.
pub(crate) fn upright_asset_bytes(asset_type: &str, raw: Vec<u8>) -> Result<Vec<u8>, String> {
    if asset_type != IMAGE_ASSET_TYPE || raw.starts_with(PDF_HEADER) {
        return Ok(raw);
    }
    oriented_bytes(raw)
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Fixtures shared with the tests of the modules that decode through here.

    use image::codecs::jpeg::JpegEncoder;
    use image::{DynamicImage, Rgb, RgbImage};

    pub(crate) const RED: Rgb<u8> = Rgb([220, 30, 30]);
    pub(crate) const GREEN: Rgb<u8> = Rgb([30, 200, 30]);
    pub(crate) const BLUE: Rgb<u8> = Rgb([30, 30, 220]);
    pub(crate) const WHITE: Rgb<u8> = Rgb([240, 240, 240]);

    /// The picture as a person sees it: four solid quadrants, each a different
    /// colour, so no rotation or mirror maps it onto itself.
    ///
    /// ```text
    /// RED   | GREEN
    /// ------+------
    /// BLUE  | WHITE
    /// ```
    pub(crate) fn upright_quadrants(width: u32, height: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |x, y| {
            match (x < width / 2, y < height / 2) {
                (true, true) => RED,
                (false, true) => GREEN,
                (true, false) => BLUE,
                (false, false) => WHITE,
            }
        })
    }

    /// The pixels a camera stores for `upright` when it tags the file with
    /// `orientation`.
    ///
    /// Written from the EXIF definition of the tag (where the stored row 0 and
    /// column 0 sit in the displayed picture), not from the crate's transforms:
    ///
    /// | value | row 0  | column 0 |
    /// |-------|--------|----------|
    /// | 1     | top    | left     |
    /// | 2     | top    | right    |
    /// | 3     | bottom | right    |
    /// | 4     | bottom | left     |
    /// | 5     | left   | top      |
    /// | 6     | right  | top      |
    /// | 7     | right  | bottom   |
    /// | 8     | left   | bottom   |
    pub(crate) fn stored_pixels(upright: &RgbImage, orientation: u8) -> RgbImage {
        let (w, h) = upright.dimensions();
        let (stored_w, stored_h) = if orientation >= 5 { (h, w) } else { (w, h) };
        RgbImage::from_fn(stored_w, stored_h, |sx, sy| {
            let (ux, uy) = match orientation {
                1 => (sx, sy),
                2 => (w - 1 - sx, sy),
                3 => (w - 1 - sx, h - 1 - sy),
                4 => (sx, h - 1 - sy),
                5 => (sy, sx),
                6 => (w - 1 - sy, sx),
                7 => (w - 1 - sy, h - 1 - sx),
                8 => (sy, h - 1 - sx),
                other => panic!("not an EXIF orientation: {other}"),
            };
            *upright.get_pixel(ux, uy)
        })
    }

    /// A minimal big-endian TIFF block with one IFD entry: Orientation.
    pub(crate) fn orientation_tiff(orientation: u8) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"MM\x00\x2A"); // big-endian TIFF header
        tiff.extend_from_slice(&8u32.to_be_bytes()); // offset of the first IFD
        tiff.extend_from_slice(&1u16.to_be_bytes()); // one entry
        tiff.extend_from_slice(&0x0112u16.to_be_bytes()); // tag: Orientation
        tiff.extend_from_slice(&3u16.to_be_bytes()); // type: SHORT
        tiff.extend_from_slice(&1u32.to_be_bytes()); // count
        tiff.extend_from_slice(&u16::from(orientation).to_be_bytes()); // value
        tiff.extend_from_slice(&[0, 0]); // value padding
        tiff.extend_from_slice(&0u32.to_be_bytes()); // no next IFD
        tiff
    }

    /// Insert an APP1/Exif segment carrying `orientation` into encoded JPEG
    /// bytes, after SOI and the JFIF APP0 segment when there is one.
    pub(crate) fn with_exif_orientation(jpeg: &[u8], orientation: u8) -> Vec<u8> {
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "not a JPEG: no SOI marker");
        let mut insert_at = 2;
        if jpeg[2..4] == [0xFF, 0xE0] {
            insert_at += 2 + usize::from(u16::from_be_bytes([jpeg[4], jpeg[5]]));
        }

        let mut payload = b"Exif\x00\x00".to_vec();
        payload.extend_from_slice(&orientation_tiff(orientation));
        let segment_len = u16::try_from(payload.len() + 2).expect("APP1 fits in a segment");

        let mut out = jpeg[..insert_at].to_vec();
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&segment_len.to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&jpeg[insert_at..]);
        out
    }

    /// Encode `pixels` as a JPEG with no metadata.
    pub(crate) fn encode_jpeg(pixels: RgbImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        DynamicImage::ImageRgb8(pixels)
            .write_with_encoder(JpegEncoder::new_with_quality(&mut bytes, 95))
            .expect("encode jpeg");
        bytes
    }

    /// A JPEG as a camera writes it: `upright` is what the person sees, the
    /// file holds the sideways pixels plus the tag.
    pub(crate) fn camera_jpeg(upright: &RgbImage, orientation: u8) -> Vec<u8> {
        with_exif_orientation(
            &encode_jpeg(stored_pixels(upright, orientation)),
            orientation,
        )
    }

    /// True when `actual` is `expected` within JPEG noise.
    pub(crate) fn close_to(actual: Rgb<u8>, expected: Rgb<u8>) -> bool {
        actual
            .0
            .iter()
            .zip(expected.0)
            .all(|(a, e)| a.abs_diff(e) <= 24)
    }

    /// Assert that `img` shows the four quadrants of [`upright_quadrants`] the
    /// right way up, sampling the centre of each one.
    pub(crate) fn assert_upright_quadrants(
        img: &DynamicImage,
        width: u32,
        height: u32,
        case: &str,
    ) {
        let rgb = img.to_rgb8();
        assert_eq!(rgb.dimensions(), (width, height), "{case}: dimensions");
        let samples = [
            (width / 4, height / 4, RED, "top-left"),
            (width * 3 / 4, height / 4, GREEN, "top-right"),
            (width / 4, height * 3 / 4, BLUE, "bottom-left"),
            (width * 3 / 4, height * 3 / 4, WHITE, "bottom-right"),
        ];
        for (x, y, expected, corner) in samples {
            let actual = *rgb.get_pixel(x, y);
            assert!(
                close_to(actual, expected),
                "{case}: {corner} is {actual:?}, expected about {expected:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use image::codecs::png::PngEncoder;
    use image::{GenericImageView, ImageEncoder};

    const W: u32 = 64;
    const H: u32 = 32;

    /// A PNG whose eXIf chunk carries `orientation`.
    fn camera_png(upright: &image::RgbImage, orientation: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = PngEncoder::new(&mut bytes);
        encoder
            .set_exif_metadata(orientation_tiff(orientation))
            .expect("png carries exif");
        DynamicImage::ImageRgb8(stored_pixels(upright, orientation))
            .write_with_encoder(encoder)
            .expect("encode png");
        bytes
    }

    #[test]
    fn decode_oriented_bytes_returns_every_orientation_upright() {
        let upright = upright_quadrants(W, H);
        for orientation in 1..=8u8 {
            let jpeg = camera_jpeg(&upright, orientation);

            let decoded = decode_oriented_bytes(&jpeg).expect("decode");

            assert_upright_quadrants(&decoded, W, H, &format!("orientation {orientation}"));
        }
    }

    #[test]
    fn decode_oriented_reads_a_file_the_same_way_and_leaves_it_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("photo.jpg");
        let jpeg = camera_jpeg(&upright_quadrants(W, H), 6);
        std::fs::write(&path, &jpeg).expect("write fixture");

        let decoded = decode_oriented(&path).expect("decode");

        assert_upright_quadrants(&decoded, W, H, "orientation 6 from a path");
        assert_eq!(std::fs::read(&path).expect("read back"), jpeg);
    }

    #[test]
    fn exif_orientation_reports_the_tag_and_defaults_to_one() {
        let upright = upright_quadrants(W, H);
        for orientation in 1..=8u8 {
            let jpeg = camera_jpeg(&upright, orientation);
            assert_eq!(exif_orientation_bytes(&jpeg), orientation);
        }
        // No tag at all, and bytes that are not an image.
        assert_eq!(exif_orientation_bytes(&encode_jpeg(upright.clone())), 1);
        assert_eq!(exif_orientation_bytes(b"not an image"), 1);

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("photo.jpg");
        std::fs::write(&path, camera_jpeg(&upright, 8)).expect("write fixture");
        assert_eq!(exif_orientation(&path), 8);
        assert_eq!(exif_orientation(dir.path().join("missing.jpg")), 1);
    }

    #[test]
    fn the_header_gives_the_upright_size_for_every_orientation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let upright = upright_quadrants(W, H);
        for orientation in 1..=8u8 {
            let path = dir.path().join(format!("photo-{orientation}.jpg"));
            std::fs::write(&path, camera_jpeg(&upright, orientation)).expect("write fixture");

            assert_eq!(
                upright_header(&path),
                Some((orientation, W, H)),
                "orientation {orientation}"
            );
        }

        let pdf = dir.path().join("doc.pdf");
        std::fs::write(&pdf, b"%PDF-1.7\n%%EOF\n").expect("write pdf");
        assert_eq!(upright_header(&pdf), None);
        assert_eq!(upright_header(dir.path().join("missing.jpg")), None);
    }

    #[test]
    fn an_untagged_image_passes_through_unchanged() {
        let upright = upright_quadrants(W, H);
        let jpeg = encode_jpeg(upright.clone());

        assert_eq!(oriented_bytes(jpeg.clone()).expect("oriented"), jpeg);
        let decoded = decode_oriented_bytes(&jpeg).expect("decode");
        let plain = image::load_from_memory(&jpeg).expect("plain decode");
        assert_eq!(decoded.as_bytes(), plain.as_bytes());
        assert_eq!(decoded.dimensions(), plain.dimensions());

        // A tag that says "already upright" is not a reason to re-encode.
        let tagged_one = with_exif_orientation(&jpeg, 1);
        assert_eq!(
            oriented_bytes(tagged_one.clone()).expect("oriented"),
            tagged_one
        );
    }

    #[test]
    fn oriented_bytes_turns_a_tagged_jpeg_into_an_upright_untagged_jpeg() {
        let upright = upright_quadrants(W, H);
        for orientation in 2..=8u8 {
            let jpeg = camera_jpeg(&upright, orientation);

            let out = oriented_bytes(jpeg).expect("oriented");

            let case = format!("orientation {orientation}");
            assert_eq!(
                image::guess_format(&out).expect("known format"),
                ImageFormat::Jpeg,
                "{case}"
            );
            assert_eq!(exif_orientation_bytes(&out), 1, "{case}");
            // Plain decode, on purpose: the bytes themselves must be upright.
            let plain = image::load_from_memory(&out).expect("plain decode");
            assert_upright_quadrants(&plain, W, H, &case);
        }
    }

    #[test]
    fn an_image_asset_reaches_its_consumers_upright_and_untagged() {
        let jpeg = camera_jpeg(&upright_quadrants(W, H), 6);

        let out = upright_asset_bytes("image", jpeg).expect("ingress");

        assert_eq!(exif_orientation_bytes(&out), 1);
        let plain = image::load_from_memory(&out).expect("plain decode");
        assert_upright_quadrants(&plain, W, H, "image asset, orientation 6");
    }

    #[test]
    fn the_ingress_hands_on_the_same_bytes_when_there_is_nothing_to_turn() {
        let untagged = encode_jpeg(upright_quadrants(W, H));
        assert_eq!(
            upright_asset_bytes("image", untagged.clone()).expect("ingress"),
            untagged
        );

        // A PDF, whether the asset says so or it sits under an image asset.
        let pdf = b"%PDF-1.7\n1 0 obj\n<< >>\nendobj\n%%EOF\n".to_vec();
        assert_eq!(
            upright_asset_bytes("pdf", pdf.clone()).expect("ingress"),
            pdf
        );
        assert_eq!(
            upright_asset_bytes("image", pdf.clone()).expect("ingress"),
            pdf
        );

        // Only image assets are turned: the type decides, not the content.
        let tagged = camera_jpeg(&upright_quadrants(W, H), 6);
        for asset_type in ["pdf", "audio"] {
            assert_eq!(
                upright_asset_bytes(asset_type, tagged.clone()).expect("ingress"),
                tagged,
                "{asset_type}"
            );
        }
    }

    #[test]
    fn oriented_bytes_turns_a_tagged_non_jpeg_into_an_upright_png() {
        let png = camera_png(&upright_quadrants(W, H), 6);
        assert_eq!(exif_orientation_bytes(&png), 6, "fixture carries the tag");

        let out = oriented_bytes(png).expect("oriented");

        assert_eq!(
            image::guess_format(&out).expect("known format"),
            ImageFormat::Png
        );
        assert_eq!(exif_orientation_bytes(&out), 1);
        let plain = image::load_from_memory(&out).expect("plain decode");
        assert_upright_quadrants(&plain, W, H, "png orientation 6");
    }
}
