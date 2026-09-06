# Shortcut hygiene for the Microsoft Store MSIX.
#
# The vendored base MSIX was captured from the Win32 MSI, so it inherits two
# MSI-era shortcut mechanisms that are wrong for a Store package:
#
#   1. Literal `.lnk` files under `VFS\Common Desktop` and `VFS\Common Programs`.
#      MSIX deploys them verbatim, and their target resolves inside
#      `C:\Program Files\WindowsApps\<identity>_<version>_...`, which Windows
#      refuses to launch from an ordinary shortcut.
#   2. `desktop7:Extension Category="windows.shortcut"` declarations whose
#      `Icon` points at `[{Package}]\...`, a path that carries the package
#      version and therefore breaks on every Store update.
#
# A Store package needs neither: the `<Application>` node alone registers Start,
# taskbar pinning and Windows Search through the package identity. Anything a
# user wants on the desktop must go through the AUMID
# (`shell:AppsFolder\<PackageFamilyName>!<ApplicationId>`), never the physical
# exe. These helpers strip both mechanisms and assert the identity that replaces
# them. The Win32 MSI/NSIS installers are untouched — they keep their classic
# shortcuts, which are valid under `C:\Program Files`.

Set-StrictMode -Version Latest

# Crockford-style base32 alphabet used by Windows for the publisher hash
# (digits and lowercase letters, minus i, l, o and u).
$script:StoreMsixPublisherIdAlphabet = "0123456789abcdefghjkmnpqrstvwxyz"

function Get-StoreMsixPackageFamilyName {
  <#
  .SYNOPSIS
    Derive the version-independent package family name from the Store identity.

  .DESCRIPTION
    PackageFamilyName = "<Identity Name>_<PublisherId>", where PublisherId is
    the first 64 bits of SHA-256 over the UTF-16LE publisher string, padded to
    65 bits and rendered as 13 base32 characters. It never contains the package
    version, which is exactly why AUMID-based shortcuts survive Store updates.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [string]$IdentityName,

    [Parameter(Mandatory = $true)]
    [string]$Publisher
  )

  $sha256 = [System.Security.Cryptography.SHA256]::Create()
  try {
    $hash = $sha256.ComputeHash([System.Text.Encoding]::Unicode.GetBytes($Publisher))
  }
  finally {
    $sha256.Dispose()
  }

  $bits = New-Object System.Collections.Generic.List[int]
  for ($byteIndex = 0; $byteIndex -lt 8; $byteIndex++) {
    for ($bitIndex = 7; $bitIndex -ge 0; $bitIndex--) {
      $bits.Add((($hash[$byteIndex] -shr $bitIndex) -band 1))
    }
  }
  $bits.Add(0)

  $publisherId = New-Object System.Text.StringBuilder
  for ($offset = 0; $offset -lt $bits.Count; $offset += 5) {
    $value = 0
    for ($step = 0; $step -lt 5; $step++) {
      $value = ($value * 2) + $bits[$offset + $step]
    }
    [void]$publisherId.Append($script:StoreMsixPublisherIdAlphabet[$value])
  }

  return "{0}_{1}" -f $IdentityName, $publisherId.ToString()
}

function Get-StoreMsixAumid {
  <#
  .SYNOPSIS
    Build the Application User Model ID that identity-based shortcuts must use.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [string]$PackageFamilyName,

    [Parameter(Mandatory = $true)]
    [string]$ApplicationId
  )

  return "{0}!{1}" -f $PackageFamilyName, $ApplicationId
}

function Remove-StoreMsixLegacyShortcuts {
  <#
  .SYNOPSIS
    Delete every `.lnk` inherited from the MSI capture and prune the folders
    they leave empty. Returns the removed payload-relative paths.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [string]$PayloadDirectory
  )

  if (-not (Test-Path -LiteralPath $PayloadDirectory -PathType Container)) {
    throw "MSIX payload directory not found: $PayloadDirectory"
  }

  $root = (Resolve-Path -LiteralPath $PayloadDirectory).Path
  $shortcuts = @(Get-ChildItem -LiteralPath $root -Filter "*.lnk" -Recurse -File -Force)

  $removed = New-Object System.Collections.Generic.List[string]
  foreach ($shortcut in $shortcuts) {
    $relative = $shortcut.FullName.Substring($root.Length).TrimStart('\', '/').Replace('\', '/')
    Remove-Item -LiteralPath $shortcut.FullName -Force
    $removed.Add($relative)
  }

  if ($removed.Count -gt 0) {
    Remove-StoreMsixEmptyDirectory -Root $root
  }

  return @($removed | Sort-Object)
}

