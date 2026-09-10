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

  It "runs pnpm pre-install forensics before installing dependencies" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $forensicsIndex = $content.IndexOf("Run pnpm pre-install forensics (rust-quality-report)")
    $installIndex = $content.IndexOf("name: Install dependencies")

    Assert-True -Condition ($forensicsIndex -ge 0) -Message "workflow must include rust-quality-report pre-install forensic step"
    Assert-True -Condition ($installIndex -ge 0) -Message "workflow must include Install dependencies step"
    Assert-True -Condition ($forensicsIndex -lt $installIndex) -Message "forensic step must run before Install dependencies"
  }

  It "runs post-checkout forensics before pnpm setup in rust-quality-report" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $jobIndex = $content.IndexOf("rust-quality-report:")
    $checkoutIndex = $content.IndexOf("- uses: actions/checkout@v6", $jobIndex)
    $postCheckoutForensicsIndex = $content.IndexOf("Run pnpm post-checkout forensics (rust-quality-report)", $jobIndex)
    $pnpmSetupIndex = $content.IndexOf("- uses: pnpm/action-setup@v6", $jobIndex)
    $nodeSetupIndex = $content.IndexOf("- uses: actions/setup-node@v6", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must include rust-quality-report job"
    Assert-True -Condition ($checkoutIndex -gt $jobIndex) -Message "rust-quality-report must include checkout step"
    Assert-True -Condition ($postCheckoutForensicsIndex -gt $checkoutIndex) -Message "post-checkout forensic step must run after checkout"
    Assert-True -Condition ($pnpmSetupIndex -gt $postCheckoutForensicsIndex) -Message "post-checkout forensic step must run before pnpm/action-setup"
    Assert-True -Condition ($nodeSetupIndex -gt $postCheckoutForensicsIndex) -Message "post-checkout forensic step must run before actions/setup-node"
  }

  It "pins rust-quality-report setup-node to Node 22 for pnpm/action-setup compatibility" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern "rust-quality-report:[\s\S]*?- uses: actions/setup-node@v6[\s\S]*?node-version:\s*22" -Message "rust-quality-report must pin actions/setup-node to Node 22 to keep pnpm/action-setup on a compatible runtime"
  }

  It "uploads rust post-checkout forensics to a distinct evidence path" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern "Upload pnpm post-checkout forensics \(rust-quality-report\)" -Message "workflow must include rust post-checkout forensic upload"
    Assert-Match -Value $content -Pattern "Upload pnpm post-checkout forensics \(rust-quality-report\)[\s\S]*?if:\s*always\(\)" -Message "rust post-checkout upload must run with if: always()"
    Assert-Match -Value $content -Pattern "Upload pnpm post-checkout forensics \(rust-quality-report\)[\s\S]*?\.ci-evidence/pnpm-preinstall/rust-quality-report-post-checkout/" -Message "rust post-checkout upload must publish distinct post-checkout evidence path"
  }

  It "runs rust post-pnpm-setup forensics between pnpm and setup-node" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $jobIndex = $content.IndexOf("rust-quality-report:")
    $pnpmSetupIndex = $content.IndexOf("- uses: pnpm/action-setup@v6", $jobIndex)
    $postPnpmForensicsIndex = $content.IndexOf("Run pnpm post-pnpm-setup forensics (rust-quality-report)", $jobIndex)
    $nodeSetupIndex = $content.IndexOf("- uses: actions/setup-node@v6", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must include rust-quality-report job"
    Assert-True -Condition ($pnpmSetupIndex -gt $jobIndex) -Message "rust-quality-report must include pnpm/action-setup"
    Assert-True -Condition ($postPnpmForensicsIndex -gt $pnpmSetupIndex) -Message "post-pnpm-setup forensic step must run after pnpm/action-setup"
    Assert-True -Condition ($nodeSetupIndex -gt $postPnpmForensicsIndex) -Message "post-pnpm-setup forensic step must run before actions/setup-node"
  }

  It "uploads rust post-pnpm-setup forensics to a distinct evidence path" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern "Upload pnpm post-pnpm-setup forensics \(rust-quality-report\)" -Message "workflow must include rust post-pnpm-setup forensic upload"
    Assert-Match -Value $content -Pattern "Upload pnpm post-pnpm-setup forensics \(rust-quality-report\)[\s\S]*?if:\s*always\(\)" -Message "rust post-pnpm-setup upload must run with if: always()"
    Assert-Match -Value $content -Pattern "Upload pnpm post-pnpm-setup forensics \(rust-quality-report\)[\s\S]*?\.ci-evidence/pnpm-preinstall/rust-quality-report-post-pnpm-setup/" -Message "rust post-pnpm-setup upload must publish distinct evidence path"
  }

  It "runs rust post-setup-node forensics between setup-node and pre-install" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $jobIndex = $content.IndexOf("rust-quality-report:")
    $nodeSetupIndex = $content.IndexOf("- uses: actions/setup-node@v6", $jobIndex)
    $postNodeForensicsIndex = $content.IndexOf("Run pnpm post-setup-node forensics (rust-quality-report)", $jobIndex)
    $preInstallForensicsIndex = $content.IndexOf("Run pnpm pre-install forensics (rust-quality-report)", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must include rust-quality-report job"
    Assert-True -Condition ($nodeSetupIndex -gt $jobIndex) -Message "rust-quality-report must include actions/setup-node"
    Assert-True -Condition ($postNodeForensicsIndex -gt $nodeSetupIndex) -Message "post-setup-node forensic step must run after actions/setup-node"
    Assert-True -Condition ($preInstallForensicsIndex -gt $postNodeForensicsIndex) -Message "post-setup-node forensic step must run before pre-install forensics"
  }

  It "uploads rust post-setup-node forensics to a distinct evidence path" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern "Upload pnpm post-setup-node forensics \(rust-quality-report\)" -Message "workflow must include rust post-setup-node forensic upload"
    Assert-Match -Value $content -Pattern "Upload pnpm post-setup-node forensics \(rust-quality-report\)[\s\S]*?if:\s*always\(\)" -Message "rust post-setup-node upload must run with if: always()"
    Assert-Match -Value $content -Pattern "Upload pnpm post-setup-node forensics \(rust-quality-report\)[\s\S]*?\.ci-evidence/pnpm-preinstall/rust-quality-report-post-setup-node/" -Message "rust post-setup-node upload must publish distinct evidence path"
  }

  It "restores canonical lockfile between post-setup-node forensics and pre-install forensics in rust-quality-report" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $jobIndex = $content.IndexOf("rust-quality-report:")
    $postNodeForensicsIndex = $content.IndexOf("Run pnpm post-setup-node forensics (rust-quality-report)", $jobIndex)
    $restoreIndex = $content.IndexOf("Restore canonical lockfile (rust-quality-report)", $jobIndex)
    $preInstallForensicsIndex = $content.IndexOf("Run pnpm pre-install forensics (rust-quality-report)", $jobIndex)
    $installIndex = $content.IndexOf("name: Install dependencies", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must include rust-quality-report job"
    Assert-True -Condition ($postNodeForensicsIndex -gt $jobIndex) -Message "rust-quality-report must include post-setup-node forensics"
    Assert-True -Condition ($restoreIndex -gt $postNodeForensicsIndex) -Message "rust-quality-report lockfile restore must run after post-setup-node forensics"
    Assert-True -Condition ($preInstallForensicsIndex -gt $restoreIndex) -Message "rust-quality-report lockfile restore must run before pre-install forensics"
    Assert-True -Condition ($installIndex -gt $preInstallForensicsIndex) -Message "rust-quality-report install must run after pre-install forensics"

    Assert-Match -Value $content -Pattern "Restore canonical lockfile \(rust-quality-report\)[\s\S]*?run:\s*git checkout -- pnpm-lock\.yaml" -Message "rust-quality-report lockfile restore must use deterministic git checkout from committed state"
  }

  It "uploads rust-quality-report pre-install forensics artifact with always policy" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern "Upload pnpm pre-install forensics \(rust-quality-report\)" -Message "workflow must include rust-quality-report forensics upload"
    Assert-Match -Value $content -Pattern "Upload pnpm pre-install forensics \(rust-quality-report\)[\s\S]*?if:\s*always\(\)" -Message "rust-quality-report forensics upload must run with if: always()"
    Assert-Match -Value $content -Pattern "Upload pnpm pre-install forensics \(rust-quality-report\)[\s\S]*?\.ci-evidence/pnpm-preinstall/rust-quality-report/" -Message "rust-quality-report forensics upload must include rust-quality-report evidence folder"
  }

  It "uploads rust-quality-report pre-install forensics after install" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $jobIndex = $content.IndexOf("rust-quality-report:")
    $installIndex = $content.IndexOf("name: Install dependencies", $jobIndex)
    $uploadIndex = $content.IndexOf("Upload pnpm pre-install forensics (rust-quality-report)", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must include rust-quality-report job"
    Assert-True -Condition ($installIndex -gt $jobIndex) -Message "rust-quality-report job must include Install dependencies"
    Assert-True -Condition ($uploadIndex -gt $installIndex) -Message "rust-quality-report pre-install forensics upload must run after install"
  }

  It "uploads baseline coverage artifacts lcov and summary" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern "Upload rust quality artifacts" -Message "workflow must upload rust quality artifacts"
    Assert-Match -Value $content -Pattern "coverage-rust/lcov.info" -Message "workflow artifacts must include lcov.info"
    Assert-Match -Value $content -Pattern "coverage-rust/summary.md" -Message "workflow artifacts must include summary.md"
  }

  It "pins pnpm/action-setup id and uses explicit pnpm bin path in rust-quality-report install" {
    $content = Get-Content -Path $script:workflowPath -Raw

    Assert-Match -Value $content -Pattern 'rust-quality-report:[\s\S]*?- uses: pnpm/action-setup@v6[\s\S]*?id:\s*pnpm_setup' -Message "rust-quality-report pnpm/action-setup must define id: pnpm_setup"
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

  It "runs generic lockfile YAML parse diagnostic before rust install with decisive logs" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $jobIndex = $content.IndexOf("rust-quality-report:")
    $yamlParseIndex = $content.IndexOf("Run generic lockfile YAML parse (rust-quality-report)", $jobIndex)
    $installIndex = $content.IndexOf("name: Install dependencies", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must include rust-quality-report job"
    Assert-True -Condition ($yamlParseIndex -gt $jobIndex) -Message "rust-quality-report must include generic lockfile YAML parse step"
    Assert-True -Condition ($installIndex -gt $yamlParseIndex) -Message "rust-quality-report lockfile YAML parse must run before install"

    Assert-Match -Value $content -Pattern 'Run generic lockfile YAML parse \(rust-quality-report\)[\s\S]*?\$parserCommand\s*=\s*''npm exec --yes --package=js-yaml@4\.1\.0 -- js-yaml pnpm-lock\.yaml''' -Message "rust-quality-report YAML parse step must define portable js-yaml CLI parser via single-quoted npm exec js-yaml@4.1.0 command"
    Assert-Match -Value $content -Pattern 'Run generic lockfile YAML parse \(rust-quality-report\)[\s\S]*?lockfile_yaml_parser=\$parserCommand' -Message "rust-quality-report YAML parse step must emit parser identity using lockfile_yaml_parser"
    Assert-Match -Value $content -Pattern 'Run generic lockfile YAML parse \(rust-quality-report\)[\s\S]*?lockfile_yaml_parse=ok' -Message "rust-quality-report YAML parse step must emit lockfile_yaml_parse=ok on success"
    Assert-Match -Value $content -Pattern 'Run generic lockfile YAML parse \(rust-quality-report\)[\s\S]*?lockfile_yaml_parse=error' -Message "rust-quality-report YAML parse step must emit lockfile_yaml_parse=error on parse failure"
    Assert-Match -Value $content -Pattern 'Run generic lockfile YAML parse \(rust-quality-report\)[\s\S]*?lockfile_yaml_parse_error=\$\(' -Message "rust-quality-report YAML parse step must emit lockfile_yaml_parse_error with the exception message"
  }

  It "blocks the build when rustfmt or clippy report findings" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $gate = [regex]::Match($content, '- name: Rust verify gate \(blocks on rustfmt/clippy\)[\s\S]*?(?=\r?\n      - name:)')
    Assert-True -Condition $gate.Success -Message "rust-quality-report must run the rust verify gate as its own step"
    Assert-Match -Value $gate.Value -Pattern "rust-verify-gate\.ps1" -Message "the gate step must run rust-verify-gate.ps1"
    Assert-True -Condition ($gate.Value -notmatch "continue-on-error\s*:") -Message "the gate step must not continue on error"
  }

  It "lints and tests the Pro local-ml build on Windows" {
    $content = Get-Content -Path $script:workflowPath -Raw

    $job = [regex]::Match($content, 'windows-rust-feature-contract:[\s\S]*?(?=\r?\n  [a-z-]+:\r?\n)')
    Assert-True -Condition $job.Success -Message "workflow must include the windows-rust-feature-contract job"
    Assert-Match -Value $job.Value -Pattern "cargo clippy --manifest-path apps/desktop/src-tauri/Cargo\.toml --features local-ml --all-targets -- -D warnings" -Message "the Pro job must run clippy on local-ml with -D warnings"
    Assert-Match -Value $job.Value -Pattern "cargo test --manifest-path apps/desktop/src-tauri/Cargo\.toml --features local-ml" -Message "the Pro job must run the local-ml test suite"
  }
}
