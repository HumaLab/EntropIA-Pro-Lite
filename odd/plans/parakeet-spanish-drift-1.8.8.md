# Parakeet Spanish drift: findings and fix design (1.8.8)

Status: Change 1 implemented on `fix/parakeet-spanish-drift`. Investigated
2026-11-22 outside EntropIA.
Replaces the 1.8.7 whole-file fallback (pendientes item 34, finding 13).

## What runs today

- Model: NVIDIA Parakeet TDT 0.6B v3, ONNX export from Hugging Face
  `altunenes/parakeet-rs` at revision `664cc742…`, fp32 encoder, run through the
  `parakeet-rs` 0.4.4 crate (`transcription/parakeet.rs`).
- Audio is split into fixed 30 s windows with a 3 s overlap (`WINDOW_SECS`).
- The language is hardcoded to `es` (`mod.rs`, `TRANSCRIPTION_LANGUAGE`), but
  Parakeet takes no language parameter, so `es` is only a label.
- 1.8.7 checks the whole transcript with `drifted_from_spanish` and, on drift,
  re-transcribes the **whole file** with faster-whisper `base`. Speaker labels
  are lost, and the Parakeet run is wasted.

## Lab setup (outside EntropIA)

- The same ONNX files (hardlinked), decoded with `onnx-asr` 0.x on
  onnxruntime 1.32 CPU. That is an independent Python implementation, not
  `parakeet-rs`.
- faster-whisper `base` int8 through the app's own `transcribe.py` and runtime.
- Word error rate (WER) against the stored AssemblyAI transcript, after
  lowercasing, stripping accents and removing speaker labels.
- Audio from the maintainer's archive:
  - Radio narration: `HACIENDO SOCIALES MICRO 1` and `MICRO 2`.
  - Field interviews: `SOIP1`, `SOIP4`, `SOIP5` and `Alcira`.
- The lab scripts are in the session scratchpad and are not committed.

## Findings

1. **The drift is in the model, not in EntropIA.** `onnx-asr` reproduces it
   with the same files: `The control obrero de la production, sin the figure of
   the empresario…`. `parakeet-rs` preprocessing is not the cause.
2. **The decoder cannot be steered.** The vocabulary carries Canary-style
   language tokens (`<|es|>` = 171, `<|en|>` = 64), but priming the
   prediction network with `<|es|>` or with Spanish words changes nothing (9/12
   windows drift either way). A logit penalty on English function words only
   hides the symptom: the content words stay translated. The language decision
   happens in the encoder.
3. **The drift depends on window length.** English share of function words on
   MICRO 2:

   | Segmenting | 30 s fixed | 8–15 s pause | 4–8 s pause | 3–6 s pause | 2–4 s pause |
   |---|---|---|---|---|---|
   | en_ratio | 0.75 | 0.23 | 0.13 | 0.02 | 0.00 |

   Pause segments are cut at the quietest 20 ms frame inside the range.
4. **The drift is content-dependent and does not hit interviews.** On the four
   interviews, 30 s windows give en_ratio 0.00, with no false triggers at any
   threshold tried.
5. **Do not route Spanish away from Parakeet.** On interviews, Parakeet is far
   better than faster-whisper `base` (WER):

   | Audio | Parakeet 30 s | Parakeet 3–6 s | faster-whisper base |
   |---|---|---|---|
   | SOIP1 (18 min) | **0.188** | 0.254 | 0.804 |
   | SOIP4 (12 min) | **0.185** | 0.257 | 0.461 |
   | SOIP5 (11 min) | **0.277** | 0.306 | 0.613 |
   | Alcira (35 min) | **0.288** | 0.363 | 0.547 |
   | MICRO 1 (radio, 4 min) | 0.541 | 0.050 | **0.022** |

   Short segments everywhere would also hurt interviews (they lose context), so
   short segments are only for the windows that drift.
6. **The hybrid fixes radio and leaves interviews untouched.** The hybrid keeps
   30 s windows, checks each window for drift, and re-decodes only the
   drifted windows as 3–6 s pause segments:

   | Audio | Windows redone | WER before | WER hybrid |
   |---|---|---|---|
   | MICRO 1 | 7/9 | 0.541 | **0.048** |
   | MICRO 2 | 11/13 | 0.691 (vs fw) | **0.157** (vs fw) |
   | SOIP4 | 0/26 | 0.185 | 0.185 |
   | SOIP5 | 0/24 | 0.277 | 0.277 |

   The result is stable across thresholds of 2 to 4 English function words and
   a 0.15 to 0.30 share. Residual errors on MICRO 2 are short leftovers
   (`the mayor of the`) and numbers written as words.

