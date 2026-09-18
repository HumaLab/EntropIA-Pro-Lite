import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import { t } from '$lib/i18n'
import WritingExportNotice from './WritingExportNotice.svelte'

/**
 * What a download reports, once the menu that started it has closed.
 *
 * Two different things can go wrong, and they must not read alike. Citation
 * trouble means the file *was* written, with each citation's last rendering.
 * An export failure means nothing was written at all. Showing the second in the
 * words of the first told the writer a file existed that did not — and blamed
 * the citations for a problem that had nothing to do with them.
 */

describe('an export that fails', () => {
  it('says nothing was saved, and does not blame the citations', () => {
    render(WritingExportNotice, {
      props: { outcome: { kind: 'failed', message: 'disk full' }, ondismiss: () => {} },
    })

    const alert = screen.getByRole('alert')
    expect(alert.textContent).toBe(t('writing.exportFailed', { message: 'disk full' }))
    expect(alert.textContent).not.toContain(
      t('writing.exportTrouble', { message: 'disk full' }).split(':')[0]!
    )
    expect(screen.queryByRole('status')).toBeNull()
  })
})

describe('an export DOCX refuses', () => {
  it('names what would have been lost', () => {
    render(WritingExportNotice, {
      props: { outcome: { kind: 'refused', elements: ['footnote'] }, ondismiss: () => {} },
    })

    expect(screen.getByRole('alert').textContent).toBe(
      t('writing.exportRefused', { elements: t('writing.exportElement.footnote') })
    )
  })
})

describe('an export that was saved', () => {
  it('says where', () => {
    render(WritingExportNotice, {
      props: {
        outcome: { kind: 'saved', path: '/out/Capítulo.md', warnings: [], trouble: null },
        ondismiss: () => {},
      },
    })

    expect(screen.getByRole('status').textContent?.trim()).toBe(
      t('writing.exportSaved', { path: '/out/Capítulo.md' })
    )
  })

  /**
   * Paragraph formatting is reported like a mark: how many blocks go out some
   * other way. The same attribute can go out one way in the body and not at all
   * in a Markdown table cell, and the notice says both.
   */
  it('reports what the format stands in for and what it drops, as separate lines', () => {
    const element = t('writing.exportElement.textAlign')
    render(WritingExportNotice, {
      props: {
        outcome: {
          kind: 'saved',
          path: '/out/Capítulo.md',
          warnings: [
            { element: 'textAlign', kind: 'attribute', support: 'fallback', count: 2 },
            { element: 'textAlign', kind: 'attribute', support: 'unsupported', count: 1 },
          ],
          trouble: null,
        },
        ondismiss: () => {},
      },
    })

    const lines = [...document.querySelectorAll('.export-notice__warning')].map(
      (item) => item.textContent
    )
    expect(lines).toEqual([
      t('writing.exportSubstituted', { element, count: '2' }),
      t('writing.exportDropped', { element, count: '1' }),
    ])
  })

  it('names a citation representation by the label the Export tab gives it', () => {
    render(WritingExportNotice, {
      props: {
        outcome: {
          kind: 'saved',
          path: '/out/Capítulo.md',
          warnings: [{ element: 'comment', kind: 'citation', support: 'fallback', count: 1 }],
          trouble: null,
        },
        ondismiss: () => {},
      },
    })

    expect(document.querySelector('.export-notice__warning')?.textContent).toBe(
      t('writing.exportSubstituted', { element: t('writing.exportCiteComment'), count: '1' })
    )
  })

  it('reports citation trouble on a file that was written', () => {
    render(WritingExportNotice, {
      props: {
        outcome: { kind: 'saved', path: '/out/a.docx', warnings: [], trouble: 'csl down' },
        ondismiss: () => {},
      },
    })

    expect(screen.getByRole('alert').textContent).toBe(
      t('writing.exportTrouble', { message: 'csl down' })
    )
    expect(screen.getByRole('status')).toBeInTheDocument()
  })
})

describe('the notice', () => {
  it('can be dismissed', async () => {
    const ondismiss = vi.fn()
    render(WritingExportNotice, {
      props: { outcome: { kind: 'failed', message: 'x' }, ondismiss },
    })

    await fireEvent.click(screen.getByRole('button', { name: t('writing.exportClose') }))

    expect(ondismiss).toHaveBeenCalledOnce()
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
