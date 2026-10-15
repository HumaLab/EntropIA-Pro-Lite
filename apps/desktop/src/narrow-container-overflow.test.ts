import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Rows that must reflow inside the box they get, not the window.
 *
 * Each case below was found by rendering the Lite frontend in Chromium at a
 * 900px window, in 640px split panes, and with the Writing research panel
 * dragged from 240px to 560px, in Spanish and English. Each one overlapped,
 * cut off or crushed content only at those widths, which no one sees while
 * editing a wide window — so the rule that fixed it is pinned here.
 *
 * Read from the stylesheets, like the other layout guards in this folder:
 * happy-dom has no layout engine to measure a real overflow with.
 */
const SRC = import.meta.dirname

function stylesOf(relativePath: string): string {
  const source = readFileSync(resolve(SRC, relativePath), 'utf-8')
  const css = relativePath.endsWith('.css')
    ? source
    : Array.from(source.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g), (m) => m[1] ?? '').join('\n')
  return css.replace(/\/\*[\s\S]*?\*\//g, '')
}

const CSS_RULE = /([^{}]+)\{([^{}]*)\}/g

/** The declarations of the first rule whose selector is exactly `selector`. */
function declarations(styles: string, selector: string): Record<string, string> {
  const rule = Array.from(styles.matchAll(CSS_RULE)).find(
    (match) => (match[1] ?? '').trim().replace(/\s+/g, ' ') === selector
  )
  expect(rule, `${selector} is no longer in the stylesheet`).toBeDefined()

  return Object.fromEntries(
    (rule?.[2] ?? '')
      .split(';')
      .map((declaration) => declaration.split(/:(.*)/s).map((part) => part.trim()))
      .filter(([property, value]) => property && value)
      .map(([property, value]) => [property as string, value as string])
  )
}

describe('narrow containers reflow instead of overlapping', () => {
  it('wraps the Zotero «Sincronizar biblioteca» row under its help text', () => {
    // Both halves carry a basis (14rem + 12rem); without wrapping, a 240px
    // research panel crushed the text into a one-word column and pushed the
    // button past the card.
    const zotero = stylesOf('views/WritingZoteroTab.svelte')

    expect(declarations(zotero, '.zotero__bibliography-sync')['flex-wrap']).toBe('wrap')
    expect(declarations(zotero, '.zotero__bibliography-sync-help')['min-width']).toBe('0')
    expect(declarations(zotero, '.zotero__bibliography-sync-action')['min-width']).toBe('0')
    // The library picker never asks for more than the panel has.
    expect(declarations(zotero, '.zotero__menu-trigger')['max-width']).toBe('min(260px, 100%)')
  })

  it('wraps the Obras search form instead of cutting its buttons off', () => {
    const search = stylesOf('views/BibliographySearchTab.svelte')

    expect(declarations(search, '.bib-search__form')['flex-wrap']).toBe('wrap')
    expect(declarations(search, '.bib-search__field')).toMatchObject({
      flex: '1 1 12rem',
      'min-width': '0',
    })
  })

  it('gives the manuscript title a basis, so the Writing bar wraps before squeezing it', () => {
    // `flex: 1` is a zero basis: the bar never wrapped and a 640px pane cut
    // the title to one letter.
    const writing = stylesOf('views/WritingView.svelte')

    expect(declarations(writing, '.writing__bar')['flex-wrap']).toBe('wrap')
    expect(declarations(writing, '.writing__doc-title')).toMatchObject({
      flex: '1 1 14rem',
      'min-width': '0',
    })
  })

  it('seats the history close button in its own grid track, not over the preview', () => {
    const history = stylesOf('views/WritingHistoryPanel.svelte')

    expect(declarations(history, '.history-panel')['grid-template-columns']).toMatch(/ auto$/)
    const close = declarations(history, '.history-panel > :global(button:first-child)')
    expect(close).not.toHaveProperty('position')
    expect(close['grid-column']).toBe('3')
  })

  it('keeps the share panel headings clear of its pinned close button', () => {
    const share = stylesOf('views/WritingSharePanel.svelte')

    expect(declarations(share, '.share-panel')['grid-template-columns']).toBe(
      'repeat(auto-fit, minmax(min(100%, 16rem), 1fr))'
    )
    expect(declarations(share, 'h3')['padding-inline-end']).toBe('calc(28px + var(--space-2))')
    expect(declarations(share, '.share-panel__row')['flex-wrap']).toBe('wrap')
  })

  it('ellipsizes a long collection name in Actividad reciente like the document beside it', () => {
    const home = stylesOf('views/HomeView.svelte')

    expect(declarations(home, '.home-view__recent-cell')).toMatchObject({
      'min-width': '0',
      overflow: 'hidden',
      'white-space': 'nowrap',
      'text-overflow': 'ellipsis',
    })
  })

  it('lets a fieldset shrink to its container like every other block', () => {
    // Browsers default <fieldset> to `min-inline-size: min-content`: one
    // ellipsized nowrap label inside it widened the import dialog's
    // destination list 400px past the dialog.
    const app = stylesOf('app.css')

    expect(declarations(app, 'fieldset')['min-inline-size']).toBe('0')
  })
})
