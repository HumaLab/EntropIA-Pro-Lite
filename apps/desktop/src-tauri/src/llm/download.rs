use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use super::{LlmDownloadCompletePayload, LlmDownloadProgressPayload};

const DOWNLOAD_CHUNK_SIZE: usize = 64 * 1024;
const DOWNLOAD_TIMEOUT_SECS: u64 = 600;

pub(crate) fn open_managed_models_child(
    parent: &cap_std::fs::Dir,
    child_name: &Path,
) -> Result<cap_std::fs::Dir, String> {
    use cap_fs_ext::{
        FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsMaybeDirExt as _,
    };

    let mut options = cap_std::fs::OpenOptions::new();
    options
        .read(true)
        .maybe_dir(true)
        .follow(FollowSymlinks::No);
    let child_handle = parent.open_with(child_name, &options).map_err(|error| {
        format!(
            "Failed to open managed models child {} without following links: {error}",
            child_name.display()
        )
    })?;
    cap_std::fs::Dir::reopen_dir(&child_handle).map_err(|error| {
        format!(
            "Failed to convert managed models child {} into a directory capability: {error}",
            child_name.display()
        )
    })
}

pub fn open_managed_models_dir(models_dir: &Path) -> Result<cap_std::fs::Dir, String> {
    super::validate_physical_models_dir(models_dir)?;
    let parent_path = models_dir
        .parent()
        .ok_or_else(|| "Managed models directory must have a parent".to_string())?;
    let child_name = models_dir
        .file_name()
        .ok_or_else(|| "Managed models directory must have a basename".to_string())?;
    let parent = cap_std::fs::Dir::open_ambient_dir(
        parent_path,
        cap_std::ambient_authority(),
    )
    .map_err(|error| {
        format!(
            "Failed to open managed models parent {}: {error}",
            parent_path.display()
        )
    })?;
    open_managed_models_child(&parent, Path::new(child_name))
}

fn temporary_filename(filename: &str) -> PathBuf {
    Path::new(filename).with_extension("download.tmp")
}

struct ManagedTemporaryFile<'a> {
    dir: &'a cap_std::fs::Dir,
    filename: PathBuf,
    file: Option<cap_std::fs::File>,
    armed: bool,
}

impl<'a> ManagedTemporaryFile<'a> {
    fn create(dir: &'a cap_std::fs::Dir, model_filename: &str) -> Result<Self, String> {
        super::validate_local_model_filename(model_filename)?;
        let filename = temporary_filename(model_filename);
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let file = dir.open_with(&filename, &options).map_err(|error| {
            format!(
                "Failed to exclusively create managed temp file {}: {error}",
                filename.display()
            )
        })?;
        Ok(Self {
            dir,
            filename,
            file: Some(file),
            armed: true,
        })
    }

    fn writer(&mut self) -> &mut cap_std::fs::File {
        self.file
            .as_mut()
            .expect("managed temporary file must remain open while writing")
    }

    fn finish(mut self, model_filename: &str) -> Result<(), String> {
        drop(self.file.take());
        validate_managed_gguf(self.dir, &self.filename)?;
        self.dir
            .rename(&self.filename, self.dir, model_filename)
            .map_err(|error| {
                format!(
                    "Failed to finalize managed download from {} to {model_filename}: {error}",
                    self.filename.display()
                )
            })?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for ManagedTemporaryFile<'_> {
    fn drop(&mut self) {
        drop(self.file.take());
        if self.armed {
            let _ = self.dir.remove_file(&self.filename);
        }
    }
}

fn validate_gguf_reader<R: Read + Seek>(reader: &mut R) -> Result<(), String> {
    let length = reader
        .seek(SeekFrom::End(0))
        .map_err(|error| format!("Failed to inspect downloaded model: {error}"))?;
    if length < 8 {
        return Err(format!(
            "Downloaded model is too small to be a valid GGUF file ({length} bytes)"
        ));
    }
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("Failed to rewind downloaded model: {error}"))?;
    let mut magic = [0_u8; 4];
    reader
        .read_exact(&mut magic)
        .map_err(|error| format!("Failed to read GGUF header: {error}"))?;
    if &magic != b"GGUF" {
        return Err("Downloaded model is not a GGUF file (missing GGUF header)".to_string());
    }
    Ok(())
}

fn validate_managed_gguf(
    managed_dir: &cap_std::fs::Dir,
    temporary_filename: &Path,
) -> Result<(), String> {
    let mut file = managed_dir.open(temporary_filename).map_err(|error| {
        format!(
            "Failed to reopen managed downloaded model {}: {error}",
            temporary_filename.display()
        )
    })?;
    validate_gguf_reader(&mut file)
}

#[cfg(test)]
pub(crate) fn persist_gguf_download<R: Read>(
    managed_dir: &cap_std::fs::Dir,
    filename: &str,
    reader: &mut R,
) -> Result<(), String> {
    let mut temporary = ManagedTemporaryFile::create(managed_dir, filename)?;
    std::io::copy(reader, temporary.writer())
        .map_err(|error| format!("Failed while writing managed download: {error}"))?;
    temporary.finish(filename)
}

/// Build the model download client with transport-level HTTPS enforcement,
/// including redirect targets.
pub(crate) fn build_model_download_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .https_only(true)
        .build()
        .map_err(|error| format!("Failed to create HTTPS-only HTTP client: {error}"))
}

