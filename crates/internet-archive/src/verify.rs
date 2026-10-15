//! Post-download PDF verification (Phase 3 task 18).
//!
//! A staged file is checked in this order: `%PDF` header, declared
//! `content_length` when present, always-on SHA-256, optional remote MD5 /
//! SHA-1 (case-insensitive, empty means absent), then page count through
//! `lopdf::Document::load_mem` against [`PageLimits`]. Any failure deletes
//! the staged file; success leaves it in place. Error displays never carry
//! filesystem paths: staging paths are private.

use std::path::Path;

use crate::error::IaError;

/// Default page cap for one remote PDF.
pub const DEFAULT_MAX_PAGES: u32 = 500;
/// Absolute page cap; larger requests clamp here and can never disable it.
pub const ABSOLUTE_MAX_PAGES: u32 = 2000;

/// Page cap for one verified PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageLimits {
    /// Maximum pages accepted for one file.
    pub max_pages: u32,
}

impl PageLimits {
    /// Clamp an optional request: `None` (and `0`, which would disable)
    /// use the default, values above the absolute max clamp down, values
    /// in between are honored. Never returns `0` nor anything above
    /// [`ABSOLUTE_MAX_PAGES`].
    #[must_use]
    pub fn clamped(requested: Option<u32>) -> Self {
        match requested {
            None | Some(0) => Self {
                max_pages: DEFAULT_MAX_PAGES,
            },
            Some(v) => Self {
                max_pages: v.clamp(1, ABSOLUTE_MAX_PAGES),
            },
        }
    }
}

/// Remote checksums / length as published by the Metadata API.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExpectedRemote {
    /// Declared MD5 (`md5`); `None` or empty means absent.
    pub md5: Option<String>,
    /// Declared SHA-1 (`sha1`); `None` or empty means absent.
    pub sha1: Option<String>,
    /// Declared byte length (`size`); checked when present.
    pub content_length: Option<u64>,
}

/// A staged PDF that passed every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPdf {
    /// File length in bytes.
    pub size: u64,
    /// Pages counted by `lopdf`.
    pub page_count: u32,
    /// Lowercase hex SHA-256 of the exact staged bytes.
    pub sha256_hex: String,
}

/// Verify a staged PDF against the remote expectations and page limits.
///
/// Order: read the file, always hash SHA-256, require the `%PDF` header,
/// require the declared `content_length` when present, compare remote MD5 /
/// SHA-1 when non-empty (case-insensitive), then count pages with
/// `lopdf::Document::load_mem` and enforce `limits`. Any failure deletes
/// the staged file and reports without its path; success leaves the file
/// in place and returns its size, page count and SHA-256.
///
/// A missing file maps to [`IaError::Io`], never a panic. `lopdf` parses
/// the whole file in memory; its input is already bounded by the download
/// byte cap, which is the accepted product risk (plan §7 point 7).
pub fn verify_staged_pdf(
    path: &Path,
    expected: &ExpectedRemote,
    limits: &PageLimits,
) -> Result<VerifiedPdf, IaError> {
    let bytes = std::fs::read(path).map_err(|e| IaError::Io(e.to_string()))?;
    // Always computed, even when the remote published no checksum: it is
    // the content identity and the fallback version fingerprint.
    let sha256_hex = {
        use sha2::Digest as _;
        format!("{:x}", sha2::Sha256::digest(&bytes))
    };
    if !bytes.starts_with(b"%PDF") {
        return Err(reject(path, "el archivo no es un PDF"));
    }
    if let Some(declared) = expected.content_length {
        if declared != bytes.len() as u64 {
            return Err(reject(path, "el tamaño no coincide con el declarado"));
        }
    }
    if let Some(want) = present(expected.md5.as_deref()) {
        let got = {
            use md5::Digest as _;
            format!("{:x}", md5::Md5::digest(&bytes))
        };
        if got != want.to_ascii_lowercase() {
            return Err(reject(path, "el md5 no coincide con el publicado"));
        }
    }
    if let Some(want) = present(expected.sha1.as_deref()) {
        if sha1_hex(&bytes) != want.to_ascii_lowercase() {
            return Err(reject(path, "el sha1 no coincide con el publicado"));
        }
    }
    let document =
        lopdf::Document::load_mem(&bytes).map_err(|_| reject(path, "el PDF no se puede abrir"))?;
    let page_count = u32::try_from(document.get_pages().len()).unwrap_or(u32::MAX);
    // A document lopdf opens but that has no pages is not importable.
    if page_count == 0 {
        return Err(reject(path, "el PDF no tiene páginas"));
    }
    if page_count > limits.max_pages {
        let _ = std::fs::remove_file(path);
        return Err(IaError::TooManyPages {
            pages: page_count,
            max_pages: limits.max_pages,
        });
    }
    Ok(VerifiedPdf {
        size: bytes.len() as u64,
        page_count,
        sha256_hex,
    })
}

/// A declared checksum counts only when non-empty after trimming; empty
/// strings from the Metadata API mean "absent", not "match nothing".
fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

