Set-StrictMode -Version Latest

# The captured base MSIX predates Pdfium, and the repack only swaps the exe in,
# so the Store build could not open, split or crop a PDF (1.0.15 and earlier).
# The app looks for the library at resources\lib\pdfium.dll beside the exe
# (src/ocr/pdf.rs, bundled_pdfium_candidate_paths).
Describe "Store MSIX payload: Pdfium" {
  BeforeAll {
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $script:TestRoot = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
    . (Join-Path $script:TestRoot "../store-msix-payload.ps1")

    function New-FakePdfium {
      param([Parameter(Mandatory = $true)][string]$Path)
      [System.IO.File]::WriteAllBytes($Path, [byte[]](1..64))
      return $Path
    }

    function New-ArchiveWith {
      param(
        [Parameter(Mandatory = $true)][string]$Path,
        [string]$EntryName,
        [byte[]]$Bytes
      )
      $zip = [System.IO.Compression.ZipFile]::Open($Path, [System.IO.Compression.ZipArchiveMode]::Create)
      try {
        $exe = $zip.CreateEntry("entropia-lite-desktop.exe")
        $stream = $exe.Open(); $stream.Dispose()
        if ($EntryName) {
          $entry = $zip.CreateEntry($EntryName)
          $stream = $entry.Open()
          try { $stream.Write($Bytes, 0, $Bytes.Length) }
          finally { $stream.Dispose() }
        }
      }
      finally { $zip.Dispose() }
      return $Path
    }
  }

  BeforeEach {
    $script:Work = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $script:Work | Out-Null
    $script:Payload = Join-Path $script:Work "payload"
    New-Item -ItemType Directory -Path $script:Payload | Out-Null
    $script:Pdfium = New-FakePdfium -Path (Join-Path $script:Work "pdfium.dll")
  }

  AfterEach {
    Remove-Item -LiteralPath $script:Work -Recurse -Force -ErrorAction SilentlyContinue
  }

  It "places pdfium.dll under resources\lib beside the exe" {
    $placed = Add-StoreMsixPdfium -PdfiumPath $script:Pdfium -PayloadDirectory $script:Payload

    $placed | Should -Be (Join-Path $script:Payload "resources\lib\pdfium.dll")
    [System.IO.File]::ReadAllBytes($placed) | Should -Be ([System.IO.File]::ReadAllBytes($script:Pdfium))
  }

  It "refuses a missing library instead of packing a Store build without it" {
    { Add-StoreMsixPdfium -PdfiumPath (Join-Path $script:Work "nope.dll") -PayloadDirectory $script:Payload } |
      Should -Throw "*Pdfium library not found*"
  }

  It "accepts a packed archive that carries the same library" {
    $archive = New-ArchiveWith -Path (Join-Path $script:Work "ok.msix") `
      -EntryName "resources/lib/pdfium.dll" -Bytes ([System.IO.File]::ReadAllBytes($script:Pdfium))

    { Assert-StoreMsixPdfiumInArchive -ArchivePath $archive -PdfiumPath $script:Pdfium } | Should -Not -Throw
  }

  It "rejects a packed archive without the library" {
    $archive = New-ArchiveWith -Path (Join-Path $script:Work "missing.msix")

    { Assert-StoreMsixPdfiumInArchive -ArchivePath $archive -PdfiumPath $script:Pdfium } |
      Should -Throw "*resources/lib/pdfium.dll not found*"
  }

  It "rejects a packed archive whose library differs from the one given" {
    $archive = New-ArchiveWith -Path (Join-Path $script:Work "stale.msix") `
      -EntryName "resources/lib/pdfium.dll" -Bytes ([byte[]](1..8))

    { Assert-StoreMsixPdfiumInArchive -ArchivePath $archive -PdfiumPath $script:Pdfium } |
      Should -Throw "*size*"
  }
}
