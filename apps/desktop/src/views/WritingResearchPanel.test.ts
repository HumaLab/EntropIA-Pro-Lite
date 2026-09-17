import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it } from 'vitest'
import WritingResearchPanel from './WritingResearchPanel.svelte'

/**
 * The right-hand panel of the three-panel shell (plan-editor.md 6.3).
 *
 * Its four tabs are filled by later units - Corpus by Unit 4, Notas by 5,
 * Zotero by 6 and Agente by 7 - so what is asserted here is the shell: that
 * every tab exists, that exactly one body is shown, and that the tabs are
 * wired for a screen reader rather than being four styled buttons.
 */

const TAB_NAMES = ['Corpus', 'Zotero', 'Notas', 'Agente']

describe('the research panel', () => {
  it('offers the four tabs 6.3 names', () => {
    render(WritingResearchPanel)

    const tabs = screen.getAllByRole('tab')
    expect(tabs.map((tab) => tab.textContent?.trim())).toEqual(TAB_NAMES)
  })

  it('shows one body at a time and marks which tab it belongs to', () => {
    render(WritingResearchPanel)

    const visible = screen.getAllByRole('tabpanel')
    expect(visible).toHaveLength(1)

    const selected = screen.getAllByRole('tab', { selected: true })
    expect(selected).toHaveLength(1)
    expect(selected[0]?.textContent?.trim()).toBe('Corpus')
    expect(visible[0]?.getAttribute('aria-labelledby')).toBe(selected[0]?.id)
  })

  it('switches the body when another tab is chosen', async () => {
    render(WritingResearchPanel)

    await fireEvent.click(screen.getByRole('tab', { name: 'Zotero' }))

    const selected = screen.getAllByRole('tab', { selected: true })
    expect(selected).toHaveLength(1)
    expect(selected[0]?.textContent?.trim()).toBe('Zotero')
    expect(screen.getAllByRole('tabpanel')).toHaveLength(1)
  })

  /**
   * An empty tab that says nothing reads as a defect. Each one names the work
   * it is waiting on, so the shell is legibly unfinished rather than broken.
   */
  it('says what each empty tab is waiting for', async () => {
    render(WritingResearchPanel)

    for (const name of TAB_NAMES) {
      await fireEvent.click(screen.getByRole('tab', { name }))
      expect(screen.getByRole('tabpanel').textContent?.trim()).not.toBe('')
    }
  })
})

/**
 * jsdom performs no layout, so the one defect a rendering test cannot see is
 * the row of tabs spilling out of the panel. `TabList` is `inline-flex` with no
 * width of its own — right for the two or three tabs its other callers have,
 * and one too few for four in a fixed column.
 */
describe('the tab row fits the column', () => {
  const STYLES = readFileSync(
    resolve(import.meta.dirname, 'WritingResearchPanel.svelte'),
    'utf-8'
  ).slice(-2000)

  it('makes the row fill the panel and wrap rather than overflow it', () => {
    const rule = STYLES.slice(STYLES.indexOf('.research__tabs)'))
    const block = rule.slice(0, rule.indexOf('}'))
    expect(block).toMatch(/width:\s*100%/)
    expect(block).toMatch(/flex-wrap:\s*wrap/)
  })
})
