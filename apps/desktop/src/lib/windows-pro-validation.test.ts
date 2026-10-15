import { existsSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

// Isolated Windows Pro artifact-only CI (.github/workflows/windows-pro-validation.yml).
// The workflow file does not exist yet (RED stage): read it as empty text so every
// positive assertion fails as a behavioral mismatch, never as an ENOENT harness error.
// String/substring convention mirrors src/lib/runtime-packaging.test.ts; no new deps.
// Bounded-section helpers below keep assertions structural (trigger/permissions/build
// sections) without adding a YAML parser dependency.
const currentDir = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(currentDir, '../../../..')
const workflowRelPath = '.github/workflows/windows-pro-validation.yml'
const workflowPath = resolve(repoRoot, workflowRelPath)

// Established Pro pipeline literals reused from .github/workflows/release.yml.
const BOOTSTRAP_MANIFEST_URL =
  'https://github.com/HumaLab/EntropIA-Pro/releases/download/runtime-bootstrap/manifest.json'
const BOOTSTRAP_PUBLIC_KEY_BASE64 = 'fSt35Xo0XtpdZUH/Q2gsPm1drMZczLMQNcU6fqAEbi0='
const BOOTSTRAP_PUBLIC_KEY_ID = 'entropia-runtime-2026'

function readWorkflowOrEmpty(): string {
  if (!existsSync(workflowPath)) return ''
  return readFileSync(workflowPath, 'utf8')
}

// Slice the workflow from `startMarker` up to the first occurrence of any stop
// marker, so trigger/permissions assertions cannot match comments or job steps.
function sectionAfter(text: string, startMarker: string, stopMarkers: string[]): string {
  const start = text.indexOf(startMarker)
  if (start === -1) return ''
  const rest = text.slice(start + startMarker.length)
  let end = rest.length
  for (const stop of stopMarkers) {
    const i = rest.indexOf(stop)
    if (i !== -1 && i < end) end = i
  }
  return rest.slice(0, end)
}

function onBlock(workflow: string): string {
  return sectionAfter(workflow, 'on:', ['\npermissions:', '\nenv:', '\njobs:'])
}

function permissionsBlock(workflow: string): string {
  return sectionAfter(workflow, 'permissions:', ['\nenv:', '\njobs:'])
}

function buildStepBlock(workflow: string): string {
  const marker = 'Build Pro installers'
  const i = workflow.indexOf(marker)
  if (i === -1) return ''
  return workflow.slice(Math.max(0, i - 1500), i + 1500)
}

function countOccurrences(text: string, needle: string): number {
  return text.split(needle).length - 1
}

function buildRunLine(workflow: string): string {
  // Match only the executable `run:` step: skip `#` comment lines so the
  // explanatory comment mentioning the same command can never satisfy the
  // contract assertions.
  const line = workflow.split('\n').find((l) => {
    const trimmed = l.trimStart()
    if (trimmed.startsWith('#')) return false
    if (!trimmed.startsWith('run:')) return false
    return l.includes('pnpm exec tauri build')
  })
  return line ?? ''
}

describe('windows pro validation workflow file', () => {
  it('registers the isolated workflow without touching the release workflow', () => {
    expect(existsSync(workflowPath), `${workflowRelPath} must exist`).toBe(true)
  })
})

describe('windows pro validation trigger', () => {
  it('triggers on push to feat/internet-archive, never on tags', () => {
    const trigger = onBlock(readWorkflowOrEmpty())
    expect(trigger).toContain('push')
    expect(trigger).toContain('branches')
    expect(trigger).toContain('- feat/internet-archive')
    expect(trigger).not.toContain('tags:')
    const items = trigger
      .split('\n')
      .map((l) => l.trim())
      .filter((l) => l.startsWith('- '))
    expect(items).toEqual(['- feat/internet-archive', `- ${workflowRelPath}`])
  })

  it('scopes the trigger to its own workflow path only', () => {
    const workflow = readWorkflowOrEmpty()
    const trigger = onBlock(workflow)
    expect(trigger).toContain('paths:')
    expect(trigger).toContain(workflowRelPath)
    expect(trigger).not.toContain('release.yml')
    expect(trigger).not.toContain('lite-preview.yml')
    expect(trigger).not.toContain('pull_request')
    expect(trigger).not.toContain('workflow_dispatch')
    expect(trigger).not.toContain('workflow_call')
    expect(trigger).not.toContain('schedule')
  })

  it('does not rely on workflow_dispatch for the branch-only workflow', () => {
    const workflow = readWorkflowOrEmpty()
    expect(workflow).not.toContain('workflow_dispatch')
    expect(workflow).not.toContain('workflow_call')
  })
})

describe('windows pro validation permissions', () => {
  it('grants exactly contents read and no writable permission', () => {
    const workflow = readWorkflowOrEmpty()
    const block = permissionsBlock(workflow)
    expect(block.trim()).toBe('contents: read')
    expect(block).not.toContain('write')
    expect(workflow).not.toContain('contents: write')
    expect(workflow).not.toContain('write-all')
    expect(countOccurrences(workflow, 'contents:')).toBe(1)
  })
})

describe('windows pro validation build', () => {
  it('builds on Windows only with a single job and no matrix', () => {
    const workflow = readWorkflowOrEmpty()
    expect(workflow).toContain('windows-latest')
    expect(countOccurrences(workflow, 'runs-on:')).toBe(1)
    expect(workflow).not.toContain('ubuntu')
    expect(workflow).not.toContain('macos')
    expect(workflow).not.toContain('matrix:')
  })

  it('runs no Lite or Unix jobs', () => {
    const workflow = readWorkflowOrEmpty()
    expect(workflow).not.toContain('build-lite')
    expect(workflow).not.toContain('lite-unix')
    expect(workflow).not.toContain('lite_only')
    expect(workflow).not.toContain('tauri.lite')
    expect(workflow).not.toContain('attach-')
  })

  it('installs Node 22 and pnpm 9 from the canonical lockfile', () => {
    const workflow = readWorkflowOrEmpty()
    expect(workflow).toContain('node-version: 22')
    expect(workflow).toContain('9.15.4')
    expect(workflow).toContain('git checkout -- pnpm-lock.yaml')
    expect(workflow).toContain('pnpm install --frozen-lockfile')
  })

  it('pins the Pro frontend to local-ml on', () => {
    const workflow = readWorkflowOrEmpty()
    expect(workflow).toContain("VITE_LOCAL_ML: '1'")
    expect(buildStepBlock(workflow)).toContain("VITE_LOCAL_ML: '1'")
    expect(workflow).not.toContain("VITE_LOCAL_ML: '0'")
  })

  it('builds the Pro bundles with the Windows resource overlay from apps/desktop', () => {
    const workflow = readWorkflowOrEmpty()
    expect(workflow).toContain('working-directory: apps/desktop')
    expect(buildRunLine(workflow)).toContain('pnpm exec tauri build')
    expect(buildRunLine(workflow)).toContain('--features local-ml')
    expect(buildRunLine(workflow)).toContain('--bundles nsis,msi')
    expect(buildRunLine(workflow)).toContain('tauri.windows.conf.json')
    expect(workflow).not.toContain('tauri-action')
    expect(workflow).not.toContain('GITHUB_TOKEN')
  })

  it('runs the build step outside pwsh so the Windows PowerShell hook keeps its own modules', () => {
    // The WebView2 repair hook runs under Windows PowerShell 5.1. A pwsh parent
    // leaks PowerShell 7 module paths into PSModulePath, so 5.1 fails to load
    // Microsoft.PowerShell.Security (Get-AuthenticodeSignature).
    const workflow = readWorkflowOrEmpty()
    const lines = workflow.split('\n')
    const start = lines.findIndex((l) => l.trim() === '- name: Build Pro installers (NSIS + MSI)')
    expect(start).toBeGreaterThan(-1)
    const rest = lines.slice(start + 1)
    const end = rest.findIndex((l) => l.trim().startsWith('- name:'))
    const step = (end === -1 ? rest : rest.slice(0, end)).filter((l) => !l.trimStart().startsWith('#'))
    expect(step.map((l) => l.trim())).toContain('shell: bash')
    expect(step.some((l) => l.trim() === 'shell: pwsh')).toBe(false)
  })

  it('reuses the signed runtime bootstrap source literals', () => {
    const workflow = readWorkflowOrEmpty()
    expect(workflow).toContain('ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL')
    expect(workflow).toContain(BOOTSTRAP_MANIFEST_URL)
    expect(workflow).toContain('ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64')
    expect(workflow).toContain(BOOTSTRAP_PUBLIC_KEY_BASE64)
    expect(workflow).toContain(BOOTSTRAP_PUBLIC_KEY_ID)
  })

  it('prepares helpers without debug cargo double-builds', () => {
    const workflow = readWorkflowOrEmpty()
    expect(workflow).toContain('pnpm/action-setup')
    expect(workflow).toContain('actions/setup-node')
    expect(workflow).toContain('dtolnay/rust-toolchain')
    expect(workflow).toContain('1.90.0')
    expect(workflow).toContain('Swatinem/rust-cache')
    // windows-feature-contract.ps1 runs DEBUG cargo builds for local-ml AND
    // the lean baseline (not unit tests); excluded here to avoid an
    // extraneous double build before the 35-60 min Pro release build. The
    // WebView2 wrapper unit tests still run via the Windows overlay hook
    // (prepare-webview2-repair.ps1, rustc --test, fail-closed).
    expect(workflow).not.toContain('windows-feature-contract.ps1')
  })
})

describe('windows pro validation artifacts', () => {
  it('guards both NSIS and MSI installers and uploads artifacts only', () => {
    const workflow = readWorkflowOrEmpty()
    const guardMarker = 'Guard NSIS + MSI presence'
    const uploadMarker = 'actions/upload-artifact'
    const guardIndex = workflow.indexOf(guardMarker)
    const uploadIndex = workflow.indexOf(uploadMarker)
    expect(guardIndex).toBeGreaterThanOrEqual(0)
    expect(uploadIndex).toBeGreaterThanOrEqual(0)
    expect(guardIndex).toBeLessThan(uploadIndex)
    const guardBlock = workflow.slice(guardIndex, uploadIndex)
    expect(guardBlock).toContain('bundle/nsis')
    expect(guardBlock).toContain('*-setup.exe')
    expect(guardBlock).toContain('bundle/msi')
    expect(guardBlock).toContain('*.msi')
    expect(guardBlock).toContain('throw')
    expect(guardBlock.indexOf('bundle/nsis')).toBeLessThan(guardBlock.indexOf('bundle/msi'))
    expect(workflow).toContain('if-no-files-found: error')
    expect(workflow).not.toContain('actions/download-artifact')
  })

  it('passes no release or tag parameters and calls no release API', () => {
    const workflow = readWorkflowOrEmpty()
    expect(workflow).not.toContain('tagName')
    expect(workflow).not.toContain('releaseName')
    expect(workflow).not.toContain('releaseBody')
    expect(workflow).not.toContain('releaseDraft')
    expect(workflow).not.toContain('prerelease')
    expect(workflow).not.toContain('gh release')
    expect(workflow).not.toContain('secrets.')
  })
})
