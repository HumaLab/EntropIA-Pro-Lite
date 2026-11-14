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
use parakeet_rs::sortformer::Sortformer;
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

/// Nemotron-3 diarization (Sortformer v3) export for the optional speaker
/// pass. OpenMDW-1.1 (see THIRD_PARTY_NOTICES.md): attribution via the
/// LICENSE file downloaded alongside the weights.
#[cfg(feature = "local-ml")]
pub(crate) const DIAR_MODEL_SUBDIR: &str = "nemotron-3-diarization";
#[cfg(feature = "local-ml")]
const DIAR_FILES: &[(&str, u64, &str)] = &[
    (
        "nemotron3_diar_v3.onnx",
        402_167_515,
        "c32d6056f1ca52bdd50a74e1e7c66d91e5147bc29f19ffa4c39cbe5452397d15",
    ),
    (
        "LICENSE",
        2_660,
        "14cf93aed5ee7c72516170ecb65fb6d7e54ef19217d328c8b00b78eaf61c8b36",
    ),
];

/// Maximum speakers the local pipeline assigns. Beyond this the transcription
/// is kept without speaker labels and the UI suggests AssemblyAI.
#[cfg(feature = "local-ml")]
const MAX_LOCAL_SPEAKERS: usize = 4;

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
/// The 400 MB diarizer loads lazily on the first request that asks for it.
#[cfg(feature = "local-ml")]
pub struct ParakeetEngine {
    model: ParakeetTDT,
    language: String,
    model_dir: std::path::PathBuf,
    diarizer: Option<Sortformer>,
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
        ensure_model_files(&config.model_dir, PARAKEET_FILES, &on_progress)?;
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
            model_dir: config.model_dir,
            diarizer: None,
        })
    }

    /// Transcribe an audio file. Same output shape as the Whisper path, plus
    /// speaker labels when `diarize` is set. Returns the result and an
    /// optional warning (e.g. more than [`MAX_LOCAL_SPEAKERS`] speakers:
    /// transcription kept, labels skipped).
    pub fn transcribe(
        &mut self,
        audio_path: &str,
        diarize: bool,
        on_progress: impl Fn(u8, &str),
    ) -> Result<(TranscriptionResult, Option<String>), String> {
        on_progress(5, "reading");
        let audio = decode_audio_file(audio_path)?;
        if audio.samples.is_empty() {
            return Ok((
                TranscriptionResult {
                    text: String::new(),
                    language: self.language.clone(),
                    segments: Vec::new(),
                    duration_ms: 0,
                },
                None,
            ));
        }
        on_progress(10, "transcribing");

        let mode = if diarize {
            TimestampMode::Words
        } else {
            TimestampMode::Sentences
        };
        let windows = split_windows(audio.samples.len());
        let mut kept: Vec<WindowToken> = Vec::new();
        for (index, window) in windows.iter().enumerate() {
            let chunk = audio.samples[window.start_sample..window.end_sample].to_vec();
            let result = self
                .model
                .transcribe_samples(chunk, SAMPLE_RATE, 1, Some(mode))
                .map_err(|e| format!("Parakeet transcription failed: {e}"))?;
            for token in result.tokens {
                kept.push(WindowToken {
                    start: window.start_secs + token.start,
                    end: window.start_secs + token.end,
                    text: token.text,
                    owner: index,
                });
            }
            on_progress(
                stage_progress(index + 1, windows.len(), 10, 80),
                "transcribing",
            );
        }

        let (segments, warning) = if diarize {
            self.assign_speakers(kept, &windows, &audio.samples)?
        } else {
            (merge_windows(kept, &windows), None)
        };
        let text = segments
            .iter()
            .map(|s| match &s.speaker {
                Some(who) => format!("{who}: {}", s.text),
                None => s.text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        on_progress(95, "done");
        log::info!(
            target: "transcription",
            "Parakeet transcription complete: {} segments, {} chars{}",
            segments.len(),
            text.len(),
            warning.as_deref().unwrap_or("")
        );
        Ok((
            TranscriptionResult {
                text,
                language: self.language.clone(),
                segments,
                duration_ms: audio.duration_ms,
            },
            warning,
        ))
    }

    /// Run the Sortformer pass and map every word to its speaker. Words are
    /// merged into same-speaker segments; speakers are labeled Hablante A…
    /// in first-appearance order (matching the AssemblyAI text format).
    /// More than [`MAX_LOCAL_SPEAKERS`] distinct speakers: labels are
    /// skipped and a warning suggests AssemblyAI instead.
    fn assign_speakers(
        &mut self,
        tokens: Vec<WindowToken>,
        windows: &[AudioWindow],
        samples: &[f32],
    ) -> Result<(Vec<Segment>, Option<String>), String> {
        let diarizer = self.ensure_diarizer()?;
        let raw = diarizer
            .diarize(samples.to_vec(), SAMPLE_RATE, 1)
            .map_err(|e| format!("Parakeet diarization failed: {e}"))?;
        let spans: Vec<SpeakerSpan> = raw
            .iter()
            .map(|s| SpeakerSpan {
                start: s.start as f32 / SAMPLE_RATE as f32,
                end: s.end as f32 / SAMPLE_RATE as f32,
                speaker: s.speaker_id,
            })
            .collect();
        let mut distinct: Vec<usize> = spans.iter().map(|s| s.speaker).collect();
        distinct.sort_unstable();
        distinct.dedup();
        if distinct.len() > MAX_LOCAL_SPEAKERS {
            return Ok((
                 merge_windows(tokens, windows),
                 Some(format!(
                     "Se detectaron {} hablantes y el modo local asigna hasta {MAX_LOCAL_SPEAKERS}: texto sin etiquetas. Probá con AssemblyAI para reuniones grandes.",
                     distinct.len()
                 )),
             ));
        }
        let mut words: Vec<WordToken> = tokens
            .into_iter()
            .map(|t| {
                let speaker = best_speaker(t.start, t.end, &spans);
                WordToken {
                    start: t.start,
                    end: t.end,
                    text: t.text,
                    owner: t.owner,
                    speaker,
                }
            })
            .collect();
        // Drop words from windows that lost the midpoint race, as in merge_windows.
        words.retain(|w| owner_of((w.start + w.end) / 2.0, windows) == w.owner);
        words.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok((merge_labeled(words), None))
    }

    /// Load the diarizer on first use (402 MB, only when asked).
    fn ensure_diarizer(&mut self) -> Result<&mut Sortformer, String> {
        if self.diarizer.is_none() {
            let dir = self.model_dir.join(DIAR_MODEL_SUBDIR);
            std::fs::create_dir_all(&dir).map_err(|e| {
                format!(
                    "Failed to create diarization model dir {}: {e}",
                    dir.display()
                )
            })?;
            ensure_model_files(&dir, DIAR_FILES, &|_, _| {})?;
            let diarizer = Sortformer::new(dir.join("nemotron3_diar_v3.onnx"))
                .map_err(|e| format!("Failed to load diarization model: {e}"))?;
            self.diarizer = Some(diarizer);
        }
        self.diarizer
            .as_mut()
            .ok_or_else(|| "Diarizer just initialized".to_string())
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
            speaker: None,
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

/// A diarization span in seconds with the Sortformer speaker id.
#[cfg(feature = "local-ml")]
#[derive(Debug, Clone)]
struct SpeakerSpan {
    start: f32,
    end: f32,
    speaker: usize,
}

/// A word with absolute timestamps, its producing window, and the assigned
/// speaker id (None when no diarization span overlaps it).
#[cfg(feature = "local-ml")]
#[derive(Debug, Clone)]
struct WordToken {
    start: f32,
    end: f32,
    text: String,
    owner: usize,
    speaker: Option<usize>,
}

/// Speaker id whose span has the largest overlap with `[start, end)`.
/// Silence between turns maps to None instead of the nearest voice.
#[cfg(feature = "local-ml")]
fn best_speaker(start: f32, end: f32, spans: &[SpeakerSpan]) -> Option<usize> {
    spans
        .iter()
        .map(|s| {
            let overlap = (end.min(s.end) - start.max(s.start)).max(0.0);
            (overlap, s.speaker)
        })
        .filter(|(overlap, _)| *overlap > 0.0)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, speaker)| speaker)
}

/// Merge consecutive words with the same speaker into [`Segment`]s, labeled
/// Hablante A… in first-appearance order (the AssemblyAI text format).
#[cfg(feature = "local-ml")]
fn merge_labeled(mut words: Vec<WordToken>) -> Vec<Segment> {
    words.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut order: Vec<usize> = Vec::new();
    for word in &words {
        if let Some(speaker) = word.speaker {
            if !order.contains(&speaker) {
                order.push(speaker);
            }
        }
    }
    let label = |speaker: Option<usize>| {
        speaker.and_then(|s| {
            order
                .iter()
                .position(|o| *o == s)
                .map(|i| format!("Hablante {}", (b'A' + i.min(25) as u8) as char))
        })
    };
    let mut segments = Vec::new();
    let mut current: Option<Segment> = None;
    let flush = |current: &mut Option<Segment>, segments: &mut Vec<Segment>| {
        if let Some(segment) = current.take() {
            // Skip punctuation-only fragments (the sentence splitter can
            // isolate a "." as its own token).
            if segment.text.chars().any(|c| c.is_alphanumeric()) {
                segments.push(segment);
            }
        }
    };
    for word in words {
        let text = word.text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        let speaker = label(word.speaker);
        match current.as_mut() {
            Some(segment) if segment.speaker == speaker => {
                segment.end = word.end as f64;
                segment.text.push(' ');
                segment.text.push_str(&text);
            }
            _ => {
                flush(&mut current, &mut segments);
                current = Some(Segment {
                    start: word.start as f64,
                    end: word.end as f64,
                    text,
                    speaker,
                });
            }
        }
    }
    flush(&mut current, &mut segments);
    segments
}

/// Download any missing model file (resume-less, whole-file) and verify
/// size + sha256 of every file.
#[cfg(feature = "local-ml")]
fn ensure_model_files(
    model_dir: &std::path::Path,
    files: &[(&str, u64, &str)],
    on_progress: &dyn Fn(u8, &str),
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build Parakeet download client: {e}"))?;
    let subdir = model_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    for (index, (name, size, sha)) in files.iter().enumerate() {
        let dest = model_dir.join(name);
        let ok = std::fs::metadata(&dest)
            .map(|m| m.len() == *size)
            .unwrap_or(false)
            && verify_sha256(&dest, sha).unwrap_or(false);
        if !ok {
            on_progress(download_progress(index, files.len()), "downloading");
            download_file(&client, &subdir, name, &dest)?;
        }
        verify_size_and_hash(&dest, name, *size, sha)?;
        on_progress(download_progress(index + 1, files.len()), "downloading");
    }
    Ok(())
}

/// Download-phase progress (0-80): usize math then clamp, so the last file
/// (index + 1 == len) can never overflow u8 in debug builds.
#[cfg(feature = "local-ml")]
fn download_progress(done: usize, total: usize) -> u8 {
    stage_progress(done, total, 0, 80)
}

/// Progress within a stage band: `base + span * done/total`, clamped.
/// usize math throughout: stage counts (windows, files) can exceed u8.
#[cfg(feature = "local-ml")]
fn stage_progress(done: usize, total: usize, base: u8, span: u8) -> u8 {
    base.saturating_add(((done * span as usize / total.max(1)).min(span as usize)) as u8)
}

#[cfg(feature = "local-ml")]
fn download_file(
    client: &reqwest::blocking::Client,
    subdir: &str,
    name: &str,
    dest: &std::path::Path,
) -> Result<(), String> {
    let url = format!(
        "https://huggingface.co/{PARAKEET_HF_REPO}/resolve/{PARAKEET_HF_REV}/{subdir}/{name}"
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
    fn best_speaker_picks_max_overlap() {
        let spans = vec![
            SpeakerSpan {
                start: 0.0,
                end: 5.0,
                speaker: 0,
            },
            SpeakerSpan {
                start: 5.0,
                end: 10.0,
                speaker: 1,
            },
        ];
        assert_eq!(best_speaker(1.0, 2.0, &spans), Some(0));
        assert_eq!(best_speaker(6.0, 8.0, &spans), Some(1));
        // Mostly in A (3 s) with a 1 s tail in B.
        assert_eq!(best_speaker(2.0, 6.0, &spans), Some(0));
        // Silence between turns maps to no speaker, not the nearest voice.
        assert_eq!(best_speaker(20.0, 22.0, &spans), None);
    }

    #[test]
    fn merge_labeled_groups_runs_and_labels_by_appearance() {
        // Speaker 1 speaks first: Hablante A. Speaker 0 second: Hablante B.
        let words = vec![
            WordToken {
                start: 0.0,
                end: 0.5,
                text: "hola".into(),
                owner: 0,
                speaker: Some(1),
            },
            WordToken {
                start: 0.6,
                end: 1.0,
                text: "qué".into(),
                owner: 0,
                speaker: Some(1),
            },
            WordToken {
                start: 1.1,
                end: 1.5,
                text: "tal".into(),
                owner: 0,
                speaker: Some(0),
            },
            WordToken {
                start: 1.6,
                end: 2.0,
                text: "estás".into(),
                owner: 0,
                speaker: Some(0),
            },
            WordToken {
                start: 2.1,
                end: 2.4,
                text: "(risa)".into(),
                owner: 0,
                speaker: None,
            },
        ];
        let segments = merge_labeled(words);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].speaker.as_deref(), Some("Hablante A"));
        assert_eq!(segments[0].text, "hola qué");
        assert_eq!(segments[1].speaker.as_deref(), Some("Hablante B"));
        assert_eq!(segments[1].text, "tal estás");
        assert_eq!(segments[2].speaker, None);
        assert_eq!(segments[2].text, "(risa)");
    }

    #[test]
    fn merge_labeled_folds_punctuation_only_fragments() {
        let words = vec![
            WordToken {
                start: 0.0,
                end: 0.5,
                text: "hola".into(),
                owner: 0,
                speaker: Some(1),
            },
            WordToken {
                start: 0.6,
                end: 1.0,
                text: "qué".into(),
                owner: 0,
                speaker: Some(1),
            },
            WordToken {
                start: 1.1,
                end: 1.2,
                text: ".".into(),
                owner: 0,
                speaker: Some(0),
            },
            WordToken {
                start: 1.3,
                end: 1.7,
                text: "tal".into(),
                owner: 0,
                speaker: Some(0),
            },
        ];
        let segments = merge_labeled(words);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "hola qué");
        // The lone "." joins the B run instead of standing alone.
        assert_eq!(segments[1].text, ". tal");
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

    /// Manual benchmark harness (ignored by default). Protocol: three ES
    /// audios — 2-person interview, 4-person meeting, noisy clip — placed
    /// in a directory and measured with and without diarization.
    ///
    /// Run: `PARAKEET_BENCH_AUDIOS=/path/audios
    ///   PARAKEET_BENCH_MODELS=/path/to/caches/hf
    ///   ORT_DYLIB_PATH=/path/to/libonnxruntime.so
    ///   cargo test --features local-ml -- --ignored bench_parakeet_diarization
    ///   --nocapture`
    #[test]
    #[ignore]
    fn bench_parakeet_diarization() {
        use std::time::Instant;

        let audios = std::env::var("PARAKEET_BENCH_AUDIOS")
            .expect("set PARAKEET_BENCH_AUDIOS to a directory with wav files");
        let models = std::env::var("PARAKEET_BENCH_MODELS")
            .expect("set PARAKEET_BENCH_MODELS to the caches/hf dir");
        let nop = |_: u8, _: &str| {};
        let mut engine = ParakeetEngine::init(
            ParakeetConfig {
                model_dir: std::path::PathBuf::from(&models).join(PARAKEET_MODEL_SUBDIR),
                language: "es".to_string(),
            },
            nop,
        )
        .expect("load parakeet");
        let mut entries: Vec<_> = std::fs::read_dir(&audios)
            .expect("read audios dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "wav").unwrap_or(false))
            .collect();
        entries.sort();
        assert!(!entries.is_empty(), "no wav files in {audios}");
        for path in entries {
            let audio = decode_audio_file(path.to_str().unwrap()).expect("decode");
            let secs = audio.duration_ms as f64 / 1000.0;
            for diarize in [false, true] {
                let start = Instant::now();
                let (result, warning) = engine
                    .transcribe(path.to_str().unwrap(), diarize, nop)
                    .expect("transcribe");
                let elapsed = start.elapsed().as_secs_f64();
                let speakers: usize = result
                    .segments
                    .iter()
                    .filter_map(|s| s.speaker.clone())
                    .collect::<std::collections::HashSet<_>>()
                    .len();
                println!(
                     "BENCH file={} diarize={} secs={:.1} wall={:.1}s rtf={:.2} segments={} speakers={} warning={:?} text={:.80}",
                     path.file_name().unwrap().to_string_lossy(),
                     diarize,
                     secs,
                     elapsed,
                     elapsed / secs.max(0.1),
                     result.segments.len(),
                     speakers,
                     warning,
                     result.text.replace('\n', " | "),
                 );
                assert!(!result.text.trim().is_empty(), "empty transcription");
            }
        }
    }
}
