import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Final visual check (split view, narrowed pane) — regression from
 * d80a7073: the toolbar's `width: 100%` rule used to be a `@media` query,
 * firing only on a truly narrow WINDOW; converting it to `@container pane`
 * made it fire inside a narrow PANE too, and its blanket
 * `.collection-toolbar :global(.btn) { width: 100%; }` selector also
 * matches the import/export ICON-ONLY buttons — which use `aspect-ratio: 1`
 * (Button.svelte), so stretching their width to 100% stretches their height
 * to match, turning them into huge empty squares instead of the small
 * icon-only controls they must stay. The search bar and any labeled button
 * may still stretch; an icon-only button must not.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'CollectionView.svelte'), 'utf-8')
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

describe('the collection toolbar stretches at a narrow pane width, but its icon-only buttons never do', () => {
  it('leaves the import/export icon buttons to the Button primitive, which is a fixed square', () => {
    // Button.svelte caps an icon-only button at its token on both axes
    // (packages/ui control-block-size.test.ts), so the view no longer carries
    // its own exception to the full-width rule.
    const block = blockFor('@container pane (max-width: 720px)')
    expect(block).toMatch(/\.collection-toolbar\s*:global\(\.search-bar\)/)
    expect(STYLES).not.toMatch(/btn--icon-only/)
  })
})
