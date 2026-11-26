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
use parakeet_rs::sortformer::{DiarizationConfig, Sortformer};
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

/// Whether `language` ("es", "es-AR", "ES") asks for Spanish.
#[cfg(feature = "local-ml")]
fn is_spanish(language: &str) -> bool {
    language
        .trim()
        .split(['-', '_'])
        .next()
        .is_some_and(|base| base.eq_ignore_ascii_case("es"))
}

/// Whether a transcript asked for in Spanish came out largely in English.
///
/// Parakeet TDT v3 takes no language parameter: it picks the language per
/// window, and on Spanish interviews it can switch to translating into
/// English. Counts function words unique to each language (words both share,
/// like `a`, `no` or `me`, are left out).
#[cfg(feature = "local-ml")]
pub(crate) fn drifted_from_spanish(text: &str) -> bool {
    let (english, spanish) = function_word_counts(text);
    // A few English function words (a quote, a title) are not drift; a third
    // of the function words in English is.
    english >= 8 && english * 3 >= english + spanish
}

/// Whether one ~30 s window drifted into English. The whole-file check
/// needs 8 English function words, more than a single window holds; here 3
/// are enough when they are at least 30% of the function words.
#[cfg(feature = "local-ml")]
fn window_drifted(text: &str) -> bool {
    let (english, spanish) = function_word_counts(text);
    english >= 3 && english * 10 >= (english + spanish) * 3
}

