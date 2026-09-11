Set-StrictMode -Version Latest

Describe "engine-pin-bump workflow" {
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

    function Read-Workflow {
      param([string]$Path)

      if (-not (Test-Path -Path $Path)) {
        throw "workflow not found: $Path"
      }
      return Get-Content -Path $Path -Raw
    }

    # The top-level `on:` block: every indented or blank line after it.
    function Get-Triggers {
      param([string]$Content)

      return [regex]::Match($Content, '(?m)^on:\r?\n(?:(?:[ \t]+.*|[ \t]*)\r?\n)+').Value
    }

    $script:TestRoot = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
    $script:RepoRoot = (Resolve-Path -Path (Join-Path $script:TestRoot "../../../../..")).Path
    $script:bumpPath = Join-Path -Path $script:RepoRoot -ChildPath ".github/workflows/engine-pin-bump.yml"
    $script:ciPath = Join-Path -Path $script:RepoRoot -ChildPath ".github/workflows/ci.yml"
  }

  It "runs every day and on demand" {
    $triggers = Get-Triggers -Content (Read-Workflow -Path $script:bumpPath)

    Assert-Match -Value $triggers -Pattern "schedule:\s*\r?\n\s*- cron:" -Message "the bump must run on a schedule"
    Assert-Match -Value $triggers -Pattern "workflow_dispatch:" -Message "the bump must be runnable on demand"
  }

  It "moves only the engine, not every dependency" {
    $content = Read-Workflow -Path $script:bumpPath

    Assert-Match -Value $content -Pattern "cargo update -p entropia-agent --manifest-path apps/desktop/src-tauri/Cargo\.toml" -Message "the bump must update entropia-agent alone"
  }

  It "dispatches CI, because its own pushes never trigger workflows" {
    $content = Read-Workflow -Path $script:bumpPath
    $ciTriggers = Get-Triggers -Content (Read-Workflow -Path $script:ciPath)

    Assert-Match -Value $ciTriggers -Pattern "workflow_dispatch:" -Message "ci.yml must accept workflow_dispatch so the bump can run it"
    Assert-Match -Value $content -Pattern "gh workflow run ci\.yml --ref" -Message "the bump must dispatch CI on its branch"
  }

  It "advances main only after CI passes on the bumped commit" {
    $content = Read-Workflow -Path $script:bumpPath

    $watch = [regex]::Match($content, "gh run watch [^\r\n]*")
    $mainPush = [regex]::Match($content, "git push [^\r\n]*refs/heads/main[^\r\n]*")
    Assert-True -Condition $watch.Success -Message "the bump must wait for the CI run"
    Assert-True -Condition $mainPush.Success -Message "the bump must push the tested commit to main"
    Assert-Match -Value $content -Pattern 'select\(\.headSha == ' -Message "the watched CI run must be the one for the bumped commit"
    Assert-Match -Value $watch.Value -Pattern "--exit-status" -Message "a failed CI run must stop the bump"
    Assert-True -Condition ($watch.Index -lt $mainPush.Index) -Message "main must move only after CI finished"
    Assert-True -Condition ($mainPush.Value -notmatch '--force|\s\+|"\+') -Message "main must only fast-forward, never be forced"
  }

  Context "deciding whether the engine moved" {
    BeforeAll {
      $gitBash = Join-Path -Path (Split-Path -Parent (Split-Path -Parent (Get-Command git).Source)) -ChildPath "bin/bash.exe"
      $script:bash = if (Test-Path -Path $gitBash) { $gitBash } else { "bash" }

      $step = [regex]::Match((Read-Workflow -Path $script:bumpPath), '- name: Update entropia-agent in Cargo\.lock[\s\S]*?run: \|\r?\n((?:(?: {10}[^\r\n]*)?\r?\n)+)')
      $script:updateScript = $step.Groups[1].Value -replace '(?m)^ {10}', ''

      $script:pinA = "1111111111111111111111111111111111111111"
      $script:pinB = "2222222222222222222222222222222222222222"

      function New-Lock {
        param([string]$Pin, [string]$WindowsSys)

        $source = if ($Pin) { "source = ""git+https://github.com/HumaLab/EntropIA-Agent?branch=main#$Pin""`n" } else { "" }
        return "[[package]]`nname = ""entropia-agent""`nversion = ""0.1.0""`n$source" + "dependencies = [`n ""$WindowsSys"",`n]`n"
      }

      # Runs the workflow's own update step in a scratch repo, with a `cargo`
      # whose update rewrites Cargo.lock to $After.
      function Invoke-UpdateStep {
        param([string]$Before, [string]$After)

        $root = Join-Path -Path ([IO.Path]::GetTempPath()) -ChildPath ("pin-bump-" + [guid]::NewGuid().ToString("N"))
        $repo = Join-Path -Path $root -ChildPath "repo"
        $lock = Join-Path -Path $repo -ChildPath "apps/desktop/src-tauri/Cargo.lock"
        New-Item -ItemType Directory -Path (Split-Path -Parent $lock) -Force | Out-Null
        [IO.File]::WriteAllText($lock, $Before)
        $afterPath = Join-Path -Path $root -ChildPath "after.lock"
        [IO.File]::WriteAllText($afterPath, $After)
        $stepPath = Join-Path -Path $root -ChildPath "step.sh"
        [IO.File]::WriteAllText($stepPath, $script:updateScript.Replace("`r`n", "`n"))
        $fakePath = Join-Path -Path $root -ChildPath "fake-cargo.sh"
        [IO.File]::WriteAllText($fakePath, "cargo() { cp ""`$FAKE_AFTER"" ""`$LOCK_PATH""; }`n")
        $outputPath = Join-Path -Path $root -ChildPath "github-output"
        [IO.File]::WriteAllText($outputPath, "")

        $variables = @{
          LOCK_PATH     = "apps/desktop/src-tauri/Cargo.lock"
          GITHUB_OUTPUT = $outputPath.Replace('\', '/')
          BASH_ENV      = $fakePath.Replace('\', '/')
          FAKE_AFTER    = $afterPath.Replace('\', '/')
        }
        $saved = @{}
        Push-Location -Path $repo
        try {
          foreach ($name in $variables.Keys) {
            $saved[$name] = [Environment]::GetEnvironmentVariable($name)
            [Environment]::SetEnvironmentVariable($name, $variables[$name])
          }
          git init -q
          git config core.autocrlf false
          git config user.name "test"
          git config user.email "test@example.com"
          git add .
          git commit -q -m "fixture"
          # The runner's `shell: bash` runs steps with these flags.
          $log = & $script:bash --noprofile --norc -eo pipefail $stepPath.Replace('\', '/') 2>&1 | Out-String
          $exitCode = $LASTEXITCODE
        } finally {
          foreach ($name in $variables.Keys) {
            [Environment]::SetEnvironmentVariable($name, $saved[$name])
          }
          Pop-Location
        }

        $result = [pscustomobject]@{
          ExitCode = $exitCode
          Output   = [IO.File]::ReadAllText($outputPath)
          Lock     = [IO.File]::ReadAllText($lock)
          Log      = $log
        }
        Remove-Item -Path $root -Recurse -Force
        return $result
      }
    }

    It "treats a rewrite that keeps the pin as no bump" {
      # cargo re-wires ranged windows-sys edges on every update, engine or not.
      $before = New-Lock -Pin $script:pinA -WindowsSys "windows-sys 0.61.2"
      $result = Invoke-UpdateStep -Before $before -After (New-Lock -Pin $script:pinA -WindowsSys "windows-sys 0.59.0")

      Assert-True -Condition ($result.ExitCode -eq 0) -Message "the step must succeed: $($result.Log)"
      Assert-Match -Value $result.Output -Pattern "(?m)^changed=false$" -Message "an unmoved pin is not a bump"
      Assert-True -Condition ($result.Lock -ceq $before) -Message "the lock churn must be discarded"
    }

    It "reports a moved pin as a bump" {
      $after = New-Lock -Pin $script:pinB -WindowsSys "windows-sys 0.59.0"
      $result = Invoke-UpdateStep -Before (New-Lock -Pin $script:pinA -WindowsSys "windows-sys 0.61.2") -After $after

      Assert-True -Condition ($result.ExitCode -eq 0) -Message "the step must succeed: $($result.Log)"
      Assert-Match -Value $result.Output -Pattern "(?m)^changed=true$" -Message "a moved pin is a bump"
      Assert-Match -Value $result.Output -Pattern "(?m)^pin=2222222222222222222222222222222222222222$" -Message "the step must report the new pin"
      Assert-True -Condition ($result.Lock -ceq $after) -Message "the updated lock must be kept"
    }

    It "fails when the update loses the pin" {
      $result = Invoke-UpdateStep -Before (New-Lock -Pin $script:pinA -WindowsSys "windows-sys 0.61.2") -After (New-Lock -Pin "" -WindowsSys "windows-sys 0.61.2")

      Assert-True -Condition ($result.ExitCode -ne 0) -Message "a lock without the GitHub pin must stop the bump"
    }
  }

  It "is run by CI" {
    $ci = Read-Workflow -Path $script:ciPath

    Assert-Match -Value $ci -Pattern "engine-pin-bump-workflow\.Tests\.ps1" -Message "CI must run this suite"
  }
}
