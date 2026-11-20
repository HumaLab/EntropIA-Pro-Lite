/// Audio decoding and resampling for Whisper transcription.
///
/// Supports WAV (PCM via hound, compressed codecs such as Microsoft ADPCM via
/// symphonia), MP3, FLAC, M4A/AAC, Ogg Vorbis (via symphonia) and
/// Opus (via libopus, see `crate::audio_codecs`).
/// All audio is converted to 16kHz mono f32 PCM for Whisper-compatible input.
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Processed audio data ready for Whisper inference.
#[derive(Debug)]
pub struct AudioData {
    /// Mono 16kHz f32 PCM samples — the format Whisper expects.
    pub samples: Vec<f32>,
    /// Duration of the audio in milliseconds.
    pub duration_ms: u64,
    /// Sample rate of the *original* audio file (before resampling).
    #[allow(dead_code)]
    pub original_sample_rate: u32,
    /// Number of channels in the original file.
    #[allow(dead_code)]
    pub channels: u16,
}

/// Target sample rate for Whisper (16kHz mono).
const WHISPER_SAMPLE_RATE: usize = 16_000;

/// Decode an audio file to 16kHz mono f32 PCM samples suitable for Whisper.
///
/// Supports: WAV, MP3, FLAC, M4A/AAC, OGG (Vorbis or Opus), OPUS.
/// Returns an error for unsupported formats.
pub fn decode_audio_file(path: &str) -> Result<AudioData, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        // hound reads only uncompressed 16-bit PCM here; a WAV holding a
        // compressed codec (Microsoft ADPCM, A-law…) or other sample formats
        // goes through symphonia instead of failing the whole transcription.
        "wav" => decode_wav(path).or_else(|pcm_error| {
            decode_symphonia(path).map_err(|error| format!("{pcm_error}; {error}"))
        }),
        "mp3" | "flac" | "m4a" | "aac" | "ogg" | "opus" | "wma" => decode_symphonia(path),
        _ => Err(format!(
            "Unsupported audio format: .{ext}. Supported: wav, mp3, flac, m4a, aac, ogg, opus"
        )),
    }
}

/// Decode WAV files using the `hound` crate.
fn decode_wav(path: &str) -> Result<AudioData, String> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| format!("Failed to open WAV file: {e}"))?;
    let spec = reader.spec();
    let num_channels = spec.channels;
    let original_sample_rate = spec.sample_rate;

    // Decode all samples as i16 (most common)
    let samples_i16: Vec<i16> = reader
        .samples()
        .collect::<Result<Vec<i16>, hound::Error>>()
        .map_err(|e| format!("Failed to decode WAV samples: {e}"))?;

    // Convert to f32 and normalize to [-1.0, 1.0]
    let samples_f32: Vec<f32> = samples_i16
        .iter()
        .map(|&s| s as f32 / i16::MAX as f32)
        .collect();

    // Mix down to mono if stereo
    let mono = if num_channels > 1 {
        mix_to_mono(&samples_f32, num_channels as usize)
    } else {
        samples_f32
    };

    // Resample to 16kHz if needed
    let resampled = resample(&mono, original_sample_rate as usize, WHISPER_SAMPLE_RATE);

    let duration_ms = (mono.len() as f64 / original_sample_rate as f64 * 1000.0) as u64;

    Ok(AudioData {
        samples: resampled,
        duration_ms,
        original_sample_rate,
        channels: num_channels,
    })
}

/// Decode audio files using the `symphonia` crate (MP3, FLAC, M4A, etc.).
fn decode_symphonia(path: &str) -> Result<AudioData, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Failed to open audio file: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    hint.with_extension(ext);

    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();
    let decoder_opts = AudioDecoderOptions::default();

    // Probe the format
    let mut format_reader = symphonia::default::get_probe()
        .probe(&hint, mss, format_opts, metadata_opts)
        .map_err(|e| format!("Failed to probe audio format: {e}"))?;

    // Find the default audio track
    let track = format_reader
        .default_track(TrackType::Audio)
        .ok_or("No audio track found in file")?;
    let track_id = track.id;
    let codec_params = track
        .codec_params
        .clone()
        .ok_or("Track has no codec parameters")?;

    // Audio-specific parameters (sample rate, channels)
    let audio_params = codec_params
        .audio()
        .ok_or("Track has no audio codec parameters")?;

    // Get original sample rate and channels from codec params
    let original_sample_rate = audio_params.sample_rate.unwrap_or(44100) as usize;
    let num_channels = audio_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(1) as u16;
    let mut decoder = crate::audio_codecs::codecs()
        .make_audio_decoder(audio_params, &decoder_opts)
        .map_err(|e| format!("Failed to create audio decoder: {e}"))?;

    // Decode all packets, converting each buffer to interleaved f32
    let mut all_samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format_reader.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => {
                log::warn!(target: "transcription", "Symphonia decode loop ended: {e}");
                break;
            }
        };

        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(e) => {
                log::info!(target: "transcription", "Failed to decode packet: {e}");
                continue;
            }
        };

        let mut interleaved = vec![0.0f32; decoded.samples_interleaved()];
        decoded.copy_to_slice_interleaved(&mut interleaved);
        all_samples.extend_from_slice(&interleaved);
    }

    // Mix down to mono if needed
    let mono = if num_channels > 1 {
        mix_to_mono(&all_samples, num_channels as usize)
    } else {
        all_samples
    };

    // Resample to 16kHz if needed
    let resampled = resample(&mono, original_sample_rate, WHISPER_SAMPLE_RATE);

    let duration_ms = (mono.len() as f64 / original_sample_rate as f64 * 1000.0) as u64;

    Ok(AudioData {
        samples: resampled,
        duration_ms,
        original_sample_rate: original_sample_rate as u32,
        channels: num_channels,
    })
}

