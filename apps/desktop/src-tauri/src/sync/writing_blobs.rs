//! Inactive HTTP orchestration for writing attachment blobs.
//!
//! These helpers deliberately own no database connection, cursor, outbox, or
//! acknowledgement. A future engine stage may call them with an immutable
//! writing envelope, and must retain its queued row whenever `Err` is returned.
//! Upload readiness is established before the envelope push; download readiness
//! is established before the receive adapter is allowed to construct a receipt.
//!
//! `SyncApi::blob_get` exposes a `reqwest::Response` while the verified writing
//! installer accepts a synchronous `Read`. Downloads are therefore buffered in
//! memory, but the buffer is capped by both the manifest size and the server's
//! advertised blob limit before every append. This is bounded buffering, not
//! streaming installation.
//!
//! The writing file layer rejects links/reparse points and verifies canonical
//! containment. There is still a check-then-open path race between those checks
//! and later filesystem operations; these helpers make no race-free filesystem
//! claim.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};

use super::http::{HealthLimits, SyncApi, SyncError};
use crate::writing::sync_envelope::{AttachmentFileV1, AttachmentManifestV1, WritingEnvelopeV1};
use crate::writing::sync_files::{
    install_verified_attachment, scan_document_attachments, verify_attachment_manifest,
    AttachmentInstallOutcome, AttachmentIssue, ATTACHMENT_EXISTS_DIFFERENT,
    ATTACHMENT_HASH_MISMATCH, ATTACHMENT_INVALID_ENTRY, ATTACHMENT_REF_UNSAFE,
};

