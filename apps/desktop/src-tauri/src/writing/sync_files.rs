//! Offline file-proving layer for the writing attachments (`writing-images/`
//! and `writing-crops/`).
//!
//! The envelope carries an `AttachmentManifestV1`, and the offline receiver
//! only accepts documents whose manifest is `Validated`. This module is what
//! turns "the document references files" into that proof:
//!
//! * [`scan_document_attachments`] walks the same reference shapes the
//!   frontend export walker uses (`writingImage` nodes and `quotedParts`
//!   image entries) and hashes every referenced file under the data root.
//!   A document whose files are missing, unsafe, unreadable or corrupted is
//!   reported as `PreparationRequired`, never as a fake validated manifest.
//! * [`install_verified_attachment`] is the defensive write side used when a
//!   peer's files arrive: stream into an exclusively created temporary file in
//!   the validated target directory, verify hash, size and media type, then
//!   publish without replacing an existing destination.
//!
//! No network, no database: the future transport wires these into the blob
//! endpoints, and the caller builds the `AttachmentInstallReceipt` only after
//! every entry of one envelope passed [`install_verified_attachment`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::Builder as TempFileBuilder;

use super::repository::{WritingError, WritingResult};
use super::sync_envelope::{is_portable_relative_path, AttachmentFileV1, AttachmentManifestV1};

pub(crate) const ATTACHMENT_REF_UNSAFE: &str = "writing_attachment_ref_unsafe";
pub(crate) const ATTACHMENT_UNREADABLE: &str = "writing_attachment_unreadable";
pub(crate) const ATTACHMENT_HASH_MISMATCH: &str = "writing_attachment_hash_mismatch";
pub(crate) const ATTACHMENT_EXISTS_DIFFERENT: &str = "writing_attachment_exists_different";
pub(crate) const ATTACHMENT_INVALID_ENTRY: &str = "writing_attachment_invalid_entry";

/// The two directories a document may reference. Everything else is rejected
/// so a synced document can never point at arbitrary local files.
const ALLOWED_ROOTS: [&str; 2] = ["writing-images", "writing-crops"];

/// Why one referenced file could not be proven transferable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AttachmentIssue {
    UnsafeReference,
    MalformedReference,
    MissingFile,
    UnreadableFile,
    UnknownMediaType,
    UnsupportedMediaPath,
    HashNameMismatch,
}

/// One unresolved reference. The document stays `PreparationRequired` while
/// any of these exists, so a manuscript with images is never silently
/// stripped of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnresolvedAttachment {
    pub(crate) reference: String,
    pub(crate) issue: AttachmentIssue,
}

/// The outcome of scanning one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachmentScanReport {
    pub(crate) manifest: AttachmentManifestV1,
    pub(crate) unresolved: Vec<UnresolvedAttachment>,
}

impl AttachmentScanReport {
    pub(crate) fn is_transfer_ready(&self) -> bool {
        self.manifest.is_transfer_ready() && self.unresolved.is_empty()
    }
}

/// Result of installing one verified attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachmentInstallOutcome {
    Installed,
    AlreadyInstalled,
}

/// Why an envelope's manifest is not proven by the files currently installed
/// under the local data root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttachmentManifestIssue {
    ManifestNotValidated,
    UnresolvedReferences(Vec<UnresolvedAttachment>),
    ManifestMismatch,
}

impl fmt::Display for AttachmentManifestIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManifestNotValidated => {
                formatter.write_str("attachment manifest still requires preparation")
            }
            Self::UnresolvedReferences(unresolved) => {
                let Some(first) = unresolved.first() else {
                    return formatter.write_str("attachment references could not be verified");
                };
                write!(
                    formatter,
                    "{} attachment reference(s) could not be verified; first is {:?} ({:?})",
                    unresolved.len(),
                    first.reference,
                    first.issue
                )
            }
            Self::ManifestMismatch => formatter.write_str(
                "attachment manifest does not exactly match the document references and local files",
            ),
        }
    }
}

// ─────────────────────────────── scan ───────────────────────────────

