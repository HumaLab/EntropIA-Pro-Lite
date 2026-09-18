import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import WritingResearchPanel from './WritingResearchPanel.svelte'

/**
 * The right-hand panel of the three-panel shell (plan-editor.md 6.3).
 *
 * Its four tabs are filled by later units - Corpus by Unit 4, Notas by 5,
 * Zotero by 6 and Agente by 7 - so what is asserted here is the shell: that
 * every tab exists, that exactly one body is shown, and that the tabs are
 * wired for a screen reader rather than being four styled buttons.
 */

const TAB_NAMES = ['Corpus', 'Zotero', 'Notas', 'Agente', 'Exportar']

vi.mock('$lib/writing-csl', () => ({
  DEFAULT_STYLE: { kind: 'bundled', name: 'apa' },
  isCslError: () => false,
  renderCluster: vi.fn(async () => ({ text: '(Ginzburg, 1976)', author_suppressed: true })),
}))

describe('the research panel', () => {
  it('offers the four tabs 6.3 names, and Export last', () => {
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

  it('keeps a Zotero citation draft while the writer visits another tab', async () => {
    const draft = {
      id: 'z1',
      items: [
        {
          itemKey: 'ABCD1234',
          title: 'Il formaggio e i vermi',
          snapshot: JSON.stringify({ id: 'ABCD1234', title: 'Il formaggio e i vermi' }),
          locator: '',
          locatorType: 'page',
          suppressAuthor: false,
        },
      ],
      affixes: { prefix: '', suffix: '' },
    }

    render(WritingResearchPanel, {
      props: {
        tab: 'zotero',
        citationDraft: draft,
        oncitationdraftchange: (next: Omit<typeof draft, 'id'>) => {
          draft.items = next.items
          draft.affixes = next.affixes
        },
        onapplycitation: vi.fn(),
        oncancelcitation: vi.fn(),
      } as never,
    })

    expect(screen.getByText('Ajustar la cita')).toBeInTheDocument()
    await fireEvent.input(screen.getByLabelText('Localizador'), { target: { value: '88' } })

    await fireEvent.click(screen.getByRole('tab', { name: 'Corpus' }))
    expect(screen.getAllByRole('tab').map((tab) => tab.textContent?.trim())).toEqual(TAB_NAMES)

    await fireEvent.click(screen.getByRole('tab', { name: 'Zotero' }))
    expect(screen.getByLabelText('Localizador')).toHaveValue('88')
  })

  it('remounts the editor when another citation is selected', async () => {
    const first = {
      id: 'z1',
      items: [
        {
          itemKey: 'FIRST',
          title: 'Primera cita',
          snapshot: JSON.stringify({ id: 'FIRST', title: 'Primera cita' }),
          locator: '1',
          locatorType: 'page',
          suppressAuthor: false,
        },
      ],
      affixes: { prefix: '', suffix: '' },
    }
    const second = {
      id: 'z2',
      items: [
        {
          itemKey: 'SECOND',
          title: 'Segunda cita',
          snapshot: JSON.stringify({ id: 'SECOND', title: 'Segunda cita' }),
          locator: '2',
          locatorType: 'chapter',
          suppressAuthor: false,
        },
      ],
      affixes: { prefix: 'cf. ', suffix: '' },
    }
    const callbacks = {
      oncitationdraftchange: vi.fn(),
      onapplycitation: vi.fn(),
      oncancelcitation: vi.fn(),
    }
    const view = render(WritingResearchPanel, {
      props: { tab: 'zotero', citationDraft: first, ...callbacks } as never,
    })

    expect(screen.getByText('Primera cita')).toBeInTheDocument()
    await view.rerender({ tab: 'zotero', citationDraft: second, ...callbacks } as never)

    expect(screen.queryByText('Primera cita')).not.toBeInTheDocument()
    expect(screen.getByText('Segunda cita')).toBeInTheDocument()
    expect(screen.getByLabelText('Localizador')).toHaveValue('2')
  })
})

/**
 * jsdom performs no layout, so the one defect a rendering test cannot see is
 * the row of tabs spilling out of the panel. `TabList` is `inline-flex` with no
 * width of its own — right for the two or three tabs its other callers have,
 * and one too few for four in a fixed column.
 *
 * What holds the row in is NOT wrapping. Each tab is `flex: 1 1 0` with
 * `min-width: 0`, so its base size is zero and the four divide the row between
 * them. An item that starts at zero and may shrink to zero cannot push past its
 * container — which is why `flex-wrap: wrap` was dropped as unreachable rather
 * than kept as a backstop. This asserts the share, not the backstop it retired.
 *
 * The whole file is read, not its last 2000 characters: the rules under test
 * sat 1395 characters from the end, so twenty more lines of CSS below them
 * would have made this fail for a reason that has nothing to do with the row.
 */
describe('the tab row fits the column', () => {
  const STYLES = readFileSync(resolve(import.meta.dirname, 'WritingResearchPanel.svelte'), 'utf-8')

  /** The declarations of one rule, found by a selector fragment unique to it. */
  function ruleFor(selector: string): string {
    const at = STYLES.indexOf(selector)
    expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
    const rule = STYLES.slice(at)
    return rule.slice(0, rule.indexOf('}'))
  }

  it('gives the row the panel width rather than the width of its labels', () => {
    expect(ruleFor('.research__tabs)')).toMatch(/width:\s*100%/)
  })

  /**
   * Five tabs changed the base, not the guarantee. An equal fifth of 280px is
   * narrower than `Exportar`, so each tab now starts from its label and the
   * row grows or shrinks them together. What keeps the row in is unchanged: a
   * tab that may shrink to nothing cannot push past its container.
   */
  it('lets every tab shrink to nothing, so none can reach past its edge', () => {
    const tab = ruleFor('.research__tabs > button)')

    expect(tab).toMatch(/flex:\s*1\s+1\s+auto/)
    expect(tab).toMatch(/min-width:\s*0/)
  })

  it('ends a label that no longer fits with an ellipsis, on one line', () => {
    const tab = ruleFor('.research__tabs > button)')

    expect(tab).toMatch(/overflow:\s*hidden/)
    expect(tab).toMatch(/text-overflow:\s*ellipsis/)
    expect(tab).toMatch(/white-space:\s*nowrap/)
  })
})