function Remove-StoreMsixEmptyDirectory {
  <#
  .SYNOPSIS
    Bottom-up prune of directories that hold nothing, never the payload root.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [string]$Root
  )

  $pruning = $true
  while ($pruning) {
    $pruning = $false
    $directories = @(
      Get-ChildItem -LiteralPath $Root -Directory -Recurse -Force |
        Sort-Object { $_.FullName.Length } -Descending
    )

    foreach ($directory in $directories) {
      $hasContent = @(Get-ChildItem -LiteralPath $directory.FullName -Force).Count -gt 0
      if (-not $hasContent) {
        Remove-Item -LiteralPath $directory.FullName -Force
        $pruning = $true
      }
    }
  }
}

function Remove-StoreMsixShortcutExtensions {
  <#
  .SYNOPSIS
    Strip every `windows.shortcut` extension from the manifest, dropping any
    `<Extensions>` container it empties. Returns the shortcut files that were
    declared, for release logging.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [xml]$Manifest
  )

  $removed = New-Object System.Collections.Generic.List[string]
  $extensions = @($Manifest.SelectNodes("//*[local-name()='Extension'][@Category='windows.shortcut']"))

  foreach ($extension in $extensions) {
    foreach ($shortcut in @($extension.SelectNodes("./*[local-name()='Shortcut']"))) {
      $file = $shortcut.GetAttribute("File")
      if ($file) { $removed.Add($file) }
    }
    [void]$extension.ParentNode.RemoveChild($extension)
  }

  foreach ($container in @($Manifest.SelectNodes("//*[local-name()='Extensions']"))) {
    if (@($container.SelectNodes("./*")).Count -eq 0) {
      [void]$container.ParentNode.RemoveChild($container)
    }
  }

  return @($removed)
}