/// Hashes every file the document references under `data_root`.
///
/// `source_regions` carries parsed `source_region_json` values. No independent
/// region attachment schema is established, so those values are scanned only
/// for the same two recognized shapes as document content. Arbitrary `source`
/// strings are deliberately ignored rather than guessed to be file references.
pub(crate) fn scan_document_attachments(
    content_json: &Value,
    source_regions: &[Value],
    data_root: &Path,
) -> AttachmentScanReport {
    let mut references: BTreeSet<String> = BTreeSet::new();
    let mut unresolved: Vec<UnresolvedAttachment> = Vec::new();

    collect_references(content_json, &mut references, &mut unresolved);
    for region in source_regions {
        collect_references(region, &mut references, &mut unresolved);
    }

    let mut files: Vec<AttachmentFileV1> = Vec::new();
    for reference in &references {
        match scan_one(reference, data_root) {
            Ok(file) => files.push(file),
            Err(issue) => unresolved.push(UnresolvedAttachment {
                reference: reference.clone(),
                issue,
            }),
        }
    }

    unresolved.sort_by(|a, b| a.reference.cmp(&b.reference));

    let manifest = if unresolved.is_empty() {
        // `files` is already sorted: `references` is a BTreeSet.
        AttachmentManifestV1::Validated { files }
    } else {
        AttachmentManifestV1::PreparationRequired
    };

    AttachmentScanReport {
        manifest,
        unresolved,
    }
}

/// Proves that a validated manifest names exactly every attachment reference
/// and that every named local file matches its hash, size, media type and
/// supported path. A manifest is evidence only for this local verification;
/// its presence alone says nothing about remote availability.
pub(crate) fn verify_attachment_manifest(
    content_json: &Value,
    source_regions: &[Value],
    data_root: &Path,
    manifest: &AttachmentManifestV1,
) -> Result<(), AttachmentManifestIssue> {
    if !matches!(manifest, AttachmentManifestV1::Validated { .. }) {
        return Err(AttachmentManifestIssue::ManifestNotValidated);
    }

    let scan = scan_document_attachments(content_json, source_regions, data_root);
    if !scan.unresolved.is_empty() {
        return Err(AttachmentManifestIssue::UnresolvedReferences(
            scan.unresolved,
        ));
    }
    if manifests_match(&scan.manifest, manifest) {
        Ok(())
    } else {
        Err(AttachmentManifestIssue::ManifestMismatch)
    }
}

/// Walks a JSON tree adding `writingImage` node `attrs.src` values and
/// `quotedParts` image `source` values — the same two shapes the frontend
/// export walker (`export-images.ts`) collects. Other source-like strings are
/// not attachment evidence.
fn collect_references(
    value: &Value,
    references: &mut BTreeSet<String>,
    unresolved: &mut Vec<UnresolvedAttachment>,
) {
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("writingImage") {
                match map
                    .get("attrs")
                    .and_then(Value::as_object)
                    .and_then(|attrs| attrs.get("src"))
                    .and_then(Value::as_str)
                    .filter(|src| !src.is_empty())
                {
                    Some(src) => {
                        references.insert(src.to_string());
                    }
                    None => unresolved.push(UnresolvedAttachment {
                        reference: "<writingImage without string src>".to_string(),
                        issue: AttachmentIssue::MalformedReference,
                    }),
                }
            }
            if let Some(parts) = map
                .get("attrs")
                .and_then(Value::as_object)
                .and_then(|attrs| attrs.get("quotedParts"))
                .and_then(Value::as_array)
            {
                for part in parts {
                    let (kind, source) = (
                        part.get("kind").and_then(Value::as_str),
                        part.get("source").and_then(Value::as_str),
                    );
                    match (kind, source) {
                        (Some("image"), Some(source)) if !source.is_empty() => {
                            references.insert(source.to_string());
                        }
                        (Some("image"), _) => {
                            unresolved.push(UnresolvedAttachment {
                                reference: "<image part without non-empty string source>"
                                    .to_string(),
                                issue: AttachmentIssue::MalformedReference,
                            });
                        }
                        _ => {}
                    }
                }
            }
            for child in map.values() {
                collect_references(child, references, unresolved);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_references(child, references, unresolved);
            }
        }
        _ => {}
    }
}