/// Download a GGUF model into an already-opened managed directory capability,
/// emitting progress events via the Tauri event bus.
pub fn download_model_file(
    url: &str,
    managed_dir: cap_std::fs::Dir,
    filename: &str,
    display_destination: &Path,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let client = build_model_download_client()?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Failed to start download: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("Download request failed with HTTP {status}"));
    }

    let total_bytes = response.content_length();
    let mut reader = response;
    let mut temporary = ManagedTemporaryFile::create(&managed_dir, filename)?;
    let mut downloaded_bytes = 0_u64;
    let mut buffer = vec![0_u8; DOWNLOAD_CHUNK_SIZE];
    let mut last_reported_pct = 0_u8;

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("Failed while reading download stream: {e}"))?;
        if read == 0 {
            break;
        }
        temporary
            .writer()
            .write_all(&buffer[..read])
            .map_err(|e| format!("Failed while writing download: {e}"))?;
        downloaded_bytes += read as u64;

        let pct = total_bytes.and_then(|total| {
            if total > 0 {
                Some(((downloaded_bytes.saturating_mul(100)) / total).min(100) as u8)
            } else {
                None
            }
        });

        if let Some(pct) = pct {
            if pct >= last_reported_pct.saturating_add(5) || pct == 100 {
                last_reported_pct = pct;
                let _ = app_handle.emit(
                    "llm:download_progress",
                    LlmDownloadProgressPayload {
                        pct,
                        downloaded_bytes,
                        total_bytes,
                    },
                );
                crate::app_logs::info(
                    app_handle,
                    "llm/download",
                    format!("Descarga de modelo local {pct}%"),
                );
            }
        }
    }

    temporary.finish(filename)?;
    let display_path = display_destination.to_string_lossy().to_string();
    let _ = app_handle.emit(
        "llm:download_complete",
        LlmDownloadCompletePayload {
            path: display_path,
        },
    );
    crate::app_logs::info(
        app_handle,
        "llm/download",
        format!(
            "Descarga de modelo local completada: {}",
            display_destination.display()
        ),
    );
    Ok(())
}

#[cfg(test)]
fn validate_gguf_download(path: &Path) -> Result<(), String> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        format!(
            "Failed to validate downloaded model {}: {error}",
            path.display()
        )
    })?;
    validate_gguf_reader(&mut file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn create_directory_alias(target: &Path, alias: &Path) {
        std::os::unix::fs::symlink(target, alias).unwrap();
    }

    #[cfg(windows)]
    fn create_directory_alias(target: &Path, alias: &Path) {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(alias)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "failed to create test junction: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn model_download_contract_refuses_last_component_alias_from_parent_capability() {
        let tmp = tempfile::tempdir().unwrap();
        let sibling = tmp.path().join("sibling");
        std::fs::create_dir_all(&sibling).unwrap();
        create_directory_alias(&sibling, &tmp.path().join("models"));
        let parent = cap_std::fs::Dir::open_ambient_dir(
            tmp.path(),
            cap_std::ambient_authority(),
        )
        .unwrap();

        let result = open_managed_models_child(&parent, Path::new("models"));

        assert!(
            result.is_err(),
            "the last component must be opened without following a link/reparse alias"
        );
    }

    #[test]
    fn model_download_contract_http_transport_is_rejected_by_actual_client() {
        let client = build_model_download_client().unwrap();

        let error = client
            .get("http://127.0.0.1:9/model.gguf")
            .send()
            .unwrap_err();

        assert!(
            error.is_builder(),
            "HTTPS-only client must reject HTTP before transport: {error}"
        );
    }


    #[test]
    fn model_download_contract_keeps_capability_relative_operations_after_models_path_swap() {
        let tmp = tempfile::tempdir().unwrap();
        let models_path = tmp.path().join("models");
        let opened_models_path = tmp.path().join("models-opened");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&models_path).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let managed_dir = cap_std::fs::Dir::open_ambient_dir(
            &models_path,
            cap_std::ambient_authority(),
        )
        .unwrap();

        let persisted_models_path = match std::fs::rename(&models_path, &opened_models_path) {
            Ok(()) => {
                create_directory_alias(&outside, &models_path);
                opened_models_path.clone()
            }
            Err(error) => {
                #[cfg(windows)]
                assert_eq!(
                    error.raw_os_error(),
                    Some(32),
                    "unexpected Windows error while attempting directory swap: {error}"
                );
                #[cfg(not(windows))]
                panic!("unexpected failure while attempting directory swap: {error}");
                assert!(models_path.is_dir());
                assert!(!opened_models_path.exists());
                models_path.clone()
            }
        };
        let outside_temporary = outside.join("gemma-model.download.tmp");
        let sentinel = b"outside-owned-content";
        std::fs::write(&outside_temporary, sentinel).unwrap();
        let mut payload = std::io::Cursor::new(b"GGUFvalid-content".to_vec());

        persist_gguf_download(&managed_dir, "gemma-model.gguf", &mut payload).unwrap();

        assert_eq!(
            std::fs::read(persisted_models_path.join("gemma-model.gguf")).unwrap(),
            b"GGUFvalid-content"
        );
        assert!(
            !persisted_models_path
                .join("gemma-model.download.tmp")
                .exists()
        );
        assert!(!outside.join("gemma-model.gguf").exists());
        assert_eq!(std::fs::read(outside_temporary).unwrap(), sentinel);
    }


    #[test]
    fn model_download_contract_gguf_validation_accepts_content_independent_of_filename() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("not-a-model.txt");
        std::fs::write(&path, b"GGUFvalid-content").unwrap();

        assert!(validate_gguf_download(&path).is_ok());
    }

    #[test]
    fn model_download_contract_gguf_validation_rejects_content_independent_of_filename() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gemma-model.gguf");
        std::fs::write(&path, b"NOT_GGUF_content").unwrap();

        let error = validate_gguf_download(&path).unwrap_err();

        assert!(error.contains("not a GGUF file"));
    }
}
