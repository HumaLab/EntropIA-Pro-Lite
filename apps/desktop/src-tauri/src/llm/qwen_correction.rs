//! OCR correction through the local Qwen3-VL-4B server (llama-server).
//!
//! Upstream OCRC is remote-only ("no local vision path"). With local Qwen
//! assets installed the app has a local vision model that follows the shared
//! correction prompt faithfully, so correction falls back to it when no
//! OpenRouter key is configured. Remote stays preferred whenever a key
//! exists: bigger context window, no truncation surprises.
//!
/// The prompt is the same `GenerationFlow::OcrCorrection` text the remote flow
/// sends (plus the page image when the asset has one); a repetition penalty
/// keeps the small model from looping, a word-count guard refuses to commit
/// deletions, a repetition guard refuses loops, and pages too long for one
/// server slot are corrected per paragraph chunk and rejoined in order —
/// never truncated, never silently cut.
use std::path::Path;
use std::time::Duration;

use tauri::AppHandle;

use super::{AssetCorrectionRun, LlmJob};
use crate::ocr::qwen_local;

/// Server request timeout: a dense page can generate up to 8k tokens.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// Repetition penalty kept near-neutral on purpose: correction MUST reuse
/// the source wording, and penalizing repetition pushes the model to
/// paraphrase (word-count drift) or dodge termination (budget fills).
/// Loops are stopped deterministically by the repetition-loop detector and
/// the output-budget guard instead (they caught the 20x La Protesta loop
/// that 1.1 could not prevent).
const REPEAT_PENALTY: f32 = 1.05;
/// Word-count band for the corrected text relative to the source. Outside it
/// the model deleted or invented content instead of correcting: fail loudly
/// instead of committing.
const MIN_WORD_RATIO: f64 = 0.7;
const MAX_WORD_RATIO: f64 = 2.0;
/// Single-pass budget (prompt + output headroom) in chars: the 16K-token
/// server slot minus image tokens and margin. Pages beyond this are
/// corrected per paragraph chunk and rejoined, never truncated.
const SINGLE_PASS_BUDGET_CHARS: u64 = 45_000;
/// Output tokens budgeted per prompt char when deciding single vs chunked
/// pass. Conservative on purpose: overestimating only chunks earlier.
const CHARS_PER_OUTPUT_TOKEN: u64 = 3;
/// Max chars per correction chunk: with the full 8k output budget a chunk
/// plus its correction always fits the 16K slot (13K chars ≈ 5K tokens +
/// 8K out + template < 16K). Splits happen only at blank lines.
const CORRECTION_CHUNK_CHARS: usize = 13_000;

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

    // The prepared prompt may already be truncated to the slot budget: only
    // its length tells whether the whole page fits. Chunk by paragraph when
    // it does not, and rejoin in order — correction (unlike summarization)
    // needs no fusion pass, just concatenation.
    let max_tokens = request.generation.params.max_tokens;
    let prompt_chars = request.prompt.chars().count() as u64;
    let fits_single = prompt_chars
        .saturating_add(max_tokens.max(0) as u64 * CHARS_PER_OUTPUT_TOKEN)
        <= SINGLE_PASS_BUDGET_CHARS;
    let image = match request.visual_source_path {
        Some(path) => Some(read_correction_image(&path)?),
        None => None,
    };
    let corrected = if fits_single {
        drop(conn);
        let url = qwen_local::ensure_server(&assets, db_path)?;
        correct_one_pass(
            &url,
            &request.prompt,
            image.as_ref(),
            max_tokens,
            "pass",
            &expected_extraction,
        )?
    } else {
        // Render per-chunk prompts while the settings connection is alive.
        let chunks = split_text_for_correction(&expected_extraction, CORRECTION_CHUNK_CHARS);
        if chunks.len() <= 1 {
            return Err(format!(
                "page needs more than the {SINGLE_PASS_BUDGET_CHARS}-char single-pass budget and cannot be split further"
            ));
        }
        log::info!(
            target: "llm-local",
            "local Qwen correction: {} paragraph chunks ({} chars)",
            chunks.len(),
            expected_extraction.chars().count(),
        );
        // Each chunk prompt names its place in the page so the model stops
        // at the fragment's end instead of continuing the article.
        let prompts: Vec<String> = chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| {
                let scoped = format!(
                    "(Fragmento {}/{} de la página. Corregí ÚNICAMENTE este fragmento y detenete exactamente donde termina: no continúes la redacción ni agregues cierre. Devolvé ÚNICAMENTE el fragmento corregido, nada más.)\n\n{}",
                    index + 1,
                    chunks.len(),
                    chunk
                );
                super::generation::render_prompt_from_settings(
                    &conn,
                    super::generation::GenerationFlow::OcrCorrection,
                    &scoped,
                )
            })
            .collect();
        // Full output budget per chunk: chunks are small enough that prompt
        // plus the largest plausible correction fits the 16K slot; loop and
        // length guards below turn degeneration into a loud failure.
        drop(conn);
        let url = qwen_local::ensure_server(&assets, db_path)?;
        // No image on chunk calls: with the whole page in context the model
        // retranscribes the page per chunk and blows every budget. The text
        // already carries the content; single passes keep the image.
        let mut partials = Vec::with_capacity(chunks.len());
        let mut failed: Vec<usize> = Vec::new();
        for (index, prompt) in prompts.iter().enumerate() {
            let part_log = format!("[chunk {}/{}]", index + 1, chunks.len());
            log::info!(
                target: "llm-local",
                "local Qwen correction {part_log}: {} input chars",
                chunks[index].chars().count(),
            );
            match correct_one_pass(&url, prompt, None, max_tokens, &part_log, &chunks[index]) {
                Ok(fixed) => {
                    log::info!(
                        target: "llm-local",
                        "local Qwen correction {part_log}: {} output chars",
                        fixed.chars().count()
                    );
                    if let Err(error) = check_word_retention(&fixed, &chunks[index]) {
                        log::warn!(
                            target: "llm-local",
                            "local Qwen correction {part_log} kept as original: {error}"
                        );
                        failed.push(index + 1);
                        partials.push(chunks[index].clone());
                    } else {
                        partials.push(fixed);
                    }
                }
                Err(error) => {
                    // A failed chunk never blocks the page: keep the original
                    // text for it (same as no correction) and say so loudly.
                    // Committing a partial correction would delete content;
                    // committing nothing would waste the chunks that worked.
                    log::warn!(
                        target: "llm-local",
                        "local Qwen correction {part_log} kept as original: {error}"
                    );
                    failed.push(index + 1);
                    partials.push(chunks[index].clone());
                }
            }
        }
        if failed.len() == chunks.len() {
            return Err(format!(
                "local Qwen correction failed on all {} chunks; nothing committed",
                chunks.len()
            ));
        }
        if !failed.is_empty() {
            log::warn!(
                target: "llm-local",
                "local Qwen correction committed with {} of {} chunks uncorrected: {:?}",
                failed.len(),
                chunks.len(),
                failed
            );
        }
        partials.join("\n\n")
    };
    check_word_retention(&corrected, &expected_extraction)?;
    check_no_repetition_loops(&corrected)?;

    Ok(AssetCorrectionRun {
        expected_extraction,
        corrected,
    })
}

