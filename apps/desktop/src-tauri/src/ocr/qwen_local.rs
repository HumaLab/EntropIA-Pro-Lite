//! Local Qwen3-VL-4B assets for OCR correction.
//!
//! The correction model is separate from OCRH's GLM-OCR: measured on
//! historical press pages, the generalist Qwen3-VL-4B preserves reference
//! text far more faithfully than the OCR-specialized GLM-OCR. It runs on the
//! same persistent `llama-server` binary (see [`super::local_server`]); the
//! two features never hold VRAM at once.

use std::path::{Path, PathBuf};

use super::local_server::{
    BUNDLED_SERVER_SUBDIR, SERVER_BINARY_ENV, data_dir, server_binary_name, server_dir,
};

/// Qwen3-VL-4B instruct, Q4_K_M: fits next to a 2 GB KV cache on an 8 GB card.
/// The Q8_0 build needs ~2 GB more VRAM and measures no better on correction.
pub const MODEL_FILENAME: &str = "Qwen3VL-4B-Instruct-Q4_K_M.gguf";
/// Vision projector officially paired with the Q8_0 decoder in the same
/// Qwen release; the projector stays at Q8 (it is small and quality-critical).
pub const MMPROJ_FILENAME: &str = "mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf";
/// Official release both GGUF files download from.
pub const MODELS_REPO_URL: &str = "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF";
pub const MODEL_URL: &str =
    "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF/resolve/main/Qwen3VL-4B-Instruct-Q4_K_M.gguf?download=true";
pub const MMPROJ_URL: &str =
    "https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF/resolve/main/mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf?download=true";
/// Folder under the app models directory that holds both GGUF files.
pub const MODELS_SUBDIR: &str = "qwen3vl-4b-correct";

/// Server configuration for local OCR correction: a single 16k slot. A full
/// correction prompt runs ~6k tokens with up to 8k generated; smaller slots
/// answer HTTP 400 on long pages. One slot is enough (correction runs one job
/// at a time) and keeps VRAM near ~5 GB on an 8 GB card.
pub(crate) const CORRECTION_SERVER_CONFIG: super::local_server::ServerConfig =
    super::local_server::ServerConfig {
        kind: super::local_server::ServerKind::Correction,
        slots: 1,
        ctx_per_slot: 16384,
        log_target: "qwen_correct",
        log_file: "qwen-server.log",
    };

/// Files needed to run local OCR correction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QwenLocalAssets {
    pub server: PathBuf,
    pub model: PathBuf,
    pub mmproj: PathBuf,
}

/// What the Settings UI shows about the local correction install.
#[derive(Clone, Debug, serde::Serialize)]
pub struct QwenLocalInfo {
    pub model_present: bool,
    pub mmproj_present: bool,
    pub server_present: bool,
    pub models_dir: String,
    pub repo_url: String,
}

/// Inspect the install without starting anything.
pub fn local_info(db_path: &Path) -> QwenLocalInfo {
    let models = models_dir(db_path);
    let server = std::env::var_os(SERVER_BINARY_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| server_dir(db_path).join(server_binary_name()));
    QwenLocalInfo {
        model_present: models.join(MODEL_FILENAME).is_file(),
        mmproj_present: models.join(MMPROJ_FILENAME).is_file(),
        server_present: server.is_file(),
        models_dir: models.display().to_string(),
        repo_url: MODELS_REPO_URL.to_string(),
    }
}

/// `<data>/models/qwen3vl-4b-correct`, next to the other local models.
pub fn models_dir(db_path: &Path) -> PathBuf {
    data_dir(db_path).join("models").join(MODELS_SUBDIR)
}

/// Resolve every asset, or explain which ones are missing.
///
/// Same order as OCRH: `ENTROPIA_LLAMA_SERVER` override, managed
/// `<data>/tools/llama-server`, bundled installer resources.
pub fn resolve_assets_with_resource_dir(
    db_path: &Path,
    resource_dir: Option<&Path>,
) -> Result<QwenLocalAssets, String> {
    let env_server = std::env::var_os(SERVER_BINARY_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    resolve_assets_from(db_path, env_server, resource_dir)
}

fn resolve_assets_from(
    db_path: &Path,
    env_server: Option<PathBuf>,
    resource_dir: Option<&Path>,
) -> Result<QwenLocalAssets, String> {
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
    let assets = QwenLocalAssets {
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
            "Corrección local no instalada; faltan: {}",
            missing.join(", ")
        ))
    }
}

fn to_server_assets(assets: &QwenLocalAssets) -> super::local_server::ServerAssets {
    super::local_server::ServerAssets {
        server: assets.server.clone(),
        model: assets.model.clone(),
        mmproj: assets.mmproj.clone(),
    }
}

/// Return the base URL of a healthy correction server, starting it if needed.
///
/// A server running OCRH is stopped first: VRAM fits a single model.
/// Blocking: call it from a blocking context.
pub fn ensure_server(assets: &QwenLocalAssets, db_path: &Path) -> Result<String, String> {
    super::local_server::ensure_server(
        &CORRECTION_SERVER_CONFIG,
        &to_server_assets(assets),
        db_path,
    )
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
            dir.path().join("models/qwen3vl-4b-correct").join(MODEL_FILENAME)
        );
        assert_eq!(
            assets.server,
            dir.path()
                .join("tools/llama-server")
                .join(server_binary_name())
        );
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

    #[test]
    fn correction_slots_fit_a_full_page_prompt() {
        // Truncation budget (prepare_remote_job_request n_ctx) must leave room
        // for the longest measured correction prompt (~6.5k tokens) plus the
        // 8k-token flow cap inside one slot.
        assert!(CORRECTION_SERVER_CONFIG.ctx_per_slot >= 16384);
    }
}