fn scan_one(reference: &str, data_root: &Path) -> Result<AttachmentFileV1, AttachmentIssue> {
    let relative = strict_relative_path(reference)?;
    let facts = read_attachment_facts(data_root, &relative)?;
    if !path_supports_media_type(&relative, facts.media_type) {
        return Err(AttachmentIssue::UnsupportedMediaPath);
    }

    // Manuscript images are content-addressed by the frontend importer. Both
    // the name and extension must agree with the bytes before the path is proof.
    if relative.starts_with("writing-images/") {
        let expected_name = format!(
            "{}.{}",
            facts.sha256,
            extension_for_media_type(facts.media_type).unwrap_or_default()
        );
        if relative.rsplit('/').next() != Some(expected_name.as_str()) {
            return Err(AttachmentIssue::HashNameMismatch);
        }
    }

    Ok(AttachmentFileV1 {
        sha256: facts.sha256,
        rel_path: relative,
        size: facts.size,
        media_type: facts.media_type.to_string(),
    })
}

struct AttachmentFileFacts {
    sha256: String,
    size: u64,
    media_type: &'static str,
}

/// Reads one regular attachment with a fixed-size buffer. Every read-only
/// proof path uses this function, so scans, incoming verification and existing
/// install checks apply the same byte-level rules without loading whole files.
fn read_attachment_facts(
    data_root: &Path,
    relative: &str,
) -> Result<AttachmentFileFacts, AttachmentIssue> {
    let absolute = safe_existing_attachment_path(data_root, relative)?;
    let mut file = fs::File::open(&absolute).map_err(issue_for_io)?;
    if !file.metadata().map_err(issue_for_io)?.is_file() {
        return Err(AttachmentIssue::UnsafeReference);
    }

    let mut hasher = Sha256::new();
    let mut prefix = [0u8; 12];
    let mut prefix_len = 0usize;
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(issue_for_io)?;
        if read == 0 {
            break;
        }
        let prefix_read = read.min(prefix.len().saturating_sub(prefix_len));
        if prefix_read != 0 {
            prefix[prefix_len..prefix_len + prefix_read].copy_from_slice(&buffer[..prefix_read]);
            prefix_len += prefix_read;
        }
        hasher.update(&buffer[..read]);
        size = size
            .checked_add(read as u64)
            .ok_or(AttachmentIssue::UnreadableFile)?;
    }

    let media_type =
        detect_media_type(&prefix[..prefix_len]).ok_or(AttachmentIssue::UnknownMediaType)?;
    Ok(AttachmentFileFacts {
        sha256: hex_digest(hasher.finalize()),
        size,
        media_type,
    })
}

// ────────────────────────────── install ──────────────────────────────

/// Streams one manifest entry's bytes into the data root after verifying
/// hash, size and media type. Idempotent for an exact existing file; never
/// replaces an existing destination, including one created during publication.
///
/// Directory and file checks reject links and Windows reparse points observed
/// at validation time. They are path based, so a hostile process that can swap
/// components between a check and a later open remains outside this guarantee;
/// fully closing that race requires handle-relative traversal from a trusted
/// directory handle on every supported platform.
pub(crate) fn install_verified_attachment(
    data_root: &Path,
    entry: &AttachmentFileV1,
    bytes: &mut impl Read,
) -> WritingResult<AttachmentInstallOutcome> {
    install_verified_attachment_inner(data_root, entry, bytes, |_, _| {})
}

/// Test-only seam for deterministically creating publication races after the
/// exclusive temporary file has been verified.
#[cfg(test)]
pub(crate) fn install_verified_attachment_with_before_publish(
    data_root: &Path,
    entry: &AttachmentFileV1,
    bytes: &mut impl Read,
    before_publish: impl FnOnce(&Path, &Path),
) -> WritingResult<AttachmentInstallOutcome> {
    install_verified_attachment_inner(data_root, entry, bytes, before_publish)
}

