//! PaddleOCR-VL on llama-server: the GGUF model files and the server they run on.
//!
//! PaddleOCR's `llama-cpp-server` backend keeps layout detection in Paddle
//! and sends each layout block to a llama-server that serves the official
//! PaddleOCR-VL GGUF build. Recognition then runs on any GPU llama.cpp
//! supports (CUDA, HIP, Vulkan, Metal), not only on Paddle's NVIDIA build,
//! and about 14x faster than Paddle's own decoder on the same card.
//!
//! The server is the one `local_server` shares with GLM-OCR, the OCR
//! correction and the chat: VRAM holds one model at a time.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::glm_ocr_local::GlmLocalAssets;
use super::local_server::{env_server_override, resolve_server, ServerSource};

/// One file of the official release, pinned by size and SHA-256.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelFile {
    pub filename: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

impl ModelFile {
    /// Download URL at the pinned [`REVISION`].
    pub fn url(&self) -> String {
        format!(
            "{MODELS_REPO_URL}/resolve/{REVISION}/{}?download=true",
            self.filename
        )
    }
}

/// Official GGUF release of PaddleOCR-VL 1.6 (Apache-2.0).
pub const MODELS_REPO_URL: &str = "https://huggingface.co/PaddlePaddle/PaddleOCR-VL-1.6-GGUF";
/// Revision both files download from, so a later push cannot change them.
pub const REVISION: &str = "511b09642bb324401f15f97cc23bc67e8f0a291d";
pub const MODEL: ModelFile = ModelFile {
    filename: "PaddleOCR-VL-1.6-GGUF.gguf",
    size: 935_769_056,
    sha256: "f3ae46ec885050acf4b3d31944431e1fd90d50664fb09126af4a3c050ba14ee8",
};
pub const MMPROJ: ModelFile = ModelFile {
    filename: "PaddleOCR-VL-1.6-GGUF-mmproj.gguf",
    size: 881_770_560,
    sha256: "204d757d7610d9b3faab10d506d69e5b244e32bf765e2bab2d0167e65e0a058a",
};
pub const FILES: [ModelFile; 2] = [MODEL, MMPROJ];
/// Folder under the app models directory that holds both GGUF files.
pub const MODELS_SUBDIR: &str = "paddleocr-vl";

/// Blocks the script sends at once; the server gets one slot per block.
pub const PARALLEL_SLOTS: u32 = 4;
/// Context per slot: PaddleX allows 8192 new tokens per block, plus up to
/// 1280 image tokens (1003520 pixels in 28x28 patches) and the prompt. The
/// KV cache costs 144 MiB per 8192 tokens, so 4 slots take about 720 MiB.
const CONTEXT_PER_SLOT: u32 = 10240;
const _: () = assert!(CONTEXT_PER_SLOT >= 8192 + 1280 + 256);

/// `<cache>/models/paddleocr-vl`, next to the other local models.
pub fn models_dir(db_path: &Path) -> PathBuf {
    crate::path_utils::models_dir(db_path).join(MODELS_SUBDIR)
}

/// Whether `file` is in `dir` at its pinned size. A partial or older file
/// does not count; the hash is checked once, right after the download.
fn installed(dir: &Path, file: &ModelFile) -> bool {
    std::fs::metadata(dir.join(file.filename))
        .is_ok_and(|meta| meta.is_file() && meta.len() == file.size)
}

/// The release files `dir` still lacks.
pub fn missing_files(dir: &Path) -> Vec<ModelFile> {
    FILES
        .into_iter()
        .filter(|file| !installed(dir, file))
        .collect()
}

/// Bytes a download into `dir` still has to fetch.
pub fn missing_bytes(dir: &Path) -> u64 {
    missing_files(dir).iter().map(|file| file.size).sum()
}

