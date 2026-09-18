import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The investigation detail: the report on the left, the cited source on the
 * right, both under the page header that sticks to the top of the app's
 * scroller.
 *
 * jsdom performs no layout, so what it cannot see is a column that stops
 * sharing the width evenly, or a sticky source panel whose offset no longer
 * clears the sticky header and slides its metadata underneath it.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'InvestigationView.svelte'), 'utf-8')

/** Comments stripped: a check its own documentation can satisfy proves nothing. */
const STYLES = SOURCE.replace(/\/\*[\s\S]*?\*\//g, '')

function ruleFor(selector: string): string {
  const at = STYLES.indexOf(selector)
  expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
  const rule = STYLES.slice(at)
  return rule.slice(0, rule.indexOf('}'))
}

describe('the report and its source share the width evenly', () => {
  it('splits the body into two equal tracks that may shrink below their content', () => {
    expect(ruleFor('.investigation-view__body {')).toMatch(
      /grid-template-columns:\s*minmax\(0,\s*1fr\)\s+minmax\(0,\s*1fr\);/
    )
  })

  it('lets the report fill its track instead of capping it and leaving a gap', () => {
    expect(ruleFor('.investigation-chat {')).not.toMatch(/max-width/)
  })
})

describe('the source panel starts below the sticky page header', () => {
  it('measures the header it has to clear', () => {
    expect(SOURCE).toMatch(
      /class="page-header investigation-view__header"[^>]*bind:this=\{headerEl\}/
    )
    expect(SOURCE).toMatch(/observer\.observe\(header\)/)
    expect(SOURCE).toMatch(/headerHeight = header\.offsetHeight/)
  })

  it('sticks at the measured header height, not at a fixed guess', () => {
    expect(ruleFor('.investigation-source {')).toMatch(
      /top:\s*calc\(var\(--investigation-header-height[^;]*\)/
    )
  })

  it('fits between the header and the bottom of the scroller that holds it', () => {
    const rule = ruleFor('.investigation-source {')
    expect(rule).toMatch(/max-height:[^;]*--investigation-scrollport-height/)
    expect(rule).toMatch(/max-height:[^;]*--investigation-header-height/)
    expect(rule).not.toMatch(/max-height:\s*calc\(100vh/)
  })
})
