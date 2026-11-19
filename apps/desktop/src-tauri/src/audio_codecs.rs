//! The audio decoders for every place that reads a user's audio file: the
//! transcription engines and the audio preview.
//!
//! Symphonia has no Opus decoder, and Opus is what WhatsApp and Telegram voice
//! notes carry (`.ogg`/`.opus`): with Symphonia's registry alone, Parakeet
//! refused them ("unsupported audio codec"), the transcription fell back to
//! faster-whisper and lost the speaker labels, and the preview could not play
//! them. libopus, compiled into the binary, fills that gap.

use std::sync::LazyLock;

use symphonia::core::codecs::registry::CodecRegistry;

static CODECS: LazyLock<CodecRegistry> = LazyLock::new(|| {
    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    registry
});

/// Symphonia's own decoders plus Opus. Use this instead of
/// `symphonia::default::get_codecs()`.
pub(crate) fn codecs() -> &'static CodecRegistry {
    &CODECS
}