7. **Other models were tested and none replaces Parakeet today** (WER, same
   references; MICRO 2 against faster-whisper):

   | Option | MICRO 1 | MICRO 2 | SOIP4 | SOIP5 |
   |---|---|---|---|---|
   | Parakeet + per-window redo (this fix) | 0.048 | 0.157 | 0.185 | 0.277 |
   | Parakeet, 1.8.7 windows | 0.541 | 0.691 | 0.185 | 0.277 |
   | Nemotron Unified, offline, `es` | 0.477 | 0.498 | 0.176 | 0.218 |
   | Nemotron Unified, streaming, `es` | 0.154 | 0.239 | 0.188 | 0.276 |
   | Nemotron 3.5 streaming, `es-ES` | 0.064 | 0.154 | 0.277 | 0.331 |
   | MOSS-Transcribe-Diarize 0.9B fp32 | 0.014 | — | 0.152 | — |
   | faster-whisper `base` | 0.022 | — | 0.461 | 0.613 |

   - Nemotron Unified takes a target language but still drifts on radio in
     offline mode. It is the best on SOIP5, so "Unified offline + the same
     per-window redo" is the one untested combination worth a look. It needs
     a new ~2.5 GB download.
   - Nemotron 3.5 streaming fixes radio but is weak on interviews.
   - MOSS has the best quality and finds both speakers on SOIP4, but fp32 on
     CPU took 666 s for 4 min of audio and 5350 s for 12 min. Its SOIP4 gain
     over Parakeet (0.152 vs 0.185) did not justify testing the quantized
     builds (ONNX INT8 ~1.6 GB, GGUF via `moss-transcribe.cpp`). They stay a
     candidate for a future engine.

## Fix design

### Change 1: per-window drift check with re-decode (`parakeet.rs`)

In `ParakeetEngine::transcribe`, after decoding window *i*:

1. Join the window's token texts and run a **window-sized** drift check,
   `window_drifted(text)`: at least 3 English function words and an English
   share of at least 0.30. It reuses the `ENGLISH`/`SPANISH` lists of
   `drifted_from_spanish`, which needs `english >= 8` and so is built for a
   whole file, not for one 30 s window.
2. When the window drifts and the language is `es`, discard its tokens. Split
   the same sample range with `pause_cuts(samples, 3.0, 6.0)` (the quietest
   20 ms RMS frame in each span) and decode each piece. Offset each piece's
   token times by `window.start_secs + piece.start_secs`, and keep
   `owner = i`. The midpoint ownership in `merge_windows`/`assign_speakers`
   then works unchanged.
3. Keep a counter of redone windows and log it:
   `Parakeet re-decoded N of M windows in short segments (English drift)`.

Speaker labels survive, because the tokens keep real timestamps and
`assign_speakers` runs on the full audio as before.

### Change 2: keep the 1.8.7 whole-file check as a safety net (`mod.rs`)

`drifted_from_spanish(&result.text)` still runs on the final text. With
Change 1 it should rarely fire. When it does, the faster-whisper fallback and
its warning stay as they are. Nothing else changes in `mod.rs`.

### Seams and tests

- Extract the window loop behind a small trait, for example
  `trait WindowDecoder { fn decode(&mut self, samples: &[f32]) -> Result<Vec<Token>, String>; }`,
  so the re-decode logic can be tested with a fake that returns English for
  long inputs and Spanish for short ones. No model is needed in CI.
- Unit tests, written RED first:
  - A drifted window is re-decoded in pieces. Its tokens are offset correctly
    and keep the window as owner.
  - A clean window is decoded exactly once, so interviews pay nothing extra.
  - `pause_cuts` never makes a piece longer than 6 s or shorter than 3 s
    (except the tail), and it cuts at the quietest frame (synthetic signal with
    a silent gap).
  - `window_drifted` fires on the MICRO 2 sample text and not on the SOIP
    sample text (short excerpts, written in the test).
  - The existing `drifted_from_spanish` tests and the drift fallback warning
    test still pass.
- An ignored local test, `spanish_audio_stays_spanish` (runs only when
  `PARAKEET_DRIFT_AUDIO` points to a WAV, with `PARAKEET_BENCH_MODELS` and
  `ORT_DYLIB_PATH`), loads the real model, transcribes with and without
  speakers, and asserts an English share of function words below 0.10.
  Archive audio is never committed.

### Cost

A re-decoded window costs about one more pass over its 30 s (MICRO 2: 32 s,
about +28 s). Clean audio costs nothing extra. The 1.8.7 path cost a full
Parakeet run plus a full faster-whisper run.

### Out of scope

- Real language selection (`TRANSCRIPTION_LANGUAGE` stays `es`).
- Replacing faster-whisper `base` with a larger model for the fallback.
- Canary-1B-v2, which takes a source-language token: a possible future
  engine, not needed for this fix. It needs a new download (~1–4 GB) and was
  not tested.

## Risk

Risk: item 6. The model's behaviour is not covered by CI; the ignored local
test and a check in the real app on MICRO 1/2 and one SOIP interview cover it.
