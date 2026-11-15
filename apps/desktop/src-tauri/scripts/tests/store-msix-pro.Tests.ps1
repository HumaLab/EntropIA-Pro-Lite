Set-StrictMode -Version Latest

# EntropIA Pro has no captured base MSIX: its Store package is generated from the
# installed layout of the Pro MSI, with a manifest written from scratch. These
# tests pin that manifest, the asset set it references, the Pro connector alias,
# and (when the Windows SDK is present) a real makeappx pack of a fake layout.
Describe "Pro Store MSIX" {
  BeforeAll {
    Add-Type -AssemblyName System.Drawing
    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $script:TestRoot = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
    . (Join-Path $script:TestRoot "../store-msix-pro.ps1")

    $script:Identity = @{
      IdentityName         = "HLab.EntropIAPro"
      IdentityPublisher    = "CN=00000000-1111-2222-3333-444444444444"
      PublisherDisplayName = "HLab"
    }
    $script:SourceIcon = (Resolve-Path (Join-Path $script:TestRoot "../../icons/icon.png")).Path
    $script:MakeAppx = Find-StoreMsixMakeAppx -ErrorAction SilentlyContinue

    function New-FakeProLayout {
      param([Parameter(Mandatory = $true)][string]$Path)
      New-Item -ItemType Directory -Path (Join-Path $Path "resources/lib") -Force | Out-Null
      [System.IO.File]::WriteAllBytes((Join-Path $Path "entropia-pro-desktop.exe"), [byte[]](1..32))
      [System.IO.File]::WriteAllBytes((Join-Path $Path "entropia-pro-agent-connector.exe"), [byte[]](33..80))
      [System.IO.File]::WriteAllBytes((Join-Path $Path "resources/lib/pdfium.dll"), [byte[]](81..120))
      Set-Content -LiteralPath (Join-Path $Path "Uninstall EntropIA Pro.lnk") -Value "lnk" -Encoding Ascii
      return $Path
    }
  }

  Context "manifest" {
    It "carries the exact identity, version and a single full-trust Pro application" {
      $manifest = New-ProStoreMsixManifest @script:Identity -StoreVersion "1.8.0.0"

      $identity = $manifest.SelectSingleNode("//*[local-name()='Identity']")
      $identity.GetAttribute("Name") | Should -BeExactly "HLab.EntropIAPro"
      $identity.GetAttribute("Publisher") | Should -BeExactly "CN=00000000-1111-2222-3333-444444444444"
      $identity.GetAttribute("Version") | Should -BeExactly "1.8.0.0"
      $identity.GetAttribute("ProcessorArchitecture") | Should -BeExactly "x64"
      $manifest.SelectSingleNode("//*[local-name()='PublisherDisplayName']").InnerText | Should -BeExactly "HLab"
      $manifest.SelectSingleNode("//*[local-name()='Properties']/*[local-name()='DisplayName']").InnerText | Should -BeExactly "EntropIA Pro"

      $apps = @($manifest.SelectNodes("//*[local-name()='Application']"))
      $apps.Count | Should -Be 1
      $apps[0].GetAttribute("Id") | Should -BeExactly "EntropIAPro"
      $apps[0].GetAttribute("Executable") | Should -BeExactly "entropia-pro-desktop.exe"
      $apps[0].GetAttribute("EntryPoint") | Should -BeExactly "Windows.FullTrustApplication"

      $capabilities = @($manifest.SelectNodes("//*[local-name()='Capability']") | ForEach-Object { $_.GetAttribute("Name") })
      $capabilities | Should -Be @("runFullTrust")
      @($manifest.SelectNodes("//*[local-name()='PackageDependency']")).Count | Should -Be 0
      @($manifest.SelectNodes("//*[@Category='windows.shortcut']")).Count | Should -Be 0
    }

    It "escapes XML-significant characters in the publisher display name" {
      $values = $script:Identity.Clone()
      $values.PublisherDisplayName = "H&Lab <Pro>"
      $manifest = New-ProStoreMsixManifest @values -StoreVersion "1.8.0.0"
      $manifest.SelectSingleNode("//*[local-name()='PublisherDisplayName']").InnerText | Should -BeExactly "H&Lab <Pro>"
    }

    It "rejects a Store version whose revision is not 0" {
      { New-ProStoreMsixManifest @script:Identity -StoreVersion "1.8.0.1" } | Should -Throw "*revision*"
    }

    It "rejects an empty identity value instead of falling back to a placeholder" {
      $values = $script:Identity.Clone()
      $values.IdentityName = ""
      { New-ProStoreMsixManifest @values -StoreVersion "1.8.0.0" } | Should -Throw
    }

    It "rejects a publisher that is not a distinguished name" {
      $values = $script:Identity.Clone()
      $values.IdentityPublisher = "HLab"
      { New-ProStoreMsixManifest @values -StoreVersion "1.8.0.0" } | Should -Throw "*CN=*"
    }
  }

  Context "assets" {
    It "creates every PNG the manifest references, at its required size" {
      $assets = Join-Path $TestDrive "Assets"
      New-ProStoreMsixAssets -SourceIcon $script:SourceIcon -AssetsDirectory $assets
      $manifest = New-ProStoreMsixManifest @script:Identity -StoreVersion "1.8.0.0"

      $referenced = @(
        $manifest.SelectNodes("//@*[contains(., 'Assets\')] | //*[local-name()='Logo']") |
          ForEach-Object { if ($_ -is [System.Xml.XmlAttribute]) { $_.Value } else { $_.InnerText } }
      )
      $referenced.Count | Should -BeGreaterThan 0
      foreach ($relative in $referenced) {
        Test-Path -LiteralPath (Join-Path $TestDrive $relative) | Should -BeTrue -Because $relative
      }

      foreach ($asset in $script:ProStoreMsixAssetSizes.GetEnumerator()) {
        $bitmap = New-Object System.Drawing.Bitmap((Join-Path $assets $asset.Key))
        try {
          "$($bitmap.Width)x$($bitmap.Height)" | Should -BeExactly "$($asset.Value[0])x$($asset.Value[1])" -Because $asset.Key
        }
        finally { $bitmap.Dispose() }
      }
    }
  }

  Context "connector alias" {
    It "declares the Pro connector under its own name, not Lite's" {
      $payload = New-FakeProLayout -Path (Join-Path $TestDrive "alias")
      $manifest = New-ProStoreMsixManifest @script:Identity -StoreVersion "1.8.0.0"
      Add-StoreMsixConnectorAlias -Manifest $manifest -ConnectorFileName "entropia-pro-agent-connector.exe" | Out-Null

      { Assert-StoreMsixConnectorAlias -Manifest $manifest -PayloadDirectory $payload -ConnectorFileName "entropia-pro-agent-connector.exe" } | Should -Not -Throw
      { Assert-StoreMsixConnectorAlias -Manifest $manifest -PayloadDirectory $payload } | Should -Throw
    }
  }

  Context "package" {
    It "refuses a layout without the Pro executable" {
      $payload = New-FakeProLayout -Path (Join-Path $TestDrive "no-exe")
      Remove-Item -LiteralPath (Join-Path $payload "entropia-pro-desktop.exe")
      { New-ProStoreMsixPackage -PayloadDirectory $payload -OutDir (Join-Path $TestDrive "out-no-exe") @script:Identity -StoreVersion "1.8.0.0" -MakeAppx "unused.exe" -SourceIcon $script:SourceIcon } |
        Should -Throw "*entropia-pro-desktop.exe*"
    }

    It "refuses a layout without the Pro connector" {
      $payload = New-FakeProLayout -Path (Join-Path $TestDrive "no-connector")
      Remove-Item -LiteralPath (Join-Path $payload "entropia-pro-agent-connector.exe")
      { New-ProStoreMsixPackage -PayloadDirectory $payload -OutDir (Join-Path $TestDrive "out-no-connector") @script:Identity -StoreVersion "1.8.0.0" -MakeAppx "unused.exe" -SourceIcon $script:SourceIcon } |
        Should -Throw "*entropia-pro-agent-connector.exe*"
    }

    It "refuses a layout without Pdfium" {
      $payload = New-FakeProLayout -Path (Join-Path $TestDrive "no-pdfium")
      Remove-Item -LiteralPath (Join-Path $payload "resources/lib/pdfium.dll")
      { New-ProStoreMsixPackage -PayloadDirectory $payload -OutDir (Join-Path $TestDrive "out-no-pdfium") @script:Identity -StoreVersion "1.8.0.0" -MakeAppx "unused.exe" -SourceIcon $script:SourceIcon } |
        Should -Throw "*pdfium.dll*"
    }

    It "packs a fake layout into an identity-correct, shortcut-free MSIX" {
      if (-not $script:MakeAppx) { Set-ItResult -Skipped -Because "makeappx.exe (Windows SDK) is not installed"; return }

      $payload = New-FakeProLayout -Path (Join-Path $TestDrive "layout")
      $result = New-ProStoreMsixPackage -PayloadDirectory $payload -OutDir (Join-Path $TestDrive "out") @script:Identity -StoreVersion "1.8.0.0" -MakeAppx $script:MakeAppx -SourceIcon $script:SourceIcon

      Split-Path -Leaf $result.Output | Should -BeExactly "EntropIAPro-Store-1.8.0.0.msix"
      $result.Name | Should -BeExactly "HLab.EntropIAPro"
      $result.Version | Should -BeExactly "1.8.0.0"
      $result.ConnectorAlias | Should -BeExactly "entropia-pro-agent-connector.exe"
      $result.Aumid | Should -BeLike "HLab.EntropIAPro_*!EntropIAPro"

      $zip = [System.IO.Compression.ZipFile]::OpenRead($result.Output)
      try {
        $names = @($zip.Entries | ForEach-Object { $_.FullName })
        $names | Should -Contain "entropia-pro-desktop.exe"
        $names | Should -Contain "entropia-pro-agent-connector.exe"
        $names | Should -Contain "resources/lib/pdfium.dll"
        @($names | Where-Object { $_ -like "*.lnk" }).Count | Should -Be 0
      }
      finally { $zip.Dispose() }

      # The source layout is never modified: the .lnk is dropped from a copy.
      Test-Path -LiteralPath (Join-Path $payload "Uninstall EntropIA Pro.lnk") | Should -BeTrue
    }
  }
}
