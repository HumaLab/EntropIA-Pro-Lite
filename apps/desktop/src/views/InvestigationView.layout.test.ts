import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, it, expect } from 'vitest'

/**
 * happy-dom does not lay out grids or evaluate container queries, so the
 * report/source layout is guarded at the stylesheet level.
 *
 * Regression (split view, 2026-09-26): inside a 640px pane of a wide window
 * the report and the sticky «Fuente» panel stayed side by side, because the
 * breakpoint measured the viewport, and the report overflowed its column
 * underneath the panel.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'InvestigationView.svelte'), 'utf-8')
const STYLES = SOURCE.slice(SOURCE.indexOf('<style>'))

function ruleBody(selector: string): string {
  const start = STYLES.indexOf(`${selector} {`)
  expect(start, `${selector} rule`).toBeGreaterThanOrEqual(0)
  return STYLES.slice(start, STYLES.indexOf('}', start))
}

describe('InvestigationView layout', () => {
  it('stacks the source under the report based on the pane width, not the window', () => {
    expect(STYLES).not.toMatch(/@media\s*\(max-width:\s*60rem\)/)
    const query = STYLES.match(
      /@container pane \(max-width: [^)]+\)\s*\{\s*\.investigation-view__body/
    )
    expect(query).not.toBeNull()
  })

  it('never lets the report grow wider than its own column', () => {
    expect(ruleBody('.investigation-chat')).toMatch(/grid-template-columns:\s*minmax\(0, 1fr\)/)
  })
})
