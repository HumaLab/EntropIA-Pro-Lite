// Single source of truth for the app version (audit finding C-04). The version
// is hand-edited ONLY in apps/desktop/package.json and mirrored, derived, in:
//
//   1. apps/desktop/src-tauri/Cargo.toml            ([package] version)
//   2. apps/desktop/src-tauri/Cargo.lock            (entropia-pro-desktop entry only)
//   3. apps/desktop/src-tauri/tauri.conf.json       (version)
//   4. apps/desktop/src-tauri/tauri.lite.conf.json  (version)
//   5. resources/runtime-pack/windows-x86_64/manifest.json (app_version)
//   6. resources/runtime-pack/linux-x86_64/manifest.json   (app_version)
//   7. scripts/repack-store-msix.ps1                (StoreVersion help + default, x.y.z.0)
//   8. msix/README.md                               (documented Store default, x.y.z.0)
//
// `.github/workflows/release.yml` keeps no version literal: on a `v*` tag it runs
// `check --tag` and derives the 4-segment Store version with `print --store`.
//
// Usage:
//   node apps/desktop/scripts/version.mjs print [--store] [--root <dir>]
//   node apps/desktop/scripts/version.mjs check [--tag <ref>] [--root <dir>]
//   node apps/desktop/scripts/version.mjs bump <x.y.z> [--root <dir>]
//
// `--root` defaults to the repository root resolved from this script location;
// tests point it at an isolated copy. Exit codes: 0 = consistent, 1 = mismatch,
// invalid version, or a declaration that cannot be rewritten.

import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

/** The one hand-edited declaration; every other value derives from it. */
export const SOURCE_PATH = 'apps/desktop/package.json'

const STORE_SUFFIX = '.0'

/**
 * Derived declarations. Each `pattern` is anchored to the exact version token
 * and captures three groups (prefix, value, suffix) so a bump is a targeted
 * text replacement, never a JSON re-serialization. `form` says which token the
 * file carries: the plain `x.y.z` or the 4-segment Store `x.y.z.0`.
 */
