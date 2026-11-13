//! OCR correction through the local Qwen3-VL-4B server (llama-server).
//!
//! Upstream OCRC is remote-only ("no local vision path"). With local Qwen
//! assets installed the app has a local vision model that follows the shared
//! correction prompt faithfully, so correction falls back to it when no
//! OpenRouter key is configured. Remote stays preferred whenever a key
//! exists: bigger context window, no truncation surprises.
//!
//! The prompt is the same `GenerationFlow::OcrCorrection` text the remote flow
//! sends (plus the page image when the asset has one); a repetition penalty
//! keeps the small model from looping, and a word-count guard refuses to
//! commit deletions instead of silently degrading the archive.

use std::path::Path;
use std::time::Duration;

use tauri::AppHandle;

use super::{AssetCorrectionRun, LlmJob};
use crate::ocr::qwen_local;

/// Server request timeout: a dense page can generate up to 8k tokens.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// Repetition penalty: small chat models loop on long generations otherwise.
const REPEAT_PENALTY: f32 = 1.1;
/// Word-count band for the corrected text relative to the source. Outside it
/// the model deleted or invented content instead of correcting: fail loudly
/// instead of committing.
const MIN_WORD_RATIO: f64 = 0.7;
const MAX_WORD_RATIO: f64 = 2.0;

/// Whether a local correction path exists right now: both GGUF files plus a
/// usable server binary (managed install or installer-bundled resources).
pub(crate) fn local_correction_available(db_path: &Path, resource_dir: Option<&Path>) -> bool {
    qwen_local::resolve_assets_with_resource_dir(db_path, resource_dir).is_ok()
}

/// [`local_correction_available`] for callers holding an `AppHandle`: the
/// installer-bundled server build is found through the resource directory.
pub(crate) fn local_correction_available_for(app_handle: &AppHandle, db_path: &Path) -> bool {
    use tauri::Manager;
    let resource_dir = app_handle.path().resource_dir().ok();
    local_correction_available(db_path, resource_dir.as_deref())
}

/// Run one `CorrectOcrAsset` job against the local Qwen3-VL server.
///
/// Builds the exact prompt the remote flow would send (same truncation budget
/// as one server slot) and attaches the page image when the asset has one.
/// Blocking: call it from a blocking context, like the OCRH subprocess path.
pub(crate) fn correct_ocr_via_local_qwen(
    app_handle: &AppHandle,
    db_path: &Path,
    asset_id: &str,
) -> Result<AssetCorrectionRun, String> {
    use tauri::Manager;

    let resource_dir = app_handle.path().resource_dir().ok();
    let assets = qwen_local::resolve_assets_with_resource_dir(db_path, resource_dir.as_deref())?;
    let job = LlmJob::CorrectOcrAsset {
        asset_id: asset_id.to_string(),
    };
    let conn = crate::db::open::open_archive_connection(db_path)?;
    let request = super::prepare_remote_job_request(
        &conn,
        &job,
        qwen_local::CORRECTION_SERVER_CONFIG.ctx_per_slot,
        db_path.parent().unwrap_or_else(|| Path::new(".")),
    )?;
    let expected_extraction = request.ocr_extraction_snapshot.clone().ok_or_else(|| {
        "OCR correction completion is missing its extraction snapshot".to_string()
    })?;

    let image = match request.visual_source_path {
        Some(path) => Some(read_correction_image(&path)?),
        None => None,
    };
    drop(conn);

    let url = qwen_local::ensure_server(&assets, db_path)?;
    let mut corrected = chat_completion(
        &url,
        &request.prompt,
        image.as_ref(),
        request.generation.params.max_tokens,
    )?;
    corrected = strip_correction_preamble(&corrected, &expected_extraction);
    check_word_retention(&corrected, &expected_extraction)?;

    Ok(AssetCorrectionRun {
        expected_extraction,
        corrected,
    })
}

/// Refuse outputs that deleted or invented content instead of correcting.
/// Counts words on both sides; the correction of a page keeps its length
/// within a wide band (column joins barely move the count).
fn check_word_retention(corrected: &str, source: &str) -> Result<(), String> {
    let source_words = source.split_whitespace().count().max(1) as f64;
    let corrected_words = corrected.split_whitespace().count() as f64;
    let ratio = corrected_words / source_words;
    if !(MIN_WORD_RATIO..=MAX_WORD_RATIO).contains(&ratio) {
        return Err(format!(
            "Local correction kept {ratio:.2}x of the source words (band {MIN_WORD_RATIO}-{MAX_WORD_RATIO}); refusing to commit a deletion"
        ));
    }
    Ok(())
}

