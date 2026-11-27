//! Managed download of the `llama-server` build behind local GLM-OCR and
//! local Qwen correction, into `<data>/tools/llama-server`.
//!
//! The app downloads llama.cpp's official release of the exact revision it
//! links (tag `b11429` = d8123504, the sources llama-cpp-sys-2 0.1.161
//! vendors), in the build this machine's GPU runs ([`super::llama_gpu`]):
//! CUDA for NVIDIA on Windows, HIP for supported Radeons, Vulkan for other
//! AMD and Intel GPUs, Metal on Apple Silicon. Every archive is checked
//! against its pinned size and SHA-256, and only `llama-server` with its
//! libraries is kept. The Linux `.deb` still bundles its CUDA build for
//! NVIDIA (scripts/fetch-llama-server.sh); installers ship no other build,
//! since each would carry hundreds of MB for the machines that can use it.
//!
//! The install records its build in [`BACKEND_MARKER`]. A HIP build that
//! lists no ROCm device after install (a card the table misnames, or Linux
//! without a working ROCm) is replaced by the Vulkan build.

use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use super::llama_gpu::{self, Backend, Platform};
use super::local_server::{
    install_serves, installed_backend, installed_revision, list_devices, marker_text,
    server_binary_name, server_dir, server_source, ServerSource, BACKEND_MARKER,
};
use crate::llm::download::{
    build_model_download_client, ModelDownloadCompletePayload, ModelDownloadErrorPayload,
    ModelDownloadProgressPayload,
};

/// Prefix of the emitted events (`llama-server:download_progress`, ...).
const EVENT_SCOPE: &str = "llama-server";
/// llama.cpp release tag of the revision the app links.
const RELEASE_TAG: &str = super::local_server::SERVER_REVISION;
const DOWNLOAD_CHUNK_SIZE: usize = 64 * 1024;

/// One archive of the pinned release.
struct ReleaseAsset {
    name: &'static str,
    size: u64,
    sha256: &'static str,
}

/// The CUDA 12.4 build and NVIDIA's runtime (cudart, cuBLAS) for it. CUDA 12
/// rather than 13: it runs on older drivers and on pre-Turing cards.
const WINDOWS_CUDA: &[ReleaseAsset] = &[
    ReleaseAsset {
        name: "llama-b11429-bin-win-cuda-12.4-x64.zip",
        size: 264_478_423,
        sha256: "dd6df685c1024e6aa55ca55ad35038692d5e284d4dc895e7a646c3245d3d14ff",
    },
    ReleaseAsset {
        name: "cudart-llama-bin-win-cuda-12.4-x64.zip",
        size: 391_443_627,
        sha256: "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6",
    },
];
const WINDOWS_VULKAN: &[ReleaseAsset] = &[ReleaseAsset {
    name: "llama-b11429-bin-win-vulkan-x64.zip",
    size: 33_337_769,
    sha256: "1bfe78ad9168b79fa02bf67f6af9f5e17a966d824d77238517f7bef12ac73b36",
}];
/// Links the system's ROCm 10.0 runtime, which it does not ship. With an
/// older ROCm it lists no device, and the install falls back to Vulkan.
const LINUX_HIP: &[ReleaseAsset] = &[ReleaseAsset {
    name: "llama-b11429-bin-ubuntu-rocm-10.0-x64.tar.gz",
    size: 243_892_368,
    sha256: "bd73e52146551f07a4c539bbe9da42f3b680c44ddc252fce61ab86d567a5932a",
}];
const LINUX_VULKAN: &[ReleaseAsset] = &[ReleaseAsset {
    name: "llama-b11429-bin-ubuntu-vulkan-x64.tar.gz",
    size: 31_636_673,
    sha256: "632c4e98feba2b94407a2130e3133e0c3aefb0ea1ab41337e926d8bfafdd0b74",
}];
const MAC_METAL: &[ReleaseAsset] = &[ReleaseAsset {
    name: "llama-b11429-bin-macos-arm64.tar.gz",
    size: 11_971_406,
    sha256: "740288ec6887be94280a5dfa25b5e23a78285cab104519e6c7e218904ee82459",
}];

