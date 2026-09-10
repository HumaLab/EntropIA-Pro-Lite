Set-StrictMode -Version Latest

function Test-RequiresRustEvidence {
  param(
    [string[]]$ChangedFiles
  )

  foreach ($file in $ChangedFiles) {
    if ($file -match "\.rs$") {
      return $true
    }
  }

  return $false
}

function Write-RustVerifyEvidence {
  param(
    [Parameter(Mandatory = $true)][string]$EvidencePath,
    [Parameter(Mandatory = $true)][bool]$RequiresRustEvidence,
    [hashtable]$Classification = @{}
  )

  $parent = Split-Path -Parent $EvidencePath
  if (-not [string]::IsNullOrWhiteSpace($parent)) {
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
  }

  if (-not $RequiresRustEvidence) {
    @(
      "## Rust Verify Evidence",
      "- Scope: out-of-scope",
      "- Justification: no .rs files changed in this changeset."
    ) | Set-Content -Path $EvidencePath -Encoding UTF8
    return
  }

  @(
    "## Rust Coverage Baseline (default lean features)",
    "- Status: $($Classification.coverage)",
    "",
    "## Rust Quality Report (fmt/clippy)",
    "- fmt: $($Classification.fmt)",
    "- clippy: $($Classification.clippy)",
    "",
    "## Classification",
    "- coverage: $($Classification.coverage)",
    "- fmt: $($Classification.fmt)",
    "- clippy: $($Classification.clippy)"
  ) | Set-Content -Path $EvidencePath -Encoding UTF8
}

function Get-ChangedFilesFromGit {
  param(
    [string]$BaseRef = "origin/main"
  )

  $output = & git diff --name-only "$BaseRef...HEAD" 2>$null
  if (-not $output) {
    return @()
  }

  return @($output)
}

# The job only runs when Rust changed, so the gate enforces whatever the report
# classified. It does not re-derive scope from git: on a push to main,
# origin/main...HEAD is empty and would wave everything through.
function Get-RustQualityGateFailures {
  param(
    [Parameter(Mandatory = $true)][string]$FmtStatus,
    [Parameter(Mandatory = $true)][string]$ClippyStatus
  )

  $failures = @()
  if ($FmtStatus -ne "pass") {
    $failures += "fmt"
  }
  if ($ClippyStatus -ne "pass") {
    $failures += "clippy"
  }

  return $failures
}

function Invoke-RustVerifyGate {
  param(
    [string]$EvidencePath = "apps/desktop/src-tauri/coverage-rust/rust-verify-evidence.md",
    [string[]]$ChangedFiles,
    [string]$CoverageStatus = "infra-error",
    [string]$FmtStatus = "infra-error",
    [string]$ClippyStatus = "infra-error"
  )

  if (-not $ChangedFiles) {
    $ChangedFiles = Get-ChangedFilesFromGit
  }

  $requires = Test-RequiresRustEvidence -ChangedFiles $ChangedFiles

  Write-RustVerifyEvidence -EvidencePath $EvidencePath -RequiresRustEvidence $requires -Classification @{
    coverage = $CoverageStatus
    fmt = $FmtStatus
    clippy = $ClippyStatus
  }

  return [pscustomobject]@{
    RequiresRustEvidence = $requires
    EvidencePath = $EvidencePath
  }
}

if ($MyInvocation.InvocationName -ne ".") {
  $summaryPath = "apps/desktop/src-tauri/coverage-rust/summary.md"
  $coverageStatus = "infra-error"
  $fmtStatus = "infra-error"
  $clippyStatus = "infra-error"

  if (Test-Path -Path $summaryPath) {
    $summary = Get-Content -Path $summaryPath -Raw

    if ($summary -match "coverage: (pass|report-only|infra-error)") {
      $coverageStatus = $Matches[1]
    }
    if ($summary -match "fmt: (pass|report-only|infra-error)") {
      $fmtStatus = $Matches[1]
    }
    if ($summary -match "clippy: (pass|report-only|infra-error)") {
      $clippyStatus = $Matches[1]
    }
  }

  $result = Invoke-RustVerifyGate -CoverageStatus $coverageStatus -FmtStatus $fmtStatus -ClippyStatus $clippyStatus
  Write-Host "[verify] rust evidence required: $($result.RequiresRustEvidence)"
  Write-Host "[verify] evidence: $($result.EvidencePath)"

  $failures = @(Get-RustQualityGateFailures -FmtStatus $fmtStatus -ClippyStatus $clippyStatus)
  if ($failures.Count -gt 0) {
    Write-Host "[gate] FAIL: $($failures -join ', ') did not pass (fmt=$fmtStatus, clippy=$clippyStatus). See fmt.log and clippy.log in the rust-quality-report artifact."
    exit 1
  }
  Write-Host "[gate] PASS: rustfmt and clippy are clean"
}
