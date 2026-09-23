import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The Continuar / Estado del corpus proportion and its responsive stacking
 * (odd/tasks/home-view.md T3a): jsdom performs no layout, so what it cannot
 * see is a ratio silently drifting or a breakpoint quietly dropped.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'HomeView.svelte'), 'utf-8')
const STYLES = SOURCE.replace(/\/\*[\s\S]*?\*\//g, '')

function ruleFor(selector: string): string {
  const at = STYLES.indexOf(selector)
  expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
  const rule = STYLES.slice(at)
  return rule.slice(0, rule.indexOf('}'))
}

describe('Continuar / Estado del corpus proportion', () => {
  it('splits the top row roughly 60/40', () => {
    expect(ruleFor('.home-view__top-row {')).toMatch(/grid-template-columns:\s*3fr\s+2fr/)
  })

  it('stacks the two panels at the app-wide 720px breakpoint', () => {
    const mediaAt = STYLES.indexOf('@media (max-width: 720px)')
    expect(mediaAt, '@media (max-width: 720px) is not used in HomeView.svelte').toBeGreaterThan(-1)
    const mediaBlock = STYLES.slice(
      mediaAt,
      STYLES.indexOf('}', STYLES.indexOf('}', mediaAt) + 1) + 1
    )
    expect(mediaBlock).toContain('.home-view__top-row')
    expect(mediaBlock).toMatch(/grid-template-columns:\s*1fr/)
  })
})

/**
 * Acceso rápido micro-layout (T3h): more inner padding, a taller/comfortable
 * box, icon/copy/arrow away from the edges and vertically centered, all four
 * cards sharing the one class so they stay equal height and aligned.
 */
describe('Acceso rápido card micro-layout', () => {
  it('gives the card more breathing room than the previous 64px/16px gutter', () => {
    const rule = ruleFor('.home-view__quick-access-card {')
    expect(rule).not.toMatch(/height:\s*64px/)
    expect(rule).not.toMatch(/padding:\s*0\s+var\(--space-4\)/)
    expect(rule).toMatch(/padding:\s*var\(--space-4\)\s+var\(--space-5\)/)
  })

  it('lays icon, copy and arrow out in one row so the content sits vertically centered', () => {
    const rule = ruleFor('.home-view__quick-access-card {')
    expect(rule).toMatch(/display:\s*flex/)
    expect(rule).toMatch(/align-items:\s*center/)
    expect(rule).toMatch(/gap:\s*var\(--space-\d\)/)
  })

  it('no longer pins the arrow to the corner: it flows in the row instead', () => {
    const rule = ruleFor('.home-view__quick-access-arrow {')
    expect(rule).not.toMatch(/position:\s*absolute/)
  })

  it('groups title and description in their own column, separate from the icon and arrow', () => {
    expect(STYLES).toContain('.home-view__quick-access-copy')
    const rule = ruleFor('.home-view__quick-access-copy {')
    expect(rule).toMatch(/flex:\s*1/)
  })

  it('uses only spacing tokens for the card gutter, never a raw hex or px literal beyond the icon size', () => {
    const rule = ruleFor('.home-view__quick-access-card {')
    expect(rule).not.toMatch(/#[0-9a-fA-F]{3,8}/)
  })
})

/**
 * Quick-access arrows are readable at rest (T3k-3): they used to sit at
 * `opacity: 0.55` on top of an already-muted color, reading as almost
 * invisible. At rest they must use a muted-but-readable text token — never
 * a border token — at full opacity, brightening to secondary text on hover.
 */
describe('Quick-access arrow is readable at rest', () => {
  it('uses the muted text token at rest, not a border token, and no extra dimming opacity', () => {
    const rule = ruleFor('.home-view__quick-access-arrow {')
    expect(rule).toMatch(/color:\s*var\(--color-text-muted\)/)
    expect(rule).not.toMatch(/--color-border/)
    expect(rule).not.toMatch(/opacity:\s*0(\.\d+)?;/)
  })

  it('brightens to secondary text on hover/focus', () => {
    const at = STYLES.indexOf('.home-view__quick-access-arrow,')
    expect(at, 'hover/focus rule for the arrow is missing').toBeGreaterThan(-1)
    const rule = STYLES.slice(at, STYLES.indexOf('}', at))
    expect(rule).toMatch(/color:\s*var\(--color-text-secondary\)/)
  })
})

/**
 * Continuar rows stay compact (T3k-1): since T3i the top row sizes to the
 * taller Estado del corpus panel, and the Continuar rows used to stretch
 * (flex: 1) to fill it — ~100px each. Rows must keep their natural, compact
 * height instead, stacked at the top, leaving any spare row height empty
 * below the last row.
 */
describe('Continuar rows stay compact instead of stretching to the panel height (T3k)', () => {
  it('does not let a Continuar row grow to fill the panel', () => {
    const rule = ruleFor('.home-view__continuar-item {')
    expect(rule).not.toMatch(/flex:\s*1\b/)
  })

  it('sizes the row from its own padding instead of a stretched 100% height', () => {
    const rule = ruleFor('.home-view__continuar-row {')
    expect(rule).not.toMatch(/height:\s*100%/)
    expect(rule).toMatch(/padding:\s*var\(--space-3\)\s+var\(--space-4\)/)
  })

  it('keeps the list itself filling the panel, so spare space lands below the last row', () => {
    const rule = ruleFor('.home-view__continuar-list {')
    expect(rule).toMatch(/flex:\s*1\b/)
  })
})

/**
 * Estado del corpus fits every indicator (T3i): the top row's original fixed
 * 250px height clipped the Embeddings row once OCR/STT/Texto/Embeddings grew
 * to six lines. The row must size to its own content instead, and the corpus
 * panel must never hide overflow that would clip a real indicator.
 */
describe('Estado del corpus panel fits every indicator without clipping (T3i)', () => {
  it('lets the top row and its panels take their content height instead of a fixed pixel height', () => {
    const topRow = ruleFor('.home-view__top-row {')
    expect(topRow).not.toMatch(/height:\s*\d/)

    const panel = ruleFor('.home-panel {')
    expect(panel).not.toMatch(/(?<!min-)height:\s*\d/)
  })

  it('never clips the corpus panel content behind overflow: hidden', () => {
    const rule = ruleFor('.home-view__corpus {')
    expect(rule).not.toMatch(/overflow:\s*hidden/)
  })
})
