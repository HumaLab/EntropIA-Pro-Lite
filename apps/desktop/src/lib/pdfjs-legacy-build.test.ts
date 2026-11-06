import { readdirSync, readFileSync } from 'node:fs'
import { relative, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * pdf.js must load from its legacy build.
 *
 * On macOS the webview is the system Safari (WKWebView), so pdf.js runs on
 * whatever engine the installed macOS ships — not on the Chromium age the
 * frontend targets. pdfjs-dist 4.10's modern `build/` needs:
 *   - class static blocks (both `build/` entry and worker), which Safari only
 *     parses from 16.4 — older Safari fails with «Unexpected token '{'»;
 *   - `Promise.withResolvers` without a polyfill, which Safari only has from
 *     17.4 — Safari 16.4–17.3 fails at runtime.
 * `legacy/build/` is transpiled for those engines and ships the polyfill, so
 * every runtime load must use it. Type-only imports from `pdfjs-dist` are
 * erased at compile time and stay allowed.
 */
const REPO_ROOT = resolve(import.meta.dirname, '../../../..')
const SOURCE_ROOTS = ['apps/desktop/src', 'packages/ui/src']
const SKIPPED_DIRS = /^(?:__tests__|__mocks__|__fixtures__|node_modules)$/

/** Runtime loads of pdf.js's modern build, each with the reason it breaks. */
const MODERN_LOADS: Array<{ pattern: RegExp; what: string }> = [
  {
    pattern: /(?:await|=|return)\s+import\(\s*['"]pdfjs-dist['"]\s*\)/,
    what: "dynamic import of pdfjs-dist's modern entry point; use 'pdfjs-dist/legacy/build/pdf.mjs'",
  },
  {
    pattern: /require\(\s*['"]pdfjs-dist['"]\s*\)/,
    what: "CommonJS require of pdfjs-dist's modern entry point; use 'pdfjs-dist/legacy/build/pdf.mjs'",
  },
  {
    pattern: /['"]pdfjs-dist\/build\//,
    what: "modern worker bundle; use 'pdfjs-dist/legacy/build/pdf.worker.min.mjs'",
  },
]

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = resolve(dir, entry.name)
    if (entry.isDirectory()) {
      return SKIPPED_DIRS.test(entry.name) ? [] : sourceFiles(full)
    }
    return entry.name.endsWith('.test.ts') || entry.name.endsWith('.d.ts') ? [] : [full]
  })
}

function violations(): string[] {
  const found: string[] = []
  for (const root of SOURCE_ROOTS) {
    for (const file of sourceFiles(resolve(REPO_ROOT, root))) {
      const lines = readFileSync(file, 'utf-8').split('\n')
      lines.forEach((line, index) => {
        for (const { pattern, what } of MODERN_LOADS) {
          if (pattern.test(line)) {
            const where = `${relative(REPO_ROOT, file).replaceAll('\\', '/')}:${index + 1}`
            found.push(`${where}: ${what}`)
          }
        }
      })
    }
  }
  return found
}

describe('pdf.js build', () => {
  it('loads only the legacy build, which macOS Safari before 17.4 can run', () => {
    expect(
      violations(),
      'PDF viewing must work in the macOS WKWebView, whose engine is the installed ' +
        "Safari. pdfjs-dist's modern build uses class static blocks (needs Safari 16.4) " +
        'and Promise.withResolvers without a polyfill (needs Safari 17.4); ' +
        'legacy/build has neither problem.'
    ).toEqual([])
  })
})
