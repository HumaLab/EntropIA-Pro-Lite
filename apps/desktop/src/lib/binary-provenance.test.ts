import { afterEach, describe, expect, it } from 'vitest'
import { execFileSync, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const currentDir = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(currentDir, '../../../..')
const verifyScript = resolve(repoRoot, 'apps/desktop/scripts/verify-binaries.mjs')
const manifestRelPath = 'apps/desktop/src-tauri/binary-provenance.json'
const uvSourcePath = 'apps/desktop/src-tauri/src/deps/uv.rs'
const pdfiumScriptPath = 'apps/desktop/scripts/fetch-pdfium.sh'

const tempDirs: string[] = []

afterEach(() => {
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true })
  }
})

function makeTempDir(prefix: string): string {
  const dir = mkdtempSync(join(tmpdir(), prefix))
  tempDirs.push(dir)
  return dir
}

interface ScriptResult {
  status: number
  stdout: string
  stderr: string
  output: string
}

function runScript(root: string, args: string[]): ScriptResult {
  const result = spawnSync(process.execPath, [verifyScript, ...args, '--root', root], {
    cwd: repoRoot,
    encoding: 'utf8',
  })
  const stdout = result.stdout ?? ''
  const stderr = result.stderr ?? ''
  return { status: result.status ?? -1, stdout, stderr, output: stdout + stderr }
}

function sha256(data: string | Buffer): string {
  return createHash('sha256').update(data).digest('hex')
}

function fixtureEntry(path: string, content: Buffer): Record<string, unknown> {
  return {
    path,
    sha256: sha256(content),
    size: content.length,
    kind: 'fixture',
    name: 'test fixture',
    version: null,
    provenance: 'test fixture',
  }
}

function writeManifest(root: string, entries: unknown[]): void {
  const target = join(root, manifestRelPath)
  mkdirSync(dirname(target), { recursive: true })
  const manifest = { schema: 1, description: 'test manifest', binaries: entries }
  writeFileSync(target, JSON.stringify(manifest, null, 2) + '\n')
}

function makeTempRepo(files: Record<string, Buffer | string>): string {
  const root = makeTempDir('entropia-binaries-')
  for (const [relative, content] of Object.entries(files)) {
    const target = join(root, relative)
    mkdirSync(dirname(target), { recursive: true })
    writeFileSync(target, content)
  }
  execFileSync('git', ['init', '-q'], { cwd: root })
  execFileSync('git', ['config', 'user.email', 'test@example.com'], { cwd: root })
  execFileSync('git', ['config', 'user.name', 'Test'], { cwd: root })
  execFileSync('git', ['add', '-A'], { cwd: root })
  return root
}

/** Builds a .tgz containing bin/x.dll with `payload` as its content. */
function buildTgz(root: string, payload: Buffer): string {
  const staging = join(root, 'staging')
  mkdirSync(join(staging, 'bin'), { recursive: true })
  writeFileSync(join(staging, 'bin', 'x.dll'), payload)
  // Relative paths: Git Bash's GNU tar reads `C:\...` as a remote host.
  execFileSync('tar', ['-czf', 'x.tgz', '-C', 'staging', 'bin/x.dll'], { cwd: root })
  return join(root, 'x.tgz')
}

function upstreamEntry(
  path: string,
  payload: Buffer,
  archivePath: string
): Record<string, unknown> {
  return {
    path,
    sha256: sha256(payload),
    size: payload.length,
    kind: 'upstream',
    name: 'test upstream',
    version: '1.0.0',
    provenance: 'test upstream',
    source: {
      url: 'https://example.invalid/x.tgz',
      archive_sha256: sha256(readFileSync(archivePath)),
      member: path,
    },
  }
}

function buildCacheDir(archivePath: string, fileName = 'x.tgz'): string {
  const cache = makeTempDir('entropia-binaries-cache-')
  writeFileSync(join(cache, fileName), readFileSync(archivePath))
  return cache
}

function readManifestBinaries(): Array<Record<string, string | null>> {
  const manifest = JSON.parse(readFileSync(join(repoRoot, manifestRelPath), 'utf8')) as {
    binaries: Array<Record<string, string | null>>
  }
  return manifest.binaries
}

