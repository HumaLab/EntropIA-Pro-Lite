//! Managed download of the `llama-server` build behind local GLM-OCR and
//! local Qwen correction, into `<data>/tools/llama-server`.
//!
//! Windows installers ship no server. Both local engines run only on an
//! NVIDIA GPU, so the useful build is CUDA, and with NVIDIA's runtime DLLs it
//! is ~610 MB compressed: every installer (MSI, NSIS and the Store MSIX made
//! from the MSI) would carry it for the few machines that can use it. The app
//! instead downloads llama.cpp's official release of the exact revision it
//! links (tag `b8783` = e21cdc11, the sources llama-cpp-sys-2 0.1.145 vendors),
//! checks every archive against its pinned SHA-256 and extracts
//! `llama-server.exe` with its DLLs. Linux keeps the server bundled in the
//! `.deb` (scripts/fetch-llama-server.sh).

use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use super::local_server::{server_binary_name, server_dir, server_source, ServerSource};
use crate::llm::download::{
    build_model_download_client, ModelDownloadCompletePayload, ModelDownloadErrorPayload,
    ModelDownloadProgressPayload,
};

/// Prefix of the emitted events (`llama-server:download_progress`, ...).
const EVENT_SCOPE: &str = "llama-server";
/// llama.cpp release tag of the revision the app links. Bump together with
/// the llama-cpp-2 crate and scripts/fetch-llama-server.sh.
const RELEASE_TAG: &str = "b8783";
const DOWNLOAD_CHUNK_SIZE: usize = 64 * 1024;

/// One archive of the pinned release.
struct ReleaseAsset {
    name: &'static str,
    size: u64,
    sha256: &'static str,
}

/// The CUDA 12.4 build and NVIDIA's runtime (cudart, cuBLAS) for it. CUDA 12
/// rather than 13: it runs on older drivers and on pre-Turing cards.
#[cfg(all(windows, target_arch = "x86_64"))]
const ASSETS: &[ReleaseAsset] = &[
    ReleaseAsset {
        name: "llama-b8783-bin-win-cuda-12.4-x64.zip",
        size: 249_262_710,
        sha256: "e34b8589ccee78511ca0673702519f8aea8b74b930e209f5256cd9b7d1a044ec",
    },
    ReleaseAsset {
        name: "cudart-llama-bin-win-cuda-12.4-x64.zip",
        size: 391_443_627,
        sha256: "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6",
    },
];
#[cfg(not(all(windows, target_arch = "x86_64")))]
const ASSETS: &[ReleaseAsset] = &[];

/// One download at a time: the GLM-OCR card, the Qwen card and the
/// assistant's `models_setup` all start the same install.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Queue the server download in the background. Emits
/// `llama-server:download_progress`, `llama-server:download_complete` and
/// `llama-server:download_error`.
pub(crate) fn start_download(db_path: &Path, app_handle: AppHandle) -> Result<String, String> {
    if server_dir(db_path).join(server_binary_name()).is_file() {
        return Ok("already complete".to_string());
    }
    match server_source() {
        ServerSource::Download => {}
        ServerSource::Bundled => {
            return Err(
                "En este sistema llama-server viene con el instalador; no se descarga aparte."
                    .to_string(),
            )
        }
        ServerSource::RequiresNvidiaGpu => {
            return Err(
                "llama-server solo se descarga en equipos con GPU NVIDIA: GLM-OCR y Qwen locales no corren en CPU."
                    .to_string(),
            )
        }
    }
    if RUNNING.swap(true, Ordering::AcqRel) {
        return Ok("already running".to_string());
    }

    crate::app_logs::info(
        &app_handle,
        "llama-server/download",
        format!("Inicio descarga de llama-server ({RELEASE_TAG}, CUDA 12.4)"),
    );
    let db_path = db_path.to_path_buf();
    tauri::async_runtime::spawn(async move {
        let worker_handle = app_handle.clone();
        let result = tokio::task::spawn_blocking(move || install(&db_path, &worker_handle))
            .await
            .unwrap_or_else(|e| Err(format!("Download task panicked: {e}")));
        RUNNING.store(false, Ordering::Release);
        match result {
            Ok(server) => {
                crate::app_logs::info(
                    &app_handle,
                    "llama-server/download",
                    format!("llama-server instalado en {}", server.display()),
                );
                let _ = app_handle.emit(
                    &format!("{EVENT_SCOPE}:download_complete"),
                    ModelDownloadCompletePayload {
                        path: server.display().to_string(),
                    },
                );
            }
            Err(error) => {
                crate::app_logs::error(
                    &app_handle,
                    "llama-server/download",
                    format!("Descarga de llama-server falló: {error}"),
                );
                let _ = app_handle.emit(
                    &format!("{EVENT_SCOPE}:download_error"),
                    ModelDownloadErrorPayload { error },
                );
            }
        }
    });
    Ok("started".to_string())
}