fn install_verified_attachment_inner<R, F>(
    data_root: &Path,
    entry: &AttachmentFileV1,
    bytes: &mut R,
    before_publish: F,
) -> WritingResult<AttachmentInstallOutcome>
where
    R: Read + ?Sized,
    F: FnOnce(&Path, &Path),
{
    let relative = strict_relative_path(&entry.rel_path).map_err(|_| {
        WritingError::new(
            ATTACHMENT_REF_UNSAFE,
            format!(
                "attachment path {:?} is not a safe portable path",
                entry.rel_path
            ),
        )
    })?;
    validate_install_entry(&relative, entry)?;

    let (attachment_root, file_name) = relative.split_once('/').ok_or_else(|| {
        WritingError::new(
            ATTACHMENT_REF_UNSAFE,
            format!("attachment path {relative:?} has no managed directory"),
        )
    })?;
    let target_directory = ensure_safe_attachment_directory(data_root, attachment_root)?;
    let target = target_directory.join(file_name);

    if let Some(outcome) = existing_attachment_outcome(data_root, &relative, entry)? {
        return Ok(outcome);
    }

    let mut temporary = TempFileBuilder::new()
        .prefix(".writing-sync-")
        .suffix(".part")
        .tempfile_in(&target_directory)
        .map_err(|error| {
            WritingError::new(
                ATTACHMENT_UNREADABLE,
                format!(
                    "cannot exclusively create an attachment temporary file in {target_directory:?}: {error}"
                ),
            )
        })?;
    let temporary_path = temporary.path().to_path_buf();
    write_part_and_verify(temporary.as_file_mut(), &temporary_path, entry, bytes)?;

    before_publish(&temporary_path, &target);

    // `persist_noclobber` uses no-replace rename where available and a
    // hard-link/unlink fallback elsewhere. It never replaces `target`; an
    // unsupported filesystem therefore fails closed instead of using rename.
    match temporary.persist_noclobber(&target) {
        Ok(_persisted) => Ok(AttachmentInstallOutcome::Installed),
        Err(error) => {
            let publish_error = error.error.to_string();
            let owned_temporary = error.file;
            let collision = existing_attachment_outcome(data_root, &relative, entry);
            drop(owned_temporary);

            match collision {
                Ok(Some(outcome)) => Ok(outcome),
                Err(error) => Err(error),
                Ok(None) => Err(WritingError::new(
                    ATTACHMENT_UNREADABLE,
                    format!(
                        "cannot publish attachment {relative} without replacing an existing path: {publish_error}"
                    ),
                )),
            }
        }
    }
}

fn validate_install_entry(relative: &str, entry: &AttachmentFileV1) -> WritingResult<()> {
    if !is_sha256(&entry.sha256)
        || extension_for_media_type(&entry.media_type).is_none()
        || !path_supports_media_type(relative, &entry.media_type)
    {
        return Err(WritingError::new(
            ATTACHMENT_INVALID_ENTRY,
            "attachment entry requires a lowercase sha256 and a supported media type/path",
        ));
    }

    if relative.starts_with("writing-images/") {
        let expected_name = format!(
            "{}.{}",
            entry.sha256,
            extension_for_media_type(&entry.media_type).unwrap_or_default()
        );
        if relative.rsplit('/').next() != Some(expected_name.as_str()) {
            return Err(WritingError::new(
                ATTACHMENT_INVALID_ENTRY,
                format!(
                    "content-addressed attachment path {relative:?} does not match its hash and media type"
                ),
            ));
        }
    }

    Ok(())
}

fn ensure_safe_attachment_directory(
    data_root: &Path,
    attachment_root: &str,
) -> WritingResult<PathBuf> {
    validate_physical_component(data_root, true)
        .map_err(|issue| install_issue(data_root, issue))?;

    let directory = data_root.join(attachment_root);
    match fs::symlink_metadata(&directory) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(WritingError::new(
                    ATTACHMENT_UNREADABLE,
                    format!("cannot create attachment directory {directory:?}: {error}"),
                ));
            }
        },
        Err(error) => {
            return Err(WritingError::new(
                ATTACHMENT_UNREADABLE,
                format!("cannot inspect attachment directory {directory:?}: {error}"),
            ));
        }
    }
    validate_physical_component(&directory, true)
        .map_err(|issue| install_issue(&directory, issue))?;

    let canonical_root = fs::canonicalize(data_root).map_err(|error| {
        WritingError::new(
            ATTACHMENT_UNREADABLE,
            format!("cannot resolve data root {data_root:?}: {error}"),
        )
    })?;
    let canonical_directory = fs::canonicalize(&directory).map_err(|error| {
        WritingError::new(
            ATTACHMENT_UNREADABLE,
            format!("cannot resolve attachment directory {directory:?}: {error}"),
        )
    })?;
    if canonical_directory.parent() != Some(canonical_root.as_path()) {
        return Err(WritingError::new(
            ATTACHMENT_REF_UNSAFE,
            format!(
                "attachment directory {directory:?} does not resolve directly beneath the data root"
            ),
        ));
    }

    Ok(directory)
}