const BYTES_PER_MIB: u64 = 1024 * 1024;
const INITIAL_DOWNLOAD_CAPACITY: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WritingBlobPendingKind {
    InvalidLimits,
    InvalidEnvelope,
    InvalidManifest,
    ManifestTooLarge,
    BlobTooLarge,
    LocalProofFailed,
    LocalContentChanged,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WritingBlobPending {
    pub(crate) kind: WritingBlobPendingKind,
    pub(crate) message: String,
}

impl WritingBlobPending {
    fn new(kind: WritingBlobPendingKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WritingBlobUploadReady {
    pub(crate) envelope_fingerprint_sha256: String,
    pub(crate) unique_blobs: usize,
    pub(crate) uploaded_blobs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WritingBlobDownloadReady {
    pub(crate) envelope_fingerprint_sha256: String,
    pub(crate) unique_blobs: usize,
    pub(crate) downloaded_blobs: usize,
    pub(crate) installed_files: usize,
    pub(crate) already_installed_files: usize,
}

#[derive(Debug, Clone, Copy)]
struct TransferLimits {
    max_envelope_bytes: u64,
    max_blob_bytes: u64,
}

/// Proves every local attachment, probes each unique hash, and uploads only
/// hashes absent from the server. A successful return is bound to the immutable
/// envelope fingerprint; it is not a push acknowledgement or a sync receipt.
pub(crate) async fn ensure_writing_blobs_uploaded<A: SyncApi>(
    api: &A,
    token: &str,
    data_root: &Path,
    envelope: &WritingEnvelopeV1,
    advertised_limits: &HealthLimits,
) -> Result<WritingBlobUploadReady, WritingBlobPending> {
    let (limits, fingerprint, files) = validate_envelope_and_manifest(envelope, advertised_limits)?;
    verify_full_manifest(
        envelope,
        data_root,
        WritingBlobPendingKind::LocalProofFailed,
    )?;

    let unique = unique_entries(files);
    let mut uploaded_blobs = 0;
    for entry in unique.values() {
        let exists = api
            .blob_head(token, &entry.sha256)
            .await
            .map_err(|error| pending_from_sync("blob HEAD", error))?;
        if exists {
            continue;
        }

        let bytes = read_upload_bytes(data_root, entry, limits.max_blob_bytes)?;
        api.blob_put(token, &entry.sha256, bytes)
            .await
            .map_err(|error| pending_from_sync("blob PUT", error))?;
        uploaded_blobs += 1;
    }

    verify_full_manifest(
        envelope,
        data_root,
        WritingBlobPendingKind::FinalProofFailed,
    )?;
    Ok(WritingBlobUploadReady {
        envelope_fingerprint_sha256: fingerprint,
        unique_blobs: unique.len(),
        uploaded_blobs,
    })
}

/// Downloads absent attachment hashes and hands their bounded buffers to the
/// existing verified, no-clobber installer. Existing files are byte-verified by
/// the real writing scanner; an `is_file` check is never accepted as proof.
pub(crate) async fn ensure_writing_blobs_installed<A: SyncApi>(
    api: &A,
    token: &str,
    data_root: &Path,
    envelope: &WritingEnvelopeV1,
    advertised_limits: &HealthLimits,
) -> Result<WritingBlobDownloadReady, WritingBlobPending> {
    let (limits, fingerprint, files) = validate_envelope_and_manifest(envelope, advertised_limits)?;
    validate_reference_coverage(envelope, files)?;
    preflight_remote_references(envelope, data_root, files)?;

    let unique_blobs = unique_entries(files).len();
    if verify_manifest(envelope, data_root).is_ok() {
        return Ok(WritingBlobDownloadReady {
            envelope_fingerprint_sha256: fingerprint,
            unique_blobs,
            downloaded_blobs: 0,
            installed_files: 0,
            already_installed_files: files.len(),
        });
    }

    let mut missing_by_hash: BTreeMap<&str, Vec<&AttachmentFileV1>> = BTreeMap::new();
    let mut already_installed_files = 0;
    for entry in files {
        match local_entry_state(data_root, entry) {
            LocalEntryState::Exact => already_installed_files += 1,
            LocalEntryState::Missing => missing_by_hash
                .entry(entry.sha256.as_str())
                .or_default()
                .push(entry),
            LocalEntryState::Different(message) => {
                return Err(WritingBlobPending::new(
                    WritingBlobPendingKind::ExistingTargetDifferent,
                    message,
                ));
            }
        }
    }

    let mut downloaded_blobs = 0;
    let mut installed_files = 0;
    for (sha256, entries) in missing_by_hash {
        let response = api
            .blob_get(token, sha256)
            .await
            .map_err(|error| pending_from_sync("blob GET", error))?;
        let expected_size = entries
            .first()
            .ok_or_else(|| {
                WritingBlobPending::new(
                    WritingBlobPendingKind::InvalidManifest,
                    "attachment hash group was unexpectedly empty",
                )
            })?
            .size;
        let bytes = read_bounded_response(response, expected_size, limits.max_blob_bytes).await?;
        downloaded_blobs += 1;

        for entry in entries {
            let mut reader = bytes.as_slice();
            match install_verified_attachment(data_root, entry, &mut reader) {
                Ok(AttachmentInstallOutcome::Installed) => installed_files += 1,
                Ok(AttachmentInstallOutcome::AlreadyInstalled) => already_installed_files += 1,
                Err(error) => return Err(pending_from_install(entry, error)),
            }
        }
    }

    verify_full_manifest(
        envelope,
        data_root,
        WritingBlobPendingKind::FinalProofFailed,
    )?;
    Ok(WritingBlobDownloadReady {
        envelope_fingerprint_sha256: fingerprint,
        unique_blobs,
        downloaded_blobs,
        installed_files,
        already_installed_files,
    })
}

fn validate_envelope_and_manifest<'a>(
    envelope: &'a WritingEnvelopeV1,
    advertised_limits: &HealthLimits,
) -> Result<(TransferLimits, String, &'a [AttachmentFileV1]), WritingBlobPending> {
    let limits = transfer_limits(advertised_limits)?;
    let canonical = envelope.to_canonical_json().map_err(|error| {
        WritingBlobPending::new(
            WritingBlobPendingKind::InvalidEnvelope,
            format!(
                "writing envelope is invalid ({}): {}",
                error.code, error.message
            ),
        )
    })?;
    if canonical.len() as u64 > limits.max_envelope_bytes {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::ManifestTooLarge,
            format!(
                "writing envelope is {} bytes; advertised push limit is {} bytes",
                canonical.len(),
                limits.max_envelope_bytes
            ),
        ));
    }

    let AttachmentManifestV1::Validated { files } = &envelope.attachments_manifest else {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::InvalidManifest,
            "writing attachment manifest still requires preparation",
        ));
    };

    let mut hash_facts: BTreeMap<&str, (u64, &str)> = BTreeMap::new();
    for entry in files {
        validate_manifest_entry(entry, limits.max_blob_bytes)?;
        if let Some((size, media_type)) = hash_facts.insert(
            entry.sha256.as_str(),
            (entry.size, entry.media_type.as_str()),
        ) {
            if size != entry.size || media_type != entry.media_type {
                return Err(WritingBlobPending::new(
                    WritingBlobPendingKind::InvalidManifest,
                    format!(
                        "attachment hash {} has inconsistent size or media type",
                        entry.sha256
                    ),
                ));
            }
        }
    }

    let fingerprint = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    Ok((limits, fingerprint, files))
}

