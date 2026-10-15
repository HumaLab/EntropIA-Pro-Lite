#!/usr/bin/env node
/**
 * Runs a PowerShell script with whichever interpreter this machine has.
 *
 * `pnpm rust:quality:report` used to invoke `powershell`, which only exists on
 * Windows. The `.ps1` it runs is cross-platform, so this launcher tries
 * PowerShell 7 (`pwsh`) first — the one that runs on Linux, macOS and Windows —
 * and only falls back to Windows PowerShell on Windows. With neither
 * installed it exits 127 and points at the PowerShell 7 installer.
 *
 * Usage:
 *   node apps/desktop/scripts/run-powershell.mjs <script.ps1> [args...]
 */

import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

/**
 * Picks the PowerShell command to run.
 *
 * Pure so tests can drive every platform without spawning anything:
 * `isAvailable(name)` answers whether that command runs here.
 *
 * @param {string} platform - `process.platform` value.
 * @param {(command: string) => boolean} isAvailable
 * @returns {'pwsh' | 'powershell' | null}
 */
export function choosePowerShell(platform, isAvailable) {
  if (isAvailable('pwsh')) return 'pwsh'
  if (platform === 'win32' && isAvailable('powershell')) return 'powershell'
  return null
}

/** Whether `command` exists and starts: status 0 and no spawn error. */
function hasCommand(command) {
  const result = spawnSync(command, ['-NoProfile', '-Command', 'exit 0'])
  return result.status === 0 && !result.error
}

/** Runs the script; returns the process exit code (127 when no PowerShell). */
function main(argv) {
  const [script, ...args] = argv
  if (!script) {
    console.error('Usage: node apps/desktop/scripts/run-powershell.mjs <script.ps1> [args...]')
    return 2
  }

  const command = choosePowerShell(process.platform, hasCommand)
  if (!command) {
    console.error(
      'PowerShell 7 (pwsh) is required to run this script. Install it from ' +
        'https://learn.microsoft.com/powershell/scripting/install/installing-powershell ' +
        '(available on Linux and macOS); Windows PowerShell is only used on Windows.'
    )
    return 127
  }

  const result = spawnSync(command, ['-NoProfile', '-File', script, ...args], {
    stdio: 'inherit',
  })
  if (result.error) {
    console.error(`Could not run ${command}: ${result.error.message}`)
    return 1
  }
  return result.status ?? 1
}

const isDirectRun =
  process.argv[1] !== undefined && pathToFileURL(resolve(process.argv[1])).href === import.meta.url

if (isDirectRun) {
  process.exit(main(process.argv.slice(2)))
}