/// Download, verify and extract every archive into a work directory next to
/// the managed one, then move the files in. The work directory is removed
/// whatever happens.
fn install(db_path: &Path, app_handle: &AppHandle) -> Result<PathBuf, String> {
    let target = server_dir(db_path);
    let work = target.with_file_name("llama-server.partial");
    // Left behind by an attempt the app did not survive.
    let _ = std::fs::remove_dir_all(&work);
    let result = download_and_stage(&work, app_handle)
        .and_then(|staging| move_into_place(&staging, &target));
    let _ = std::fs::remove_dir_all(&work);
    result.map(|()| target.join(server_binary_name()))
}

fn download_and_stage(work: &Path, app_handle: &AppHandle) -> Result<PathBuf, String> {
    let staging = work.join("staging");
    std::fs::create_dir_all(&staging)
        .map_err(|e| format!("Cannot create {}: {e}", staging.display()))?;
    let client = build_model_download_client()?;
    let total: u64 = ASSETS.iter().map(|asset| asset.size).sum();
    let mut done = 0_u64;
    let mut last_pct = 0_u8;

    for asset in ASSETS {
        let url = format!(
            "https://github.com/ggml-org/llama.cpp/releases/download/{RELEASE_TAG}/{}",
            asset.name
        );
        let mut response = client
            .get(&url)
            .send()
            .map_err(|e| format!("Failed to start download of {}: {e}", asset.name))?;
        if !response.status().is_success() {
            return Err(format!(
                "Download of {} failed with HTTP {}",
                asset.name,
                response.status()
            ));
        }
        let archive_path = work.join(asset.name);
        let mut archive = std::fs::File::create(&archive_path)
            .map_err(|e| format!("Cannot create {}: {e}", archive_path.display()))?;
        // Batched writes (R8): one read is about one TLS record.
        let mut buffered = std::io::BufWriter::with_capacity(1 << 20, &mut archive);
        copy_verified(&mut response, &mut buffered, asset, |bytes| {
            done += bytes;
            let pct = (done.saturating_mul(100) / total).min(100) as u8;
            if pct >= last_pct.saturating_add(5) || pct == 100 {
                last_pct = pct;
                let _ = app_handle.emit(
                    &format!("{EVENT_SCOPE}:download_progress"),
                    ModelDownloadProgressPayload {
                        pct,
                        downloaded_bytes: done,
                        total_bytes: Some(total),
                    },
                );
            }
        })?;
        drop(buffered);
        archive
            .rewind()
            .map_err(|e| format!("Cannot reread {}: {e}", archive_path.display()))?;
        extract_server_files(archive, &staging).map_err(|e| format!("{}: {e}", asset.name))?;
        let _ = std::fs::remove_file(&archive_path);
    }
    if !staging.join(server_binary_name()).is_file() {
        return Err(format!("The release has no {}", server_binary_name()));
    }
    Ok(staging)
}

/// Stream `reader` into `writer`, failing unless exactly `asset.size` bytes
/// with `asset.sha256` arrive. `on_bytes` gets each chunk length.
fn copy_verified<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    asset: &ReleaseAsset,
    mut on_bytes: impl FnMut(u64),
) -> Result<(), String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; DOWNLOAD_CHUNK_SIZE];
    let mut received = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("Failed while reading {}: {e}", asset.name))?;
        if read == 0 {
            break;
        }
        received += read as u64;
        if received > asset.size {
            return Err(format!(
                "{} is larger than {} bytes",
                asset.name, asset.size
            ));
        }
        hasher.update(&buffer[..read]);
        writer
            .write_all(&buffer[..read])
            .map_err(|e| format!("Failed while writing {}: {e}", asset.name))?;
        on_bytes(read as u64);
    }
    writer
        .flush()
        .map_err(|e| format!("Failed while writing {}: {e}", asset.name))?;
    if received != asset.size {
        return Err(format!(
            "{} is incomplete: {received} of {} bytes",
            asset.name, asset.size
        ));
    }
    let digest = hex::encode(hasher.finalize());
    if digest != asset.sha256 {
        return Err(format!(
            "{} does not match its pinned SHA-256 (got {digest})",
            asset.name
        ));
    }
    Ok(())
}

