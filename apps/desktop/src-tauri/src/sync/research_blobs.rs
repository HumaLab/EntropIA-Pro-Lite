//! Inactive file/HTTP helpers for the optional content-addressed `report.md`
//! of one terminal Investigations job.
//!
//! The managed file is exactly `research/artifacts/<safe job id>/report.md` —
//! the same portable manifest `ResearchEnvelopeV1::report_file` already
//! carries. This module owns the whole local half of that manifest:
//!
//! * [`report_upload_plan`] validates the manifest (managed relative path
//!   only, canonical lowercase SHA-256, bounded size) and proves the local
//!   bytes with ONE bounded read, so a push draft can never advertise a file
//!   that cannot be read back byte-for-byte.
//! * [`ensure_research_report_uploaded`] re-proves the local file and moves it
//!   to the content-addressed blob store through `blob_head`/`blob_put`.
//! * [`ensure_research_report_installed`] downloads exactly one hash with a
//!   bounded `blob_get` reader (never `response.bytes()`), verifies the exact
//!   size and SHA-256 BEFORE accepting the bytes, and publishes them
//!   atomically through a same-directory temporary file that never clobbers an
//!   existing path. An exact existing match is an `AlreadyInstalled` no-op and
//!   every install is re-verified with the authoritative envelope checker.
//!
//! Errors are typed ([`ResearchBlobPending`]) and every read is bounded, so a
//! future engine can report durable pending work without parsing message
//! text. Absolute local paths never enter a wire payload: the manifest travels
//! as `report.md` plus its hash and size, and only the hash addresses the
//! network.
//!
//! Path safety: separators, absolute prefixes, drive colons, unsafe job ids,
//! symlinks/Windows reparse points and directories are rejected both for the
//! upload proof and for the install target. As in the writing file layer there
//! is still a check-then-open race between those checks and later filesystem
//! operations; these helpers make no race-free filesystem claim.

// Inactive slice: the future engine calls these helpers after catch-up.
#![allow(dead_code)]

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::Builder as TempFileBuilder;

use super::http::{HealthLimits, SyncApi, SyncError};
use super::research_envelope::{
    is_safe_path_component, verify_report_file, ResearchFileManifestV1, MAX_REPORT_FILE_BYTES,
    REPORT_FILE_REL_PATH,
};

const BYTES_PER_MIB: u64 = 1024 * 1024;
const INITIAL_DOWNLOAD_CAPACITY: usize = 64 * 1024;
/// Same-directory temporary file prefix/suffix for the atomic publish.
const TEMP_PREFIX: &str = ".research-report-";
const TEMP_SUFFIX: &str = ".part";

/// Every way a report blob transfer can stay durably pending. Typed so the
/// engine branches on the variant, never on message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResearchBlobPendingKind {
    /// The server did not advertise a usable `max_blob_mb` limit.
    InvalidLimits,
    /// Unsafe job id, non-managed relative path, or non-canonical hash.
    InvalidManifest,
    /// The declared size exceeds the local ceiling or the advertised limit.
    BlobTooLarge,
    /// A symlink/reparse point or a directory sits on the managed path.
    UnsafeLocalPath,
    /// The local file is missing or cannot be read.
    LocalProofFailed,
    /// The local bytes no longer match the manifest captured for them.
    LocalContentChanged,
    /// A regular file already owns the target with different bytes.
    ExistingTargetDifferent,
    Network,
    Unauthorized,
    AccessDenied,
    RemoteMissing,
    RemoteRejected,
    DownloadTooLarge,
    DownloadSizeMismatch,
    DownloadInvalid,
    InstallFailed,
    FinalProofFailed,
}

/// One bounded, typed reason a report blob stayed pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResearchBlobPending {
    pub(crate) kind: ResearchBlobPendingKind,
    pub(crate) message: String,
}

impl ResearchBlobPending {
    fn new(kind: ResearchBlobPendingKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// The typed local upload plan for one job's managed `report.md`: the validated
/// manifest plus the job it belongs to. Built only after the local bytes were
/// proven, so carrying this plan is proof that the file was readable and
/// matched its manifest at plan time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResearchReportUploadPlan {
    pub(crate) job_id: String,
    pub(crate) manifest: ResearchFileManifestV1,
}

/// Outcome of installing one verified report file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResearchReportInstallOutcome {
    Installed,
    AlreadyInstalled,
}

