Set-StrictMode -Version Latest

Describe "Store MSIX shortcut hygiene" {
  BeforeAll {
    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $script:TestRoot = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
    . (Join-Path $script:TestRoot "../store-msix-shortcuts.ps1")

    function New-CapturedManifest {
      param(
        [Parameter(Mandatory = $true)]
        [string]$Path
      )

      $xml = @'
<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10" xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10" xmlns:desktop7="http://schemas.microsoft.com/appx/manifest/desktop/windows10/7" xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities" IgnorableNamespaces="uap desktop7 rescap">
  <Identity Name="CONICET.EntropIALite" Publisher="CN=89DF40E5-581A-4120-9A24-F701205485D6" Version="1.0.11.0" ProcessorArchitecture="x64" />
  <Properties>
    <DisplayName>EntropIA Lite</DisplayName>
    <PublisherDisplayName>HLab</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Applications>
    <Application Id="EntropIALite" Executable="entropia-lite-desktop.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements BackgroundColor="transparent" DisplayName="EntropIA Lite" Square150x150Logo="Assets\S150.png" Square44x44Logo="Assets\S44.png" Description="EntropIA Lite" />
      <Extensions>
        <desktop7:Extension Category="windows.shortcut">
          <desktop7:Shortcut File="[{Desktop}]\EntropIA Lite.lnk" Icon="[{Package}]\entropia-lite-desktop.exe" Description="Runs EntropIA Lite" />
        </desktop7:Extension>
        <desktop7:Extension Category="windows.shortcut">
          <desktop7:Shortcut File="[{Common Programs}]\EntropIA Lite\EntropIA Lite.lnk" Icon="[{Package}]\entropia-lite-desktop.exe" Description="Runs EntropIA Lite" />
        </desktop7:Extension>
      </Extensions>
    </Application>
  </Applications>
</Package>
'@

      Set-Content -LiteralPath $Path -Value $xml -Encoding UTF8
    }
  }

  BeforeEach {
    $script:Payload = Join-Path $TestDrive "payload"
    New-Item -Path $script:Payload -ItemType Directory -Force | Out-Null
    New-Item -Path (Join-Path $script:Payload "VFS/Common Desktop") -ItemType Directory -Force | Out-Null
    New-Item -Path (Join-Path $script:Payload "VFS/Common Programs/EntropIA Lite") -ItemType Directory -Force | Out-Null
    New-Item -Path (Join-Path $script:Payload "Assets") -ItemType Directory -Force | Out-Null

    Set-Content -LiteralPath (Join-Path $script:Payload "entropia-lite-desktop.exe") -Value "MZ" -Encoding Ascii
    Set-Content -LiteralPath (Join-Path $script:Payload "Assets/StoreLogo.png") -Value "png" -Encoding Ascii
    Set-Content -LiteralPath (Join-Path $script:Payload "Uninstall EntropIA Lite.lnk") -Value "lnk" -Encoding Ascii
    Set-Content -LiteralPath (Join-Path $script:Payload "VFS/Common Desktop/EntropIA Lite.lnk") -Value "lnk" -Encoding Ascii
    Set-Content -LiteralPath (Join-Path $script:Payload "VFS/Common Programs/EntropIA Lite/EntropIA Lite.lnk") -Value "lnk" -Encoding Ascii

    $script:ManifestPath = Join-Path $script:Payload "AppxManifest.xml"
    New-CapturedManifest -Path $script:ManifestPath
  }

  Context "Remove-StoreMsixLegacyShortcuts" {
    It "removes every captured .lnk from the payload and reports them" {
      $removed = @(Remove-StoreMsixLegacyShortcuts -PayloadDirectory $script:Payload)

      $removed | Should -Contain "Uninstall EntropIA Lite.lnk"
      $removed | Should -Contain "VFS/Common Desktop/EntropIA Lite.lnk"
      $removed | Should -Contain "VFS/Common Programs/EntropIA Lite/EntropIA Lite.lnk"
      @(Get-ChildItem -LiteralPath $script:Payload -Filter "*.lnk" -Recurse -File).Count | Should -Be 0
    }

    It "prunes the directories the removed shortcuts left empty without touching live payload" {
      Remove-StoreMsixLegacyShortcuts -PayloadDirectory $script:Payload | Out-Null

      Test-Path -LiteralPath (Join-Path $script:Payload "VFS/Common Desktop") | Should -BeFalse
      Test-Path -LiteralPath (Join-Path $script:Payload "VFS/Common Programs") | Should -BeFalse
      Test-Path -LiteralPath (Join-Path $script:Payload "Assets/StoreLogo.png") | Should -BeTrue
      Test-Path -LiteralPath (Join-Path $script:Payload "entropia-lite-desktop.exe") | Should -BeTrue
      Test-Path -LiteralPath $script:Payload | Should -BeTrue
    }

    It "is idempotent on an already clean payload" {
      Remove-StoreMsixLegacyShortcuts -PayloadDirectory $script:Payload | Out-Null

      @(Remove-StoreMsixLegacyShortcuts -PayloadDirectory $script:Payload).Count | Should -Be 0
    }

    It "rejects a missing payload directory" {
      { Remove-StoreMsixLegacyShortcuts -PayloadDirectory (Join-Path $TestDrive "nope") } |
        Should -Throw "MSIX payload directory not found*"
    }
  }

  Context "Remove-StoreMsixShortcutExtensions" {
    It "drops every windows.shortcut extension and reports the shortcut files it declared" {
      [xml]$manifest = Get-Content -LiteralPath $script:ManifestPath

      $removed = @(Remove-StoreMsixShortcutExtensions -Manifest $manifest)

      $removed | Should -Contain "[{Desktop}]\EntropIA Lite.lnk"
      $removed | Should -Contain "[{Common Programs}]\EntropIA Lite\EntropIA Lite.lnk"
      @($manifest.SelectNodes("//*[local-name()='Extension'][@Category='windows.shortcut']")).Count | Should -Be 0
    }

    It "removes the Extensions container once it holds nothing else" {
      [xml]$manifest = Get-Content -LiteralPath $script:ManifestPath

      Remove-StoreMsixShortcutExtensions -Manifest $manifest | Out-Null

      @($manifest.SelectNodes("//*[local-name()='Extensions']")).Count | Should -Be 0
    }

    It "keeps an Extensions container that still declares a non-shortcut extension" {
      [xml]$manifest = Get-Content -LiteralPath $script:ManifestPath
      $extensions = $manifest.SelectSingleNode("//*[local-name()='Extensions']")
      $keeper = $manifest.CreateElement("uap", "Extension", "http://schemas.microsoft.com/appx/manifest/uap/windows10")
      $keeper.SetAttribute("Category", "windows.fileTypeAssociation")
      $extensions.AppendChild($keeper) | Out-Null

      Remove-StoreMsixShortcutExtensions -Manifest $manifest | Out-Null

      @($manifest.SelectNodes("//*[local-name()='Extensions']")).Count | Should -Be 1
      @($manifest.SelectNodes("//*[local-name()='Extension']")).Count | Should -Be 1
    }

    It "leaves the application registration untouched" {
      [xml]$manifest = Get-Content -LiteralPath $script:ManifestPath

      Remove-StoreMsixShortcutExtensions -Manifest $manifest | Out-Null

      $app = $manifest.SelectSingleNode("//*[local-name()='Application']")
      $app.Id | Should -Be "EntropIALite"
      $app.Executable | Should -Be "entropia-lite-desktop.exe"
      $app.EntryPoint | Should -Be "Windows.FullTrustApplication"
    }

    It "is idempotent" {
      [xml]$manifest = Get-Content -LiteralPath $script:ManifestPath
      Remove-StoreMsixShortcutExtensions -Manifest $manifest | Out-Null

      @(Remove-StoreMsixShortcutExtensions -Manifest $manifest).Count | Should -Be 0
    }
  }

  Context "Assert-StoreMsixAppRegistration" {
    BeforeEach {
      [xml]$script:Manifest = Get-Content -LiteralPath $script:ManifestPath
      Remove-StoreMsixShortcutExtensions -Manifest $script:Manifest | Out-Null
    }

    It "accepts a correctly registered, shortcut-free package" {
      $registration = Assert-StoreMsixAppRegistration `
        -Manifest $script:Manifest `
        -PayloadDirectory $script:Payload `
        -ExpectedIdentityName "CONICET.EntropIALite" `
        -ExpectedPublisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" `
        -ExpectedVersion "1.0.11.0"

      $registration.ApplicationId | Should -Be "EntropIALite"
      $registration.Executable | Should -Be "entropia-lite-desktop.exe"
      $registration.EntryPoint | Should -Be "Windows.FullTrustApplication"
      $registration.PackageFamilyName | Should -Be "CONICET.EntropIALite_b16na7gwepwme"
      $registration.Aumid | Should -Be "CONICET.EntropIALite_b16na7gwepwme!EntropIALite"
    }

    It "rejects an identity that drifted from the Partner Center binding" {
      $identity = $script:Manifest.SelectSingleNode("//*[local-name()='Identity']")
      $identity.SetAttribute("Name", "CONICET.EntropIALiteTypo")

      { Assert-StoreMsixAppRegistration -Manifest $script:Manifest -PayloadDirectory $script:Payload -ExpectedIdentityName "CONICET.EntropIALite" -ExpectedPublisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" -ExpectedVersion "1.0.11.0" } |
        Should -Throw "*Identity Name*"
    }

    It "rejects a Store version whose revision is not zero" {
      $identity = $script:Manifest.SelectSingleNode("//*[local-name()='Identity']")
      $identity.SetAttribute("Version", "1.0.11.3")

      { Assert-StoreMsixAppRegistration -Manifest $script:Manifest -PayloadDirectory $script:Payload -ExpectedIdentityName "CONICET.EntropIALite" -ExpectedPublisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" -ExpectedVersion "1.0.11.3" } |
        Should -Throw "*revision*"
    }

    It "rejects an Executable that is missing from the payload" {
      $app = $script:Manifest.SelectSingleNode("//*[local-name()='Application']")
      $app.SetAttribute("Executable", "not-shipped.exe")

      { Assert-StoreMsixAppRegistration -Manifest $script:Manifest -PayloadDirectory $script:Payload -ExpectedIdentityName "CONICET.EntropIALite" -ExpectedPublisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" -ExpectedVersion "1.0.11.0" } |
        Should -Throw "*not-shipped.exe*"
    }

    It "rejects an Executable declared through an absolute WindowsApps path" {
      $app = $script:Manifest.SelectSingleNode("//*[local-name()='Application']")
      $app.SetAttribute("Executable", "C:\Program Files\WindowsApps\CONICET.EntropIALite_1.0.11.0_x64__b16na7gwepwme\entropia-lite-desktop.exe")

      { Assert-StoreMsixAppRegistration -Manifest $script:Manifest -PayloadDirectory $script:Payload -ExpectedIdentityName "CONICET.EntropIALite" -ExpectedPublisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" -ExpectedVersion "1.0.11.0" } |
        Should -Throw "*package-relative*"
    }

    It "rejects a missing EntryPoint for a full-trust desktop app" {
      $app = $script:Manifest.SelectSingleNode("//*[local-name()='Application']")
      $app.SetAttribute("EntryPoint", "")

      { Assert-StoreMsixAppRegistration -Manifest $script:Manifest -PayloadDirectory $script:Payload -ExpectedIdentityName "CONICET.EntropIALite" -ExpectedPublisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" -ExpectedVersion "1.0.11.0" } |
        Should -Throw "*EntryPoint*"
    }

    It "rejects a package that still declares a windows.shortcut extension" {
      [xml]$dirty = Get-Content -LiteralPath $script:ManifestPath

      { Assert-StoreMsixAppRegistration -Manifest $dirty -PayloadDirectory $script:Payload -ExpectedIdentityName "CONICET.EntropIALite" -ExpectedPublisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" -ExpectedVersion "1.0.11.0" } |
        Should -Throw "*windows.shortcut*"
    }
  }

  Context "Assert-StoreMsixShortcutHygieneInArchive" {
    BeforeEach {
      [xml]$manifest = Get-Content -LiteralPath $script:ManifestPath
      Remove-StoreMsixShortcutExtensions -Manifest $manifest | Out-Null
      $manifest.Save($script:ManifestPath)
      $script:Archive = Join-Path $TestDrive "package-$([guid]::NewGuid().ToString('n')).msix"
    }

    It "passes for a payload with no shortcut of any kind" {
      Remove-StoreMsixLegacyShortcuts -PayloadDirectory $script:Payload | Out-Null
      [System.IO.Compression.ZipFile]::CreateFromDirectory($script:Payload, $script:Archive)

      { Assert-StoreMsixShortcutHygieneInArchive -ArchivePath $script:Archive } | Should -Not -Throw
    }

    It "fails when a captured .lnk survived into the packed MSIX" {
      [System.IO.Compression.ZipFile]::CreateFromDirectory($script:Payload, $script:Archive)

      { Assert-StoreMsixShortcutHygieneInArchive -ArchivePath $script:Archive } |
        Should -Throw "*EntropIA*Lite.lnk*"
    }

    It "fails when the packed manifest still declares a windows.shortcut extension" {
      Remove-StoreMsixLegacyShortcuts -PayloadDirectory $script:Payload | Out-Null
      New-CapturedManifest -Path $script:ManifestPath
      [System.IO.Compression.ZipFile]::CreateFromDirectory($script:Payload, $script:Archive)

      { Assert-StoreMsixShortcutHygieneInArchive -ArchivePath $script:Archive } |
        Should -Throw "*windows.shortcut*"
    }

    It "rejects a missing archive" {
      { Assert-StoreMsixShortcutHygieneInArchive -ArchivePath (Join-Path $TestDrive "absent.msix") } |
        Should -Throw "MSIX archive not found*"
    }
  }

  Context "Get-StoreMsixPackageFamilyName" {
    It "derives the published package family name from the Store identity" {
      Get-StoreMsixPackageFamilyName -IdentityName "CONICET.EntropIALite" -Publisher "CN=89DF40E5-581A-4120-9A24-F701205485D6" |
        Should -Be "CONICET.EntropIALite_b16na7gwepwme"
    }

    It "derives a family name that carries no version segment" {
      $family = Get-StoreMsixPackageFamilyName -IdentityName "CONICET.EntropIALite" -Publisher "CN=89DF40E5-581A-4120-9A24-F701205485D6"

      $family | Should -Not -Match "\d+\.\d+\.\d+\.\d+"
    }
  }
}
