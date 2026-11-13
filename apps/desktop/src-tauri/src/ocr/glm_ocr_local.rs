//! Local GLM-OCR: asset resolution and the persistent `llama-server` it runs on.
//!
//! OCRH's local GLM-OCR backend reuses the PaddleVL subprocess contract
//! (`scripts/glm_ocr_local.py` prints the same sentinel JSON). The script runs
//! layout detection on CPU and sends every box to one `llama-server` process
//! that keeps GLM-OCR loaded on the GPU, so the model is loaded once per app
//! session instead of once per box.
//!
//! The server is started lazily on the first OCRH run, reused while its assets
//! do not change, and stopped on app exit ([`shutdown`]). A server left behind
//! by a crashed session is killed through its pid file on the next start.
//! `PR_SET_PDEATHSIG` is deliberately not used: it fires when the spawning
//! *thread* exits, and tokio recycles idle blocking threads after ~10 s.

use std::path::{Path, PathBuf};

pub(crate) use super::local_server::{
    BUNDLED_SERVER_SUBDIR, SERVER_BINARY_ENV, data_dir, server_binary_name, server_dir,
};

/// Recognition model: Q8_0 decoder. Measured against f16 on scanned press
/// pages, it recovers the reference text fully while being 1.5x faster.
pub const MODEL_FILENAME: &str = "GLM-OCR-Q8_0.gguf";
/// Vision projector officially paired with the Q8_0 decoder in the same
/// `ggml-org/GLM-OCR-GGUF` release. Validated on the same scanned press
/// pages: identical recall to the f16 projector at half the download size.
pub const MMPROJ_FILENAME: &str = "mmproj-GLM-OCR-Q8_0.gguf";
/// Official release both GGUF files download from (same revision, paired).
pub const MODELS_REPO_URL: &str = "https://huggingface.co/ggml-org/GLM-OCR-GGUF";
pub const MODEL_URL: &str =
    "https://huggingface.co/ggml-org/GLM-OCR-GGUF/resolve/main/GLM-OCR-Q8_0.gguf?download=true";
pub const MMPROJ_URL: &str =
    "https://huggingface.co/ggml-org/GLM-OCR-GGUF/resolve/main/mmproj-GLM-OCR-Q8_0.gguf?download=true";
/// Folder under the app models directory that holds both GGUF files.
pub const MODELS_SUBDIR: &str = "glm-ocr";
/// Setting that picks the local OCRH engine: `auto`, `glm_ocr` or `paddle_vl`.
pub const OCRH_LOCAL_ENGINE_SETTING: &str = "ocrh_local_engine";

pub const ENGINE_AUTO: &str = "auto";
pub const ENGINE_GLM_OCR: &str = "glm_ocr";

/// What the Settings UI shows about the local GLM-OCR install.
#[derive(Clone, Debug, serde::Serialize)]
pub struct GlmOcrLocalInfo {
    pub model_present: bool,
    pub mmproj_present: bool,
    pub server_present: bool,
    pub models_dir: String,
    pub repo_url: String,
}

/// Inspect the install without starting anything.
pub fn local_info(db_path: &Path) -> GlmOcrLocalInfo {
    let models = models_dir(db_path);
    let server = std::env::var_os(SERVER_BINARY_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| server_dir(db_path).join(server_binary_name()));
    GlmOcrLocalInfo {
        model_present: models.join(MODEL_FILENAME).is_file(),
        mmproj_present: models.join(MMPROJ_FILENAME).is_file(),
        server_present: server.is_file(),
        models_dir: models.display().to_string(),
        repo_url: MODELS_REPO_URL.to_string(),
    }
}

/// Concurrent requests the server accepts; the script sends this many boxes at once.
pub const PARALLEL_SLOTS: u32 = 4;
/// Context per slot. One box is ~150 prompt tokens plus at most 2048 generated;
/// llama.cpp would otherwise size the KV cache from the model's 131k training
/// context and reserve over 5 GB of VRAM. Output is identical either way.
const CONTEXT_PER_SLOT: u32 = 4096;
/// Files needed to run local GLM-OCR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlmLocalAssets {
    pub server: PathBuf,
    pub model: PathBuf,
    pub mmproj: PathBuf,
}

/// `<data>/models/glm-ocr`, next to the local LLM models.
pub fn models_dir(db_path: &Path) -> PathBuf {
    data_dir(db_path).join("models").join(MODELS_SUBDIR)
}


