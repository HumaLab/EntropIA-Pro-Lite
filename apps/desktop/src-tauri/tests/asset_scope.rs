//! The asset protocol scope, exercised as an integration test.
//!
//! These tests drive `tauri::test::mock_app()`, which pulls the Tauri
//! runtime's Common Controls v6-only import (`TaskDialogIndirect`) into the
//! test binary. build.rs embeds the v6 manifest only for integration test
//! targets (`cargo:rustc-link-arg-tests`), never the lib unit-test harness,
//! so kept under `src/` they died at startup with 0xc0000139
//! STATUS_ENTRYPOINT_NOT_FOUND. Here they link with the manifest, like
//! tests/app_acl.rs does.

use entropia_desktop_lib::asset_scope::grant_asset_protocol_scope;
use tauri::Manager;

/// Mirrors the private `SQLITE_BASENAME` in `src/lib.rs`: the database files
/// the grant forbids one by one. Repeated here because the const is not
/// exported; keep both in step.
const SQLITE_BASENAME: &str = "entropia.sqlite";

/// A mock app with the grant setup applies, over temporary directories.
fn granted_scope() -> (
    tauri::App<tauri::test::MockRuntime>,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let app = tauri::test::mock_app();
    let app_dir = tempfile::tempdir().expect("archive dir");
    let cache_dir = tempfile::tempdir().expect("cache dir");
    grant_asset_protocol_scope(app.handle(), app_dir.path(), cache_dir.path())
        .expect("grant the asset protocol scope");
    (app, app_dir, cache_dir)
}

#[test]
fn serves_the_archive_subdirectories_the_webview_loads() {
    let (app, app_dir, cache_dir) = granted_scope();
    let scope = app.asset_protocol_scope();
    for path in [
        app_dir.path().join("assets/collection/item/scan.jpg"),
        app_dir.path().join("writing-images/hash.png"),
        app_dir.path().join("writing-crops/crop.webp"),
        app_dir.path().join("web-captures/source/capture.pdf"),
        cache_dir.path().join("thumbnails/asset.png"),
    ] {
        assert!(scope.is_allowed(&path), "{} must be served", path.display());
    }
}

#[test]
fn refuses_the_database_and_everything_outside_the_served_subdirectories() {
    let (app, app_dir, cache_dir) = granted_scope();
    let scope = app.asset_protocol_scope();
    for path in [
        app_dir.path().join(SQLITE_BASENAME),
        app_dir.path().join(format!("{SQLITE_BASENAME}-wal")),
        app_dir.path().join(format!("{SQLITE_BASENAME}-shm")),
        app_dir.path().join(format!("{SQLITE_BASENAME}-journal")),
        app_dir.path().join("research/report.md"),
        app_dir.path().join("sample-staging/document.pdf"),
        app_dir.path().join("temp/dictation/take.webm"),
        cache_dir.path().join("audio-previews/preview.wav"),
        cache_dir.path().join("navegador/downloads/part.pdf"),
    ] {
        assert!(
            !scope.is_allowed(&path),
            "{} must not be served",
            path.display()
        );
    }
}
