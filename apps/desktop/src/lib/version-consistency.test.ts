import { afterEach, describe, expect, it } from 'vitest'
import { spawnSync } from 'node:child_process'
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const currentDir = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(currentDir, '../../../..')
const versionScript = resolve(repoRoot, 'apps/desktop/scripts/version.mjs')

/**
 * Every file the version script owns. The script itself must know the same list;
 * the fixture build below needs it to materialize an isolated repository copy.
 */
const DECLARATION_PATHS = [
  'apps/desktop/package.json',
  'apps/desktop/src-tauri/Cargo.toml',
  'apps/desktop/src-tauri/Cargo.lock',
  'apps/desktop/src-tauri/tauri.conf.json',
  'apps/desktop/src-tauri/tauri.lite.conf.json',
  'apps/desktop/src-tauri/resources/runtime-pack/windows-x86_64/manifest.json',
  'apps/desktop/src-tauri/resources/runtime-pack/linux-x86_64/manifest.json',
  'apps/desktop/src-tauri/scripts/repack-store-msix.ps1',
  'apps/desktop/src-tauri/msix/README.md',
] as const

const tempDirs: string[] = []

afterEach(() => {
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true })
  }
})

function readRepoFile(...segments: string[]): string {
  return readFileSync(resolve(repoRoot, ...segments), 'utf8')
}

interface ScriptResult {
  status: number
  stdout: string
  stderr: string
}

function runVersionScript(args: string[]): ScriptResult {
  const result = spawnSync(process.execPath, [versionScript, ...args], {
    cwd: repoRoot,
    encoding: 'utf8',
  })
  return {
    status: result.status ?? -1,
    stdout: result.stdout ?? '',
    stderr: result.stderr ?? '',
  }
}

/** Isolated copy of the real declaration files under a temp root. */
function makeTempRepo(): string {
  const root = mkdtempSync(join(tmpdir(), 'entropia-version-'))
  tempDirs.push(root)
  for (const relative of DECLARATION_PATHS) {
    const target = join(root, relative)
    mkdirSync(dirname(target), { recursive: true })
    copyFileSync(join(repoRoot, relative), target)
  }
  return root
}

function fixtureSnapshot(root: string): Record<string, string> {
  const snapshot: Record<string, string> = {}
  for (const relative of DECLARATION_PATHS) {
    snapshot[relative] = readFileSync(join(root, relative), 'utf8')
  }
  return snapshot
}

function readFixture(root: string, relative: string): string {
  return readFileSync(join(root, relative), 'utf8')
}

const packageJson = JSON.parse(readRepoFile('apps/desktop/package.json')) as { version: string }
const version = packageJson.version
const storeVersion = `${version}.0`

describe('version single source of truth (C-04)', () => {
  it('check passes on the real repository', () => {
    const result = runVersionScript(['check'])
    expect(result.stdout + result.stderr).toContain(version)
    expect(result.status).toBe(0)
  })

  it('print emits the package.json version and --store the 4-segment Store version', () => {
    const plain = runVersionScript(['print'])
    expect(plain.status).toBe(0)
    expect(plain.stdout.trim()).toBe(version)

    const store = runVersionScript(['print', '--store'])
    expect(store.status).toBe(0)
    expect(store.stdout.trim()).toBe(storeVersion)
  })

  it('check --tag accepts the matching tag and rejects a mismatching one', () => {
    expect(runVersionScript(['check', '--tag', `v${version}`]).status).toBe(0)

    const wrong = runVersionScript(['check', '--tag', 'v9.9.9'])
    expect(wrong.status).toBe(1)
    expect(wrong.stdout + wrong.stderr).toContain('v9.9.9')
  })

  it('check names the file of a drifted declaration', () => {
    const root = makeTempRepo()
    const ps1 = 'apps/desktop/src-tauri/scripts/repack-store-msix.ps1'
    const target = join(root, ps1)
    writeFileSync(target, readFixture(root, ps1).replace(storeVersion, '0.9.9.0'))

    const result = runVersionScript(['check', '--root', root])
    expect(result.status).toBe(1)
    expect(result.stdout + result.stderr).toContain(ps1)
    expect(result.stdout + result.stderr).toContain('0.9.9.0')
  })

  it('bump rewrites all nine declarations with targeted edits only', () => {
    const root = makeTempRepo()
    const result = runVersionScript(['bump', '2.3.4', '--root', root])
    expect(result.stdout + result.stderr).toContain('2.3.4')
    expect(result.status).toBe(0)

    const bumpedPackage = JSON.parse(readFixture(root, 'apps/desktop/package.json')) as {
      version: string
    }
    expect(bumpedPackage.version).toBe('2.3.4')

    const cargoToml = readFixture(root, 'apps/desktop/src-tauri/Cargo.toml')
    expect(cargoToml).toMatch(/^version = "2\.3\.4"$/m)

    const cargoLock = readFixture(root, 'apps/desktop/src-tauri/Cargo.lock')
    expect(cargoLock).toMatch(/name = "entropia-pro-desktop"\r?\nversion = "2\.3\.4"/)
    // Other crates that happened to share 1.0.20 (e.g. dyn-clone) must not move.
    expect(cargoLock).toMatch(/name = "dyn-clone"\r?\nversion = "1\.0\.20"/)

    expect(JSON.parse(readFixture(root, 'apps/desktop/src-tauri/tauri.conf.json'))).toMatchObject({
      version: '2.3.4',
    })
    expect(
      JSON.parse(readFixture(root, 'apps/desktop/src-tauri/tauri.lite.conf.json'))
    ).toMatchObject({ version: '2.3.4' })

    for (const manifest of [
      'apps/desktop/src-tauri/resources/runtime-pack/windows-x86_64/manifest.json',
      'apps/desktop/src-tauri/resources/runtime-pack/linux-x86_64/manifest.json',
    ]) {
      expect(JSON.parse(readFixture(root, manifest))).toMatchObject({ app_version: '2.3.4' })
    }

    const ps1 = readFixture(root, 'apps/desktop/src-tauri/scripts/repack-store-msix.ps1')
    expect(ps1).not.toContain(storeVersion)
    expect(ps1.match(/2\.3\.4\.0/g)).toHaveLength(2)

    const msixReadme = readFixture(root, 'apps/desktop/src-tauri/msix/README.md')
    expect(msixReadme).not.toContain(storeVersion)
    expect(msixReadme).toContain('2.3.4.0')

    expect(runVersionScript(['check', '--root', root]).status).toBe(0)
  })

  it('bump rejects a non-x.y.z version and changes nothing', () => {
    const root = makeTempRepo()
    const before = fixtureSnapshot(root)

    const result = runVersionScript(['bump', '1.2', '--root', root])
    expect(result.status).not.toBe(0)
    expect(fixtureSnapshot(root)).toEqual(before)
  })

  it('release.yml derives the Store version instead of hardcoding it', () => {
    const releaseYml = readRepoFile('.github/workflows', 'release.yml')
    expect(releaseYml).not.toMatch(/\d+\.\d+\.\d+\.0/)
    expect(releaseYml).toContain('steps.version.outputs.store_version')
  })
})
