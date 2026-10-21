use crate::path_utils::normalize_windows_path_string;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use symphonia::core::codecs::audio::{AudioDecoderOptions, CODEC_ID_NULL_AUDIO};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

#[tauri::command]
pub async fn prepare_audio_preview(
    asset_path: String,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let asset_path = crate::path_utils::resolve_asset_path_at_boundary(&asset_path, &app_handle)?;
    let app_dir = crate::path_utils::cache_dir(&app_handle)?;
    let preview_dir = app_dir.join("audio-previews");

    tokio::task::spawn_blocking(move || prepare_audio_preview_file(&asset_path, &preview_dir))
        .await
        .map_err(|e| format!("Audio preview task panicked: {e}"))?
}

fn prepare_audio_preview_file(asset_path: &str, preview_dir: &Path) -> Result<String, String> {
    let source = PathBuf::from(asset_path);
    let metadata = std::fs::metadata(&source)
        .map_err(|e| format!("Audio preview source is not readable: {e}"))?;
    if !metadata.is_file() {
        return Err("Audio preview source is not a file".to_string());
    }

    std::fs::create_dir_all(preview_dir)
        .map_err(|e| format!("Failed to create audio preview directory: {e}"))?;

    let preview_path = preview_dir.join(preview_filename(&source, &metadata));
    if preview_path.exists() {
        return Ok(normalize_windows_path_string(&preview_path));
    }

    let temp_path = preview_path.with_extension("tmp");
    decode_to_pcm16_wav(&source, &temp_path)
        .map_err(|e| format!("Audio preview decode failed: {e}"))?;
    std::fs::rename(&temp_path, &preview_path)
        .map_err(|e| format!("Failed to finalize audio preview file: {e}"))?;

    Ok(normalize_windows_path_string(&preview_path))
}

fn preview_filename(source: &Path, metadata: &std::fs::Metadata) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_windows_path_string(source));
    hasher.update(metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
            hasher.update(duration.as_nanos().to_le_bytes());
        }
    }
    format!("{}.wav", hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_to_pcm16_wav(source: &Path, output: &Path) -> Result<(), String> {
    let file = File::open(source).map_err(|e| format!("Failed to open source audio: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(extension) = source.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(extension);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("Unsupported or unreadable audio container: {e}"))?;

    let (track_id, codec_params) = format
        .tracks()
        .iter()
        .find_map(|track| {
            let params = track.codec_params.as_ref()?.audio()?;
            (params.codec != CODEC_ID_NULL_AUDIO).then(|| (track.id, params.clone()))
        })
        .ok_or_else(|| "No decodable audio track found".to_string())?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&codec_params, &AudioDecoderOptions::default())
        .map_err(|e| format!("Unsupported audio codec: {e}"))?;

    let mut writer: Option<hound::WavWriter<std::io::BufWriter<File>>> = None;
    let mut wrote_samples = false;
    let mut samples: Vec<i16> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            // End of stream. An I/O error (e.g. a truncated file) also ends the
            // preview with whatever decoded so far, as before.
            Ok(None) | Err(SymphoniaError::IoError(_)) => break,
            Err(error) => return Err(format!("Failed to read audio packet: {error}")),
        };

        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(format!("Failed to decode audio packet: {error}")),
        };

        if writer.is_none() {
            let spec = decoded.spec();
            let wav_spec = hound::WavSpec {
                channels: spec.channels().count() as u16,
                sample_rate: spec.rate(),
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            writer = Some(
                hound::WavWriter::create(output, wav_spec)
                    .map_err(|e| format!("Failed to create preview WAV: {e}"))?,
            );
        }

        decoded.copy_to_vec_interleaved(&mut samples);

        let writer = writer
            .as_mut()
            .ok_or_else(|| "Preview WAV writer was not initialized".to_string())?;
        for sample in &samples {
            writer
                .write_sample(*sample)
                .map_err(|e| format!("Failed to write preview sample: {e}"))?;
            wrote_samples = true;
        }
    }

    let writer = writer.ok_or_else(|| "No decoded audio frames found".to_string())?;
    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize preview WAV: {e}"))?;

    if !wrote_samples {
        let _ = std::fs::remove_file(output);
        return Err("Decoded audio contained no samples".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_filename_is_wav_and_stable_for_same_file_state() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("tone.wav");
        std::fs::write(&source, b"not-a-real-wav").unwrap();
        let metadata = std::fs::metadata(&source).unwrap();

        let first = preview_filename(&source, &metadata);
        let second = preview_filename(&source, &metadata);

        assert_eq!(first, second);
        assert!(first.ends_with(".wav"));
        assert_eq!(first.len(), 68);
    }

    #[test]
    fn prepares_pcm_wav_preview_from_generated_wav() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("tone.wav");
        write_test_wav(&source);

        let preview_path =
            prepare_audio_preview_file(source.to_str().unwrap(), dir.path()).unwrap();
        let preview_path = PathBuf::from(preview_path);

        assert_eq!(
            preview_path.extension().and_then(|ext| ext.to_str()),
            Some("wav")
        );
        assert!(preview_path.exists());

        let mut reader = hound::WavReader::open(preview_path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 8_000);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);

        let samples: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
        assert_eq!(samples.len(), 80, "every source sample is kept");
        assert!(
            samples
                .iter()
                .enumerate()
                .all(|(index, sample)| *sample == test_sample(index)),
            "16-bit PCM passes through unchanged"
        );
    }

    fn test_sample(index: usize) -> i16 {
        if index.is_multiple_of(2) {
            i16::MAX / 4
        } else {
            i16::MIN / 4
        }
    }

    fn write_test_wav(path: &Path) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for index in 0..80 {
            writer.write_sample(test_sample(index)).unwrap();
        }
        writer.finalize().unwrap();
    }
}
