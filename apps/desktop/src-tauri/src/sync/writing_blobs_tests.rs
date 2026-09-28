use std::fs;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::http::HealthLimits;
use super::test_support::{MockBlobEvent, MockBlobFailure, MockSyncApi};
use super::writing_blobs::{
    ensure_writing_blobs_installed, ensure_writing_blobs_uploaded, WritingBlobPendingKind,
};
use crate::writing::sync_envelope::{
    AttachmentFileV1, AttachmentManifestV1, CitationProjectionsV1, DocumentSettingsV1,
    WritingEnvelopeV1,
};
use crate::writing::sync_files::verify_attachment_manifest;

fn tiny_png() -> Vec<u8> {
    // A complete, valid 1x1 PNG rather than a signature-only media fixture.
    STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
        .expect("valid embedded PNG")
}

fn hash_of(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn image_entry(bytes: &[u8]) -> AttachmentFileV1 {
    let sha256 = hash_of(bytes);
    AttachmentFileV1 {
        rel_path: format!("writing-images/{sha256}.png"),
        sha256,
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    }
}

fn crop_entry(bytes: &[u8], name: &str, media_type: &str) -> AttachmentFileV1 {
    AttachmentFileV1 {
        sha256: hash_of(bytes),
        rel_path: format!("writing-crops/{name}"),
        size: bytes.len() as u64,
        media_type: media_type.to_string(),
    }
}

fn envelope(files: Vec<AttachmentFileV1>) -> WritingEnvelopeV1 {
    let mut content = vec![serde_json::json!({
        "type": "paragraph",
        "content": [{ "type": "text", "text": "Bounded blob fixture" }]
    })];
    content.extend(files.iter().map(|entry| {
        serde_json::json!({
            "type": "writingImage",
            "attrs": { "src": entry.rel_path }
        })
    }));

    WritingEnvelopeV1 {
        id: "document-blob-fixture".to_string(),
        envelope_version: 1,
        content_json: serde_json::json!({
            "schemaVersion": 1,
            "doc": { "type": "doc", "content": content }
        }),
        title: "Blob fixture".to_string(),
        document_type: "article".to_string(),
        status: "active".to_string(),
        schema_version: 1,
        settings: DocumentSettingsV1 {
            citation_style_id: None,
            citation_locale: None,
            bibliography_enabled: true,
        },
        collection_associations: Vec::new(),
        citation_projections: CitationProjectionsV1 {
            corpus: Vec::new(),
            zotero: Vec::new(),
        },
        attachments_manifest: AttachmentManifestV1::Validated { files },
    }
}

fn limits() -> HealthLimits {
    HealthLimits {
        max_push_bytes: 8 * 1024 * 1024,
        max_blob_mb: 1,
    }
}

fn write_attachment(root: &Path, entry: &AttachmentFileV1, bytes: &[u8]) {
    let target = root.join(&entry.rel_path);
    fs::create_dir_all(target.parent().expect("attachment parent"))
        .expect("create attachment parent");
    fs::write(target, bytes).expect("write attachment fixture");
}

fn target_path(root: &Path, entry: &AttachmentFileV1) -> PathBuf {
    root.join(&entry.rel_path)
}

fn events(api: &MockSyncApi) -> Vec<MockBlobEvent> {
    api.blob_events.lock().expect("blob events").clone()
}

fn seed_caller_markers(root: &Path) -> (PathBuf, PathBuf) {
    let manuscript = root.join("caller-manuscript-state");
    let outbox = root.join("caller-outbox-state");
    fs::write(&manuscript, b"manuscript-retained").expect("manuscript marker");
    fs::write(&outbox, b"outbox-retained").expect("outbox marker");
    (manuscript, outbox)
}

fn assert_caller_markers_unchanged(markers: &(PathBuf, PathBuf)) {
    assert_eq!(
        fs::read(&markers.0).expect("manuscript marker"),
        b"manuscript-retained"
    );
    assert_eq!(
        fs::read(&markers.1).expect("outbox marker"),
        b"outbox-retained"
    );
}

#[tokio::test]
async fn missing_upload_runs_head_then_put_before_readiness() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let entry = image_entry(&bytes);
    write_attachment(root.path(), &entry, &bytes);
    let envelope = envelope(vec![entry.clone()]);
    let api = MockSyncApi::default();

    let ready = ensure_writing_blobs_uploaded(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect("upload readiness");

    assert_eq!(ready.unique_blobs, 1);
    assert_eq!(ready.uploaded_blobs, 1);
    assert_eq!(
        events(&api),
        vec![
            MockBlobEvent::Head(entry.sha256.clone()),
            MockBlobEvent::Put(entry.sha256, bytes.len()),
        ]
    );
}

#[tokio::test]
async fn existing_remote_hash_avoids_put() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let entry = image_entry(&bytes);
    write_attachment(root.path(), &entry, &bytes);
    let envelope = envelope(vec![entry.clone()]);
    let api = MockSyncApi::default();
    api.existing_blobs
        .lock()
        .expect("existing blobs")
        .insert(entry.sha256.clone());

    let ready = ensure_writing_blobs_uploaded(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect("upload readiness");

    assert_eq!(ready.uploaded_blobs, 0);
    assert_eq!(
        events(&api),
        vec![MockBlobEvent::Head(entry.sha256.clone())]
    );
}

#[tokio::test]
async fn shared_upload_hash_is_probed_and_put_once() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let image = image_entry(&bytes);
    let crop = crop_entry(&bytes, "shared.png", "image/png");
    write_attachment(root.path(), &image, &bytes);
    write_attachment(root.path(), &crop, &bytes);
    let envelope = envelope(vec![image.clone(), crop]);
    let api = MockSyncApi::default();

    let ready = ensure_writing_blobs_uploaded(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect("deduplicated upload readiness");

    assert_eq!(ready.unique_blobs, 1);
    assert_eq!(ready.uploaded_blobs, 1);
    assert_eq!(
        events(&api),
        vec![
            MockBlobEvent::Head(image.sha256.clone()),
            MockBlobEvent::Put(image.sha256, bytes.len()),
        ]
    );
}

#[tokio::test]
async fn missing_local_upload_file_fails_before_network() {
    let root = TempDir::new().expect("temp data root");
    let entry = image_entry(&tiny_png());
    let envelope = envelope(vec![entry]);
    let api = MockSyncApi::default();

    let pending = ensure_writing_blobs_uploaded(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("missing file must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::LocalProofFailed);
    assert!(events(&api).is_empty());
}

#[tokio::test]
async fn changed_local_upload_file_fails_before_network() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let entry = image_entry(&bytes);
    let envelope = envelope(vec![entry.clone()]);
    let mut changed = bytes;
    changed.push(0);
    write_attachment(root.path(), &entry, &changed);
    let api = MockSyncApi::default();

    let pending = ensure_writing_blobs_uploaded(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("changed file must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::LocalProofFailed);
    assert!(events(&api).is_empty());
}

#[tokio::test]
async fn upload_network_error_is_an_explicit_pending_result() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let entry = image_entry(&bytes);
    write_attachment(root.path(), &entry, &bytes);
    let envelope = envelope(vec![entry.clone()]);
    let api = MockSyncApi::default();
    *api.blob_failure.lock().expect("blob failure") =
        Some(MockBlobFailure::Network("loopback unavailable".to_string()));

    let pending = ensure_writing_blobs_uploaded(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("network error must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::Network);
    assert_eq!(events(&api), vec![MockBlobEvent::Head(entry.sha256)]);
}

#[tokio::test]
async fn valid_download_installs_then_finally_verifies() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let entry = image_entry(&bytes);
    let envelope = envelope(vec![entry.clone()]);
    let api = MockSyncApi::default();
    api.put_blob_bytes(&entry.sha256, bytes.clone());

    let ready = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect("download readiness");

    assert_eq!(ready.downloaded_blobs, 1);
    assert_eq!(ready.installed_files, 1);
    assert_eq!(
        fs::read(target_path(root.path(), &entry)).expect("installed file"),
        bytes
    );
    verify_attachment_manifest(
        &envelope.content_json,
        &[],
        root.path(),
        &envelope.attachments_manifest,
    )
    .expect("final real verifier proof");
}

#[tokio::test]
async fn missing_remote_download_preserves_caller_state_and_publishes_nothing() {
    let root = TempDir::new().expect("temp data root");
    let entry = image_entry(&tiny_png());
    let envelope = envelope(vec![entry.clone()]);
    let markers = seed_caller_markers(root.path());
    let api = MockSyncApi::default();

    let pending = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("404 must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::RemoteMissing);
    assert!(!target_path(root.path(), &entry).exists());
    assert_caller_markers_unchanged(&markers);
}

#[tokio::test]
async fn wrong_download_hash_preserves_caller_state_and_publishes_nothing() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let sha256 = "a".repeat(64);
    let entry = AttachmentFileV1 {
        rel_path: format!("writing-images/{sha256}.png"),
        sha256,
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    };
    let envelope = envelope(vec![entry.clone()]);
    let markers = seed_caller_markers(root.path());
    let api = MockSyncApi::default();
    api.put_blob_bytes(&entry.sha256, bytes);

    let pending = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("wrong hash must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::DownloadInvalid);
    assert!(!target_path(root.path(), &entry).exists());
    assert_caller_markers_unchanged(&markers);
}

#[tokio::test]
async fn short_download_preserves_caller_state_and_publishes_nothing() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let mut entry = image_entry(&bytes);
    entry.size += 1;
    let envelope = envelope(vec![entry.clone()]);
    let markers = seed_caller_markers(root.path());
    let api = MockSyncApi::default();
    api.put_blob_bytes(&entry.sha256, bytes);

    let pending = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("short body must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::DownloadSizeMismatch);
    assert!(!target_path(root.path(), &entry).exists());
    assert_caller_markers_unchanged(&markers);
}

#[tokio::test]
async fn oversized_download_body_is_rejected_before_install() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let mut entry = image_entry(&bytes);
    entry.size -= 1;
    let envelope = envelope(vec![entry.clone()]);
    let api = MockSyncApi::default();
    api.put_blob_bytes(&entry.sha256, bytes);

    let pending = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("oversized body must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::DownloadTooLarge);
    assert!(!target_path(root.path(), &entry).exists());
}

#[tokio::test]
async fn wrong_download_media_preserves_caller_state_and_publishes_nothing() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let entry = crop_entry(&bytes, "remote.jpg", "image/jpeg");
    let envelope = envelope(vec![entry.clone()]);
    let markers = seed_caller_markers(root.path());
    let api = MockSyncApi::default();
    api.put_blob_bytes(&entry.sha256, bytes);

    let pending = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("wrong media must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::DownloadInvalid);
    assert!(!target_path(root.path(), &entry).exists());
    assert_caller_markers_unchanged(&markers);
}

#[tokio::test]
async fn existing_identical_attachment_skips_download() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let entry = image_entry(&bytes);
    write_attachment(root.path(), &entry, &bytes);
    let envelope = envelope(vec![entry.clone()]);
    let api = MockSyncApi::default();

    let ready = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect("existing attachment readiness");

    assert_eq!(ready.downloaded_blobs, 0);
    assert_eq!(ready.already_installed_files, 1);
    assert!(events(&api).is_empty());
}

#[tokio::test]
async fn existing_different_destination_is_not_overwritten() {
    let root = TempDir::new().expect("temp data root");
    let entry = image_entry(&tiny_png());
    let original = b"locally-owned-different-bytes";
    write_attachment(root.path(), &entry, original);
    let envelope = envelope(vec![entry.clone()]);
    let api = MockSyncApi::default();

    let pending = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("collision must stay pending");

    assert_eq!(
        pending.kind,
        WritingBlobPendingKind::ExistingTargetDifferent
    );
    assert_eq!(
        fs::read(target_path(root.path(), &entry)).expect("preserved destination"),
        original
    );
    assert!(events(&api).is_empty());
}

#[tokio::test]
async fn unsafe_manifest_is_rejected_before_network_or_path_escape() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let entry = AttachmentFileV1 {
        sha256: hash_of(&bytes),
        rel_path: "writing-images/../escape.png".to_string(),
        size: bytes.len() as u64,
        media_type: "image/png".to_string(),
    };
    let envelope = envelope(vec![entry]);
    let api = MockSyncApi::default();

    let pending = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("unsafe manifest must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::InvalidEnvelope);
    assert!(!root.path().join("escape.png").exists());
    assert!(events(&api).is_empty());
}

#[tokio::test]
async fn image_reference_cannot_be_hidden_by_an_empty_manifest() {
    let root = TempDir::new().expect("temp data root");
    let entry = image_entry(&tiny_png());
    let mut envelope = envelope(vec![entry]);
    envelope.attachments_manifest = AttachmentManifestV1::Validated { files: Vec::new() };
    let api = MockSyncApi::default();

    let pending = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("incomplete manifest must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::InvalidManifest);
    assert!(events(&api).is_empty());
}

#[tokio::test]
async fn blob_larger_than_advertised_limit_is_rejected_before_network() {
    let root = TempDir::new().expect("temp data root");
    let bytes = tiny_png();
    let mut entry = image_entry(&bytes);
    entry.size = 1024 * 1024 + 1;
    let envelope = envelope(vec![entry]);
    let api = MockSyncApi::default();

    let pending = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("oversized manifest entry must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::BlobTooLarge);
    assert!(events(&api).is_empty());
}

#[tokio::test]
async fn envelope_larger_than_advertised_push_limit_is_rejected_before_network() {
    let root = TempDir::new().expect("temp data root");
    let envelope = envelope(Vec::new());
    let api = MockSyncApi::default();
    let tiny_limit = HealthLimits {
        max_push_bytes: 32,
        max_blob_mb: 1,
    };

    let pending =
        ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &tiny_limit)
            .await
            .expect_err("oversized envelope must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::ManifestTooLarge);
    assert!(events(&api).is_empty());
}

#[tokio::test]
async fn unauthorized_download_is_an_explicit_pending_result() {
    let root = TempDir::new().expect("temp data root");
    let entry = image_entry(&tiny_png());
    let envelope = envelope(vec![entry.clone()]);
    let api = MockSyncApi::default();
    *api.blob_failure.lock().expect("blob failure") = Some(MockBlobFailure::Api {
        status: 401,
        code: "unauthorized".to_string(),
        message: "token expired".to_string(),
    });

    let pending = ensure_writing_blobs_installed(&api, "token", root.path(), &envelope, &limits())
        .await
        .expect_err("unauthorized GET must stay pending");

    assert_eq!(pending.kind, WritingBlobPendingKind::Unauthorized);
    assert_eq!(events(&api), vec![MockBlobEvent::Get(entry.sha256)]);
}

#[tokio::test]
async fn empty_attachment_document_makes_no_blob_calls() {
    let upload_root = TempDir::new().expect("upload data root");
    let download_root = TempDir::new().expect("download data root");
    let envelope = envelope(Vec::new());
    let api = MockSyncApi::default();

    let upload =
        ensure_writing_blobs_uploaded(&api, "token", upload_root.path(), &envelope, &limits())
            .await
            .expect("empty upload readiness");
    let download =
        ensure_writing_blobs_installed(&api, "token", download_root.path(), &envelope, &limits())
            .await
            .expect("empty download readiness");

    assert_eq!(upload.unique_blobs, 0);
    assert_eq!(download.unique_blobs, 0);
    assert!(events(&api).is_empty());
}
