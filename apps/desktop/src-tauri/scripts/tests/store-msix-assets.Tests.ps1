Set-StrictMode -Version Latest

Describe "Store MSIX icon assets" {
  BeforeAll {
    Add-Type -AssemblyName System.Drawing
    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $script:TestRoot = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
    $script:AssetScript = Join-Path $script:TestRoot "../store-msix-assets.ps1"
    . $script:AssetScript

    function New-TestPng {
      param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [int]$Width,
        [Parameter(Mandatory = $true)]
        [int]$Height,
        [Parameter(Mandatory = $true)]
        [System.Drawing.Color]$Color
      )

      $bitmap = New-Object System.Drawing.Bitmap($Width, $Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
      try {
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try { $graphics.Clear($Color) }
        finally { $graphics.Dispose() }
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
      }
      finally { $bitmap.Dispose() }
    }
  }

  BeforeEach {
    $script:FixtureRoot = Join-Path $TestDrive "fixture"
    $script:AssetsDirectory = Join-Path $script:FixtureRoot "Assets"
    New-Item -Path $script:AssetsDirectory -ItemType Directory -Force | Out-Null

    $script:SourceIcon = Join-Path $script:FixtureRoot "icon.png"
    New-TestPng -Path $script:SourceIcon -Width 64 -Height 64 -Color ([System.Drawing.Color]::FromArgb(255, 220, 30, 40))

    New-TestPng -Path (Join-Path $script:AssetsDirectory "EntropIALite-Square44x44Logo.targetsize-32.png") -Width 32 -Height 32 -Color ([System.Drawing.Color]::Blue)
    New-TestPng -Path (Join-Path $script:AssetsDirectory "EntropIALite-Wide310x150Logo.scale-100.png") -Width 310 -Height 150 -Color ([System.Drawing.Color]::Blue)
  }

  It "regenerates every existing PNG from the canonical icon while preserving required dimensions" {
    Update-StoreMsixIconAssets -SourceIcon $script:SourceIcon -AssetsDirectory $script:AssetsDirectory

    $square = New-Object System.Drawing.Bitmap((Join-Path $script:AssetsDirectory "EntropIALite-Square44x44Logo.targetsize-32.png"))
    $wide = New-Object System.Drawing.Bitmap((Join-Path $script:AssetsDirectory "EntropIALite-Wide310x150Logo.scale-100.png"))
    try {
      $square.Width | Should -Be 32
      $square.Height | Should -Be 32
      $square.GetPixel(16, 16).R | Should -Be 220
      $square.GetPixel(16, 16).B | Should -Be 40

      $wide.Width | Should -Be 310
      $wide.Height | Should -Be 150
      $wide.GetPixel(155, 75).R | Should -Be 220
      $wide.GetPixel(0, 75).A | Should -Be 0
    }
    finally {
      $square.Dispose()
      $wide.Dispose()
    }
  }

  It "rejects an asset directory without PNG targets" {
    Remove-Item -Path (Join-Path $script:AssetsDirectory "*.png") -Force

    { Update-StoreMsixIconAssets -SourceIcon $script:SourceIcon -AssetsDirectory $script:AssetsDirectory } |
      Should -Throw "No Store PNG assets found*"
  }

  It "detects when a packed MSIX asset differs from the generated payload" {
    Update-StoreMsixIconAssets -SourceIcon $script:SourceIcon -AssetsDirectory $script:AssetsDirectory

    $archivePath = Join-Path $TestDrive "fixture.msix"
    [System.IO.Compression.ZipFile]::CreateFromDirectory($script:FixtureRoot, $archivePath)

    Assert-StoreMsixIconAssetsInArchive -ArchivePath $archivePath -AssetsDirectory $script:AssetsDirectory

    $tamperedRoot = Join-Path $TestDrive "tampered"
    [System.IO.Compression.ZipFile]::ExtractToDirectory($archivePath, $tamperedRoot)
    New-TestPng -Path (Join-Path $tamperedRoot "Assets/EntropIALite-Square44x44Logo.targetsize-32.png") -Width 32 -Height 32 -Color ([System.Drawing.Color]::Black)
    Remove-Item -LiteralPath $archivePath -Force
    [System.IO.Compression.ZipFile]::CreateFromDirectory($tamperedRoot, $archivePath)

    { Assert-StoreMsixIconAssetsInArchive -ArchivePath $archivePath -AssetsDirectory $script:AssetsDirectory } |
      Should -Throw "Packed Store icon asset differs from generated source:*"
  }
}
