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

# The captured base MSIX declares no execution alias, and the packaged exe lives
# under the versioned WindowsApps directory, which Windows blocks from an
# ordinary command line. Without the alias the Settings › Asistentes lines cannot
# start the connector at all (D15).
Describe "Store MSIX payload: assistant connector" {
  BeforeAll {
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $script:TestRoot = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
    . (Join-Path $script:TestRoot "../store-msix-payload.ps1")

    $script:ConnectorName = "entropia-agent-connector.exe"

    function New-FakeConnector {
      param([Parameter(Mandatory = $true)][string]$Path)
      [System.IO.File]::WriteAllBytes($Path, [byte[]](1..64))
      return $Path
    }

    function New-FakeManifest {
      # Returns the path, never the XmlDocument: PowerShell enumerates an
      # XmlDocument's children when it comes back through the pipeline.
      param([Parameter(Mandatory = $true)][string]$Path)
      $text = @'
<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10" xmlns:desktop7="http://schemas.microsoft.com/appx/manifest/desktop/windows10/7">
  <Identity Name="CONICET.EntropIALite" Publisher="CN=test" Version="1.1.1.0" ProcessorArchitecture="x64" />
  <Applications>
    <Application Id="EntropIALite" Executable="entropia-lite-desktop.exe" EntryPoint="Windows.FullTrustApplication" />
  </Applications>
</Package>
'@
      [System.IO.File]::WriteAllText($Path, $text)
      return $Path
    }

    function New-ArchiveWithConnector {
      param(
        [Parameter(Mandatory = $true)][string]$Path,
        [byte[]]$Bytes
      )
      $zip = [System.IO.Compression.ZipFile]::Open($Path, [System.IO.Compression.ZipArchiveMode]::Create)
      try {
        if ($Bytes) {
          $entry = $zip.CreateEntry($script:ConnectorName)
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
    $script:Connector = New-FakeConnector -Path (Join-Path $script:Work "built.exe")
    $script:ManifestPath = New-FakeManifest -Path (Join-Path $script:Work "AppxManifest.xml")
    [xml]$script:Manifest = Get-Content -LiteralPath $script:ManifestPath
  }

  AfterEach {
    Remove-Item -LiteralPath $script:Work -Recurse -Force -ErrorAction SilentlyContinue
  }

  It "places the connector in the payload root under the alias name" {
    $placed = Add-StoreMsixConnector -ConnectorPath $script:Connector -PayloadDirectory $script:Payload

    $placed | Should -Be (Join-Path $script:Payload $script:ConnectorName)
    [System.IO.File]::ReadAllBytes($placed) | Should -Be ([System.IO.File]::ReadAllBytes($script:Connector))
  }

  It "refuses a missing connector instead of packing a Store build without it" {
    { Add-StoreMsixConnector -ConnectorPath (Join-Path $script:Work "nope.exe") -PayloadDirectory $script:Payload } |
      Should -Throw "*Agent connector not found*"
  }

  It "declares one execution alias that survives a manifest round trip" {
    Add-StoreMsixConnectorAlias -Manifest $script:Manifest | Out-Null
    Add-StoreMsixConnector -ConnectorPath $script:Connector -PayloadDirectory $script:Payload | Out-Null

    $script:Manifest.DocumentElement.GetNamespaceOfPrefix("uap3") |
      Should -Be "http://schemas.microsoft.com/appx/manifest/uap/windows10/3"

    [xml]$reloaded = $script:Manifest.OuterXml
    { Assert-StoreMsixConnectorAlias -Manifest $reloaded -PayloadDirectory $script:Payload } | Should -Not -Throw
    $aliases = @($reloaded.SelectNodes("//*[local-name()='Extension'][@Category='windows.appExecutionAlias']"))
    $aliases.Count | Should -Be 1
    $aliases[0].GetAttribute("Executable") | Should -Be $script:ConnectorName
    $aliases[0].GetAttribute("EntryPoint") | Should -Be "Windows.FullTrustApplication"
  }

  It "does not duplicate the alias when it is added twice" {
    Add-StoreMsixConnectorAlias -Manifest $script:Manifest | Out-Null
    Add-StoreMsixConnectorAlias -Manifest $script:Manifest | Out-Null
    Add-StoreMsixConnector -ConnectorPath $script:Connector -PayloadDirectory $script:Payload | Out-Null

    @($script:Manifest.SelectNodes("//*[local-name()='Extension'][@Category='windows.appExecutionAlias']")).Count |
      Should -Be 1
  }

  It "rejects a manifest that declares no alias" {
    Add-StoreMsixConnector -ConnectorPath $script:Connector -PayloadDirectory $script:Payload | Out-Null

    { Assert-StoreMsixConnectorAlias -Manifest $script:Manifest -PayloadDirectory $script:Payload } |
      Should -Throw "*exactly one windows.appExecutionAlias*"
  }

  It "rejects an alias whose payload file is missing" {
    Add-StoreMsixConnectorAlias -Manifest $script:Manifest | Out-Null

    { Assert-StoreMsixConnectorAlias -Manifest $script:Manifest -PayloadDirectory $script:Payload } |
      Should -Throw "*no $($script:ConnectorName)*"
  }

  It "accepts a packed archive that carries the same exe" {
    $archive = New-ArchiveWithConnector -Path (Join-Path $script:Work "ok.msix") `
      -Bytes ([System.IO.File]::ReadAllBytes($script:Connector))

    { Assert-StoreMsixConnectorInArchive -ArchivePath $archive -ConnectorPath $script:Connector } | Should -Not -Throw
  }

  It "rejects a packed archive without the exe" {
    $archive = New-ArchiveWithConnector -Path (Join-Path $script:Work "missing.msix")

    { Assert-StoreMsixConnectorInArchive -ArchivePath $archive -ConnectorPath $script:Connector } |
      Should -Throw "*not found*"
  }

  It "rejects a packed archive whose exe differs from the one given" {
    $archive = New-ArchiveWithConnector -Path (Join-Path $script:Work "stale.msix") -Bytes ([byte[]](1..8))

    { Assert-StoreMsixConnectorInArchive -ArchivePath $archive -ConnectorPath $script:Connector } |
      Should -Throw "*size*"
  }
}
