function Get-StoreMsixPngAssets {
  param(
    [Parameter(Mandatory = $true)]
    [string]$AssetsDirectory
  )

  if (-not (Test-Path -LiteralPath $AssetsDirectory -PathType Container)) {
    throw "Store assets directory not found: $AssetsDirectory"
  }

  $assets = @(
    Get-ChildItem -LiteralPath $AssetsDirectory -Filter "*.png" -File |
      Sort-Object Name
  )
  if ($assets.Count -eq 0) {
    throw "No Store PNG assets found in: $AssetsDirectory"
  }

  return $assets
}

function Get-Sha256HexFromStream {
  param(
    [Parameter(Mandatory = $true)]
    [System.IO.Stream]$Stream
  )

  $sha256 = [System.Security.Cryptography.SHA256]::Create()
  try {
    $hash = $sha256.ComputeHash($Stream)
    return -join ($hash | ForEach-Object { $_.ToString("x2") })
  }
  finally {
    $sha256.Dispose()
  }
}

function Update-StoreMsixIconAssets {
  param(
    [Parameter(Mandatory = $true)]
    [string]$SourceIcon,

    [Parameter(Mandatory = $true)]
    [string]$AssetsDirectory
  )

  if (-not (Test-Path -LiteralPath $SourceIcon -PathType Leaf)) {
    throw "Canonical Store icon not found: $SourceIcon"
  }

  Add-Type -AssemblyName System.Drawing
  $assets = @(Get-StoreMsixPngAssets -AssetsDirectory $AssetsDirectory)
  $sourceStream = [System.IO.File]::OpenRead((Resolve-Path -LiteralPath $SourceIcon).Path)
  try {
    $sourceBitmap = [System.Drawing.Bitmap]::FromStream($sourceStream)
    try {
      foreach ($asset in $assets) {
        $existingBitmap = New-Object System.Drawing.Bitmap($asset.FullName)
        try {
          $width = $existingBitmap.Width
          $height = $existingBitmap.Height
        }
        finally {
          $existingBitmap.Dispose()
        }

        $scale = [Math]::Min($width / $sourceBitmap.Width, $height / $sourceBitmap.Height)
        $drawWidth = [Math]::Max(1, [int][Math]::Round($sourceBitmap.Width * $scale))
        $drawHeight = [Math]::Max(1, [int][Math]::Round($sourceBitmap.Height * $scale))
        $left = [int][Math]::Floor(($width - $drawWidth) / 2)
        $top = [int][Math]::Floor(($height - $drawHeight) / 2)

        $rendered = New-Object System.Drawing.Bitmap($width, $height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
          $graphics = [System.Drawing.Graphics]::FromImage($rendered)
          try {
            $graphics.Clear([System.Drawing.Color]::Transparent)
            $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
            $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
            $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
            $destination = New-Object System.Drawing.Rectangle($left, $top, $drawWidth, $drawHeight)
            $graphics.DrawImage($sourceBitmap, $destination)
          }
          finally {
            $graphics.Dispose()
          }

          $temporaryPath = "$($asset.FullName).new"
          try {
            $rendered.Save($temporaryPath, [System.Drawing.Imaging.ImageFormat]::Png)
            Move-Item -LiteralPath $temporaryPath -Destination $asset.FullName -Force
          }
          finally {
            Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
          }
        }
        finally {
          $rendered.Dispose()
        }
      }
    }
    finally {
      $sourceBitmap.Dispose()
    }
  }
  finally {
    $sourceStream.Dispose()
  }
}

function Assert-StoreMsixIconAssetsInArchive {
  param(
    [Parameter(Mandatory = $true)]
    [string]$ArchivePath,

    [Parameter(Mandatory = $true)]
    [string]$AssetsDirectory
  )

  if (-not (Test-Path -LiteralPath $ArchivePath -PathType Leaf)) {
    throw "MSIX archive not found: $ArchivePath"
  }

  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $assets = @(Get-StoreMsixPngAssets -AssetsDirectory $AssetsDirectory)
  $archive = [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $ArchivePath).Path)
  try {
    $entriesByName = @{}
    foreach ($archiveEntry in $archive.Entries) {
      $normalizedName = $archiveEntry.FullName.Replace(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
      )
      $entriesByName[$normalizedName] = $archiveEntry
    }

    foreach ($asset in $assets) {
      $entryName = "Assets/$($asset.Name)"
      $entry = $entriesByName[$entryName]
      if (-not $entry) {
        throw "Packed Store icon asset missing: $entryName"
      }

      $sourceStream = [System.IO.File]::OpenRead($asset.FullName)
      try { $sourceHash = Get-Sha256HexFromStream -Stream $sourceStream }
      finally { $sourceStream.Dispose() }

      $packedStream = $entry.Open()
      try { $packedHash = Get-Sha256HexFromStream -Stream $packedStream }
      finally { $packedStream.Dispose() }

      if ($sourceHash -ne $packedHash) {
        throw "Packed Store icon asset differs from generated source: $entryName"
      }
    }
  }
  finally {
    $archive.Dispose()
  }
}
