import { existsSync, readFileSync, readdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

// Lite on every platform is built with an extra overlay on top of
// tauri.lite.conf.json (.github/workflows/lite-preview.yml and release.yml):
//   Linux:   --config tauri.lite.conf.json --config tauri.lite.linux.conf.json
//   macOS:   --config tauri.lite.conf.json --config tauri.lite.macos.conf.json
//   Windows: --config tauri.lite.conf.json --config tauri.lite.windows.conf.json
// The Store MSIX repack reads none of them: it patches a captured base package.

const tauriDir = join(dirname(fileURLToPath(import.meta.url)), '../../src-tauri')

interface TauriConfig {
  productName?: string
  mainBinaryName?: string
  identifier?: string
  app?: unknown
  bundle?: {
    shortDescription?: string
    longDescription?: string
    resources?: string[]
    linux?: { deb?: { desktopTemplate?: string; conflicts?: string[]; replaces?: string[] } }
    macOS?: { frameworks?: string[] }
  }
}

function readConfig(name: string): TauriConfig {
  const path = join(tauriDir, name)
  expect(existsSync(path), `${name} must exist`).toBe(true)
  return JSON.parse(readFileSync(path, 'utf8')) as TauriConfig
}

// Kebab-case the way tauri-bundler names the Debian package (heck::AsKebabCase
// of productName): "EntropIA Lite" became "entrop-ia-lite".
function debPackageName(productName: string): string {
  return productName
    .replace(/([a-z0-9])([A-Z])/g, '$1-$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1-$2')
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .join('-')
    .toLowerCase()
}

// The paths a tauri-bundler .deb owns, per crates/tauri-bundler/src/bundle/linux/debian.rs
// and freedesktop/mod.rs (tauri-cli 2.10): the binary, the resource dir, the
// desktop entry, the icons and the changelog dir.
function debOwnedPaths(productName: string, binaryName: string): string[] {
  return [
    `/usr/bin/${binaryName}`,
    `/usr/lib/${productName}/`,
    `/usr/share/applications/${productName}.desktop`,
    `/usr/share/icons/hicolor/32x32/apps/${binaryName}.png`,
    `/usr/share/icons/hicolor/128x128/apps/${binaryName}.png`,
    `/usr/share/icons/hicolor/256x256@2/apps/${binaryName}.png`,
    `/usr/share/doc/${productName}/`,
  ]
}

describe('Lite Linux bundle identity (tauri.lite.linux.conf.json)', () => {
  const base = readConfig('tauri.conf.json')
  const linux = readConfig('tauri.linux.conf.json')
  const lite = readConfig('tauri.lite.conf.json')
  const liteLinux = readConfig('tauri.lite.linux.conf.json')

  it('names the package and the binary entropia-lite', () => {
    expect(debPackageName('EntropIA Lite')).toBe('entrop-ia-lite')
    expect(debPackageName(liteLinux.productName ?? '')).toBe('entropia-lite')
    expect(liteLinux.mainBinaryName).toBe('entropia-lite')
  })

  it('shares no installed path with the Pro .deb', () => {
    const proPaths = debOwnedPaths(base.productName ?? '', 'entropia-pro-desktop')
    const litePaths = debOwnedPaths(liteLinux.productName ?? '', liteLinux.mainBinaryName ?? '')
    for (const path of litePaths) {
      expect(proPaths).not.toContain(path)
    }
    expect(debPackageName(liteLinux.productName ?? '')).not.toBe(
      debPackageName(base.productName ?? '')
    )
  })

  it('describes Lite, not Pro', () => {
    const short = liteLinux.bundle?.shortDescription ?? ''
    const long = liteLinux.bundle?.longDescription ?? ''
    expect(short).not.toBe('')
    expect(long).toContain('EntropIA Lite')
    expect(`${short} ${long}`).not.toContain('EntropIA Pro')
    expect(`${short} ${long}`).not.toMatch(/run on local models|on-device/i)
  })

  it('shows "EntropIA Lite" in the launcher', () => {
    const template = liteLinux.bundle?.linux?.deb?.desktopTemplate ?? ''
    expect(template).not.toBe('')
    const desktop = readFileSync(join(tauriDir, template), 'utf8')
    expect(desktop).toMatch(/^Name=EntropIA Lite$/m)
    expect(desktop).toMatch(/^Exec=\{\{exec\}\}$/m)
    expect(desktop).toMatch(/^Icon=\{\{icon\}\}$/m)
  })

  it('keeps every Linux resource and adds the pinned Pdfium library', () => {
    const resources = liteLinux.bundle?.resources ?? []
    for (const resource of linux.bundle?.resources ?? []) {
      expect(resources).toContain(resource)
    }
    expect(resources).toContain('resources/pdfium/libpdfium.so')
  })

  it('replaces the package the 1.0.5 .deb installed under the old name', () => {
    // Up to 1.0.5 the package was "entrop-ia-lite"; apt keeps it beside
    // "entropia-lite" unless the new one conflicts with and replaces it.
    expect(liteLinux.bundle?.linux?.deb?.conflicts).toContain('entrop-ia-lite')
    expect(liteLinux.bundle?.linux?.deb?.replaces).toContain('entrop-ia-lite')
  })

  it('leaves the app identity and windows to tauri.lite.conf.json', () => {
    expect(liteLinux.identifier).toBeUndefined()
    expect(liteLinux.app).toBeUndefined()
    expect(lite.identifier).toBe('com.entropia.lite')
  })
})

describe('Lite macOS bundle (tauri.lite.macos.conf.json)', () => {
  const liteMac = readConfig('tauri.lite.macos.conf.json')

  it('ships the pinned Pdfium library in Contents/Frameworks', () => {
    expect(liteMac.bundle?.macOS?.frameworks).toContain('resources/pdfium/libpdfium.dylib')
  })

  it('does not change the product identity', () => {
    expect(liteMac.productName).toBeUndefined()
    expect(liteMac.identifier).toBeUndefined()
    expect(liteMac.app).toBeUndefined()
  })
})

describe('Lite Windows bundle (tauri.lite.windows.conf.json)', () => {
  const windows = readConfig('tauri.windows.conf.json')
  const liteWindows = readConfig('tauri.lite.windows.conf.json')
  const resources = liteWindows.bundle?.resources ?? []

  it('ships what Lite reads at runtime, and the VC runtime the NSIS hook installs', () => {
    // provider-compatibility.json: transcription/assemblyai/config.rs (not feature-gated).
    // pdfium.dll: ocr/pdf.rs, Lite's only copy. vc-runtime: windows/hooks.nsh.
    for (const resource of [
      'resources/provider-compatibility.json',
      'resources/lib/pdfium.dll',
      'resources/lib/LICENSE',
      'resources/fonts/LICENSE',
      'target/release/vc-runtime/*',
    ]) {
      expect(resources).toContain(resource)
    }
  })

  it('leaves out the local-ml payload: uv, models, runtime pack and Python scripts', () => {
    for (const resource of resources) {
      expect(resource).not.toMatch(/tools\/uv|models\/|runtime-pack|^scripts\//)
    }
  })

  it('only narrows the Windows resource list, never adds to it', () => {
    for (const resource of resources) {
      expect(windows.bundle?.resources).toContain(resource)
    }
  })

  it('clears what an older install left behind before copying the new files', () => {
    // The NSIS installer overwrites files but never removes the ones a newer
    // version stopped shipping: 1.0.17 over 1.0.16 kept 100 MB of uv and models.
    // The pre-install hook runs for Pro too, which copies them right back.
    const hook = readFileSync(join(tauriDir, 'windows/hooks.nsh'), 'utf8')
    const preinstall = hook.match(/!macro NSIS_HOOK_PREINSTALL([\s\S]*?)!macroend/)?.[1] ?? ''
    const removed = [...preinstall.matchAll(/(?:RMDir \/r|Delete) "\$INSTDIR\\([^"]+)"/g)].map(
      (m) => m[1]
    )
    const dropped = (windows.bundle?.resources ?? []).filter((r) => !resources.includes(r))
    expect(dropped.length).toBeGreaterThan(0)
    for (const resource of dropped) {
      const path = resource.replace(/\/\*\*\/\*$|\/\*$/, '').replace(/\//g, '\\')
      expect(
        removed.some((target) => path === target || path.startsWith(`${target}\\`)),
        `${resource} is not cleaned up by NSIS_HOOK_PREINSTALL`
      ).toBe(true)
    }
  })

  it('does not change the product identity', () => {
    expect(liteWindows.productName).toBeUndefined()
    expect(liteWindows.identifier).toBeUndefined()
    expect(liteWindows.app).toBeUndefined()
  })

  it('is passed to the Windows Lite release build', () => {
    const release = readFileSync(join(tauriDir, '../../../.github/workflows/release.yml'), 'utf8')
    expect(release).toContain(
      '--config apps/desktop/src-tauri/tauri.lite.conf.json --config apps/desktop/src-tauri/tauri.lite.windows.conf.json --bundles nsis,msi'
    )
  })
})

describe('Lite overlays', () => {
  it('keeps the Windows Lite name and binary as shipped', () => {
    const lite = readConfig('tauri.lite.conf.json')
    expect(lite.productName).toBe('EntropIA Lite')
    expect(lite.mainBinaryName).toBeUndefined()
  })

  it('names the overlays so Tauri never auto-merges them on any platform', () => {
    // Tauri merges tauri.<platform>.conf.json on its own; the Lite overlays must
    // only ever apply when passed explicitly with --config.
    const overlays = readdirSync(tauriDir).filter((f) => /^tauri\.lite\.\w+\.conf\.json$/.test(f))
    expect(overlays.sort()).toEqual([
      'tauri.lite.linux.conf.json',
      'tauri.lite.macos.conf.json',
      'tauri.lite.windows.conf.json',
    ])
    for (const overlay of overlays) {
      expect(overlay).not.toMatch(/^tauri\.(windows|linux|macos)\.conf\.json$/)
    }
  })
})