/// The archives of `backend`'s build on `platform`; empty when the release
/// has none to download there (Linux CUDA comes with the `.deb`).
fn assets_for(platform: Platform, backend: Backend) -> &'static [ReleaseAsset] {
    match (platform, backend) {
        (Platform::Windows, Backend::Cuda) => WINDOWS_CUDA,
        (Platform::Windows, Backend::Vulkan) => WINDOWS_VULKAN,
        (Platform::Linux, Backend::Hip) => LINUX_HIP,
        (Platform::Linux, Backend::Vulkan) => LINUX_VULKAN,
        (Platform::MacArm, Backend::Metal) => MAC_METAL,
        _ => &[],
    }
}

/// Download size in bytes of `backend`'s build on this platform.
pub(crate) fn download_size(backend: Backend) -> u64 {
    assets_for(Platform::current(), backend)
        .iter()
        .map(|asset| asset.size)
        .sum()
}

/// One download at a time: the GLM-OCR card, the Qwen card and the
/// assistant's `models_setup` all start the same install.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Queue the server download in the background. Emits
/// `llama-server:download_progress`, `llama-server:download_complete` and
/// `llama-server:download_error`.
pub(crate) fn start_download(db_path: &Path, app_handle: AppHandle) -> Result<String, String> {
    let gpu = match server_source() {
        ServerSource::Download => llama_gpu::detected()
            .ok_or_else(|| "No se detectó la GPU de este equipo.".to_string())?,
        ServerSource::Bundled => {
            return Err(
                "En este sistema llama-server viene con el instalador; no se descarga aparte."
                    .to_string(),
            )
        }
        ServerSource::RequiresGpu => return Err(requires_gpu_message()),
    };
    let revision = installed_revision(db_path);
    if installed_backend(db_path)
        .is_some_and(|installed| install_serves(installed, revision.as_deref(), gpu.backend))
    {
        return Ok("already complete".to_string());
    }
    if RUNNING.swap(true, Ordering::AcqRel) {
        return Ok("already running".to_string());
    }

    crate::app_logs::info(
        &app_handle,
        "llama-server/download",
        format!(
            "Inicio descarga de llama-server ({RELEASE_TAG}, {})",
            gpu.backend.as_str()
        ),
    );
    let db_path = db_path.to_path_buf();
    tauri::async_runtime::spawn(async move {
        let worker_handle = app_handle.clone();
        let result = tokio::task::spawn_blocking(move || {
            install_with_fallback(&db_path, gpu.backend, &worker_handle)
        })
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

/// Why no build is offered on a machine without a usable GPU.
pub(crate) fn requires_gpu_message() -> String {
    format!(
        "llama-server solo se descarga en equipos con GPU NVIDIA, o AMD, Intel o Apple con al menos {} GB de memoria de video: GLM-OCR y Qwen locales no corren en CPU.",
        llama_gpu::Engine::GlmOcr.min_vram_gb()
    )
}

/// Install `backend`'s build; a HIP build that sees no ROCm device is
/// replaced by the Vulkan build of the same platform.
fn install_with_fallback(
    db_path: &Path,
    backend: Backend,
    app_handle: &AppHandle,
) -> Result<PathBuf, String> {
    match install(db_path, backend, app_handle) {
        Err(InstallError::NoDevice(listing)) if backend == Backend::Hip => {
            crate::app_logs::info(
                app_handle,
                "llama-server/download",
                format!("El build HIP no ve ninguna GPU ROCm; se instala Vulkan. {listing}"),
            );
            install(db_path, Backend::Vulkan, app_handle).map_err(InstallError::into_message)
        }
        result => result.map_err(InstallError::into_message),
    }
}

#[derive(Debug)]
enum InstallError {
    /// The staged build lists no device of its kind; carries the listing.
    NoDevice(String),
    Other(String),
}

impl InstallError {
    fn into_message(self) -> String {
        match self {
            InstallError::NoDevice(listing) => format!(
                "llama-server no encontró la GPU (¿falta actualizar el driver de video?). {listing}"
            ),
            InstallError::Other(message) => message,
        }
    }
}

impl From<String> for InstallError {
    fn from(message: String) -> Self {
        InstallError::Other(message)
    }
}

/// Download, verify and extract every archive into a work directory next to
/// the managed one, check the build sees its GPU, then replace the managed
/// install with it. The work directory is removed whatever happens.
fn install(
    db_path: &Path,
    backend: Backend,
    app_handle: &AppHandle,
) -> Result<PathBuf, InstallError> {
    let assets = assets_for(Platform::current(), backend);
    if assets.is_empty() {
        return Err(InstallError::Other(format!(
            "No hay build {} de llama-server para este sistema",
            backend.as_str()
        )));
    }
    let target = server_dir(db_path);
    let work = target.with_file_name("llama-server.partial");
    // Left behind by an attempt the app did not survive.
    let _ = std::fs::remove_dir_all(&work);
    let result = download_and_stage(&work, assets, app_handle)
        .map_err(InstallError::Other)
        .and_then(|staging| finish_install(&staging, &target, backend));
    let _ = std::fs::remove_dir_all(&work);
    result.map(|()| target.join(server_binary_name()))
}

/// Check the staged build, record which one it is and put it in place.
fn finish_install(staging: &Path, target: &Path, backend: Backend) -> Result<(), InstallError> {
    check_sees_gpu(staging, backend)?;
    std::fs::write(staging.join(BACKEND_MARKER), marker_text(backend))
        .map_err(|e| format!("Cannot record the build: {e}"))?;
    replace_install(staging, target)?;
    Ok(())
}

/// A Vulkan or HIP build that lists no device of its kind would run on the
/// CPU, which neither engine can afford. CUDA and Metal are not probed: CUDA
/// is offered because `nvidia-smi` saw a card, Metal is the Mac's own GPU.
fn check_sees_gpu(staging: &Path, backend: Backend) -> Result<(), InstallError> {
    if !matches!(backend, Backend::Vulkan | Backend::Hip) {
        return Ok(());
    }
    let listing = list_devices(&staging.join(server_binary_name()))?;
    if llama_gpu::lists_device(&listing, backend) {
        Ok(())
    } else {
        Err(InstallError::NoDevice(device_lines(&listing)))
    }
}

/// The `Available devices:` part of a listing, for the log.
fn device_lines(listing: &str) -> String {
    listing
        .lines()
        .skip_while(|line| !line.starts_with("Available devices"))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replace whatever the managed directory holds with the staged build: a
/// CUDA install left in place would leave its `ggml-cuda.dll` for a Vulkan
/// server to load. A running server holds its files, so it stops first.
/// The directory itself stays: Windows refuses to delete a directory that
/// Explorer or an antivirus scan has open, while its entries still go.
fn replace_install(staging: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        super::local_server::shutdown();
        clear_dir(target).map_err(|e| format!("Cannot remove the previous llama-server: {e}"))?;
    }
    move_into_place(staging, target)
}

fn clear_dir(dir: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn download_and_stage(
    work: &Path,
    assets: &[ReleaseAsset],
    app_handle: &AppHandle,
) -> Result<PathBuf, String> {
    let staging = work.join("staging");
    std::fs::create_dir_all(&staging)
        .map_err(|e| format!("Cannot create {}: {e}", staging.display()))?;
    let client = build_model_download_client()?;
    let total: u64 = assets.iter().map(|asset| asset.size).sum();
    crate::llm::download::ensure_room_in(work, required_room(assets))?;
    let mut done = 0_u64;
    let mut last_pct = 0_u8;

    for asset in assets {
        let _turn = crate::llm::download::wait_for_download_turn(|| {
            crate::llm::download::log_waiting_for_turn(app_handle, "llama_server/download")
        });
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
        let mut archive = create_archive_file(&archive_path)
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
        let extracted = if asset.name.ends_with(".zip") {
            extract_server_files(archive, &staging).map(drop)
        } else {
            drop(archive);
            extract_tarball(&archive_path, work, &staging)
        };
        extracted.map_err(|e| format!("{}: {e}", asset.name))?;
        let _ = std::fs::remove_file(&archive_path);
    }
    if !staging.join(server_binary_name()).is_file() {
        return Err(format!("The release has no {}", server_binary_name()));
    }
    Ok(staging)
}

/// Disk a release install needs: each archive plus what it extracts to, which
/// sit side by side until the archive is removed. Twice the archives covers
/// the extracted files, which are never smaller than their archive.
fn required_room(assets: &[ReleaseAsset]) -> u64 {
    assets
        .iter()
        .map(|asset| asset.size)
        .sum::<u64>()
        .saturating_mul(2)
}

/// The file one release archive downloads into. It is written, then read back
/// through the same handle to extract it, so it opens for both: a handle from
/// `File::create` is write-only, and Windows refuses the read with "access
/// denied" (os error 5).
fn create_archive_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
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

/// Folders of ROCm kernel libraries the HIP build loads next to its DLLs.
const LIBRARY_DIRS: &[&str] = &["rocblas", "hipblaslt"];

/// Files the server needs from a release archive, by their path inside it
/// (`/`-separated, relative to the build's root): the server executable,
/// every shared library at the root (`.dll`, `.so`, `.so.N`, `.dylib`), and
/// the HIP build's [`LIBRARY_DIRS`]. The other tools (llama-cli, ...) stay out.
fn is_server_file(name: &str) -> bool {
    if name.contains('\\') || name.split('/').any(|part| part.is_empty() || part == "..") {
        return false;
    }
    match name.split_once('/') {
        None => {
            let lower = name.to_ascii_lowercase();
            name == server_binary_name()
                || lower.ends_with(".dll")
                || lower.ends_with(".dylib")
                || lower.ends_with(".so")
                || lower.contains(".so.")
        }
        Some((dir, _)) => LIBRARY_DIRS.contains(&dir),
    }
}

/// Unpack a `.tar.gz` release (Linux, macOS) with the system's `tar`, which
/// keeps the libraries' symlinks and executable bits, then move its
/// [`is_server_file`] entries into `dest`. The archive is already verified.
fn extract_tarball(archive: &Path, work: &Path, dest: &Path) -> Result<(), String> {
    let unpacked = work.join("unpacked");
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked)
        .map_err(|e| format!("Cannot create {}: {e}", unpacked.display()))?;
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(&unpacked)
        .status()
        .map_err(|e| format!("Cannot run tar: {e}"))?;
    if !status.success() {
        return Err(format!("tar failed ({status})"));
    }
    let root = release_root(&unpacked)?;
    move_server_files(&root, dest)?;
    let _ = std::fs::remove_dir_all(&unpacked);
    let _ = std::fs::remove_file(archive);
    Ok(())
}

/// The directory holding the server: the archive's single top folder
/// (`llama-b11429/`), or the unpack directory itself when it is flat.
fn release_root(unpacked: &Path) -> Result<PathBuf, String> {
    if unpacked.join(server_binary_name()).exists() {
        return Ok(unpacked.to_path_buf());
    }
    std::fs::read_dir(unpacked)
        .map_err(|e| format!("Cannot list {}: {e}", unpacked.display()))?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join(server_binary_name()).is_file())
        .ok_or_else(|| format!("The release has no {}", server_binary_name()))
}

/// Move the [`is_server_file`] entries at `root`'s top level into `dest`.
/// Renaming keeps a symlink a symlink.
fn move_server_files(root: &Path, dest: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(root)
        .map_err(|e| format!("Cannot list {}: {e}", root.display()))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        let keep = if entry.path().is_dir() && !entry.path().is_symlink() {
            LIBRARY_DIRS.contains(&name.as_str())
        } else {
            is_server_file(&name)
        };
        if keep {
            std::fs::rename(entry.path(), dest.join(&name))
                .map_err(|e| format!("Cannot move {name}: {e}"))?;
        }
    }
    Ok(())
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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
        }
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

    #[test]
    fn an_install_needs_room_for_the_archives_and_what_they_extract_to() {
        let other = ReleaseAsset {
            name: "other.zip",
            size: 7,
            sha256: ASSET.sha256,
        };

        assert_eq!(required_room(&[ASSET, other]), 24);
    }

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

    /// The download path end to end on a real file: written and verified,
    /// rewound, then extracted through the same handle. With a write-only
    /// handle Windows refuses the read ("Acceso denegado, os error 5"), which
    /// failed every llama-server install on Windows in 1.8.5.
    #[test]
    fn a_downloaded_archive_is_read_back_from_its_own_file() {
        let mut zip_bytes = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut zip_bytes);
            zip.start_file(
                server_binary_name(),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"server").unwrap();
            zip.finish().unwrap();
        }
        let zip_bytes = zip_bytes.into_inner();
        let digest = hex::encode(Sha256::digest(&zip_bytes));
        let asset = ReleaseAsset {
            name: "llama-test.zip",
            size: zip_bytes.len() as u64,
            sha256: Box::leak(digest.into_boxed_str()),
        };
        let work = tempfile::tempdir().unwrap();
        let staging = work.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();

        let mut archive = create_archive_file(&work.path().join(asset.name)).unwrap();
        let mut buffered = std::io::BufWriter::new(&mut archive);
        copy_verified(&mut &zip_bytes[..], &mut buffered, &asset, |_| {}).unwrap();
        drop(buffered);
        archive.rewind().unwrap();
        let extracted = extract_server_files(archive, &staging).unwrap();

        assert_eq!(extracted, vec![server_binary_name()]);
        assert_eq!(
            std::fs::read(staging.join(server_binary_name())).unwrap(),
            b"server"
        );
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

    #[test]
    fn every_gpu_build_has_its_pinned_archives() {
        use Backend::*;
        let names = |platform, backend| -> Vec<&str> {
            assets_for(platform, backend)
                .iter()
                .map(|a| a.name)
                .collect()
        };
        assert_eq!(
            names(Platform::Windows, Cuda),
            [
                "llama-b11429-bin-win-cuda-12.4-x64.zip",
                "cudart-llama-bin-win-cuda-12.4-x64.zip"
            ]
        );
        assert_eq!(
            names(Platform::Windows, Vulkan),
            ["llama-b11429-bin-win-vulkan-x64.zip"]
        );
        assert_eq!(
            names(Platform::Linux, Hip),
            ["llama-b11429-bin-ubuntu-rocm-10.0-x64.tar.gz"]
        );
        assert_eq!(
            names(Platform::Linux, Vulkan),
            ["llama-b11429-bin-ubuntu-vulkan-x64.tar.gz"]
        );
        assert_eq!(
            names(Platform::MacArm, Metal),
            ["llama-b11429-bin-macos-arm64.tar.gz"]
        );
        // The .deb bundles Linux CUDA; nothing else downloads elsewhere.
        assert!(assets_for(Platform::Linux, Cuda).is_empty());
        // Windows Radeons run Vulkan: the ROCm archive lacks rocBLAS.
        assert!(assets_for(Platform::Windows, Hip).is_empty());
        assert!(assets_for(Platform::Other, Vulkan).is_empty());
        for backend in [Cuda, Hip, Vulkan, Metal] {
            for platform in [Platform::Windows, Platform::Linux, Platform::MacArm] {
                for asset in assets_for(platform, backend) {
                    assert!(asset.name.contains(RELEASE_TAG) || asset.name.starts_with("cudart"));
                    assert_eq!(asset.sha256.len(), 64, "{}", asset.name);
                    assert!(asset.size > 0);
                }
            }
        }
    }

    #[test]
    fn the_hip_build_keeps_its_rocm_kernel_folders() {
        assert!(is_server_file("rocblas/library/TensileLibrary_gfx1100.dat"));
        assert!(is_server_file(
            "hipblaslt/library/Kernels.so-000-gfx1201.hsaco"
        ));
        assert!(is_server_file("rocblas.dll"));
        assert!(!is_server_file("rocblas/../../escape.dll"));
        assert!(!is_server_file("other/lib.dll"));
        assert!(!is_server_file("/rocblas/x"));
    }

    #[test]
    fn unix_builds_keep_their_shared_libraries() {
        for name in [
            "libggml-vulkan.so",
            "libllama.so.0",
            "libllama.so.0.0.8783",
            "libggml-metal.0.dylib",
        ] {
            assert!(is_server_file(name), "{name}");
        }
        for name in ["llama-cli", "LICENSE", "rpc-server"] {
            assert!(!is_server_file(name), "{name}");
        }
    }

    #[test]
    fn a_zip_with_kernel_folders_extracts_them_in_place() {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default();
            for name in [
                server_binary_name(),
                "ggml-hip.dll",
                "rocblas/library/a.dat",
            ] {
                zip.start_file(name, options).unwrap();
                zip.write_all(name.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        buffer.rewind().unwrap();
        let dest = tempfile::tempdir().unwrap();
        extract_server_files(buffer, dest.path()).unwrap();
        assert_eq!(
            std::fs::read(dest.path().join("rocblas/library/a.dat")).unwrap(),
            b"rocblas/library/a.dat"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_tarball_release_keeps_the_server_its_libraries_and_symlinks() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("src/llama-b11429");
        std::fs::create_dir_all(&source).unwrap();
        for name in [
            server_binary_name(),
            "libllama.so.0.0.8783",
            "llama-cli",
            "LICENSE",
        ] {
            std::fs::write(source.join(name), name).unwrap();
        }
        std::os::unix::fs::symlink("libllama.so.0.0.8783", source.join("libllama.so.0")).unwrap();
        let archive = work.path().join("release.tar.gz");
        let status = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(work.path().join("src"))
            .arg("llama-b11429")
            .status()
            .unwrap();
        assert!(status.success());
        let staging = work.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();

        extract_tarball(&archive, work.path(), &staging).unwrap();

        assert!(staging.join(server_binary_name()).is_file());
        assert!(staging.join("libllama.so.0").is_symlink());
        assert!(!staging.join("llama-cli").exists());
        assert!(!staging.join("LICENSE").exists());
    }

    #[test]
    fn replacing_an_install_drops_the_previous_builds_files() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        let target = dir.path().join("tools/llama-server");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("ggml-cuda.dll"), b"cuda").unwrap();
        std::fs::write(target.join(server_binary_name()), b"old").unwrap();
        std::fs::write(staging.join("ggml-vulkan.dll"), b"vk").unwrap();
        std::fs::write(staging.join(BACKEND_MARKER), "vulkan").unwrap();
        std::fs::write(staging.join(server_binary_name()), b"new").unwrap();

        replace_install(&staging, &target).unwrap();

        assert!(!target.join("ggml-cuda.dll").exists());
        assert_eq!(
            std::fs::read(target.join(server_binary_name())).unwrap(),
            b"new"
        );
        assert_eq!(
            std::fs::read_to_string(target.join(BACKEND_MARKER)).unwrap(),
            "vulkan"
        );
    }

    /// Explorer or an antivirus scan holding the managed directory open must
    /// not fail the install: Windows refuses to delete an open directory.
    #[cfg(windows)]
    #[test]
    fn replacing_an_install_works_while_the_directory_is_held_open() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        // FILE_SHARE_READ | FILE_SHARE_WRITE, without FILE_SHARE_DELETE.
        const SHARE_READ_WRITE: u32 = 0x1 | 0x2;

        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        let target = dir.path().join("tools/llama-server");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("ggml-cuda.dll"), b"cuda").unwrap();
        std::fs::write(staging.join(server_binary_name()), b"new").unwrap();
        let _held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(SHARE_READ_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&target)
            .unwrap();

        replace_install(&staging, &target).unwrap();

        assert!(!target.join("ggml-cuda.dll").exists());
        assert_eq!(
            std::fs::read(target.join(server_binary_name())).unwrap(),
            b"new"
        );
    }

    #[test]
    fn the_log_keeps_only_the_device_list() {
        let listing = "load_backend: loaded CPU backend\nAvailable devices:\n  Vulkan0: RX 560 (4096 MiB, 3463 MiB free)\n";
        assert_eq!(
            device_lines(listing),
            "Available devices: Vulkan0: RX 560 (4096 MiB, 3463 MiB free)"
        );
    }
}
