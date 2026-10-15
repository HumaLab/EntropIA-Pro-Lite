// Release gate for the Pro bootstrap runtime. At first launch the app resolves
// its runtime against the signed manifest with an EXACT (app_version, platform)
// match (runtime/manifest.rs `select_release`), so a release whose version is
// absent from that manifest would ship an installer that can never download its
// runtime on a clean machine. This script fails the Release workflow before the
// expensive build instead.
//
// Usage:
//   node apps/desktop/scripts/check-runtime-bootstrap.mjs \
//     --platform <windows-x86_64|linux-x86_64> [--app-version <x.y.z>] \
//     (--manifest-url <url> | --manifest-file <path>) [--timeout-ms <ms>]
//
// Exit codes: 0 = matching release found; 1 = manifest has no matching release;
// 2 = bad usage, fetch error or timeout, or unreadable/invalid manifest.

import { readFileSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const packageJsonPath = resolve(here, '../package.json')

const USAGE =
  'usage: node apps/desktop/scripts/check-runtime-bootstrap.mjs ' +
  '--platform <windows-x86_64|linux-x86_64> [--app-version <x.y.z>] ' +
  '(--manifest-url <url> | --manifest-file <path>) [--timeout-ms <ms>]'

const OPTION_KEYS = {
  '--platform': 'platform',
  '--app-version': 'appVersion',
  '--manifest-url': 'manifestUrl',
  '--manifest-file': 'manifestFile',
  '--timeout-ms': 'timeoutMs',
}

// Bounds the whole manifest fetch, body included. A hung release host must
// fail the gate quickly instead of stalling the job until its own timeout.
const DEFAULT_TIMEOUT_MS = 30_000

/**
 * Pure decision: the release the app would pick, or `undefined`. Mirrors the
 * exact-match lookup in runtime/manifest.rs `select_release`.
 */
export function findRuntimeRelease(index, appVersion, platform) {
  if (!Array.isArray(index?.releases)) return undefined
  return index.releases.find(
    (release) => release.app_version === appVersion && release.platform === platform
  )
}

/** `--app-version` default: the version of the desktop app being released. */
function defaultAppVersion() {
  const { version } = JSON.parse(readFileSync(packageJsonPath, 'utf8'))
  if (typeof version !== 'string' || version.length === 0) {
    throw new Error(`no version found in ${packageJsonPath}`)
  }
  return version
}

function parseArgs(args) {
  const options = {}
  for (let i = 0; i < args.length; i++) {
    const key = OPTION_KEYS[args[i]]
    if (key === undefined) throw new Error(`unknown argument: ${args[i]}\n${USAGE}`)
    if (i + 1 >= args.length) throw new Error(`missing value for ${args[i]}\n${USAGE}`)
    options[key] = args[++i]
  }
  if (!options.platform) throw new Error(`missing required --platform\n${USAGE}`)
  if (Boolean(options.manifestUrl) === Boolean(options.manifestFile)) {
    throw new Error(`pass exactly one of --manifest-url or --manifest-file\n${USAGE}`)
  }
  if (!options.appVersion) options.appVersion = defaultAppVersion()
  if (options.timeoutMs === undefined) {
    options.timeoutMs = DEFAULT_TIMEOUT_MS
  } else {
    const timeoutMs = Number(options.timeoutMs)
    if (!Number.isInteger(timeoutMs) || timeoutMs <= 0) {
      throw new Error(`--timeout-ms must be a positive integer, got ${options.timeoutMs}`)
    }
    options.timeoutMs = timeoutMs
  }
  return options
}

/** Reads the manifest from disk or over HTTP (redirects followed) and parses it. */
async function loadIndex({ manifestUrl, manifestFile, timeoutMs }) {
  let raw
  if (manifestFile) {
    raw = await readFile(manifestFile, 'utf8')
  } else {
    try {
      const signal = AbortSignal.timeout(timeoutMs)
      const response = await fetch(manifestUrl, { redirect: 'follow', signal })
      if (!response.ok) {
        throw new Error(
          `manifest URL responded with HTTP ${response.status} ${response.statusText}`
        )
      }
      raw = await response.text()
    } catch (error) {
      if (error instanceof Error && error.name === 'TimeoutError') {
        throw new Error(`manifest URL timed out after ${timeoutMs} ms`)
      }
      throw error
    }
  }

  let index
  try {
    index = JSON.parse(raw)
  } catch {
    throw new Error('manifest is not valid JSON')
  }
  if (!Array.isArray(index?.releases)) {
    throw new Error('manifest is invalid: "releases" is not an array')
  }
  return index
}

function missingReleaseMessage(index, { appVersion, platform }) {
  const pairs = index.releases.map(
    (release) => `${release.app_version ?? '?'} / ${release.platform ?? '?'}`
  )
  const published = pairs.length > 0 ? pairs.map((pair) => `  ${pair}`).join('\n') : '  (none)'
  return [
    `runtime bootstrap: no published release matches app ${appVersion} on ${platform}`,
    '',
    'Published app_version / platform pairs:',
    published,
    '',
    `The app resolves its runtime with an exact (app_version, platform) match, so this Pro`,
    `installer could not download its runtime on a clean machine. Before tagging, run`,
    `"Build Runtime Pack" for ${appVersion} and "Publish Runtime Bootstrap" for this version`,
    `and platform so the signed manifest contains (${appVersion}, ${platform}).`,
  ].join('\n')
}

async function main() {
  const fail = (message) => {
    console.error(`runtime bootstrap: ${message}`)
    return 2
  }
  const reason = (error) => (error instanceof Error ? error.message : String(error))

  let options
  try {
    options = parseArgs(process.argv.slice(2))
  } catch (error) {
    return fail(reason(error))
  }

  let index
  try {
    index = await loadIndex(options)
  } catch (error) {
    return fail(reason(error))
  }

  const release = findRuntimeRelease(index, options.appVersion, options.platform)
  if (!release) {
    console.error(missingReleaseMessage(index, options))
    return 1
  }

  console.log(
    `runtime bootstrap: found ${options.platform} ${options.appVersion} (pack ${release.pack_version})`
  )
  return 0
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = await main()
}
