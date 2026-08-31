<#
.SYNOPSIS
  Repack the captured base MSIX into a Store-ready, identity-correct, UNSIGNED MSIX.

.DESCRIPTION
  CI-portable parameterization of EntropIA-Lite's repack-store-msix-on-host.ps1.
  Unpacks the vendored base MSIX, force-sets the exact Partner Center identity,
  regenerates every packaged visual PNG from the canonical app icon, swaps in
  the freshly built lean exe, strips the signature/blockmap (the Store signs),
  repacks, reports manifest identity, and verifies packaged icon bytes.

  The identity literals (Name / Publisher / PublisherDisplayName) are bound to
  Partner Center and MUST NOT change — a typo is only rejected late, at upload.

.PARAMETER BaseMsix
  Path to the vendored captured base MSIX fixture
  (apps/desktop/src-tauri/msix/EntropIALite-base.msix).

.PARAMETER ExePath
  Path to the freshly built lean entropia-lite-desktop.exe to swap into the payload.

.PARAMETER OutDir
  Directory where the repacked MSIX (and its work dir) are written.

.PARAMETER StoreVersion
  4-segment Store version (Major.Minor.Build.Revision). Revision MUST be 0
  (Partner Center rejects nonzero). Defaults to 1.0.9.0.
#>
param(
  [Parameter(Mandatory = $true)]
  [string]$BaseMsix,

  [Parameter(Mandatory = $true)]
  [string]$ExePath,

  [Parameter(Mandatory = $true)]
  [string]$OutDir,

  [string]$StoreVersion = "1.0.9.0"
)

. (Join-Path $PSScriptRoot "store-msix-assets.ps1")

$ErrorActionPreference = "Stop"

# Store identity (Partner Center binding — DO NOT change these literals).
$IdentityName = "CONICET.EntropIALite"
$IdentityPublisher = "CN=89DF40E5-581A-4120-9A24-F701205485D6"
$PublisherDisplay = "HLab"

$source = (Resolve-Path -LiteralPath $BaseMsix).Path
$latestExe = (Resolve-Path -LiteralPath $ExePath).Path
$canonicalStoreIcon = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "../icons/icon.png")).Path

if (-not (Test-Path -LiteralPath $OutDir)) {
  New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
}
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path

$workDir = Join-Path $OutDir "repack-store-host"
$output = Join-Path $OutDir "EntropIALite-Store-HLab-$StoreVersion.msix"
$outputAlias = Join-Path $OutDir "EntropIALite-Store-HLab.msix"

if (-not (Test-Path -LiteralPath $source)) {
  throw "Source MSIX not found: $source"
}
if (-not (Test-Path -LiteralPath $latestExe)) {
  throw "Lean exe not found: $latestExe"
}

$makeappxCandidates = @(
  Get-ChildItem -LiteralPath "C:\Program Files (x86)\Windows Kits\10\bin" -Filter "makeappx.exe" -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\makeappx\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -ExpandProperty FullName
)

$makeappxCandidates += @(
  "C:\Program Files (x86)\Windows Kits\10\App Certification Kit\makeappx.exe"
)

$makeappx = $makeappxCandidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1

if (-not $makeappx) {
  throw "makeappx.exe not found on host. Install Windows SDK or App Certification Kit."
}

if (Test-Path -LiteralPath $workDir) {
  Remove-Item -LiteralPath $workDir -Recurse -Force
}

foreach ($artifact in @($output, $outputAlias)) {
  if (Test-Path -LiteralPath $artifact) {
    Remove-Item -LiteralPath $artifact -Force
  }
}

New-Item -ItemType Directory -Path $workDir | Out-Null

& $makeappx unpack /p $source /d $workDir /o
if ($LASTEXITCODE -ne 0) {
  throw "makeappx unpack failed with exit code $LASTEXITCODE"
}

$manifestPath = Join-Path $workDir "AppxManifest.xml"
[xml]$manifest = Get-Content -LiteralPath $manifestPath

$identity = $manifest.SelectSingleNode("//*[local-name()='Identity']")
$properties = $manifest.SelectSingleNode("//*[local-name()='Properties']")
$publisherDisplayName = $properties.SelectSingleNode("./*[local-name()='PublisherDisplayName']")