/// Result of one bounded local proof/read used for upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResearchBlobUploadReady {
    pub(crate) job_id: String,
    /// `false` when the content-addressed blob was already on the server.
    pub(crate) uploaded: bool,
}

/// Validates the managed `report.md` manifest and proves the local bytes with
/// one bounded read. The upload plan is only minted for a file that matches
/// its manifest byte-for-byte right now.
pub(crate) fn report_upload_plan(
    artifacts_root: &Path,
    job_id: &str,
    manifest: &ResearchFileManifestV1,
) -> Result<ResearchReportUploadPlan, ResearchBlobPending> {
    validate_report_manifest(job_id, manifest, None)?;
    prove_local_report(artifacts_root, job_id, manifest)?;
    Ok(ResearchReportUploadPlan {
        job_id: job_id.to_string(),
        manifest: manifest.clone(),
    })
}

/// Re-proves the local report file and uploads it under its own SHA-256 when
/// the server does not hold the blob yet. A successful return is bound to the
/// exact manifest; it is not a push acknowledgement or a sync receipt.
pub(crate) async fn ensure_research_report_uploaded<A: SyncApi>(
    api: &A,
    token: &str,
    artifacts_root: &Path,
    plan: &ResearchReportUploadPlan,
    advertised_limits: &HealthLimits,
) -> Result<ResearchBlobUploadReady, ResearchBlobPending> {
    let max_blob_bytes = advertised_blob_limit(advertised_limits)?;
    validate_report_manifest(&plan.job_id, &plan.manifest, Some(max_blob_bytes))?;
    let bytes = prove_local_report(artifacts_root, &plan.job_id, &plan.manifest)?;

    let exists = api
        .blob_head(token, &plan.manifest.sha256)
        .await
        .map_err(|error| pending_from_sync("blob HEAD", error))?;
    let uploaded = if exists {
        false
    } else {
        api.blob_put(token, &plan.manifest.sha256, bytes)
            .await
            .map_err(|error| pending_from_sync("blob PUT", error))?;
        true
    };

    prove_local_report(artifacts_root, &plan.job_id, &plan.manifest).map_err(|error| {
        ResearchBlobPending::new(
            ResearchBlobPendingKind::FinalProofFailed,
            format!(
                "local report file changed around the upload of {}: {}",
                plan.manifest.sha256, error.message
            ),
        )
    })?;

    Ok(ResearchBlobUploadReady {
        job_id: plan.job_id.clone(),
        uploaded,
    })
}

/// Downloads the one content-addressed report blob with a bounded reader and
/// installs it atomically, or reports the exact installed state. Bytes are
/// verified against the manifest BEFORE anything is written, and the install
/// is re-verified with the authoritative envelope checker afterwards.
pub(crate) async fn ensure_research_report_installed<A: SyncApi>(
    api: &A,
    token: &str,
    artifacts_root: &Path,
    job_id: &str,
    manifest: &ResearchFileManifestV1,
    advertised_limits: &HealthLimits,
) -> Result<ResearchReportInstallOutcome, ResearchBlobPending> {
    let max_blob_bytes = advertised_blob_limit(advertised_limits)?;
    validate_report_manifest(job_id, manifest, Some(max_blob_bytes))?;

    match local_report_state(artifacts_root, job_id, manifest)? {
        LocalReportState::Exact => return Ok(ResearchReportInstallOutcome::AlreadyInstalled),
        LocalReportState::Different(message) => {
            return Err(ResearchBlobPending::new(
                ResearchBlobPendingKind::ExistingTargetDifferent,
                message,
            ))
        }
        LocalReportState::Missing => {}
    }

    let response = api
        .blob_get(token, &manifest.sha256)
        .await
        .map_err(|error| pending_from_sync("blob GET", error))?;
    let bytes = read_bounded_response(response, manifest.size, max_blob_bytes).await?;
    if format!("{:x}", Sha256::digest(&bytes)) != manifest.sha256 {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::DownloadInvalid,
            format!(
                "blob {} bytes do not match the report manifest",
                manifest.sha256
            ),
        ));
    }

    install_verified_report(artifacts_root, job_id, manifest, &bytes)
}

