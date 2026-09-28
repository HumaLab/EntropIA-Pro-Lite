//! Tests for the offline attachment file layer. Every case runs against a
//! real temporary data root: hashing, atomic install and collision behavior
//! are exactly what the tests must exercise.

use std::collections::BTreeSet;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

use super::sync_envelope::{AttachmentFileV1, AttachmentManifestV1};
use super::sync_files::{
    install_verified_attachment, install_verified_attachment_with_before_publish,
    scan_document_attachments, verify_attachment_manifest, AttachmentInstallOutcome,
    AttachmentIssue, AttachmentManifestIssue, AttachmentScanReport, ATTACHMENT_EXISTS_DIFFERENT,
    ATTACHMENT_HASH_MISMATCH, ATTACHMENT_INVALID_ENTRY, ATTACHMENT_REF_UNSAFE,
};

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A real, self-deleting data root under the OS temp directory.
struct TempDataRoot {
    path: PathBuf,
}

impl TempDataRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "writing-sync-files-{}-{}",
            std::process::id(),
            DIR_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("temp data root");
        Self { path }
    }

    fn write(&self, rel_path: &str, bytes: &[u8]) {
        let target = self.path.join(rel_path);
        fs::create_dir_all(target.parent().expect("parent")).expect("parent dir");
        fs::write(target, bytes).expect("fixture file");
    }
}

impl Drop for TempDataRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(unix)]
fn create_directory_alias(target: &Path, alias: &Path) {
    std::os::unix::fs::symlink(target, alias).expect("symlink fixture");
}

