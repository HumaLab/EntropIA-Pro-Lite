import { readdirSync, readFileSync, statSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const GET_DOCUMENT_CALL = /getDocument\s*\(/g
const VIA_HELPER = /^\s*pdfDocumentOptions\s*\(/

/**
 * The filenames and directories whose contents are not shipped code: tests
 * (and their fixtures) call pdf.js directly on purpose, and `node_modules` is
 * not ours. Everything else under the two source roots is production code and
 * must open documents through the shared helper.
 */
function isSourceFile(name: string): boolean {
  return /\.(ts|svelte)$/.test(name) && !/\.(test|spec)\./.test(name)
}

function sourcesUnder(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = resolve(dir, entry.name)
    if (entry.isDirectory()) {
      return entry.name === 'node_modules' || entry.name === '__tests__' ? [] : sourcesUnder(full)
    }
    return isSourceFile(entry.name) ? [full] : []
  })
}

/**
 * Every shipped source wherever it is authored. The rule describes the app's
 * whole pdf.js surface, not one package's: a call site that escaped into the
 * other root would keep passing while the viewer stayed slow there.
 */
function everySourceFile(): string[] {
  return [
    ...sourcesUnder(import.meta.dirname),
    ...sourcesUnder(resolve(import.meta.dirname, '../../../packages/ui/src')),
  ]
}

function lineOf(source: string, index: number): number {
  return source.slice(0, index).split('\n').length
}

describe('getDocument call sites', () => {
  it('open documents only through pdfDocumentOptions', () => {
    const offenders: string[] = []

    for (const file of everySourceFile()) {
      const source = readFileSync(file, 'utf-8')
      for (const match of source.matchAll(GET_DOCUMENT_CALL)) {
        const index = match.index ?? 0
        const after = source.slice(index + match[0].length)
        if (!VIA_HELPER.test(after)) {
          offenders.push(`${file}:${lineOf(source, index)}`)
        }
      }
    }

    expect(offenders).toEqual([])
  })

  it('scan the two source roots (guard cannot silently see nothing)', () => {
    const files = everySourceFile()
    expect(files.length).toBeGreaterThan(50)
    expect(files.some((file) => file.endsWith('DocumentViewer.svelte'))).toBe(true)
    expect(files.some((file) => file.endsWith('ocr-rich-text.ts'))).toBe(true)
    // Sanity: the scan is real files on disk.
    expect(files.every((file) => statSync(file).isFile())).toBe(true)
  })
})
