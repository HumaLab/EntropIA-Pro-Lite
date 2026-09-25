import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Split view lets two panes each show a view of the same kind side by side,
 * and a second Home tab is the one the spec calls out by name as reachable
 * today. A fixed DOM id is invalid HTML and an accessibility-tooling
 * foot-gun the moment that happens, so every view-owned id gets suffixed
 * with the pane id — see WritingView.pane-scope.test.ts, the first view
 * this pattern was proven on.
 *
 * This file proves the suffix is present in the source (interpolation is a
 * fact about the markup). HomeView.pane-scope.render.test.ts proves the
 * runtime consequence: two mounted HomeViews under two different panes
 * produce no duplicate id anywhere in the document.
 *
 * `home-continuar-title` and `home-activity-title` legitimately appear
 * twice each in the source (an if/else pair, and a title reused by a
 * `role="table"` region) — both occurrences get the identical `-{paneId}`
 * suffix, so this file checks for at least the expected count rather than
 * exactly one.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'HomeView.svelte'), 'utf-8')

const paneContextImports = [
  ...SOURCE.matchAll(/import\s*\{([^}]*)\}\s*from\s*'\$lib\/pane-context'/g),
].map(([, names]) => names)

function occurrences(source: string, pattern: RegExp): number {
  return [...source.matchAll(pattern)].length
}

describe('pane-scoped ids in the home view', () => {
  it('imports getPaneId from pane-context alongside getNavigation', () => {
    expect(paneContextImports.join(' ')).toMatch(/\bgetNavigation\b/)
    expect(paneContextImports.join(' ')).toMatch(/\bgetPaneId\b/)
  })

  it('captures its own pane id at init', () => {
    expect(SOURCE).toMatch(/const paneId = getPaneId\(\)/)
  })

  it('suffixes the home title id and its aria-labelledby reference with the pane id', () => {
    expect(SOURCE).toMatch(/aria-labelledby="home-title-\{paneId\}"/)
    expect(SOURCE).toMatch(/id="home-title-\{paneId\}"/)
  })

  it('suffixes both home-continuar-title declarations and their shared aria-labelledby reference', () => {
    expect(SOURCE).toMatch(/aria-labelledby="home-continuar-title-\{paneId\}"/)
    expect(occurrences(SOURCE, /id="home-continuar-title-\{paneId\}"/g)).toBe(2)
  })

  it('suffixes the home-corpus title id and its aria-labelledby reference with the pane id', () => {
    expect(SOURCE).toMatch(/aria-labelledby="home-corpus-title-\{paneId\}"/)
    expect(SOURCE).toMatch(/id="home-corpus-title-\{paneId\}"/)
  })

  it('suffixes the quick-access title id and its aria-labelledby reference with the pane id', () => {
    expect(SOURCE).toMatch(/aria-labelledby="home-quick-access-title-\{paneId\}"/)
    expect(SOURCE).toMatch(/id="home-quick-access-title-\{paneId\}"/)
  })

  it('suffixes the activity title id and both its aria-labelledby references with the pane id', () => {
    expect(occurrences(SOURCE, /aria-labelledby="home-activity-title-\{paneId\}"/g)).toBe(2)
    expect(SOURCE).toMatch(/id="home-activity-title-\{paneId\}"/)
  })

  it('leaves no unsuffixed literal id or reference behind', () => {
    expect(SOURCE).not.toMatch(/id="home-title"/)
    expect(SOURCE).not.toMatch(/aria-labelledby="home-title"/)
    expect(SOURCE).not.toMatch(/id="home-continuar-title"/)
    expect(SOURCE).not.toMatch(/aria-labelledby="home-continuar-title"/)
    expect(SOURCE).not.toMatch(/id="home-corpus-title"/)
    expect(SOURCE).not.toMatch(/aria-labelledby="home-corpus-title"/)
    expect(SOURCE).not.toMatch(/id="home-quick-access-title"/)
    expect(SOURCE).not.toMatch(/aria-labelledby="home-quick-access-title"/)
    expect(SOURCE).not.toMatch(/id="home-activity-title"/)
    expect(SOURCE).not.toMatch(/aria-labelledby="home-activity-title"/)
  })
})