fn existing_attachment_outcome(
    data_root: &Path,
    relative: &str,
    entry: &AttachmentFileV1,
) -> WritingResult<Option<AttachmentInstallOutcome>> {
    let facts = match read_attachment_facts(data_root, relative) {
        Ok(facts) => facts,
        Err(AttachmentIssue::MissingFile) => return Ok(None),
        Err(AttachmentIssue::UnsafeReference) => {
            return Err(WritingError::new(
                ATTACHMENT_REF_UNSAFE,
                format!("existing attachment {relative:?} is not a safe regular file"),
            ));
        }
        Err(AttachmentIssue::UnreadableFile) => {
            return Err(WritingError::new(
                ATTACHMENT_UNREADABLE,
                format!("existing attachment {relative:?} cannot be read safely"),
            ));
        }
        Err(issue) => {
            return Err(WritingError::new(
                ATTACHMENT_EXISTS_DIFFERENT,
                format!("existing attachment {relative:?} is corrupt or unsupported ({issue:?})"),
            ));
        }
    };

    if facts.sha256 == entry.sha256
        && facts.size == entry.size
        && facts.media_type == entry.media_type
        && path_supports_media_type(relative, facts.media_type)
    {
        Ok(Some(AttachmentInstallOutcome::AlreadyInstalled))
    } else {
        Err(WritingError::new(
            ATTACHMENT_EXISTS_DIFFERENT,
            format!("attachment {relative} exists but does not exactly match its manifest entry"),
        ))
    }
}

fn install_issue(path: &Path, issue: AttachmentIssue) -> WritingError {
    let code = match issue {
        AttachmentIssue::UnsafeReference => ATTACHMENT_REF_UNSAFE,
        _ => ATTACHMENT_UNREADABLE,
    };
    WritingError::new(
        code,
        format!("attachment path {path:?} failed physical validation ({issue:?})"),
    )
}

/// Streams at most the declared size plus one probe byte into an already
/// exclusive temporary file, then verifies and durably flushes valid content.
fn write_part_and_verify(
    file: &mut fs::File,
    part_path: &Path,
    entry: &AttachmentFileV1,
    bytes: &mut (impl Read + ?Sized),
) -> WritingResult<()> {
    let mut hasher = Sha256::new();
    let mut prefix = [0u8; 12];
    let mut prefix_len = 0usize;
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let remaining = entry.size - total;
        let read_limit = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let read = bytes
            .read(&mut buffer[..read_limit])
            .map_err(|error| WritingError::new(ATTACHMENT_UNREADABLE, error.to_string()))?;
        if read == 0 {
            break;
        }
        if read as u64 > remaining {
            return Err(WritingError::new(
                ATTACHMENT_HASH_MISMATCH,
                format!(
                    "attachment {} is larger than its manifest size",
                    entry.rel_path
                ),
            ));
        }

        let prefix_read = read.min(prefix.len().saturating_sub(prefix_len));
        if prefix_read != 0 {
            prefix[prefix_len..prefix_len + prefix_read].copy_from_slice(&buffer[..prefix_read]);
            prefix_len += prefix_read;
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read]).map_err(|error| {
            WritingError::new(
                ATTACHMENT_UNREADABLE,
                format!("cannot write {part_path:?}: {error}"),
            )
        })?;
        total += read as u64;
    }

    if total != entry.size {
        return Err(WritingError::new(
            ATTACHMENT_HASH_MISMATCH,
            format!(
                "attachment {} has size {total}, manifest says {}",
                entry.rel_path, entry.size
            ),
        ));
    }
    let digest = hex_digest(hasher.finalize());
    if digest != entry.sha256 {
        return Err(WritingError::new(
            ATTACHMENT_HASH_MISMATCH,
            format!(
                "attachment {} does not match its manifest hash",
                entry.rel_path
            ),
        ));
    }
    let detected = detect_media_type(&prefix[..prefix_len]).unwrap_or_default();
    if detected != entry.media_type {
        return Err(WritingError::new(
            ATTACHMENT_INVALID_ENTRY,
            format!(
                "attachment {} declares media type {:?} but its bytes are {:?}",
                entry.rel_path, entry.media_type, detected
            ),
        ));
    }

    file.flush().map_err(|error| {
        WritingError::new(
            ATTACHMENT_UNREADABLE,
            format!("cannot flush {part_path:?}: {error}"),
        )
    })?;
    file.sync_all().map_err(|error| {
        WritingError::new(
            ATTACHMENT_UNREADABLE,
            format!("cannot sync {part_path:?}: {error}"),
        )
    })?;
    Ok(())
}