/// Resolve every asset, or explain which ones are missing.
///
/// Server binary resolution order: `ENTROPIA_LLAMA_SERVER` override,
/// managed `<data>/tools/llama-server`, then bundled installer resources
/// (`tools/llama-server`, fetched at build time, never committed).
pub fn resolve_assets_with_resource_dir(
    db_path: &Path,
    resource_dir: Option<&Path>,
) -> Result<GlmLocalAssets, String> {
    let env_server = std::env::var_os(SERVER_BINARY_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    resolve_assets_from(db_path, env_server, resource_dir)
}


fn resolve_assets_from(
    db_path: &Path,
    env_server: Option<PathBuf>,
    resource_dir: Option<&Path>,
) -> Result<GlmLocalAssets, String> {
    let server = env_server.unwrap_or_else(|| {
        let managed = server_dir(db_path).join(server_binary_name());
        if managed.is_file() {
            return managed;
        }
        if let Some(root) = resource_dir {
            let bundled = root.join(BUNDLED_SERVER_SUBDIR).join(server_binary_name());
            if bundled.is_file() {
                return bundled;
            }
        }
        managed
    });
    let models = models_dir(db_path);
    let assets = GlmLocalAssets {
        server,
        model: models.join(MODEL_FILENAME),
        mmproj: models.join(MMPROJ_FILENAME),
    };
    let missing: Vec<String> = [&assets.server, &assets.model, &assets.mmproj]
        .into_iter()
        .filter(|path| !path.is_file())
        .map(|path| path.display().to_string())
        .collect();
    if missing.is_empty() {
        Ok(assets)
    } else {
        Err(format!(
            "GLM-OCR local no está instalado; faltan: {}",
            missing.join(", ")
        ))
    }
}

/// Server configuration for OCRH's GLM-OCR: 4 parallel slots of 4k ctx,
/// sized for one layout box per slot (~150 prompt tokens, ≤2048 generated).
pub(crate) const OCRH_SERVER_CONFIG: super::local_server::ServerConfig =
    super::local_server::ServerConfig {
        kind: super::local_server::ServerKind::Ocrh,
        slots: PARALLEL_SLOTS,
        ctx_per_slot: CONTEXT_PER_SLOT,
        log_target: "glm_ocr_local",
        log_file: "llama-server.log",
    };

fn to_server_assets(assets: &GlmLocalAssets) -> super::local_server::ServerAssets {
    super::local_server::ServerAssets {
        server: assets.server.clone(),
        model: assets.model.clone(),
        mmproj: assets.mmproj.clone(),
    }
}

/// Return the base URL of a healthy server for `assets`, starting it if needed.
///
/// A server running the correction model is stopped first: VRAM fits one.
/// Blocking: call it from a blocking context (the OCRH subprocess path).
pub fn ensure_server(assets: &GlmLocalAssets, db_path: &Path) -> Result<String, String> {
    super::local_server::ensure_server(&OCRH_SERVER_CONFIG, &to_server_assets(assets), db_path)
}

/// Stop the server, if one is running. Called on app exit.
pub fn shutdown() {
    super::local_server::shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_assets_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("entropia.sqlite");
        let error = resolve_assets_from(&db, None, None).unwrap_err();
        assert!(error.contains(MODEL_FILENAME));
        assert!(error.contains(MMPROJ_FILENAME));
        assert!(error.contains(server_binary_name()));
    }

    #[test]
    fn complete_assets_resolve_under_the_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("entropia.sqlite");
        std::fs::create_dir_all(models_dir(&db)).unwrap();
        std::fs::create_dir_all(server_dir(&db)).unwrap();
        std::fs::write(models_dir(&db).join(MODEL_FILENAME), b"m").unwrap();
        std::fs::write(models_dir(&db).join(MMPROJ_FILENAME), b"p").unwrap();
        std::fs::write(server_dir(&db).join(server_binary_name()), b"s").unwrap();

        let assets = resolve_assets_from(&db, None, None).unwrap();
        assert_eq!(
            assets.model,
            dir.path().join("models/glm-ocr").join(MODEL_FILENAME)
        );
        assert_eq!(
            assets.server,
            dir.path()
                .join("tools/llama-server")
                .join(server_binary_name())
        );
    }

    #[test]
    fn env_override_replaces_the_managed_server_path() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("entropia.sqlite");
        let custom = dir.path().join("custom-server");
        let error = resolve_assets_from(&db, Some(custom.clone()), None).unwrap_err();
        assert!(error.contains(&custom.display().to_string()));
    }

    #[test]
    fn bundled_server_is_used_when_managed_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("entropia.sqlite");
        let resources = dir.path().join("resources");
        let bundled_dir = resources.join(BUNDLED_SERVER_SUBDIR);
        std::fs::create_dir_all(models_dir(&db)).unwrap();
        std::fs::create_dir_all(&bundled_dir).unwrap();
        std::fs::write(models_dir(&db).join(MODEL_FILENAME), b"m").unwrap();
        std::fs::write(models_dir(&db).join(MMPROJ_FILENAME), b"p").unwrap();
        std::fs::write(bundled_dir.join(server_binary_name()), b"s").unwrap();

        let assets = resolve_assets_from(&db, None, Some(&resources)).unwrap();
        assert_eq!(assets.server, bundled_dir.join(server_binary_name()));
    }

}
