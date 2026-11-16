import { readdirSync, readFileSync } from 'node:fs'
import { relative, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Step 29 of docs/PLAN_HOMOGENEIZACION.md: the scales that are left after the
 * view migrations stay on tokens. Each rule below reads every stylesheet of the
 * app and of the UI package, so a literal that comes back fails here with the
 * file and line, not in a screenshot.
 */
const ROOTS = [
  resolve(import.meta.dirname),
  resolve(import.meta.dirname, '../../../packages/ui/src'),
]
const REPO = resolve(import.meta.dirname, '../../..')

/**
 * The exceptions GUIA_DISENO.md names, by file and declaration. Nothing else
 * may carry a literal for these properties.
 */
const EXCEPTIONS: Record<string, string> = {
  // The image host must add no line box of its own around the <img>.
  'packages/ui/src/components/WritingEditor/WritingEditor.svelte': 'line-height: 0',
}

function stylesheets(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(dir, entry.name)
    if (entry.isDirectory()) return entry.name === 'node_modules' ? [] : stylesheets(path)
    return /\.(svelte|css)$/.test(entry.name) ? [path] : []
  })
}

/** Each <style> line (or each line of a .css file) with its 1-based number, comments blanked. */
function styleLines(path: string): Array<[number, string]> {
  const source = readFileSync(path, 'utf-8')
  const start = path.endsWith('.css') ? 0 : source.indexOf('<style')
  if (start < 0) return []
  const end = path.endsWith('.css') ? source.length : source.lastIndexOf('</style>')
  const offset = source.slice(0, start).split('\n').length - 1
  return source
    .slice(start, end)
    .replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '))
    .split('\n')
    .map((line, index): [number, string] => [offset + index + 1, line])
}

function offenders(pattern: RegExp): string[] {
  return ROOTS.flatMap(stylesheets).flatMap((path) => {
    const file = relative(REPO, path).replaceAll('\\', '/')
    return styleLines(path)
      .filter(([, line]) => pattern.test(line))
      .filter(([, line]) => !(EXCEPTIONS[file] && line.includes(EXCEPTIONS[file])))
      .map(([number, line]) => `${file}:${number} ${line.trim()}`)
  })
}

describe('residual scales stay on tokens (plan step 29)', () => {
  it('has no global z-index literal: layers of 6 and up go through --z-*', () => {
    expect(offenders(/z-index:\s*(?:[6-9]|\d{2,})\b/)).toEqual([])
  })

  it('has no line-height literal', () => {
    expect(offenders(/line-height:\s*\d/)).toEqual([])
  })

  it('has no letter-spacing literal', () => {
    expect(offenders(/letter-spacing:\s*-?\d/)).toEqual([])
  })

  it('has no literal duration in a transition or animation', () => {
    expect(offenders(/(?:transition|animation)[\w-]*:.*\b\d*\.?\d+m?s\b/)).toEqual([])
    // A multi-line transition puts each duration on its own line.
    expect(offenders(/^\s+[a-z-]+\s+\d*\.?\d+m?s\b/)).toEqual([])
  })

  it('has no relative font-size: inline code uses --font-size-code-inline', () => {
    expect(offenders(/font-size:\s*\d*\.?\d+(?:em|rem|%)/)).toEqual([])
  })

  it('names no --font-weight-bold: semibold is the heaviest weight (F9)', () => {
    expect(offenders(/--font-weight-bold\b/)).toEqual([])
  })
})
