<#
.SYNOPSIS
  Build the unsigned Microsoft Store MSIX for EntropIA Pro from the Pro MSI.

.DESCRIPTION
  Pro has no captured base MSIX (Lite's repack-store-msix.ps1 path). This script
  extracts the Pro MSI with an administrative install (`msiexec /a`: files only,
  no custom actions), then packs that installed layout with a generated
  AppxManifest, the visual assets rendered from icons/icon.png, and the
  `entropia-pro-agent-connector.exe` execution alias. See store-msix-pro.ps1.

  The MSI must come from a Pro release build that bundled the assistant
  connector and baked the runtime-bootstrap source:
    pnpm exec tauri build --features local-ml --bundles msi `
      --config src-tauri/tauri.connector.pro.conf.json
  with the three ENTROPIA_RUNTIME_BOOTSTRAP_* variables set as in release.yml.

  The package ships UNSIGNED; Microsoft Store signs it after submission.

.PARAMETER MsiPath
  The Pro MSI (target/release/bundle/msi/EntropIA Pro_<version>_x64_en-US.msi).

.PARAMETER IdentityName
  Package/Identity/Name from Partner Center > Product management > Product identity.

.PARAMETER IdentityPublisher
  Package/Identity/Publisher (CN=...) from the same page.

.PARAMETER PublisherDisplayName
  Package/Properties/PublisherDisplayName from the same page.

.PARAMETER OutDir
  Directory for the extracted layout, the work dir and the MSIX.

.PARAMETER StoreVersion
  4-segment Store version (Major.Minor.Build.Revision). Revision MUST be 0
  (Partner Center rejects nonzero). Defaults to 1.8.1.0.
#>
param(
  [Parameter(Mandatory = $true)]
  [string]$MsiPath,

  [Parameter(Mandatory = $true)]
  [string]$IdentityName,

  [Parameter(Mandatory = $true)]
  [string]$IdentityPublisher,

  [Parameter(Mandatory = $true)]
  [string]$PublisherDisplayName,

  [Parameter(Mandatory = $true)]
  [string]$OutDir,

  [string]$StoreVersion = "1.8.1.0"
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "store-msix-pro.ps1")

# Fail on a bad identity or version before the slow MSI extraction.
New-ProStoreMsixManifest -IdentityName $IdentityName -IdentityPublisher $IdentityPublisher -PublisherDisplayName $PublisherDisplayName -StoreVersion $StoreVersion | Out-Null

$makeappx = Find-StoreMsixMakeAppx
if (-not $makeappx) {
  throw "makeappx.exe not found on host. Install the Windows SDK or the App Certification Kit."
}

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path

$layout = Expand-ProStoreMsixMsiLayout -MsiPath $MsiPath -Destination (Join-Path $OutDir "msi-admin")

New-ProStoreMsixPackage `
  -PayloadDirectory $layout `
  -OutDir $OutDir `
  -IdentityName $IdentityName `
  -IdentityPublisher $IdentityPublisher `
  -PublisherDisplayName $PublisherDisplayName `
  -StoreVersion $StoreVersion `
  -MakeAppx $makeappx `
  -SourceIcon (Join-Path $PSScriptRoot "../icons/icon.png")
