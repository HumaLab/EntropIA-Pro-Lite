import { describe, expect, it } from 'vitest'
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const currentDir = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(currentDir, '../../../..')
const tauriRoot = resolve(repoRoot, 'apps/desktop/src-tauri')

interface ManifestEntry {
  path: string
  sha256: string
  size: number
  executable?: boolean
}

interface RuntimePackFixtureManifest {
  pack_version: string
  app_version: string
  platform: string
  payload_profile: string
  release_injection_required: boolean
  external_artifacts_required: string[]
  python_relpath: string
  uv_relpath: string
  python_files: ManifestEntry[]
  uv_files: ManifestEntry[]
  script_files: ManifestEntry[]
  wheelhouse: ManifestEntry[]
  caches: ManifestEntry[]
  native_assets: ManifestEntry[]
}

function readDesktopVersion(): string {
  const tauriConfig = JSON.parse(readRepoFile('apps/desktop/src-tauri/tauri.conf.json')) as {
    version: string
  }
  const packageJson = JSON.parse(readRepoFile('apps/desktop/package.json')) as { version: string }

  expect(packageJson.version).toBe(tauriConfig.version)
  return tauriConfig.version
}

function readRepoFile(...segments: string[]): string {
  return readFileSync(resolve(repoRoot, ...segments), 'utf8')
}

function readPackManifest(platform: string): RuntimePackFixtureManifest {
  return JSON.parse(
    readRepoFile('apps/desktop/src-tauri/resources/runtime-pack', platform, 'manifest.json')
  ) as RuntimePackFixtureManifest
}

/** Shared libraries under `dir` smaller than 1 MiB: stubs `dlopen` rejects. */
function findTinySharedLibraries(dir: string): string[] {
  if (!existsSync(dir)) return []
  const tiny: string[] = []
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = resolve(dir, entry.name)
    if (entry.isDirectory()) {
      tiny.push(...findTinySharedLibraries(full))
    } else if (entry.isFile() && entry.name.endsWith('.so') && statSync(full).size < 1024 * 1024) {
      tiny.push(full)
    }
  }
  return tiny
}

function allEntries(manifest: RuntimePackFixtureManifest): ManifestEntry[] {
  return [
    ...manifest.python_files,
    ...manifest.uv_files,
    ...manifest.script_files,
    ...manifest.wheelhouse,
    ...manifest.caches,
    ...manifest.native_assets,
  ]
}

