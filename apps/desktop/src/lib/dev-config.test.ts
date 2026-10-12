import { describe, expect, it } from 'vitest'
import { execFileSync } from 'node:child_process'
import { readFileSync, readdirSync } from 'node:fs'
import { dirname, isAbsolute, relative, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

/**
 * Debug-configuration guards.
 *
 * `tauri.conf.json` must not assume a sibling EntropIA-Agent checkout: the
 * watch list is empty by default, and a developer who wants hot-reload over a
 * sibling checkout passes `--config` on their own `tauri dev` invocation. That
 * only stays true while no committed config points outside the repository.
 *
 * The dev window title is variant-neutral on purpose: debug builds override it
 * at runtime with `PRODUCT_NAME (dev)`, so the static title names no variant.
 *
 * The PowerShell runner must find an interpreter on every platform: `pwsh`
 * first (Linux, macOS, Windows), then Windows PowerShell as a fallback.
 */

const currentDir = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(currentDir, '../../../..')
const tauriRoot = resolve(repoRoot, 'apps/desktop/src-tauri')
const runnerPath = resolve(repoRoot, 'apps/desktop/scripts/run-powershell.mjs')

interface TauriConfig {
  build?: {
    additionalWatchFolders?: string[]
  }
  app?: {
    windows?: Array<{ title?: string }>
  }
}

function readConfig(file: string): TauriConfig {
  return JSON.parse(readFileSync(resolve(tauriRoot, file), 'utf8')) as TauriConfig
}

function tauriConfigs(): string[] {
  return readdirSync(tauriRoot).filter((name) => /^tauri.*\.conf\.json$/.test(name))
}

/** True when `target` resolves inside the repository root. */
function isInsideRepo(target: string): boolean {
  const path = relative(repoRoot, target)
  return path === '' || (!path.startsWith('..') && !isAbsolute(path))
}

/** Runs `choosePowerShell` from the real runner module in a fresh Node process. */
function chooseWithNode(platform: string, commands: string[]): string | null {
  const script = `
    const { choosePowerShell } = await import(${JSON.stringify(pathToFileURL(runnerPath).href)})
    const available = new Set(${JSON.stringify(commands)})
    console.log(choosePowerShell(${JSON.stringify(platform)}, (name) => available.has(name)))
  `
  const output = execFileSync(process.execPath, ['--input-type=module', '-e', script], {
    encoding: 'utf8',
  })
  const trimmed = output.trim()
  return trimmed === 'null' ? null : trimmed
}

describe('tauri debug configuration', () => {
  it('never watches a folder outside the repository', () => {
    const outside: string[] = []
    for (const file of tauriConfigs()) {
      for (const folder of readConfig(file).build?.additionalWatchFolders ?? []) {
        const resolved = resolve(tauriRoot, folder)
        if (!isInsideRepo(resolved)) {
          outside.push(`${file}: ${folder} -> ${resolved}`)
        }
      }
    }
    expect(outside, 'additionalWatchFolders may not leave the repository').toEqual([])
  })

  it('keeps the dev window title variant-neutral', () => {
    const title = readConfig('tauri.dev.conf.json').app?.windows?.[0]?.title
    expect(title, 'tauri.dev.conf.json declares the main window title').toBeDefined()
    expect(title).not.toContain('Lite')
    expect(title).not.toContain('Pro')
  })
})

describe('run-powershell.mjs', () => {
  it('prefers pwsh wherever it is available', () => {
    expect(chooseWithNode('linux', ['pwsh'])).toBe('pwsh')
    expect(chooseWithNode('win32', ['pwsh', 'powershell'])).toBe('pwsh')
  })

  it('falls back to Windows PowerShell only on win32', () => {
    expect(chooseWithNode('win32', ['powershell'])).toBe('powershell')
    expect(chooseWithNode('linux', ['powershell'])).toBeNull()
  })

  it('returns null when no interpreter is available', () => {
    expect(chooseWithNode('linux', [])).toBeNull()
  })
})
