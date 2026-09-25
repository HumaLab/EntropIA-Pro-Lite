import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Split view, narrow pane: the collections toolbar grew into a ~260px-tall
 * block. The search bar's `flex: 1 1 260px` is a width basis while its
 * wrapper is a row; the `@container pane (max-width: 720px)` rule turned the
 * wrapper into a column, so the same basis became a 260px HEIGHT, and the
 * toolbar row's `align-items: stretch` then pulled the "new collection"
 * button to that height too. The toolbar sizes to its content: the wrapper
 * stays a row and the toolbar centres its items.
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

describe('the collections toolbar keeps its content height at a narrow pane width', () => {
  it('never turns the search wrapper into a column, where the bar flex basis becomes a height', () => {
    const narrow = blockFor('@container pane (max-width: 720px)')
    expect(narrow).not.toMatch(/flex-direction:\s*column/)
  })

  it('never stretches the toolbar row items to the tallest one', () => {
    const narrow = blockFor('@container pane (max-width: 720px)')
    expect(narrow).not.toMatch(/align-items:\s*stretch/)
  })

  it('leaves the icon-only button to the Button primitive, which is a fixed square', () => {
    // Button.svelte caps an icon-only button at its token on both axes
    // (packages/ui control-block-size.test.ts), so the view no longer carries
    // its own exception to the full-width rule.
    const block = blockFor('@container pane (max-width: 720px)')
    expect(block).toMatch(/\.collections-controls\s*:global\(\.search-bar\)/)
    expect(STYLES).not.toMatch(/btn--icon-only/)
  })
})
