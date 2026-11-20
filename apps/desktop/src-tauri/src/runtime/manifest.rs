use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub executable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeManifest {
    pub pack_version: String,
    #[serde(default = "default_app_version")]
    pub app_version: String,
    pub platform: String,
    #[serde(default = "default_payload_profile")]
    pub payload_profile: String,
    #[serde(default)]
    pub release_injection_required: bool,
    #[serde(default)]
    pub external_artifacts_required: Vec<String>,
    pub python_relpath: String,
    pub uv_relpath: String,
    #[serde(default)]
    pub python_files: Vec<ManifestEntry>,
    #[serde(default)]
    pub uv_files: Vec<ManifestEntry>,
    #[serde(default)]
    pub script_files: Vec<ManifestEntry>,
    #[serde(default)]
    pub wheelhouse: Vec<ManifestEntry>,
    #[serde(default)]
    pub caches: Vec<ManifestEntry>,
    #[serde(default)]
    pub native_assets: Vec<ManifestEntry>,
    /// What the app needs from the runtime, apart from the Python scripts
    /// (Python ABI, the wheels and native libraries it loads). A runtime built
    /// for another app version is reused when this and its scripts match the
    /// app's own bundled manifest, so an app release that leaves the runtime
    /// unchanged does not download it again. Bump it in both bundled
    /// manifests whenever the wheelhouse, Python or native libraries change
    /// in a way the app depends on. Absent (runtimes built before it existed):
    /// only the exact app version is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_abi: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapReleaseManifest {
    pub app_version: String,
    pub platform: String,
    pub pack_version: String,
    pub archive_url: String,
    /// Extra archive part URLs for multi-part hosting (e.g. GitHub's 2 GiB asset
    /// limit). `archive_url` is part 1; these are parts 2..N, fetched and
    /// concatenated in order. `archive_sha256`/`archive_size` cover the whole.
    /// Empty (the default) means a single-archive download — schema-compatible
    /// with older manifests.
    #[serde(default)]
    pub additional_part_urls: Vec<String>,
    pub archive_sha256: String,
    pub archive_size: u64,
    pub signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapManifestIndex {
    pub channel: String,
    pub generated_at: String,
    #[serde(default)]
    pub releases: Vec<BootstrapReleaseManifest>,
}

fn default_payload_profile() -> String {
    "release".to_string()
}

fn default_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

impl RuntimeManifest {
    pub fn load_from_path(path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(path).map_err(|error| {
            format!(
                "Failed to read runtime manifest {}: {error}",
                path.display()
            )
        })?;
        serde_json::from_str(&content).map_err(|error| {
            format!(
                "Failed to parse runtime manifest {}: {error}",
                path.display()
            )
        })
    }

    pub fn all_entries(&self) -> Vec<&ManifestEntry> {
        self.python_files
            .iter()
            .chain(self.uv_files.iter())
            .chain(self.script_files.iter())
            .chain(self.wheelhouse.iter())
            .chain(self.caches.iter())
            .chain(self.native_assets.iter())
            .collect()
    }

    /// Entries whose integrity is critical enough to re-hash on every health
    /// inspection: the python and uv launchers plus every native asset (native
    /// model libs / DLLs). The multi-GB wheelhouse and model caches are
    /// deliberately excluded so inspection stays bounded — only these few small
    /// executables/native libs are recomputed.
    pub fn critical_entries(&self) -> Vec<&ManifestEntry> {
        let mut entries: Vec<&ManifestEntry> = Vec::new();
        for entry in self.python_files.iter().chain(self.uv_files.iter()) {
            if entry.path == self.python_relpath || entry.path == self.uv_relpath {
                entries.push(entry);
            }
        }
        entries.extend(self.native_assets.iter());
        entries
    }
}

impl BootstrapManifestIndex {
    pub fn select_release(
        &self,
        app_version: &str,
        platform: &str,
    ) -> Option<&BootstrapReleaseManifest> {
        self.releases
            .iter()
            .find(|release| release.app_version == app_version && release.platform == platform)
    }
}

impl BootstrapReleaseManifest {
    pub fn signature_payload(&self) -> String {
        let mut payload = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            self.app_version,
            self.platform,
            self.pack_version,
            self.archive_url,
            self.archive_sha256,
            self.archive_size
        );
        // Extra parts are appended so they are covered by the signature too.
        // When empty this is byte-identical to the single-archive payload.
        for url in &self.additional_part_urls {
            payload.push('\n');
            payload.push_str(url);
        }
        payload
    }

    pub fn verify_signature(&self, public_key_base64: &str) -> Result<(), String> {
        let public_key_bytes = base64::engine::general_purpose::STANDARD
            .decode(public_key_base64)
            .map_err(|error| format!("Failed to decode bootstrap public key: {error}"))?;
        let public_key = VerifyingKey::from_bytes(
            &public_key_bytes
                .try_into()
                .map_err(|_| "Bootstrap public key must be 32 bytes".to_string())?,
        )
        .map_err(|error| format!("Invalid bootstrap public key: {error}"))?;
        let signature_bytes = base64::engine::general_purpose::STANDARD
            .decode(&self.signature)
            .map_err(|error| format!("Failed to decode bootstrap signature: {error}"))?;
        let signature = Signature::from_slice(&signature_bytes)
            .map_err(|error| format!("Invalid bootstrap signature bytes: {error}"))?;

        public_key
            .verify(self.signature_payload().as_bytes(), &signature)
            .map_err(|error| format!("Bootstrap signature verification failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::tempdir;

    #[test]
    fn loads_manifest_and_flattens_entries() {
        let dir = tempdir().expect("temp dir");
        let manifest_path = dir.path().join("manifest.json");

        fs::write(
            &manifest_path,
            r#"{
  "pack_version": "2026.05.0",
  "app_version": "0.0.10",
  "platform": "linux-x86_64",
  "payload_profile": "fixture",
  "release_injection_required": true,
  "external_artifacts_required": ["relocatable-python", "offline-wheelhouse-core"],
  "python_relpath": "python/bin/python3",
  "uv_relpath": "uv/bin/uv",
  "python_files": [
    {
      "path": "python/bin/python3",
      "sha256": "abc",
      "size": 3,
      "executable": true
    }
  ],
  "uv_files": [
    {
      "path": "uv/bin/uv",
      "sha256": "def",
      "size": 2,
      "executable": true
    }
  ],
  "script_files": [
    {
      "path": "scripts/transcribe.py",
      "sha256": "xyz",
      "size": 5,
      "executable": false
    }
  ],
  "wheelhouse": [],
  "caches": [],
  "native_assets": [
    {
      "path": "resources/lib/libpdfium.so",
      "sha256": "ghi",
      "size": 4,
      "executable": false
    }
  ]
}"#,
        )
        .expect("write manifest");

        let manifest = RuntimeManifest::load_from_path(&manifest_path).expect("manifest loads");

        assert_eq!(manifest.pack_version, "2026.05.0");
        assert_eq!(manifest.app_version, "0.0.10");
        assert_eq!(manifest.platform, "linux-x86_64");
        assert_eq!(manifest.payload_profile, "fixture");
        assert!(manifest.release_injection_required);
        assert_eq!(
            manifest.external_artifacts_required,
            vec!["relocatable-python", "offline-wheelhouse-core"]
        );
        assert_eq!(manifest.python_relpath, "python/bin/python3");
        assert_eq!(manifest.uv_relpath, "uv/bin/uv");
        assert_eq!(manifest.all_entries().len(), 4);
    }

    #[test]
    fn critical_entries_cover_launchers_and_native_assets_only() {
        let dir = tempdir().expect("temp dir");
        let manifest_path = dir.path().join("manifest.json");

        fs::write(
            &manifest_path,
            r#"{
  "pack_version": "2026.05.0",
  "app_version": "0.0.10",
  "platform": "linux-x86_64",
  "payload_profile": "release",
  "python_relpath": "python/bin/python3",
  "uv_relpath": "uv/bin/uv",
  "python_files": [
    { "path": "python/bin/python3", "sha256": "py", "size": 3, "executable": true },
    { "path": "python/lib/extra.so", "sha256": "lib", "size": 9, "executable": false }
  ],
  "uv_files": [
    { "path": "uv/bin/uv", "sha256": "uv", "size": 2, "executable": true }
  ],
  "script_files": [
    { "path": "scripts/transcribe.py", "sha256": "sc", "size": 5, "executable": false }
  ],
  "wheelhouse": [
    { "path": "wheelhouse/big.whl", "sha256": "wh", "size": 999, "executable": false }
  ],
  "caches": [
    { "path": "caches/hf/model.bin", "sha256": "mb", "size": 4096, "executable": false }
  ],
  "native_assets": [
    { "path": "resources/lib/libpdfium.so", "sha256": "nat", "size": 4, "executable": false }
  ]
}"#,
        )
        .expect("write manifest");

        let manifest = RuntimeManifest::load_from_path(&manifest_path).expect("manifest loads");

        let critical: Vec<&str> = manifest
            .critical_entries()
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();

        // Only the declared python/uv launchers and native assets are critical.
        // The bulky wheelhouse + model caches and incidental python libs are NOT
        // re-hashed on every inspection.
        assert_eq!(
            critical,
            vec![
                "python/bin/python3",
                "uv/bin/uv",
                "resources/lib/libpdfium.so"
            ]
        );
    }

    #[test]
    fn selects_bootstrap_release_for_matching_app_version_and_platform() {
        let index = BootstrapManifestIndex {
            channel: "stable".to_string(),
            generated_at: "2026-05-06T00:00:00Z".to_string(),
            releases: vec![
                BootstrapReleaseManifest {
                    app_version: "0.0.10".to_string(),
                    platform: "windows-x86_64".to_string(),
                    pack_version: "2026.05.0".to_string(),
                    archive_url: "https://example.com/windows.zip".to_string(),
                    additional_part_urls: Vec::new(),
                    archive_sha256: "win-sha".to_string(),
                    archive_size: 42,
                    signature: "sig-win".to_string(),
                },
                BootstrapReleaseManifest {
                    app_version: "0.0.10".to_string(),
                    platform: "linux-x86_64".to_string(),
                    pack_version: "2026.05.1".to_string(),
                    archive_url: "https://example.com/linux.zip".to_string(),
                    additional_part_urls: Vec::new(),
                    archive_sha256: "linux-sha".to_string(),
                    archive_size: 84,
                    signature: "sig-linux".to_string(),
                },
            ],
        };

        let selected = index
            .select_release("0.0.10", "linux-x86_64")
            .expect("linux release should be selected");

        assert_eq!(selected.pack_version, "2026.05.1");
        assert_eq!(selected.archive_sha256, "linux-sha");
    }

    #[test]
    fn ignores_bootstrap_releases_for_other_versions_or_platforms() {
        let index = BootstrapManifestIndex {
            channel: "stable".to_string(),
            generated_at: "2026-05-06T00:00:00Z".to_string(),
            releases: vec![BootstrapReleaseManifest {
                app_version: "9.9.9".to_string(),
                platform: "windows-x86_64".to_string(),
                pack_version: "2026.05.9".to_string(),
                archive_url: "https://example.com/windows.zip".to_string(),
                additional_part_urls: Vec::new(),
                archive_sha256: "sha".to_string(),
                archive_size: 42,
                signature: "sig".to_string(),
            }],
        };

        assert_eq!(index.select_release("0.0.10", "linux-x86_64"), None);
    }

    #[test]
    fn bootstrap_release_signature_payload_is_stable() {
        let release = BootstrapReleaseManifest {
            app_version: "0.0.10".to_string(),
            platform: "linux-x86_64".to_string(),
            pack_version: "2026.05.1".to_string(),
            archive_url: "https://example.com/runtime-pack.zip".to_string(),
            additional_part_urls: Vec::new(),
            archive_sha256: "archive-sha".to_string(),
            archive_size: 1024,
            signature: "sig".to_string(),
        };

        assert_eq!(
            release.signature_payload(),
            "0.0.10\nlinux-x86_64\n2026.05.1\nhttps://example.com/runtime-pack.zip\narchive-sha\n1024"
        );
    }

    #[test]
    fn bootstrap_release_signature_payload_appends_additional_parts() {
        let release = BootstrapReleaseManifest {
            app_version: "0.0.10".to_string(),
            platform: "linux-x86_64".to_string(),
            pack_version: "2026.05.1".to_string(),
            archive_url: "https://example.com/part1".to_string(),
            additional_part_urls: vec![
                "https://example.com/part2".to_string(),
                "https://example.com/part3".to_string(),
            ],
            archive_sha256: "archive-sha".to_string(),
            archive_size: 1024,
            signature: "sig".to_string(),
        };

        // Extra parts are appended after archive_size, each on its own line, so the
        // signature covers exactly where every byte of the archive comes from.
        assert_eq!(
            release.signature_payload(),
            "0.0.10\nlinux-x86_64\n2026.05.1\nhttps://example.com/part1\narchive-sha\n1024\nhttps://example.com/part2\nhttps://example.com/part3"
        );
    }

    #[test]
    fn verifies_valid_bootstrap_release_signature() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let release_without_signature = BootstrapReleaseManifest {
            app_version: "0.0.10".to_string(),
            platform: "linux-x86_64".to_string(),
            pack_version: "2026.05.1".to_string(),
            archive_url: "https://example.com/runtime-pack.zip".to_string(),
            additional_part_urls: Vec::new(),
            archive_sha256: "archive-sha".to_string(),
            archive_size: 1024,
            signature: String::new(),
        };
        let signature = signing_key.sign(release_without_signature.signature_payload().as_bytes());
        let release = BootstrapReleaseManifest {
            signature: base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
            ..release_without_signature
        };
        let public_key = base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes());

        assert!(release.verify_signature(&public_key).is_ok());
    }

    #[test]
    fn rejects_invalid_bootstrap_release_signature() {
        let release = BootstrapReleaseManifest {
            app_version: "0.0.10".to_string(),
            platform: "linux-x86_64".to_string(),
            pack_version: "2026.05.1".to_string(),
            archive_url: "https://example.com/runtime-pack.zip".to_string(),
            additional_part_urls: Vec::new(),
            archive_sha256: "archive-sha".to_string(),
            archive_size: 1024,
            signature: base64::engine::general_purpose::STANDARD.encode([9u8; 64]),
        };
        let public_key = base64::engine::general_purpose::STANDARD.encode([3u8; 32]);

        let error = release
            .verify_signature(&public_key)
            .expect_err("invalid signature must fail");

        assert!(error.contains("signature") || error.contains("Signature"));
    }
}
