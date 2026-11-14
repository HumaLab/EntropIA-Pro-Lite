/// Local transcription engine backed by NVIDIA Parakeet TDT 0.6B v3 (ONNX).
///
/// Same contract as [`super::engine::TranscriptionResult`] so persistence in
/// `transcriptions` and the frontend need no changes. Runs in-process on the
/// transcription worker thread (like the embeddings ORT sessions), behind
/// `local-ml`: the Python subprocess existed to isolate whisper.cpp/ggml
/// crashes, not ORT — ORT already lives in this process.
///
/// Long audio is processed in overlapping windows and rejoined on TDT word
/// timestamps (midpoint ownership), so no word is cut at a window edge.
#[cfg(feature = "local-ml")]
use parakeet_rs::{ParakeetTDT, TimestampMode, Transcriber as _};

#[cfg(feature = "local-ml")]
use super::audio::decode_audio_file;
#[cfg(feature = "local-ml")]
use super::engine::{Segment, TranscriptionResult};

/// Pinned Hugging Face revision of the ONNX export. Every file below is
/// verified by content hash after download; the revision only selects what
/// the server serves over HTTPS.
#[cfg(feature = "local-ml")]
const PARAKEET_HF_REV: &str = "664cc742197053fa11afd12f5100749fb8000181";
#[cfg(feature = "local-ml")]
const PARAKEET_HF_REPO: &str = "altunenes/parakeet-rs";
#[cfg(feature = "local-ml")]
pub(crate) const PARAKEET_MODEL_SUBDIR: &str = "parakeet-tdt-0.6b-v3";
/// Model name recorded in `transcriptions.model` and asset history.
#[cfg(feature = "local-ml")]
pub const PARAKEET_MODEL_NAME: &str = "parakeet/tdt-0.6b-v3";

/// (file name, expected bytes, expected sha256 hex). The `.data` blob is the
/// 2.5 GB fp32 encoder; there is no official int8 export for v3.
#[cfg(feature = "local-ml")]
const PARAKEET_FILES: &[(&str, u64, &str)] = &[
    (
        "encoder-model.onnx",
        3_437_572,
        "8f72233edd8175da74950bbcec59f4fe8405455f6e038cdb2e530c54ab0d46ed",
    ),
    (
        "encoder-model.onnx.data",
        2_476_376_064,
        "40c1f3a0455a376fe906eb6caab976231bf0420df1e4e70929b11fba1410ccbc",
    ),
    (
        "decoder_joint-model.onnx",
        72_584_074,
        "9ce45c12b84c88366e259d51701eb6061bb40a8f3caee486fb7c3fe52ac131ac",
    ),
    (
        "vocab.txt",
        93_939,
        "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
    ),
];

/// The 25 European languages Parakeet TDT v3 covers (NVIDIA model card).
#[cfg(feature = "local-ml")]
const PARAKEET_LANGUAGES: &[&str] = &[
    "en", "es", "fr", "de", "bg", "hr", "cs", "da", "nl", "et", "fi", "el", "hu", "it", "lv", "lt",
    "mt", "pl", "pt", "ro", "sk", "sl", "sv", "ru", "uk",
];

/// Windowing: 30 s windows with a 3 s overlap; a token belongs to the window
/// holding its midpoint, so words are never cut at an edge.
#[cfg(feature = "local-ml")]
const WINDOW_SECS: f32 = 30.0;
#[cfg(feature = "local-ml")]
const OVERLAP_SECS: f32 = 3.0;
#[cfg(feature = "local-ml")]
const SAMPLE_RATE: u32 = 16_000;

/// Whether Parakeet v3 can transcribe `language` ("es", "es-ES", "auto").
/// "auto" is the model's own detection: always allowed.
#[cfg(feature = "local-ml")]
pub fn parakeet_supports_language(language: &str) -> bool {
    let lang = language.trim().to_lowercase();
    if lang == "auto" {
        return true;
    }
    let base = lang.split(['-', '_']).next().unwrap_or("");
    PARAKEET_LANGUAGES.contains(&base)
}