fn transfer_limits(limits: &HealthLimits) -> Result<TransferLimits, WritingBlobPending> {
    let max_envelope_bytes = u64::try_from(limits.max_push_bytes)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            WritingBlobPending::new(
                WritingBlobPendingKind::InvalidLimits,
                "server did not advertise a positive max_push_bytes limit",
            )
        })?;
    let max_blob_mb = u64::try_from(limits.max_blob_mb)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            WritingBlobPending::new(
                WritingBlobPendingKind::InvalidLimits,
                "server did not advertise a positive max_blob_mb limit",
            )
        })?;
    let max_blob_bytes = max_blob_mb.checked_mul(BYTES_PER_MIB).ok_or_else(|| {
        WritingBlobPending::new(
            WritingBlobPendingKind::InvalidLimits,
            "server max_blob_mb limit overflows bytes",
        )
    })?;

    Ok(TransferLimits {
        max_envelope_bytes,
        max_blob_bytes,
    })
}

fn validate_manifest_entry(
    entry: &AttachmentFileV1,
    max_blob_bytes: u64,
) -> Result<(), WritingBlobPending> {
    if entry.sha256.len() != 64
        || !entry
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::InvalidManifest,
            "attachment sha256 must be 64 lowercase hex digits",
        ));
    }
    if entry.size == 0 || entry.size > max_blob_bytes {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::BlobTooLarge,
            format!(
                "attachment {} declares {} bytes; allowed range is 1..={max_blob_bytes}",
                entry.rel_path, entry.size
            ),
        ));
    }

    let Some((root, name)) = entry.rel_path.split_once('/') else {
        return Err(invalid_manifest_path(&entry.rel_path));
    };
    if name.contains('/')
        || !matches!(root, "writing-images" | "writing-crops")
        || name.is_empty()
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(invalid_manifest_path(&entry.rel_path));
    }

    let extension = match entry.media_type.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => {
            return Err(WritingBlobPending::new(
                WritingBlobPendingKind::InvalidManifest,
                format!(
                    "attachment {} has unsupported media type {:?}",
                    entry.rel_path, entry.media_type
                ),
            ));
        }
    };
    if !name.ends_with(&format!(".{extension}")) {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::InvalidManifest,
            format!(
                "attachment {} does not match media type {:?}",
                entry.rel_path, entry.media_type
            ),
        ));
    }
    if root == "writing-images" && name != format!("{}.{}", entry.sha256, extension) {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::InvalidManifest,
            format!(
                "content-addressed attachment {} does not match its hash",
                entry.rel_path
            ),
        ));
    }

    Ok(())
}

fn invalid_manifest_path(path: &str) -> WritingBlobPending {
    WritingBlobPending::new(
        WritingBlobPendingKind::InvalidManifest,
        format!("attachment path {path:?} is not a safe managed writing path"),
    )
}

fn source_regions(envelope: &WritingEnvelopeV1) -> Vec<serde_json::Value> {
    envelope
        .citation_projections
        .corpus
        .iter()
        .filter_map(|citation| citation.source_region_json.clone())
        .collect()
}

