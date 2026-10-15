Set-StrictMode -Version Latest

Describe "rust-quality-report workflow" {
  BeforeAll {
    function Assert-True {
      param(
        [bool]$Condition,
        [string]$Message
      )

      if (-not $Condition) {
        throw $Message
      }
    }

    function Assert-Match {
      param(
        [string]$Value,
        [string]$Pattern,
        [string]$Message
      )

      if ($Value -notmatch $Pattern) {
        throw $Message
      }
    }

    $script:TestRoot = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
    $script:RepoRoot = (Resolve-Path -Path (Join-Path $script:TestRoot "../../../../..")).Path
    $script:workflowPath = Join-Path -Path $script:RepoRoot -ChildPath ".github/workflows/ci.yml"
  }

  It "resolves workflow path robustly" {
    Assert-True -Condition (-not [string]::IsNullOrWhiteSpace($script:workflowPath)) -Message "workflowPath should not be null or empty"
    Assert-True -Condition (Test-Path -Path $script:workflowPath) -Message "workflowPath should exist"
    Assert-Match -Value $script:workflowPath -Pattern "[\\/]\.github[\\/]workflows[\\/]ci\.yml$" -Message "workflowPath should target .github/workflows/ci.yml"
  }

  It "installs rustfmt and clippy components" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern "rustup component add llvm-tools-preview rustfmt clippy" -Message "workflow must install llvm-tools-preview rustfmt clippy"
  }

  It "keeps cargo-llvm-cov installation step" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern "Install cargo-llvm-cov" -Message "workflow must keep cargo-llvm-cov installation step"
  }

  It "runs Rust quality Pester suites explicitly in CI" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern "Run Rust quality Pester suites" -Message "workflow must contain explicit Pester suites step"
    Assert-Match -Value $content -Pattern "Invoke-Pester" -Message "workflow must invoke Pester explicitly"
    Assert-Match -Value $content -Pattern "rust-quality-contract.Tests.ps1" -Message "workflow must run rust-quality-contract suite"
    Assert-Match -Value $content -Pattern "rust-verify-gate.Tests.ps1" -Message "workflow must run rust-verify-gate suite"
    Assert-Match -Value $content -Pattern "ci-rust-quality-workflow.Tests.ps1" -Message "workflow must run workflow contract suite"
  }

  It "no longer carries pnpm forensics diagnostics" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-True -Condition ($content -notmatch "forensics") -Message "workflow must not contain pnpm forensics diagnostics"
    Assert-True -Condition ($content -notmatch "pnpm-preinstall-classify") -Message "workflow must not contain pnpm pre-install classification diagnostics"
  }

  It "pins rust-quality-report setup-node to Node 22 for pnpm/action-setup compatibility" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern "rust-quality-report:[\s\S]*?- uses: actions/setup-node@[0-9a-f]{40} # v\d+\.[\s\S]*?node-version:\s*22" -Message "rust-quality-report must pin actions/setup-node to Node 22 to keep pnpm/action-setup on a compatible runtime"
  }

  It "restores canonical lockfile after setup-node and before install in rust-quality-report" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $jobIndex = $content.IndexOf("rust-quality-report:")
    $nodeSetupIndex = $content.IndexOf("- uses: actions/setup-node@", $jobIndex)
    $restoreIndex = $content.IndexOf("Restore canonical lockfile (rust-quality-report)", $jobIndex)
    $installIndex = $content.IndexOf("name: Install dependencies", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must include rust-quality-report job"
    Assert-True -Condition ($nodeSetupIndex -gt $jobIndex) -Message "rust-quality-report must include actions/setup-node"
    Assert-True -Condition ($restoreIndex -gt $nodeSetupIndex) -Message "rust-quality-report lockfile restore must run after actions/setup-node"
    Assert-True -Condition ($installIndex -gt $restoreIndex) -Message "rust-quality-report lockfile restore must run before Install dependencies"

    Assert-Match -Value $content -Pattern "Restore canonical lockfile \(rust-quality-report\)[\s\S]*?run:\s*git checkout -- pnpm-lock\.yaml" -Message "rust-quality-report lockfile restore must use deterministic git checkout from committed state"
  }

  It "uploads baseline coverage artifacts lcov and summary" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern "Upload rust quality artifacts" -Message "workflow must upload rust quality artifacts"
    Assert-Match -Value $content -Pattern "coverage-rust/lcov.info" -Message "workflow artifacts must include lcov.info"
    Assert-Match -Value $content -Pattern "coverage-rust/summary.md" -Message "workflow artifacts must include summary.md"
  }

  It "pins pnpm/action-setup id and uses explicit pnpm bin path in rust-quality-report install" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?- uses: pnpm/action-setup@[0-9a-f]{40} # v6\.[\s\S]*?id:\s*pnpm_setup' -Message "rust-quality-report pnpm/action-setup must define id: pnpm_setup"
    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?name:\s*Install dependencies[\s\S]*?steps\.pnpm_setup\.outputs\.bin_dest' -Message "rust-quality-report install must use steps.pnpm_setup.outputs.bin_dest"
    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?name:\s*Install dependencies[\s\S]*?Write-Host\s+"pnpm_bin=' -Message "rust-quality-report install must print pnpm binary path"
    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?name:\s*Install dependencies[\s\S]*?&\s*\$pnpmExe\s+-v' -Message "rust-quality-report install must print pnpm version from pinned binary"
    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?name:\s*Install dependencies[\s\S]*?node\s+-v' -Message "rust-quality-report install must print node version"
    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?name:\s*Install dependencies[\s\S]*?npm\s+exec\s+--package=pnpm@9\.15\.4\s+--\s+pnpm\s+install\s+--frozen-lockfile' -Message "rust-quality-report install must execute frozen lockfile install through npm exec pnpm@9.15.4"
  }

  It "restores lockfile inside rust-quality-report install step before pnpm install" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?npm\s+exec\s+--package=pnpm@9\.15\.4\s+--\s+pnpm\s+install\s+--frozen-lockfile' -Message "rust-quality-report install step must restore canonical lockfile immediately before npm exec pnpm install"
  }

  It "emits same-step lockfile diagnostics inside rust-quality-report install" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?lockfile_diag\.sha256=' -Message "rust-quality-report install must emit same-step lockfile sha256 diagnostic"
    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?lockfile_diag\.first_line_is_yaml_doc=' -Message "rust-quality-report install must emit same-step YAML doc first-line diagnostic"
    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?lockfile_diag\.head_blob=' -Message "rust-quality-report install must emit same-step HEAD lockfile blob diagnostic"
    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?lockfile_diag\.matches_head_blob=[\s\S]*?npm\s+exec\s+--package=pnpm@9\.15\.4\s+--\s+pnpm\s+install\s+--frozen-lockfile' -Message "rust-quality-report install must compare working lockfile to HEAD blob before npm exec pnpm install"
  }

  It "blocks the build when rustfmt or clippy report findings" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $gate = [regex]::Match($content, '- name: Rust verify gate \(blocks on rustfmt/clippy\)[\s\S]*?(?=\r?\n      - name:)')
    Assert-True -Condition $gate.Success -Message "rust-quality-report must run the rust verify gate as its own step"
    Assert-Match -Value $gate.Value -Pattern "rust-verify-gate\.ps1" -Message "the gate step must run rust-verify-gate.ps1"
    Assert-True -Condition ($gate.Value -notmatch "continue-on-error\s*:") -Message "the gate step must not continue on error"
  }

  It "pins the research engine to a GitHub commit in Cargo.lock" {
    $lockPath = Join-Path -Path $script:RepoRoot -ChildPath "apps/desktop/src-tauri/Cargo.lock"
    $lock = Get-Content -Path $lockPath -Raw

    # A local [patch] for the sibling checkout rewrites this entry without a
    # source; committing that lock would silently unpin the engine in CI.
    Assert-Match -Value $lock -Pattern 'name = "entropia-agent"\r?\nversion = "[^"]+"\r?\nsource = "git\+https://github\.com/HumaLab/EntropIA-Agent(\?[^#"]*)?#[0-9a-f]{40}"' -Message "Cargo.lock must pin entropia-agent to a commit of github.com/HumaLab/EntropIA-Agent"
  }

  It "lints and tests the Pro local-ml build on Windows" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $job = [regex]::Match($content, 'windows-rust-feature-contract:[\s\S]*?(?=\r?\n  [a-z-]+:\r?\n)')
    Assert-True -Condition $job.Success -Message "workflow must include the windows-rust-feature-contract job"
    Assert-Match -Value $job.Value -Pattern "cargo clippy --manifest-path apps/desktop/src-tauri/Cargo\.toml --features local-ml --all-targets -- -D warnings" -Message "the Pro job must run clippy on local-ml with -D warnings"
    Assert-Match -Value $job.Value -Pattern "cargo test --manifest-path apps/desktop/src-tauri/Cargo\.toml --features local-ml" -Message "the Pro job must run the local-ml test suite"
  }
}