/// Configuration for the Parakeet engine.
#[cfg(feature = "local-ml")]
#[derive(Clone)]
pub struct ParakeetConfig {
    /// Directory holding (or to download) the model files.
    pub model_dir: std::path::PathBuf,
    /// Language code ("es") or "auto" for model detection.
    pub language: String,
}

/// Persistent Parakeet engine: the 2.5 GB model loads once and stays resident
/// for the app session (unlike the Whisper path, which reloads per call).
#[cfg(feature = "local-ml")]
pub struct ParakeetEngine {
    model: ParakeetTDT,
    language: String,
}

#[cfg(feature = "local-ml")]
impl ParakeetEngine {
    /// Download (if needed) + verify + load. Blocking; call on the worker.
    /// `on_progress` receives 0-100 for the whole operation.
    pub fn init(config: ParakeetConfig, on_progress: impl Fn(u8, &str)) -> Result<Self, String> {
        std::fs::create_dir_all(&config.model_dir).map_err(|e| {
            format!(
                "Failed to create Parakeet model dir {}: {e}",
                config.model_dir.display()
            )
        })?;
        ensure_model_files(&config.model_dir, &on_progress)?;
        on_progress(85, "loading");

        // Global ORT env is shared with embeddings/layout (idempotent).
        crate::nlp::embeddings::ensure_local_ort_init(&config.model_dir)?;

        let model = ParakeetTDT::from_pretrained(&config.model_dir, None)
            .map_err(|e| format!("Failed to load Parakeet TDT model: {e}"))?;
        on_progress(100, "ready");
        log::info!(target: "transcription", "Parakeet engine ready");
        Ok(Self {
            model,
            language: config.language,
        })
    }

    /// Transcribe an audio file. Same output shape as the Whisper path.
    pub fn transcribe(
        &mut self,
        audio_path: &str,
        on_progress: impl Fn(u8, &str),
    ) -> Result<TranscriptionResult, String> {
        on_progress(5, "reading");
        let audio = decode_audio_file(audio_path)?;
        if audio.samples.is_empty() {
            return Ok(TranscriptionResult {
                text: String::new(),
                language: self.language.clone(),
                segments: Vec::new(),
                duration_ms: 0,
            });
        }
        on_progress(10, "transcribing");

        let windows = split_windows(audio.samples.len());
        let mut kept: Vec<WindowToken> = Vec::new();
        for (index, window) in windows.iter().enumerate() {
            let chunk = audio.samples[window.start_sample..window.end_sample].to_vec();
            let result = self
                .model
                .transcribe_samples(chunk, SAMPLE_RATE, 1, Some(TimestampMode::Sentences))
                .map_err(|e| format!("Parakeet transcription failed: {e}"))?;
            for token in result.tokens {
                kept.push(WindowToken {
                    start: window.start_secs + token.start,
                    end: window.start_secs + token.end,
                    text: token.text,
                    owner: index,
                });
            }
            let pct = 10 + ((index + 1) as u8 * 80 / windows.len().max(1) as u8);
            on_progress(pct.min(90), "transcribing");
        }

        let segments = merge_windows(kept, &windows);
        let text = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        on_progress(95, "done");
        log::info!(
            target: "transcription",
            "Parakeet transcription complete: {} segments, {} chars",
            segments.len(),
            text.len()
        );
        Ok(TranscriptionResult {
            text,
            language: self.language.clone(),
            segments,
            duration_ms: audio.duration_ms,
        })
    }
}

/// A window over the sample buffer.
#[cfg(feature = "local-ml")]
#[derive(Debug, Clone, PartialEq)]
struct AudioWindow {
    start_sample: usize,
    end_sample: usize,
    start_secs: f32,
}