fn verify_manifest(
    envelope: &WritingEnvelopeV1,
    data_root: &Path,
) -> Result<(), crate::writing::sync_files::AttachmentManifestIssue> {
    let regions = source_regions(envelope);
    verify_attachment_manifest(
        &envelope.content_json,
        &regions,
        data_root,
        &envelope.attachments_manifest,
    )
}

fn verify_full_manifest(
    envelope: &WritingEnvelopeV1,
    data_root: &Path,
    kind: WritingBlobPendingKind,
) -> Result<(), WritingBlobPending> {
    verify_manifest(envelope, data_root)
        .map_err(|issue| WritingBlobPending::new(kind, issue.to_string()))
}

fn unique_entries(files: &[AttachmentFileV1]) -> BTreeMap<&str, &AttachmentFileV1> {
    let mut unique = BTreeMap::new();
    for entry in files {
        unique.entry(entry.sha256.as_str()).or_insert(entry);
    }
    unique
}

fn validated_local_path(
    data_root: &Path,
    entry: &AttachmentFileV1,
) -> Result<PathBuf, WritingBlobPending> {
    let Some((root, name)) = entry.rel_path.split_once('/') else {
        return Err(invalid_manifest_path(&entry.rel_path));
    };
    Ok(data_root.join(root).join(name))
}

fn read_upload_bytes(
    data_root: &Path,
    entry: &AttachmentFileV1,
    max_blob_bytes: u64,
) -> Result<Vec<u8>, WritingBlobPending> {
    if entry.size > max_blob_bytes {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::BlobTooLarge,
            format!("attachment {} exceeds the blob limit", entry.rel_path),
        ));
    }
    let path = validated_local_path(data_root, entry)?;
    let file = File::open(&path).map_err(|error| {
        WritingBlobPending::new(
            WritingBlobPendingKind::LocalContentChanged,
            format!("cannot reopen verified attachment {path:?}: {error}"),
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        WritingBlobPending::new(
            WritingBlobPendingKind::LocalContentChanged,
            format!("cannot inspect reopened attachment {path:?}: {error}"),
        )
    })?;
    if !metadata.is_file() || metadata.len() != entry.size {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::LocalContentChanged,
            format!(
                "attachment {} changed after manifest verification",
                entry.rel_path
            ),
        ));
    }

    let read_limit = entry.size.checked_add(1).ok_or_else(|| {
        WritingBlobPending::new(
            WritingBlobPendingKind::BlobTooLarge,
            format!("attachment {} has an unrepresentable size", entry.rel_path),
        )
    })?;
    let mut bytes = Vec::with_capacity(entry.size.min(INITIAL_DOWNLOAD_CAPACITY as u64) as usize);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            WritingBlobPending::new(
                WritingBlobPendingKind::LocalContentChanged,
                format!("cannot read verified attachment {path:?}: {error}"),
            )
        })?;
    if bytes.len() as u64 != entry.size || format!("{:x}", Sha256::digest(&bytes)) != entry.sha256 {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::LocalContentChanged,
            format!(
                "attachment {} bytes changed after manifest verification",
                entry.rel_path
            ),
        ));
    }
    Ok(bytes)
}

fn validate_reference_coverage(
    envelope: &WritingEnvelopeV1,
    files: &[AttachmentFileV1],
) -> Result<(), WritingBlobPending> {
    // Reuse the authoritative scanner against a guaranteed-empty root so every
    // safe reference is reported as missing and can be compared before HTTP or
    // any path operation beneath the caller's real data root.
    let empty_root = tempfile::tempdir().map_err(|error| {
        WritingBlobPending::new(
            WritingBlobPendingKind::LocalProofFailed,
            format!("cannot create bounded reference-validation root: {error}"),
        )
    })?;
    let regions = source_regions(envelope);
    let report = scan_document_attachments(&envelope.content_json, &regions, empty_root.path());
    let mut referenced_paths = BTreeSet::new();
    for unresolved in report.unresolved {
        if unresolved.issue != AttachmentIssue::MissingFile {
            return Err(WritingBlobPending::new(
                WritingBlobPendingKind::InvalidManifest,
                format!(
                    "remote envelope has an unsafe or malformed attachment reference {:?} ({:?})",
                    unresolved.reference, unresolved.issue
                ),
            ));
        }
        referenced_paths.insert(unresolved.reference);
    }
    let manifest_paths: BTreeSet<String> =
        files.iter().map(|entry| entry.rel_path.clone()).collect();
    if referenced_paths != manifest_paths {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::InvalidManifest,
            "remote manifest does not exactly cover the envelope references",
        ));
    }
    Ok(())
}

