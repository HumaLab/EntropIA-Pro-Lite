//! Keeps the Visual C++ runtime dynamically linked on Windows.
//!
//! tauri-build 2.7 links the VC runtime statically unless told otherwise
//! (`build > windows > staticVCRuntime` defaults to true). The app ships the
//! VC runtime DLLs next to the binary (build.rs stages them) and the native
//! ML libraries are built against the dynamic CRT; llama-cpp-sys-2 even adds
//! the debug CRT (`msvcrtd`) in dev and test builds. A static CRT next to them
//! mixes two heaps: the Pro test executable hit `_CrtIsValidHeapPointer`
//! before running a single test and hung CI on the assertion dialog.
//!
//! Read as text on purpose: the setting lives in the build script and the
//! Tauri configuration, neither of which a test can reach any other way.

use std::path::PathBuf;

fn read(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn the_build_script_opts_out_of_the_static_vc_runtime() {
    let build = read("build.rs");
    assert!(
        build.contains(".static_vc_runtime(false)"),
        "build.rs must pass WindowsAttributes::static_vc_runtime(false) to tauri-build"
    );
}

#[test]
fn no_tauri_configuration_turns_the_static_vc_runtime_back_on() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for entry in std::fs::read_dir(&dir).expect("read src-tauri") {
        let path = entry.expect("dir entry").path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("tauri") && name.ends_with(".json") {
            let text = std::fs::read_to_string(&path).expect("read tauri config");
            assert!(
                !text.contains("staticVCRuntime"),
                "{name} sets staticVCRuntime; keep it out so build.rs decides"
            );
        }
    }
}
