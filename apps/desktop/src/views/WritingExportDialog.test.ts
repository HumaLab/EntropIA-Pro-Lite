import { fireEvent, render, screen } from '@testing-library/svelte'
import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { t } from '$lib/i18n'
import WritingExportDialog from './WritingExportDialog.svelte'

/**
 * What the export dialog says when an export goes wrong.
 *
 * Two different things can go wrong, and they must not read alike. Citation
 * trouble means the file *was* written, with each citation's last rendering.
 * An export failure means nothing was written at all. Showing the second in the
 * words of the first told the writer a file existed that did not — and blamed
 * the citations for a problem that had nothing to do with them.
 */

const doc = {
  type: 'doc',
  content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Sin citas.' }] }],
}

beforeEach(() => {
  vi.mocked(save).mockReset()
  vi.mocked(writeFile).mockReset()
})

async function exportIt() {
  render(WritingExportDialog, { props: { doc, title: 'Capítulo', onclose: () => {} } })
  await fireEvent.click(screen.getByRole('button', { name: t('writing.exportAction') }))
  return screen.findByRole('alert')
}

describe('an export that fails', () => {
  it('says nothing was saved, and does not blame the citations', async () => {
    vi.mocked(save).mockResolvedValue('/out/Capítulo.docx')
    vi.mocked(writeFile).mockRejectedValue(new Error('disk full'))

    const alert = await exportIt()

    expect(alert.textContent).toBe(t('writing.exportFailed', { message: 'disk full' }))
    expect(alert.textContent).not.toContain(
      t('writing.exportTrouble', { message: 'disk full' }).split(':')[0]!
    )
    expect(screen.queryByRole('status')).toBeNull()
  })
})

/**
 * Paragraph formatting is reported like a mark: how many blocks go out some
 * other way. The same attribute can go out one way in the body and not at all
 * in a Markdown table cell, and the dialog says both.
 */
describe('the paragraph formatting warnings', () => {
  it('reports what Markdown stands in for and what it drops, as two lines', async () => {
    vi.mocked(save).mockResolvedValue('/out/Capítulo.md')
    vi.mocked(writeFile).mockResolvedValue()
    const centered = {
      type: 'paragraph',
      attrs: { textAlign: 'center' },
      content: [{ type: 'text', text: 'x' }],
    }
    const formatted = {
      type: 'doc',
      content: [
        centered,
        centered,
        {
          type: 'table',
          content: [{ type: 'tableRow', content: [{ type: 'tableCell', content: [centered] }] }],
        },
      ],
    }

    render(WritingExportDialog, { props: { doc: formatted, title: 'Capítulo', onclose: () => {} } })
    await fireEvent.click(screen.getByRole('button', { name: t('writing.exportMarkdown') }))
    await fireEvent.click(screen.getByRole('button', { name: t('writing.exportAction') }))
    await screen.findByRole('status')

    const lines = [...document.querySelectorAll('.export__warning')].map((item) => item.textContent)
    const element = t('writing.exportElement.textAlign')
    expect(lines).toEqual([
      t('writing.exportSubstituted', { element, count: '2' }),
      t('writing.exportDropped', { element, count: '1' }),
    ])
  })
})

describe('both locales', () => {
  it('name the failure and say no file was saved', async () => {
    const { locale } = await import('$lib/i18n')
    locale.set('es')
    expect(t('writing.exportFailed', { message: 'x' })).toBe(
      'No se pudo exportar el documento: x. No se guardó ningún archivo.'
    )
    locale.set('en')
    expect(t('writing.exportFailed', { message: 'x' })).toBe(
      'The document could not be exported: x. No file was saved.'
    )
    locale.set('es')
  })
})

describe('the bibliography checkbox', () => {
  it('says what it does on screen, not only to a screen reader', () => {
    render(WritingExportDialog, { props: { doc, title: 'Capítulo', onclose: () => {} } })
    const box = screen.getByRole('checkbox', { name: t('writing.exportWithBibliography') })
    expect(box.closest('label')?.textContent?.trim()).toBe(t('writing.exportWithBibliography'))
  })
})

describe('the corpus citation choice', () => {
  /**
   * The default reads as the editor does: the quoted fragment stays in the
   * text, and a note says where it came from. Footnote-only moved the quotation
   * out of the sentence, which surprised the writer who had placed it there.
   */
  it('starts on the quoted text with a note', () => {
    render(WritingExportDialog, { props: { doc, title: 'Capítulo', onclose: () => {} } })
    const chosen = screen.getByRole('button', { name: t('writing.exportCiteQuote') })
    const footnote = screen.getByRole('button', { name: t('writing.exportCiteFootnote') })
    expect(chosen.className).toContain('btn--secondary')
    expect(footnote.className).not.toContain('btn--secondary')
  })
})
