Set-StrictMode -Version Latest

# EntropIA Pro Microsoft Store MSIX, generated from scratch.
#
# Lite repacks a base MSIX captured once in a Hyper-V VM. Pro has no such
# capture: its package is the installed layout of the Pro MSI (extracted with an
# administrative install, which copies files and runs no custom action) plus an
# AppxManifest written here. Nothing in the Pro app writes into its install
# directory (all state lives under %APPDATA% / %LOCALAPPDATA%), so the read-only
# WindowsApps location is safe; the bundled uv probe falls back to app data.
#
# The package ships UNSIGNED: Microsoft Store signs it after submission.

. (Join-Path $PSScriptRoot "store-msix-assets.ps1")
. (Join-Path $PSScriptRoot "store-msix-shortcuts.ps1")
. (Join-Path $PSScriptRoot "store-msix-payload.ps1")

$script:ProStoreMsixExecutable = "entropia-pro-desktop.exe"
$script:ProStoreMsixConnectorFileName = "entropia-pro-agent-connector.exe"
$script:ProStoreMsixApplicationId = "EntropIAPro"
$script:ProStoreMsixDisplayName = "EntropIA Pro"

# Every visual asset the manifest references, with its required pixel size.
$script:ProStoreMsixAssetSizes = [ordered]@{
  "StoreLogo.png"         = @(50, 50)
  "Square44x44Logo.png"   = @(44, 44)
  "Square71x71Logo.png"   = @(71, 71)
  "Square150x150Logo.png" = @(150, 150)
  "Square310x310Logo.png" = @(310, 310)
  "Wide310x150Logo.png"   = @(310, 150)
}

function Find-StoreMsixMakeAppx {
  <#
  .SYNOPSIS
    Locate the newest x64 makeappx.exe from the Windows SDK; $null when absent.
  #>
  [CmdletBinding()]
  param()

  $candidates = @(
    Get-ChildItem -LiteralPath "C:\Program Files (x86)\Windows Kits\10\bin" -Filter "makeappx.exe" -Recurse -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -match '\\x64\\makeappx\.exe$' } |
      Sort-Object FullName -Descending |
      Select-Object -ExpandProperty FullName
  )
  $candidates += @("C:\Program Files (x86)\Windows Kits\10\App Certification Kit\makeappx.exe")

  return $candidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
}

function New-ProStoreMsixManifest {
  <#
  .SYNOPSIS
    Build the Pro AppxManifest from the Partner Center identity.

  .DESCRIPTION
    The identity values are bound to the Partner Center product and have no
    default: a wrong one is only rejected late, at upload. Store versions are
    Major.Minor.Build.0 (Partner Center reserves the revision).
  #>
  param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$IdentityName,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$IdentityPublisher,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$PublisherDisplayName,

    [Parameter(Mandatory = $true)]
    [string]$StoreVersion
  )

  if ($IdentityName -notmatch '^[A-Za-z0-9.\-]{3,50}$') {
    throw "Identity Name '$IdentityName' is not a valid package name (3-50 letters, digits, '.' or '-')."
  }
  if ($IdentityPublisher -notmatch '^CN=') {
    throw "Identity Publisher '$IdentityPublisher' must be the Partner Center distinguished name (CN=...)."
  }
  if ($StoreVersion -notmatch '^(\d+)\.(\d+)\.(\d+)\.(\d+)$') {
    throw "Store version '$StoreVersion' must have four numeric segments."
  }
  if ($Matches[4] -ne '0') {
    throw "Store version '$StoreVersion' must end in revision 0; Partner Center reserves the revision."
  }
  if ([int]$Matches[1] -lt 1) {
    throw "Store version '$StoreVersion' must start at major version 1."
  }

  $e = { param($value) [System.Security.SecurityElement]::Escape($value) }
  $name = & $e $IdentityName
  $publisher = & $e $IdentityPublisher
  $publisherDisplay = & $e $PublisherDisplayName
  $display = & $e $script:ProStoreMsixDisplayName

  $text = @"