describe('binary provenance check', () => {
  it('passes on the real repository', () => {
    const result = runScript(repoRoot, ['check'])
    expect(result.status).toBe(0)
    expect(result.stdout).toContain('binaries: OK')
  })

  it('fails and names an unlisted tracked .dll', () => {
    const listed = Buffer.from('listed fixture bytes')
    const unlisted = Buffer.from('unlisted dll bytes')
    const root = makeTempRepo({ 'a.dll': listed, 'extra.dll': unlisted })
    writeManifest(root, [fixtureEntry('a.dll', listed)])
    const result = runScript(root, ['check'])
    expect(result.status).toBe(1)
    expect(result.output).toContain('extra.dll')
  })

  it('fails on a tampered byte', () => {
    const content = Buffer.from('original bytes')
    const root = makeTempRepo({ 'a.dll': content })
    writeManifest(root, [fixtureEntry('a.dll', content)])
    // Same length, one byte flipped: only the digest comparison can catch it.
    writeFileSync(join(root, 'a.dll'), Buffer.from('Original bytes'))
    const result = runScript(root, ['check'])
    expect(result.status).toBe(1)
    expect(result.output).toContain('a.dll: sha256')
    expect(result.output).not.toContain('a.dll: size')
  })

  it('fails when a listed file is missing', () => {
    const content = Buffer.from('present bytes')
    const missing = Buffer.from('missing bytes')
    const root = makeTempRepo({ 'a.dll': content })
    writeManifest(root, [fixtureEntry('a.dll', content), fixtureEntry('missing.dll', missing)])
    const result = runScript(root, ['check'])
    expect(result.status).toBe(1)
    expect(result.output).toContain('missing.dll')
  })

  it('fails on an upstream entry without archive_sha256', () => {
    const content = Buffer.from('upstream-ish bytes')
    const root = makeTempRepo({ 'a.dll': content })
    const entry = fixtureEntry('a.dll', content)
    writeManifest(root, [
      {
        ...entry,
        kind: 'upstream',
        source: { url: 'https://example.invalid/a.tgz', member: 'a.dll' },
      },
    ])
    const result = runScript(root, ['check'])
    expect(result.status).toBe(1)
    expect(result.output).toContain('archive_sha256')
  })

  it('fails on a fixture entry carrying a source', () => {
    const content = Buffer.from('fixture bytes')
    const root = makeTempRepo({ 'a.dll': content })
    const entry = fixtureEntry('a.dll', content)
    writeManifest(root, [
      {
        ...entry,
        source: {
          url: 'https://example.invalid/a.tgz',
          archive_sha256: sha256('nope'),
          member: 'a.dll',
        },
      },
    ])
    const result = runScript(root, ['check'])
    expect(result.status).toBe(1)
    expect(result.output).toContain('source')
  })
})

describe('binary provenance verify-upstream', () => {
  it('accepts a cached archive whose member matches', () => {
    const payload = Buffer.from('upstream payload bytes')
    const root = makeTempRepo({ 'bin/x.dll': payload })
    const archive = buildTgz(root, payload)
    writeManifest(root, [upstreamEntry('bin/x.dll', payload, archive)])
    const result = runScript(root, ['verify-upstream', '--cache', buildCacheDir(archive)])
    expect(result.status).toBe(0)
  })

  it('fails on a wrong archive_sha256', () => {
    const payload = Buffer.from('upstream payload bytes')
    const root = makeTempRepo({ 'bin/x.dll': payload })
    const archive = buildTgz(root, payload)
    const entry = upstreamEntry('bin/x.dll', payload, archive)
    ;(entry.source as Record<string, string>).archive_sha256 = sha256('different archive')
    writeManifest(root, [entry])
    const result = runScript(root, ['verify-upstream', '--cache', buildCacheDir(archive)])
    expect(result.status).toBe(1)
  })

  it('fails when the member content differs from the committed sha', () => {
    const payload = Buffer.from('upstream payload bytes')
    const root = makeTempRepo({ 'bin/x.dll': payload })
    const archive = buildTgz(root, payload)
    const entry = upstreamEntry('bin/x.dll', payload, archive)
    entry.sha256 = sha256('a different payload')
    writeManifest(root, [entry])
    const result = runScript(root, ['verify-upstream', '--cache', buildCacheDir(archive)])
    expect(result.status).toBe(1)
  })
})