/// Split `total_samples` into 30 s windows with 3 s overlap (no overlap past
/// the end). A single short file is one window.
#[cfg(feature = "local-ml")]
fn split_windows(total_samples: usize) -> Vec<AudioWindow> {
    let window = (WINDOW_SECS * SAMPLE_RATE as f32) as usize;
    let overlap = (OVERLAP_SECS * SAMPLE_RATE as f32) as usize;
    let step = window.saturating_sub(overlap).max(1);
    let mut windows = Vec::new();
    let mut start = 0;
    while start < total_samples {
        let end = (start + window).min(total_samples);
        windows.push(AudioWindow {
            start_sample: start,
            end_sample: end,
            start_secs: start as f32 / SAMPLE_RATE as f32,
        });
        if end == total_samples {
            break;
        }
        start += step;
    }
    windows
}

/// A token with absolute timestamps and the window that produced it.
#[cfg(feature = "local-ml")]
#[derive(Debug, Clone)]
struct WindowToken {
    start: f32,
    end: f32,
    text: String,
    owner: usize,
}

/// Keep each token once: the window holding its midpoint wins. Tokens are
/// then grouped into [`Segment`]s in time order.
#[cfg(feature = "local-ml")]
fn merge_windows(mut tokens: Vec<WindowToken>, windows: &[AudioWindow]) -> Vec<Segment> {
    tokens.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut segments = Vec::new();
    for token in tokens {
        let mid = (token.start + token.end) / 2.0;
        let owner = owner_of(mid, windows);
        if owner != token.owner {
            continue;
        }
        let text = token.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        segments.push(Segment {
            start: token.start as f64,
            end: token.end as f64,
            text,
        });
    }
    segments
}

/// Index of the window whose exclusive zone holds `mid`. The exclusive zone
/// is the window minus the overlap margin on each side shared with a
/// neighbor (first window keeps its head, last keeps its tail).
#[cfg(feature = "local-ml")]
fn owner_of(mid: f32, windows: &[AudioWindow]) -> usize {
    for (index, window) in windows.iter().enumerate() {
        let start = window.start_secs + if index == 0 { 0.0 } else { OVERLAP_SECS / 2.0 };
        let end = window.start_secs
            + (window.end_sample - window.start_sample) as f32 / SAMPLE_RATE as f32
            - if index + 1 == windows.len() {
                0.0
            } else {
                OVERLAP_SECS / 2.0
            };
        if mid >= start && mid < end {
            return index;
        }
    }
    windows.len().saturating_sub(1)
}

/// Download any missing model file (resume-less, whole-file) and verify
/// size + sha256 of every file.
#[cfg(feature = "local-ml")]
fn ensure_model_files(
    model_dir: &std::path::Path,
    on_progress: &dyn Fn(u8, &str),
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build Parakeet download client: {e}"))?;
    for (index, (name, size, sha)) in PARAKEET_FILES.iter().enumerate() {
        let dest = model_dir.join(name);
        let ok = std::fs::metadata(&dest)
            .map(|m| m.len() == *size)
            .unwrap_or(false)
            && verify_sha256(&dest, sha).unwrap_or(false);
        if !ok {
            on_progress(
                download_progress(index, PARAKEET_FILES.len()),
                "downloading",
            );
            download_file(&client, name, &dest)?;
        }
        verify_size_and_hash(&dest, name, *size, sha)?;
        on_progress(
            download_progress(index + 1, PARAKEET_FILES.len()),
            "downloading",
        );
    }
    Ok(())
}

/// Download-phase progress (0-80): usize math then clamp, so the last file
/// (index + 1 == len) can never overflow u8 in debug builds.
#[cfg(feature = "local-ml")]
fn download_progress(done: usize, total: usize) -> u8 {
    ((done * 80 / total.max(1)).min(80)) as u8
}

#[cfg(feature = "local-ml")]
fn download_file(
    client: &reqwest::blocking::Client,
    name: &str,
    dest: &std::path::Path,
) -> Result<(), String> {
    let url = format!(
        "https://huggingface.co/{PARAKEET_HF_REPO}/resolve/{PARAKEET_HF_REV}/{PARAKEET_MODEL_SUBDIR}/{name}"
    );
    log::info!(target: "transcription", "Downloading Parakeet model file: {name}");
    let mut response = client
        .get(&url)
        .send()
        .map_err(|e| format!("Failed to download Parakeet model file {name}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to download Parakeet model file {name}: HTTP {}",
            response.status()
        ));
    }
    let tmp = dest.with_extension("part");
    let mut file =
        std::fs::File::create(&tmp).map_err(|e| format!("Failed to write {name}: {e}"))?;
    std::io::copy(&mut response, &mut file).map_err(|e| format!("Failed to write {name}: {e}"))?;
    std::fs::rename(&tmp, dest).map_err(|e| format!("Failed to store {name}: {e}"))?;
    Ok(())
}

