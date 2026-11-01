// Build the assistant connector and put it where Tauri expects a sidecar (D15).
//
// The connector must never live in tauri.conf.json: tauri-build copies every
// `bundle.externalBin` during ANY `cargo build`, `cargo test` or `cargo clippy`
// of the app crate and fails when the file is missing, so a plain developer
// build would start depending on a compiled connector. Instead it is declared in
// the explicit overlay `src-tauri/tauri.connector.conf.json`, which only the
// installer builds pass, and this script produces the file that overlay names:
//
//   apps/desktop/src-tauri/binaries/entropia-agent-connector-<triple>[.exe]      (Lite)
//   apps/desktop/src-tauri/binaries/entropia-pro-agent-connector-<triple>[.exe]  (Pro)
//
// Pro gets its own name (D16): the Lite and Pro .deb packages both install into
// /usr/bin and can be installed together, so one shared file name would make
// dpkg refuse the second package. Pro builds pass --variant pro and the
// tauri.connector.pro.conf.json overlay.
//
// A universal macOS bundle needs both Apple arches and a lipo'd universal file,
// because the bundler compiles each arch (its build script copies that arch's
// sidecar) and then also looks for the universal name.
//
// Usage:
//   node apps/desktop/scripts/build-agent-connector.mjs [--target <triple>] [--variant lite|pro]
//
// `--target` defaults to the host triple from `rustc -vV`. Exit codes: 0 = the
// sidecars are in place (their paths are printed, one per line), 1 = a cargo
// build, lipo, or the destination check failed.

import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

/** The binary name, matching the crate's `[[bin]]` default. */
export const CONNECTOR_STEM = 'entropia-agent-connector'

/** The installed name per app variant (D16); Lite keeps the crate's name. */
const SIDECAR_STEMS = { lite: CONNECTOR_STEM, pro: 'entropia-pro-agent-connector' }

/** The sidecar crate: a standalone workspace member with its own Cargo.lock. */
export const CRATE_MANIFEST = 'crates/entropia-agent-connector/Cargo.toml'

/** Where tauri-build looks for `binaries/entropia-agent-connector-<triple>`. */
export const SIDECAR_DIR = 'apps/desktop/src-tauri/binaries'

/** The pseudo-triple `tauri build --target universal-apple-darwin` uses. */
export const UNIVERSAL_TRIPLE = 'universal-apple-darwin'

/** The two arches that make up a universal macOS binary. */
const APPLE_ARCHES = ['aarch64-apple-darwin', 'x86_64-apple-darwin']

const here = dirname(fileURLToPath(import.meta.url))
const defaultRoot = resolve(here, '../../..')

const USAGE = [
  'usage:',
  '  node apps/desktop/scripts/build-agent-connector.mjs [--target <triple>] [--variant lite|pro]',
].join('\n')

/** The `host:` line of `rustc -vV`, which is the default target triple. */
export function parseRustcHost(output) {
  const line = output
    .split(/\r?\n/)
    .map((entry) => entry.trim())
    .find((entry) => entry.startsWith('host:'))
  if (!line) {
    throw new Error(`rustc -vV printed no host triple:\n${output.trim()}`)
  }
  return line.slice('host:'.length).trim()
}

export function isUniversalTriple(triple) {
  return triple === UNIVERSAL_TRIPLE
}

/** The OS segment of a target triple: `<arch>-<vendor>-<os>[-<env>]`. */
function targetOs(triple) {
  return triple.split('-')[2] ?? ''
}

export function isWindowsTriple(triple) {
  return targetOs(triple) === 'windows'
}

/** Cargo names the artifact after the crate, with the platform's suffix. */
export function plainExecutableName(triple) {
  return `${CONNECTOR_STEM}${isWindowsTriple(triple) ? '.exe' : ''}`
}