describe('binary provenance path safety', () => {
  it.each(['../escape.dll', '/etc/x.dll', 'bin/../../x.dll', 'bin\\x.dll'])(
    'rejects the upstream member %s',
    (member) => {
      const payload = Buffer.from('upstream payload bytes')
      const root = makeTempRepo({ 'bin/x.dll': payload })
      const archive = buildTgz(root, payload)
      const entry = upstreamEntry('bin/x.dll', payload, archive)
      ;(entry.source as Record<string, string>).member = member
      writeManifest(root, [entry])
      const result = runScript(root, ['check'])
      expect(result.status).toBe(1)
      expect(result.output).toContain('source.member')
    }
  )

  it('rejects an archive URL whose file name decodes to a path', () => {
    const payload = Buffer.from('upstream payload bytes')
    const root = makeTempRepo({ 'bin/x.dll': payload })
    const archive = buildTgz(root, payload)
    const entry = upstreamEntry('bin/x.dll', payload, archive)
    ;(entry.source as Record<string, string>).url = 'https://example.invalid/a/..%2F..%2Fx.tgz'
    writeManifest(root, [entry])
    const result = runScript(root, ['check'])
    expect(result.status).toBe(1)
    expect(result.output).toContain('source.url')
  })
})

describe('binary provenance verify-upstream reports every entry', () => {
  it('keeps checking after an entry fails to download or extract', () => {
    const payload = Buffer.from('upstream payload bytes')
    const root = makeTempRepo({ 'bin/x.dll': payload, 'bin/y.dll': payload })
    const archive = buildTgz(root, payload)
    // The first entry names a member the archive lacks; the second is fine and
    // must still be checked and reported.
    const broken = upstreamEntry('bin/x.dll', payload, archive)
    ;(broken.source as Record<string, string>).member = 'bin/missing.dll'
    const good = upstreamEntry('bin/y.dll', payload, archive)
    ;(good.source as Record<string, string>).member = 'bin/x.dll'
    writeManifest(root, [broken, good])
    const result = runScript(root, ['verify-upstream', '--cache', buildCacheDir(archive)])
    expect(result.status).toBe(2)
    expect(result.output).toContain('bin/x.dll:')
    expect(result.output).toContain('bin/y.dll: OK')
  })
})

describe('manifest pins agree with the code', () => {
  it('pins the uv version and digests used by deps/uv.rs', () => {
    const source = readFileSync(join(repoRoot, uvSourcePath), 'utf8')
    const binaries = readManifestBinaries()
    const x64 = binaries.find((entry) => String(entry.path).endsWith('windows-x86_64/uv.exe'))
    const aarch64 = binaries.find((entry) => String(entry.path).endsWith('windows-aarch64/uv.exe'))
    expect(x64).toBeDefined()
    expect(aarch64).toBeDefined()
    expect(source).toContain(`UV_VERSION: &str = "${x64?.version}"`)
    const x64Digest = source.match(/EXPECTED_UV_SHA256_WINDOWS_X86_64: &str =\s*"([0-9a-f]{64})"/)
    const aarch64Digest = source.match(
      /EXPECTED_UV_SHA256_WINDOWS_AARCH64: &str =\s*"([0-9a-f]{64})"/
    )
    expect(x64Digest?.[1]).toBe(x64?.sha256)
    expect(aarch64Digest?.[1]).toBe(aarch64?.sha256)
  })

  it('pins the pdfium tag used by fetch-pdfium.sh', () => {
    const script = readFileSync(join(repoRoot, pdfiumScriptPath), 'utf8')
    const tag = script.match(/^TAG="([^"]+)"/m)
    const pdfium = readManifestBinaries().find((entry) =>
      String(entry.path).endsWith('resources/lib/pdfium.dll')
    )
    expect(tag?.[1]).toBeDefined()
    expect(pdfium?.version).toBe(tag?.[1])
  })
})