/// (English, Spanish) function words in `text`.
#[cfg(feature = "local-ml")]
fn function_word_counts(text: &str) -> (usize, usize) {
    const ENGLISH: &[&str] = &[
        "the", "of", "and", "to", "in", "is", "that", "it", "was", "for", "with", "they", "this",
        "from", "by", "are", "were", "which", "their", "been", "would", "there", "what", "when",
        "who", "will", "about", "into", "than", "them", "these", "its", "our", "we", "you", "but",
        "not", "or", "an", "at", "on", "my", "since", "during", "have", "had",
    ];
    const SPANISH: &[&str] = &[
        "de", "la", "que", "el", "en", "y", "los", "se", "del", "las", "un", "por", "con", "una",
        "su", "para", "es", "al", "lo", "como", "pero", "sus", "le", "ya", "fue", "este", "ha",
        "porque", "esta", "entre", "cuando", "muy", "sin", "sobre", "hasta", "hay", "donde", "nos",
        "durante", "todo", "eso",
    ];
    let (mut english, mut spanish) = (0usize, 0usize);
    for word in text
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
    {
        let word = word.to_lowercase();
        if ENGLISH.contains(&word.as_str()) {
            english += 1;
        } else if SPANISH.contains(&word.as_str()) {
            spanish += 1;
        }
    }
    (english, spanish)
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
    /// Verify + load. Blocking; call on the worker. The model must already be
    /// on disk ([`download_model_files`]): loading never downloads, so a
    /// dictation can no longer hang on a silent 2.5 GB transfer (R11).
    /// `on_progress` receives 0-100 for the whole operation.
    pub fn init(config: ParakeetConfig, on_progress: impl Fn(u8, &str)) -> Result<Self, String> {
        if !missing_model_files(&config.model_dir, PARAKEET_FILES).is_empty() {
            return Err(PARAKEET_NOT_DOWNLOADED.to_string());
        }
        on_progress(85, "loading");

        let model =
            ParakeetTDT::from_pretrained(&config.model_dir, Some(ort_config(&config.model_dir)?))
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
        // Parakeet takes no language; on Spanish audio a window can drift into
        // English and is decoded again in short pieces.
        let redo_drift = is_spanish(&self.language);
        let (kept, redone) = decode_windows(
            &mut TdtDecoder {
                model: &mut self.model,
                mode,
            },
            &audio.samples,
            &windows,
            redo_drift,
            |done| {
                on_progress(stage_progress(done, windows.len(), 10, 80), "transcribing");
            },
        )?;
        if redone > 0 {
            log::warn!(
                target: "transcription",
                "Parakeet re-decoded {redone} of {} windows in short segments (English drift)",
                windows.len()
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
            download_model_files(&dir, DIAR_FILES, &mut |_, _| {})?;
            let diarizer = Sortformer::with_config(
                dir.join("nemotron3_diar_v3.onnx"),
                Some(ort_config(&self.model_dir)?),
                DiarizationConfig::default(),
            )
            .map_err(|e| format!("Failed to load diarization model: {e}"))?;
            self.diarizer = Some(diarizer);
        }
        self.diarizer
            .as_mut()
            .ok_or_else(|| "Diarizer just initialized".to_string())
    }
}

/// Run Parakeet sessions in the process-wide ONNX Runtime environment, shared
/// with embeddings and OCR layout: since ort 2.0.0-rc.15 a session needs an
/// explicit environment and a process may build only one.
#[cfg(feature = "local-ml")]
fn ort_config(model_dir: &std::path::Path) -> Result<parakeet_rs::ExecutionConfig, String> {
    let environment = crate::nlp::embeddings::ensure_local_ort_init(model_dir)?;
    Ok(parakeet_rs::ExecutionConfig::new().with_environment(environment))
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

/// Shortest and longest piece a drifted window is decoded again in. Parakeet
/// v3 drifts into English on long Spanish context and stays Spanish on short
/// pieces: on the radio program that drifted, 3-6 s pieces took the English
/// share of function words from 0.75 to 0.02 (lab run 2026-11-22, see
/// `odd/plans/parakeet-spanish-drift-1.8.8.md`). Shorter pieces lose context.
#[cfg(feature = "local-ml")]
const PIECE_MIN_SECS: f32 = 3.0;
#[cfg(feature = "local-ml")]
const PIECE_MAX_SECS: f32 = 6.0;
/// Frame the quietest-point search measures loudness over.
#[cfg(feature = "local-ml")]
const PAUSE_FRAME_SECS: f32 = 0.02;

/// Split `samples` into contiguous pieces of `min_secs`..`max_secs`, each cut
/// at the quietest 20 ms frame in that range, so a cut falls in a pause and
/// not inside a word. The last piece takes what is left and may be shorter.
/// Returns sample ranges `(start, end)` relative to `samples`.
#[cfg(feature = "local-ml")]
fn pause_cuts(samples: &[f32], min_secs: f32, max_secs: f32) -> Vec<(usize, usize)> {
    if samples.is_empty() {
        return Vec::new();
    }
    let frame = ((PAUSE_FRAME_SECS * SAMPLE_RATE as f32) as usize).max(1);
    let loudness: Vec<f32> = samples
        .chunks(frame)
        .map(|chunk| chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32)
        .collect();
    let min_frames = ((min_secs / PAUSE_FRAME_SECS) as usize).max(1);
    let max_frames = ((max_secs / PAUSE_FRAME_SECS) as usize).max(min_frames);
    let mut cuts = vec![0usize];
    loop {
        let last = *cuts.last().unwrap_or(&0);
        if last + max_frames >= loudness.len() {
            break;
        }
        let quietest = (last + min_frames..=last + max_frames)
            .min_by(|a, b| {
                loudness[*a]
                    .partial_cmp(&loudness[*b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(last + max_frames);
        cuts.push(quietest);
    }
    let mut pieces: Vec<(usize, usize)> = cuts
        .windows(2)
        .map(|pair| (pair[0] * frame, pair[1] * frame))
        .collect();
    let tail_start = cuts.last().copied().unwrap_or(0) * frame;
    pieces.push((tail_start, samples.len()));
    pieces
}

/// A token as the model returns it, timed from the start of its input.
#[cfg(feature = "local-ml")]
#[derive(Debug, Clone)]
struct DecodedToken {
    start: f32,
    end: f32,
    text: String,
}

/// Turns one stretch of samples into timed tokens: the seam between the
/// window logic and the model, so the logic is tested without the model.
#[cfg(feature = "local-ml")]
trait WindowDecoder {
    fn decode(&mut self, samples: &[f32]) -> Result<Vec<DecodedToken>, String>;
}

/// Parakeet TDT with the timestamp mode of the current request.
#[cfg(feature = "local-ml")]
struct TdtDecoder<'a> {
    model: &'a mut ParakeetTDT,
    mode: TimestampMode,
}

#[cfg(feature = "local-ml")]
impl WindowDecoder for TdtDecoder<'_> {
    fn decode(&mut self, samples: &[f32]) -> Result<Vec<DecodedToken>, String> {
        let result = self
            .model
            .transcribe_samples(samples.to_vec(), SAMPLE_RATE, 1, Some(self.mode))
            .map_err(|e| format!("Parakeet transcription failed: {e}"))?;
        Ok(result
            .tokens
            .into_iter()
            .map(|token| DecodedToken {
                start: token.start,
                end: token.end,
                text: token.text,
            })
            .collect())
    }
}

/// Decode every window. With `redo_drift` (Spanish audio), a window whose
/// text drifted into English ([`window_drifted`]) is decoded again as short
/// pause-cut pieces ([`pause_cuts`]), which keep Spanish. The pieces' tokens
/// get absolute times and keep the window as their owner, so the midpoint
/// ownership and the speaker pass work as for any window. Returns the
/// tokens and how many windows were decoded again. `on_window` receives the
/// count of windows done.
#[cfg(feature = "local-ml")]
fn decode_windows(
    decoder: &mut impl WindowDecoder,
    samples: &[f32],
    windows: &[AudioWindow],
    redo_drift: bool,
    mut on_window: impl FnMut(usize),
) -> Result<(Vec<WindowToken>, usize), String> {
    let mut kept = Vec::new();
    let mut redone = 0;
    for (index, window) in windows.iter().enumerate() {
        let chunk = &samples[window.start_sample..window.end_sample];
        let mut pieces = vec![(0usize, decoder.decode(chunk)?)];
        if redo_drift {
            let text = pieces[0]
                .1
                .iter()
                .map(|token| token.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if window_drifted(&text) {
                redone += 1;
                pieces = pause_cuts(chunk, PIECE_MIN_SECS, PIECE_MAX_SECS)
                    .into_iter()
                    .map(|(start, end)| decoder.decode(&chunk[start..end]).map(|t| (start, t)))
                    .collect::<Result<_, _>>()?;
            }
        }
        for (offset, tokens) in pieces {
            let offset_secs = window.start_secs + offset as f32 / SAMPLE_RATE as f32;
            kept.extend(tokens.into_iter().map(|token| WindowToken {
                start: offset_secs + token.start,
                end: offset_secs + token.end,
                text: token.text,
                owner: index,
            }));
        }
        on_window(index + 1);
    }
    Ok((kept, redone))
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

/// How far (seconds) a word may sit outside every diarization span and still
/// take the nearest one's speaker. Parakeet's word edges and Sortformer's
/// turn edges disagree by a few hundred ms, so without this a quarter of the
/// words of a long interview came out with no speaker.
#[cfg(feature = "local-ml")]
const SPEAKER_SNAP_SECS: f32 = 1.5;

/// Speaker id whose span has the largest overlap with `[start, end)`. A word
/// that overlaps no span (it sits in a gap, or has zero duration) takes the
/// nearest span within [`SPEAKER_SNAP_SECS`]; farther than that it is
/// silence between turns and maps to None.
#[cfg(feature = "local-ml")]
fn best_speaker(start: f32, end: f32, spans: &[SpeakerSpan]) -> Option<usize> {
    let overlapping = spans
        .iter()
        .map(|s| {
            let overlap = (end.min(s.end) - start.max(s.start)).max(0.0);
            (overlap, s.speaker)
        })
        .filter(|(overlap, _)| *overlap > 0.0)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, speaker)| speaker);
    if overlapping.is_some() {
        return overlapping;
    }
    spans
        .iter()
        .map(|s| {
            let gap = (s.start - end).max(start - s.end).max(0.0);
            (gap, s.speaker)
        })
        .filter(|(gap, _)| *gap <= SPEAKER_SNAP_SECS)
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, speaker)| speaker)
}

/// Punctuation Parakeet emits as its own word that belongs to the text before
/// it (the same set parakeet-rs attaches when it formats sentences).
#[cfg(feature = "local-ml")]
fn is_trailing_punctuation(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | ')' | '…'))
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
        if is_trailing_punctuation(word.text.trim()) {
            continue;
        }
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
        // Punctuation closes the text before it, with no leading space and
        // whatever speaker its own timestamp got; with nothing before it
        // (the transcript opens with it) it is dropped.
        if is_trailing_punctuation(&text) {
            if let Some(segment) = current.as_mut() {
                segment.end = segment.end.max(word.end as f64);
                segment.text.push_str(&text);
            }
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

/// Progress within a stage band: `base + span * done/total`, clamped.
/// usize math throughout: stage counts (windows, files) can exceed u8.
#[cfg(feature = "local-ml")]
fn stage_progress(done: usize, total: usize, base: u8, span: u8) -> u8 {
    base.saturating_add(((done * span as usize / total.max(1)).min(span as usize)) as u8)
}

/// What every local transcription path says while the model is missing.
#[cfg(feature = "local-ml")]
pub(crate) const PARAKEET_NOT_DOWNLOADED: &str =
    "El modelo Parakeet no está descargado. Descargalo en Configuración > Transcripción local (~2,5 GB).";

/// `<cache>/models/parakeet-tdt-0.6b-v3`, with the other downloaded models
/// (R26). A copy an earlier build left in a runtime's Hugging Face cache is
/// adopted first, so it is not downloaded again.
#[cfg(feature = "local-ml")]
pub(crate) fn parakeet_model_dir(db_path: &std::path::Path) -> std::path::PathBuf {
    let dir = crate::path_utils::models_dir(db_path).join(PARAKEET_MODEL_SUBDIR);
    let legacy = legacy_parakeet_dirs(&crate::path_utils::model_root(db_path));
    adopt_legacy_model_files(&dir, &legacy, PARAKEET_FILES);
    dir
}

/// Where earlier builds kept the model: the Hugging Face cache of each
/// runtime version (`runtime/<ver>/caches/hf`), which a runtime update
/// abandoned, or the dev fallback `hf_cache`.
#[cfg(feature = "local-ml")]
fn legacy_parakeet_dirs(cache_root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![cache_root.join("hf_cache").join(PARAKEET_MODEL_SUBDIR)];
    if let Ok(entries) = std::fs::read_dir(cache_root.join("runtime")) {
        dirs.extend(entries.flatten().map(|entry| {
            entry
                .path()
                .join("caches")
                .join("hf")
                .join(PARAKEET_MODEL_SUBDIR)
        }));
    }
    dirs
}

/// Move finished files (exact size) and the diarizer folder from `legacy`
/// into `target`. Same volume: a rename, never a 2.5 GB copy.
#[cfg(feature = "local-ml")]
fn adopt_legacy_model_files(
    target: &std::path::Path,
    legacy: &[std::path::PathBuf],
    files: &[(&str, u64, &str)],
) {
    for source in legacy.iter().filter(|source| source.is_dir()) {
        if source == target {
            continue;
        }
        let mut moves: Vec<(std::path::PathBuf, std::path::PathBuf)> = files
            .iter()
            .filter(|(name, size, _)| {
                !has_size(&target.join(name), *size) && has_size(&source.join(name), *size)
            })
            .map(|(name, _, _)| (source.join(name), target.join(name)))
            .collect();
        let diarizer = source.join(DIAR_MODEL_SUBDIR);
        if diarizer.is_dir() && !target.join(DIAR_MODEL_SUBDIR).exists() {
            moves.push((diarizer, target.join(DIAR_MODEL_SUBDIR)));
        }
        if moves.is_empty() || std::fs::create_dir_all(target).is_err() {
            continue;
        }
        for (from, to) in moves {
            match std::fs::rename(&from, &to) {
                Ok(()) => log::info!(
                    target: "transcription",
                    "Parakeet: adopted {} from {}",
                    to.display(),
                    from.display()
                ),
                Err(error) => log::warn!(
                    target: "transcription",
                    "Parakeet: could not adopt {}: {error}",
                    from.display()
                ),
            }
        }
    }
}

#[cfg(feature = "local-ml")]
fn has_size(path: &std::path::Path, size: u64) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == size)
}