#[cfg(feature = "local-ml")]
fn verify_size_and_hash(
    dest: &std::path::Path,
    name: &str,
    size: u64,
    sha: &str,
) -> Result<(), String> {
    let len = std::fs::metadata(dest)
        .map_err(|e| format!("Parakeet model file {name} missing: {e}"))?
        .len();
    if len != size {
        return Err(format!(
            "Parakeet model file {name} has {len} bytes, expected {size}"
        ));
    }
    if !verify_sha256(dest, sha).map_err(|e| format!("Failed to hash {name}: {e}"))? {
        return Err(format!(
            "Parakeet model file {name} failed sha256 verification"
        ));
    }
    Ok(())
}

#[cfg(feature = "local-ml")]
fn verify_sha256(path: &std::path::Path, expected_hex: &str) -> Result<bool, String> {
    use sha2::{Digest, Sha256};
    let bytes =
        std::fs::read(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()) == expected_hex)
}

#[cfg(all(test, feature = "local-ml"))]
mod tests {
    use super::*;

    #[test]
    fn parakeet_language_guard() {
        assert!(parakeet_supports_language("es"));
        assert!(parakeet_supports_language("es-ES"));
        assert!(parakeet_supports_language("auto"));
        assert!(parakeet_supports_language("pt"));
        assert!(!parakeet_supports_language("zh"));
        assert!(!parakeet_supports_language("ja"));
        assert!(!parakeet_supports_language(""));
    }

    #[test]
    fn download_progress_never_overflows_u8() {
        // Regression: `(index + 1) as u8 * 80` panicked with overflow on the
        // last file in debug builds. usize math then clamp instead.
        assert_eq!(download_progress(0, 4), 0);
        assert_eq!(download_progress(3, 4), 60);
        assert_eq!(download_progress(4, 4), 80);
        assert_eq!(download_progress(100, 4), 80);
    }

    #[test]
    fn short_audio_is_one_window() {
        let windows = split_windows(SAMPLE_RATE as usize * 10);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].start_sample, 0);
        assert_eq!(windows[0].end_sample, SAMPLE_RATE as usize * 10);
    }

    #[test]
    fn long_audio_windows_overlap() {
        let windows = split_windows(SAMPLE_RATE as usize * 65);
        assert!(windows.len() >= 3);
        for pair in windows.windows(2) {
            assert!(pair[0].end_sample > pair[1].start_sample);
            assert_eq!(
                pair[1].start_sample - pair[0].start_sample,
                (27.0 * SAMPLE_RATE as f32) as usize
            );
        }
        assert_eq!(
            windows.last().unwrap().end_sample,
            SAMPLE_RATE as usize * 65
        );
    }

    #[test]
    fn midpoint_ownership_dedupes_overlap() {
        // Two 30 s windows over 33 s of audio, overlapping 27-30 s. The
        // ownership boundary sits at t=28.5; tokens clearly on either side
        // survive only from their owning window.
        let total = SAMPLE_RATE as usize * 33;
        let windows = split_windows(total);
        assert_eq!(windows.len(), 2);
        let tokens = vec![
            WindowToken {
                start: 28.0,
                end: 28.2,
                text: "hola".to_string(),
                owner: 1,
            },
            WindowToken {
                start: 29.0,
                end: 29.2,
                text: "mundo".to_string(),
                owner: 1,
            },
            WindowToken {
                start: 5.0,
                end: 5.4,
                text: "inicio".to_string(),
                owner: 0,
            },
        ];
        let segments = merge_windows(tokens, &windows);
        let texts: Vec<_> = segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["inicio", "mundo"]);
    }
}
