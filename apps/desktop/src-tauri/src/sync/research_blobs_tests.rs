//! Tests for the inactive research report-blob helpers: manifest/path
//! validation before any I/O, bounded local proof for upload, content-addressed
//! `blob_head`/`blob_put` upload, bounded `blob_get` download with exact size
//! and SHA-256 verification, and the atomic no-clobber install with its
//! already-installed outcome and retry behavior. Mocks follow the existing
//! `writing_blobs`/`http` test style.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::http::HealthLimits;
use super::research_blobs::{
    ensure_research_report_installed, ensure_research_report_uploaded,
    install_verified_report_with_before_publish, report_upload_plan, ResearchBlobPendingKind,
    ResearchReportInstallOutcome, ResearchReportUploadPlan,
};
use super::research_envelope::{
    verify_report_file, ResearchFileManifestV1, MAX_REPORT_FILE_BYTES, REPORT_FILE_REL_PATH,
};
use super::test_support::{MockBlobEvent, MockSyncApi};

fn limits() -> HealthLimits {
    HealthLimits {
        max_push_bytes: 8 * 1024 * 1024,
        max_blob_mb: 1,
    }
}

fn hash_of(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn report_bytes() -> Vec<u8> {
    b"# Informe de investigacion\n\nResumen verificable.\n".to_vec()
}

fn manifest_for(bytes: &[u8]) -> ResearchFileManifestV1 {
    ResearchFileManifestV1 {
        rel_path: REPORT_FILE_REL_PATH.to_string(),
        sha256: hash_of(bytes),
        size: bytes.len() as u64,
    }
}

fn write_report(root: &Path, job_id: &str, bytes: &[u8]) {
    let job_dir = root.join(job_id);
    fs::create_dir_all(&job_dir).expect("job dir");
    fs::write(job_dir.join(REPORT_FILE_REL_PATH), bytes).expect("report fixture");
}

fn report_path(root: &Path, job_id: &str) -> PathBuf {
    root.join(job_id).join(REPORT_FILE_REL_PATH)
}

fn events(api: &MockSyncApi) -> Vec<MockBlobEvent> {
    api.blob_events.lock().expect("blob events").clone()
}

fn temp_files(job_dir: &Path) -> Vec<String> {
    fs::read_dir(job_dir)
        .expect("job dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".research-report-"))
        .collect()
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

#[tokio::test]
async fn report_blob_round_trips_upload_then_verified_install() {
    let source = TempDir::new().expect("source root");
    let bytes = report_bytes();
    let manifest = manifest_for(&bytes);
    write_report(source.path(), "job-1", &bytes);

    let plan = report_upload_plan(source.path(), "job-1", &manifest).expect("upload plan");
    assert_eq!(plan.job_id, "job-1");
    assert_eq!(plan.manifest, manifest);

    let api = MockSyncApi::default();
    let ready = ensure_research_report_uploaded(&api, "token", source.path(), &plan, &limits())
        .await
        .expect("upload readiness");
    assert_eq!(ready.job_id, "job-1");
    assert!(ready.uploaded);
    assert_eq!(
        events(&api),
        vec![
            MockBlobEvent::Head(manifest.sha256.clone()),
            MockBlobEvent::Put(manifest.sha256.clone(), bytes.len()),
        ]
    );

    // The content-addressed blob is already there: HEAD skips the PUT.
    let ready = ensure_research_report_uploaded(&api, "token", source.path(), &plan, &limits())
        .await
        .expect("second upload");
    assert!(!ready.uploaded);

    // Fresh device: bounded download, atomic install, re-verified bytes.
    api.put_blob_bytes(&manifest.sha256, bytes.clone());
    let target = TempDir::new().expect("target root");
    let outcome = ensure_research_report_installed(
        &api,
        "token",
        target.path(),
        "job-1",
        &manifest,
        &limits(),
    )
    .await
    .expect("install");
    assert_eq!(outcome, ResearchReportInstallOutcome::Installed);
    assert_eq!(
        fs::read(report_path(target.path(), "job-1")).expect("installed file"),
        bytes
    );
    verify_report_file(target.path(), "job-1", &manifest).expect("final verifier proof");

    // An exact match is an already-installed no-op and makes no blob calls.
    let calls = events(&api).len();
    let outcome = ensure_research_report_installed(
        &api,
        "token",
        target.path(),
        "job-1",
        &manifest,
        &limits(),
    )
    .await
    .expect("reinstall");
    assert_eq!(outcome, ResearchReportInstallOutcome::AlreadyInstalled);
    assert_eq!(events(&api).len(), calls);
}

#[test]
fn upload_proof_rejects_missing_and_changed_local_files() {
    let root = TempDir::new().expect("root");
    let bytes = report_bytes();
    let manifest = manifest_for(&bytes);

    // Missing file: no plan is ever minted.
    let error = report_upload_plan(root.path(), "job-1", &manifest)
        .expect_err("missing file must not plan");
    assert_eq!(error.kind, ResearchBlobPendingKind::LocalProofFailed);

    // Changed file: the bytes no longer match the captured manifest hash.
    write_report(root.path(), "job-1", b"# otro informe");
    let error = report_upload_plan(root.path(), "job-1", &manifest)
        .expect_err("changed file must not plan");
    assert_eq!(error.kind, ResearchBlobPendingKind::LocalContentChanged);

    // Size mismatch between the manifest and the real bytes.
    let mut larger = manifest_for(&bytes);
    larger.size += 1;
    write_report(root.path(), "job-2", &bytes);
    let error =
        report_upload_plan(root.path(), "job-2", &larger).expect_err("size mismatch must not plan");
    assert_eq!(error.kind, ResearchBlobPendingKind::LocalContentChanged);

    // Hash-only mismatch with the exact size.
    let mut wrong_hash = manifest_for(&bytes);
    wrong_hash.sha256 = "a".repeat(64);
    let error = report_upload_plan(root.path(), "job-2", &wrong_hash)
        .expect_err("hash mismatch must not plan");
    assert_eq!(error.kind, ResearchBlobPendingKind::LocalContentChanged);
}

#[test]
fn manifest_paths_job_ids_and_hashes_are_validated_before_any_io() {
    let root = TempDir::new().expect("root");
    let bytes = report_bytes();
    let valid = manifest_for(&bytes);

    for rel_path in [
        "sub/report.md",
        "./report.md",
        "../report.md",
        "/report.md",
        "report.md/",
        "report.md\\x",
        "C:/report.md",
        "",
        "report.mdx",
    ] {
        let manifest = ResearchFileManifestV1 {
            rel_path: rel_path.to_string(),
            ..valid.clone()
        };
        let error = report_upload_plan(root.path(), "job-1", &manifest)
            .expect_err("unsafe rel_path must be rejected");
        assert_eq!(
            error.kind,
            ResearchBlobPendingKind::InvalidManifest,
            "{rel_path:?}"
        );
    }

    for job_id in ["", "a/b", "..", ".", "a\\b", "C:x"] {
        let error = report_upload_plan(root.path(), job_id, &valid)
            .expect_err("unsafe job id must be rejected");
        assert_eq!(
            error.kind,
            ResearchBlobPendingKind::InvalidManifest,
            "{job_id:?}"
        );
    }

    for sha256 in [
        hash_of(&bytes).to_uppercase(),
        "a".repeat(63),
        "a".repeat(65),
        "z".repeat(64),
    ] {
        let manifest = ResearchFileManifestV1 {
            sha256,
            ..valid.clone()
        };
        let error = report_upload_plan(root.path(), "job-1", &manifest)
            .expect_err("non-canonical hash must be rejected");
        assert_eq!(error.kind, ResearchBlobPendingKind::InvalidManifest);
    }

    // Nothing under the empty root was created and no manifest was minted.
    assert_eq!(
        fs::read_dir(root.path()).expect("root").count(),
        0,
        "validation must not touch the filesystem"
    );
}

#[tokio::test]
async fn size_and_limit_violations_are_rejected_before_the_network() {
    let root = TempDir::new().expect("root");
    let bytes = report_bytes();
    write_report(root.path(), "job-1", &bytes);
    let api = MockSyncApi::default();

    // Above the absolute local ceiling.
    let oversize = ResearchFileManifestV1 {
        rel_path: REPORT_FILE_REL_PATH.to_string(),
        sha256: hash_of(&bytes),
        size: MAX_REPORT_FILE_BYTES + 1,
    };
    let error = report_upload_plan(root.path(), "job-1", &oversize)
        .expect_err("local size bound must reject");
    assert_eq!(error.kind, ResearchBlobPendingKind::BlobTooLarge);

    // Above the advertised server blob limit (1 MiB in the fixture limits).
    let plan = ResearchReportUploadPlan {
        job_id: "job-1".to_string(),
        manifest: ResearchFileManifestV1 {
            rel_path: REPORT_FILE_REL_PATH.to_string(),
            sha256: "a".repeat(64),
            size: 2 * 1024 * 1024,
        },
    };
    let error = ensure_research_report_uploaded(&api, "token", root.path(), &plan, &limits())
        .await
        .expect_err("advertised blob limit must reject");
    assert_eq!(error.kind, ResearchBlobPendingKind::BlobTooLarge);

    // The download side rejects the same declared size before any request.
    let error = ensure_research_report_installed(
        &api,
        "token",
        root.path(),
        "job-1",
        &plan.manifest,
        &limits(),
    )
    .await
    .expect_err("advertised blob limit must reject downloads too");
    assert_eq!(error.kind, ResearchBlobPendingKind::BlobTooLarge);

    // Unusable advertised limits fail closed.
    let zero = HealthLimits {
        max_push_bytes: 8 * 1024 * 1024,
        max_blob_mb: 0,
    };
    let error = ensure_research_report_uploaded(&api, "token", root.path(), &plan, &zero)
        .await
        .expect_err("zero blob limit must fail closed");
    assert_eq!(error.kind, ResearchBlobPendingKind::InvalidLimits);
    assert!(
        events(&api).is_empty(),
        "every limit rejection happens before the network"
    );
}

#[tokio::test]
async fn download_rejects_hash_size_mismatches_and_oversize_responses() {
    let bytes = report_bytes();

    // Same length, different content: the hash check rejects the bytes.
    let manifest = manifest_for(&bytes);
    let mut impostor = bytes.clone();
    impostor[0] = b'X';
    let api = MockSyncApi::default();
    api.put_blob_bytes(&manifest.sha256, impostor);
    let target = TempDir::new().expect("target root");
    let error = ensure_research_report_installed(
        &api,
        "token",
        target.path(),
        "job-1",
        &manifest,
        &limits(),
    )
    .await
    .expect_err("wrong hash must stay pending");
    assert_eq!(error.kind, ResearchBlobPendingKind::DownloadInvalid);
    assert!(!report_path(target.path(), "job-1").exists());

    // Short body: the exact size check rejects it.
    let mut larger = manifest_for(&bytes);
    larger.size += 1;
    let api = MockSyncApi::default();
    api.put_blob_bytes(&larger.sha256, bytes.clone());
    let target = TempDir::new().expect("target root");
    let error =
        ensure_research_report_installed(&api, "token", target.path(), "job-1", &larger, &limits())
            .await
            .expect_err("short body must stay pending");
    assert_eq!(error.kind, ResearchBlobPendingKind::DownloadSizeMismatch);
    assert!(!report_path(target.path(), "job-1").exists());

    // Oversized body: the bounded reader never buffers past the manifest size.
    let mut smaller = manifest_for(&bytes);
    smaller.size -= 1;
    let api = MockSyncApi::default();
    api.put_blob_bytes(&smaller.sha256, bytes.clone());
    let target = TempDir::new().expect("target root");
    let error = ensure_research_report_installed(
        &api,
        "token",
        target.path(),
        "job-1",
        &smaller,
        &limits(),
    )
    .await
    .expect_err("oversized body must stay pending");
    assert_eq!(error.kind, ResearchBlobPendingKind::DownloadTooLarge);
    assert!(!report_path(target.path(), "job-1").exists());
}

#[tokio::test]
async fn existing_different_target_is_never_clobbered() {
    let target = TempDir::new().expect("target root");
    let bytes = report_bytes();
    let manifest = manifest_for(&bytes);
    let owned = b"# contenido local propio";
    write_report(target.path(), "job-1", owned);
    let api = MockSyncApi::default();
    api.put_blob_bytes(&manifest.sha256, bytes.clone());

    let error = ensure_research_report_installed(
        &api,
        "token",
        target.path(),
        "job-1",
        &manifest,
        &limits(),
    )
    .await
    .expect_err("collision must stay pending");
    assert_eq!(error.kind, ResearchBlobPendingKind::ExistingTargetDifferent);
    assert_eq!(
        fs::read(report_path(target.path(), "job-1")).expect("preserved target"),
        owned
    );
    assert!(
        events(&api).is_empty(),
        "the refusal happens before any download"
    );
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn link_and_directory_paths_are_rejected_on_both_directions() {
    let outside = TempDir::new().expect("outside root");
    let bytes = report_bytes();
    let manifest = manifest_for(&bytes);
    write_report(outside.path(), "job-1", &bytes);
    let api = MockSyncApi::default();

    // A linked job directory resolves outside the artifacts root.
    let root = TempDir::new().expect("root");
    create_directory_alias(&outside.path().join("job-1"), &root.path().join("job-1"));
    let error =
        report_upload_plan(root.path(), "job-1", &manifest).expect_err("linked job dir rejected");
    assert_eq!(error.kind, ResearchBlobPendingKind::UnsafeLocalPath);
    let error =
        ensure_research_report_installed(&api, "token", root.path(), "job-1", &manifest, &limits())
            .await
            .expect_err("linked job dir rejected on install");
    assert_eq!(error.kind, ResearchBlobPendingKind::UnsafeLocalPath);

    // A directory where the managed `report.md` must be.
    let root = TempDir::new().expect("root");
    fs::create_dir_all(report_path(root.path(), "job-1")).expect("report directory");
    let error =
        report_upload_plan(root.path(), "job-1", &manifest).expect_err("report directory rejected");
    assert_eq!(error.kind, ResearchBlobPendingKind::UnsafeLocalPath);
    let error =
        ensure_research_report_installed(&api, "token", root.path(), "job-1", &manifest, &limits())
            .await
            .expect_err("report directory rejected on install");
    assert_eq!(error.kind, ResearchBlobPendingKind::UnsafeLocalPath);

    // A link where the managed file must be.
    let root = TempDir::new().expect("root");
    fs::create_dir_all(root.path().join("job-1")).expect("job dir");
    create_directory_alias(outside.path(), &report_path(root.path(), "job-1"));
    let error =
        report_upload_plan(root.path(), "job-1", &manifest).expect_err("linked report rejected");
    assert_eq!(error.kind, ResearchBlobPendingKind::UnsafeLocalPath);
    let error =
        ensure_research_report_installed(&api, "token", root.path(), "job-1", &manifest, &limits())
            .await
            .expect_err("linked report rejected on install");
    assert_eq!(error.kind, ResearchBlobPendingKind::UnsafeLocalPath);

    assert!(events(&api).is_empty(), "path rejections need no network");
}

#[test]
fn failed_publish_is_atomic_and_a_retry_converges() {
    let root = TempDir::new().expect("root");
    let bytes = report_bytes();
    let manifest = manifest_for(&bytes);

    // A competing writer creates the target between verification and publish:
    // the no-clobber publish refuses and the raced bytes survive untouched.
    let raced = b"# escritura competidora";
    let error = install_verified_report_with_before_publish(
        root.path(),
        "job-1",
        &manifest,
        &bytes,
        |_, target| {
            fs::write(target, raced).expect("raced target");
        },
    )
    .expect_err("publish collision must refuse");
    assert_eq!(error.kind, ResearchBlobPendingKind::ExistingTargetDifferent);
    assert_eq!(
        fs::read(report_path(root.path(), "job-1")).expect("raced bytes"),
        raced
    );
    assert!(
        temp_files(&root.path().join("job-1")).is_empty(),
        "a refused publish leaves no temporary litter"
    );

    // Once the race clears, a plain retry installs atomically and re-verifies.
    fs::remove_file(report_path(root.path(), "job-1")).expect("clear raced target");
    let outcome = install_verified_report_with_before_publish(
        root.path(),
        "job-1",
        &manifest,
        &bytes,
        |_, _| {},
    )
    .expect("atomic retry");
    assert_eq!(outcome, ResearchReportInstallOutcome::Installed);
    assert_eq!(
        fs::read(report_path(root.path(), "job-1")).expect("installed file"),
        bytes
    );
    verify_report_file(root.path(), "job-1", &manifest).expect("final verifier proof");
    assert!(
        temp_files(&root.path().join("job-1")).is_empty(),
        "an installed publish leaves no temporary litter"
    );
}

#[tokio::test]
async fn failed_download_leaves_no_partial_state_and_a_retry_installs() {
    let target = TempDir::new().expect("target root");
    let bytes = report_bytes();
    let manifest = manifest_for(&bytes);
    let mut impostor = bytes.clone();
    impostor[0] = b'X';
    let api = MockSyncApi::default();
    api.put_blob_bytes(&manifest.sha256, impostor);

    let error = ensure_research_report_installed(
        &api,
        "token",
        target.path(),
        "job-1",
        &manifest,
        &limits(),
    )
    .await
    .expect_err("wrong bytes must stay pending");
    assert_eq!(error.kind, ResearchBlobPendingKind::DownloadInvalid);
    assert!(
        !target.path().join("job-1").exists(),
        "a rejected download never creates the job directory"
    );

    // The retry with correct bytes converges through the same atomic install.
    api.put_blob_bytes(&manifest.sha256, bytes.clone());
    let outcome = ensure_research_report_installed(
        &api,
        "token",
        target.path(),
        "job-1",
        &manifest,
        &limits(),
    )
    .await
    .expect("retry installs");
    assert_eq!(outcome, ResearchReportInstallOutcome::Installed);
    assert_eq!(
        fs::read(report_path(target.path(), "job-1")).expect("installed file"),
        bytes
    );
}