<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10" xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10" xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities" IgnorableNamespaces="uap rescap">
  <Identity Name="$name" Publisher="$publisher" Version="$StoreVersion" ProcessorArchitecture="x64" />
  <Properties>
    <DisplayName>$display</DisplayName>
    <PublisherDisplayName>$publisherDisplay</PublisherDisplayName>
    <Description>$display</Description>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Resources>
    <Resource Language="en-us" />
  </Resources>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.22621.0" />
  </Dependencies>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
  </Capabilities>
  <Applications>
    <Application Id="$($script:ProStoreMsixApplicationId)" Executable="$($script:ProStoreMsixExecutable)" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="$display" Description="$display" BackgroundColor="transparent" Square150x150Logo="Assets\Square150x150Logo.png" Square44x44Logo="Assets\Square44x44Logo.png">
        <uap:DefaultTile Wide310x150Logo="Assets\Wide310x150Logo.png" Square71x71Logo="Assets\Square71x71Logo.png" Square310x310Logo="Assets\Square310x310Logo.png" />
      </uap:VisualElements>
    </Application>
  </Applications>
</Package>
"@

  [xml]$manifest = $text
  return $manifest
}

function New-ProStoreMsixAssets {
  <#
  .SYNOPSIS
    Render every manifest asset from the canonical app icon, at its own size.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [string]$SourceIcon,

    [Parameter(Mandatory = $true)]
    [string]$AssetsDirectory
  )

  Add-Type -AssemblyName System.Drawing
  New-Item -ItemType Directory -Path $AssetsDirectory -Force | Out-Null

  # Blank canvases at the required sizes; Update-StoreMsixIconAssets keeps each
  # file's size and letterboxes the icon over it (the same renderer as Lite).
  foreach ($asset in $script:ProStoreMsixAssetSizes.GetEnumerator()) {
    $bitmap = New-Object System.Drawing.Bitmap($asset.Value[0], $asset.Value[1])
    try { $bitmap.Save((Join-Path $AssetsDirectory $asset.Key), [System.Drawing.Imaging.ImageFormat]::Png) }
    finally { $bitmap.Dispose() }
  }

  Update-StoreMsixIconAssets -SourceIcon $SourceIcon -AssetsDirectory $AssetsDirectory
}

function Expand-ProStoreMsixMsiLayout {
  <#
  .SYNOPSIS
    Extract the Pro MSI with an administrative install; return the app directory.

  .DESCRIPTION
    `msiexec /a` copies the files to TARGETDIR without installing anything or
    running custom actions (no WebView2 bootstrapper, no registry). The app
    directory is the one holding entropia-pro-desktop.exe, exactly as the MSI
    would install it under Program Files.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [string]$MsiPath,

    [Parameter(Mandatory = $true)]
    [string]$Destination
  )

  $msi = (Resolve-Path -LiteralPath $MsiPath).Path
  if (Test-Path -LiteralPath $Destination) {
    Remove-Item -LiteralPath $Destination -Recurse -Force
  }
  New-Item -ItemType Directory -Path $Destination -Force | Out-Null
  $Destination = (Resolve-Path -LiteralPath $Destination).Path
  $log = Join-Path (Split-Path -Parent $Destination) "msi-admin-extract.log"

  $process = Start-Process -FilePath "msiexec.exe" -Wait -PassThru -NoNewWindow -ArgumentList @(
    "/a", "`"$msi`"", "/qn", "TARGETDIR=`"$Destination`"", "/L*v", "`"$log`""
  )
  if ($process.ExitCode -ne 0) {
    throw "msiexec /a failed with exit code $($process.ExitCode); see $log"
  }

  $executables = @(Get-ChildItem -LiteralPath $Destination -Recurse -Filter $script:ProStoreMsixExecutable -File)
  if ($executables.Count -ne 1) {
    throw "Expected exactly one $($script:ProStoreMsixExecutable) in the extracted MSI; found $($executables.Count)."
  }
  return $executables[0].DirectoryName
}

