/// Audio decoding and resampling for Whisper transcription.
///
/// Supports WAV (via hound), MP3, FLAC, M4A/AAC (via symphonia).
/// All audio is converted to 16kHz mono f32 PCM for Whisper-compatible input.
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
#[allow(unused_imports)]
use symphonia::core::units::Time;

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
/// Supports: WAV, MP3, FLAC, M4A/AAC, OGG.
/// Returns an error for unsupported formats.
pub fn decode_audio_file(path: &str) -> Result<AudioData, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "wav" => decode_wav(path),
        "mp3" | "flac" | "m4a" | "aac" | "ogg" | "wma" => decode_symphonia(path),
        _ => Err(format!(
            "Unsupported audio format: .{ext}. Supported: wav, mp3, flac, m4a, aac, ogg"
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

    let format_opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let metadata_opts = MetadataOptions::default();
    let decoder_opts = DecoderOptions {
        verify: false,
        ..Default::default()
    };

    // Probe the format
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .map_err(|e| format!("Failed to probe audio format: {e}"))?;

    let mut format_reader = probed.format;

    // Find the default audio track
    let track = format_reader
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("No audio track found in file")?;

    let codec_params = track.codec_params.clone();
    let track_id = track.id;

    // Get original sample rate and channels from codec params
    let original_sample_rate = codec_params.sample_rate.unwrap_or(44100) as usize;
    let num_channels = codec_params.channels.map(|c| c.count()).unwrap_or(1) as u16;

    // Create decoder
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &decoder_opts)
        .map_err(|e| format!("Failed to create audio decoder: {e}"))?;

    // Decode all frames and collect samples using SampleBuffer
    let mut all_samples: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format_reader.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => {
                log::warn!(target: "transcription", "Symphonia decode loop ended: {e}");
                break;
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(e) => {
                log::info!(target: "transcription", "Failed to decode packet: {e}");
                continue;
            }
        };

        // Use SampleBuffer to convert AudioBufferRef to interleaved f32 samples
        let spec = *decoded.spec();
        let frames = decoded.capacity();

        // Recreate sample buffer if needed (first decode or different capacity)
        let needs_resize = match sample_buf {
            None => true,
            Some(_) => false, // Keep same buffer — SampleBuffer::copy_interleaved_ref handles resizing
        };

        if needs_resize {
            sample_buf = Some(SampleBuffer::<f32>::new(frames as u64, spec));
        }

        if let Some(ref mut buf) = sample_buf {
            buf.copy_interleaved_ref(decoded);
            all_samples.extend_from_slice(buf.samples());
        }
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