export const DECLARATIONS = [
  {
    path: 'apps/desktop/package.json',
    form: 'semver',
    patterns: [/^(\s*"version":\s*")(?<value>[^"]+)(")/m],
  },
  {
    path: 'apps/desktop/src-tauri/Cargo.toml',
    form: 'semver',
    patterns: [/(\[package\][^[]*?^version\s*=\s*")(?<value>[^"]+)(")/m],
  },
  {
    // Only the entropia-pro-desktop package entry: other crates (dyn-clone,
    // ...) may legitimately share the same x.y.z or x.y.z.0 value.
    path: 'apps/desktop/src-tauri/Cargo.lock',
    form: 'semver',
    patterns: [/(^name = "entropia-pro-desktop"\r?\nversion = ")(?<value>[^"]+)(")/m],
  },
  {
    path: 'apps/desktop/src-tauri/tauri.conf.json',
    form: 'semver',
    patterns: [/^(\s*"version":\s*")(?<value>[^"]+)(")/m],
  },
  {
    path: 'apps/desktop/src-tauri/tauri.lite.conf.json',
    form: 'semver',
    patterns: [/^(\s*"version":\s*")(?<value>[^"]+)(")/m],
  },
  {
    path: 'apps/desktop/src-tauri/resources/runtime-pack/windows-x86_64/manifest.json',
    form: 'semver',
    patterns: [/^(\s*"app_version":\s*")(?<value>[^"]+)(")/m],
  },
  {
    path: 'apps/desktop/src-tauri/resources/runtime-pack/linux-x86_64/manifest.json',
    form: 'semver',
    patterns: [/^(\s*"app_version":\s*")(?<value>[^"]+)(")/m],
  },
  {
    path: 'apps/desktop/src-tauri/scripts/repack-store-msix.ps1',
    form: 'store',
    patterns: [
      /(Defaults to )(?<value>\d+\.\d+\.\d+\.0)(\.)/,
      /(\[string\]\$StoreVersion\s*=\s*")(?<value>\d+\.\d+\.\d+\.0)(")/,
    ],
  },
  {
    path: 'apps/desktop/src-tauri/msix/README.md',
    form: 'store',
    patterns: [/(\(default `)(?<value>\d+\.\d+\.\d+\.0)(`)/],
  },
]

const USAGE = [
  'usage:',
  '  node apps/desktop/scripts/version.mjs print [--store] [--root <dir>]',
  '  node apps/desktop/scripts/version.mjs check [--tag <ref>] [--root <dir>]',
  '  node apps/desktop/scripts/version.mjs bump <x.y.z> [--root <dir>]',
].join('\n')

/** Values the declaration's patterns currently extract (null when unmatched). */
export function extractValues(declaration, text) {
  return declaration.patterns.map((pattern) => pattern.exec(text)?.groups?.value ?? null)
}

/** Reads the source-of-truth version from `<root>/apps/desktop/package.json`. */
export function readSourceVersion(root) {
  const text = readFileSync(resolve(root, SOURCE_PATH), 'utf8')
  const [value] = extractValues(DECLARATIONS[0], text)
  if (!value) throw new Error(`no version found in ${SOURCE_PATH}`)
  return value
}

/** Reads every declaration (path, form, patterns, current text) under `root`. */
export function readDeclarations(root) {
  return DECLARATIONS.map((declaration) => ({
    ...declaration,
    text: readFileSync(resolve(root, declaration.path), 'utf8'),
  }))
}

/** Expected token for a declaration form: plain or 4-segment Store version. */
function expectedToken(declaration, version) {
  return declaration.form === 'store' ? `${version}${STORE_SUFFIX}` : version
}

/**
 * Pure: returns one `file: found X, expected Y` line per inconsistent
 * declaration, plus a tag line when `--tag` disagrees with `v<version>`.
 */
export function checkDeclarations(declarations, version, tag) {
  const mismatches = []
  for (const declaration of declarations) {
    const expected = expectedToken(declaration, version)
    const found = extractValues(declaration, declaration.text)
    if (found.includes(null) || found.some((value) => value !== expected)) {
      const distinct = [...new Set(found.map((value) => value ?? '<missing>'))]
      mismatches.push(`${declaration.path}: found ${distinct.join(', ')}, expected ${expected}`)
    }
  }
  if (tag !== undefined && tag !== `v${version}`) {
    mismatches.push(`tag ${tag}: found ${tag}, expected v${version}`)
  }
  return mismatches
}

/** Pure: targeted text replacement of every version token in `text`. */
export function bumpText(text, declaration, nextVersion) {
  const token = expectedToken(declaration, nextVersion)
  let next = text
  for (const pattern of declaration.patterns) {
    next = next.replace(pattern, (_match, prefix, _value, suffix) => `${prefix}${token}${suffix}`)
  }
  return next
}

function parseArgs(args, { allowTag = false, allowStore = false } = {}) {
  const options = {}
  const positionals = []
  for (let i = 0; i < args.length; i++) {
    const arg = args[i]
    if (arg === '--root') {
      if (i + 1 >= args.length) throw new Error(`missing value for --root\n${USAGE}`)
      options.root = args[++i]
    } else if (allowTag && arg === '--tag') {
      if (i + 1 >= args.length) throw new Error(`missing value for --tag\n${USAGE}`)
      options.tag = args[++i]
    } else if (allowStore && arg === '--store') {
      options.store = true
    } else if (arg.startsWith('--')) {
      throw new Error(`unknown argument: ${arg}\n${USAGE}`)
    } else {
      positionals.push(arg)
    }
  }
  options.root = resolve(
    options.root ?? resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
  )
  return { options, positionals }
}

function printMismatches(mismatches) {
  for (const line of mismatches) console.error(line)
  console.error(`${mismatches.length} version mismatch(es); fix with "pnpm version:bump <x.y.z>".`)
}

function runPrint(args) {
  const { options } = parseArgs(args, { allowStore: true })
  const version = readSourceVersion(options.root)
  console.log(options.store ? `${version}${STORE_SUFFIX}` : version)
  return 0
}

function runCheck(args) {
  const { options } = parseArgs(args, { allowTag: true })
  const version = readSourceVersion(options.root)
  const mismatches = checkDeclarations(readDeclarations(options.root), version, options.tag)
  if (mismatches.length > 0) {
    printMismatches(mismatches)
    return 1
  }
  console.log(`version-consistency: OK (${version})`)
  return 0
}

function runBump(args) {
  const { options, positionals } = parseArgs(args)
  const nextVersion = positionals[0]
  if (positionals.length !== 1 || !nextVersion) {
    throw new Error(`bump needs exactly one <x.y.z> version\n${USAGE}`)
  }
  if (!/^\d+\.\d+\.\d+$/.test(nextVersion)) {
    console.error(`invalid version "${nextVersion}": expected strict x.y.z (e.g. 1.2.3)`)
    return 1
  }

  // Validate every rewrite before touching the tree: one bad declaration must
  // leave the repository exactly as it was.
  const plans = []
  for (const declaration of readDeclarations(options.root)) {
    const expected = expectedToken(declaration, nextVersion)
    const text = bumpText(declaration.text, declaration, nextVersion)
    const values = extractValues(declaration, text)
    if (text === declaration.text || values.includes(null) || values.some((v) => v !== expected)) {
      console.error(`cannot rewrite ${declaration.path}: version pattern not found or ambiguous`)
      return 1
    }
    plans.push({ path: declaration.path, text })
  }

  for (const plan of plans) writeFileSync(resolve(options.root, plan.path), plan.text)

  const mismatches = checkDeclarations(readDeclarations(options.root), nextVersion)
  if (mismatches.length > 0) {
    printMismatches(mismatches)
    return 1
  }
  console.log(`version: bumped to ${nextVersion} (Store ${nextVersion}${STORE_SUFFIX})`)
  return 0
}

/** CLI entry point; exported so tests can reason about command dispatch. */
export function main(args = process.argv.slice(2)) {
  const [command, ...rest] = args
  try {
    if (command === 'print') return runPrint(rest)
    if (command === 'check') return runCheck(rest)
    if (command === 'bump') return runBump(rest)
    throw new Error(`unknown command: ${command ?? '(none)'}\n${USAGE}`)
  } catch (error) {
    console.error(`version: ${error instanceof Error ? error.message : String(error)}`)
    return 1
  }
}

const invokedDirectly =
  process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href
if (invokedDirectly) {
  process.exitCode = main()
}