/// Check a downloaded file against its pinned size and hash. A mismatch
/// removes the file, so it is never served and downloads again.
pub fn verify_download(path: &Path, file: &ModelFile) -> Result<(), String> {
    let result = file_matches(path, file);
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn file_matches(path: &Path, file: &ModelFile) -> Result<(), String> {
    let mut reader = std::fs::File::open(path)
        .map_err(|e| format!("No se pudo leer {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    let mut size = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("No se pudo leer {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if size == file.size && digest == file.sha256 {
        Ok(())
    } else {
        Err(format!(
            "{} no coincide con la versión publicada (tamaño o huella distintos); se borró. Volvé a descargarlo.",
            file.filename
        ))
    }
}

/// What the Settings UI shows about the PaddleOCR-VL GGUF install.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PaddleVlLocalInfo {
    pub model_present: bool,
    pub mmproj_present: bool,
    pub server_present: bool,
    pub server_source: ServerSource,
    /// This machine has a GPU with enough memory for this engine.
    pub gpu_fits: bool,
    /// The memory floor for a non-NVIDIA GPU, in nominal GB.
    pub min_vram_gb: u32,
    /// Size of both files together, for the download button.
    pub download_bytes: u64,
    pub models_dir: String,
    pub repo_url: String,
}

/// Inspect the install without starting anything.
pub fn local_info(db_path: &Path, resource_dir: Option<&Path>) -> PaddleVlLocalInfo {
    use super::llama_gpu::{engine_runs, Engine};
    let models = models_dir(db_path);
    PaddleVlLocalInfo {
        model_present: installed(&models, &MODEL),
        mmproj_present: installed(&models, &MMPROJ),
        server_present: super::local_server::server_ready(
            &resolve_server(db_path, env_server_override(), resource_dir),
            db_path,
        ),
        server_source: super::local_server::server_source(),
        gpu_fits: engine_runs(Engine::PaddleVl),
        min_vram_gb: Engine::PaddleVl.min_vram_gb(),
        download_bytes: FILES.iter().map(|file| file.size).sum(),
        models_dir: models.display().to_string(),
        repo_url: MODELS_REPO_URL.to_string(),
    }
}

/// Resolve the server and both model files, or explain what is missing.
pub fn resolve_assets_with_resource_dir(
    db_path: &Path,
    resource_dir: Option<&Path>,
) -> Result<GlmLocalAssets, String> {
    resolve_assets_from(db_path, env_server_override(), resource_dir)
}

fn resolve_assets_from(
    db_path: &Path,
    env_server: Option<PathBuf>,
    resource_dir: Option<&Path>,
) -> Result<GlmLocalAssets, String> {
    let server = resolve_server(db_path, env_server, resource_dir);
    let models = models_dir(db_path);
    let mut missing: Vec<String> = missing_files(&models)
        .iter()
        .map(|file| models.join(file.filename).display().to_string())
        .collect();
    if !server.is_file() {
        missing.insert(0, server.display().to_string());
    }
    if missing.is_empty() {
        Ok(GlmLocalAssets {
            server,
            model: models.join(MODEL.filename),
            mmproj: models.join(MMPROJ.filename),
        })
    } else {
        Err(format!(
            "PaddleOCR-VL en llama-server no está instalado; faltan: {}",
            missing.join(", ")
        ))
    }
}

/// Server configuration for PaddleOCR-VL: 4 slots, one layout block each.
pub(crate) const SERVER_CONFIG: super::local_server::ServerConfig =
    super::local_server::ServerConfig {
        kind: super::local_server::ServerKind::Ocrh,
        slots: PARALLEL_SLOTS,
        ctx_per_slot: CONTEXT_PER_SLOT,
        log_target: "paddle_vl_local",
        log_file: "llama-server.log",
    };

/// Return a lease on a healthy server for `assets`, starting it if needed.
/// Blocking: call it from a blocking context (the OCRH subprocess path).
pub fn ensure_server(
    assets: &GlmLocalAssets,
    db_path: &Path,
) -> Result<super::local_server::Lease, String> {
    super::local_server::ensure_server(
        &SERVER_CONFIG,
        &super::local_server::ServerAssets {
            server: assets.server.clone(),
            model: assets.model.clone(),
            mmproj: Some(assets.mmproj.clone()),
        },
        db_path,
    )
}

#[cfg(test)]
mod tests {
    use super::super::local_server::{server_binary_name, server_dir};
    use super::*;

    fn write_sized(path: &Path, size: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(path).unwrap().set_len(size).unwrap();
    }

    #[test]
    fn urls_point_at_the_pinned_revision() {
        assert_eq!(
            MODEL.url(),
            "https://huggingface.co/PaddlePaddle/PaddleOCR-VL-1.6-GGUF/resolve/511b09642bb324401f15f97cc23bc67e8f0a291d/PaddleOCR-VL-1.6-GGUF.gguf?download=true"
        );
    }

    #[test]
    fn a_file_of_the_wrong_size_still_counts_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        write_sized(&dir.path().join(MODEL.filename), MODEL.size);
        write_sized(&dir.path().join(MMPROJ.filename), 10);

        assert_eq!(missing_files(dir.path()), vec![MMPROJ]);
        assert_eq!(missing_bytes(dir.path()), MMPROJ.size);
    }

    #[test]
    fn complete_assets_resolve_under_the_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("entropia.sqlite");
        write_sized(&models_dir(&db).join(MODEL.filename), MODEL.size);
        write_sized(&models_dir(&db).join(MMPROJ.filename), MMPROJ.size);
        write_sized(&server_dir(&db).join(server_binary_name()), 1);

        let assets = resolve_assets_from(&db, None, None).unwrap();

        assert_eq!(
            assets.model,
            dir.path().join("models/paddleocr-vl").join(MODEL.filename)
        );
        assert_eq!(
            assets.mmproj,
            dir.path().join("models/paddleocr-vl").join(MMPROJ.filename)
        );
    }

    #[test]
    fn missing_assets_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("entropia.sqlite");

        let error = resolve_assets_from(&db, None, None).unwrap_err();

        assert!(error.contains(MODEL.filename), "{error}");
        assert!(error.contains(MMPROJ.filename), "{error}");
        assert!(error.contains(server_binary_name()), "{error}");
    }

    #[test]
    fn a_download_with_the_wrong_hash_is_refused_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.gguf");
        std::fs::write(&path, b"abc").unwrap();
        let expected = ModelFile {
            filename: "x.gguf",
            size: 3,
            // SHA-256 of "abd".
            sha256: "a52d159f262b2c6ddb724a61840befc36eb30c88877a4030b65cbe86298449c9",
        };

        let error = verify_download(&path, &expected).unwrap_err();

        assert!(error.starts_with("x.gguf no coincide"), "{error}");
        assert!(!path.exists());
    }

    #[test]
    fn a_download_with_the_pinned_hash_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.gguf");
        std::fs::write(&path, b"abc").unwrap();
        let expected = ModelFile {
            filename: "x.gguf",
            size: 3,
            // SHA-256 of "abc".
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };

        verify_download(&path, &expected).unwrap();

        assert!(path.exists());
    }
}