function Assert-StoreMsixAppRegistration {
  <#
  .SYNOPSIS
    Verify the manifest registers the app through package identity alone.

  .DESCRIPTION
    Checks the Partner Center identity binding, the 4-segment/zero-revision
    Store version, and the single `<Application>` node's Id / Executable /
    EntryPoint — including that the Executable is package-relative, so no
    launch surface can bake in a versioned WindowsApps path. Returns the
    resulting package family name and AUMID.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [xml]$Manifest,

    [Parameter(Mandatory = $true)]
    [string]$PayloadDirectory,

    [Parameter(Mandatory = $true)]
    [string]$ExpectedIdentityName,

    [Parameter(Mandatory = $true)]
    [string]$ExpectedPublisher,

    [Parameter(Mandatory = $true)]
    [string]$ExpectedVersion
  )

  if (-not (Test-Path -LiteralPath $PayloadDirectory -PathType Container)) {
    throw "MSIX payload directory not found: $PayloadDirectory"
  }

  $identity = $Manifest.SelectSingleNode("//*[local-name()='Identity']")
  if (-not $identity) { throw "MSIX manifest declares no Identity node." }

  $identityName = $identity.GetAttribute("Name")
  $publisher = $identity.GetAttribute("Publisher")
  $version = $identity.GetAttribute("Version")
  $architecture = $identity.GetAttribute("ProcessorArchitecture")

  if ($identityName -cne $ExpectedIdentityName) {
    throw "MSIX Identity Name '$identityName' does not match the Partner Center binding '$ExpectedIdentityName'."
  }
  if ($publisher -cne $ExpectedPublisher) {
    throw "MSIX Identity Publisher '$publisher' does not match the Partner Center binding '$ExpectedPublisher'."
  }
  if ($version -notmatch '^\d+\.\d+\.\d+\.0$') {
    throw "MSIX Identity Version '$version' is invalid: Partner Center requires four segments with a revision of 0."
  }
  if ($version -cne $ExpectedVersion) {
    throw "MSIX Identity Version '$version' does not match the requested Store version '$ExpectedVersion'."
  }
  if (-not $architecture) {
    throw "MSIX Identity declares no ProcessorArchitecture."
  }

  $applications = @($Manifest.SelectNodes("//*[local-name()='Applications']/*[local-name()='Application']"))
  if ($applications.Count -ne 1) {
    throw "MSIX manifest must declare exactly one Application; found $($applications.Count)."
  }

  $application = $applications[0]
  $applicationId = $application.GetAttribute("Id")
  $executable = $application.GetAttribute("Executable")
  $entryPoint = $application.GetAttribute("EntryPoint")

  if ($applicationId -notmatch '^[A-Za-z][A-Za-z0-9.\-]{0,63}$') {
    throw "MSIX Application Id '$applicationId' is not a valid Store application identifier."
  }
  if ($entryPoint -cne "Windows.FullTrustApplication") {
    throw "MSIX Application '$applicationId' must declare EntryPoint 'Windows.FullTrustApplication'; found '$entryPoint'."
  }
  if (-not $executable) {
    throw "MSIX Application '$applicationId' declares no Executable."
  }
  if ([System.IO.Path]::IsPathRooted($executable) -or $executable -match '(^|[\\/])\.\.([\\/]|$)' -or $executable -match 'WindowsApps') {
    throw "MSIX Application Executable '$executable' must be package-relative; an absolute or WindowsApps path breaks Store updates."
  }
  if (-not (Test-Path -LiteralPath (Join-Path $PayloadDirectory $executable) -PathType Leaf)) {
    throw "MSIX Application Executable '$executable' is not present in the packaged payload."
  }

  $visualElements = $application.SelectSingleNode("./*[local-name()='VisualElements']")
  if (-not $visualElements) {
    throw "MSIX Application '$applicationId' declares no VisualElements; Start and Search would have nothing to show."
  }
  if (-not $visualElements.GetAttribute("DisplayName")) {
    throw "MSIX Application '$applicationId' declares no VisualElements DisplayName."
  }

  $shortcutExtensions = @($Manifest.SelectNodes("//*[local-name()='Extension'][@Category='windows.shortcut']"))
  if ($shortcutExtensions.Count -gt 0) {
    throw "MSIX manifest still declares $($shortcutExtensions.Count) windows.shortcut extension(s); a Store package registers Start and Search through package identity instead."
  }

  $packageFamilyName = Get-StoreMsixPackageFamilyName -IdentityName $identityName -Publisher $publisher

  return [pscustomobject]@{
    IdentityName          = $identityName
    Publisher             = $publisher
    Version               = $version
    ProcessorArchitecture = $architecture
    ApplicationId         = $applicationId
    Executable            = $executable
    EntryPoint            = $entryPoint
    PackageFamilyName     = $packageFamilyName
    Aumid                 = Get-StoreMsixAumid -PackageFamilyName $packageFamilyName -ApplicationId $applicationId
  }
}

function Assert-StoreMsixShortcutHygieneInArchive {
  <#
  .SYNOPSIS
    Final gate on the packed MSIX: no `.lnk` payload, no `windows.shortcut`
    extension, no WindowsApps path baked into the manifest.
  #>
  param(
    [Parameter(Mandatory = $true)]
    [string]$ArchivePath
  )

  if (-not (Test-Path -LiteralPath $ArchivePath -PathType Leaf)) {
    throw "MSIX archive not found: $ArchivePath"
  }

  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $archive = [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $ArchivePath).Path)
  try {
    $shortcutEntries = @(
      $archive.Entries |
        Where-Object { $_.FullName -match '\.lnk$' } |
        ForEach-Object { [uri]::UnescapeDataString($_.FullName) } |
        Sort-Object
    )
    if ($shortcutEntries.Count -gt 0) {
      throw "Packed MSIX still ships legacy shortcut(s): $($shortcutEntries -join ', ')"
    }

    $manifestEntry = $archive.GetEntry("AppxManifest.xml")
    if (-not $manifestEntry) {
      throw "AppxManifest.xml not found in packed MSIX: $ArchivePath"
    }

    $reader = New-Object System.IO.StreamReader($manifestEntry.Open())
    try { $manifestText = $reader.ReadToEnd() }
    finally { $reader.Dispose() }
  }
  finally {
    $archive.Dispose()
  }

  [xml]$manifest = $manifestText
  $shortcutExtensions = @($manifest.SelectNodes("//*[local-name()='Extension'][@Category='windows.shortcut']"))
  if ($shortcutExtensions.Count -gt 0) {
    throw "Packed MSIX manifest still declares $($shortcutExtensions.Count) windows.shortcut extension(s)."
  }

  if ($manifestText -match 'WindowsApps') {
    throw "Packed MSIX manifest references a WindowsApps path; launch surfaces must resolve through package identity."
  }
}