/// The files of `files` not finished in `model_dir`. A file is renamed into
/// place only after its sha256 matched, so its size alone tells finished
/// from missing: the old check read the 2.5 GB file into memory, twice, on
/// every engine start (R23).
#[cfg(feature = "local-ml")]
pub(crate) fn missing_model_files<'a>(
    model_dir: &std::path::Path,
    files: &[(&'a str, u64, &'a str)],
) -> Vec<(&'a str, u64, &'a str)> {
    files
        .iter()
        .copied()
        .filter(|(name, size, _)| !has_size(&model_dir.join(name), *size))
        .collect()
}

/// Whether the Parakeet transcription model is ready to load.
#[cfg(feature = "local-ml")]
pub(crate) fn parakeet_installed(model_dir: &std::path::Path) -> bool {
    missing_model_files(model_dir, PARAKEET_FILES).is_empty()
}

/// Bytes still to download for Parakeet, and the model's total size.
#[cfg(feature = "local-ml")]
pub(crate) fn parakeet_download_size(model_dir: &std::path::Path) -> (u64, u64) {
    let total = PARAKEET_FILES.iter().map(|(_, size, _)| size).sum();
    let missing = missing_model_files(model_dir, PARAKEET_FILES)
        .iter()
        .map(|(_, size, _)| size)
        .sum();
    (missing, total)
}

