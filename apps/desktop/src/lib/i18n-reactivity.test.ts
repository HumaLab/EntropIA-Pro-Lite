import { render, screen } from '@testing-library/svelte'
import { tick } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { locale, t } from './i18n'
import WritingExportTab from '../views/WritingExportTab.svelte'
import WritingResearchPanel from '../views/WritingResearchPanel.svelte'

/**
 * Changing the language changes what is already on screen.
 *
 * Nothing remounts the views when the language changes, so a view that was
 * open stayed in the language it was opened in — the writing workspace kept
 * "Volver a los documentos" and "Esquema" after the top bar had switched to
 * English. `t` itself is what a template depends on, so `t` is what has to be
 * reactive; fixing it per view would leave every view that does not know to.
 */

vi.mock('$lib/writing-csl', () => ({
  DEFAULT_STYLE: { kind: 'bundled', name: 'apa' },
  isCslError: () => false,
  renderCluster: vi.fn(),
}))

afterEach(() => locale.set('es'))

describe('a view already on screen', () => {
  it('follows the language when it changes', async () => {
    locale.set('es')
    render(WritingExportTab, {
      props: {
        preferences: { citations: 'footnote', bibliography: true },
        onchange: vi.fn(),
      },
    })
    expect(screen.getByText('Citas del corpus')).toBeInTheDocument()

    locale.set('en')
    await tick()

    expect(screen.getByText('Corpus citations')).toBeInTheDocument()
    expect(screen.queryByText('Citas del corpus')).toBeNull()
  })
})

describe('the research tabs in English', () => {
  it('are English, apart from the two proper names', () => {
    locale.set('en')
    render(WritingResearchPanel)

    expect(screen.getAllByRole('tab').map((tab) => tab.textContent?.trim())).toEqual([
      'Corpus',
      'Zotero',
      'Notes',
      'Agent',
      'Export',
    ])
  })

  it('keep their Spanish names in Spanish', () => {
    expect(t('writing.tab.notes')).toBe('Notas')
    expect(t('writing.tab.agent')).toBe('Agente')
  })
})