fn preflight_remote_references(
    envelope: &WritingEnvelopeV1,
    data_root: &Path,
    files: &[AttachmentFileV1],
) -> Result<(), WritingBlobPending> {
    let manifest_paths: BTreeSet<&str> =
        files.iter().map(|entry| entry.rel_path.as_str()).collect();
    let regions = source_regions(envelope);
    let report = scan_document_attachments(&envelope.content_json, &regions, data_root);

    for unresolved in &report.unresolved {
        match unresolved.issue {
            AttachmentIssue::MissingFile
                if manifest_paths.contains(unresolved.reference.as_str()) => {}
            AttachmentIssue::MissingFile
            | AttachmentIssue::UnsafeReference
            | AttachmentIssue::MalformedReference => {
                return Err(WritingBlobPending::new(
                    WritingBlobPendingKind::InvalidManifest,
                    format!(
                        "remote envelope has an unproven attachment reference {:?} ({:?})",
                        unresolved.reference, unresolved.issue
                    ),
                ));
            }
            _ if manifest_paths.contains(unresolved.reference.as_str()) => {
                return Err(WritingBlobPending::new(
                    WritingBlobPendingKind::ExistingTargetDifferent,
                    format!(
                        "existing attachment {:?} is different or corrupt ({:?})",
                        unresolved.reference, unresolved.issue
                    ),
                ));
            }
            _ => {
                return Err(WritingBlobPending::new(
                    WritingBlobPendingKind::InvalidManifest,
                    format!(
                        "remote envelope reference {:?} is not installable ({:?})",
                        unresolved.reference, unresolved.issue
                    ),
                ));
            }
        }
    }

    if let AttachmentManifestV1::Validated {
        files: scanned_files,
    } = report.manifest
    {
        let scanned_paths: BTreeSet<&str> = scanned_files
            .iter()
            .map(|entry| entry.rel_path.as_str())
            .collect();
        if scanned_paths != manifest_paths {
            return Err(WritingBlobPending::new(
                WritingBlobPendingKind::InvalidManifest,
                "remote manifest does not exactly cover the envelope references",
            ));
        }
        if !entries_match_by_path(&scanned_files, files) {
            return Err(WritingBlobPending::new(
                WritingBlobPendingKind::ExistingTargetDifferent,
                "existing attachment bytes differ from the remote manifest",
            ));
        }
    }

    Ok(())
}

fn entries_match_by_path(left: &[AttachmentFileV1], right: &[AttachmentFileV1]) -> bool {
    let left: BTreeMap<&str, &AttachmentFileV1> = left
        .iter()
        .map(|entry| (entry.rel_path.as_str(), entry))
        .collect();
    let right: BTreeMap<&str, &AttachmentFileV1> = right
        .iter()
        .map(|entry| (entry.rel_path.as_str(), entry))
        .collect();
    left == right
}

enum LocalEntryState {
    Exact,
    Missing,
    Different(String),
}

fn local_entry_state(data_root: &Path, entry: &AttachmentFileV1) -> LocalEntryState {
    let probe = json!({
        "type": "writingImage",
        "attrs": { "src": entry.rel_path }
    });
    let report = scan_document_attachments(&probe, &[], data_root);
    match report.manifest {
        AttachmentManifestV1::Validated { files }
            if files.as_slice() == std::slice::from_ref(entry) =>
        {
            LocalEntryState::Exact
        }
        AttachmentManifestV1::PreparationRequired
            if report.unresolved.len() == 1
                && report.unresolved[0].reference == entry.rel_path
                && report.unresolved[0].issue == AttachmentIssue::MissingFile =>
        {
            LocalEntryState::Missing
        }
        _ => LocalEntryState::Different(format!(
            "existing attachment {} does not exactly match the remote manifest",
            entry.rel_path
        )),
    }
}

