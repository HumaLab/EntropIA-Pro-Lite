import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import { t } from '$lib/i18n'
import { DEFAULT_EXPORT_PREFERENCES } from '$lib/export-preferences'
import WritingExportTab from './WritingExportTab.svelte'

/**
 * The Export tab of the research panel: how a manuscript is exported, not in
 * what format. It holds preferences and nothing else — no button starts an
 * export from here; the download button in the bar does that.
 */

const CHOICES = [
  'writing.exportCiteFootnote',
  'writing.exportCiteInline',
  'writing.exportCiteComment',
  'writing.exportCiteQuote',
] as const

function renderTab(preferences = DEFAULT_EXPORT_PREFERENCES) {
  const onchange = vi.fn()
  render(WritingExportTab, { props: { preferences, onchange } })
  return onchange
}

describe('the corpus citation choice', () => {
  it('offers the four representations as one group of exclusive choices', () => {
    renderTab()

    const group = screen.getByRole('radiogroup', { name: t('writing.exportCitations') })
    const radios = [...group.querySelectorAll('input[type="radio"]')]
    expect(
      radios.map(
        (radio) => radio.getAttribute('aria-label') ?? radio.closest('label')?.textContent?.trim()
      )
    ).toEqual(CHOICES.map((key) => t(key)))
  })

  it('starts on the quoted text with a note, and the bibliography included', () => {
    renderTab()

    expect(screen.getByRole('radio', { name: t('writing.exportCiteQuote') })).toBeChecked()
    expect(screen.getByRole('radio', { name: t('writing.exportCiteFootnote') })).not.toBeChecked()
    expect(
      screen.getByRole('checkbox', { name: t('writing.exportWithBibliography') })
    ).toBeChecked()
  })

  it('shows the saved choice rather than the default', () => {
    renderTab({ citations: 'footnote', bibliography: false })

    expect(screen.getByRole('radio', { name: t('writing.exportCiteFootnote') })).toBeChecked()
    expect(
      screen.getByRole('checkbox', { name: t('writing.exportWithBibliography') })
    ).not.toBeChecked()
  })

  it('reports a new choice with the bibliography left as it was', async () => {
    const onchange = renderTab({ citations: 'quote_with_note', bibliography: false })

    await fireEvent.click(screen.getByRole('radio', { name: t('writing.exportCiteFootnote') }))

    expect(onchange).toHaveBeenCalledWith({ citations: 'footnote', bibliography: false })
  })

  it('says what Markdown does with a comment, only when a comment is chosen', () => {
    renderTab({ citations: 'comment', bibliography: true })
    expect(screen.getByText(t('writing.exportCiteMarkdownComment'))).toBeInTheDocument()
  })

  it('does not mention Markdown otherwise', () => {
    renderTab()
    expect(screen.queryByText(t('writing.exportCiteMarkdownComment'))).toBeNull()
  })
})

describe('the bibliography checkbox', () => {
  it('reports the change with the citation choice left as it was', async () => {
    const onchange = renderTab({ citations: 'inline', bibliography: true })

    await fireEvent.click(
      screen.getByRole('checkbox', { name: t('writing.exportWithBibliography') })
    )

    expect(onchange).toHaveBeenCalledWith({ citations: 'inline', bibliography: false })
  })

  it('says what it does on screen, not only to a screen reader', () => {
    renderTab()
    const box = screen.getByRole('checkbox', { name: t('writing.exportWithBibliography') })
    expect(box.closest('label')?.textContent?.trim()).toBe(t('writing.exportWithBibliography'))
  })
})

describe('what the tab does not hold', () => {
  it('has no format choice and no button that exports', () => {
    renderTab()

    expect(screen.queryAllByRole('button')).toEqual([])
    expect(screen.queryByText('Markdown')).toBeNull()
    expect(screen.queryByText(/DOCX/)).toBeNull()
  })
})

/**
 * jsdom performs no layout, so the reflow is a fact about the source. The
 * choices use the Agent tab's own grid: across the panel's 200–560 range the
 * 120px floor steps 1 / 2 / 3 columns, two at the default 280.
 */
describe('the choices reflow with the panel', () => {
  const STYLES = readFileSync(
    resolve(import.meta.dirname, 'WritingExportTab.svelte'),
    'utf-8'
  ).replace(/\/\*[\s\S]*?\*\//g, '')

  function ruleFor(selector: string): string {
    const at = STYLES.indexOf(selector)
    expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
    const rule = STYLES.slice(at)
    return rule.slice(0, rule.indexOf('}'))
  }

  it('lays them out in as many columns as fit, as the Agent tab does', () => {
    expect(ruleFor('.export-tab__choices {')).toMatch(
      /grid-template-columns:\s*repeat\(auto-fit,\s*minmax\(120px,\s*1fr\)\)/
    )
  })

  it('wraps a long label instead of cutting it', () => {
    expect(ruleFor('.export-tab__label {')).not.toMatch(/white-space:\s*nowrap/)
  })
})

/**
 * Informative only: counts of the manuscript on screen, under the bibliography
 * checkbox. Nothing here changes an export.
 */
describe('the statistics', () => {
  const manuscript = {
    type: 'doc',
    content: [
      { type: 'heading', attrs: { level: 1 }, content: [{ type: 'text', text: 'Título' }] },
      { type: 'paragraph', content: [{ type: 'text', text: 'Hola mundo.' }] },
    ],
  }

  function value(label: string): string | undefined {
    const term = screen.getByText(label, { selector: 'dt' })
    return term.nextElementSibling?.textContent?.trim()
  }

  it('show the counts of the manuscript', () => {
    render(WritingExportTab, {
      props: { preferences: DEFAULT_EXPORT_PREFERENCES, onchange: vi.fn(), doc: manuscript },
    })

    expect(screen.getByText(t('writing.stats.title'))).toBeInTheDocument()
    expect(value(t('writing.stats.words'))).toBe('3')
    expect(value(t('writing.stats.characters'))).toBe('17')
    expect(value(t('writing.stats.charactersNoSpaces'))).toBe('16')
    expect(value(t('writing.stats.paragraphs'))).toBe('2')
    expect(value(t('writing.stats.footnotes'))).toBe('0')
  })

  it('are not shown when there is no manuscript to count', () => {
    renderTab()
    expect(screen.queryByText(t('writing.stats.title'))).toBeNull()
  })
})
