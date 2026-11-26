use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

/// Progress payload for managed GGUF downloads. The `download_model_file`
/// `event_scope` decides the event name (`{scope}:download_progress`); the
/// JSON shape is the same for every model kind.
#[derive(Clone, serde::Serialize)]
pub struct ModelDownloadProgressPayload {
    pub pct: u8,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

/// Completion payload for managed GGUF downloads (`{scope}:download_complete`).
#[derive(Clone, serde::Serialize)]
pub struct ModelDownloadCompletePayload {
    pub path: String,
}

/// Error payload for managed GGUF downloads (`{scope}:download_error`).
/// Emitted by the download callers, which own the failure context.
#[derive(Clone, serde::Serialize)]
pub struct ModelDownloadErrorPayload {
    pub error: String,
}

const DOWNLOAD_CHUNK_SIZE: usize = 64 * 1024;
/// Disk writes are batched to this size (R8).
const DOWNLOAD_WRITE_BUFFER: usize = 1 << 20;
const DOWNLOAD_TIMEOUT_SECS: u64 = 600;

pub(crate) fn open_managed_models_child(
    parent: &cap_std::fs::Dir,
    child_name: &Path,
) -> Result<cap_std::fs::Dir, String> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsMaybeDirExt as _};

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
    let parent = cap_std::fs::Dir::open_ambient_dir(parent_path, cap_std::ambient_authority())
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

/// Free space a model download leaves on top of the file itself, so it never
/// fills the disk the archive and the app also live on.
const DOWNLOAD_FREE_SPACE_MARGIN: u64 = 512 * 1024 * 1024;

/// Refuses a download the disk cannot hold. With the free space or the size
/// unknown there is nothing to compare, and the download goes ahead.
fn ensure_room_for_download(
    free_bytes: Option<u64>,
    needed_bytes: Option<u64>,
) -> Result<(), String> {
    let (Some(free), Some(needed)) = (free_bytes, needed_bytes) else {
        return Ok(());
    };
    let required = needed.saturating_add(DOWNLOAD_FREE_SPACE_MARGIN);
    if free >= required {
        return Ok(());
    }
    Err(format!(
        "Espacio en disco insuficiente para descargar el modelo: se requieren ~{:.1} GB libres y hay {:.1} GB. Liberá espacio y volvé a intentar.",
        required as f64 / 1_000_000_000.0,
        free as f64 / 1_000_000_000.0
    ))
}

/// Refuses a download of `needed_bytes` into `dir` when the disk that holds
/// `dir` cannot take it. For downloaders outside [`download_model_file`] that
/// know the size up front: checking before the first byte avoids a download
/// that dies at 90% with a full disk.
#[cfg(feature = "local-ml")]
pub(crate) fn ensure_room_in(dir: &Path, needed_bytes: u64) -> Result<(), String> {
    ensure_room_for_download(fs2::available_space(dir).ok(), Some(needed_bytes))
}

/// Refuses a download that ended before the size the server announced.
fn ensure_download_complete(
    received_bytes: u64,
    expected_bytes: Option<u64>,
) -> Result<(), String> {
    match expected_bytes {
        Some(expected) if received_bytes != expected => Err(format!(
            "La descarga del modelo quedó incompleta: llegaron {:.1} MB de {:.1} MB. Volvé a intentar.",
            received_bytes as f64 / 1_000_000.0,
            expected as f64 / 1_000_000.0
        )),
        _ => Ok(()),
    }
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

/// Why one download attempt failed: a cut connection or a busy server is
/// worth another attempt, a refusal or a full disk is not.
#[derive(Debug, PartialEq)]
enum AttemptError {
    Transient(String),
    Final(String),
}

/// Attempts a model download gets in all, the first one included.
const DOWNLOAD_ATTEMPTS: u32 = 3;

/// Wait before the attempt that follows failed attempt number `failed`
/// (1-based), or `None` when no attempt is left.
fn retry_delay(failed: u32) -> Option<Duration> {
    if failed >= DOWNLOAD_ATTEMPTS {
        return None;
    }
    // 2 s, 8 s: long enough for a dropped Wi-Fi or a busy CDN node to come
    // back, short enough that the progress bar does not look dead.
    Some(Duration::from_secs(2 * 4_u64.pow(failed.saturating_sub(1))))
}

/// A status a server returns when it is busy or failing for a moment.
fn status_is_transient(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
}

/// Parses `bytes START-END/TOTAL` into its start and, when known, the full size.
fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let range = value.trim().strip_prefix("bytes ")?;
    let (span, total) = range.split_once('/')?;
    let (start, _end) = span.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let total = match total.trim() {
        "*" => None,
        total => Some(total.parse().ok()?),
    };
    Some((start, total))
}