async fn read_bounded_response(
    mut response: reqwest::Response,
    expected_size: u64,
    max_blob_bytes: u64,
) -> Result<Vec<u8>, WritingBlobPending> {
    if expected_size > max_blob_bytes {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::BlobTooLarge,
            format!("declared download size {expected_size} exceeds {max_blob_bytes}"),
        ));
    }
    if let Some(content_length) = response.content_length() {
        if content_length > expected_size || content_length > max_blob_bytes {
            return Err(WritingBlobPending::new(
                WritingBlobPendingKind::DownloadTooLarge,
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
        WritingBlobPending::new(
            WritingBlobPendingKind::Network,
            format!("blob GET body failed: {error}"),
        )
    })? {
        let next_len = (bytes.len() as u64)
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| {
                WritingBlobPending::new(
                    WritingBlobPendingKind::DownloadTooLarge,
                    "blob response length overflowed",
                )
            })?;
        if next_len > expected_size || next_len > max_blob_bytes {
            return Err(WritingBlobPending::new(
                WritingBlobPendingKind::DownloadTooLarge,
                format!("blob response exceeded its {expected_size}-byte manifest size"),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }

    if bytes.len() as u64 != expected_size {
        return Err(WritingBlobPending::new(
            WritingBlobPendingKind::DownloadSizeMismatch,
            format!(
                "blob response contained {} bytes; manifest says {expected_size}",
                bytes.len()
            ),
        ));
    }
    Ok(bytes)
}

fn pending_from_sync(context: &str, error: SyncError) -> WritingBlobPending {
    match error {
        SyncError::Network(message) => WritingBlobPending::new(
            WritingBlobPendingKind::Network,
            format!("{context} failed: {message}"),
        ),
        SyncError::Api {
            status: 401,
            code,
            message,
        } => WritingBlobPending::new(
            WritingBlobPendingKind::Unauthorized,
            format!("{context} was unauthorized ({code}): {message}"),
        ),
        SyncError::Api {
            status: 403,
            code,
            message,
        } => WritingBlobPending::new(
            WritingBlobPendingKind::AccessDenied,
            format!("{context} was denied ({code}): {message}"),
        ),
        SyncError::Api {
            status: 404,
            code,
            message,
        } => WritingBlobPending::new(
            WritingBlobPendingKind::RemoteMissing,
            format!("{context} could not find the blob ({code}): {message}"),
        ),
        SyncError::Api {
            status: 413,
            code,
            message,
        } => WritingBlobPending::new(
            WritingBlobPendingKind::BlobTooLarge,
            format!("{context} rejected an oversized blob ({code}): {message}"),
        ),
        SyncError::Api {
            status,
            code,
            message,
        } => WritingBlobPending::new(
            WritingBlobPendingKind::RemoteRejected,
            format!("{context} failed with {status} ({code}): {message}"),
        ),
        SyncError::InvalidUrl(message) | SyncError::Decode(message) => WritingBlobPending::new(
            WritingBlobPendingKind::RemoteRejected,
            format!("{context} failed: {message}"),
        ),
    }
}

fn pending_from_install(
    entry: &AttachmentFileV1,
    error: crate::writing::repository::WritingError,
) -> WritingBlobPending {
    let kind = match error.code.as_str() {
        ATTACHMENT_EXISTS_DIFFERENT | ATTACHMENT_REF_UNSAFE => {
            WritingBlobPendingKind::ExistingTargetDifferent
        }
        ATTACHMENT_HASH_MISMATCH | ATTACHMENT_INVALID_ENTRY => {
            WritingBlobPendingKind::DownloadInvalid
        }
        _ => WritingBlobPendingKind::InstallFailed,
    };
    WritingBlobPending::new(
        kind,
        format!(
            "cannot install attachment {} ({}): {}",
            entry.rel_path, error.code, error.message
        ),
    )
}