/// Installs verified report bytes through the same-directory temporary file
/// and the no-clobber publish, then re-verifies with the authoritative
/// envelope checker. An exact existing file is `AlreadyInstalled`; anything
/// else already owning the target is never replaced.
pub(crate) fn install_verified_report(
    artifacts_root: &Path,
    job_id: &str,
    manifest: &ResearchFileManifestV1,
    bytes: &[u8],
) -> Result<ResearchReportInstallOutcome, ResearchBlobPending> {
    install_verified_report_inner(artifacts_root, job_id, manifest, bytes, |_, _| {})
}

/// Test-only seam for deterministically creating publication races between
/// the verified exclusive temporary file and the no-clobber publish.
#[cfg(test)]
pub(crate) fn install_verified_report_with_before_publish(
    artifacts_root: &Path,
    job_id: &str,
    manifest: &ResearchFileManifestV1,
    bytes: &[u8],
    before_publish: impl FnOnce(&Path, &Path),
) -> Result<ResearchReportInstallOutcome, ResearchBlobPending> {
    install_verified_report_inner(artifacts_root, job_id, manifest, bytes, before_publish)
}

fn install_verified_report_inner<F>(
    artifacts_root: &Path,
    job_id: &str,
    manifest: &ResearchFileManifestV1,
    bytes: &[u8],
    before_publish: F,
) -> Result<ResearchReportInstallOutcome, ResearchBlobPending>
where
    F: FnOnce(&Path, &Path),
{
    validate_report_manifest(job_id, manifest, None)?;
    if bytes.len() as u64 != manifest.size
        || format!("{:x}", Sha256::digest(bytes)) != manifest.sha256
    {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::DownloadInvalid,
            "report bytes do not match the manifest before install".to_string(),
        ));
    }

    match local_report_state(artifacts_root, job_id, manifest)? {
        LocalReportState::Exact => return Ok(ResearchReportInstallOutcome::AlreadyInstalled),
        LocalReportState::Different(message) => {
            return Err(ResearchBlobPending::new(
                ResearchBlobPendingKind::ExistingTargetDifferent,
                message,
            ))
        }
        LocalReportState::Missing => {}
    }

    let job_dir = ensure_managed_job_directory(artifacts_root, job_id)?;
    let target = job_dir.join(REPORT_FILE_REL_PATH);

    let mut temporary = TempFileBuilder::new()
        .prefix(TEMP_PREFIX)
        .suffix(TEMP_SUFFIX)
        .tempfile_in(&job_dir)
        .map_err(|error| {
            ResearchBlobPending::new(
                ResearchBlobPendingKind::InstallFailed,
                format!("cannot create the report temporary file in {job_dir:?}: {error}"),
            )
        })?;
    temporary.write_all(bytes).map_err(|error| {
        ResearchBlobPending::new(
            ResearchBlobPendingKind::InstallFailed,
            format!("cannot write the report temporary file in {job_dir:?}: {error}"),
        )
    })?;
    temporary.as_file().sync_all().map_err(|error| {
        ResearchBlobPending::new(
            ResearchBlobPendingKind::InstallFailed,
            format!("cannot sync the report temporary file in {job_dir:?}: {error}"),
        )
    })?;

    before_publish(temporary.path(), &target);

    // `persist_noclobber` uses no-replace rename where available and a
    // hard-link/unlink fallback elsewhere. It never replaces `target`; an
    // unsupported filesystem therefore fails closed instead of using rename.
    match temporary.persist_noclobber(&target) {
        Ok(_persisted) => {}
        Err(error) => {
            let publish_error = error.error.to_string();
            drop(error.file);
            return match local_report_state(artifacts_root, job_id, manifest)? {
                LocalReportState::Exact => Ok(ResearchReportInstallOutcome::AlreadyInstalled),
                LocalReportState::Different(message) => Err(ResearchBlobPending::new(
                    ResearchBlobPendingKind::ExistingTargetDifferent,
                    message,
                )),
                LocalReportState::Missing => Err(ResearchBlobPending::new(
                    ResearchBlobPendingKind::InstallFailed,
                    format!(
                        "cannot publish {REPORT_FILE_REL_PATH} for {job_id:?} without replacing an existing path: {publish_error}"
                    ),
                )),
            };
        }
    }

    verify_report_file(artifacts_root, job_id, manifest).map_err(|error| {
        ResearchBlobPending::new(
            ResearchBlobPendingKind::FinalProofFailed,
            format!(
                "installed report file for {job_id:?} failed final verification ({}): {}",
                error.code, error.message
            ),
        )
    })?;
    Ok(ResearchReportInstallOutcome::Installed)
}