/// Where the body of a response goes in the temporary file.
#[derive(Debug, PartialEq)]
struct ResumeStart {
    /// Bytes already in the file that the body continues; 0 starts it again.
    offset: u64,
    /// Size of the whole model, when the server says.
    total_bytes: Option<u64>,
}

/// Reads how a server answered a request for the bytes from `requested_offset`.
/// A 200 sends the whole file, so it starts again; a 206 must continue at
/// exactly that byte, or the parts would not join.
fn resume_start(
    requested_offset: u64,
    status: reqwest::StatusCode,
    content_range: Option<&str>,
    content_length: Option<u64>,
) -> Result<ResumeStart, String> {
    if status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Ok(ResumeStart {
            offset: 0,
            total_bytes: content_length,
        });
    }
    match content_range.and_then(parse_content_range) {
        Some((start, total)) if start == requested_offset => Ok(ResumeStart {
            offset: start,
            total_bytes: total.or_else(|| content_length.map(|len| start.saturating_add(len))),
        }),
        _ => Err(format!(
            "El servidor no continuó la descarga del modelo desde el byte {requested_offset}."
        )),
    }
}

/// Runs `attempt` until it succeeds, fails for good, or runs out of attempts,
/// sleeping [`retry_delay`] between them. `on_retry` hears each failure that
/// gets another attempt.
fn run_with_retries<T>(
    mut attempt: impl FnMut(u32) -> Result<T, AttemptError>,
    mut sleep: impl FnMut(Duration),
    mut on_retry: impl FnMut(u32, &str, Duration),
) -> Result<T, String> {
    let mut number = 1;
    loop {
        match attempt(number) {
            Ok(value) => return Ok(value),
            Err(AttemptError::Final(error)) => return Err(error),
            Err(AttemptError::Transient(error)) => {
                let Some(delay) = retry_delay(number) else {
                    return Err(error);
                };
                on_retry(number, &error, delay);
                sleep(delay);
                number += 1;
            }
        }
    }
}

/// Download a GGUF model into an already-opened managed directory capability,
/// emitting progress events via the Tauri event bus.
///
/// A cut connection or a busy server gets [`DOWNLOAD_ATTEMPTS`] attempts in
/// all; each one after the first asks for the bytes the temporary file does
/// not have yet, so a cut at 90% does not start the model over.
///
/// `event_scope` prefixes the emitted events (`{scope}:download_progress`,
/// `{scope}:download_complete`); the LLM flow passes `"llm"`, other model
/// downloads pass their own scope and define matching payload structs.
pub fn download_model_file(
    event_scope: &str,
    url: &str,
    managed_dir: cap_std::fs::Dir,
    filename: &str,
    display_destination: &Path,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let client = build_model_download_client()?;
    let mut temporary = ManagedTemporaryFile::create(&managed_dir, filename)?;
    let mut progress = DownloadProgress::new(|pct, downloaded_bytes, total_bytes| {
        let _ = app_handle.emit(
            &format!("{event_scope}:download_progress"),
            ModelDownloadProgressPayload {
                pct,
                downloaded_bytes,
                total_bytes,
            },
        );
        crate::app_logs::info(
            app_handle,
            format!("{event_scope}/download"),
            format!("Descarga de modelo local {pct}%"),
        );
    });
    let space_dir = display_destination.parent();

    run_with_retries(
        |_| download_attempt(&client, url, temporary.writer(), space_dir, &mut progress),
        std::thread::sleep,
        |attempt, error, delay| {
            crate::app_logs::warn(
                app_handle,
                format!("{event_scope}/download"),
                format!(
                    "Falló el intento {attempt} de {DOWNLOAD_ATTEMPTS} de descargar el modelo; se reintenta en {} s: {error}",
                    delay.as_secs()
                ),
            );
        },
    )?;

    temporary.finish(filename)?;
    let display_path = display_destination.to_string_lossy().to_string();
    let _ = app_handle.emit(
        &format!("{event_scope}:download_complete"),
        ModelDownloadCompletePayload { path: display_path },
    );
    crate::app_logs::info(
        app_handle,
        format!("{event_scope}/download"),
        format!(
            "Descarga de modelo local completada: {}",
            display_destination.display()
        ),
    );
    Ok(())
}

