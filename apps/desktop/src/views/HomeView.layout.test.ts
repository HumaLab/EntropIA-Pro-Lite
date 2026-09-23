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