// ─────────────────────────────── validation ───────────────────────────────

/// Enforces the single managed relative path, the safe job id, the canonical
/// lowercase SHA-256 and both size ceilings (local and advertised). Runs
/// before any filesystem or network access.
fn validate_report_manifest(
    job_id: &str,
    manifest: &ResearchFileManifestV1,
    advertised_max_blob_bytes: Option<u64>,
) -> Result<(), ResearchBlobPending> {
    if !is_safe_path_component(job_id) {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::InvalidManifest,
            format!("job id {job_id:?} is not a safe path component"),
        ));
    }
    // The only managed relative path: no separator, absolute prefix, drive
    // colon or traversal can ever appear in the wire manifest.
    if manifest.rel_path != REPORT_FILE_REL_PATH {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::InvalidManifest,
            format!(
                "report file path {:?} is not the managed {REPORT_FILE_REL_PATH} path",
                manifest.rel_path
            ),
        ));
    }
    if manifest.sha256.len() != 64
        || !manifest
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::InvalidManifest,
            "report file sha256 must be 64 lowercase hex digits".to_string(),
        ));
    }
    if manifest.size > MAX_REPORT_FILE_BYTES {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::BlobTooLarge,
            format!(
                "report file declares {} bytes; local bound is {MAX_REPORT_FILE_BYTES}",
                manifest.size
            ),
        ));
    }
    if let Some(max_blob_bytes) = advertised_max_blob_bytes {
        if manifest.size > max_blob_bytes {
            return Err(ResearchBlobPending::new(
                ResearchBlobPendingKind::BlobTooLarge,
                format!(
                    "report file declares {} bytes; advertised blob limit is {max_blob_bytes}",
                    manifest.size
                ),
            ));
        }
    }
    Ok(())
}

/// The advertised `max_blob_mb` limit as bytes. Zero, negative or overflowing
/// limits fail closed instead of silently allowing any size.
fn advertised_blob_limit(limits: &HealthLimits) -> Result<u64, ResearchBlobPending> {
    u64::try_from(limits.max_blob_mb)
        .ok()
        .filter(|value| *value > 0)
        .and_then(|value| value.checked_mul(BYTES_PER_MIB))
        .ok_or_else(|| {
            ResearchBlobPending::new(
                ResearchBlobPendingKind::InvalidLimits,
                format!(
                    "server did not advertise a usable max_blob_mb limit ({})",
                    limits.max_blob_mb
                ),
            )
        })
}

// ─────────────────────────── local filesystem proof ───────────────────────

/// Bounded local proof of one managed report file. Rejects links/reparse
/// points and directories, caps the read at the manifest size plus one byte,
/// and verifies the exact size and SHA-256 against the manifest. The returned
/// bytes are exactly the proven ones, ready for upload.
fn prove_local_report(
    artifacts_root: &Path,
    job_id: &str,
    manifest: &ResearchFileManifestV1,
) -> Result<Vec<u8>, ResearchBlobPending> {
    let Some(job_dir) = managed_job_directory(artifacts_root, job_id)? else {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::LocalProofFailed,
            format!("managed report file for {job_id:?} is missing"),
        ));
    };
    let target = job_dir.join(REPORT_FILE_REL_PATH);
    reject_unsafe_target(&target)?;

    let bytes = read_bounded_file(&target, manifest.size).map_err(|error| {
        ResearchBlobPending::new(
            ResearchBlobPendingKind::LocalProofFailed,
            format!("cannot prove {}: {error}", target.display()),
        )
    })?;
    if bytes.len() as u64 != manifest.size {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::LocalContentChanged,
            format!(
                "{} holds {} bytes; the manifest declares {}",
                target.display(),
                bytes.len(),
                manifest.size
            ),
        ));
    }
    if format!("{:x}", Sha256::digest(&bytes)) != manifest.sha256 {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::LocalContentChanged,
            format!(
                "{} no longer matches manifest hash {}",
                target.display(),
                manifest.sha256
            ),
        ));
    }
    Ok(bytes)
}

/// What the local managed path holds for one manifest.
enum LocalReportState {
    /// A regular file byte-identical to the manifest.
    Exact,
    /// Nothing owns the managed path.
    Missing,
    /// A regular file owns the target with different (or unreadable) bytes.
    Different(String),
}