#[cfg(windows)]
fn create_directory_alias(target: &Path, alias: &Path) {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(alias)
        .arg(target)
        .output()
        .expect("junction command");
    assert!(
        output.status.success(),
        "failed to create junction fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn file_names(directory: &Path) -> BTreeSet<String> {
    fs::read_dir(directory)
        .expect("fixture directory")
        .map(|entry| {
            entry
                .expect("fixture entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn png_bytes(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend_from_slice(payload);
    bytes
}

fn hash_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn image_node(src: &str) -> Value {
    json!({ "type": "writingImage", "attrs": { "src": src, "alt": null, "title": null } })
}

fn citation_with_parts(parts: Vec<Value>) -> Value {
    json!({ "type": "documentCitation", "attrs": { "quotedText": "x", "quotedParts": parts } })
}

fn image_part(source: &str) -> Value {
    json!({ "kind": "image", "source": source })
}

fn document_with(nodes: Vec<Value>) -> Value {
    json!({ "type": "doc", "content": nodes })
}

fn files_of(report: &AttachmentScanReport) -> Vec<(String, String, u64, String)> {
    match &report.manifest {
        AttachmentManifestV1::Validated { files } => files
            .iter()
            .map(|file| {
                (
                    file.rel_path.clone(),
                    file.sha256.clone(),
                    file.size,
                    file.media_type.clone(),
                )
            })
            .collect(),
        AttachmentManifestV1::PreparationRequired => Vec::new(),
    }
}

fn issues(report: &AttachmentScanReport) -> BTreeSet<(String, AttachmentIssue)> {
    report
        .unresolved
        .iter()
        .map(|entry| (entry.reference.clone(), entry.issue))
        .collect()
}

#[test]
fn scan_hashes_images_and_crops_into_sorted_validated_manifest() {
    let root = TempDataRoot::new();
    let png = png_bytes(b"image-bytes");
    let crop = png_bytes(b"crop-bytes");
    root.write("writing-images/aaa.png", &png);
    root.write("writing-crops/crop-1.png", &crop);
    // Rename the content-addressed image to its true hash name.
    let true_name = format!("writing-images/{}.png", hash_of(&png));
    fs::rename(
        root.path.join("writing-images/aaa.png"),
        root.path.join(&true_name),
    )
    .expect("rename to content-addressed name");

    let doc = document_with(vec![image_node(&true_name)]);
    let region = json!({
        "attrs": {
            "quotedParts": [
                { "kind": "text", "source": "palabras" },
                image_part("writing-crops/crop-1.png")
            ]
        }
    });

    let report = scan_document_attachments(&doc, &[region], &root.path);
    assert!(
        report.is_transfer_ready(),
        "unresolved: {:?}",
        report.unresolved
    );
    let files = files_of(&report);
    assert_eq!(files.len(), 2, "duplicate refs must collapse: {files:?}");
    let rel_paths: Vec<&str> = files.iter().map(|(rel, ..)| rel.as_str()).collect();
    assert_eq!(
        rel_paths,
        vec!["writing-crops/crop-1.png", &true_name],
        "sorted by rel_path for deterministic fingerprints"
    );
    let crop_entry = &files[0];
    assert_eq!(crop_entry.1, hash_of(&crop));
    assert_eq!(crop_entry.2, crop.len() as u64);
    assert_eq!(crop_entry.3, "image/png");
}

#[test]
fn missing_unsafe_and_corrupt_references_keep_the_document_not_transfer_ready() {
    let root = TempDataRoot::new();
    let png = png_bytes(b"wrong-name-content");
    let wrong_name = format!("writing-images/{}.png", "f".repeat(64));
    root.write(&wrong_name, &png);

    let doc = document_with(vec![
        image_node(
            "writing-images/0000000000000000000000000000000000000000000000000000000000000000.png",
        ),
        image_node(&wrong_name),
        image_node("../escape.png"),
        image_node("/etc/passwd"),
        image_node(r"writing-images\win.png"),
        image_node("https://example.invalid/x.png"),
        image_node("writing-images/%2e%2e/x.png"),
        image_node("other-dir/file.png"),
        citation_with_parts(vec![json!({ "kind": "image" })]),
    ]);

    let report = scan_document_attachments(&doc, &[], &root.path);
    assert!(!report.is_transfer_ready());
    assert!(matches!(
        report.manifest,
        AttachmentManifestV1::PreparationRequired
    ));
    let found = issues(&report);
    assert!(found.contains(&(
        "writing-images/0000000000000000000000000000000000000000000000000000000000000000.png"
            .to_string(),
        AttachmentIssue::MissingFile
    )));
    assert!(found.contains(&(wrong_name, AttachmentIssue::HashNameMismatch)));
    for unsafe_ref in [
        "../escape.png",
        "/etc/passwd",
        r"writing-images\win.png",
        "https://example.invalid/x.png",
        "writing-images/%2e%2e/x.png",
        "other-dir/file.png",
    ] {
        assert!(
            found.contains(&(unsafe_ref.to_string(), AttachmentIssue::UnsafeReference)),
            "{unsafe_ref} must be unsafe: {found:?}"
        );
    }
    assert!(found.contains(&(
        "<image part without non-empty string source>".to_string(),
        AttachmentIssue::MalformedReference
    )));
}

#[test]
fn scan_rejects_missing_non_string_and_empty_writing_image_sources() {
    let root = TempDataRoot::new();
    let document = document_with(vec![
        json!({ "type": "writingImage", "attrs": {} }),
        json!({ "type": "writingImage", "attrs": { "src": 7 } }),
        json!({ "type": "writingImage", "attrs": { "src": "" } }),
    ]);

    let report = scan_document_attachments(&document, &[], &root.path);

    assert!(matches!(
        report.manifest,
        AttachmentManifestV1::PreparationRequired
    ));
    assert_eq!(report.unresolved.len(), 3);
    assert!(report
        .unresolved
        .iter()
        .all(|entry| entry.issue == AttachmentIssue::MalformedReference));
}

#[test]
fn scan_rejects_data_url_image_sources() {
    let root = TempDataRoot::new();
    let source = "data:image/png;base64,iVBORw0KGgo=";
    let document = document_with(vec![image_node(source)]);

    let report = scan_document_attachments(&document, &[], &root.path);

    assert!(issues(&report).contains(&(source.to_string(), AttachmentIssue::UnsafeReference)));
}

#[test]
fn scan_ignores_arbitrary_region_source_strings_that_are_not_image_reference_shapes() {
    let root = TempDataRoot::new();
    let content = document_with(Vec::new());
    let region = json!({
        "kind": "text",
        "source": "writing-crops/citation-text.png",
        "metadata": { "source": "writing-images/not-an-attachment.png" }
    });

    let report = scan_document_attachments(&content, &[region], &root.path);

    assert!(report.is_transfer_ready());
    assert!(files_of(&report).is_empty());
}

#[test]
fn scan_rejects_a_detected_media_type_with_the_wrong_extension() {
    let root = TempDataRoot::new();
    let bytes = png_bytes(b"wrong-extension");
    let rel_path = "writing-crops/wrong-extension.jpg";
    root.write(rel_path, &bytes);
    let doc = document_with(vec![image_node(rel_path)]);

    let report = scan_document_attachments(&doc, &[], &root.path);

    assert!(
        issues(&report).contains(&(rel_path.to_string(), AttachmentIssue::UnsupportedMediaPath))
    );
}

#[test]
fn document_without_references_has_an_empty_validated_manifest() {
    let root = TempDataRoot::new();
    let doc = document_with(vec![citation_with_parts(vec![
        json!({ "kind": "text", "source": "solo texto" }),
    ])]);
    let report = scan_document_attachments(&doc, &[], &root.path);
    assert!(report.is_transfer_ready());
    assert!(files_of(&report).is_empty());
    verify_attachment_manifest(&doc, &[], &root.path, &report.manifest)
        .expect("empty validated manifest covers a document without references");
}

#[test]
fn verifier_accepts_exact_content_and_region_coverage_regardless_of_manifest_order() {
    let root = TempDataRoot::new();
    let image = png_bytes(b"manifest-image");
    let crop = png_bytes(b"manifest-crop");
    let image_path = format!("writing-images/{}.png", hash_of(&image));
    let crop_path = "writing-crops/manifest-crop.png";
    root.write(&image_path, &image);
    root.write(crop_path, &crop);
    let content = document_with(vec![image_node(&image_path)]);
    let regions = [json!({
        "attrs": { "quotedParts": [image_part(crop_path)] }
    })];
    let report = scan_document_attachments(&content, &regions, &root.path);
    let AttachmentManifestV1::Validated { mut files } = report.manifest else {
        panic!("exact fixture must scan to a validated manifest");
    };
    files.reverse();
    let manifest = AttachmentManifestV1::Validated { files };

    verify_attachment_manifest(&content, &regions, &root.path, &manifest)
        .expect("the exact file set is independent of manifest order");
}

#[test]
fn verifier_rejects_incomplete_manifest_coverage() {
    let root = TempDataRoot::new();
    let first = png_bytes(b"manifest-first");
    let second = png_bytes(b"manifest-second");
    let first_path = format!("writing-images/{}.png", hash_of(&first));
    let second_path = format!("writing-images/{}.png", hash_of(&second));
    root.write(&first_path, &first);
    root.write(&second_path, &second);
    let content = document_with(vec![image_node(&first_path), image_node(&second_path)]);
    let report = scan_document_attachments(&content, &[], &root.path);
    let AttachmentManifestV1::Validated { mut files } = report.manifest else {
        panic!("complete fixture must scan to a validated manifest");
    };
    files.pop();

    assert_eq!(
        verify_attachment_manifest(
            &content,
            &[],
            &root.path,
            &AttachmentManifestV1::Validated { files }
        ),
        Err(AttachmentManifestIssue::ManifestMismatch)
    );
}

#[test]
fn verifier_rejects_path_hash_size_and_media_mismatches() {
    let root = TempDataRoot::new();
    let bytes = png_bytes(b"manifest-fields");
    let rel_path = format!("writing-images/{}.png", hash_of(&bytes));
    root.write(&rel_path, &bytes);
    let content = document_with(vec![image_node(&rel_path)]);
    let report = scan_document_attachments(&content, &[], &root.path);
    let AttachmentManifestV1::Validated { files } = report.manifest else {
        panic!("fixture must scan to a validated manifest");
    };
    let exact = files[0].clone();
    let mismatches = [
        AttachmentFileV1 {
            rel_path: "writing-images/other.png".to_string(),
            ..exact.clone()
        },
        AttachmentFileV1 {
            sha256: "0".repeat(64),
            ..exact.clone()
        },
        AttachmentFileV1 {
            size: exact.size + 1,
            ..exact.clone()
        },
        AttachmentFileV1 {
            media_type: "image/jpeg".to_string(),
            ..exact
        },
    ];

    for mismatch in mismatches {
        assert_eq!(
            verify_attachment_manifest(
                &content,
                &[],
                &root.path,
                &AttachmentManifestV1::Validated {
                    files: vec![mismatch]
                }
            ),
            Err(AttachmentManifestIssue::ManifestMismatch)
        );
    }
}

#[test]
fn verifier_rejects_duplicate_manifest_paths() {
    let root = TempDataRoot::new();
    let bytes = png_bytes(b"manifest-duplicate");
    let rel_path = format!("writing-images/{}.png", hash_of(&bytes));
    root.write(&rel_path, &bytes);
    let content = document_with(vec![image_node(&rel_path)]);
    let report = scan_document_attachments(&content, &[], &root.path);
    let AttachmentManifestV1::Validated { files } = report.manifest else {
        panic!("fixture must scan to a validated manifest");
    };

    assert_eq!(
        verify_attachment_manifest(
            &content,
            &[],
            &root.path,
            &AttachmentManifestV1::Validated {
                files: vec![files[0].clone(), files[0].clone()]
            }
        ),
        Err(AttachmentManifestIssue::ManifestMismatch)
    );
}

#[test]
fn verifier_requires_a_validated_manifest_even_without_references() {
    let root = TempDataRoot::new();
    let content = document_with(Vec::new());

    assert_eq!(
        verify_attachment_manifest(
            &content,
            &[],
            &root.path,
            &AttachmentManifestV1::PreparationRequired
        ),
        Err(AttachmentManifestIssue::ManifestNotValidated)
    );
}

#[cfg(any(unix, windows))]
#[test]
fn scan_rejects_a_link_or_reparse_attachment_directory() {
    let outside = TempDataRoot::new();
    let root = TempDataRoot::new();
    let bytes = png_bytes(b"outside");
    let rel_path = format!("writing-images/{}.png", hash_of(&bytes));
    outside.write(&rel_path, &bytes);
    create_directory_alias(
        &outside.path.join("writing-images"),
        &root.path.join("writing-images"),
    );
    let content = document_with(vec![image_node(&rel_path)]);

    let report = scan_document_attachments(&content, &[], &root.path);

    assert!(issues(&report).contains(&(rel_path, AttachmentIssue::UnsafeReference)));
}

#[test]
fn scan_order_is_deterministic_across_reference_orderings() {
    let root = TempDataRoot::new();
    let png_a = png_bytes(b"a");
    let png_b = png_bytes(b"b");
    let name_a = format!("writing-images/{}.png", hash_of(&png_a));
    let name_b = format!("writing-images/{}.png", hash_of(&png_b));
    root.write(&name_a, &png_a);
    root.write(&name_b, &png_b);

    let first = scan_document_attachments(
        &document_with(vec![image_node(&name_a), image_node(&name_b)]),
        &[],
        &root.path,
    );
    let second = scan_document_attachments(
        &document_with(vec![image_node(&name_b), image_node(&name_a)]),
        &[],
        &root.path,
    );
    assert_eq!(
        first.manifest, second.manifest,
        "order must not leak into the manifest"
    );
}

#[test]
fn install_writes_then_verifies_and_reinstalls_identical_content() {
    let root = TempDataRoot::new();
    let bytes = png_bytes(b"install-me");
    let rel_path = format!("writing-images/{}.png", hash_of(&bytes));
    let entry = AttachmentFileV1 {
        sha256: hash_of(&bytes),
        rel_path: rel_path.clone(),
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    };

    let outcome = install_verified_attachment(&root.path, &entry, &mut bytes.as_slice())
        .expect("first install");
    assert_eq!(outcome, AttachmentInstallOutcome::Installed);
    assert_eq!(
        fs::read(root.path.join(&rel_path)).expect("installed file"),
        bytes
    );
    assert!(
        !root.path.join(format!("{rel_path}.part")).exists(),
        "no part file survives"
    );

    let again = install_verified_attachment(&root.path, &entry, &mut bytes.as_slice())
        .expect("idempotent reinstall");
    assert_eq!(again, AttachmentInstallOutcome::AlreadyInstalled);
}

#[test]
fn install_existing_same_hash_with_wrong_size_is_not_idempotent() {
    let root = TempDataRoot::new();
    let bytes = png_bytes(b"existing-size");
    let rel_path = "writing-crops/existing-size.png";
    root.write(rel_path, &bytes);
    let entry = AttachmentFileV1 {
        sha256: hash_of(&bytes),
        rel_path: rel_path.to_string(),
        size: bytes.len() as u64 + 1,
        media_type: "image/png".to_string(),
    };

    let error = install_verified_attachment(&root.path, &entry, &mut Cursor::new(Vec::new()))
        .expect_err("size metadata must be verified on the existing-file path");

    assert_eq!(error.code, ATTACHMENT_EXISTS_DIFFERENT);
    assert_eq!(fs::read(root.path.join(rel_path)).expect("original"), bytes);
}

#[test]
fn install_existing_same_hash_with_wrong_media_is_not_idempotent() {
    let root = TempDataRoot::new();
    let bytes = png_bytes(b"existing-media");
    let rel_path = "writing-crops/existing-media.jpg";
    root.write(rel_path, &bytes);
    let entry = AttachmentFileV1 {
        sha256: hash_of(&bytes),
        rel_path: rel_path.to_string(),
        size: bytes.len() as u64,
        media_type: "image/jpeg".to_string(),
    };

    let error = install_verified_attachment(&root.path, &entry, &mut Cursor::new(Vec::new()))
        .expect_err("detected media must be verified on the existing-file path");

    assert_eq!(error.code, ATTACHMENT_EXISTS_DIFFERENT);
    assert_eq!(fs::read(root.path.join(rel_path)).expect("original"), bytes);
}

#[test]
fn install_uses_an_exclusive_unique_temp_and_leaves_unrelated_parts_untouched() {
    let root = TempDataRoot::new();
    let bytes = png_bytes(b"exclusive-temp");
    let rel_path = "writing-crops/exclusive.png";
    let unrelated_path = root.path.join("writing-crops/exclusive.png.part");
    let unrelated = b"unrelated partial download";
    root.write("writing-crops/exclusive.png.part", unrelated);
    let entry = AttachmentFileV1 {
        sha256: hash_of(&bytes),
        rel_path: rel_path.to_string(),
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    };

    let outcome = install_verified_attachment_with_before_publish(
        &root.path,
        &entry,
        &mut bytes.as_slice(),
        |temporary, target| {
            assert_eq!(temporary.parent(), target.parent());
            assert_ne!(temporary, unrelated_path);
            let error = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temporary)
                .expect_err("the operation must already own its exclusive temp path");
            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        },
    )
    .expect("exclusive temp install");

    assert_eq!(outcome, AttachmentInstallOutcome::Installed);
    assert_eq!(fs::read(unrelated_path).expect("unrelated part"), unrelated);
    assert_eq!(fs::read(root.path.join(rel_path)).expect("target"), bytes);
}

#[test]
fn install_publication_collision_never_overwrites_the_racing_destination() {
    let root = TempDataRoot::new();
    let incoming = png_bytes(b"incoming-after-check");
    let racing = png_bytes(b"racing-winner");
    let rel_path = "writing-crops/publication-race.png";
    let entry = AttachmentFileV1 {
        sha256: hash_of(&incoming),
        rel_path: rel_path.to_string(),
        size: incoming.len() as u64,
        media_type: "image/png".to_string(),
    };

    let error = install_verified_attachment_with_before_publish(
        &root.path,
        &entry,
        &mut incoming.as_slice(),
        |_, target| fs::write(target, &racing).expect("racing destination"),
    )
    .expect_err("a racing destination must win without being replaced");

    assert_eq!(error.code, ATTACHMENT_EXISTS_DIFFERENT);
    assert_eq!(
        fs::read(root.path.join(rel_path)).expect("racing target"),
        racing
    );
    assert_eq!(
        file_names(&root.path.join("writing-crops")),
        BTreeSet::from(["publication-race.png".to_string()]),
        "the operation-owned temp must be cleaned after the collision"
    );
}

#[test]
fn install_hash_failure_leaves_target_absent_and_unrelated_temp_untouched() {
    let root = TempDataRoot::new();
    let real = png_bytes(b"real");
    let fake = png_bytes(b"fake");
    let rel_path = "writing-crops/crop-7.png".to_string();
    let unrelated = b"another operation";
    root.write("writing-crops/unrelated.part", unrelated);
    let entry = AttachmentFileV1 {
        sha256: hash_of(&real),
        rel_path: rel_path.clone(),
        size: real.len() as u64,
        media_type: "image/png".to_string(),
    };

    let error = install_verified_attachment(&root.path, &entry, &mut fake.as_slice())
        .expect_err("mismatched bytes");

    assert_eq!(error.code, ATTACHMENT_HASH_MISMATCH);
    assert!(!root.path.join(&rel_path).exists(), "no final file");
    assert_eq!(
        fs::read(root.path.join("writing-crops/unrelated.part")).expect("unrelated temp"),
        unrelated
    );
    assert_eq!(
        file_names(&root.path.join("writing-crops")),
        BTreeSet::from(["unrelated.part".to_string()]),
        "only the unrelated temp may remain"
    );
}

#[test]
fn install_never_overwrites_existing_different_content() {
    let root = TempDataRoot::new();
    let existing = png_bytes(b"already-here");
    root.write("writing-crops/keep.png", &existing);
    let incoming = png_bytes(b"incoming");
    let entry = AttachmentFileV1 {
        sha256: hash_of(&incoming),
        rel_path: "writing-crops/keep.png".to_string(),
        size: incoming.len() as u64,
        media_type: "image/png".to_string(),
    };

    let error = install_verified_attachment(&root.path, &entry, &mut incoming.as_slice())
        .expect_err("collision");
    assert_eq!(error.code, ATTACHMENT_EXISTS_DIFFERENT);
    assert_eq!(
        fs::read(root.path.join("writing-crops/keep.png")).expect("original"),
        existing,
        "the existing file must be untouched"
    );
}

#[test]
fn install_size_and_media_failures_leave_targets_absent_and_unrelated_temp_untouched() {
    let root = TempDataRoot::new();
    let bytes = png_bytes(b"checks");
    let unrelated = b"another transfer";
    root.write("writing-crops/unrelated.part", unrelated);

    let wrong_size = AttachmentFileV1 {
        sha256: hash_of(&bytes),
        rel_path: "writing-crops/size.png".to_string(),
        size: bytes.len() as u64 + 1,
        media_type: "image/png".to_string(),
    };
    let error = install_verified_attachment(&root.path, &wrong_size, &mut bytes.as_slice())
        .expect_err("wrong size");
    assert_eq!(error.code, ATTACHMENT_HASH_MISMATCH);

    let wrong_media = AttachmentFileV1 {
        sha256: hash_of(&bytes),
        media_type: "image/jpeg".to_string(),
        rel_path: "writing-crops/media.jpg".to_string(),
        size: bytes.len() as u64,
    };
    let error = install_verified_attachment(&root.path, &wrong_media, &mut bytes.as_slice())
        .expect_err("media mismatch");
    assert_eq!(error.code, ATTACHMENT_INVALID_ENTRY);

    assert_eq!(
        fs::read(root.path.join("writing-crops/unrelated.part")).expect("unrelated temp"),
        unrelated
    );
    assert_eq!(
        file_names(&root.path.join("writing-crops")),
        BTreeSet::from(["unrelated.part".to_string()]),
        "failed installs may clean only their own unique temp"
    );
}

#[test]
fn install_rejects_unsafe_paths_before_creating_attachment_directories() {
    let root = TempDataRoot::new();
    let bytes = png_bytes(b"unsafe");
    let entry = AttachmentFileV1 {
        sha256: hash_of(&bytes),
        rel_path: "../outside.png".to_string(),
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    };

    let error = install_verified_attachment(&root.path, &entry, &mut bytes.as_slice())
        .expect_err("unsafe path");

    assert_eq!(error.code, ATTACHMENT_REF_UNSAFE);
    assert!(!root.path.join("writing-crops").exists());
    assert!(!root.path.join("writing-images").exists());
}

#[test]
fn install_rejects_excess_stream_after_only_one_probe_byte() {
    let root = TempDataRoot::new();
    let mut bytes = png_bytes(&vec![b'x'; 256 * 1024]);
    let declared_size = 8usize;
    let entry = AttachmentFileV1 {
        sha256: hash_of(&bytes[..declared_size]),
        rel_path: "writing-crops/bounded.png".to_string(),
        size: declared_size as u64,
        media_type: "image/png".to_string(),
    };
    let mut stream = Cursor::new(std::mem::take(&mut bytes));

    let error =
        install_verified_attachment(&root.path, &entry, &mut stream).expect_err("oversized stream");

    assert_eq!(error.code, ATTACHMENT_HASH_MISMATCH);
    assert_eq!(
        stream.position(),
        entry.size + 1,
        "the installer must stop after the first excess byte"
    );
    assert!(file_names(&root.path.join("writing-crops")).is_empty());
}

#[cfg(any(unix, windows))]
#[test]
fn install_rejects_a_link_or_reparse_attachment_directory_without_writing_outside() {
    let outside = TempDataRoot::new();
    let root = TempDataRoot::new();
    create_directory_alias(&outside.path, &root.path.join("writing-crops"));
    let bytes = png_bytes(b"must-stay-inside");
    let entry = AttachmentFileV1 {
        sha256: hash_of(&bytes),
        rel_path: "writing-crops/escaped.png".to_string(),
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    };

    let error = install_verified_attachment(&root.path, &entry, &mut bytes.as_slice())
        .expect_err("aliased parent directory");

    assert_eq!(error.code, ATTACHMENT_REF_UNSAFE);
    assert!(!outside.path.join("escaped.png").exists());
}

#[test]
fn scan_then_install_roundtrip_produces_a_receipt_ready_manifest() {
    let root = TempDataRoot::new();
    let bytes = png_bytes(b"roundtrip");
    let rel_path = format!("writing-images/{}.png", hash_of(&bytes));
    let entry = AttachmentFileV1 {
        sha256: hash_of(&bytes),
        rel_path: rel_path.clone(),
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    };

    // Sender side: file exists and the scan proves it.
    root.write(&rel_path, &bytes);
    let sender =
        scan_document_attachments(&document_with(vec![image_node(&rel_path)]), &[], &root.path);
    assert!(sender.is_transfer_ready());
    let scanned = files_of(&sender);
    assert_eq!(scanned.len(), 1);
    assert_eq!(scanned[0].0, rel_path);

    // Receiver side: same entry installed from a stream into a fresh root.
    let receiver_root = TempDataRoot::new();
    install_verified_attachment(&receiver_root.path, &entry, &mut bytes.as_slice())
        .expect("install");
    let receiver = scan_document_attachments(
        &document_with(vec![image_node(&rel_path)]),
        &[],
        &receiver_root.path,
    );
    assert_eq!(
        sender.manifest, receiver.manifest,
        "both sides must agree on the manifest"
    );
}