/// Progress state that lives across the attempts of one download.
struct DownloadProgress<'a> {
    /// Hears every 5% step: percent, bytes in the file, size of the model.
    emit: Box<dyn FnMut(u8, u64, Option<u64>) + 'a>,
    last_reported_pct: u8,
    /// The strong ETag, or else the Last-Modified, of the last full answer. A
    /// resumed request sends it as `If-Range`: a model that changed on the
    /// server then comes whole, instead of half of each version in one file.
    validator: Option<String>,
}

impl<'a> DownloadProgress<'a> {
    fn new(emit: impl FnMut(u8, u64, Option<u64>) + 'a) -> Self {
        Self {
            emit: Box::new(emit),
            last_reported_pct: 0,
            validator: None,
        }
    }

    fn report(&mut self, downloaded_bytes: u64, total_bytes: Option<u64>) {
        let Some(total) = total_bytes.filter(|total| *total > 0) else {
            return;
        };
        let pct = ((downloaded_bytes.saturating_mul(100)) / total).min(100) as u8;
        if pct < self.last_reported_pct.saturating_add(5) && pct != 100 {
            return;
        }
        self.last_reported_pct = pct;
        (self.emit)(pct, downloaded_bytes, total_bytes);
    }
}

fn response_validator(response: &reqwest::blocking::Response) -> Option<String> {
    let header = |name| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    // `If-Range` takes only a strong ETag; a weak one would never match.
    header(reqwest::header::ETAG)
        .filter(|etag| !etag.starts_with("W/"))
        .or_else(|| header(reqwest::header::LAST_MODIFIED))
}

fn disk_error(error: std::io::Error) -> AttemptError {
    AttemptError::Final(format!("No se pudo guardar el modelo en el disco: {error}"))
}