fn local_report_state(
    artifacts_root: &Path,
    job_id: &str,
    manifest: &ResearchFileManifestV1,
) -> Result<LocalReportState, ResearchBlobPending> {
    let Some(job_dir) = managed_job_directory(artifacts_root, job_id)? else {
        return Ok(LocalReportState::Missing);
    };
    let target = job_dir.join(REPORT_FILE_REL_PATH);
    reject_unsafe_target(&target)?;

    let bytes = match read_bounded_file(&target, manifest.size) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LocalReportState::Missing)
        }
        Err(error) => {
            return Ok(LocalReportState::Different(format!(
                "existing {} cannot be read safely: {error}",
                target.display()
            )))
        }
    };
    if bytes.len() as u64 == manifest.size
        && format!("{:x}", Sha256::digest(&bytes)) == manifest.sha256
    {
        Ok(LocalReportState::Exact)
    } else {
        Ok(LocalReportState::Different(format!(
            "existing {} differs from the report manifest",
            target.display()
        )))
    }
}

/// Resolves `research/artifacts/<job id>` through physical components only.
/// `Ok(None)` when the directory does not exist yet; links/reparse points,
/// non-directories and directories that do not resolve directly beneath the
/// artifacts root are hard errors.
fn managed_job_directory(
    artifacts_root: &Path,
    job_id: &str,
) -> Result<Option<PathBuf>, ResearchBlobPending> {
    let job_dir = artifacts_root.join(job_id);
    let metadata = match fs::symlink_metadata(&job_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ResearchBlobPending::new(
                ResearchBlobPendingKind::LocalProofFailed,
                format!("cannot inspect job directory {job_dir:?}: {error}"),
            ))
        }
    };
    if is_link_or_reparse_point(&metadata) {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::UnsafeLocalPath,
            format!("job directory {job_dir:?} must not be a symlink or reparse point"),
        ));
    }
    if !metadata.is_dir() {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::UnsafeLocalPath,
            format!("job directory {job_dir:?} is not a directory"),
        ));
    }

    let canonical_root = fs::canonicalize(artifacts_root).map_err(|error| {
        ResearchBlobPending::new(
            ResearchBlobPendingKind::LocalProofFailed,
            format!("cannot resolve artifacts root {artifacts_root:?}: {error}"),
        )
    })?;
    let canonical_dir = fs::canonicalize(&job_dir).map_err(|error| {
        ResearchBlobPending::new(
            ResearchBlobPendingKind::LocalProofFailed,
            format!("cannot resolve job directory {job_dir:?}: {error}"),
        )
    })?;
    if canonical_dir.parent() != Some(canonical_root.as_path()) {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::UnsafeLocalPath,
            format!(
                "job directory {job_dir:?} does not resolve directly beneath the artifacts root"
            ),
        ));
    }
    Ok(Some(job_dir))
}

/// Creates (or re-checks) the managed job directory through physical
/// components only. Used before the atomic publish.
fn ensure_managed_job_directory(
    artifacts_root: &Path,
    job_id: &str,
) -> Result<PathBuf, ResearchBlobPending> {
    if let Some(job_dir) = managed_job_directory(artifacts_root, job_id)? {
        return Ok(job_dir);
    }
    let job_dir = artifacts_root.join(job_id);
    fs::create_dir_all(&job_dir).map_err(|error| {
        ResearchBlobPending::new(
            ResearchBlobPendingKind::InstallFailed,
            format!("cannot create job directory {job_dir:?}: {error}"),
        )
    })?;
    managed_job_directory(artifacts_root, job_id)?.ok_or_else(|| {
        ResearchBlobPending::new(
            ResearchBlobPendingKind::InstallFailed,
            format!("job directory {job_dir:?} vanished during install"),
        )
    })
}

/// The managed `report.md` target must be absent or a plain regular file: a
/// symlink/reparse point or a directory is a hard rejection, never something
/// to overwrite or read through.
fn reject_unsafe_target(target: &Path) -> Result<(), ResearchBlobPending> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ResearchBlobPending::new(
                ResearchBlobPendingKind::LocalProofFailed,
                format!("cannot inspect {}: {error}", target.display()),
            ))
        }
    };
    if is_link_or_reparse_point(&metadata) {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::UnsafeLocalPath,
            format!(
                "{} must not be a symlink or reparse point",
                target.display()
            ),
        ));
    }
    if !metadata.is_file() {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::UnsafeLocalPath,
            format!("{} is not a regular file", target.display()),
        ));
    }
    Ok(())
}