describe('runtime pack packaging', () => {
  it('isolates platform-specific bundle resources', () => {
    const baseConfig = JSON.parse(readRepoFile('apps/desktop/src-tauri/tauri.conf.json')) as {
      bundle?: { resources?: string[] }
    }
    const windowsConfig = JSON.parse(
      readRepoFile('apps/desktop/src-tauri/tauri.windows.conf.json')
    ) as { bundle?: { resources?: string[]; windows?: object } }
    const linuxConfig = JSON.parse(
      readRepoFile('apps/desktop/src-tauri/tauri.linux.conf.json')
    ) as { bundle?: { resources?: string[] } }
    const liteConfig = JSON.parse(readRepoFile('apps/desktop/src-tauri/tauri.lite.conf.json')) as {
      bundle?: { resources?: string[]; targets?: string[] }
    }

    const baseResources = baseConfig.bundle?.resources ?? []
    const windowsResources = windowsConfig.bundle?.resources ?? []
    const linuxResources = linuxConfig.bundle?.resources ?? []

    expect(baseResources).not.toContain('resources/*')
    expect(baseResources).not.toContain('target/release/vc-runtime/*')
    expect(baseResources).not.toContain('resources/runtime-pack/windows-x86_64/**/*')
    expect(baseResources).not.toContain('resources/runtime-pack/linux-x86_64/**/*')

    for (const resource of baseResources) {
      expect(windowsResources).toContain(resource)
      expect(linuxResources).toContain(resource)
    }

    expect(windowsConfig.bundle?.windows).toBeDefined()
    expect(windowsResources).toContain('resources/lib/pdfium.dll')
    expect(windowsResources).toContain('resources/tools/uv/windows-x86_64/*')
    expect(windowsResources).toContain('resources/tools/uv/windows-aarch64/*')
    expect(windowsResources).toContain('resources/runtime-pack/windows-x86_64/**/*')
    expect(windowsResources).toContain('target/release/vc-runtime/*')
    expect(windowsResources).not.toContain('resources/lib/linux-x86_64/**/*')
    expect(windowsResources).not.toContain('resources/runtime-pack/linux-x86_64/**/*')

    expect(linuxResources).not.toContain('resources/lib/linux-x86_64/**/*')
    expect(linuxResources).toContain('resources/runtime-pack/linux-x86_64/**/*')
    // Tauri merges tauri.linux.conf.json into every Linux build, `cargo build`
    // and `cargo test` included, and fails when a listed resource is missing.
    // The fetched Pdfium therefore lives in the explicit Pro Linux release
    // overlay, so a fresh Linux checkout still compiles without the network.
    expect(linuxResources).not.toContain('resources/pdfium/libpdfium.so')
    expect(linuxResources).not.toContain('resources/pdfium/LICENSE')
    expect(linuxResources).not.toContain('resources/lib/pdfium.dll')
    expect(linuxResources).not.toContain('resources/tools/uv/windows-x86_64/*')
    expect(linuxResources).not.toContain('resources/tools/uv/windows-aarch64/*')
    expect(linuxResources).not.toContain('resources/runtime-pack/windows-x86_64/**/*')
    expect(linuxResources).not.toContain('target/release/vc-runtime/*')

    expect(liteConfig.bundle?.resources).toBeUndefined()
    expect(liteConfig.bundle?.targets).toBeUndefined()
  })

  it('ships the fetched Linux Pdfium and never a stub shared library', () => {
    const linuxResources =
      (
        JSON.parse(readRepoFile('apps/desktop/src-tauri/tauri.linux.conf.json')) as {
          bundle?: { resources?: string[] }
        }
      ).bundle?.resources ?? []
    const proLinuxResources =
      (
        JSON.parse(readRepoFile('apps/desktop/src-tauri/tauri.pro.linux.conf.json')) as {
          bundle?: { resources?: string[] }
        }
      ).bundle?.resources ?? []
    const liteLinuxResources =
      (
        JSON.parse(readRepoFile('apps/desktop/src-tauri/tauri.lite.linux.conf.json')) as {
          bundle?: { resources?: string[] }
        }
      ).bundle?.resources ?? []

    // Bundle resource arrays replace each other when configs merge, so the Pro
    // overlay must restate every Pro Linux resource and add only Pdfium.
    expect(proLinuxResources).toEqual([
      ...linuxResources,
      'resources/pdfium/libpdfium.so',
      'resources/pdfium/LICENSE',
    ])

    for (const resources of [proLinuxResources, liteLinuxResources]) {
      expect(resources).toContain('resources/pdfium/libpdfium.so')
      expect(resources).toContain('resources/pdfium/LICENSE')
      expect(resources).not.toContain('resources/lib/linux-x86_64/**/*')
    }

    // The text fixtures moved to the test tree; no Linux bundle glob points at
    // their old resource paths anymore.
    for (const fixture of ['libpdfium.so', 'libonnxruntime.so']) {
      expect(existsSync(resolve(tauriRoot, 'resources/lib/linux-x86_64', fixture))).toBe(false)
      expect(
        existsSync(resolve(tauriRoot, 'tests/fixtures/resource-audit', `${fixture}.fixture`))
      ).toBe(true)
    }

    // Any `*.so` a Linux resource glob can reach under the native-lib dirs must
    // be a real-enough library. The runtime-pack `.so` stubs are a deliberate
    // fixture (replaced by the bootstrap download, never loaded directly), so
    // they are out of scope here.
    const tiny = [
      resolve(tauriRoot, 'resources/lib'),
      resolve(tauriRoot, 'resources/pdfium'),
    ].flatMap((dir) => findTinySharedLibraries(dir))
    expect(tiny).toEqual([])
  })

  it('keeps every Lite bundle free of the Pro payload', () => {
    const liteConfigNames = readdirSync(tauriRoot).filter((name) =>
      /^tauri\.lite(\.[a-z]+)?\.conf\.json$/.test(name)
    )
    expect(liteConfigNames).toEqual(
      expect.arrayContaining([
        'tauri.lite.conf.json',
        'tauri.lite.linux.conf.json',
        'tauri.lite.macos.conf.json',
        'tauri.lite.windows.conf.json',
      ])
    )

    // Lite has no local ML: no models, runtime pack, uv or Python sidecars.
    const proOnly = [
      /^resources\/models\//,
      /^resources\/runtime-pack\//,
      /^resources\/tools\/uv\//,
      /^scripts\/.*\.py$/,
    ]
    for (const name of liteConfigNames) {
      const resources =
        (
          JSON.parse(readRepoFile('apps/desktop/src-tauri', name)) as {
            bundle?: { resources?: string[] }
          }
        ).bundle?.resources ?? []
      for (const resource of resources) {
        for (const pattern of proOnly) {
          expect(resource, `${name}: ${resource}`).not.toMatch(pattern)
        }
      }
    }

    // Windows-only payload must not leak into the Unix Lite overlays: build.rs
    // stages vc-runtime only for windows-msvc, so the glob would match nothing
    // and fail the bundle.
    for (const name of ['tauri.lite.linux.conf.json', 'tauri.lite.macos.conf.json']) {
      const raw = readRepoFile('apps/desktop/src-tauri', name)
      expect(raw, name).not.toMatch(/\.dll\b/)
      expect(raw, name).not.toContain('vc-runtime')
    }

    // Every Lite Linux resource is a literal path that a clean checkout has, or
    // that `fetch-pdfium.sh linux-x64` writes. No globs: one that matches
    // nothing fails the bundle.
    const tracked = new Set(
      execFileSync('git', ['ls-files', '--', 'apps/desktop/src-tauri'], {
        cwd: repoRoot,
        encoding: 'utf8',
      })
        .split('\n')
        .filter(Boolean)
    )
    const fetchedByPdfiumScript = ['resources/pdfium/libpdfium.so', 'resources/pdfium/LICENSE']
    const liteLinuxResources =
      (
        JSON.parse(readRepoFile('apps/desktop/src-tauri/tauri.lite.linux.conf.json')) as {
          bundle?: { resources?: string[] }
        }
      ).bundle?.resources ?? []
    expect(liteLinuxResources).toEqual(expect.arrayContaining(fetchedByPdfiumScript))
    for (const resource of liteLinuxResources) {
      expect(resource, resource).not.toMatch(/[*?[{]/)
      const available =
        tracked.has(`apps/desktop/src-tauri/${resource}`) ||
        fetchedByPdfiumScript.includes(resource)
      expect(available, `${resource} is neither tracked nor fetched`).toBe(true)
    }
  })

  it('checks the built Lite .deb for Pdfium and against the Pro payload', () => {
    const litePreview = readRepoFile('.github/workflows/lite-preview.yml')
    expect(litePreview).toContain("grep -qE '/resources/pdfium/libpdfium\\.so$'")
    expect(litePreview).toContain("grep -E '/(models|runtime-pack|tools/uv)/|\\.py$|\\.dll$'")
  })

  it('fetches the real Linux Pdfium for the Pro release build', () => {
    const releaseWorkflow = readRepoFile('.github/workflows/release.yml')
    const fetchIndex = releaseWorkflow.indexOf(
      'bash apps/desktop/scripts/fetch-pdfium.sh linux-x64'
    )
    const buildIndex = releaseWorkflow.indexOf('- name: Build Tauri application')
    expect(fetchIndex).toBeGreaterThan(-1)
    expect(buildIndex).toBeGreaterThan(-1)
    expect(fetchIndex).toBeLessThan(buildIndex)

    const fetchStep = releaseWorkflow.slice(
      releaseWorkflow.lastIndexOf('- name:', fetchIndex),
      buildIndex
    )
    expect(fetchStep).toContain("if: matrix.platform == 'ubuntu-22.04'")

    // The Pro Linux leg bundles through the explicit overlay that lists Pdfium.
    expect(releaseWorkflow).toContain('--config apps/desktop/src-tauri/tauri.pro.linux.conf.json')
    expect(releaseWorkflow).not.toContain('--config apps/desktop/src-tauri/tauri.linux.conf.json')
  })

  it('ships fixture runtime-pack manifests for windows and linux with concrete payload files', () => {
    const platforms = ['windows-x86_64', 'linux-x86_64'] as const
    const desktopVersion = readDesktopVersion()

    for (const platform of platforms) {
      const manifest = readPackManifest(platform)
      const packRoot = resolve(tauriRoot, 'resources/runtime-pack', platform)

      expect(manifest.platform).toBe(platform)
      expect(manifest.app_version).toBe(desktopVersion)
      expect(manifest.payload_profile).toBe('fixture')
      expect(manifest.release_injection_required).toBe(true)
      expect(manifest.external_artifacts_required.length).toBeGreaterThan(0)
      expect(manifest.pack_version).toMatch(/^\d{4}\.\d{2}\.\d+$/)
      expect(manifest.python_relpath.length).toBeGreaterThan(0)
      expect(manifest.uv_relpath.length).toBeGreaterThan(0)
      expect(allEntries(manifest).length).toBeGreaterThan(5)

      for (const entry of allEntries(manifest)) {
        expect(entry.sha256).toMatch(/^[a-f0-9]{64}$/)
        expect(entry.size).toBeGreaterThan(0)
        expect(existsSync(resolve(packRoot, entry.path))).toBe(true)
      }
    }
  })

  it('keeps runtime-pack scripts byte-for-byte aligned with their canonical sources', () => {
    const platforms = ['windows-x86_64', 'linux-x86_64'] as const
    const scripts = ['paddle_vl.py', 'spacy_ner.py', 'transcribe.py'] as const

    for (const script of scripts) {
      const canonicalSource = readRepoFile('apps/desktop/src-tauri/scripts', script)

      for (const platform of platforms) {
        expect(
          readRepoFile('apps/desktop/src-tauri/resources/runtime-pack', platform, 'scripts', script)
        ).toBe(canonicalSource)
      }
    }
  })

  it('ships a lean installer with a baked signed bootstrap source instead of bundling the runtime', () => {
    const releaseWorkflow = readRepoFile('.github/workflows/release.yml')
    const publishWorkflow = readRepoFile('.github/workflows/publish-runtime-bootstrap.yml')
    const ciWorkflow = readRepoFile('.github/workflows/ci.yml')

    // Lean model: the ~2.2GB runtime is NOT assembled into / injected onto the
    // installer (it overruns the NSIS/WiX 2 GiB bundler limits). The committed
    // fixture ships and the app downloads the runtime at first launch.
    expect(releaseWorkflow).not.toContain('build_runtime_pack.py')
    expect(releaseWorkflow).not.toContain('Inject assembled runtime-pack')

    // The trusted download source is baked into release builds so a clean machine
    // has somewhere to fetch the runtime from.
    expect(releaseWorkflow).toContain('ENTROPIA_RUNTIME_BOOTSTRAP_MANIFEST_URL')
    expect(releaseWorkflow).toContain('ENTROPIA_RUNTIME_BOOTSTRAP_PUBLIC_KEY_BASE64')
    expect(releaseWorkflow).toContain('--bundles nsis')

    // The signed runtime archive + manifest are hosted by the dedicated publish
    // workflow, which verifies the signing key matches the baked public key.
    expect(publishWorkflow).toContain('publish_runtime_bootstrap.py')
    expect(publishWorkflow).toContain('Verify signing key matches baked public key')

    // CI still assembles + smoke-checks a runtime-pack and runs this packaging test.
    expect(ciWorkflow).toContain('runtime-pack-smoke')
    expect(ciWorkflow).toContain('--root apps/desktop/src-tauri/target/runtime-pack')
    expect(ciWorkflow).toContain('src/lib/runtime-packaging.test.ts')
  })

  it('documents fixture scope and the lean download-at-launch model', () => {
    const rootReadme = readRepoFile('README.md')
    const resourcesReadme = readRepoFile('apps/desktop/src-tauri/resources/README.md')
    const maintenanceDoc = readRepoFile(
      'apps/desktop/src-tauri/resources/runtime-pack/MAINTENANCE.md'
    )
    const windowsAssemblyNotes = readRepoFile(
      'apps/desktop/src-tauri/resources/runtime-pack/windows-x86_64/ASSEMBLY_NOTES.md'
    )
    const linuxAssemblyNotes = readRepoFile(
      'apps/desktop/src-tauri/resources/runtime-pack/linux-x86_64/ASSEMBLY_NOTES.md'
    )

    expect(rootReadme).toContain('runtime-pack')
    expect(rootReadme).toContain('instalador liviano')
    expect(resourcesReadme).toContain('payload_profile: fixture')
    expect(resourcesReadme).toContain('instalador liviano')
    expect(maintenanceDoc).toContain('--payload-root')
    expect(maintenanceDoc).toContain('assembly-summary.json')
    expect(maintenanceDoc).toContain('windows-x86_64')
    expect(maintenanceDoc).toContain('linux-x86_64')
    expect(windowsAssemblyNotes).toContain('python/python.exe')
    expect(windowsAssemblyNotes).toContain('resources/lib/pdfium.dll')
    expect(linuxAssemblyNotes).toContain('python/bin/python3')
    expect(linuxAssemblyNotes).toContain('resources/lib/libonnxruntime.so')
  })
})
