import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * `.icon-button.right-panel-toggle` (the 20px full-height edge strip that
 * opts out of IconButton's fixed square) used to have the SAME specificity
 * as IconButton's own scoped rules that set `--icon-button-size` and the
 * box dimensions: both are exactly two class selectors
 * (`.icon-button.right-panel-toggle` vs. e.g. `.icon-button.svelte-HASH` or
 * `.icon-button--sm.svelte-HASH`), so it only won by stylesheet import
 * order — a production bundle could reorder that and collapse the 20px
 * strip into a 28px square.
 *
 * The fix scopes the override under the view's own top-level class
 * (`.collection-shell` / `.item-view`). Svelte appends its scoping class to
 * every compound selector segment outside `:global()`, so that ancestor
 * segment alone already carries two class selectors, giving the whole
 * selector strictly more class-level specificity than any single-class (or
 * single-class-plus-scope-hash) rule IconButton.svelte declares — a win
 * that holds regardless of stylesheet order.
 */
const ICON_BUTTON_SOURCE = readFileSync(
  resolve(
    import.meta.dirname,
    '../../../../packages/ui/src/components/IconButton/IconButton.svelte'
  ),
  'utf-8'
)
const COLLECTION_VIEW_SOURCE = readFileSync(
  resolve(import.meta.dirname, 'CollectionView.svelte'),
  'utf-8'
)
const ITEM_VIEW_SOURCE = readFileSync(resolve(import.meta.dirname, 'ItemView.svelte'), 'utf-8')

/** Every rule prelude (selector list, pre-`{`) in a component's <style> block. */
function ruleSelectors(source: string): string[] {
  const style = source
    .slice(source.indexOf('>', source.indexOf('<style')) + 1, source.lastIndexOf('</style>'))
    .replace(/\/\*[\s\S]*?\*\//g, '')
  const selectors: string[] = []
  const ruleRe = /([^{}]+)\{([^{}]*)\}/g
  let match: RegExpExecArray | null
  while ((match = ruleRe.exec(style))) {
    const [, prelude] = match
    for (const selector of prelude!.split(',')) selectors.push(selector.trim())
  }
  return selectors
}

/** Rules in IconButton.svelte whose declaration block sets the size the
 * `right-panel-toggle` opt-out must be able to override. */
function boxSizingSelectors(source: string): string[] {
  const style = source
    .slice(source.indexOf('>', source.indexOf('<style')) + 1, source.lastIndexOf('</style>'))
    .replace(/\/\*[\s\S]*?\*\//g, '')
  const out: string[] = []
  const ruleRe = /([^{}]+)\{([^{}]*)\}/g
  let match: RegExpExecArray | null
  while ((match = ruleRe.exec(style))) {
    const [, prelude, body] = match
    if (/--icon-button-size|(?:^|\s)(?:min-|max-)?(?:width|height):/m.test(body!)) {
      for (const selector of prelude!.split(',')) out.push(selector.trim())
    }
  }
  return out
}

describe('the right-panel-toggle opt-out beats IconButton on specificity deterministically', () => {
  it('every IconButton.svelte rule that sets a box dimension is a single bare class selector', () => {
    // A single class (plus the one scope hash Svelte appends) is the
    // weakest case this test has to beat; confirming it stays that way
    // means the assertion below is not accidentally comparing against a
    // stronger selector that crept in later.
    const selectors = boxSizingSelectors(ICON_BUTTON_SOURCE)
    expect(selectors.length).toBeGreaterThan(0)
    for (const selector of selectors) {
      expect(selector, `unexpectedly compound selector: ${selector}`).toMatch(/^\.[\w-]+$/)
    }
  })

  it('CollectionView scopes the toggle override under its own top-level class', () => {
    const selectors = ruleSelectors(COLLECTION_VIEW_SOURCE).filter((s) =>
      s.includes('right-panel-toggle')
    )
    expect(selectors.length).toBeGreaterThan(0)
    for (const selector of selectors) {
      // An ancestor class segment outside :global() always gets Svelte's
      // scope hash appended, so `.collection-shell` alone already carries
      // two class selectors before the two more inside :global(...).
      expect(selector).toMatch(/^\.collection-shell\s+:global\(\.icon-button\.right-panel-toggle/)
    }
  })

  it('ItemView scopes the toggle override under its own top-level class', () => {
    const selectors = ruleSelectors(ITEM_VIEW_SOURCE).filter((s) =>
      s.includes('right-panel-toggle')
    )
    expect(selectors.length).toBeGreaterThan(0)
    for (const selector of selectors) {
      expect(selector).toMatch(/^\.item-view\s+:global\(\.icon-button\.right-panel-toggle/)
    }
  })
})