/// One request for the bytes `file` is missing, written after the ones it
/// already has. `space_dir` is a folder on the disk that receives the model.
fn download_attempt(
    client: &reqwest::blocking::Client,
    url: &str,
    file: &mut cap_std::fs::File,
    space_dir: Option<&Path>,
    progress: &mut DownloadProgress<'_>,
) -> Result<(), AttemptError> {
    let on_disk = file.metadata().map_err(disk_error)?.len();
    let mut request = client.get(url);
    let requested_offset = match &progress.validator {
        Some(validator) if on_disk > 0 => {
            request = request
                .header(reqwest::header::RANGE, format!("bytes={on_disk}-"))
                .header(reqwest::header::IF_RANGE, validator);
            on_disk
        }
        _ => 0,
    };

    let mut response = request.send().map_err(|e| {
        AttemptError::Transient(format!("No se pudo iniciar la descarga del modelo: {e}"))
    })?;
    let status = response.status();
    if !status.is_success() {
        let error = format!(
            "El servidor rechazó la descarga del modelo (HTTP {status}). Volvé a intentar más tarde."
        );
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            // The file holds bytes the server does not have: start it again.
            file.set_len(0).map_err(disk_error)?;
            return Err(AttemptError::Transient(error));
        }
        return Err(if status_is_transient(status) {
            AttemptError::Transient(error)
        } else {
            AttemptError::Final(error)
        });
    }

    let content_range = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok());
    let start = match resume_start(
        requested_offset,
        status,
        content_range,
        response.content_length(),
    ) {
        Ok(start) => start,
        Err(error) => {
            file.set_len(0).map_err(disk_error)?;
            return Err(AttemptError::Transient(error));
        }
    };
    if start.offset == 0 {
        progress.validator = response_validator(&response);
        progress.last_reported_pct = 0;
        file.set_len(0).map_err(disk_error)?;
    }
    file.seek(SeekFrom::Start(start.offset))
        .map_err(disk_error)?;

    ensure_room_for_download(
        space_dir.and_then(|dir| fs2::available_space(dir).ok()),
        start
            .total_bytes
            .map(|total| total.saturating_sub(start.offset)),
    )
    .map_err(AttemptError::Final)?;

    let mut downloaded_bytes = start.offset;
    let mut buffer = vec![0_u8; DOWNLOAD_CHUNK_SIZE];
    // A blocking read returns about one TLS record (~16 KB); writing each
    // straight to disk is a syscall (and an antivirus hook) per record (R8).
    // Batch them into 1 MiB writes. A cut leaves a prefix of the model in the
    // file either way, which the next attempt continues.
    let mut writer = std::io::BufWriter::with_capacity(DOWNLOAD_WRITE_BUFFER, &mut *file);
    loop {
        let read = response.read(&mut buffer).map_err(|e| {
            AttemptError::Transient(format!(
                "Se cortó la descarga del modelo: {e}. Volvé a intentar."
            ))
        })?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read]).map_err(disk_error)?;
        downloaded_bytes += read as u64;
        progress.report(downloaded_bytes, start.total_bytes);
    }
    writer.flush().map_err(disk_error)?;
    drop(writer);

    ensure_download_complete(downloaded_bytes, start.total_bytes).map_err(AttemptError::Transient)
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
        let parent =
            cap_std::fs::Dir::open_ambient_dir(tmp.path(), cap_std::ambient_authority()).unwrap();

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
        let managed_dir =
            cap_std::fs::Dir::open_ambient_dir(&models_path, cap_std::ambient_authority()).unwrap();

        let persisted_models_path = match std::fs::rename(&models_path, &opened_models_path) {
            Ok(()) => {
                create_directory_alias(&outside, &models_path);
                opened_models_path.clone()
            }
            Err(error) => {
                #[cfg(not(windows))]
                panic!("unexpected failure while attempting directory swap: {error}");
                #[cfg(windows)]
                {
                    assert_eq!(
                        error.raw_os_error(),
                        Some(32),
                        "unexpected Windows error while attempting directory swap: {error}"
                    );
                    assert!(models_path.is_dir());
                    assert!(!opened_models_path.exists());
                    models_path.clone()
                }
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
        assert!(!persisted_models_path
            .join("gemma-model.download.tmp")
            .exists());
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

    const GB: u64 = 1_000_000_000;

    #[test]
    fn a_download_the_disk_cannot_hold_is_refused_before_it_starts() {
        let error = ensure_room_for_download(Some(2 * GB), Some(2_900_000_000)).unwrap_err();
        assert!(
            error.starts_with("Espacio en disco insuficiente para descargar el modelo"),
            "{error}"
        );
        assert!(
            error.contains("se requieren ~3.4 GB libres y hay 2.0 GB"),
            "{error}"
        );
        assert!(
            error.contains("Liberá espacio y volvé a intentar."),
            "{error}"
        );
    }

    #[test]
    fn the_free_space_margin_counts_against_the_download() {
        let size = 2 * GB;
        assert!(ensure_room_for_download(Some(size), Some(size)).is_err());
        assert!(
            ensure_room_for_download(Some(size + DOWNLOAD_FREE_SPACE_MARGIN), Some(size)).is_ok()
        );
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn a_folder_is_checked_against_the_free_space_of_its_disk() {
        let dir = tempfile::tempdir().expect("temp dir");

        assert!(ensure_room_in(dir.path(), 1).is_ok());
        let error = ensure_room_in(dir.path(), u64::MAX / 2).unwrap_err();
        assert!(
            error.starts_with("Espacio en disco insuficiente"),
            "{error}"
        );
    }

    #[test]
    fn an_unknown_free_space_or_size_lets_the_download_start() {
        assert!(ensure_room_for_download(None, Some(10 * GB)).is_ok());
        assert!(ensure_room_for_download(Some(1), None).is_ok());
    }

    #[test]
    fn a_download_shorter_than_announced_is_refused() {
        let error = ensure_download_complete(812_400_000, Some(906_400_000)).unwrap_err();
        assert_eq!(
            error,
            "La descarga del modelo quedó incompleta: llegaron 812.4 MB de 906.4 MB. Volvé a intentar."
        );
    }

    #[test]
    fn a_complete_or_unsized_download_is_accepted() {
        assert!(ensure_download_complete(906_400_000, Some(906_400_000)).is_ok());
        assert!(ensure_download_complete(5, None).is_ok());
    }

    #[test]
    fn a_failed_attempt_waits_longer_each_time_and_gives_up_after_three() {
        assert_eq!(retry_delay(1), Some(Duration::from_secs(2)));
        assert_eq!(retry_delay(2), Some(Duration::from_secs(8)));
        assert_eq!(retry_delay(3), None);
    }

    #[test]
    fn only_server_side_and_rate_limit_statuses_are_retried() {
        use reqwest::StatusCode;
        for status in [
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::REQUEST_TIMEOUT,
        ] {
            assert!(status_is_transient(status), "{status}");
        }
        for status in [
            StatusCode::NOT_FOUND,
            StatusCode::FORBIDDEN,
            StatusCode::UNAUTHORIZED,
            StatusCode::BAD_REQUEST,
        ] {
            assert!(!status_is_transient(status), "{status}");
        }
    }

    #[test]
    fn a_content_range_gives_the_start_and_the_full_size() {
        assert_eq!(
            parse_content_range("bytes 100-999/1000"),
            Some((100, Some(1000)))
        );
        assert_eq!(parse_content_range("bytes 100-999/*"), Some((100, None)));
        assert_eq!(parse_content_range("bytes */1000"), None);
        assert_eq!(parse_content_range("garbage"), None);
    }

    #[test]
    fn a_partial_answer_from_the_right_byte_appends_to_the_file() {
        let start = resume_start(
            100,
            reqwest::StatusCode::PARTIAL_CONTENT,
            Some("bytes 100-999/1000"),
            Some(900),
        )
        .unwrap();
        assert_eq!(
            start,
            ResumeStart {
                offset: 100,
                total_bytes: Some(1000)
            }
        );
    }

    #[test]
    fn a_full_answer_starts_the_file_again() {
        let start = resume_start(100, reqwest::StatusCode::OK, None, Some(1000)).unwrap();
        assert_eq!(
            start,
            ResumeStart {
                offset: 0,
                total_bytes: Some(1000)
            }
        );
    }

    #[test]
    fn a_partial_answer_from_another_byte_is_refused() {
        assert!(resume_start(
            100,
            reqwest::StatusCode::PARTIAL_CONTENT,
            Some("bytes 0-999/1000"),
            Some(1000),
        )
        .is_err());
        assert!(resume_start(100, reqwest::StatusCode::PARTIAL_CONTENT, None, Some(900)).is_err());
    }

    #[test]
    fn a_partial_answer_without_a_full_size_counts_from_the_offset() {
        let start = resume_start(
            100,
            reqwest::StatusCode::PARTIAL_CONTENT,
            Some("bytes 100-999/*"),
            Some(900),
        )
        .unwrap();
        assert_eq!(start.total_bytes, Some(1000));
    }

    fn run_scripted(
        outcomes: Vec<Result<u8, AttemptError>>,
    ) -> (Result<u8, String>, Vec<Duration>, Vec<u32>) {
        let mut outcomes = outcomes.into_iter();
        let mut sleeps = Vec::new();
        let mut retries = Vec::new();
        let result = run_with_retries(
            |_| outcomes.next().expect("no more scripted attempts"),
            |delay| sleeps.push(delay),
            |attempt, _error, _delay| retries.push(attempt),
        );
        (result, sleeps, retries)
    }

    #[test]
    fn a_cut_download_is_retried_with_growing_waits_until_it_works() {
        let (result, sleeps, retries) = run_scripted(vec![
            Err(AttemptError::Transient("cut".into())),
            Err(AttemptError::Transient("cut".into())),
            Ok(7),
        ]);
        assert_eq!(result, Ok(7));
        assert_eq!(sleeps, vec![Duration::from_secs(2), Duration::from_secs(8)]);
        assert_eq!(retries, vec![1, 2]);
    }

    #[test]
    fn a_download_that_keeps_failing_reports_the_last_error() {
        let (result, sleeps, _) = run_scripted(vec![
            Err(AttemptError::Transient("first".into())),
            Err(AttemptError::Transient("second".into())),
            Err(AttemptError::Transient("third".into())),
        ]);
        assert_eq!(result, Err("third".to_string()));
        assert_eq!(sleeps.len(), 2);
    }

    /// Serves one scripted answer per connection (head, then body bytes) and
    /// hands back the request heads it received.
    fn scripted_server(
        answers: Vec<(String, Vec<u8>)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/model.gguf", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (head, body) in answers {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    request.push_str(&line);
                }
                requests.push(request.to_ascii_lowercase());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
            requests
        });
        (url, handle)
    }

    fn model_bytes(len: usize, seed: u8) -> Vec<u8> {
        let mut bytes = b"GGUF".to_vec();
        bytes.extend((4..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)));
        bytes
    }

    fn download_scripted(url: &str) -> (Result<(), String>, Vec<u8>, Vec<u8>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let managed = cap_std::fs::Dir::open_ambient_dir(dir.path(), cap_std::ambient_authority())
            .expect("open dir");
        let mut file = managed.create("model.download.tmp").expect("create");
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let mut reported = Vec::new();
        let mut progress = DownloadProgress::new(|pct, _, _| reported.push(pct));
        let result = run_with_retries(
            |_| download_attempt(&client, url, &mut file, None, &mut progress),
            |_| {},
            |_, _, _| {},
        );
        drop(progress);
        drop(file);
        let written = std::fs::read(dir.path().join("model.download.tmp")).unwrap();
        (result, written, reported)
    }

    #[test]
    fn a_cut_download_continues_from_the_byte_it_reached() {
        let model = model_bytes(1000, 7);
        let (url, server) = scripted_server(vec![
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n"
                    .into(),
                model[..400].to_vec(),
            ),
            (
                "HTTP/1.1 206 Partial Content\r\nContent-Length: 600\r\nContent-Range: bytes 400-999/1000\r\nConnection: close\r\n\r\n"
                    .into(),
                model[400..].to_vec(),
            ),
        ]);

        let (result, written, reported) = download_scripted(&url);
        let requests = server.join().unwrap();

        assert_eq!(result, Ok(()));
        assert_eq!(written, model);
        assert!(!requests[0].contains("range:"), "{}", requests[0]);
        assert!(requests[1].contains("range: bytes=400-"), "{}", requests[1]);
        assert!(requests[1].contains("if-range: \"v1\""), "{}", requests[1]);
        assert_eq!(reported.last(), Some(&100));
    }

    #[test]
    fn a_model_that_changed_on_the_server_is_downloaded_whole() {
        let old = model_bytes(1000, 7);
        let new = model_bytes(800, 9);
        let (url, server) = scripted_server(vec![
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n"
                    .into(),
                old[..400].to_vec(),
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 800\r\nETag: \"v2\"\r\nConnection: close\r\n\r\n"
                    .into(),
                new.clone(),
            ),
        ]);

        let (result, written, _) = download_scripted(&url);
        server.join().unwrap();

        assert_eq!(result, Ok(()));
        assert_eq!(written, new);
    }

    #[test]
    fn a_download_without_a_validator_starts_over_instead_of_joining_parts() {
        let model = model_bytes(1000, 3);
        let (url, server) = scripted_server(vec![
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n".into(),
                model[..400].to_vec(),
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n".into(),
                model.clone(),
            ),
        ]);

        let (result, written, _) = download_scripted(&url);
        let requests = server.join().unwrap();

        assert_eq!(result, Ok(()));
        assert_eq!(written, model);
        assert!(!requests[1].contains("range:"), "{}", requests[1]);
    }

    #[test]
    fn a_refused_download_is_not_retried() {
        let (url, server) = scripted_server(vec![(
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            Vec::new(),
        )]);

        let (result, _, _) = download_scripted(&url);
        let requests = server.join().unwrap();

        assert_eq!(
            result,
            Err("El servidor rechazó la descarga del modelo (HTTP 404 Not Found). Volvé a intentar más tarde.".to_string())
        );
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn a_final_failure_is_not_retried() {
        let (result, sleeps, retries) =
            run_scripted(vec![Err(AttemptError::Final("disk full".into()))]);
        assert_eq!(result, Err("disk full".to_string()));
        assert!(sleeps.is_empty());
        assert!(retries.is_empty());
    }
}
