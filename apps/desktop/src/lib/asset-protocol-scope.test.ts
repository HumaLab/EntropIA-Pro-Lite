import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * The asset protocol serves exactly the archive subdirectories the webview
 * loads files from. The three Tauri configs declare that scope statically and
 * `src-tauri/src/asset_scope.rs` grants the same subdirectories at runtime from
 * the resolved data and cache directories. This test keeps the copies in step:
 * no config may drift to a whole-directory scope again, and neither the static
 * configs nor the runtime list may name a subdirectory the other does not.
 */

const currentDir = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(currentDir, '../../../..')
const tauriRoot = resolve(repoRoot, 'apps/desktop/src-tauri')

const CONFIGS = ['tauri.conf.json', 'tauri.lite.conf.json', 'tauri.dev.conf.json'] as const

/** Platform overlays that must not re-declare the scope. */
const PLATFORM_OVERLAYS = [
  'tauri.linux.conf.json',
  'tauri.windows.conf.json',
  'tauri.lite.linux.conf.json',
  'tauri.lite.macos.conf.json',
  'tauri.lite.windows.conf.json',
] as const

interface AssetProtocolScope {
  allow: string[]
  deny: string[]
}

interface TauriConfig {
  app?: {
    security?: {
      assetProtocol?: {
        enable?: boolean
        scope?: AssetProtocolScope
      }
    }
  }
}

function readTauriConfig(file: string): TauriConfig {
  return JSON.parse(readFileSync(resolve(tauriRoot, file), 'utf8')) as TauriConfig
}

function assetScope(file: string): AssetProtocolScope {
  const scope = readTauriConfig(file).app?.security?.assetProtocol?.scope
  expect(scope, `${file} declares an assetProtocol scope`).toBeDefined()
  return scope as AssetProtocolScope
}

/** The `&str` values of one `pub const NAME: [&str; N]` in `asset_scope.rs`. */
function rustSubdirs(source: string, name: string): string[] {
  const match = new RegExp(`pub const ${name}: \\[&str; \\d+\\] =\\s*\\[([^\\]]*)]`).exec(source)
  expect(match, `${name} is declared in asset_scope.rs`).not.toBeNull()
  return (match?.[1] ?? '')
    .split(',')
    .map((entry) => entry.trim().replace(/^"|"$/g, ''))
    .filter((entry) => entry.length > 0)
}

describe('asset protocol scope', () => {
  it('declares the same scope in the base, Lite and dev configs', () => {
    const [base, lite, dev] = CONFIGS.map(assetScope)
    expect(lite, 'tauri.lite.conf.json drifted from tauri.conf.json').toEqual(base)
    expect(dev, 'tauri.dev.conf.json drifted from tauri.conf.json').toEqual(base)
  })

  it('allows only the served archive subdirectories and denies the database and capture HTML', () => {
    const scope = assetScope('tauri.conf.json')
    expect(scope.allow).toEqual([
      '$DATA/com.entropia.shared/assets/**',
      '$DATA/com.entropia.shared/writing-images/**',
      '$DATA/com.entropia.shared/writing-crops/**',
      '$DATA/com.entropia.shared/web-captures/**',
      '$LOCALDATA/com.entropia.shared/thumbnails/**',
    ])
    expect(scope.deny).toEqual([
      '$DATA/com.entropia.shared/**/*.sqlite*',
      '$LOCALDATA/com.entropia.shared/**/*.sqlite*',
      '$DATA/com.entropia.shared/**/web-captures/**/*.html',
    ])
  })

  it('keeps the runtime grant in asset_scope.rs aligned with the static scope', () => {
    const source = readFileSync(resolve(tauriRoot, 'src/asset_scope.rs'), 'utf8')
    const expected = [
      ...rustSubdirs(source, 'ASSET_DATA_DIRS').map(
        (dir) => `$DATA/com.entropia.shared/${dir}/**`
      ),
      ...rustSubdirs(source, 'ASSET_CACHE_DIRS').map(
        (dir) => `$LOCALDATA/com.entropia.shared/${dir}/**`
      ),
    ]
    expect([...assetScope('tauri.conf.json').allow].sort()).toEqual(expected.sort())
  })

  it('leaves the platform overlays without an assetProtocol block', () => {
    for (const file of PLATFORM_OVERLAYS) {
      expect(
        readTauriConfig(file).app?.security?.assetProtocol,
        `${file} must not re-declare the asset protocol scope`
      ).toBeUndefined()
    }
  })
})