// ─────────────────────────────── helpers ───────────────────────────────

fn manifests_match(scanned: &AttachmentManifestV1, claimed: &AttachmentManifestV1) -> bool {
    let (
        AttachmentManifestV1::Validated {
            files: scanned_files,
        },
        AttachmentManifestV1::Validated {
            files: claimed_files,
        },
    ) = (scanned, claimed)
    else {
        return false;
    };

    manifest_files_by_path(scanned_files)
        .zip(manifest_files_by_path(claimed_files))
        .is_some_and(|(scanned, claimed)| scanned == claimed)
}

fn manifest_files_by_path(files: &[AttachmentFileV1]) -> Option<BTreeMap<&str, &AttachmentFileV1>> {
    let mut by_path = BTreeMap::new();
    for file in files {
        if by_path.insert(file.rel_path.as_str(), file).is_some() {
            return None;
        }
    }
    Some(by_path)
}

/// Resolves an existing attachment only through physical directories and a
/// regular final file. The canonical containment check is defense in depth;
/// each writable component is also rejected when it is a link/reparse point.
/// This detects aliases present during validation but cannot bind the later
/// file open to those checks; that needs handle-relative no-follow operations.
fn safe_existing_attachment_path(
    data_root: &Path,
    relative: &str,
) -> Result<PathBuf, AttachmentIssue> {
    validate_physical_component(data_root, true)?;

    let (attachment_root, file_name) = relative
        .split_once('/')
        .ok_or(AttachmentIssue::UnsafeReference)?;
    let attachment_dir = data_root.join(attachment_root);
    validate_physical_component(&attachment_dir, true)?;

    let target = attachment_dir.join(file_name);
    validate_physical_component(&target, false)?;

    let canonical_root = fs::canonicalize(data_root).map_err(issue_for_io)?;
    let canonical_target = fs::canonicalize(&target).map_err(issue_for_io)?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(AttachmentIssue::UnsafeReference);
    }

    Ok(target)
}

fn validate_physical_component(path: &Path, directory: bool) -> Result<(), AttachmentIssue> {
    let metadata = fs::symlink_metadata(path).map_err(issue_for_io)?;
    if is_link_or_reparse_point(&metadata)
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(AttachmentIssue::UnsafeReference);
    }
    Ok(())
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

fn issue_for_io(error: io::Error) -> AttachmentIssue {
    match error.kind() {
        io::ErrorKind::NotFound => AttachmentIssue::MissingFile,
        _ => AttachmentIssue::UnreadableFile,
    }
}

fn path_supports_media_type(relative: &str, media_type: &str) -> bool {
    let extension = Path::new(relative)
        .extension()
        .and_then(|value| value.to_str());
    match media_type {
        "image/png" => extension == Some("png"),
        "image/jpeg" => matches!(extension, Some("jpg" | "jpeg")),
        "image/gif" => extension == Some("gif"),
        "image/webp" => extension == Some("webp"),
        _ => false,
    }
}

fn extension_for_media_type(media_type: &str) -> Option<&'static str> {
    match media_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// The portable path rules for real attachment references: exactly
/// `writing-images/<file>` or `writing-crops/<file>`, plain ASCII names, no
/// traversal, no absolute paths, no percent-encoded tricks.
fn strict_relative_path(reference: &str) -> Result<String, AttachmentIssue> {
    if !is_portable_relative_path(reference) {
        return Err(AttachmentIssue::UnsafeReference);
    }
    let mut components = reference.split('/');
    let root = components.next().unwrap_or_default();
    let name = components.next().unwrap_or_default();
    if components.next().is_some()
        || !ALLOWED_ROOTS.contains(&root)
        || name.is_empty()
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(AttachmentIssue::UnsafeReference);
    }
    Ok(format!("{root}/{name}"))
}

fn detect_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && bytes[..8] == [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a] {
        return Some("image/png");
    }
    if bytes.len() >= 3 && bytes[..3] == [0xff, 0xd8, 0xff] {
        return Some("image/jpeg");
    }
    if bytes.len() >= 6 && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
