Set-StrictMode -Version Latest

# Native libraries and binaries the Store MSIX must carry beyond the exe.
#
# The captured base MSIX predates them and the repack only swaps the exe in, so
# every file the app loads at runtime has to be added here explicitly. The app
# looks for Pdfium at resources\lib\pdfium.dll beside the exe, the same place the
# NSIS/MSI installers put it (tauri.windows.conf.json bundle.resources).
#
# The assistant connector is the one file an outside program launches instead of
# the app: the Settings › Asistentes lines name it, and inside the package the
# physical exe sits under the versioned WindowsApps directory, which Windows
# refuses to launch from an ordinary command line. So the payload carries it and
# the manifest declares a `uap3:AppExecutionAlias` with the same name (D15),
# which resolves for the signed-in user and runs the exe in package context.

$script:StoreMsixPdfiumRelativePath = "resources/lib/pdfium.dll"
$script:StoreMsixConnectorFileName = "entropia-agent-connector.exe"
$script:StoreMsixUap3Namespace = "http://schemas.microsoft.com/appx/manifest/uap/windows10/3"
$script:StoreMsixDesktopNamespace = "http://schemas.microsoft.com/appx/manifest/desktop/windows10"

function Add-StoreMsixPdfium {
  param(
    [Parameter(Mandatory = $true)]
    [string]$PdfiumPath,

    [Parameter(Mandatory = $true)]
    [string]$PayloadDirectory
  )

  if (-not (Test-Path -LiteralPath $PdfiumPath -PathType Leaf)) {
    throw "Pdfium library not found: $PdfiumPath"
  }

  $destination = Join-Path $PayloadDirectory ($script:StoreMsixPdfiumRelativePath -replace '/', '\')
  New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
  Copy-Item -LiteralPath $PdfiumPath -Destination $destination -Force
  return $destination
}

function Assert-StoreMsixPdfiumInArchive {
  param(
    [Parameter(Mandatory = $true)]
    [string]$ArchivePath,

    [Parameter(Mandatory = $true)]
    [string]$PdfiumPath
  )

  $expected = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $PdfiumPath).Path)
  $expectedHash = [System.BitConverter]::ToString(
    [System.Security.Cryptography.SHA256]::Create().ComputeHash($expected))

  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = [System.IO.Compression.ZipFile]::OpenRead($ArchivePath)
  try {
    $entry = $zip.GetEntry($script:StoreMsixPdfiumRelativePath)
    if (-not $entry) {
      throw "$($script:StoreMsixPdfiumRelativePath) not found in $ArchivePath"
    }
    if ($entry.Length -ne $expected.Length) {
      throw "$($script:StoreMsixPdfiumRelativePath) in the MSIX has size $($entry.Length), expected $($expected.Length)"
    }

    $stream = $entry.Open()
    try {
      $memory = New-Object System.IO.MemoryStream
      $stream.CopyTo($memory)
      $actualHash = [System.BitConverter]::ToString(
        [System.Security.Cryptography.SHA256]::Create().ComputeHash($memory.ToArray()))
    }
    finally { $stream.Dispose() }

    if ($actualHash -ne $expectedHash) {
      throw "$($script:StoreMsixPdfiumRelativePath) in the MSIX differs from $PdfiumPath"
    }
  }
  finally {
    $zip.Dispose()
  }
}

function Add-StoreMsixConnector {
  <#
  .SYNOPSIS
    Copy the assistant connector into the package payload, beside the app exe.

  .DESCRIPTION
    The Settings › Asistentes lines launch this file by name, so it has to exist
    in the package root under exactly the name the execution alias declares.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [string]$ConnectorPath,

    [Parameter(Mandatory = $true)]
    [string]$PayloadDirectory
  )

  if (-not (Test-Path -LiteralPath $ConnectorPath -PathType Leaf)) {
    throw "Agent connector not found: $ConnectorPath"
  }

  $destination = Join-Path $PayloadDirectory $script:StoreMsixConnectorFileName
  Copy-Item -LiteralPath $ConnectorPath -Destination $destination -Force
  return $destination
}

