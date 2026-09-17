import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The outline panel's layout, read from the stylesheet.
 *
 * jsdom performs no layout, so the defect this guards cannot be seen by
 * rendering: the section controls were hidden with `opacity: 0` but still took
 * their width, so every heading in the column read as truncated while the space
 * it needed sat empty beside it. An invisible control that still occupies the
 * row is the same bug as one that overflows it.
 */

const SOURCE = readFileSync(resolve(import.meta.dirname, 'WritingView.svelte'), 'utf-8')
const STYLES = SOURCE.slice(SOURCE.indexOf('<style>'))

function ruleFor(selector: string): string {
  const at = STYLES.indexOf(selector)
  expect(at, `no rule for ${selector}`).toBeGreaterThan(-1)
  const block = STYLES.slice(at)
  return block.slice(0, block.indexOf('}'))
}

describe('the outline section controls', () => {
  it('are taken out of the flow, so a heading gets the whole column', () => {
    expect(ruleFor('.writing__outline-actions {')).toMatch(/position:\s*absolute/)
  })

  it('leave no click target behind while they are hidden', () => {
    expect(ruleFor('.writing__outline-actions {')).toMatch(/pointer-events:\s*none/)
  })

  /** An overlay over a transparent row would show the title through the icons. */
  it('cover what they overlap rather than sitting on top of the text', () => {
    expect(ruleFor('.writing__outline-actions {')).toMatch(/background:\s*var\(--/)
  })
})
