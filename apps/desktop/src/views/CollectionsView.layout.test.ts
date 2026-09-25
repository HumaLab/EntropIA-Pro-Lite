import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Final visual check (split view, narrowed pane) — the same regression as
 * CollectionView.layout.test.ts, from the same `@container pane` conversion
 * (d80a7073): `.collections-controls :global(.btn) { width: 100%; }` also
 * matches the "new collection" button when it renders icon-only
 * (`iconOnly={!showCreate}`), stretching it into a huge empty square via its
 * `aspect-ratio: 1` (Button.svelte) instead of keeping it a small icon-only
 * control.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'CollectionsView.svelte'), 'utf-8')
const STYLES = SOURCE.replace(/\/\*[\s\S]*?\*\//g, '')

function blockFor(selector: string): string {
  const at = STYLES.indexOf(selector)
  expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
  const open = STYLES.indexOf('{', at)
  let depth = 0
  for (let i = open; i < STYLES.length; i++) {
    if (STYLES[i] === '{') depth++
    else if (STYLES[i] === '}') {
      depth--
      if (depth === 0) return STYLES.slice(at, i + 1)
    }
  }
  throw new Error(`unterminated block for ${selector}`)
}

describe('the collections toolbar stretches at a narrow pane width, but its icon-only "new collection" button never does', () => {
  it('keeps the icon-only new-collection button at its normal fixed size', () => {
    const block = blockFor('@container pane (max-width: 720px)')
    expect(block).toMatch(/\.collections-controls\s*:global\(\.search-bar\)/)
    expect(block).toMatch(
      /\.collections-controls\s*:global\(\.btn\.btn--icon-only\)\s*\{\s*width:\s*var\(--control-height-md\);/
    )
  })
})
