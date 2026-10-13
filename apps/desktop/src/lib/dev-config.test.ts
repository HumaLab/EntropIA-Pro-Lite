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

describe('opencode.json MCP servers (R-02)', () => {
  interface McpServer {
    type: 'local' | 'remote'
    command?: string[]
  }
  // The committed (index) copy, not the working file: a developer may keep a
  // personal opencode.json under `git update-index --skip-worktree`.
  const committedConfig = (): string => {
    try {
      return execFileSync('git', ['show', ':opencode.json'], { cwd: repoRoot, encoding: 'utf8' })
    } catch {
      return readFileSync(resolve(repoRoot, 'opencode.json'), 'utf8')
    }
  }
  const config = JSON.parse(committedConfig()) as { mcp: Record<string, McpServer> }
  const local = Object.entries(config.mcp).filter(([, server]) => server.type === 'local')

  it('pins every npm package an MCP server runs to an exact version', () => {
    // With `--package=` flags those name the packages (the positional is then
    // a bin name); otherwise the first non-flag argument after the runner (and
    // `dlx`) is the package. `node@<major>` only selects the Node runtime. A
    // Windows `cmd /c` wrapper is looked through.
    for (const [name, server] of local) {
      const command = server.command ?? []
      const [runner, ...args] =
        command[0] === 'cmd' && command[1]?.toLowerCase() === '/c' ? command.slice(2) : command
      if (runner !== 'npx' && runner !== 'pnpm') continue
      const flagged = args.filter((arg) => arg.startsWith('--package=')).map((arg) => arg.slice(10))
      const packages = flagged.length
        ? flagged.filter((spec) => !/^node@\d+$/.test(spec))
        : args.filter((arg) => arg !== 'dlx' && !arg.startsWith('-')).slice(0, 1)
      expect(packages.length, name).toBeGreaterThan(0)
      for (const spec of packages) {
        expect(spec, `${name}: ${spec}`).toMatch(/^(@[\w.-]+\/)?[\w.-]+@\d+\.\d+\.\d+$/)
      }
    }
  })

  it('runs no executable from a personal absolute path', () => {
    for (const [name, server] of local) {
      const executable = server.command?.[0] ?? ''
      expect(executable, name).not.toMatch(/^([A-Za-z]:[\\/]|\/|~)/)
      expect(executable, name).not.toMatch(/Users[\\/]|home[\\/]/)
    }
  })
})