/// Download the Parakeet transcription model. `on_progress(done, total)`
/// counts bytes over the files still missing.
#[cfg(feature = "local-ml")]
pub(crate) fn download_parakeet_model(
    model_dir: &std::path::Path,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    download_model_files(model_dir, PARAKEET_FILES, on_progress)
}

/// Download every missing file of `files`, resuming a partial one, and verify
/// size and sha256 while the bytes stream to disk.
#[cfg(feature = "local-ml")]
fn download_model_files(
    model_dir: &std::path::Path,
    files: &[(&str, u64, &str)],
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    let missing = missing_model_files(model_dir, files);
    if missing.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(model_dir)
        .map_err(|e| format!("Failed to create model dir {}: {e}", model_dir.display()))?;
    let subdir = model_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let client = crate::llm::download::build_model_download_client()?;
    let total: u64 = missing.iter().map(|(_, size, _)| size).sum();
    crate::llm::download::ensure_room_in(model_dir, remaining_bytes(model_dir, &missing))?;
    let mut done_before = 0_u64;
    for (name, size, sha) in missing {
        let url = format!(
            "https://huggingface.co/{PARAKEET_HF_REPO}/resolve/{PARAKEET_HF_REV}/{subdir}/{name}"
        );
        download_file(
            &client,
            &url,
            &model_dir.join(name),
            size,
            sha,
            &mut |bytes| on_progress(done_before + bytes, total),
        )?;
        done_before += size;
    }
    Ok(())
}

/// Bytes still to download for `missing`: a `.part` left by an interrupted run
/// resumes, so what it already holds needs no new room.
#[cfg(feature = "local-ml")]
fn remaining_bytes(model_dir: &std::path::Path, missing: &[(&str, u64, &str)]) -> u64 {
    missing
        .iter()
        .map(|(name, size, _)| {
            let have = std::fs::metadata(partial_path(&model_dir.join(name)))
                .map(|meta| meta.len())
                .unwrap_or(0);
            size.saturating_sub(have)
        })
        .sum()
}

