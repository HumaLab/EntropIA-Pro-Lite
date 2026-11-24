//! Gemma on the GPU for the interactive local flows (RAG answer, intent,
//! query rewrite, help). The managed `llama-server` GLM-OCR uses runs the
//! model; whenever it cannot, the caller generates on CPU in-process as
//! before, so a fallback is never an error for the user.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ocr::local_server::{self, ServerAssets, ServerConfig, ServerKind};

/// The Gemma server's window: a ~12K-token evidence prompt plus its answer
/// budget. Measured on a 4 GB RX 560 at 16K: 1.4 GB of weights, 108 MB of KV
/// and 545 MB of compute on the GPU, so 32K leaves room on a 4 GB card.
pub(crate) const GEMMA_SERVER_CTX: u32 = 32_768;

/// Tokens kept free past the answer, as the in-process engine's `fit_n_ctx`.
const TAIL_RESERVE: usize = 256;

/// Set once the GPU server fails to start: every later request would pay the
/// startup timeout again before falling back, so the session stays on CPU.
static START_FAILED: AtomicBool = AtomicBool::new(false);

const CONFIG: ServerConfig = ServerConfig {
    kind: ServerKind::Text,
    slots: 1,
    ctx_per_slot: GEMMA_SERVER_CTX,
    log_target: "llm-local",
    log_file: "gemma-server.log",
};

/// Why a request runs on CPU instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fallback {
    /// No GPU llama.cpp can use here (or not enough video memory).
    NoGpu,
    /// The managed `llama-server` is not installed yet.
    NoServer,
    /// A model other than the bundled Gemma: its prompt format on CPU may
    /// come from its own template, which the server would not reproduce.
    CustomModel,
    /// OCR or correction holds the GPU server.
    Busy,
    /// The prompt does not fit the server's window.
    TooLong { prompt_tokens: usize },
    /// The server failed to start or to answer.
    Failed(String),
    /// The server failed to start earlier in this session.
    StartFailedBefore,
}

impl std::fmt::Display for Fallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoGpu => write!(f, "no hay una GPU utilizable"),
            Self::NoServer => write!(f, "llama-server no está instalado"),
            Self::CustomModel => write!(f, "modelo local personalizado"),
            Self::Busy => write!(f, "la GPU está ocupada con OCR"),
            Self::TooLong { prompt_tokens } => write!(
                f,
                "el pedido de {prompt_tokens} tokens no entra en la ventana de la GPU ({GEMMA_SERVER_CTX})"
            ),
            Self::Failed(error) => write!(f, "falló el servidor de la GPU: {error}"),
            Self::StartFailedBefore => {
                write!(f, "el servidor de la GPU no arrancó antes en esta sesión")
            }
        }
    }
}

/// The answer budget for a prompt of `prompt_tokens` in a window of `ctx`:
/// `max_tokens`, cut to the space left as the CPU engine does, or `None` when
/// the prompt leaves no room.
pub(crate) fn answer_budget(prompt_tokens: usize, max_tokens: i32, ctx: u32) -> Option<i32> {
    let room = (ctx as usize).checked_sub(prompt_tokens + TAIL_RESERVE)?;
    if room == 0 {
        return None;
    }
    Some(max_tokens.min(i32::try_from(room).unwrap_or(i32::MAX)))
}

/// The window a request gets on the server: the caller's own `n_ctx`, so the
/// answer is cut exactly as on CPU, within the server's.
pub(crate) fn request_window(n_ctx: u32) -> u32 {
    n_ctx.min(GEMMA_SERVER_CTX)
}

/// Generate `instruction` (raw text, wrapped here in the Gemma turn format as
/// the CPU path does) on the GPU server, or say why not.
pub(crate) fn generate(
    app_handle: &tauri::AppHandle,
    db_path: &Path,
    instruction: &str,
    max_tokens: i32,
    n_ctx: u32,
    temperature: f32,
    seed: u32,
) -> Result<String, Fallback> {
    use tauri::Manager;

    // The CPU engine refuses these; let it, with its own message.
    if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
        return Err(Fallback::Failed(format!(
            "temperatura inválida: {temperature}"
        )));
    }
    if crate::ocr::llama_gpu::detected().is_none() {
        return Err(Fallback::NoGpu);
    }
    if START_FAILED.load(Ordering::Relaxed) {
        return Err(Fallback::StartFailedBefore);
    }
    let server = local_server::resolve_server(
        db_path,
        local_server::env_server_override(),
        app_handle.path().resource_dir().ok().as_deref(),
    );
    if !local_server::server_ready(&server, db_path) {
        return Err(Fallback::NoServer);
    }
    let model = super::resolve_model_path(db_path);
    if model.file_name().and_then(|name| name.to_str()) != Some(super::MODEL_FILENAME)
        || !model.is_file()
    {
        return Err(Fallback::CustomModel);
    }
    let assets = ServerAssets {
        server,
        model,
        mmproj: None,
    };
    // Held to the end: OCR does not stop the server under this request.
    let lease = match local_server::ensure_server_if_free(&CONFIG, &assets, db_path) {
        Ok(Some(lease)) => lease,
        Ok(None) => return Err(Fallback::Busy),
        Err(error) => {
            START_FAILED.store(true, Ordering::Relaxed);
            return Err(Fallback::Failed(error));
        }
    };
    let base_url = lease.base_url();

    let prompt = super::prompt::gemma_wrap(instruction);
    let prompt_tokens =
        super::gemma_server::token_count(base_url, &prompt).map_err(Fallback::Failed)?;
    let budget = answer_budget(prompt_tokens, max_tokens, request_window(n_ctx))
        .ok_or(Fallback::TooLong { prompt_tokens })?;
    let output = super::gemma_server::complete(
        base_url,
        &super::gemma_server::Completion {
            prompt: &prompt,
            max_tokens: budget,
            temperature,
            seed,
        },
    )
    .map_err(Fallback::Failed)?;
    Ok(super::engine::LlmEngine::sanitize_text_output(&output.text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_keeps_its_budget_when_the_window_has_room() {
        assert_eq!(answer_budget(12_000, 8_192, 32_768), Some(8_192));
    }

    #[test]
    fn the_answer_is_cut_to_the_space_left_like_on_cpu() {
        // 32768 - 30000 - 256 = 2512 tokens left for the answer.
        assert_eq!(answer_budget(30_000, 8_192, 32_768), Some(2_512));
    }

    #[test]
    fn a_prompt_that_fills_the_window_goes_to_cpu() {
        assert_eq!(answer_budget(32_512, 128, 32_768), None);
        assert_eq!(answer_budget(40_000, 128, 32_768), None);
    }

    #[test]
    fn a_request_keeps_the_window_its_caller_asked_for() {
        // Intent and rewrite ask for 4096: a prompt that would not fit on
        // CPU does not get the larger server window instead.
        assert_eq!(request_window(4_096), 4_096);
        assert_eq!(answer_budget(4_000, 128, request_window(4_096)), None);
        // The RAG answer asks for 131K: the server's 32K is the ceiling.
        assert_eq!(request_window(131_072), GEMMA_SERVER_CTX);
    }

    #[test]
    fn the_fallback_reason_is_readable_in_the_log() {
        assert_eq!(Fallback::Busy.to_string(), "la GPU está ocupada con OCR");
        assert_eq!(
            Fallback::TooLong {
                prompt_tokens: 40_000
            }
            .to_string(),
            "el pedido de 40000 tokens no entra en la ventana de la GPU (32768)"
        );
    }
}