/// Mix interleaved multi-channel audio down to mono by averaging channels.
fn mix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let num_frames = interleaved.len() / channels;
    let mut mono = Vec::with_capacity(num_frames);
    for i in 0..num_frames {
        let offset = i * channels;
        let sum: f32 = (0..channels).map(|c| interleaved[offset + c]).sum();
        mono.push(sum / channels as f32);
    }
    mono
}

/// Resample audio from `from_rate` to `to_rate` using linear interpolation.
///
/// If `from_rate == to_rate`, returns the input unchanged.
fn resample(samples: &[f32], from_rate: usize, to_rate: usize) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }

    // Use linear interpolation for resampling — good enough for speech recognition
    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = ((samples.len() as f64) / ratio).ceil() as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        if idx + 1 < samples.len() {
            let frac = pos - idx as f64;
            output.push(samples[idx] * (1.0 - frac as f32) + samples[idx + 1] * frac as f32);
        } else if idx < samples.len() {
            output.push(samples[idx]);
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One second of a 440 Hz tone, encoded like a WhatsApp voice note: Ogg
    /// container, Opus codec, mono, 48 kHz (ffmpeg libopus, 16 kbit/s, voip).
    const OPUS_VOICE_NOTE: &[u8] = include_bytes!("testdata/tone-opus.ogg");

    fn decode_fixture(extension: &str) -> Result<AudioData, String> {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(format!("voice-note.{extension}"));
        std::fs::write(&path, OPUS_VOICE_NOTE).expect("write fixture");
        decode_audio_file(path.to_str().expect("utf-8 path"))
    }

    fn assert_is_the_one_second_tone(audio: &AudioData) {
        assert!(
            (900..=1100).contains(&audio.duration_ms),
            "duration {} ms",
            audio.duration_ms
        );
        assert!(
            (14_400..=17_600).contains(&audio.samples.len()),
            "{} samples at 16 kHz",
            audio.samples.len()
        );
        let rms =
            (audio.samples.iter().map(|s| s * s).sum::<f32>() / audio.samples.len() as f32).sqrt();
        assert!(rms > 0.05, "decoded signal is silent (rms {rms})");
    }

    #[test]
    fn decodes_an_opus_voice_note_in_ogg() {
        let audio = decode_fixture("ogg").expect("Opus in Ogg must decode");
        assert_is_the_one_second_tone(&audio);
    }

    #[test]
    fn decodes_an_opus_file_with_the_opus_extension() {
        let audio = decode_fixture("opus").expect(".opus must decode");
        assert_is_the_one_second_tone(&audio);
    }

    /// One second of a 440 Hz tone in a WAV file compressed with Microsoft
    /// ADPCM, mono, 8 kHz (ffmpeg `adpcm_ms`), like old dictaphone recordings.
    const ADPCM_WAV: &[u8] = include_bytes!("testdata/tone-adpcm-ms.wav");

    #[test]
    fn decodes_a_wav_compressed_with_microsoft_adpcm() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("recording.wav");
        std::fs::write(&path, ADPCM_WAV).expect("write fixture");
        let audio =
            decode_audio_file(path.to_str().expect("utf-8 path")).expect("ADPCM WAV must decode");
        assert_is_the_one_second_tone(&audio);
    }

    #[test]
    fn still_decodes_a_plain_16_bit_pcm_wav() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("plain.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).expect("create wav");
        for n in 0..16_000 {
            let phase = n as f32 * 440.0 * std::f32::consts::TAU / 16_000.0;
            writer
                .write_sample((phase.sin() * 0.5 * i16::MAX as f32) as i16)
                .expect("write sample");
        }
        writer.finalize().expect("finalize wav");

        let audio = decode_audio_file(path.to_str().expect("utf-8 path")).expect("PCM WAV");
        assert_is_the_one_second_tone(&audio);
        assert_eq!(audio.original_sample_rate, 16_000);
    }

    #[test]
    fn test_mix_to_mono_stereo() {
        let interleaved = vec![1.0, 0.5, 0.0, -0.5, -1.0, 0.0];
        let mono = mix_to_mono(&interleaved, 2);
        assert_eq!(mono.len(), 3);
        assert!((mono[0] - 0.75).abs() < 0.001);
        assert!((mono[1] - (-0.25)).abs() < 0.001);
        assert!((mono[2] - (-0.5)).abs() < 0.001);
    }

    #[test]
    fn test_mix_to_mono_passthrough() {
        let mono = vec![0.5, 0.3, -0.2];
        let result = mix_to_mono(&mono, 1);
        assert_eq!(result, mono);
    }

    #[test]
    fn test_resample_identity() {
        let samples = vec![0.5; 16000];
        let resampled = resample(&samples, 16000, 16000);
        assert_eq!(resampled.len(), 16000);
    }

    #[test]
    fn test_linear_resample_basic() {
        let samples = vec![0.0, 1.0, 0.0, -1.0, 0.0];
        let resampled = resample(&samples, 8000, 16000);
        assert!(resampled.len() >= samples.len() * 2 - 1);
    }

    #[test]
    fn test_decode_audio_unsupported_format() {
        let result = decode_audio_file("test.xyz");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported audio format"));
    }
}