/// Files the server needs from a release archive: the server executable and
/// every DLL at the archive root. The other tools (llama-cli, ...) stay out.
fn is_server_file(name: &str) -> bool {
    !name.contains(['/', '\\'])
        && (name == server_binary_name() || name.to_ascii_lowercase().ends_with(".dll"))
}

/// Extract the [`is_server_file`] entries of `archive` into `dest`.
fn extract_server_files<R: Read + Seek>(archive: R, dest: &Path) -> Result<Vec<String>, String> {
    let mut archive =
        zip::ZipArchive::new(archive).map_err(|e| format!("Not a valid zip archive: {e}"))?;
    let mut extracted = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("Cannot read archive entry {index}: {e}"))?;
        let name = entry
            .name()
            .map_err(|e| format!("Cannot read archive entry {index} name: {e}"))?
            .to_string();
        if !entry.is_file() || !is_server_file(&name) {
            continue;
        }
        let path = dest.join(&name);
        let mut file = std::fs::File::create(&path)
            .map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
        std::io::copy(&mut entry, &mut file).map_err(|e| format!("Cannot extract {name}: {e}"))?;
        extracted.push(name);
    }
    Ok(extracted)
}

/// Move the staged files into `target`, the server executable last: its
/// presence is what marks the install complete, so a failure halfway leaves
/// the server "missing" instead of a server without its DLLs.
fn move_into_place(staging: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target)
        .map_err(|e| format!("Cannot create {}: {e}", target.display()))?;
    let mut names: Vec<std::ffi::OsString> = std::fs::read_dir(staging)
        .map_err(|e| format!("Cannot list {}: {e}", staging.display()))?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("Cannot list {}: {e}", staging.display()))?;
    names.sort_by_key(|name| name == server_binary_name());
    for name in names {
        let to = target.join(&name);
        std::fs::rename(staging.join(&name), &to)
            .map_err(|e| format!("Cannot move {} into place: {e}", to.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSET: ReleaseAsset = ReleaseAsset {
        name: "test.zip",
        size: 5,
        // SHA-256 of "hello".
        sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    };

    fn copy(bytes: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        copy_verified(&mut &bytes[..], &mut out, &ASSET, |_| {})?;
        Ok(out)
    }

    #[test]
    fn only_the_pinned_bytes_are_accepted() {
        assert_eq!(copy(b"hello").unwrap(), b"hello");
        assert!(copy(b"hellO").unwrap_err().contains("SHA-256"));
        assert!(copy(b"hell").unwrap_err().contains("incomplete"));
        assert!(copy(b"hello!").unwrap_err().contains("larger"));
    }

    #[test]
    fn extraction_keeps_the_server_and_root_dlls_only() {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default();
            for name in [
                server_binary_name(),
                "ggml-cuda.dll",
                "cublas64_12.DLL",
                "llama-cli.exe",
                "nested/evil.dll",
                "../escape.dll",
            ] {
                zip.start_file(name, options).unwrap();
                zip.write_all(name.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        buffer.rewind().unwrap();
        let dest = tempfile::tempdir().unwrap();

        let mut extracted = extract_server_files(buffer, dest.path()).unwrap();
        extracted.sort();
        let mut expected = vec![server_binary_name(), "cublas64_12.DLL", "ggml-cuda.dll"];
        expected.sort();
        assert_eq!(extracted, expected);
        assert!(!dest.path().join("llama-cli.exe").exists());
        assert!(!dest.path().parent().unwrap().join("escape.dll").exists());
    }

    #[test]
    fn the_server_executable_is_moved_last() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        let target = dir.path().join("tools/llama-server");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        for name in [server_binary_name(), "a.dll", "z.dll"] {
            std::fs::write(staging.join(name), name).unwrap();
        }
        // A file in the way of the last DLL makes the move fail midway.
        std::fs::create_dir(target.join("z.dll")).unwrap();
        std::fs::write(target.join("z.dll").join("x"), b"x").unwrap();

        assert!(move_into_place(&staging, &target).is_err());
        assert!(!target.join(server_binary_name()).exists());

        std::fs::remove_dir_all(target.join("z.dll")).unwrap();
        move_into_place(&staging, &target).unwrap();
        assert!(target.join(server_binary_name()).is_file());
        assert!(target.join("z.dll").is_file());
    }
}
