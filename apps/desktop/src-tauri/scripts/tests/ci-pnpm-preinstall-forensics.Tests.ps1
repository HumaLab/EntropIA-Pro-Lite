Set-StrictMode -Version Latest

Describe "ci pnpm pre-install forensics workflow contracts" {
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
    $script:workflow = Get-Content -Path $script:workflowPath -Raw
  }

  It "has lint-typecheck pre-install forensics step before install" {
    $jobIndex = $script:workflow.IndexOf("lint-typecheck:")
    $forensicsIndex = $script:workflow.IndexOf("Run pnpm pre-install forensics (lint-typecheck)")
    $installIndex = $script:workflow.IndexOf("name: Install dependencies", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must contain lint-typecheck job"
    Assert-True -Condition ($forensicsIndex -gt $jobIndex) -Message "workflow must include lint-typecheck pre-install forensic step"
    Assert-True -Condition ($installIndex -gt $forensicsIndex) -Message "lint-typecheck forensic step must be before install"
  }

  It "runs lint-typecheck post-checkout forensics before pnpm setup" {
    $jobIndex = $script:workflow.IndexOf("lint-typecheck:")
    $checkoutIndex = $script:workflow.IndexOf("- uses: actions/checkout@v6", $jobIndex)
    $postCheckoutForensicsIndex = $script:workflow.IndexOf("Run pnpm post-checkout forensics (lint-typecheck)", $jobIndex)
    $pnpmSetupIndex = $script:workflow.IndexOf("- uses: pnpm/action-setup@v6", $jobIndex)
    $nodeSetupIndex = $script:workflow.IndexOf("- uses: actions/setup-node@v6", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must contain lint-typecheck job"
    Assert-True -Condition ($checkoutIndex -gt $jobIndex) -Message "lint-typecheck job must include checkout step"
    Assert-True -Condition ($postCheckoutForensicsIndex -gt $checkoutIndex) -Message "lint-typecheck post-checkout forensic step must run after checkout"
    Assert-True -Condition ($pnpmSetupIndex -gt $postCheckoutForensicsIndex) -Message "lint-typecheck post-checkout forensic step must run before pnpm/action-setup"
    Assert-True -Condition ($nodeSetupIndex -gt $postCheckoutForensicsIndex) -Message "lint-typecheck post-checkout forensic step must run before actions/setup-node"
  }

  It "uploads lint-typecheck post-checkout forensics artifact with distinct path" {
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-checkout forensics \(lint-typecheck\)" -Message "workflow must include lint-typecheck post-checkout forensics upload step"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-checkout forensics \(lint-typecheck\)[\s\S]*?if:\s*always\(\)" -Message "lint-typecheck post-checkout upload must run with if: always()"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-checkout forensics \(lint-typecheck\)[\s\S]*?\.ci-evidence/pnpm-preinstall/lint-typecheck-post-checkout/" -Message "lint-typecheck post-checkout upload must publish distinct post-checkout evidence path"
  }

  It "runs lint-typecheck post-pnpm-setup forensics between pnpm and setup-node" {
    $jobIndex = $script:workflow.IndexOf("lint-typecheck:")
    $pnpmSetupIndex = $script:workflow.IndexOf("- uses: pnpm/action-setup@v6", $jobIndex)
    $postPnpmForensicsIndex = $script:workflow.IndexOf("Run pnpm post-pnpm-setup forensics (lint-typecheck)", $jobIndex)
    $nodeSetupIndex = $script:workflow.IndexOf("- uses: actions/setup-node@v6", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must contain lint-typecheck job"
    Assert-True -Condition ($pnpmSetupIndex -gt $jobIndex) -Message "lint-typecheck job must include pnpm/action-setup"
    Assert-True -Condition ($postPnpmForensicsIndex -gt $pnpmSetupIndex) -Message "lint-typecheck post-pnpm-setup forensics must run after pnpm/action-setup"
    Assert-True -Condition ($nodeSetupIndex -gt $postPnpmForensicsIndex) -Message "lint-typecheck post-pnpm-setup forensics must run before actions/setup-node"
  }

  It "uploads lint-typecheck post-pnpm-setup forensics artifact with distinct path" {
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-pnpm-setup forensics \(lint-typecheck\)" -Message "workflow must include lint-typecheck post-pnpm-setup upload step"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-pnpm-setup forensics \(lint-typecheck\)[\s\S]*?if:\s*always\(\)" -Message "lint-typecheck post-pnpm-setup upload must run with if: always()"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-pnpm-setup forensics \(lint-typecheck\)[\s\S]*?\.ci-evidence/pnpm-preinstall/lint-typecheck-post-pnpm-setup/" -Message "lint-typecheck post-pnpm-setup upload must publish distinct evidence path"
  }

  It "runs lint-typecheck post-setup-node forensics between setup-node and pre-install" {
    $jobIndex = $script:workflow.IndexOf("lint-typecheck:")
    $nodeSetupIndex = $script:workflow.IndexOf("- uses: actions/setup-node@v6", $jobIndex)
    $postNodeForensicsIndex = $script:workflow.IndexOf("Run pnpm post-setup-node forensics (lint-typecheck)", $jobIndex)
    $preInstallForensicsIndex = $script:workflow.IndexOf("Run pnpm pre-install forensics (lint-typecheck)", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must contain lint-typecheck job"
    Assert-True -Condition ($nodeSetupIndex -gt $jobIndex) -Message "lint-typecheck job must include actions/setup-node"
    Assert-True -Condition ($postNodeForensicsIndex -gt $nodeSetupIndex) -Message "lint-typecheck post-setup-node forensics must run after actions/setup-node"
    Assert-True -Condition ($preInstallForensicsIndex -gt $postNodeForensicsIndex) -Message "lint-typecheck post-setup-node forensics must run before pre-install forensics"
  }

  It "uploads lint-typecheck post-setup-node forensics artifact with distinct path" {
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-setup-node forensics \(lint-typecheck\)" -Message "workflow must include lint-typecheck post-setup-node upload step"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-setup-node forensics \(lint-typecheck\)[\s\S]*?if:\s*always\(\)" -Message "lint-typecheck post-setup-node upload must run with if: always()"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-setup-node forensics \(lint-typecheck\)[\s\S]*?\.ci-evidence/pnpm-preinstall/lint-typecheck-post-setup-node/" -Message "lint-typecheck post-setup-node upload must publish distinct evidence path"
  }

  It "uploads lint-typecheck forensics artifact with always policy" {
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm pre-install forensics \(lint-typecheck\)" -Message "workflow must include lint-typecheck forensics upload step"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm pre-install forensics \(lint-typecheck\)[\s\S]*?if:\s*always\(\)" -Message "lint-typecheck forensics upload must run with if: always()"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm pre-install forensics \(lint-typecheck\)[\s\S]*?uses:\s*actions/upload-artifact@v7" -Message "lint-typecheck forensics upload must use actions/upload-artifact@v7"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm pre-install forensics \(lint-typecheck\)[\s\S]*?\.ci-evidence/pnpm-preinstall/lint-typecheck/" -Message "lint-typecheck forensics upload must publish lint-typecheck evidence folder"
  }

  It "uploads lint-typecheck pre-install forensics after install" {
    $jobIndex = $script:workflow.IndexOf("lint-typecheck:")
    $installIndex = $script:workflow.IndexOf("name: Install dependencies", $jobIndex)
    $uploadIndex = $script:workflow.IndexOf("Upload pnpm pre-install forensics (lint-typecheck)", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must contain lint-typecheck job"
    Assert-True -Condition ($installIndex -gt $jobIndex) -Message "lint-typecheck job must include Install dependencies"
    Assert-True -Condition ($uploadIndex -gt $installIndex) -Message "lint-typecheck pre-install forensics upload must run after install"
  }

  It "keeps frozen lockfile install command and forbids bypass flags" {
    Assert-Match -Value $script:workflow -Pattern "lint-typecheck:[\s\S]*?name:\s*Install dependencies[\s\S]*?install\s+--frozen-lockfile" -Message "lint-typecheck install must keep frozen lockfile semantics"
    Assert-Match -Value $script:workflow -Pattern "rust-quality-report:[\s\S]*?name:\s*Install dependencies[\s\S]*?install\s+--frozen-lockfile" -Message "rust-quality-report install must keep frozen lockfile semantics"
    Assert-Match -Value $script:workflow -Pattern "test:[\s\S]*?name:\s*Install dependencies[\s\S]*?install\s+--frozen-lockfile" -Message "test install must keep frozen lockfile semantics"
    Assert-Match -Value $script:workflow -Pattern "build:[\s\S]*?name:\s*Install dependencies[\s\S]*?install\s+--frozen-lockfile" -Message "build install must keep frozen lockfile semantics"

    Assert-True -Condition (-not ($script:workflow -match "--no-frozen-lockfile")) -Message "workflow must not include --no-frozen-lockfile"
    Assert-True -Condition (-not ($script:workflow -match "--fix-lockfile")) -Message "workflow must not include --fix-lockfile"
  }

  It "contains mandatory forensic evidence keys in workflow invocation" {
    Assert-Match -Value $script:workflow -Pattern "node_version" -Message "forensics contract must include node_version"
    Assert-Match -Value $script:workflow -Pattern "pnpm_version" -Message "forensics contract must include pnpm_version"
    Assert-Match -Value $script:workflow -Pattern "pnpm_exec_path" -Message "forensics contract must include pnpm_exec_path"
    Assert-Match -Value $script:workflow -Pattern "store_dir" -Message "forensics contract must include store_dir"
    Assert-Match -Value $script:workflow -Pattern "virtual_store_dir" -Message "forensics contract must include virtual_store_dir"
    Assert-Match -Value $script:workflow -Pattern "sha256" -Message "forensics contract must include lockfile sha256"
    Assert-Match -Value $script:workflow -Pattern "size_bytes" -Message "forensics contract must include lockfile size"
    Assert-Match -Value $script:workflow -Pattern "line_count" -Message "forensics contract must include lockfile line count"
    Assert-Match -Value $script:workflow -Pattern "first_bytes_hex" -Message "forensics contract must include lockfile first bytes sample"
    Assert-Match -Value $script:workflow -Pattern "has_bom" -Message "forensics contract must include BOM detection"
    Assert-Match -Value $script:workflow -Pattern "has_nul" -Message "forensics contract must include NUL detection"
    Assert-Match -Value $script:workflow -Pattern "has_yaml_multidoc_separator" -Message "forensics contract must include YAML multi-doc detection"
    Assert-Match -Value $script:workflow -Pattern "git_blob_sha256" -Message "forensics contract must include git blob sha256"
    Assert-Match -Value $script:workflow -Pattern "git_blob_first_bytes_hex" -Message "forensics contract must include git blob first-bytes sample"
    Assert-Match -Value $script:workflow -Pattern "lockfile_comparison_status" -Message "forensics contract must include lockfile comparison status"
  }

  It "pins CI pnpm version to 9.15.4 for lockfile experiment" {
    Assert-Match -Value $script:workflow -Pattern "env:\s*[\s\S]*?PNPM_VERSION:\s*9\.15\.4" -Message "workflow must pin PNPM_VERSION to 9.15.4 for the CI lockfile experiment"
  }

  It "runs optional pre-install classification before install in targeted jobs" {
    $lintForensicsIndex = $script:workflow.IndexOf("Run pnpm pre-install forensics (lint-typecheck)")
    $lintClassificationIndex = $script:workflow.IndexOf("Run pnpm pre-install classification (lint-typecheck)")
    $lintInstallIndex = $script:workflow.IndexOf("name: Install dependencies", $script:workflow.IndexOf("lint-typecheck:"))

    $rustForensicsIndex = $script:workflow.IndexOf("Run pnpm pre-install forensics (rust-quality-report)")
    $rustClassificationIndex = $script:workflow.IndexOf("Run pnpm pre-install classification (rust-quality-report)")
    $rustInstallIndex = $script:workflow.IndexOf("name: Install dependencies", $script:workflow.IndexOf("rust-quality-report:"))

    Assert-True -Condition ($lintForensicsIndex -ge 0) -Message "lint-typecheck forensics step must exist"
    Assert-True -Condition ($lintClassificationIndex -gt $lintForensicsIndex) -Message "lint-typecheck classification step must run after forensics"
    Assert-True -Condition ($lintInstallIndex -gt $lintClassificationIndex) -Message "lint-typecheck classification must run before install"

    Assert-True -Condition ($rustForensicsIndex -ge 0) -Message "rust-quality-report forensics step must exist"
    Assert-True -Condition ($rustClassificationIndex -gt $rustForensicsIndex) -Message "rust-quality-report classification step must run after forensics"
    Assert-True -Condition ($rustInstallIndex -gt $rustClassificationIndex) -Message "rust-quality-report classification must run before install"

    Assert-Match -Value $script:workflow -Pattern "classification\.md" -Message "forensics artifact bundle must include classification.md"
  }

  It "adds pre-install forensics and artifact upload to test and build jobs" {
    $testJobIndex = $script:workflow.IndexOf("test:")
    $testForensicsIndex = $script:workflow.IndexOf("Run pnpm pre-install forensics (test)")
    $testInstallIndex = $script:workflow.IndexOf("name: Install dependencies", $testJobIndex)

    $buildJobIndex = $script:workflow.IndexOf("build:")
    $buildForensicsIndex = $script:workflow.IndexOf("Run pnpm pre-install forensics (build)")
    $buildInstallIndex = $script:workflow.IndexOf("name: Install dependencies", $buildJobIndex)

    Assert-True -Condition ($testJobIndex -ge 0) -Message "workflow must contain test job"
    Assert-True -Condition ($testForensicsIndex -gt $testJobIndex) -Message "workflow must include test pre-install forensic step"
    Assert-True -Condition ($testInstallIndex -gt $testForensicsIndex) -Message "test forensic step must be before install"

    Assert-Match -Value $script:workflow -Pattern "Upload pnpm pre-install forensics \(test\)" -Message "workflow must include test forensics upload step"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm pre-install forensics \(test\)[\s\S]*?if:\s*always\(\)" -Message "test forensics upload must run with if: always()"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm pre-install forensics \(test\)[\s\S]*?\.ci-evidence/pnpm-preinstall/test/" -Message "test forensics upload must publish test evidence folder"

    Assert-True -Condition ($buildJobIndex -ge 0) -Message "workflow must contain build job"
    Assert-True -Condition ($buildForensicsIndex -gt $buildJobIndex) -Message "workflow must include build pre-install forensic step"
    Assert-True -Condition ($buildInstallIndex -gt $buildForensicsIndex) -Message "build forensic step must be before install"

    Assert-Match -Value $script:workflow -Pattern "Upload pnpm pre-install forensics \(build\)" -Message "workflow must include build forensics upload step"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm pre-install forensics \(build\)[\s\S]*?if:\s*always\(\)" -Message "build forensics upload must run with if: always()"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm pre-install forensics \(build\)[\s\S]*?\.ci-evidence/pnpm-preinstall/build/" -Message "build forensics upload must publish build evidence folder"
  }

  It "uses pwsh shell in Linux pre-install forensic/classification steps" {
    Assert-Match -Value $script:workflow -Pattern "Run pnpm pre-install forensics \(lint-typecheck\)[\s\S]*?shell:\s*pwsh" -Message "lint-typecheck forensics step must use pwsh on Linux"
    Assert-Match -Value $script:workflow -Pattern "Run pnpm pre-install classification \(lint-typecheck\)[\s\S]*?shell:\s*pwsh" -Message "lint-typecheck classification step must use pwsh on Linux"
    Assert-Match -Value $script:workflow -Pattern "Run pnpm pre-install forensics \(test\)[\s\S]*?shell:\s*pwsh" -Message "test forensics step must use pwsh on Linux"
    Assert-Match -Value $script:workflow -Pattern "Run pnpm pre-install forensics \(build\)[\s\S]*?shell:\s*pwsh" -Message "build forensics step must use pwsh on Linux"
  }

  It "pins expected Node versions and disables setup-node pnpm caching in install jobs" {
    Assert-Match -Value $script:workflow -Pattern "rust-quality-report:[\s\S]*?- uses: actions/setup-node@v6[\s\S]*?node-version:\s*22[\s\S]*?package-manager-cache:\s*false" -Message "rust-quality-report setup-node must pin Node 22 and explicitly disable package-manager auto-cache"
    Assert-Match -Value $script:workflow -Pattern "lint-typecheck:[\s\S]*?- uses: actions/setup-node@v6[\s\S]*?node-version:\s*22[\s\S]*?package-manager-cache:\s*false" -Message "lint-typecheck setup-node must pin Node 22 and explicitly disable package-manager auto-cache"
    Assert-Match -Value $script:workflow -Pattern "test:[\s\S]*?- uses: actions/setup-node@v6[\s\S]*?node-version:\s*22[\s\S]*?package-manager-cache:\s*false" -Message "test setup-node must pin Node 22 and explicitly disable package-manager auto-cache"
    Assert-Match -Value $script:workflow -Pattern "build:[\s\S]*?- uses: actions/setup-node@v6[\s\S]*?node-version:\s*22[\s\S]*?package-manager-cache:\s*false" -Message "build setup-node must pin Node 22 and explicitly disable package-manager auto-cache"

    Assert-True -Condition (-not ($script:workflow -match "cache:\s*pnpm")) -Message "workflow must not configure setup-node with cache: pnpm"
  }

  It "restores canonical lockfile between post-setup-node and pre-install forensics in lint-typecheck" {
    $jobIndex = $script:workflow.IndexOf("lint-typecheck:")
    $postNodeForensicsIndex = $script:workflow.IndexOf("Run pnpm post-setup-node forensics (lint-typecheck)", $jobIndex)
    $restoreIndex = $script:workflow.IndexOf("Restore canonical lockfile (lint-typecheck)", $jobIndex)
    $preInstallForensicsIndex = $script:workflow.IndexOf("Run pnpm pre-install forensics (lint-typecheck)", $jobIndex)
    $installIndex = $script:workflow.IndexOf("name: Install dependencies", $jobIndex)

    Assert-True -Condition ($jobIndex -ge 0) -Message "workflow must contain lint-typecheck job"
    Assert-True -Condition ($postNodeForensicsIndex -gt $jobIndex) -Message "lint-typecheck must include post-setup-node forensics"
    Assert-True -Condition ($restoreIndex -gt $postNodeForensicsIndex) -Message "lint-typecheck lockfile restore must run after post-setup-node forensics"
    Assert-True -Condition ($preInstallForensicsIndex -gt $restoreIndex) -Message "lint-typecheck lockfile restore must run before pre-install forensics"
    Assert-True -Condition ($installIndex -gt $preInstallForensicsIndex) -Message "lint-typecheck install must run after pre-install forensics"

    Assert-Match -Value $script:workflow -Pattern "Restore canonical lockfile \(lint-typecheck\)[\s\S]*?run:\s*git checkout -- pnpm-lock\.yaml" -Message "lint-typecheck lockfile restore must use deterministic git checkout from committed state"
  }

  It "restores canonical lockfile after setup-node and before pre-install forensics in test and build" {
    $testJobIndex = $script:workflow.IndexOf("test:")
    $testSetupNodeIndex = $script:workflow.IndexOf("- uses: actions/setup-node@v6", $testJobIndex)
    $testRestoreIndex = $script:workflow.IndexOf("Restore canonical lockfile (test)", $testJobIndex)
    $testForensicsIndex = $script:workflow.IndexOf("Run pnpm pre-install forensics (test)", $testJobIndex)
    $testInstallIndex = $script:workflow.IndexOf("name: Install dependencies", $testJobIndex)

    Assert-True -Condition ($testJobIndex -ge 0) -Message "workflow must contain test job"
    Assert-True -Condition ($testSetupNodeIndex -gt $testJobIndex) -Message "test job must include actions/setup-node"
    Assert-True -Condition ($testRestoreIndex -gt $testSetupNodeIndex) -Message "test lockfile restore must run after setup-node"
    Assert-True -Condition ($testForensicsIndex -gt $testRestoreIndex) -Message "test lockfile restore must run before pre-install forensics"
    Assert-True -Condition ($testInstallIndex -gt $testForensicsIndex) -Message "test install must run after pre-install forensics"

    $buildJobIndex = $script:workflow.IndexOf("build:")
    $buildSetupNodeIndex = $script:workflow.IndexOf("- uses: actions/setup-node@v6", $buildJobIndex)
    $buildRestoreIndex = $script:workflow.IndexOf("Restore canonical lockfile (build)", $buildJobIndex)
    $buildForensicsIndex = $script:workflow.IndexOf("Run pnpm pre-install forensics (build)", $buildJobIndex)
    $buildInstallIndex = $script:workflow.IndexOf("name: Install dependencies", $buildJobIndex)

    Assert-True -Condition ($buildJobIndex -ge 0) -Message "workflow must contain build job"
    Assert-True -Condition ($buildSetupNodeIndex -gt $buildJobIndex) -Message "build job must include actions/setup-node"
    Assert-True -Condition ($buildRestoreIndex -gt $buildSetupNodeIndex) -Message "build lockfile restore must run after setup-node"
    Assert-True -Condition ($buildForensicsIndex -gt $buildRestoreIndex) -Message "build lockfile restore must run before pre-install forensics"
    Assert-True -Condition ($buildInstallIndex -gt $buildForensicsIndex) -Message "build install must run after pre-install forensics"

    Assert-Match -Value $script:workflow -Pattern "Restore canonical lockfile \(test\)[\s\S]*?run:\s*git checkout -- pnpm-lock\.yaml" -Message "test lockfile restore must use deterministic git checkout from committed state"
    Assert-Match -Value $script:workflow -Pattern "Restore canonical lockfile \(build\)[\s\S]*?run:\s*git checkout -- pnpm-lock\.yaml" -Message "build lockfile restore must use deterministic git checkout from committed state"
  }

  It "pins pnpm/action-setup id and uses explicit pnpm bin path in install for lint, test and build" {
    Assert-Match -Value $script:workflow -Pattern 'lint-typecheck:[\s\S]*?- uses: pnpm/action-setup@v6[\s\S]*?id:\s*pnpm_setup' -Message "lint-typecheck pnpm/action-setup must define id: pnpm_setup"
    Assert-Match -Value $script:workflow -Pattern 'test:[\s\S]*?- uses: pnpm/action-setup@v6[\s\S]*?id:\s*pnpm_setup' -Message "test pnpm/action-setup must define id: pnpm_setup"
    Assert-Match -Value $script:workflow -Pattern 'build:[\s\S]*?- uses: pnpm/action-setup@v6[\s\S]*?id:\s*pnpm_setup' -Message "build pnpm/action-setup must define id: pnpm_setup"

    Assert-Match -Value $script:workflow -Pattern 'lint-typecheck:[\s\S]*?name:\s*Install dependencies[\s\S]*?steps\.pnpm_setup\.outputs\.bin_dest' -Message "lint-typecheck install must use steps.pnpm_setup.outputs.bin_dest"
    Assert-Match -Value $script:workflow -Pattern 'test:[\s\S]*?name:\s*Install dependencies[\s\S]*?steps\.pnpm_setup\.outputs\.bin_dest' -Message "test install must use steps.pnpm_setup.outputs.bin_dest"
    Assert-Match -Value $script:workflow -Pattern 'build:[\s\S]*?name:\s*Install dependencies[\s\S]*?steps\.pnpm_setup\.outputs\.bin_dest' -Message "build install must use steps.pnpm_setup.outputs.bin_dest"

    Assert-Match -Value $script:workflow -Pattern 'lint-typecheck:[\s\S]*?name:\s*Install dependencies[\s\S]*?Write-Host\s+"pnpm_bin=' -Message "lint-typecheck install must print pnpm binary path"
    Assert-Match -Value $script:workflow -Pattern 'test:[\s\S]*?name:\s*Install dependencies[\s\S]*?Write-Host\s+"pnpm_bin=' -Message "test install must print pnpm binary path"
    Assert-Match -Value $script:workflow -Pattern 'build:[\s\S]*?name:\s*Install dependencies[\s\S]*?Write-Host\s+"pnpm_bin=' -Message "build install must print pnpm binary path"

    Assert-Match -Value $script:workflow -Pattern 'lint-typecheck:[\s\S]*?name:\s*Install dependencies[\s\S]*?&\s*\$pnpmExe\s+-v' -Message "lint-typecheck install must print pnpm version from pinned binary"
    Assert-Match -Value $script:workflow -Pattern 'test:[\s\S]*?name:\s*Install dependencies[\s\S]*?&\s*\$pnpmExe\s+-v' -Message "test install must print pnpm version from pinned binary"
    Assert-Match -Value $script:workflow -Pattern 'build:[\s\S]*?name:\s*Install dependencies[\s\S]*?&\s*\$pnpmExe\s+-v' -Message "build install must print pnpm version from pinned binary"

    Assert-Match -Value $script:workflow -Pattern 'lint-typecheck:[\s\S]*?name:\s*Install dependencies[\s\S]*?node\s+-v' -Message "lint-typecheck install must print node version"
    Assert-Match -Value $script:workflow -Pattern 'test:[\s\S]*?name:\s*Install dependencies[\s\S]*?node\s+-v' -Message "test install must print node version"
    Assert-Match -Value $script:workflow -Pattern 'build:[\s\S]*?name:\s*Install dependencies[\s\S]*?node\s+-v' -Message "build install must print node version"

    Assert-Match -Value $script:workflow -Pattern 'lint-typecheck:[\s\S]*?name:\s*Install dependencies[\s\S]*?npm\s+exec\s+--package=pnpm@9\.15\.4\s+--\s+pnpm\s+install\s+--frozen-lockfile' -Message "lint-typecheck install must execute frozen lockfile install through npm exec pnpm@9.15.4"
    Assert-Match -Value $script:workflow -Pattern 'test:[\s\S]*?name:\s*Install dependencies[\s\S]*?npm\s+exec\s+--package=pnpm@9\.15\.4\s+--\s+pnpm\s+install\s+--frozen-lockfile' -Message "test install must execute frozen lockfile install through npm exec pnpm@9.15.4"
    Assert-Match -Value $script:workflow -Pattern 'build:[\s\S]*?name:\s*Install dependencies[\s\S]*?npm\s+exec\s+--package=pnpm@9\.15\.4\s+--\s+pnpm\s+install\s+--frozen-lockfile' -Message "build install must execute frozen lockfile install through npm exec pnpm@9.15.4"
  }

  It "restores lockfile inside lint-typecheck install step before pnpm install" {
    Assert-Match -Value $script:workflow -Pattern 'lint-typecheck:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?npm\s+exec\s+--package=pnpm@9\.15\.4\s+--\s+pnpm\s+install\s+--frozen-lockfile' -Message "lint-typecheck install step must restore canonical lockfile immediately before npm exec pnpm install"
  }

  It "emits same-step lockfile diagnostics inside lint-typecheck install" {
    Assert-Match -Value $script:workflow -Pattern 'lint-typecheck:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?lockfile_diag\.sha256=' -Message "lint-typecheck install must emit same-step lockfile sha256 diagnostic"
    Assert-Match -Value $script:workflow -Pattern 'lint-typecheck:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?lockfile_diag\.first_line_is_yaml_doc=' -Message "lint-typecheck install must emit same-step YAML doc first-line diagnostic"
    Assert-Match -Value $script:workflow -Pattern 'lint-typecheck:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?lockfile_diag\.head_blob=' -Message "lint-typecheck install must emit same-step HEAD lockfile blob diagnostic"
    Assert-Match -Value $script:workflow -Pattern 'lint-typecheck:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?lockfile_diag\.matches_head_blob=[\s\S]*?npm\s+exec\s+--package=pnpm@9\.15\.4\s+--\s+pnpm\s+install\s+--frozen-lockfile' -Message "lint-typecheck install must compare working lockfile to HEAD blob before npm exec pnpm install"
  }

  It "enforces test/build install-step lockfile parity with lint-typecheck" {
    Assert-Match -Value $script:workflow -Pattern 'test:[\s\S]*?name:\s*Install dependencies[\s\S]*?\$pnpmBinDir\s*=\s*"\$\{\{\s*steps\.pnpm_setup\.outputs\.bin_dest\s*\}\}"[\s\S]*?\$pnpmExe\s*=\s*Join-Path\s+\$pnpmBinDir\s+"pnpm"' -Message "test install must keep the same pnpm_setup/bin_dest -> Join-Path pnpm shape as lint-typecheck"
    Assert-Match -Value $script:workflow -Pattern 'build:[\s\S]*?name:\s*Install dependencies[\s\S]*?\$pnpmBinDir\s*=\s*"\$\{\{\s*steps\.pnpm_setup\.outputs\.bin_dest\s*\}\}"[\s\S]*?\$pnpmExe\s*=\s*Join-Path\s+\$pnpmBinDir\s+"pnpm"' -Message "build install must keep the same pnpm_setup/bin_dest -> Join-Path pnpm shape as lint-typecheck"

    Assert-Match -Value $script:workflow -Pattern 'test:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?lockfile_diag\.sha256=[\s\S]*?lockfile_diag\.first_line_is_yaml_doc=[\s\S]*?lockfile_diag\.working_blob=[\s\S]*?lockfile_diag\.head_blob=[\s\S]*?lockfile_diag\.matches_head_blob=[\s\S]*?npm\s+exec\s+--package=pnpm@9\.15\.4\s+--\s+pnpm\s+install\s+--frozen-lockfile' -Message "test install must restore lockfile in-step and emit full same-step lockfile diagnostics before npm exec pnpm install"
    Assert-Match -Value $script:workflow -Pattern 'build:[\s\S]*?name:\s*Install dependencies[\s\S]*?git checkout -- pnpm-lock\.yaml[\s\S]*?lockfile_diag\.sha256=[\s\S]*?lockfile_diag\.first_line_is_yaml_doc=[\s\S]*?lockfile_diag\.working_blob=[\s\S]*?lockfile_diag\.head_blob=[\s\S]*?lockfile_diag\.matches_head_blob=[\s\S]*?npm\s+exec\s+--package=pnpm@9\.15\.4\s+--\s+pnpm\s+install\s+--frozen-lockfile' -Message "build install must restore lockfile in-step and emit full same-step lockfile diagnostics before npm exec pnpm install"
  }

  It "runs generic lockfile YAML parse diagnostic before install in targeted jobs" {
    $lintJobIndex = $script:workflow.IndexOf("lint-typecheck:")
    $lintYamlParseIndex = $script:workflow.IndexOf("Run generic lockfile YAML parse (lint-typecheck)", $lintJobIndex)
    $lintInstallIndex = $script:workflow.IndexOf("name: Install dependencies", $lintJobIndex)

    $rustJobIndex = $script:workflow.IndexOf("rust-quality-report:")
    $rustYamlParseIndex = $script:workflow.IndexOf("Run generic lockfile YAML parse (rust-quality-report)", $rustJobIndex)
    $rustInstallIndex = $script:workflow.IndexOf("name: Install dependencies", $rustJobIndex)

    Assert-True -Condition ($lintJobIndex -ge 0) -Message "workflow must contain lint-typecheck job"
    Assert-True -Condition ($lintYamlParseIndex -gt $lintJobIndex) -Message "lint-typecheck must include generic lockfile YAML parse step"
    Assert-True -Condition ($lintInstallIndex -gt $lintYamlParseIndex) -Message "lint-typecheck lockfile YAML parse must run before install"

    Assert-True -Condition ($rustJobIndex -ge 0) -Message "workflow must contain rust-quality-report job"
    Assert-True -Condition ($rustYamlParseIndex -gt $rustJobIndex) -Message "rust-quality-report must include generic lockfile YAML parse step"
    Assert-True -Condition ($rustInstallIndex -gt $rustYamlParseIndex) -Message "rust-quality-report lockfile YAML parse must run before install"

    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(lint-typecheck\)[\s\S]*?\$parserCommand\s*=\s*''npm exec --yes --package=js-yaml@4\.1\.0 -- js-yaml pnpm-lock\.yaml''' -Message "lint-typecheck YAML parse step must define parser command with PowerShell-safe single-quoted npm exec js-yaml CLI invocation"
    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(rust-quality-report\)[\s\S]*?\$parserCommand\s*=\s*''npm exec --yes --package=js-yaml@4\.1\.0 -- js-yaml pnpm-lock\.yaml''' -Message "rust-quality-report YAML parse step must define parser command with PowerShell-safe single-quoted npm exec js-yaml CLI invocation"
    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(lint-typecheck\)[\s\S]*?lockfile_yaml_parser=\$parserCommand' -Message "lint-typecheck YAML parse step must emit parser identity using lockfile_yaml_parser"
    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(rust-quality-report\)[\s\S]*?lockfile_yaml_parser=\$parserCommand' -Message "rust-quality-report YAML parse step must emit parser identity using lockfile_yaml_parser"
    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(lint-typecheck\)[\s\S]*?lockfile_yaml_parse=ok' -Message "lint-typecheck YAML parse step must emit lockfile_yaml_parse=ok on success"
    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(lint-typecheck\)[\s\S]*?lockfile_yaml_parse=error' -Message "lint-typecheck YAML parse step must emit lockfile_yaml_parse=error on parse failure"
    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(lint-typecheck\)[\s\S]*?lockfile_yaml_parse_error=\$\(' -Message "lint-typecheck YAML parse step must emit lockfile_yaml_parse_error with the exception message"
    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(rust-quality-report\)[\s\S]*?lockfile_yaml_parse=ok' -Message "rust-quality-report YAML parse step must emit lockfile_yaml_parse=ok on success"
    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(rust-quality-report\)[\s\S]*?lockfile_yaml_parse=error' -Message "rust-quality-report YAML parse step must emit lockfile_yaml_parse=error on parse failure"
    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(rust-quality-report\)[\s\S]*?lockfile_yaml_parse_error=\$\(' -Message "rust-quality-report YAML parse step must emit lockfile_yaml_parse_error with the exception message"
  }

  It "aligns test and build pre-install sequence with post-checkout/post-pnpm/post-node forensics, classification, and YAML parse" {
    $testJobIndex = $script:workflow.IndexOf("test:")
    $testCheckoutIndex = $script:workflow.IndexOf("- uses: actions/checkout@v6", $testJobIndex)
    $testPostCheckoutForensicsIndex = $script:workflow.IndexOf("Run pnpm post-checkout forensics (test)", $testJobIndex)
    $testPostCheckoutUploadIndex = $script:workflow.IndexOf("Upload pnpm post-checkout forensics (test)", $testJobIndex)
    $testPnpmSetupIndex = $script:workflow.IndexOf("- uses: pnpm/action-setup@v6", $testJobIndex)
    $testPostPnpmForensicsIndex = $script:workflow.IndexOf("Run pnpm post-pnpm-setup forensics (test)", $testJobIndex)
    $testPostPnpmUploadIndex = $script:workflow.IndexOf("Upload pnpm post-pnpm-setup forensics (test)", $testJobIndex)
    $testSetupNodeIndex = $script:workflow.IndexOf("- uses: actions/setup-node@v6", $testJobIndex)
    $testPostNodeForensicsIndex = $script:workflow.IndexOf("Run pnpm post-setup-node forensics (test)", $testJobIndex)
    $testPostNodeUploadIndex = $script:workflow.IndexOf("Upload pnpm post-setup-node forensics (test)", $testJobIndex)
    $testRestoreIndex = $script:workflow.IndexOf("Restore canonical lockfile (test)", $testJobIndex)
    $testPreInstallForensicsIndex = $script:workflow.IndexOf("Run pnpm pre-install forensics (test)", $testJobIndex)
    $testClassificationIndex = $script:workflow.IndexOf("Run pnpm pre-install classification (test)", $testJobIndex)
    $testYamlParseIndex = $script:workflow.IndexOf("Run generic lockfile YAML parse (test)", $testJobIndex)
    $testInstallIndex = $script:workflow.IndexOf("name: Install dependencies", $testJobIndex)

    Assert-True -Condition ($testJobIndex -ge 0) -Message "workflow must contain test job"
    Assert-True -Condition ($testCheckoutIndex -gt $testJobIndex) -Message "test job must include checkout"
    Assert-True -Condition ($testPostCheckoutForensicsIndex -gt $testCheckoutIndex) -Message "test post-checkout forensics must run after checkout"
    Assert-True -Condition ($testPostCheckoutUploadIndex -gt $testPostCheckoutForensicsIndex) -Message "test post-checkout upload must run after post-checkout forensics"
    Assert-True -Condition ($testPnpmSetupIndex -gt $testPostCheckoutUploadIndex) -Message "test pnpm setup must run after post-checkout forensics and upload"
    Assert-True -Condition ($testPostPnpmForensicsIndex -gt $testPnpmSetupIndex) -Message "test post-pnpm-setup forensics must run after pnpm setup"
    Assert-True -Condition ($testPostPnpmUploadIndex -gt $testPostPnpmForensicsIndex) -Message "test post-pnpm upload must run after post-pnpm forensics"
    Assert-True -Condition ($testSetupNodeIndex -gt $testPostPnpmUploadIndex) -Message "test setup-node must run after post-pnpm forensics and upload"
    Assert-True -Condition ($testPostNodeForensicsIndex -gt $testSetupNodeIndex) -Message "test post-setup-node forensics must run after setup-node"
    Assert-True -Condition ($testPostNodeUploadIndex -gt $testPostNodeForensicsIndex) -Message "test post-setup-node upload must run after post-setup-node forensics"
    Assert-True -Condition ($testRestoreIndex -gt $testPostNodeUploadIndex) -Message "test lockfile restore must run after post-setup-node forensics/upload"
    Assert-True -Condition ($testPreInstallForensicsIndex -gt $testRestoreIndex) -Message "test pre-install forensics must run after lockfile restore"
    Assert-True -Condition ($testClassificationIndex -gt $testPreInstallForensicsIndex) -Message "test classification must run after pre-install forensics"
    Assert-True -Condition ($testYamlParseIndex -gt $testClassificationIndex) -Message "test generic YAML parse must run after classification"
    Assert-True -Condition ($testInstallIndex -gt $testYamlParseIndex) -Message "test install must run after YAML parse"

    $buildJobIndex = $script:workflow.IndexOf("build:")
    $buildCheckoutIndex = $script:workflow.IndexOf("- uses: actions/checkout@v6", $buildJobIndex)
    $buildPostCheckoutForensicsIndex = $script:workflow.IndexOf("Run pnpm post-checkout forensics (build)", $buildJobIndex)
    $buildPostCheckoutUploadIndex = $script:workflow.IndexOf("Upload pnpm post-checkout forensics (build)", $buildJobIndex)
    $buildPnpmSetupIndex = $script:workflow.IndexOf("- uses: pnpm/action-setup@v6", $buildJobIndex)
    $buildPostPnpmForensicsIndex = $script:workflow.IndexOf("Run pnpm post-pnpm-setup forensics (build)", $buildJobIndex)
    $buildPostPnpmUploadIndex = $script:workflow.IndexOf("Upload pnpm post-pnpm-setup forensics (build)", $buildJobIndex)
    $buildSetupNodeIndex = $script:workflow.IndexOf("- uses: actions/setup-node@v6", $buildJobIndex)
    $buildPostNodeForensicsIndex = $script:workflow.IndexOf("Run pnpm post-setup-node forensics (build)", $buildJobIndex)
    $buildPostNodeUploadIndex = $script:workflow.IndexOf("Upload pnpm post-setup-node forensics (build)", $buildJobIndex)
    $buildRestoreIndex = $script:workflow.IndexOf("Restore canonical lockfile (build)", $buildJobIndex)
    $buildPreInstallForensicsIndex = $script:workflow.IndexOf("Run pnpm pre-install forensics (build)", $buildJobIndex)
    $buildClassificationIndex = $script:workflow.IndexOf("Run pnpm pre-install classification (build)", $buildJobIndex)
    $buildYamlParseIndex = $script:workflow.IndexOf("Run generic lockfile YAML parse (build)", $buildJobIndex)
    $buildInstallIndex = $script:workflow.IndexOf("name: Install dependencies", $buildJobIndex)

    Assert-True -Condition ($buildJobIndex -ge 0) -Message "workflow must contain build job"
    Assert-True -Condition ($buildCheckoutIndex -gt $buildJobIndex) -Message "build job must include checkout"
    Assert-True -Condition ($buildPostCheckoutForensicsIndex -gt $buildCheckoutIndex) -Message "build post-checkout forensics must run after checkout"
    Assert-True -Condition ($buildPostCheckoutUploadIndex -gt $buildPostCheckoutForensicsIndex) -Message "build post-checkout upload must run after post-checkout forensics"
    Assert-True -Condition ($buildPnpmSetupIndex -gt $buildPostCheckoutUploadIndex) -Message "build pnpm setup must run after post-checkout forensics and upload"
    Assert-True -Condition ($buildPostPnpmForensicsIndex -gt $buildPnpmSetupIndex) -Message "build post-pnpm-setup forensics must run after pnpm setup"
    Assert-True -Condition ($buildPostPnpmUploadIndex -gt $buildPostPnpmForensicsIndex) -Message "build post-pnpm upload must run after post-pnpm forensics"
    Assert-True -Condition ($buildSetupNodeIndex -gt $buildPostPnpmUploadIndex) -Message "build setup-node must run after post-pnpm forensics and upload"
    Assert-True -Condition ($buildPostNodeForensicsIndex -gt $buildSetupNodeIndex) -Message "build post-setup-node forensics must run after setup-node"
    Assert-True -Condition ($buildPostNodeUploadIndex -gt $buildPostNodeForensicsIndex) -Message "build post-setup-node upload must run after post-setup-node forensics"
    Assert-True -Condition ($buildRestoreIndex -gt $buildPostNodeUploadIndex) -Message "build lockfile restore must run after post-setup-node forensics/upload"
    Assert-True -Condition ($buildPreInstallForensicsIndex -gt $buildRestoreIndex) -Message "build pre-install forensics must run after lockfile restore"
    Assert-True -Condition ($buildClassificationIndex -gt $buildPreInstallForensicsIndex) -Message "build classification must run after pre-install forensics"
    Assert-True -Condition ($buildYamlParseIndex -gt $buildClassificationIndex) -Message "build generic YAML parse must run after classification"
    Assert-True -Condition ($buildInstallIndex -gt $buildYamlParseIndex) -Message "build install must run after YAML parse"

    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-checkout forensics \(test\)[\s\S]*?if:\s*always\(\)" -Message "test post-checkout upload must run with if: always()"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-pnpm-setup forensics \(test\)[\s\S]*?if:\s*always\(\)" -Message "test post-pnpm upload must run with if: always()"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-setup-node forensics \(test\)[\s\S]*?if:\s*always\(\)" -Message "test post-setup-node upload must run with if: always()"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-checkout forensics \(build\)[\s\S]*?if:\s*always\(\)" -Message "build post-checkout upload must run with if: always()"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-pnpm-setup forensics \(build\)[\s\S]*?if:\s*always\(\)" -Message "build post-pnpm upload must run with if: always()"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-setup-node forensics \(build\)[\s\S]*?if:\s*always\(\)" -Message "build post-setup-node upload must run with if: always()"

    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-checkout forensics \(test\)[\s\S]*?\.ci-evidence/pnpm-preinstall/test-post-checkout/" -Message "test post-checkout upload must publish distinct post-checkout evidence path"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-pnpm-setup forensics \(test\)[\s\S]*?\.ci-evidence/pnpm-preinstall/test-post-pnpm-setup/" -Message "test post-pnpm upload must publish distinct post-pnpm evidence path"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-setup-node forensics \(test\)[\s\S]*?\.ci-evidence/pnpm-preinstall/test-post-setup-node/" -Message "test post-setup-node upload must publish distinct post-setup-node evidence path"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-checkout forensics \(build\)[\s\S]*?\.ci-evidence/pnpm-preinstall/build-post-checkout/" -Message "build post-checkout upload must publish distinct post-checkout evidence path"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-pnpm-setup forensics \(build\)[\s\S]*?\.ci-evidence/pnpm-preinstall/build-post-pnpm-setup/" -Message "build post-pnpm upload must publish distinct post-pnpm evidence path"
    Assert-Match -Value $script:workflow -Pattern "Upload pnpm post-setup-node forensics \(build\)[\s\S]*?\.ci-evidence/pnpm-preinstall/build-post-setup-node/" -Message "build post-setup-node upload must publish distinct post-setup-node evidence path"

    Assert-Match -Value $script:workflow -Pattern 'Run pnpm pre-install classification \(test\)[\s\S]*?\.ci-evidence/pnpm-preinstall/test/preinstall-evidence\.json' -Message "test classification must consume test preinstall evidence"
    Assert-Match -Value $script:workflow -Pattern 'Run pnpm pre-install classification \(build\)[\s\S]*?\.ci-evidence/pnpm-preinstall/build/preinstall-evidence\.json' -Message "build classification must consume build preinstall evidence"
    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(test\)[\s\S]*?\$parserCommand\s*=\s*''npm exec --yes --package=js-yaml@4\.1\.0 -- js-yaml pnpm-lock\.yaml''' -Message "test YAML parse step must define js-yaml parser command"
    Assert-Match -Value $script:workflow -Pattern 'Run generic lockfile YAML parse \(build\)[\s\S]*?\$parserCommand\s*=\s*''npm exec --yes --package=js-yaml@4\.1\.0 -- js-yaml pnpm-lock\.yaml''' -Message "build YAML parse step must define js-yaml parser command"
  }
}
