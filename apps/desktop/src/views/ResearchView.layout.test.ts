import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The two grids on the Investigación page: Anteriores in the left column and
 * the collection scope inside the form.
 *
 * jsdom performs no layout, so what it cannot see is a track rule that stops
 * reflowing and a name that stops truncating — and in the scope box, a header
 * that slides inside the scroll and takes the selected count with it.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'ResearchView.svelte'), 'utf-8')

/** Comments stripped: these rules are documented in prose naming the very
 *  declarations under test, and a check its own documentation can satisfy
 *  proves nothing. */
const STYLES = SOURCE.replace(/\/\*[\s\S]*?\*\//g, '')

function ruleFor(selector: string): string {
  const at = STYLES.indexOf(selector)
  expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
  const rule = STYLES.slice(at)
  return rule.slice(0, rule.indexOf('}'))
}

const TRACK_FUNCTION =
  /grid-template-columns:\s*repeat\(auto-(fit|fill),\s*minmax\(\d+px,\s*1fr\)\)/

function truncates(rule: string): boolean[] {
  return [
    /min-width:\s*0/.test(rule),
    /overflow:\s*hidden/.test(rule),
    /text-overflow:\s*ellipsis/.test(rule),
    /white-space:\s*nowrap/.test(rule),
  ]
}

describe('both grids size themselves from the space they have', () => {
  // A fixed count — repeat(2, 1fr) — renders fine and silently stops reflowing,
  // which is the regression no rendering test would catch.
  it('sizes Anteriores from its column, not from a column count', () => {
    expect(ruleFor('.research-view__job-list {')).toMatch(TRACK_FUNCTION)
  })

  it('sizes the collection scope from the form column it sits in', () => {
    expect(ruleFor('.research-form__scope-list {')).toMatch(TRACK_FUNCTION)
  })
})

describe('long names end in an ellipsis rather than in the next card', () => {
  // min-width is the one dropped as redundant that is not: without it a flex or
  // grid child refuses to shrink below its content, so the text never overflows
  // its box, the ellipsis never fires, and the card widens instead.
  it('truncates a long investigation title', () => {
    expect(truncates(ruleFor('.research-job-card__question {'))).toEqual([true, true, true, true])
  })

  it('truncates a long collection name', () => {
    expect(truncates(ruleFor('.research-form__scope-name {'))).toEqual([true, true, true, true])
  })

  it('lets the copy beside the job card actions shrink at all', () => {
    // The title can only ellipse if every box between it and the card agrees to
    // be narrower than its contents.
    expect(ruleFor('.research-job-card__copy {')).toMatch(/min-width:\s*0/)
    expect(ruleFor('.research-job-card__header {')).toMatch(/min-width:\s*0/)
  })
})

describe('the selected count stays put while the collections scroll', () => {
  it('keeps the scope header outside the scrolling box', () => {
    expect(ruleFor('.research-form__scope-list {')).toMatch(/overflow-y:\s*auto/)

    const listAt = SOURCE.indexOf('<div class="research-form__scope-list">')
    const headerAt = SOURCE.indexOf('<div class="research-form__scope-header">')
    expect([headerAt > -1, listAt > -1]).toEqual([true, true])

    // Sibling and earlier, not nested: a header inside the scroller scrolls away
    // and the count it carries goes with it.
    const list = SOURCE.slice(listAt, SOURCE.indexOf('</div>', listAt))
    expect([headerAt < listAt, list.includes('research-form__scope-header')]).toEqual([true, false])
  })
})
