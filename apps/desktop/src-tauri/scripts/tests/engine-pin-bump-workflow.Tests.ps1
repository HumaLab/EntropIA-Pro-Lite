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

  It "is run by CI" {
    $ci = Read-Workflow -Path $script:ciPath

    Assert-Match -Value $ci -Pattern "engine-pin-bump-workflow\.Tests\.ps1" -Message "CI must run this suite"
  }
}