/// Reads at most `size` bytes plus one, so an oversized or shrunk file is
/// detected by length instead of being hashed truncated.
fn read_bounded_file(path: &Path, size: u64) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut bytes = Vec::with_capacity(
        size.min(INITIAL_DOWNLOAD_CAPACITY as u64)
            .try_into()
            .unwrap_or(INITIAL_DOWNLOAD_CAPACITY),
    );
    file.take(size.checked_add(1).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    #[cfg(not(windows))]
    false
}

// ───────────────────────────── bounded download ───────────────────────────

/// Chunked, size-capped `blob_get` body reader. The manifest size and the
/// advertised blob limit both bound every append; `response.bytes()` is never
/// used.
async fn read_bounded_response(
    mut response: reqwest::Response,
    expected_size: u64,
    max_blob_bytes: u64,
) -> Result<Vec<u8>, ResearchBlobPending> {
    if expected_size > max_blob_bytes {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::BlobTooLarge,
            format!("declared download size {expected_size} exceeds {max_blob_bytes}"),
        ));
    }
    if let Some(content_length) = response.content_length() {
        if content_length > expected_size || content_length > max_blob_bytes {
            return Err(ResearchBlobPending::new(
                ResearchBlobPendingKind::DownloadTooLarge,
                format!("blob response declares {content_length} bytes; expected {expected_size}"),
            ));
        }
    }

    let mut bytes = Vec::with_capacity(
        expected_size
            .min(INITIAL_DOWNLOAD_CAPACITY as u64)
            .try_into()
            .unwrap_or(INITIAL_DOWNLOAD_CAPACITY),
    );
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        ResearchBlobPending::new(
            ResearchBlobPendingKind::Network,
            format!("blob GET body failed: {error}"),
        )
    })? {
        let next_len = (bytes.len() as u64)
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| {
                ResearchBlobPending::new(
                    ResearchBlobPendingKind::DownloadTooLarge,
                    "blob response length overflowed".to_string(),
                )
            })?;
        if next_len > expected_size || next_len > max_blob_bytes {
            return Err(ResearchBlobPending::new(
                ResearchBlobPendingKind::DownloadTooLarge,
                format!("blob response exceeded its {expected_size}-byte manifest size"),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }

    if bytes.len() as u64 != expected_size {
        return Err(ResearchBlobPending::new(
            ResearchBlobPendingKind::DownloadSizeMismatch,
            format!(
                "blob response contained {} bytes; manifest says {expected_size}",
                bytes.len()
            ),
        ));
    }
    Ok(bytes)
}

fn pending_from_sync(context: &str, error: SyncError) -> ResearchBlobPending {
    match error {
        SyncError::Network(message) => ResearchBlobPending::new(
            ResearchBlobPendingKind::Network,
            format!("{context} failed: {message}"),
        ),
        SyncError::Api {
            status: 401,
            code,
            message,
        } => ResearchBlobPending::new(
            ResearchBlobPendingKind::Unauthorized,
            format!("{context} was unauthorized ({code}): {message}"),
        ),
        SyncError::Api {
            status: 403,
            code,
            message,
        } => ResearchBlobPending::new(
            ResearchBlobPendingKind::AccessDenied,
            format!("{context} was denied ({code}): {message}"),
        ),
        SyncError::Api {
            status: 404,
            code,
            message,
        } => ResearchBlobPending::new(
            ResearchBlobPendingKind::RemoteMissing,
            format!("{context} could not find the blob ({code}): {message}"),
        ),
        SyncError::Api {
            status: 413,
            code,
            message,
        } => ResearchBlobPending::new(
            ResearchBlobPendingKind::BlobTooLarge,
            format!("{context} rejected an oversized blob ({code}): {message}"),
        ),
        SyncError::Api {
            status,
            code,
            message,
        } => ResearchBlobPending::new(
            ResearchBlobPendingKind::RemoteRejected,
            format!("{context} failed with {status} ({code}): {message}"),
        ),
        SyncError::InvalidUrl(message) | SyncError::Decode(message) => ResearchBlobPending::new(
            ResearchBlobPendingKind::RemoteRejected,
            format!("{context} failed: {message}"),
        ),
    }
}