/// Delete a rejected staged file and report without its path: staging
/// paths are private and never enter an error Display.
fn reject(path: &Path, reason: &str) -> IaError {
    let _ = std::fs::remove_file(path);
    IaError::VerifyFailed {
        reason: reason.to_string(),
    }
}

/// SHA-1 (FIPS 180-4) over `bytes`, lowercase hex. Hand-rolled so this
/// crate adds no dependency beyond `lopdf` / `sha2` / `md-5`: `sha2`
/// deliberately omits SHA-1 and no other hash crate is allowed here.
#[allow(clippy::needless_range_loop)] // the schedule reads `w[i-3]`, inherently indexed
fn sha1_hex(bytes: &[u8]) -> String {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut msg = bytes.to_vec();
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    format!(
        "{:08x}{:08x}{:08x}{:08x}{:08x}",
        h[0], h[1], h[2], h[3], h[4]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_pdf(pages: u32) -> Vec<u8> {
        use lopdf::{dictionary, Document, Object, Stream};
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
        let page_ids = (0..pages)
            .map(|index| {
                let content = document.add_object(Stream::new(
                    dictionary! {},
                    format!("BT /F1 12 Tf 20 100 Td (Page {index}) Tj ET").into_bytes(),
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
                "Count" => i64::from(pages),
                "Kids" => page_ids.iter().map(|id| (*id).into()).collect::<Vec<Object>>(),
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("serialize test PDF");
        bytes
    }

    fn stage(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("staged.pdf");
        std::fs::write(&path, bytes).unwrap();
        (tmp, path)
    }

    fn md5_of(bytes: &[u8]) -> String {
        use md5::Digest as _;
        format!("{:x}", md5::Md5::digest(bytes))
    }

    #[test]
    fn limits_clamp_table() {
        assert_eq!(PageLimits::clamped(None).max_pages, 500);
        assert_eq!(PageLimits::clamped(Some(0)).max_pages, 500);
        assert_eq!(PageLimits::clamped(Some(9_999)).max_pages, 2000);
    }

    #[test]
    fn valid_one_page_pdf_succeeds() {
        let bytes = tiny_pdf(1);
        let (_tmp, path) = stage(&bytes);
        let out = verify_staged_pdf(
            &path,
            &ExpectedRemote::default(),
            &PageLimits::clamped(None),
        )
        .unwrap();
        assert_eq!(out.page_count, 1);
        assert_eq!(out.size, bytes.len() as u64);
        assert_eq!(out.sha256_hex.len(), 64);
        assert!(out.sha256_hex.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(
            out.sha256_hex,
            out.sha256_hex.to_ascii_lowercase(),
            "hex must be lowercase"
        );
        assert!(path.exists(), "a verified file stays in place");
    }

    #[test]
    fn non_pdf_bytes_fail_and_delete() {
        let (_tmp, path) = stage(b"not a pdf");
        let err = verify_staged_pdf(
            &path,
            &ExpectedRemote::default(),
            &PageLimits::clamped(None),
        )
        .unwrap_err();
        assert!(matches!(err, IaError::VerifyFailed { .. }), "got {err:?}");
        assert!(
            format!("{err}").contains("verificación fallida"),
            "display must contain marker, got {err}"
        );
        assert!(!path.exists(), "a rejected file is deleted");
    }

    #[test]
    fn content_length_mismatch_fails_and_deletes() {
        let bytes = tiny_pdf(1);
        let (_tmp, path) = stage(&bytes);
        let wrong = ExpectedRemote {
            content_length: Some(bytes.len() as u64 + 1),
            ..ExpectedRemote::default()
        };
        let err = verify_staged_pdf(&path, &wrong, &PageLimits::clamped(None)).unwrap_err();
        assert!(matches!(err, IaError::VerifyFailed { .. }), "got {err:?}");
        assert!(format!("{err}").contains("verificación fallida"));
        assert!(!path.exists());
    }

    #[test]
    fn wrong_md5_fails_and_matching_md5_any_case_succeeds() {
        let bytes = tiny_pdf(1);
        let (_tmp, path) = stage(&bytes);
        let wrong = ExpectedRemote {
            md5: Some("0".repeat(32)),
            ..ExpectedRemote::default()
        };
        let err = verify_staged_pdf(&path, &wrong, &PageLimits::clamped(None)).unwrap_err();
        assert!(matches!(err, IaError::VerifyFailed { .. }), "got {err:?}");
        assert!(format!("{err}").contains("verificación fallida"));
        assert!(!path.exists());

        let hex = md5_of(&bytes);
        for candidate in [hex.to_ascii_lowercase(), hex.to_ascii_uppercase()] {
            let (_tmp, path) = stage(&bytes);
            let expected = ExpectedRemote {
                md5: Some(candidate),
                content_length: Some(bytes.len() as u64),
                ..ExpectedRemote::default()
            };
            let out = verify_staged_pdf(&path, &expected, &PageLimits::clamped(None)).unwrap();
            assert_eq!(out.page_count, 1);
            assert!(path.exists());
        }
    }

    #[test]
    fn too_many_pages_fails_and_deletes() {
        let bytes = tiny_pdf(3);
        let (_tmp, path) = stage(&bytes);
        let limits = PageLimits::clamped(Some(2));
        assert_eq!(limits.max_pages, 2);
        let err = verify_staged_pdf(&path, &ExpectedRemote::default(), &limits).unwrap_err();
        assert!(matches!(err, IaError::TooManyPages { .. }), "got {err:?}");
        assert!(
            format!("{err}").contains("demasiadas páginas"),
            "display must contain marker, got {err}"
        );
        assert!(!path.exists());
    }

    #[test]
    fn zero_pages_is_verify_failed_and_deleted() {
        let bytes = tiny_pdf(0);
        let (_tmp, path) = stage(&bytes);
        let err = verify_staged_pdf(
            &path,
            &ExpectedRemote::default(),
            &PageLimits::clamped(None),
        )
        .unwrap_err();
        assert!(matches!(err, IaError::VerifyFailed { .. }), "got {err:?}");
        assert!(!path.exists());
    }

    #[test]
    fn sha1_vectors_match_fips180() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    #[test]
    fn wrong_sha1_fails_and_matching_sha1_any_case_succeeds() {
        let bytes = tiny_pdf(1);
        let (_tmp, path) = stage(&bytes);
        let wrong = ExpectedRemote {
            sha1: Some("0".repeat(40)),
            ..ExpectedRemote::default()
        };
        let err = verify_staged_pdf(&path, &wrong, &PageLimits::clamped(None)).unwrap_err();
        assert!(matches!(err, IaError::VerifyFailed { .. }), "got {err:?}");
        assert!(format!("{err}").contains("verificación fallida"));
        assert!(!path.exists());

        let hex = sha1_hex(&bytes);
        for candidate in [hex.to_ascii_lowercase(), hex.to_ascii_uppercase()] {
            let (_tmp, path) = stage(&bytes);
            let expected = ExpectedRemote {
                sha1: Some(candidate),
                ..ExpectedRemote::default()
            };
            let out = verify_staged_pdf(&path, &expected, &PageLimits::clamped(None)).unwrap();
            assert_eq!(out.page_count, 1);
            assert!(path.exists());
        }
    }

    #[test]
    fn empty_checksums_count_as_absent() {
        let bytes = tiny_pdf(1);
        let (_tmp, path) = stage(&bytes);
        let expected = ExpectedRemote {
            md5: Some(String::new()),
            sha1: Some("   ".to_string()),
            content_length: None,
        };
        let out = verify_staged_pdf(&path, &expected, &PageLimits::clamped(None)).unwrap();
        assert_eq!(out.page_count, 1);
        assert!(path.exists());
    }

    #[test]
    fn pdf_header_with_unparseable_body_fails_and_deletes() {
        let (_tmp, path) = stage(b"%PDF-1.4 garbage that lopdf cannot open");
        let err = verify_staged_pdf(
            &path,
            &ExpectedRemote::default(),
            &PageLimits::clamped(None),
        )
        .unwrap_err();
        assert!(matches!(err, IaError::VerifyFailed { .. }), "got {err:?}");
        assert!(format!("{err}").contains("verificación fallida"));
        assert!(!path.exists());
    }

    #[test]
    fn missing_file_is_an_io_error_not_a_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("absent-distinctive-name.pdf");
        let err = verify_staged_pdf(
            &path,
            &ExpectedRemote::default(),
            &PageLimits::clamped(None),
        )
        .unwrap_err();
        assert!(
            matches!(err, IaError::Io(_)),
            "a missing staged file must be Io, got {err:?}"
        );
        assert!(
            !format!("{err}").contains("absent-distinctive-name"),
            "error Display must not leak the staging path, got {err}"
        );
    }

    #[test]
    fn rejection_displays_never_carry_the_staging_path() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = "staging-marker-xyz";
        let path = tmp.path().join(format!("{marker}.pdf"));
        std::fs::write(&path, b"not a pdf").unwrap();
        let err = verify_staged_pdf(
            &path,
            &ExpectedRemote::default(),
            &PageLimits::clamped(None),
        )
        .unwrap_err();
        assert!(matches!(err, IaError::VerifyFailed { .. }), "got {err:?}");
        assert!(
            !format!("{err}").contains(marker),
            "VerifyFailed Display must not leak the staging path, got {err}"
        );
        assert!(!path.exists());

        let bytes = tiny_pdf(3);
        let path = tmp.path().join(format!("{marker}-pages.pdf"));
        std::fs::write(&path, &bytes).unwrap();
        let err = verify_staged_pdf(
            &path,
            &ExpectedRemote::default(),
            &PageLimits::clamped(Some(2)),
        )
        .unwrap_err();
        assert!(matches!(err, IaError::TooManyPages { .. }), "got {err:?}");
        assert!(
            !format!("{err}").contains(marker),
            "TooManyPages Display must not leak the staging path, got {err}"
        );
        assert!(!path.exists());
    }
}
