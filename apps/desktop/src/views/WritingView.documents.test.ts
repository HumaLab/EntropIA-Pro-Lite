import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The document grid in Escritura's landing view.
 *
 * jsdom performs no layout, so the three ways this breaks are three it cannot
 * see: a track rule that stops reflowing, a title that stops truncating, and a
 * delete control that drifts inside the card button and starts opening the
 * document it was meant to delete. All three are facts about the source.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'WritingView.svelte'), 'utf-8')

/** Comments are stripped: the rules below are documented in prose that names
 *  the very declarations under test, and a check that can be satisfied by its
 *  own documentation is a check that proves nothing. */
const STYLES = SOURCE.replace(/\/\*[\s\S]*?\*\//g, '')

function ruleFor(selector: string): string {
  const at = STYLES.indexOf(selector)
  expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
  const rule = STYLES.slice(at)
  return rule.slice(0, rule.indexOf('}'))
}

describe('the document grid reflows', () => {
  it('sizes its columns from the space available, not from a column count', () => {
    const list = ruleFor('.writing__list {')

    // A fixed count — repeat(3, 1fr) — renders fine and silently stops
    // reflowing, which is the regression no rendering test would catch.
    expect(list).toMatch(
      /grid-template-columns:\s*repeat\(auto-(fit|fill),\s*minmax\(\d+px,\s*1fr\)\)/
    )
  })

  it('gives a long document title every declaration an ellipsis needs', () => {
    const title = ruleFor('.writing__card-title {')

    // min-width is the one dropped as redundant that is not: without it a flex
    // item refuses to shrink below its content, so the text never overflows its
    // box, the ellipsis never appears, and the card widens instead.
    expect([
      /min-width:\s*0/.test(title),
      /overflow:\s*hidden/.test(title),
      /text-overflow:\s*ellipsis/.test(title),
      /white-space:\s*nowrap/.test(title),
    ]).toEqual([true, true, true, true])
  })
})

describe('deleting a document is not opening it', () => {
  it('keeps the delete control outside the card button', () => {
    const opening = SOURCE.indexOf('<button type="button" class="writing__card"')
    expect(opening, 'the document card button is gone').toBeGreaterThan(-1)

    const card = SOURCE.slice(opening, SOURCE.indexOf('</button>', opening))

    // A button nested in a button is invalid markup, and the browser hands the
    // click to the OUTER one — so a delete that drifts inside the card would
    // open the document rather than offer to discard it, with nothing visibly
    // wrong about the page.
    expect(card).not.toContain('IconButton')
  })

  it('places that control over the corner the card reserves for it', () => {
    // The control is positioned rather than laid out in flow, so the card has
    // to keep a matching inset on the right or the title runs underneath it.
    expect(ruleFor('.writing__row :global(.writing__card-discard) {')).toMatch(
      /position:\s*absolute/
    )
    expect(ruleFor('.writing__card {')).toMatch(/padding:[^;]*28px/)
  })
})