/// One server call: correct `prompt`, strip the preamble, and refuse loops
/// right away so the failing chunk is identified in the error.
fn correct_one_pass(
    server_url: &str,
    prompt: &str,
    image: Option<&(Vec<u8>, &'static str)>,
    max_tokens: i32,
    log_prefix: &str,
    source: &str,
) -> Result<String, String> {
    let mut out = chat_completion(server_url, prompt, image, max_tokens)
        .map_err(|error| format!("{log_prefix} local Qwen correction failed: {error}"))?;
    out = strip_correction_preamble(&out, source);
    check_no_repetition_loops(&out).map_err(|error| format!("{log_prefix} {error}"))?;
    Ok(out)
}

/// Split text into runs of whole paragraphs of at most `max_chars` each.
/// Splits only at blank lines (never mid-sentence); an unsplittable block
/// becomes its own chunk and the caller errors loudly instead of cutting it.
/// A stub tail (under a quarter of `max_chars`: dot-leader tables, page
/// numbers) folds into the previous chunk instead of going to the model
/// alone, where it comes back near-empty and burns a server call plus a
/// retention refusal. At most one fold, so the merged chunk stays within
/// ~125% of `max_chars` and still fits the 16K server slot.
const TAIL_MERGE_RATIO: f64 = 0.25;
fn split_text_for_correction(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for paragraph in text.split("\n\n") {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        if !current.is_empty()
            && current.chars().count() + paragraph.chars().count() + 2 > max_chars
        {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(paragraph);
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    if chunks.len() > 1 {
        let tail_chars = chunks.last().map(|c| c.chars().count()).unwrap_or(0);
        if tail_chars < (max_chars as f64 * TAIL_MERGE_RATIO) as usize {
            let tail = chunks.pop().unwrap();
            let prev = chunks.last_mut().unwrap();
            prev.push_str("\n\n");
            prev.push_str(&tail);
            log::info!(
                target: "llm-local",
                "local Qwen correction: stub tail of {tail_chars} chars folded into previous chunk, {} chunks total",
                chunks.len()
            );
        }
    }
    chunks
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

/// Longest repeated block, in characters, that trips the loop detector: real
/// newspaper text repeats short headers, never a full paragraph five times.
const LOOP_MIN_BLOCK_CHARS: usize = 60;
/// How many verbatim repeats of one block prove a degeneration loop.
const LOOP_MAX_REPEATS: usize = 5;

/// Refuse outputs stuck in a repetition loop (the same sentence or paragraph
/// emitted over and over). Word-count retention cannot catch this: a loop
/// keeps the length inside the band while destroying content. Splits on blank
/// lines so one repeated paragraph counts once per emission, not per line.
fn check_no_repetition_loops(corrected: &str) -> Result<(), String> {
    use std::collections::HashMap;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for paragraph in corrected.split("\n\n") {
        let normalized: String = paragraph.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.chars().count() >= LOOP_MIN_BLOCK_CHARS {
            *counts.entry(normalized).or_insert(0) += 1;
        }
    }
    if let Some((block, times)) = counts.iter().max_by_key(|(_, count)| **count) {
        if *times >= LOOP_MAX_REPEATS {
            let preview: String = block.chars().take(120).collect();
            return Err(format!(
                "Local correction loops: one {block_len}-char block repeats {times}x (\"{preview}…\"); refusing to commit",
                block_len = block.chars().count(),
            ));
        }
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
        // 0.0 greedy loops forever on long repetitive OCR: a small temperature
        // lets the sampler escape repetition without changing corrections.
        "temperature": 0.2,
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
    // A budget-cut correction would silently drop the tail of the page: fail
    // loudly instead of committing a partial text.
    if payload
        .pointer("/choices/0/finish_reason")
        .and_then(|value| value.as_str())
        == Some("length")
    {
        let partial = payload
            .pointer("/choices/0/message/content")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let preview: String = partial.chars().take(120).collect();
        let tail: String = partial
            .chars()
            .rev()
            .take(120)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let out_chars = partial.chars().count();
        // Distinguish a degeneration loop (same block over and over) from a
        // genuinely long answer: the loop is the model's failure, length alone
        // may just need smaller chunks.
        let looped = check_no_repetition_loops(&strip_correction_preamble(partial, "")).is_err();
        log::warn!(
            target: "llm-local",
            "local Qwen correction hit the {max_tokens}-token output budget ({out_chars} chars, loop={looped}); head: {preview:?} tail: {tail:?}"
        );
        if looped {
            return Err(
                "Local Qwen correction looped instead of correcting; refusing to commit"
                    .to_string(),
            );
        }
        return Err(
            "Local Qwen correction hit the output token budget; refusing a cut correction"
                .to_string(),
        );
    }
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

    #[test]
    fn correction_chunks_cover_the_whole_page() {
        let a = "Párrafo uno con contenido suficiente.\n\n";
        let b = "Párrafo dos también largo para contar.\n\n";
        let text = a.repeat(500) + &b.repeat(500);
        let chunks = split_text_for_correction(&text, 4000);
        assert!(chunks.len() > 2);
        let joined: String = chunks.join("\n\n");
        let norm = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(norm(&joined), norm(&text));
        assert!(chunks
            .iter()
            .all(|c| c.chars().count() <= 4000 + (4000.0 * TAIL_MERGE_RATIO) as usize));
    }

    #[test]
    fn repetition_loops_are_refused() {
        // The observed failure: one sentence emitted ~20x while the tail of
        // the page vanished, inside the word-count band.
        let looped =
            "La producción industrial no tiene una salida adecuada con la criada de sus elementos.\n\n"
                .repeat(20);
        let err = check_no_repetition_loops(&looped).unwrap_err();
        assert!(err.contains("20x"), "{err}");
    }

    #[test]
    fn healthy_corrections_and_short_headers_pass() {
        let ok = "Titular primero.\n\nSegundo párrafo con contenido distinto y suficiente longitud para contar.\n\nTercer párrafo también distinto y largo.";
        assert!(check_no_repetition_loops(ok).is_ok());
        // Short recurring headers (mastheads, page numbers) never trip it.
        let headers = "LA CAPITAL\n\n".repeat(10)
            + "Cuerpo largo con contenido real y distinto del encabezado repetido.";
        assert!(check_no_repetition_loops(&headers).is_ok());
    }

    #[test]
    fn stub_tail_folds_into_previous_chunk() {
        // Full chunks plus a stub tail (page number, dot table): no stub
        // survives alone, and nothing is lost.
        let body = "Párrafo de cuerpo con contenido real para llenar el bloque.\n\n";
        let text = body.repeat(200) + "12";
        let chunks = split_text_for_correction(&text, 4000);
        assert!(
            chunks.len() >= 2,
            "need chunks to fold, got {}",
            chunks.len()
        );
        let stub_max = (4000.0 * TAIL_MERGE_RATIO) as usize;
        assert!(
            chunks.iter().all(|c| c.chars().count() >= stub_max),
            "stub tail survived: {:?}",
            chunks.iter().map(|c| c.chars().count()).collect::<Vec<_>>()
        );
        let norm = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(norm(&chunks.join("\n\n")), norm(&text));
    }

    #[test]
    fn healthy_tail_stays_its_own_chunk() {
        // Two balanced halves: no fold, both within budget.
        let half = "Mitad equilibrada de la página con varios párrafos reales.\n\n";
        let text = half.repeat(100);
        let chunks = split_text_for_correction(&text, 4000);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| c.chars().count() <= 4000));
    }
}