/// `name.part` beside the destination. `with_extension("part")` turned
/// `encoder-model.onnx.data` into `encoder-model.onnx.part`, the stem of a
/// different file.
#[cfg(feature = "local-ml")]
fn partial_path(dest: &std::path::Path) -> std::path::PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dest.with_file_name(format!("{name}.part"))
}

/// Stream one file to `dest`. A `.part` left by an interrupted run resumes
/// with an HTTP range instead of starting over: closing the app at 85% of a
/// 2.5 GB file used to cost the whole download (R23). The sha256 is computed
/// as the bytes arrive; the file is renamed into place only if it matches.
#[cfg(feature = "local-ml")]
fn download_file(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &std::path::Path,
    size: u64,
    expected_sha: &str,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};

    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let part = partial_path(dest);
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > size {
        let _ = std::fs::remove_file(&part);
        have = 0;
    }
    let mut hasher = Sha256::new();
    if have > 0 {
        let mut existing = std::fs::File::open(&part)
            .map_err(|e| format!("Failed to reopen partial {name}: {e}"))?;
        let mut buffer = vec![0_u8; 1 << 20];
        loop {
            let read = existing
                .read(&mut buffer)
                .map_err(|e| format!("Failed to read partial {name}: {e}"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        on_bytes(have);
    }

    if have < size {
        log::info!(
            target: "transcription",
            "Downloading Parakeet model file: {name} (from byte {have} of {size})"
        );
        let mut request = client.get(url);
        if have > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let mut response = request
            .send()
            .map_err(|e| format!("Failed to download Parakeet model file {name}: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "Failed to download Parakeet model file {name}: HTTP {status}"
            ));
        }
        let resumed = have > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
        if have > 0 && !resumed {
            // The server ignored the range: the body is the whole file.
            have = 0;
            hasher = Sha256::new();
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(&part)
            .map_err(|e| format!("Failed to write {name}: {e}"))?;
        let mut writer = std::io::BufWriter::with_capacity(4 << 20, file);
        let mut buffer = vec![0_u8; 1 << 20];
        loop {
            let read = response
                .read(&mut buffer)
                .map_err(|e| format!("Failed while downloading {name}: {e}"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            writer
                .write_all(&buffer[..read])
                .map_err(|e| format!("Failed to write {name}: {e}"))?;
            have += read as u64;
            on_bytes(have);
        }
        writer
            .flush()
            .map_err(|e| format!("Failed to write {name}: {e}"))?;
    }

    if have != size {
        // Short body: keep the partial file, the next attempt resumes it.
        return Err(format!(
            "Parakeet model file {name}: got {have} of {size} bytes; the download resumes on the next attempt"
        ));
    }
    if hex::encode(hasher.finalize()) != expected_sha {
        let _ = std::fs::remove_file(&part);
        return Err(format!(
            "Parakeet model file {name} failed sha256 verification"
        ));
    }
    std::fs::rename(&part, dest).map_err(|e| format!("Failed to store {name}: {e}"))?;
    Ok(())
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
    fn spanish_transcript_with_english_drift_is_detected() {
        // The 1.8.6 output for a Spanish interview (finding 13).
        let drifted = "Los procesos of recuperation of empresas for visibilization in Argentina \
            during the década de los 90. The control obrero of the production, since the \
            empresarial patron burguese, has numerous references historically, and that is \
            what we were looking at with the workers of the factory when they took it.";
        assert!(drifted_from_spanish(drifted));
    }

    #[test]
    fn plain_spanish_transcript_is_not_drift() {
        let spanish = "Hablante A: Los procesos de recuperación de empresas en la Argentina \
            durante la década del noventa fueron muy importantes para los trabajadores. \
            Hablante B: Sí, porque cuando la patronal se fue, ellos tomaron la fábrica y \
            empezaron a producir sin el dueño, con una asamblea que decidía todo.";
        assert!(!drifted_from_spanish(spanish));
        // Spanish with a few English loanwords stays Spanish.
        let loanwords = "El delivery de la empresa y el marketing del producto fueron \
            parte del plan de la cooperativa, que trabajó con los vecinos del barrio \
            para que la fábrica siguiera abierta durante todo el conflicto.";
        assert!(!drifted_from_spanish(loanwords));
        assert!(!drifted_from_spanish(""));
    }

    #[test]
    fn a_resumed_file_needs_room_only_for_what_is_left() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("encoder.onnx.part"), [0_u8; 30]).expect("part");
        let missing = [("encoder.onnx", 100, "sha"), ("decoder.onnx", 50, "sha")];

        assert_eq!(remaining_bytes(dir.path(), &missing), 70 + 50);
    }

    #[test]
    fn partial_download_names_keep_the_full_file_name() {
        let dest = std::path::Path::new("m").join("encoder-model.onnx.data");
        assert_eq!(
            partial_path(&dest),
            std::path::Path::new("m").join("encoder-model.onnx.data.part")
        );
    }

    /// R23: a model downloaded into a runtime's HF cache is adopted, not
    /// downloaded again; a file of the wrong size stays where it was.
    #[test]
    fn legacy_model_files_of_the_right_size_are_adopted() {
        let root = tempfile::tempdir().expect("tempdir");
        let legacy = root
            .path()
            .join("runtime")
            .join("2026.11.4")
            .join("caches")
            .join("hf")
            .join(PARAKEET_MODEL_SUBDIR);
        let target = root.path().join("models").join(PARAKEET_MODEL_SUBDIR);
        std::fs::create_dir_all(legacy.join(DIAR_MODEL_SUBDIR)).expect("legacy dir");
        std::fs::write(legacy.join("a.onnx"), b"1234").expect("file a");
        std::fs::write(legacy.join("b.onnx"), b"12").expect("file b");
        let files: &[(&str, u64, &str)] = &[("a.onnx", 4, ""), ("b.onnx", 3, "")];

        adopt_legacy_model_files(&target, &legacy_parakeet_dirs(root.path()), files);

        assert!(has_size(&target.join("a.onnx"), 4), "finished file moved");
        assert!(!legacy.join("a.onnx").exists());
        assert!(
            legacy.join("b.onnx").exists(),
            "a wrong-size file is not adopted"
        );
        assert!(
            target.join(DIAR_MODEL_SUBDIR).is_dir(),
            "the diarizer folder moves too"
        );
        assert_eq!(missing_model_files(&target, files), vec![("b.onnx", 3, "")]);
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
    fn best_speaker_snaps_words_just_outside_a_span_to_the_nearest_voice() {
        let spans = vec![
            SpeakerSpan {
                start: 0.0,
                end: 5.0,
                speaker: 0,
            },
            SpeakerSpan {
                start: 8.0,
                end: 10.0,
                speaker: 1,
            },
        ];
        // Parakeet's word edges and Sortformer's turn edges disagree by a few
        // hundred ms: a word in that gap still belongs to the nearest voice.
        assert_eq!(best_speaker(5.2, 5.5, &spans), Some(0));
        assert_eq!(best_speaker(7.4, 7.8, &spans), Some(1));
        // A zero-duration word inside a span has no overlap but is inside it.
        assert_eq!(best_speaker(9.0, 9.0, &spans), Some(1));
        // Farther than the snap distance from every span: no speaker.
        assert_eq!(best_speaker(15.0, 16.0, &spans), None);
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
        // The lone "." closes the sentence it follows, whatever speaker
        // Sortformer gave its own timestamp, and takes no leading space.
        assert_eq!(segments[0].text, "hola qué.");
        assert_eq!(segments[0].end, 1.2_f32 as f64);
        assert_eq!(segments[1].text, "tal");
        assert_eq!(segments[1].speaker.as_deref(), Some("Hablante B"));
    }

    #[test]
    fn merge_labeled_attaches_punctuation_without_a_leading_space() {
        let word = |start: f32, text: &str| WordToken {
            start,
            end: start + 0.2,
            text: text.into(),
            owner: 0,
            speaker: Some(0),
        };
        let words = vec![
            word(0.0, "Bueno"),
            word(0.3, ","),
            word(0.6, "la"),
            word(0.9, "productividad"),
            word(1.2, "?"),
            word(1.5, "sí"),
            word(1.8, "."),
        ];
        let segments = merge_labeled(words);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "Bueno, la productividad? sí.");
    }

    #[test]
    fn merge_labeled_drops_punctuation_that_opens_the_transcript() {
        let words = vec![
            WordToken {
                start: 0.0,
                end: 0.1,
                text: ",".into(),
                owner: 0,
                speaker: Some(0),
            },
            WordToken {
                start: 0.2,
                end: 0.5,
                text: "eso".into(),
                owner: 0,
                speaker: Some(0),
            },
        ];
        let segments = merge_labeled(words);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "eso");
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

    /// One 30 s window of the radio program where Parakeet switched to
    /// English (HACIENDO SOCIALES MICRO 2, 30-60 s, lab run 2026-11-22).
    const DRIFTED_WINDOW: &str = "The control obrero de la production, sin the figure of the \
        empresario patron burgués, has numerosas referencias historias. The mayor of the \
        empresas recuperadas by the trabajadores del presente in our país have adopted the \
        form of cooperative.";

    #[test]
    fn only_spanish_turns_the_drift_redo_on() {
        assert!(is_spanish("es"));
        assert!(is_spanish("ES"));
        assert!(is_spanish("es-AR"));
        assert!(!is_spanish("auto"));
        assert!(!is_spanish("en"));
        assert!(!is_spanish(""));
    }

    #[test]
    fn a_single_drifted_window_is_detected() {
        assert!(window_drifted(DRIFTED_WINDOW));
        // A window that starts drifting midway: too few English words for the
        // whole-file check (it needs 8), enough for the window check.
        let starting = "La cooperativa de trabajo presenta el programa sobre las fábricas. \
            The control obrero of the production was la respuesta de los trabajadores.";
        assert!(window_drifted(starting));
        assert!(!drifted_from_spanish(starting));
    }

    #[test]
    fn a_spanish_window_is_not_drift() {
        let interview = "Entrevistador: ¿Y cuándo fue la toma? Entrevistado: No sé, por ahí en \
            el dos mil uno, cuando la patronal se fue y nosotros nos quedamos con las máquinas, \
            porque no había otra.";
        assert!(!window_drifted(interview));
        // A title or a quote in English is not drift.
        let quote = "Ellos leían a Owen, el de New Harmony, y decían que the people united \
            iba a ganar, pero la cooperativa de la fábrica se armó con los vecinos del barrio.";
        assert!(!window_drifted(quote));
        assert!(!window_drifted(""));
    }

    /// Ten seconds of tone, then a silent gap, repeated: silence at
    /// 4.0-4.3 s, 9.0-9.3 s and 14.0-14.3 s.
    fn tone_with_gaps(total_secs: f32) -> Vec<f32> {
        let rate = SAMPLE_RATE as f32;
        (0..(total_secs * rate) as usize)
            .map(|i| {
                let t = i as f32 / rate;
                let in_gap = (t % 5.0) >= 4.0 && (t % 5.0) < 4.3;
                if in_gap {
                    0.0
                } else {
                    (t * 440.0 * std::f32::consts::TAU).sin() * 0.5
                }
            })
            .collect()
    }

    #[test]
    fn pause_cuts_split_inside_the_silences_within_bounds() {
        let samples = tone_with_gaps(17.0);
        let cuts = pause_cuts(&samples, 3.0, 6.0);
        let rate = SAMPLE_RATE as f32;
        assert_eq!(cuts.first().unwrap().0, 0);
        assert_eq!(cuts.last().unwrap().1, samples.len());
        for pair in cuts.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "pieces are contiguous");
        }
        for (index, (start, end)) in cuts.iter().enumerate() {
            let secs = (end - start) as f32 / rate;
            assert!(secs <= 6.0 + 1e-3, "piece {index} is {secs} s");
            if index + 1 < cuts.len() {
                assert!(secs >= 3.0 - 1e-3, "piece {index} is {secs} s");
                let cut = *end as f32 / rate;
                assert!((cut % 5.0) >= 4.0 && (cut % 5.0) <= 4.3, "cut at {cut} s");
            }
        }
    }

    #[test]
    fn pause_cuts_keep_short_audio_whole() {
        let samples = tone_with_gaps(5.0);
        assert_eq!(pause_cuts(&samples, 3.0, 6.0), vec![(0, samples.len())]);
        assert!(pause_cuts(&[], 3.0, 6.0).is_empty());
    }

    /// Stands in for Parakeet: drifts into English on inputs longer than
    /// 10 s, stays Spanish on shorter ones (the behaviour measured on the
    /// radio program). Emits one word per second of input.
    struct FakeDecoder {
        calls: Vec<usize>,
        always_spanish: bool,
    }

    impl WindowDecoder for FakeDecoder {
        fn decode(&mut self, samples: &[f32]) -> Result<Vec<DecodedToken>, String> {
            self.calls.push(samples.len());
            let secs = samples.len() / SAMPLE_RATE as usize;
            let english = !self.always_spanish && secs > 10;
            let words: Vec<&str> = if english {
                DRIFTED_WINDOW.split_whitespace().collect()
            } else {
                "la cooperativa de los trabajadores"
                    .split_whitespace()
                    .collect()
            };
            Ok((0..secs)
                .map(|i| DecodedToken {
                    start: i as f32,
                    end: i as f32 + 0.5,
                    text: words[i % words.len()].to_string(),
                })
                .collect())
        }
    }

    #[test]
    fn only_a_drifted_window_is_decoded_again_in_short_pieces() {
        let samples = tone_with_gaps(33.0);
        let windows = split_windows(samples.len());
        assert_eq!(windows.len(), 2);
        let mut decoder = FakeDecoder {
            calls: Vec::new(),
            always_spanish: false,
        };
        let (tokens, redone) =
            decode_windows(&mut decoder, &samples, &windows, true, |_| {}).expect("decode");
        // Window 0 (30 s) drifts and is redone in pieces; window 1 (6 s) is not.
        assert_eq!(redone, 1);
        let pieces = pause_cuts(
            &samples[windows[0].start_sample..windows[0].end_sample],
            3.0,
            6.0,
        );
        assert_eq!(decoder.calls.len(), 1 + pieces.len() + 1);
        let window_0: Vec<_> = tokens.iter().filter(|t| t.owner == 0).collect();
        assert!(window_0.iter().all(|t| !t.text.contains("the")));
        // Piece tokens carry absolute times: the second piece starts where
        // the first one ends, so its first word sits at that offset.
        let second_piece_start = pieces[1].0 as f32 / SAMPLE_RATE as f32;
        assert!(window_0
            .iter()
            .any(|t| (t.start - second_piece_start).abs() < 1e-3));
        let window_1: Vec<_> = tokens.iter().filter(|t| t.owner == 1).collect();
        assert!(window_1
            .iter()
            .all(|t| t.start >= windows[1].start_secs - 1e-3));
    }

    #[test]
    fn clean_windows_are_decoded_once() {
        let samples = tone_with_gaps(33.0);
        let windows = split_windows(samples.len());
        let mut decoder = FakeDecoder {
            calls: Vec::new(),
            always_spanish: true,
        };
        let (_, redone) =
            decode_windows(&mut decoder, &samples, &windows, true, |_| {}).expect("decode");
        assert_eq!(redone, 0);
        assert_eq!(decoder.calls.len(), windows.len());
    }

    #[test]
    fn drift_is_not_redone_when_the_language_is_not_spanish() {
        let samples = tone_with_gaps(33.0);
        let windows = split_windows(samples.len());
        let mut decoder = FakeDecoder {
            calls: Vec::new(),
            always_spanish: false,
        };
        let (_, redone) =
            decode_windows(&mut decoder, &samples, &windows, false, |_| {}).expect("decode");
        assert_eq!(redone, 0);
        assert_eq!(decoder.calls.len(), windows.len());
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

    /// Live check (ignored by default): Spanish audio that Parakeet used to
    /// translate into English comes out Spanish, with and without speakers.
    /// Archive audio is never committed; point the test at a local wav.
    ///
    /// Run: `PARAKEET_DRIFT_AUDIO=/path/audio.wav
    ///   PARAKEET_BENCH_MODELS=/path/to/models
    ///   ORT_DYLIB_PATH=/path/to/onnxruntime.dll
    ///   cargo test --features local-ml -- --ignored spanish_audio_stays_spanish
    ///   --nocapture`
    #[test]
    #[ignore]
    fn spanish_audio_stays_spanish() {
        let audio = std::env::var("PARAKEET_DRIFT_AUDIO")
            .expect("set PARAKEET_DRIFT_AUDIO to a Spanish wav");
        let models = std::env::var("PARAKEET_BENCH_MODELS")
            .expect("set PARAKEET_BENCH_MODELS to the models dir");
        let nop = |_: u8, _: &str| {};
        let mut engine = ParakeetEngine::init(
            ParakeetConfig {
                model_dir: std::path::PathBuf::from(&models).join(PARAKEET_MODEL_SUBDIR),
                language: "es".to_string(),
            },
            nop,
        )
        .expect("load parakeet");
        for diarize in [false, true] {
            let (result, _) = engine.transcribe(&audio, diarize, nop).expect("transcribe");
            let (english, spanish) = function_word_counts(&result.text);
            let share = english as f32 / (english + spanish).max(1) as f32;
            let speakers = result
                .segments
                .iter()
                .filter_map(|s| s.speaker.clone())
                .collect::<std::collections::HashSet<_>>()
                .len();
            println!(
                "DRIFT diarize={diarize} english={english} spanish={spanish} share={share:.3} speakers={speakers} text={:.120}",
                result.text.replace('\n', " | ")
            );
            assert!(share < 0.10, "English share {share:.3} on Spanish audio");
            if diarize {
                assert!(speakers >= 1, "speaker labels lost");
            }
        }
    }

    /// Live check (ignored by default) against the real Sortformer model.
    /// parakeet-rs 0.3.8 built the mel features column-major, and ONNX
    /// Runtime refused them ("Array has a non-contiguous layout") whenever
    /// the whole clip fit in one chunk with a frame count that is a multiple
    /// of 8. 0.4.4 hands them over in standard layout; this pins that fix.
    ///
    /// Run: `PARAKEET_DIAR_MODEL_DIR=<models>/parakeet-tdt-0.6b-v3/nemotron-3-diarization
    ///   ORT_DYLIB_PATH=/path/to/onnxruntime.dll
    ///   cargo test --features local-ml -- --ignored sortformer_diarizes_single_chunk_clips`
    #[test]
    #[ignore]
    fn sortformer_diarizes_single_chunk_clips() {
        let dir = std::env::var("PARAKEET_DIAR_MODEL_DIR")
            .expect("set PARAKEET_DIAR_MODEL_DIR to the nemotron-3-diarization dir");
        let dir = std::path::Path::new(&dir);
        let mut diarizer = Sortformer::with_config(
            dir.join("nemotron3_diar_v3.onnx"),
            Some(ort_config(dir).expect("ONNX Runtime environment")),
            DiarizationConfig::default(),
        )
        .expect("load diarization model");
        // Frames = samples / 160 + 1: 2400 frames is a multiple of 8, 2401 is not.
        for frames in [2400usize, 2401] {
            let samples: Vec<f32> = (0..(frames - 1) * 160)
                .map(|i| 0.1 * (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / 16_000.0).sin())
                .collect();
            diarizer
                .diarize(samples.clone(), SAMPLE_RATE, 1)
                .unwrap_or_else(|e| panic!("diarize failed at {frames} frames: {e}"));
        }
    }
}
