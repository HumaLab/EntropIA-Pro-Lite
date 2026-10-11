#Requires -Version 5.1
<#
.SYNOPSIS
  Pre-bundle hook (Windows-only): stages the WebView2 repair wrapper for the MSI.

  Downloads the exact Microsoft WebView2 evergreen bootstrapper Tauri embeds,
  verifies it over plain HTTPS (default certificate validation, no TLS bypass)
  with a valid Authenticode signature from the exact Microsoft signer, then
  compiles windows/webview2-repair.rs with the system rustc (static CRT, no new
  dependencies) into target/webview2-installer/repair.exe, which the WiX
  fragment embeds as a Binary. Also compiles and runs the wrapper unit tests so
  the Windows filesystem/ACL branches are exercised on the real platform.

.DESCRIPTION
  Fail-closed throughout: any unexpected triple, download, signature, compile,
  or test failure throws, so `beforeBundleCommand` aborts the bundle instead of
  shipping an unsigned, unverified, or untested wrapper. The bootstrapper path
  is exposed to rustc only through $env:WEBVIEW2_BOOTSTRAPPER_PATH for the
  duration of the production compile and is restored in `finally`.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# The exact evergreen bootstrapper URL Tauri uses.
$BootstrapperUrl = 'https://go.microsoft.com/fwlink/p/?LinkId=2124703'
# Exact accepted signer identity: compared per-field with case-sensitive
# equality (never a subject substring) to avoid false-positive matches.
$ExpectedSignerCN = 'Microsoft Corporation'
$ExpectedSignerO = 'Microsoft Corporation'
$SupportedTriples = @('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')

# All repo paths derive from this script's location, never from the caller cwd.
$ScriptsDir = $PSScriptRoot
$TauriDir = Split-Path -Parent $ScriptsDir
$SourcePath = Join-Path $TauriDir 'windows\webview2-repair.rs'
$OutDir = Join-Path $TauriDir 'target\webview2-installer'
$BootstrapperPath = Join-Path $OutDir 'MicrosoftEdgeWebview2Setup.exe'
$RepairPath = Join-Path $OutDir 'repair.exe'
$RepairTestsPath = Join-Path $OutDir 'repair-tests.exe'

$Triple = $env:TAURI_ENV_TARGET_TRIPLE
if ([string]::IsNullOrWhiteSpace($Triple) -or ($SupportedTriples -notcontains $Triple)) {
    throw "Unrecognized TAURI_ENV_TARGET_TRIPLE '$Triple'. Expected one of: $($SupportedTriples -join ', '). Refusing to guess the Rust target."
}
if (-not (Test-Path -LiteralPath $SourcePath -PathType Leaf)) {
    throw "Repair wrapper source not found at '$SourcePath'."
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Write-Host "[webview2-repair] target triple: $Triple"
Write-Host "[webview2-repair] source:        $SourcePath"
Write-Host "[webview2-repair] output dir:    $OutDir"

Write-Host "[webview2-repair] downloading $BootstrapperUrl"
Invoke-WebRequest -Uri $BootstrapperUrl -OutFile $BootstrapperPath -UseBasicParsing

$signature = Get-AuthenticodeSignature -FilePath $BootstrapperPath
if ($signature.Status -ne 'Valid') {
    throw "Bootstrapper Authenticode status is '$($signature.Status)', expected 'Valid'. File: $BootstrapperPath"
}
$rdn = @{}
foreach ($part in $signature.SignerCertificate.Subject -split ',\s*') {
    $kv = $part -split '=', 2
    if ($kv.Count -eq 2) { $rdn[$kv[0]] = $kv[1] }
}
if ($rdn['CN'] -cne $ExpectedSignerCN -or $rdn['O'] -cne $ExpectedSignerO) {
    throw "Bootstrapper signer '$($signature.SignerCertificate.Subject)' is not the accepted Microsoft signer (CN='$ExpectedSignerCN', O='$ExpectedSignerO')."
}

# Build provenance for the CI log: hash, signature, and file version.
$hash = (Get-FileHash -LiteralPath $BootstrapperPath -Algorithm SHA256).Hash
$version = (Get-Item -LiteralPath $BootstrapperPath).VersionInfo.FileVersion
Write-Host "[webview2-repair] sha256:         $hash"
Write-Host "[webview2-repair] signature:      $($signature.Status) ($($signature.SignerCertificate.Subject), $($signature.SignerCertificate.Thumbprint))"
Write-Host "[webview2-repair] file version:   $version"

# Exercise the Windows-only branches (private ACL staging, locked read-back)
# on the real platform before embedding anything.
Write-Host '[webview2-repair] compiling wrapper unit tests'
& rustc --edition=2021 --test $SourcePath -o $RepairTestsPath
if ($LASTEXITCODE -ne 0) {
    throw "rustc --test failed with exit code $LASTEXITCODE."
}
Write-Host '[webview2-repair] running wrapper unit tests'
& $RepairTestsPath
if ($LASTEXITCODE -ne 0) {
    throw "wrapper unit tests failed with exit code $LASTEXITCODE."
}

# Production compile: static CRT, size-optimized, abort on panic — no MSVC
# runtime DLLs, no .NET, no new crates. The payload path env var lives only
# for this invocation and is restored in `finally`.
$previousPayloadPath = $env:WEBVIEW2_BOOTSTRAPPER_PATH
try {
    $env:WEBVIEW2_BOOTSTRAPPER_PATH = $BootstrapperPath
    Write-Host '[webview2-repair] compiling repair.exe'
    & rustc --edition=2021 -C panic=abort -C opt-level=s -C target-feature=+crt-static --target $Triple $SourcePath -o $RepairPath
    if ($LASTEXITCODE -ne 0) {
        throw "rustc failed with exit code $LASTEXITCODE."
    }
}
finally {
    if ($null -eq $previousPayloadPath) {
        Remove-Item Env:\WEBVIEW2_BOOTSTRAPPER_PATH -ErrorAction SilentlyContinue
    }
    else {
        $env:WEBVIEW2_BOOTSTRAPPER_PATH = $previousPayloadPath
    }
}

$repairHash = (Get-FileHash -LiteralPath $RepairPath -Algorithm SHA256).Hash
Write-Host "[webview2-repair] repair.exe:      $RepairPath"
Write-Host "[webview2-repair] repair.exe sha:  $repairHash"