/** The sidecar stem the app variant's overlay names in `bundle.externalBin`. */
export function sidecarStem(variant) {
  const stem = SIDECAR_STEMS[variant]
  if (!stem) throw new Error(`--variant must be lite or pro (got ${String(variant)})`)
  return stem
}

/** The path tauri-build copies into the bundle for this triple. */
export function sidecarPath(root, triple, variant = 'lite') {
  const suffix = isWindowsTriple(triple) ? '.exe' : ''
  return join(root, SIDECAR_DIR, `${sidecarStem(variant)}-${triple}${suffix}`)
}

/** The universal macOS sidecar, which the bundler lipos the per-arch ones into. */
export function universalSidecarPath(root, variant = 'lite') {
  return sidecarPath(root, UNIVERSAL_TRIPLE, variant)
}

/** The triples to compile for the requested one. */
export function buildTriples(triple) {
  return isUniversalTriple(triple) ? [...APPLE_ARCHES] : [triple]
}

/** Where cargo leaves the release binary for an explicitly requested target. */
export function cargoArtifactPath(root, triple) {
  return join(
    root,
    'crates/entropia-agent-connector/target',
    triple,
    'release',
    plainExecutableName(triple)
  )
}

/** The exact command D15 asks for: release, locked, one target. */
export function cargoBuildArgs(triple) {
  return ['build', '--release', '--locked', '--manifest-path', CRATE_MANIFEST, '--target', triple]
}

/** The `--target` value, when the caller gave one. */
function requestedTriple(args) {
  const index = args.indexOf('--target')
  if (index === -1) return null
  const value = args[index + 1]
  if (!value) throw new Error(`--target needs a triple\n${USAGE}`)
  return value
}

/** The `--variant` value: `lite` when absent, an error for anything else. */
export function parseVariant(args) {
  const index = args.indexOf('--variant')
  if (index === -1) return 'lite'
  const value = args[index + 1]
  if (value !== 'lite' && value !== 'pro') {
    throw new Error(`--variant must be lite or pro (got ${String(value)})\n${USAGE}`)
  }
  return value
}

function run(command, args, cwd) {
  execFileSync(command, args, { cwd, stdio: 'inherit' })
}

/**
 * Build every triple the requested target needs and place the sidecars.
 * Returns the absolute paths it wrote, in build order.
 */
export function buildSidecars(triple, { root = defaultRoot, variant = 'lite' } = {}) {
  mkdirSync(join(root, SIDECAR_DIR), { recursive: true })

  const written = []
  for (const arch of buildTriples(triple)) {
    console.error(`[connector] cargo build --release --locked --target ${arch}`)
    run('cargo', cargoBuildArgs(arch), root)

    const artifact = cargoArtifactPath(root, arch)
    if (!existsSync(artifact)) {
      throw new Error(`cargo reported success but ${artifact} is missing`)
    }
    const destination = sidecarPath(root, arch, variant)
    copyFileSync(artifact, destination)
    written.push(destination)
  }

  if (isUniversalTriple(triple)) {
    const universal = universalSidecarPath(root, variant)
    console.error(`[connector] lipo -create ${APPLE_ARCHES.join(' ')} -output ${universal}`)
    run(
      'lipo',
      [
        '-create',
        ...buildTriples(triple).map((arch) => sidecarPath(root, arch, variant)),
        '-output',
        universal,
      ],
      root
    )
    written.push(universal)
  }

  return written
}

/** CLI entry point; exported so tests can reason about argument parsing. */
export function main(args = process.argv.slice(2), options = {}) {
  try {
    const variant = parseVariant(args)
    const triple =
      requestedTriple(args) ?? parseRustcHost(execFileSync('rustc', ['-vV'], { encoding: 'utf8' }))
    for (const path of buildSidecars(triple, { variant, ...options })) console.log(path)
    return 0
  } catch (error) {
    console.error(`connector: ${error instanceof Error ? error.message : String(error)}`)
    return 1
  }
}

const invokedDirectly =
  process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href
if (invokedDirectly) {
  process.exitCode = main()
}
