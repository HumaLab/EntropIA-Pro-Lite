import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { spawn, spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer, type Server } from 'node:http'
import type { AddressInfo } from 'node:net'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const currentDir = dirname(fileURLToPath(import.meta.url))
const desktopRoot = resolve(currentDir, '../..')
const scriptPath = resolve(desktopRoot, 'scripts/check-runtime-bootstrap.mjs')

interface RuntimeBootstrapRelease {
  app_version: string
  pack_version: string
  platform: string
}

let tmpDir: string

beforeAll(() => {
  tmpDir = mkdtempSync(join(tmpdir(), 'runtime-bootstrap-gate-'))
})

afterAll(() => {
  rmSync(tmpDir, { recursive: true, force: true })
})

/** Writes a manifest-shaped fixture and returns its path. */
function writeManifest(name: string, releases: unknown): string {
  const file = join(tmpDir, name)
  writeFileSync(
    file,
    JSON.stringify({
      channel: 'stable',
      generated_at: '2026-01-01T00:00:00Z',
      releases,
    })
  )
  return file
}

function runScript(args: string[]): { status: number | null; output: string } {
  const result = spawnSync(process.execPath, [scriptPath, ...args], { encoding: 'utf8' })
  expect(result.error).toBeUndefined()
  return { status: result.status, output: `${result.stdout}${result.stderr}` }
}

/** Like `runScript`, but async so an in-process HTTP server keeps serving. */
function runScriptAsync(args: string[]): Promise<{ status: number | null; output: string }> {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(process.execPath, [scriptPath, ...args])
    let output = ''
    child.stdout.on('data', (chunk) => (output += chunk))
    child.stderr.on('data', (chunk) => (output += chunk))
    child.on('error', reject)
    child.on('close', (status) => resolvePromise({ status, output }))
  })
}

function release(
  appVersion: string,
  platform: string,
  packVersion = '2026.01.0'
): RuntimeBootstrapRelease {
  return { app_version: appVersion, pack_version: packVersion, platform }
}

describe('runtime bootstrap release gate', () => {
  it('exits 0 with the found line when the exact app_version + platform match', () => {
    const manifest = writeManifest('match.json', [release('9.9.9', 'windows-x86_64', '2026.05.0')])
    const result = runScript([
      '--platform',
      'windows-x86_64',
      '--app-version',
      '9.9.9',
      '--manifest-file',
      manifest,
    ])

    expect(result.status).toBe(0)
    expect(result.output).toContain(
      'runtime bootstrap: found windows-x86_64 9.9.9 (pack 2026.05.0)'
    )
  })

  it('exits 1 and lists the published pairs when only another platform matches', () => {
    const manifest = writeManifest('other-platform.json', [
      release('9.9.9', 'linux-x86_64'),
      release('9.9.8', 'windows-x86_64'),
    ])
    const result = runScript([
      '--platform',
      'windows-x86_64',
      '--app-version',
      '9.9.9',
      '--manifest-file',
      manifest,
    ])

    expect(result.status).toBe(1)
    expect(result.output).toContain('9.9.9 / linux-x86_64')
    expect(result.output).toContain('9.9.8 / windows-x86_64')
  })

  it('exits 1 when the platform matches but the app version does not', () => {
    const manifest = writeManifest('other-version.json', [release('9.9.8', 'windows-x86_64')])
    const result = runScript([
      '--platform',
      'windows-x86_64',
      '--app-version',
      '9.9.9',
      '--manifest-file',
      manifest,
    ])

    expect(result.status).toBe(1)
    expect(result.output).toContain('9.9.8 / windows-x86_64')
  })

  it('exits 1 when the manifest publishes no releases at all', () => {
    const manifest = writeManifest('empty.json', [])
    const result = runScript([
      '--platform',
      'windows-x86_64',
      '--app-version',
      '9.9.9',
      '--manifest-file',
      manifest,
    ])

    expect(result.status).toBe(1)
    expect(result.output).toContain('(none)')
  })

  it('exits 2 on invalid JSON', () => {
    const file = join(tmpDir, 'invalid.json')
    writeFileSync(file, '{ "releases": [')
    const result = runScript([
      '--platform',
      'windows-x86_64',
      '--app-version',
      '9.9.9',
      '--manifest-file',
      file,
    ])

    expect(result.status).toBe(2)
    expect(result.output).toContain('manifest is not valid JSON')
  })

  it('exits 2 when the manifest URL never answers', async () => {
    // Accepts the request and never responds: a hung release host must fail
    // the gate instead of stalling the release job until its own timeout.
    const server: Server = createServer(() => {})
    await new Promise<void>((done) => server.listen(0, '127.0.0.1', done))
    const { port } = server.address() as AddressInfo
    try {
      const result = await runScriptAsync([
        '--platform',
        'windows-x86_64',
        '--app-version',
        '9.9.9',
        '--manifest-url',
        `http://127.0.0.1:${port}/manifest.json`,
        '--timeout-ms',
        '300',
      ])

      expect(result.status).toBe(2)
      expect(result.output).toContain('timed out after 300 ms')
    } finally {
      server.closeAllConnections()
      await new Promise<void>((done) => server.close(() => done()))
    }
  })

  it('exits 2 on a non-positive --timeout-ms', () => {
    const manifest = writeManifest('bad-timeout.json', [release('9.9.9', 'windows-x86_64')])
    const result = runScript([
      '--platform',
      'windows-x86_64',
      '--app-version',
      '9.9.9',
      '--manifest-file',
      manifest,
      '--timeout-ms',
      '0',
    ])

    expect(result.status).toBe(2)
    expect(result.output).toContain('--timeout-ms must be a positive integer')
  })

  it('exits 2 when --platform is missing', () => {
    const manifest = writeManifest('no-platform.json', [release('9.9.9', 'windows-x86_64')])
    const result = runScript(['--app-version', '9.9.9', '--manifest-file', manifest])

    expect(result.status).toBe(2)
    expect(result.output).toContain('--platform')
  })

  it('defaults --app-version to the version in apps/desktop/package.json', () => {
    const { version } = JSON.parse(readFileSync(resolve(desktopRoot, 'package.json'), 'utf8')) as {
      version: string
    }
    const manifest = writeManifest('default-version.json', [release(version, 'windows-x86_64')])
    const result = runScript(['--platform', 'windows-x86_64', '--manifest-file', manifest])

    expect(result.status).toBe(0)
    expect(result.output).toContain(`runtime bootstrap: found windows-x86_64 ${version}`)
  })
})