function New-ProStoreMsixPackage {
  <#
  .SYNOPSIS
    Pack an installed Pro layout into an unsigned, Store-ready MSIX.

  .DESCRIPTION
    The layout is copied, never modified. The copy loses every .lnk, gains the
    generated Assets and AppxManifest (with the Pro connector execution alias),
    and is checked before and after `makeappx pack`.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [string]$PayloadDirectory,

    [Parameter(Mandatory = $true)]
    [string]$OutDir,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$IdentityName,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$IdentityPublisher,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$PublisherDisplayName,

    [Parameter(Mandatory = $true)]
    [string]$StoreVersion,

    [Parameter(Mandatory = $true)]
    [string]$MakeAppx,

    [Parameter(Mandatory = $true)]
    [string]$SourceIcon
  )

  foreach ($required in @($script:ProStoreMsixExecutable, $script:ProStoreMsixConnectorFileName, $script:StoreMsixPdfiumRelativePath)) {
    if (-not (Test-Path -LiteralPath (Join-Path $PayloadDirectory $required) -PathType Leaf)) {
      throw "The Pro layout $PayloadDirectory has no $required; build the MSI with the tauri.connector.pro.conf.json overlay."
    }
  }

  # Validate the identity before copying a multi-hundred-MB layout.
  $manifest = New-ProStoreMsixManifest -IdentityName $IdentityName -IdentityPublisher $IdentityPublisher -PublisherDisplayName $PublisherDisplayName -StoreVersion $StoreVersion

  if (-not (Test-Path -LiteralPath $MakeAppx -PathType Leaf)) {
    throw "makeappx.exe not found: $MakeAppx"
  }

  New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
  $OutDir = (Resolve-Path -LiteralPath $OutDir).Path
  $workDir = Join-Path $OutDir "pro-store-layout"
  $output = Join-Path $OutDir "EntropIAPro-Store-$StoreVersion.msix"

  foreach ($stale in @($workDir, $output)) {
    if (Test-Path -LiteralPath $stale) { Remove-Item -LiteralPath $stale -Recurse -Force }
  }
  Copy-Item -LiteralPath (Resolve-Path -LiteralPath $PayloadDirectory).Path -Destination $workDir -Recurse

  foreach ($generated in @("AppxManifest.xml", "AppxBlockMap.xml", "AppxSignature.p7x", "[Content_Types].xml")) {
    Remove-Item -LiteralPath (Join-Path $workDir $generated) -Force -ErrorAction SilentlyContinue
  }
  $removedShortcutFiles = @(Remove-StoreMsixLegacyShortcuts -PayloadDirectory $workDir)

  $assetsDirectory = Join-Path $workDir "Assets"
  New-ProStoreMsixAssets -SourceIcon $SourceIcon -AssetsDirectory $assetsDirectory

  Add-StoreMsixConnectorAlias -Manifest $manifest -ConnectorFileName $script:ProStoreMsixConnectorFileName | Out-Null

  $manifestPath = Join-Path $workDir "AppxManifest.xml"
  $settings = New-Object System.Xml.XmlWriterSettings
  $settings.Encoding = New-Object System.Text.UTF8Encoding($false)
  $settings.Indent = $true
  $writer = [System.Xml.XmlWriter]::Create($manifestPath, $settings)
  try { $manifest.Save($writer) }
  finally { $writer.Dispose() }

  [xml]$staged = Get-Content -LiteralPath $manifestPath
  $registration = Assert-StoreMsixAppRegistration `
    -Manifest $staged `
    -PayloadDirectory $workDir `
    -ExpectedIdentityName $IdentityName `
    -ExpectedPublisher $IdentityPublisher `
    -ExpectedVersion $StoreVersion
  Assert-StoreMsixConnectorAlias -Manifest $staged -PayloadDirectory $workDir -ConnectorFileName $script:ProStoreMsixConnectorFileName

  & $MakeAppx pack /d $workDir /p $output /o | Out-Host
  if ($LASTEXITCODE -ne 0) {
    throw "makeappx pack failed with exit code $LASTEXITCODE"
  }

  Assert-StoreMsixIconAssetsInArchive -ArchivePath $output -AssetsDirectory $assetsDirectory
  Assert-StoreMsixShortcutHygieneInArchive -ArchivePath $output
  Assert-StoreMsixPdfiumInArchive -ArchivePath $output -PdfiumPath (Join-Path $PayloadDirectory $script:StoreMsixPdfiumRelativePath)
  Assert-StoreMsixConnectorInArchive -ArchivePath $output -ConnectorPath (Join-Path $PayloadDirectory $script:ProStoreMsixConnectorFileName) -ConnectorFileName $script:ProStoreMsixConnectorFileName

  return [pscustomobject]@{
    Output               = $output
    Name                 = $IdentityName
    Publisher            = $IdentityPublisher
    PublisherDisplayName = $PublisherDisplayName
    Version              = $StoreVersion
    ApplicationId        = $registration.ApplicationId
    Executable           = $registration.Executable
    PackageFamilyName    = $registration.PackageFamilyName
    Aumid                = $registration.Aumid
    ConnectorAlias       = $script:ProStoreMsixConnectorFileName
    RemovedShortcutFiles = $removedShortcutFiles
    Size                 = (Get-Item -LiteralPath $output).Length
    Sha256               = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash
  }
}
