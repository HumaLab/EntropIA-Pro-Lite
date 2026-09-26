Set-StrictMode -Version Latest

# Native libraries the Store MSIX must carry beyond the exe.
#
# The captured base MSIX predates them and the repack only swaps the exe in, so
# every file the app loads at runtime has to be added here explicitly. The app
# looks for Pdfium at resources\lib\pdfium.dll beside the exe, the same place the
# NSIS/MSI installers put it (tauri.windows.conf.json bundle.resources).

$script:StoreMsixPdfiumRelativePath = "resources/lib/pdfium.dll"

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