function Add-StoreMsixConnectorAlias {
  <#
  .SYNOPSIS
    Declare the `uap3:AppExecutionAlias` that makes the connector launchable.

  .DESCRIPTION
    `entropia-agent-connector.exe` resolves for the signed-in user and starts the
    packaged exe in package context (which also applies the MSIX data-directory
    redirection). Any earlier alias declaration is dropped first, so re-repacking
    cannot duplicate it. The `uap3` and `desktop` prefixes are declared on
    `<Package>` when the captured manifest does not already carry them.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [xml]$Manifest
  )

  $package = $Manifest.DocumentElement
  if (-not $package) { throw "MSIX manifest has no document element." }

  if (-not $package.GetNamespaceOfPrefix("uap3")) {
    $namespace = $Manifest.CreateAttribute("xmlns", "uap3", "http://www.w3.org/2000/xmlns/")
    $namespace.Value = $script:StoreMsixUap3Namespace
    [void]$package.Attributes.Append($namespace)
  }
  if (-not $package.GetNamespaceOfPrefix("desktop")) {
    $namespace = $Manifest.CreateAttribute("xmlns", "desktop", "http://www.w3.org/2000/xmlns/")
    $namespace.Value = $script:StoreMsixDesktopNamespace
    [void]$package.Attributes.Append($namespace)
  }

  $application = $Manifest.SelectSingleNode("//*[local-name()='Applications']/*[local-name()='Application']")
  if (-not $application) { throw "MSIX manifest declares no Application node." }

  foreach ($stale in @($Manifest.SelectNodes("//*[local-name()='Extension'][@Category='windows.appExecutionAlias']"))) {
    [void]$stale.ParentNode.RemoveChild($stale)
  }

  $extensions = $application.SelectSingleNode("./*[local-name()='Extensions']")
  if (-not $extensions) {
    $extensions = $Manifest.CreateElement("Extensions", $application.NamespaceURI)
    [void]$application.AppendChild($extensions)
  }

  $extension = $Manifest.CreateElement("uap3", "Extension", $script:StoreMsixUap3Namespace)
  $extension.SetAttribute("Category", "windows.appExecutionAlias")
  $extension.SetAttribute("Executable", $script:StoreMsixConnectorFileName)
  $extension.SetAttribute("EntryPoint", "Windows.FullTrustApplication")

  $aliasContainer = $Manifest.CreateElement("uap3", "AppExecutionAlias", $script:StoreMsixUap3Namespace)
  $executionAlias = $Manifest.CreateElement("desktop", "ExecutionAlias", $script:StoreMsixDesktopNamespace)
  $executionAlias.SetAttribute("Alias", $script:StoreMsixConnectorFileName)

  [void]$aliasContainer.AppendChild($executionAlias)
  [void]$extension.AppendChild($aliasContainer)
  [void]$extensions.AppendChild($extension)

  return $extension
}

function Assert-StoreMsixConnectorAlias {
  <#
  .SYNOPSIS
    Verify the manifest declares exactly one working alias for the connector.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [xml]$Manifest,

    [Parameter(Mandatory = $true)]
    [string]$PayloadDirectory
  )

  if (-not (Test-Path -LiteralPath $PayloadDirectory -PathType Container)) {
    throw "MSIX payload directory not found: $PayloadDirectory"
  }

  $packaged = Join-Path $PayloadDirectory $script:StoreMsixConnectorFileName
  if (-not (Test-Path -LiteralPath $packaged -PathType Leaf)) {
    throw "MSIX payload has no $($script:StoreMsixConnectorFileName); the execution alias would resolve to nothing."
  }

  $aliases = @($Manifest.SelectNodes("//*[local-name()='Extension'][@Category='windows.appExecutionAlias']"))
  if ($aliases.Count -ne 1) {
    throw "MSIX manifest must declare exactly one windows.appExecutionAlias extension; found $($aliases.Count)."
  }

  $alias = $aliases[0]
  $executable = $alias.GetAttribute("Executable")
  $entryPoint = $alias.GetAttribute("EntryPoint")
  if ($executable -cne $script:StoreMsixConnectorFileName) {
    throw "MSIX appExecutionAlias Executable '$executable' is not '$($script:StoreMsixConnectorFileName)'."
  }
  if ([System.IO.Path]::IsPathRooted($executable) -or $executable -match 'WindowsApps') {
    throw "MSIX appExecutionAlias Executable '$executable' must be package-relative."
  }
  if ($entryPoint -cne "Windows.FullTrustApplication") {
    throw "MSIX appExecutionAlias must launch Windows.FullTrustApplication; found '$entryPoint'."
  }

  $names = @(
    $alias.SelectNodes(".//*[local-name()='ExecutionAlias']") |
      ForEach-Object { $_.GetAttribute("Alias") } |
      Where-Object { $_ }
  )
  if ($names.Count -ne 1 -or $names[0] -cne $script:StoreMsixConnectorFileName) {
    throw "MSIX appExecutionAlias must declare the single alias '$($script:StoreMsixConnectorFileName)'; found: $($names -join ', ')."
  }
}

function Assert-StoreMsixConnectorInArchive {
  <#
  .SYNOPSIS
    Final gate on the packed MSIX: the connector bytes match the built exe.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [string]$ArchivePath,

    [Parameter(Mandatory = $true)]
    [string]$ConnectorPath
  )

  $expected = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $ConnectorPath).Path)
  $expectedHash = [System.BitConverter]::ToString(
    [System.Security.Cryptography.SHA256]::Create().ComputeHash($expected))

  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = [System.IO.Compression.ZipFile]::OpenRead($ArchivePath)
  try {
    $entry = $zip.GetEntry($script:StoreMsixConnectorFileName)
    if (-not $entry) {
      throw "$($script:StoreMsixConnectorFileName) not found in $ArchivePath"
    }
    if ($entry.Length -ne $expected.Length) {
      throw "$($script:StoreMsixConnectorFileName) in the MSIX has size $($entry.Length), expected $($expected.Length)"
    }

    $stream = $entry.Open()
    try {
      $memory = New-Object System.IO.MemoryStream
      $stream.CopyTo($memory)
      $actualHash = [System.BitConverter]::ToString(
        [System.Security.Cryptography.SHA256]::Create().ComputeHash($memory.ToArray()))
    }
    finally { $stream.Dispose() }

    if ($actualHash -ne $expectedHash) {
      throw "$($script:StoreMsixConnectorFileName) in the MSIX differs from $ConnectorPath"
    }
  }
  finally {
    $zip.Dispose()
  }
}
