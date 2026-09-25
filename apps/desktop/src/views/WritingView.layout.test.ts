import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Screenshot regression: the research panel's tabs, search and checkbox were
 * cut off at the split pane's right edge (~650px). The row `.writing__workspace`
 * lays out (outline, editor, research) never left the research panel less
 * than its full floor while the outline was also open — the three floors plus
 * their gaps and handles added up to more than the pane had, and
 * `overflow-x: hidden` (WorkPane.svelte) clipped the difference instead of
 * showing it.
 *
 * The fix (writing-layout.ts) is a pure function, fully covered by
 * writing-layout.test.ts. This file only guards that WritingView.svelte
 * actually wires its output in — jsdom/happy-dom perform no real layout, so
 * a rendering test would report every one of these widths as "fits", the
 * same way it would have reported the original bug as passing.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'WritingView.svelte'), 'utf-8')
const STYLES = SOURCE.replace(/\/\*[\s\S]*?\*\//g, '')

function ruleFor(selector: string): string {
  const at = STYLES.indexOf(selector)
  expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
  const rule = STYLES.slice(at)
  return rule.slice(0, rule.indexOf('}'))
}

describe('the outline is forced shut, not just squeezed, when it would not fit', () => {
  it('renders the outline section from the computed decision, not the raw preference', () => {
    expect(SOURCE).toContain('{#if layout.effectiveOutlineOpen}')
    // The raw preference must stay reserved for the toggle's own read/write —
    // an `{#if outlineOpen}` anywhere else would let the outline render open
    // while the pane is too narrow to fit it.
    expect(SOURCE).not.toMatch(/\{#if outlineOpen\}/)
  })

  it('disables the toggle and explains why, instead of leaving a dead control', () => {
    const toggle = SOURCE.slice(
      SOURCE.indexOf("label={t('writing.toggleOutline')}"),
      SOURCE.indexOf('</IconButton>', SOURCE.indexOf("label={t('writing.toggleOutline')}"))
    )
    expect(toggle).toContain('disabled={layout.forceOutlineCollapse}')
    expect(toggle).toContain("t('writing.outlineNoRoom')")
    expect(toggle).toContain('active={layout.effectiveOutlineOpen}')
  })

  it('never writes the forced state back to the persisted preference', () => {
    // Only `toggleOutline` (the button's own handler) may call
    // `writePanelPreference` for the outline key — the layout effect and the
    // derived `layout` value must not, or widening the pane back out would
    // "restore" a preference the user never actually changed.
    const writes = [...SOURCE.matchAll(/writePanelPreference\(OUTLINE_PREFERENCE[^)]*\)/g)]
    expect(writes).toHaveLength(1)
    const call = writes[0]!
    const before = SOURCE.slice(0, call.index).lastIndexOf('function ')
    expect(SOURCE.slice(before, before + 40)).toContain('toggleOutline')
  })

  it('measures the pane through a cleaned-up observer, not the window or the split ratio', () => {
    expect(SOURCE).toContain('watchPaneWidth(writingRootEl')
    expect(SOURCE).toContain('bind:this={writingRootEl}')
  })
})

describe('the research panel is squeezed to what is actually left, never clipped', () => {
  it('drives its floor from the computed layout, not the static bound', () => {
    expect(SOURCE).toContain('style:min-width="{layout.researchMinWidth}px"')
    expect(SOURCE).not.toContain('style:min-width="{RESEARCH_BOUNDS.squeeze}px"')
  })

  it("establishes its own container, so its tabs reflow off ITS width, not the pane's", () => {
    const rule = ruleFor('.writing__research {')
    expect(rule).toMatch(/container-type:\s*inline-size/)
    expect(rule).toMatch(/container-name:\s*research-panel/)
  })
})