/// Drop a model preamble line small chat models insist on. Only the first
/// non-blank line is ever considered, and only when it is a known preamble
/// phrase or a single short token absent from the source text — a genuine
/// headline always overlaps the extraction it corrects.
fn strip_correction_preamble(corrected: &str, source: &str) -> String {
    let mut lines = corrected.lines();
    let first = loop {
        match lines.next() {
            Some(line) if line.trim().is_empty() => continue,
            other => break other,
        }
    };
    let Some(head) = first else {
        return String::new();
    };
    let trimmed = head.trim();
    let lowered = trimmed.to_lowercase();
    let known = [
        "respondir",
        "respondido",
        "respondo",
        "respuesta",
        "he aquí",
        "aquí está",
        "aquí tienes",
        "texto corregido",
    ];
    let is_preamble = known
        .iter()
        .any(|phrase| lowered == *phrase || lowered.starts_with(&format!("{phrase}:")))
        || (!trimmed.contains(char::is_whitespace)
            && trimmed.chars().count() <= 15
            && !source.contains(trimmed));
    if !is_preamble {
        return corrected.trim_start().to_string();
    }
    lines
        .collect::<Vec<_>>()
        .join("\n")
        .trim_start()
        .to_string()
}

/// Image bytes plus MIME type for the chat request. PDFs contribute their
/// first rendered page, like the remote flow; anything else goes in raw.
fn read_correction_image(path: &Path) -> Result<(Vec<u8>, &'static str), String> {
    let bytes = std::fs::read(path).map_err(|e| {
        format!(
            "Failed to read OCR correction image {}: {e}",
            path.display()
        )
    })?;
    let is_pdf = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));
    if is_pdf {
        let png = crate::ocr::render_pdf_first_page_for_ocr_correction(&bytes)?;
        return Ok((png, "image/png"));
    }
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else {
        return Err(format!(
            "Unsupported image type for OCR correction: {}",
            path.display()
        ));
    };
    Ok((bytes, mime))
}

/// One OpenAI-compatible chat call against the local server.
fn chat_completion(
    server_url: &str,
    prompt: &str,
    image: Option<&(Vec<u8>, &'static str)>,
    max_tokens: i32,
) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let content = match image {
        Some((bytes, mime)) => serde_json::json!([
            { "type": "text", "text": prompt },
            { "type": "image_url", "image_url": {
                "url": format!("data:{mime};base64,{}", STANDARD.encode(bytes)) } },
        ]),
        None => serde_json::json!(prompt),
    };
    let body = serde_json::json!({
        "messages": [
            { "role": "system", "content": "Devolvé únicamente el texto corregido, sin prólogo, sin comentarios y sin bloques de código." },
            { "role": "user", "content": content }
        ],
        "temperature": 0.0,
        "repeat_penalty": REPEAT_PENALTY,
        "max_tokens": max_tokens,
    });

    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to create local correction client: {e}"))?;
    let response = client
        .post(format!("{server_url}/v1/chat/completions"))
        .json(&body)
        .send()
        .map_err(|e| format!("Local Qwen correction request failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail: String = response
            .text()
            .unwrap_or_default()
            .chars()
            .take(500)
            .collect();
        return Err(format!(
            "Local Qwen correction failed with HTTP {status}: {detail}"
        ));
    }
    let payload: serde_json::Value = response
        .json()
        .map_err(|e| format!("Local Qwen correction returned invalid JSON: {e}"))?;
    payload
        .pointer("/choices/0/message/content")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Local Qwen correction returned no text".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_without_assets() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("entropia.sqlite");
        assert!(!local_correction_available(&db, None));
    }

    #[test]
    fn png_magic_sniffs_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.png");
        std::fs::write(&path, [0x89, b'P', b'N', b'G', 0, 0]).unwrap();
        let (_, mime) = read_correction_image(&path).unwrap();
        assert_eq!(mime, "image/png");
    }

    #[test]
    fn jpeg_magic_sniffs_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        std::fs::write(&path, [0xFF, 0xD8, 0xFF, 0x00]).unwrap();
        let (_, mime) = read_correction_image(&path).unwrap();
        assert_eq!(mime, "image/jpeg");
    }

    #[test]
    fn unknown_bytes_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        std::fs::write(&path, b"not an image").unwrap();
        assert!(read_correction_image(&path).is_err());
    }

    #[test]
    fn known_preamble_line_is_dropped() {
        let out = strip_correction_preamble("Respondir\n\nBUENOS AIRES, 7 (UPI).", "mesa eje-");
        assert_eq!(out, "BUENOS AIRES, 7 (UPI).");
    }

    #[test]
    fn unknown_single_token_absent_from_source_is_dropped() {
        let out = strip_correction_preamble("Xyzzyx\n\nTexto real", "Texto real largo");
        assert_eq!(out, "Texto real");
    }

    #[test]
    fn genuine_headline_overlapping_source_is_kept() {
        let text = "## MALES Y REMEDIOS\n\nCuerpo de la nota";
        let out = strip_correction_preamble(text, "MALES Y REMEDIOS");
        assert_eq!(out, text);
    }

    #[test]
    fn real_first_line_is_kept_verbatim() {
        let text = "BUENOS AIRES, 7 (UPI). — Texto";
        assert_eq!(strip_correction_preamble(text, "BUENOS AIRES"), text);
    }

    #[test]
    fn retention_guard_rejects_deletions() {
        let source = "word ".repeat(100);
        assert!(check_word_retention(&source, &source).is_ok());
        assert!(check_word_retention("too short", &source).is_err());
    }

    #[test]
    fn retention_guard_rejects_rambles() {
        let source = "word ".repeat(100);
        let ramble = "word ".repeat(500);
        assert!(check_word_retention(&ramble, &source).is_err());
    }
}