if (-not $identity) { throw "Identity node not found" }
if (-not $publisherDisplayName) { throw "PublisherDisplayName node not found" }

$identity.SetAttribute("Name", $IdentityName)
$identity.SetAttribute("Publisher", $IdentityPublisher)
$identity.SetAttribute("Version", $StoreVersion)
$publisherDisplayName.InnerText = $PublisherDisplay

$settings = New-Object System.Xml.XmlWriterSettings
$settings.Encoding = New-Object System.Text.UTF8Encoding($false)
$settings.Indent = $true
$writer = [System.Xml.XmlWriter]::Create($manifestPath, $settings)
try { $manifest.Save($writer) }
finally { $writer.Dispose() }

# MSIX ships UNSIGNED — the Microsoft Store applies the signature. Strip the
# signature/blockmap/content-types (regenerated on pack).
Remove-Item -LiteralPath (Join-Path $workDir "AppxBlockMap.xml") -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath (Join-Path $workDir "AppxSignature.p7x") -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath (Join-Path $workDir "[Content_Types].xml") -Force -ErrorAction SilentlyContinue

$capturedJunk = @(
  "Uninstall EntropIA Lite.lnk",
  "VFS\Local AppData\Microsoft\TokenBroker",
  "VFS\LocalAppDataLow\Microsoft\CryptnetUrlCache",
  "VFS\SystemX64\config\systemprofile\AppData\Local\Microsoft\InstallService",
  "VFS\Windows\Logs",
  "VFS\Windows\Microsoft.NET\Framework64\v4.0.30319\ngen.log"
)

foreach ($relativePath in $capturedJunk) {
  Remove-Item -LiteralPath (Join-Path $workDir $relativePath) -Recurse -Force -ErrorAction SilentlyContinue
}

$storeAssetsDirectory = Join-Path $workDir "Assets"
Update-StoreMsixIconAssets -SourceIcon $canonicalStoreIcon -AssetsDirectory $storeAssetsDirectory

# Swap in the freshly built lean exe over the one captured in the base payload.
Copy-Item -LiteralPath $latestExe -Destination (Join-Path $workDir "entropia-lite-desktop.exe") -Force

& $makeappx pack /d $workDir /p $output /o
if ($LASTEXITCODE -ne 0) {
  throw "makeappx pack failed with exit code $LASTEXITCODE"
}

Assert-StoreMsixIconAssetsInArchive -ArchivePath $output -AssetsDirectory $storeAssetsDirectory

Copy-Item -LiteralPath $output -Destination $outputAlias -Force

Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::OpenRead($output)
try {
  $exeEntries = @($zip.Entries | Where-Object { $_.FullName -match '(^|/)entropia-lite-desktop\.exe$' })
  if ($exeEntries.Count -eq 0) { throw "entropia-lite-desktop.exe not found in repacked MSIX" }

  $manifestEntry = $zip.GetEntry("AppxManifest.xml")
  if (-not $manifestEntry) { throw "AppxManifest.xml not found in repacked MSIX" }

  $reader = New-Object System.IO.StreamReader($manifestEntry.Open())
  try { $xmlText = $reader.ReadToEnd() }
  finally { $reader.Dispose() }
}
finally {
  $zip.Dispose()
}

[xml]$xml = $xmlText
$id = $xml.SelectSingleNode("//*[local-name()='Identity']")
$props = $xml.SelectSingleNode("//*[local-name()='Properties']")
$apps = @($xml.SelectNodes("//*[local-name()='Applications']/*[local-name()='Application']"))

[pscustomobject]@{
  Output = $output
  OutputAlias = $outputAlias
  MakeAppx = $makeappx
  Name = $id.Name
  Publisher = $id.Publisher
  Version = $id.Version
  PublisherDisplayName = $props.SelectSingleNode("./*[local-name()='PublisherDisplayName']").InnerText
  DisplayName = $props.SelectSingleNode("./*[local-name()='DisplayName']").InnerText
  ApplicationCount = $apps.Count
  ExeCount = $exeEntries.Count
  Size = (Get-Item -LiteralPath $output).Length
}

Get-FileHash -LiteralPath $output -Algorithm SHA256
