# EntropIA Code Signing Policy

**Español:** [CODE_SIGNING.md](./CODE_SIGNING.md)

Installer signing is not integrated into the repository. Installers and packages produced today may be unsigned; the Ed25519 signature on Pro's remote bootstrap protects a different trust boundary and is not Authenticode signing of the installer.

## Current status

| Area | Pro | Lite | Status |
| ---- | --- | ---- | ------ |
| Windows NSIS/MSI | Yes | Yes | No signing provider or Authenticode step is integrated; they may be unsigned. |
| Microsoft Store MSIX | No | Yes | CI deliberately produces it unsigned; Microsoft Store is expected to sign it after submission. |
| Linux DEB | Yes | No | `Release` can build it; no package signing is integrated. |
| macOS arm64 | Yes, through a manual run with `release_platform=all` | No | No code signing or notarization is integrated. |
| Remote runtime bootstrap | Yes, for lean installers | No | Ed25519-signed manifest/catalog and SHA-256-validated archive. |
| GitHub Release | `Release` creates a draft with the applicable Pro assets. | NSIS/MSI are attached on tags; the MSIX remains an Actions artifact. | Draft publication is manual and outside the workflow. |
| Supply-chain controls | Both variants | Both variants | No provenance attestation, generated SBOM, published installer checksums, or enforced manual signing approval. |

## Trust boundaries

### 1. Installer and package signing

There is no integration for Authenticode, Linux package signing, or Apple code signing/notarization. No signing provider has been formally selected either. Until those controls are integrated and approved, no asset should be presented as an installer signed by the project.

### 2. Pro remote bootstrap signing and integrity

`Publish Runtime Bootstrap` publishes an Ed25519-signed manifest/catalog and the runtime archive's SHA-256 digest. The application verifies the signature with the baked public key and requires an entry whose `app_version` and platform match exactly before downloading and validating the archive.

These controls protect the remote runtime, not the installer. The conditions `payload_profile=release`, `release_injection_required=false`, and empty `external_artifacts_required` describe only a self-contained release payload; they are not general signing conditions for current lean installers or Lite.

### 3. GitHub Release draft and manual publication

On a tag, `Release` creates the Pro release as a draft and attaches the applicable assets. Lite NSIS/MSI installers are also attached on tags; the MSIX remains a separate artifact for Microsoft Store. Publishing the draft is a manual action outside the workflow and is not an enforced CI signing approval.

## Current release path

This is the operational order for a lean Pro release; it is not an automatic dependency among all workflows:

1. Run `Build Runtime Pack` for the intended application version. It currently builds the `windows-x86_64` pack, produces the `runtime-archive` and `runtime-payloads` artifacts, and runs the runtime-pack smoke test.
2. Run `Publish Runtime Bootstrap` with that run's `runtime_pack_run_id`. It consumes `runtime-archive`, publishes the archive or its parts, and updates the signed manifest/catalog.
3. Mandatorily verify that a compatible live signed catalog entry exists for every lean Pro artifact, with an exact `app_version` and platform match.
4. Trigger `Release` by tag or manually. Its only manual inputs are `release_platform` and `lite_only`; it does not accept `runtime_payload_artifact` or `runtime_payload_run_id`.
5. Review the draft and publish it manually when the applicable artifacts and controls are correct.

`Release` intentionally includes fixture runtime packs in lean Pro installers and compiles in the bootstrap manifest URL and public key. Installer builds are not gated on the smoke workflow result, so verifying the live catalog is a mandatory pre-release control.

## Pre-publication checklist

- [ ] Record the exact tag and commit and the `Release` run.
- [ ] For lean Pro, record the `Build Runtime Pack` and `Publish Runtime Bootstrap` runs, the compatible signed manifest/catalog entry, and the runtime archive's SHA-256 digest.
- [ ] Confirm the exact `app_version` and platform match against the live signed catalog without relying on ephemeral contents documented here.
- [ ] Review the draft assets and confirm which formats and platforms are unsigned.
- [ ] Complete the applicable license, third-party notice, and redistributable-component review; the current notices/SBOM/license gate remains incomplete.
- [ ] Before any future signing, generate and review the exact installer digest. No explicit installer checksum or equivalent provenance is currently published in release assets or notes.

## Requirements before integrating installer signing

- Formally select a provider and define identities, key custody, permissions, and approval policy.
- Sign only CI artifacts traceable to the reviewed tag and commit; do not sign local ad-hoc builds.
- Keep certificates, keys, and tokens outside the repository and apply rotation and least privilege.
- Add post-signing verification and explicit checksum and provenance publication.
- Complete the license, third-party notice, and SBOM gate for the components actually distributed.
- Define platform-specific controls before claiming support: Authenticode for Windows, package signing for Linux, and signing/notarization for macOS.

## Required provenance

A future installer signature must be traceable to:

- the exact tag and commit;
- the `Release` run that produced the asset;
- for lean Pro, the `Build Runtime Pack` and `Publish Runtime Bootstrap` runs;
- the signed manifest/catalog, its exact `app_version` and platform entry, and the runtime archive digest;
- the installer digest generated and reviewed before signing.

The repository does not yet generate or publish the final digest as an asset or release-note checksum; digests exposed by the GitHub API do not replace that explicit publication or a provenance attestation.

## Incident response

If a future signed installer or a signed manifest/runtime is suspected to be compromised:

1. Stop publication or withdraw the affected assets and add a warning to the GitHub Release.
2. Identify the affected tag, commit, runs, catalog entries, and digests.
3. Publish the affected versions, platforms, and digests with mitigation instructions.
4. Revoke or rotate the relevant credentials; for the runtime key, coordinate the new manifest with the public key baked into compatible applications.
5. Rebuild from clean runs, repeat verification, and publish corrected artifacts.
