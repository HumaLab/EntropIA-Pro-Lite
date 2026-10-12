// Committed-binary provenance ledger and checker (audit finding D-07).
//
// The repository tracks native libraries, bundled tools, OCR models, a captured
// base MSIX, release output and runtime-pack fixtures. Before this script those
// bytes had no recorded provenance: a replaced, truncated or corrupted binary
// could only be noticed by hand. This script reads
// apps/desktop/src-tauri/binary-provenance.json and:
//
//   check            validates the manifest schema, checks every listed file
//                    against its recorded size/sha256, and fails if any
//                    git-tracked binary of a known extension is not listed.
//   verify-upstream  downloads each `upstream` archive, verifies its pinned
//                    archive_sha256, extracts the pinned member and compares
//                    its sha256 with the committed digest. `--cache <dir>`
//                    reuses `<dir>/<archive basename>` instead of downloading.
//
// Usage:
//   node apps/desktop/scripts/verify-binaries.mjs check [--root <dir>]
//   node apps/desktop/scripts/verify-binaries.mjs verify-upstream [--root <dir>] [--cache <dir>]
//
// `--root` defaults to the repository root resolved from this script location.
// Exit codes: check 0 = OK, 1 = problems. verify-upstream 0 = all match,
// 1 = mismatch, 2 = download, tool or usage error.

