//! The optional PP-OCRv6 Tiny recognition profile for OCRL: its three MNN
//! files, downloaded on demand from a pinned commit and checked by size and
//! SHA-256 before they count as installed.
//!
//! The files are the MNN conversions `ocr-rs` publishes (Apache-2.0); Paddle
//! publishes only the original inference weights. v5 Latin stays bundled and
//! remains the default profile.

use std::path::{Path, PathBuf};

use super::paddle_vl_local::ModelFile;

/// Repository and commit the files download from, so a later push cannot
/// change them.
pub const REPO: &str = "zibo-chen/rust-paddle-ocr";
pub const REVISION: &str = "664a945953e46186b37203fa4902a20d60a32809";

pub const DET: ModelFile = ModelFile {
    filename: "PP-OCRv6_tiny_det.mnn",
    size: 901_896,
    sha256: "7fab7b858f136bc93a760bdca66aaf25f0ff10accabb31e6ef853a897fb9cfec",
};
pub const REC: ModelFile = ModelFile {
    filename: "PP-OCRv6_tiny_rec.mnn",
    size: 2_251_616,
    sha256: "0a43c3c979a98b905f5e84913209998f510189419b5a5d4152bbb01ce8d17a93",
};
pub const KEYS: ModelFile = ModelFile {
    filename: "ppocr_keys_v6_tiny.txt",
    size: 27_156,
    sha256: "c5cbe34ef40c29c4df07ed012bf96569cb69a2d2a01a07027e9f13cb832bd9cd",
};
pub const FILES: [ModelFile; 3] = [DET, REC, KEYS];
/// Folder under the app models directory that holds the three files.
pub const MODELS_SUBDIR: &str = "ppocr-v6-tiny";

/// Download URL of `file` at the pinned [`REVISION`].
pub fn url(file: &ModelFile) -> String {
    format!(
        "https://raw.githubusercontent.com/{REPO}/{REVISION}/models/{}",
        file.filename
    )
}

/// `<cache>/models/ppocr-v6-tiny`, next to the other local models.
pub fn models_dir(db_path: &Path) -> PathBuf {
    crate::path_utils::models_dir(db_path).join(MODELS_SUBDIR)
}

/// Whether `file` is in `dir` at its pinned size. The hash is checked once,
/// right after the download.
fn installed(dir: &Path, file: &ModelFile) -> bool {
    std::fs::metadata(dir.join(file.filename))
        .is_ok_and(|meta| meta.is_file() && meta.len() == file.size)
}

/// The files `dir` still lacks.
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

/// What the Settings UI shows about the v6 Tiny install.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PpocrV6LocalInfo {
    pub installed: bool,
    /// Size of the three files together, for the download button.
    pub download_bytes: u64,
    pub models_dir: String,
    pub repo_url: String,
}

/// Inspect the install without loading anything.
pub fn local_info(db_path: &Path) -> PpocrV6LocalInfo {
    let dir = models_dir(db_path);
    PpocrV6LocalInfo {
        installed: missing_files(&dir).is_empty(),
        download_bytes: FILES.iter().map(|file| file.size).sum(),
        models_dir: dir.display().to_string(),
        repo_url: format!("https://github.com/{REPO}/tree/{REVISION}/models"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::paddle::RecProfile;
    use super::*;

    #[test]
    fn the_pinned_files_are_the_ones_the_v6_profile_loads() {
        assert_eq!(FILES.map(|file| file.filename), RecProfile::V6Tiny.files());
    }

    #[test]
    fn urls_point_at_the_pinned_commit() {
        assert_eq!(
            url(&DET),
            "https://raw.githubusercontent.com/zibo-chen/rust-paddle-ocr/664a945953e46186b37203fa4902a20d60a32809/models/PP-OCRv6_tiny_det.mnn"
        );
    }

    #[test]
    fn only_files_at_their_pinned_size_count_as_installed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(missing_files(dir.path()).len(), 3);
        assert_eq!(missing_bytes(dir.path()), 901_896 + 2_251_616 + 27_156);

        std::fs::write(dir.path().join(KEYS.filename), vec![b'x'; 27_156]).unwrap();
        std::fs::write(dir.path().join(DET.filename), b"partial").unwrap();
        assert_eq!(
            missing_files(dir.path())
                .iter()
                .map(|file| file.filename)
                .collect::<Vec<_>>(),
            [DET.filename, REC.filename]
        );
    }

    #[test]
    fn info_reports_an_incomplete_install() {
        let dir = tempfile::tempdir().unwrap();
        let info = local_info(&dir.path().join("entropia.sqlite"));
        assert!(!info.installed);
        assert_eq!(info.download_bytes, 3_180_668);
        assert!(info.models_dir.ends_with(MODELS_SUBDIR));
    }
}
