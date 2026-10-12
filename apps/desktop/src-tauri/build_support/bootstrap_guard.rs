//! Shared decision for the release bootstrap guard.
//!
//! Both `build.rs` (the `guard_lean_bootstrap_source` step) and
//! `tests/bootstrap_guard.rs` compile this file through `#[path = ...]`, so the
//! tested function is exactly the one the build script runs. It is std-only and
//! has no crate dependencies: a build script cannot use the crate's
//! dependencies and the test target adds none. Everything is passed in — the
//! function never reads the environment or the filesystem itself.

/// Everything the guard needs, already read from the environment and the
/// bundled runtime-pack manifest.
pub struct BootstrapGuardInput<'a> {
    /// Cargo build profile (`PROFILE`); only `"release"` is guarded.
    pub profile: &'a str,
    /// `CARGO_CFG_TARGET_OS` of the build target.
    pub target_os: &'a str,
    /// Whether the `local-ml` feature is enabled (Pro). Lite compiles the
    /// runtime out, so the guard must not demand a source for it.
    pub local_ml: bool,
    /// Contents of the bundled runtime-pack manifest, or `None` when it cannot
    /// be read (missing platform manifest, missing `CARGO_MANIFEST_DIR`, ...).
    pub manifest: Option<&'a str>,
    /// `ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL` at build time.
    pub manifest_url: &'a str,
    /// `ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID` at build time.
    pub public_key_id: &'a str,
    /// `ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64` at build time.
    pub public_key_base64: &'a str,
}

/// Returns `None` when the build may proceed, or the panic message naming the
/// platform and exactly the bootstrap variables that are not baked in.
///
/// The build is allowed for: non-release profiles; Lite (`local_ml == false`),
/// where every reader of these variables is `#[cfg(feature = "local-ml")]` in
/// src/settings.rs; non-Windows/Linux targets (macOS ships no runtime-pack); an
/// unreadable manifest (historical behavior: no evidence of a fixture pack);
/// and a real release pack, which already ships the runtime inside the installer
/// so no download source is needed. Everything else is a fixture pack on a
/// Windows/Linux Pro release, and the three variables must all be set.
pub fn bootstrap_guard_error(input: &BootstrapGuardInput<'_>) -> Option<String> {
    if input.profile != "release" {
        return None;
    }
    // Lite has no runtime: the feature is compiled out and so is every reader of
    // these variables, so the build must not be forced to bake them.
    if !input.local_ml {
        return None;
    }
    // The runtime-pack is bundled only for Windows and Linux.
    let platform = match input.target_os {
        "windows" => "windows-x86_64",
        "linux" => "linux-x86_64",
        _ => return None,
    };
    // An unreadable manifest keeps today's behavior: allow the build, since
    // there is no evidence that a fixture pack is being bundled.
    let manifest = input.manifest?;
    // Match the same whitespace-insensitive shape as the old inline guard: the
    // committed fixture manifest is pretty-printed, so compare without spaces.
    let compact: String = manifest.chars().filter(|c| !c.is_whitespace()).collect();
    let bundles_real_release = compact.contains("\"payload_profile\":\"release\"")
        && compact.contains("\"release_injection_required\":false");
    if bundles_real_release {
        return None;
    }

    let missing: Vec<&str> = [
        (
            "ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL",
            input.manifest_url,
        ),
        (
            "ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_ID",
            input.public_key_id,
        ),
        (
            "ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64",
            input.public_key_base64,
        ),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.trim().is_empty().then_some(name))
    .collect();
    if missing.is_empty() {
        return None;
    }

    let names = missing.join(" / ");
    let (noun, verb) = if missing.len() == 1 {
        ("variable", "is")
    } else {
        ("variables", "are")
    };
    Some(format!(
        "Release build for {platform} bundles a non-release (fixture) runtime-pack, but the \
         bootstrap {noun} {names} {verb} not baked in. A lean installer with no signed download \
         source dead-ends on a clean machine. Set {names} (see .github/workflows/release.yml) \
         before a release build, or bundle a real release runtime-pack."
    ))
}
