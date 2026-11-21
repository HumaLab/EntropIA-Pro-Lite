//! The process-wide ONNX Runtime environment.
//!
//! Since ort 2.0.0-rc.15 every session is created in an explicit
//! [`Environment`], and a process may build one only once: `build()` refuses
//! a second environment even after the first is dropped. Embeddings, the
//! reranker, OCR layout and Parakeet all load the same ONNX Runtime dylib, so
//! they share the one environment kept here for the life of the process.

use std::sync::{Mutex, OnceLock};

use ort::environment::{Environment, EnvironmentBuilder};

static ENVIRONMENT: OnceLock<Environment> = OnceLock::new();
static INIT: Mutex<()> = Mutex::new(());

/// The shared environment, building it from `builder` on first use.
///
/// `builder` runs only while no environment exists. Its failure (for example
/// no ONNX Runtime library near a model) is returned and not cached, so a
/// later caller with a different model directory can still succeed.
pub(crate) fn get_or_init(
    builder: impl FnOnce() -> Result<EnvironmentBuilder, String>,
) -> Result<Environment, String> {
    if let Some(environment) = ENVIRONMENT.get() {
        return Ok(environment.clone());
    }
    let _guard = INIT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(environment) = ENVIRONMENT.get() {
        return Ok(environment.clone());
    }
    let environment = builder()?
        .build()
        .map_err(|e| format!("Failed to create the ONNX Runtime environment: {e}"))?;
    Ok(ENVIRONMENT.get_or_init(|| environment).clone())
}