import { execFileSync, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

/** Manifest location, relative to the repository root. */
export const MANIFEST_PATH = 'apps/desktop/src-tauri/binary-provenance.json'

/** Extensions of committed binaries that must be listed in the manifest (D-07). */
export const BINARY_EXTENSION_PATTERN =
  /\.(exe|dll|so|dylib|mnn|onnx|msix|appx|zip|tgz|tar|gz|bin|node|pyd|whl|pdb|lib)$/i

const KINDS = new Set(['upstream', 'vendored', 'release-output', 'fixture'])
const SHA256_PATTERN = /^[0-9a-f]{64}$/
const DOWNLOAD_TIMEOUT_MS = 120000

const here = dirname(fileURLToPath(import.meta.url))
const defaultRoot = resolve(here, '../../..')

/** SHA-256 (lowercase hex) of a file on disk. */
export function sha256File(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

function loadManifest(root) {
  try {
    return { manifest: JSON.parse(readFileSync(join(root, MANIFEST_PATH), 'utf8')), problems: [] }
  } catch (error) {
    return {
      manifest: null,
      problems: [`${MANIFEST_PATH}: cannot read or parse the manifest (${error.message})`],
    }
  }
}

/**
 * The member is joined under a fresh extraction dir and handed to tar/unzip, so
 * it must stay inside that dir: relative, forward slashes only, no '..'.
 */
function isSafeMember(member) {
  if (member === '' || member.startsWith('/') || member.includes('\\')) return false
  return member.split('/').every((segment) => segment !== '' && segment !== '..')
}

/** The decoded URL basename becomes a file name in the cache or download dir. */
function isSafeArchiveName(url) {
  try {
    const name = cacheFileName(url)
    return !name.includes('/') && !name.includes('\\') && name !== '.' && name !== '..'
  } catch {
    return false
  }
}

/** Schema problems of a parsed manifest, as `<path or field>: <problem>` lines. */
export function validateManifest(manifest) {
  if (manifest === null || typeof manifest !== 'object' || Array.isArray(manifest)) {
    return ['manifest: must be a JSON object']
  }
  const problems = []
  if (manifest.schema !== 1) problems.push('manifest: schema must be 1')
  if (typeof manifest.description !== 'string' || manifest.description.trim() === '') {
    problems.push('manifest: description must be a non-empty string')
  }
  if (!Array.isArray(manifest.binaries)) {
    problems.push('manifest: binaries must be an array')
    return problems
  }
  const seenPaths = new Set()
  for (let index = 0; index < manifest.binaries.length; index += 1) {
    const entry = manifest.binaries[index]
    const where =
      entry !== null &&
      typeof entry === 'object' &&
      typeof entry.path === 'string' &&
      entry.path !== ''
        ? entry.path
        : `binaries[${index}]`
    if (entry === null || typeof entry !== 'object' || Array.isArray(entry)) {
      problems.push(`${where}: entry must be an object`)
      continue
    }
    if (typeof entry.path !== 'string' || entry.path === '') {
      problems.push(`${where}: path must be a non-empty string`)
    } else if (seenPaths.has(entry.path)) {
      problems.push(`${where}: duplicate path`)
    } else {
      seenPaths.add(entry.path)
    }
    if (typeof entry.sha256 !== 'string' || !SHA256_PATTERN.test(entry.sha256)) {
      problems.push(`${where}: sha256 must be a 64-character lowercase hex digest`)
    }
    if (!Number.isInteger(entry.size) || entry.size < 0) {
      problems.push(`${where}: size must be a non-negative integer`)
    }
    if (!KINDS.has(entry.kind)) {
      problems.push(`${where}: kind must be one of ${[...KINDS].join(', ')}`)
    }
    if (typeof entry.name !== 'string' || entry.name === '') {
      problems.push(`${where}: name must be a non-empty string`)
    }
    if (entry.version !== null && typeof entry.version !== 'string') {
      problems.push(`${where}: version must be a string or null`)
    }
    if (typeof entry.provenance !== 'string' || entry.provenance === '') {
      problems.push(`${where}: provenance must be a non-empty string`)
    }
    if (entry.kind === 'upstream') {
      const source = entry.source
      if (source === null || typeof source !== 'object' || Array.isArray(source)) {
        problems.push(`${where}: upstream entries require a source object`)
      } else {
        if (typeof source.url !== 'string' || !source.url.startsWith('https://')) {
          problems.push(`${where}: source.url must be an https URL`)
        } else if (!isSafeArchiveName(source.url)) {
          problems.push(`${where}: source.url must end in a plain archive file name`)
        }
        if (
          typeof source.archive_sha256 !== 'string' ||
          !SHA256_PATTERN.test(source.archive_sha256)
        ) {
          problems.push(
            `${where}: source.archive_sha256 must be a 64-character lowercase hex digest`
          )
        }
        if (typeof source.member !== 'string' || !isSafeMember(source.member)) {
          problems.push(
            `${where}: source.member must be a relative forward-slash path without '..' segments`
          )
        }
      }
    } else if (entry.source !== undefined) {
      problems.push(`${where}: non-upstream entries must not carry a source`)
    }
  }
  return problems
}

/** Repository-relative paths of git-tracked files with a binary extension. */
export function listTrackedBinaries(root) {
  const output = execFileSync('git', ['ls-files', '-z'], {
    cwd: root,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  })
  return output.split('\0').filter((entry) => entry !== '' && BINARY_EXTENSION_PATTERN.test(entry))
}

/** Offline `check`: manifest schema, listed files and tracked-binary coverage. */
export function checkBinaries(root) {
  const { manifest, problems } = loadManifest(root)
  if (manifest === null) return { problems, count: 0 }
  problems.push(...validateManifest(manifest))
  const entries = Array.isArray(manifest.binaries)
    ? manifest.binaries.filter((entry) => entry !== null && typeof entry === 'object')
    : []
  for (const entry of entries) {
    if (typeof entry.path !== 'string' || entry.path === '') continue
    const filePath = resolve(root, entry.path)
    if (!existsSync(filePath)) {
      problems.push(`${entry.path}: listed in the manifest but missing on disk`)
      continue
    }
    const size = statSync(filePath).size
    if (Number.isInteger(entry.size) && size !== entry.size) {
      problems.push(`${entry.path}: size ${size} does not match the manifest ${entry.size}`)
    }
    const digest = sha256File(filePath)
    if (typeof entry.sha256 === 'string' && digest !== entry.sha256) {
      problems.push(`${entry.path}: sha256 ${digest} does not match the manifest ${entry.sha256}`)
    }
  }
  const listed = new Set(entries.map((entry) => entry.path))
  for (const tracked of listTrackedBinaries(root)) {
    if (!listed.has(tracked)) {
      problems.push(`${tracked}: tracked binary missing from ${MANIFEST_PATH}`)
    }
  }
  return { problems, count: entries.length }
}

/** Basename of an archive URL, percent-decoded. */
export function cacheFileName(url) {
  const name = decodeURIComponent(new URL(url).pathname.split('/').pop() ?? '')
  if (name === '') throw new Error(`cannot derive a file name from ${url}`)
  return name
}

async function resolveArchive(source, cacheDir, tempDirs) {
  const fileName = cacheFileName(source.url)
  if (cacheDir !== undefined) {
    const cached = join(cacheDir, fileName)
    if (existsSync(cached)) return cached
  }
  const downloadDir = mkdtempSync(join(tmpdir(), 'entropia-verify-binaries-dl-'))
  tempDirs.push(downloadDir)
  const target = join(downloadDir, fileName)
  let response
  try {
    response = await fetch(source.url, { signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MS) })
  } catch (error) {
    throw new Error(`download failed: ${error.message}`)
  }
  if (!response.ok) throw new Error(`download failed: HTTP ${response.status}`)
  writeFileSync(target, Buffer.from(await response.arrayBuffer()))
  return target
}

function extractArchive(archivePath, source, extractDir) {
  const pathname = new URL(source.url).pathname
  let command
  let args
  if (/\.(tgz|tar\.gz)$/i.test(pathname)) {
    command = 'tar'
    args = ['-xzf', archivePath, '-C', extractDir, source.member]
  } else if (/\.zip$/i.test(pathname)) {
    command = 'unzip'
    args = ['-o', '-q', archivePath, source.member, '-d', extractDir]
  } else {
    throw new Error(`unsupported archive type for ${source.url}; use .tgz, .tar.gz or .zip`)
  }
  const result = spawnSync(command, args, { encoding: 'utf8' })
  if (result.error !== undefined)
    throw new Error(`failed to run ${command}: ${result.error.message}`)
  if (result.status !== 0) {
    const detail = (result.stderr ?? '').trim()
    throw new Error(`${command} exited with ${result.status}${detail === '' ? '' : `: ${detail}`}`)
  }
}

/**
 * Downloads (or reuses from `cacheDir`) every upstream archive, verifies its
 * pinned archive_sha256 and compares the extracted member with the committed
 * digest. Returns the exit code: 0 all match, 1 any mismatch, 2 download/tool.
 */
export async function verifyUpstream(root, options = {}) {
  const { manifest, problems } = loadManifest(root)
  if (manifest === null) {
    for (const problem of problems) console.error(problem)
    return 2
  }
  const schemaProblems = validateManifest(manifest)
  if (schemaProblems.length > 0) {
    for (const problem of schemaProblems) console.error(problem)
    return 2
  }
  const tempDirs = []
  let exitCode = 0
  let toolFailure = false
  try {
    for (const entry of manifest.binaries.filter((candidate) => candidate.kind === 'upstream')) {
      try {
        const archivePath = await resolveArchive(entry.source, options.cacheDir, tempDirs)
        const archiveDigest = sha256File(archivePath)
        if (archiveDigest !== entry.source.archive_sha256) {
          console.error(
            `${entry.path}: archive sha256 ${archiveDigest} does not match the manifest ${entry.source.archive_sha256}`
          )
          exitCode = 1
          continue
        }
        const extractDir = mkdtempSync(join(tmpdir(), 'entropia-verify-binaries-'))
        tempDirs.push(extractDir)
        extractArchive(archivePath, entry.source, extractDir)
        const extractedPath = join(extractDir, entry.source.member)
        if (!existsSync(extractedPath)) {
          throw new Error(`member ${entry.source.member} not found after extraction`)
        }
        const extractedDigest = sha256File(extractedPath)
        if (extractedDigest !== entry.sha256) {
          console.error(
            `${entry.path}: extracted sha256 ${extractedDigest} does not match the committed ${entry.sha256}`
          )
          exitCode = 1
        } else {
          console.log(`${entry.path}: OK`)
        }
      } catch (error) {
        // Report it and keep going: one unreachable archive must not hide the
        // state of every other upstream binary.
        console.error(`${entry.path}: ${error.message}`)
        toolFailure = true
      }
    }
  } finally {
    for (const dir of tempDirs) rmSync(dir, { recursive: true, force: true })
  }
  return toolFailure ? 2 : exitCode
}

/** Parses the CLI arguments into `{ command, root, cacheDir }` or `{ error }`. */
export function parseArgs(argv) {
  const [command, ...rest] = argv
  const options = { command, root: undefined, cacheDir: undefined }
  for (let index = 0; index < rest.length; index += 1) {
    const flag = rest[index]
    if (flag !== '--root' && flag !== '--cache') return { error: `unknown argument: ${flag}` }
    const value = rest[index + 1]
    if (value === undefined || value.startsWith('--')) {
      return { error: `${flag} requires a directory value` }
    }
    if (flag === '--root') options.root = value
    else options.cacheDir = value
    index += 1
  }
  return options
}

export async function main(args = process.argv.slice(2)) {
  if (args.length === 0) {
    console.error(
      'usage: verify-binaries.mjs <check|verify-upstream> [--root <dir>] [--cache <dir>]'
    )
    return 2
  }
  const parsed = parseArgs(args)
  if (parsed.error !== undefined) {
    console.error(`verify-binaries: ${parsed.error}`)
    return 2
  }
  const root = resolve(parsed.root ?? defaultRoot)
  if (parsed.command === 'check') {
    const { problems, count } = checkBinaries(root)
    if (problems.length > 0) {
      for (const problem of problems) console.error(problem)
      return 1
    }
    console.log(`binaries: OK (${count} files)`)
    return 0
  }
  if (parsed.command === 'verify-upstream') {
    return verifyUpstream(root, { cacheDir: parsed.cacheDir })
  }
  console.error(`verify-binaries: unknown command: ${parsed.command}`)
  return 2
}

const invokedDirectly =
  process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href
if (invokedDirectly) {
  process.exitCode = await main()
}
