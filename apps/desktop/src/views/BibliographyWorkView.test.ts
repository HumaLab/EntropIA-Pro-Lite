import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import BibliographyWorkView from './BibliographyWorkView.svelte'

const mockInvoke = vi.mocked(invoke)

const pdfMock = vi.hoisted(() => ({ getDocument: vi.fn() }))

// The viewer must open the PDF exactly once however often the tabs change,
// so the load is counted here instead of letting real pdf.js fetch.
vi.mock('pdfjs-dist', () => ({
  getDocument: pdfMock.getDocument,
  GlobalWorkerOptions: { workerSrc: '' },
}))

const props = {
  itemId: 'item-1',
  itemKey: 'AAAA1111',
  libraryRowId: 'lib-row-1',
  title: 'El oficio de historiador',
}

function attachment(over: Record<string, unknown> = {}) {
  return {
    attachmentKey: 'ATT1',
    contentType: 'application/pdf',
    linkMode: 'imported_file',
    filename: 'oficio.pdf',
    url: null,
    ...over,
  }
}

function detail(over: Record<string, unknown> = {}) {
  return {
    itemId: 'item-1',
    itemKey: 'AAAA1111',
    title: 'El oficio de historiador',
    authors: 'Bloch',
    year: 1949,
    libraryName: 'Mi biblioteca',
    libraryType: 'user',
    libraryNativeId: '0',
    cslJson: '{}',
    item: {
      itemKey: 'AAAA1111',
      itemType: 'book',
      title: 'El oficio de historiador',
      creators: [{ creatorType: 'author', firstName: 'Marc', lastName: 'Bloch' }],
      publicationTitle: 'Annales',
      publisher: 'Alcan',
      date: '1949',
      doi: '10.0000/synth',
      isbn: null,
      abstract: 'Un ensayo sobre el oficio.',
      language: 'fr',
      url: 'https://example.invalid/oficio',
      itemVersion: 3,
      collections: ['Historia'],
      tags: ['metodología'],
      attachments: [attachment()],
    },
    ...over,
  }
}

function opened(over: Record<string, unknown> = {}) {
  return {
    itemId: 'item-1',
    itemKey: 'AAAA1111',
    title: 'El oficio de historiador',
    attachmentKey: 'ATT1',
    originalKind: 'pdf',
    originalPath: 'C:/zotero/storage/ATT1/oficio.pdf',
    openError: null,
    pages: [{ pageNumber: 1, method: 'native', quality: 'rich', text: 'Primera página.' }],
    snapshotText: '',
    extracted: true,
    ...over,
  }
}

/** Answers each command on its own, as the real backend does. */
function backend(
  options: {
    detail?: unknown
    detailError?: boolean
    open?: Record<string, unknown> | ((attachmentKey: string) => Record<string, unknown>)
  } = {}
) {
  mockInvoke.mockImplementation(async (command: string, payload?: unknown) => {
    switch (command) {
      case 'bibliography_work_detail':
        if (options.detailError) throw new Error('boom')
        return options.detail ?? detail()
      case 'bibliography_open_work_attachment': {
        const key = (payload as { attachmentKey: string }).attachmentKey
        return typeof options.open === 'function' ? options.open(key) : (options.open ?? opened())
      }
      default:
        throw new Error(`unexpected command ${command}`)
    }
  })
}

function callsFor(command: string) {
  return mockInvoke.mock.calls.filter(([name]) => name === command)
}

beforeEach(() => {
  mockInvoke.mockReset()
  locale.set('es')
  pdfMock.getDocument.mockReset()
  pdfMock.getDocument.mockImplementation(() => ({
    promise: Promise.resolve({
      numPages: 1,
      getPage: () =>
        Promise.resolve({
          getViewport: ({ scale }: { scale: number }) => ({
            width: 800 * scale,
            height: 600 * scale,
            scale,
          }),
          render: () => ({ promise: Promise.resolve(), cancel: () => {} }),
        }),
    }),
    destroy: () => Promise.resolve(),
  }))
})

describe('BibliographyWorkView', () => {
  it('shows the work header and opens the first attachment in the Original tab', async () => {
    backend()
    render(BibliographyWorkView, { props })

    expect(await screen.findByText('El oficio de historiador')).toBeInTheDocument()
    expect(screen.getByText('Bloch · 1949')).toBeInTheDocument()
    expect(screen.getByText('Mi biblioteca')).toBeInTheDocument()

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('bibliography_open_work_attachment', {
        itemId: 'item-1',
        attachmentKey: 'ATT1',
      })
    })
    expect(await screen.findByTestId('work-original-viewer')).toBeInTheDocument()
    expect(screen.getByText('Se muestra el PDF original del adjunto.')).toBeInTheDocument()
  })

  it('hosts the viewer in the fill-height Original pane', async () => {
    // jsdom cannot measure heights, so the fill-height contract is pinned by
    // the structure that carries it (the same chain ItemAssetPanel uses for
    // ItemView): .work-view (flex column, min-height: 100% of the WorkPane
    // body) > section.work-section--original (flex: 1; min-height: 0) >
    // .work-viewer (display: flex; flex: 1) > :global(.document-viewer)
    // (flex: 1; min-height: 0) — the DocumentViewer root must be a flex
    // child of the viewer box, never a percentage child of a min-height-only
    // frame (that collapsed the PDF to the toolbar's ~20px).
    backend()
    render(BibliographyWorkView, { props })

    const viewer = await screen.findByTestId('work-original-viewer')
    expect(viewer.classList.contains('work-viewer')).toBe(true)
    expect(viewer.parentElement?.classList.contains('work-section--original')).toBe(true)
    expect(viewer.closest('.work-view')).not.toBeNull()
    expect(viewer.querySelector('.document-viewer')).not.toBeNull()
  })

  it('gives the viewer a definite height only while the Original tab shows a PDF', async () => {
    // A min-height chain is not a definite height: the viewer measured ~0
    // and the page stayed hidden behind a clipped toolbar. While a PDF is
    // shown, the view takes the pane's full height (like ItemView's
    // `height: 100%`); the reading tabs keep their natural flow and the
    // pane scrolls them.
    backend()
    render(BibliographyWorkView, { props })

    const viewer = await screen.findByTestId('work-original-viewer')
    expect(viewer.closest('.work-view')?.classList.contains('work-view--fill')).toBe(true)

    await fireEvent.click(screen.getByRole('tab', { name: 'Texto' }))
    await waitFor(() =>
      expect(document.querySelector('.work-view')?.classList.contains('work-view--fill')).toBe(
        false
      )
    )
  })

  it('pins the fill-height rules in the component styles', () => {
    // jsdom does not lay out, so the CSS that makes the height definite is
    // pinned on the source itself.
    const source = readFileSync(
      resolve(import.meta.dirname, 'BibliographyWorkView.svelte'),
      'utf-8'
    )
    const rule = (selector: string) =>
      source.match(new RegExp(`\\n  ${selector.replace(/[.]/g, '\\.')} \\{([^}]*)\\}`))?.[1] ?? ''
    expect(rule('.work-view--fill')).toMatch(/height: 100%;/)
    expect(rule('.work-view--fill')).not.toMatch(/min-height: 100%/)
    expect(rule('.work-section--original')).toMatch(/flex: 1;/)
    expect(rule('.work-section--original')).toMatch(/min-height: 0;/)
    expect(rule('.work-viewer')).toMatch(/flex: 1;/)
    expect(rule('.work-viewer')).toMatch(/min-height: 0;/)
  })

  it('keeps the Original viewer mounted and does not reopen the PDF across tabs', async () => {
    backend()
    render(BibliographyWorkView, { props })

    await screen.findByTestId('work-original-viewer')
    await waitFor(() => expect(pdfMock.getDocument).toHaveBeenCalledTimes(1))

    await fireEvent.click(screen.getByRole('tab', { name: 'Texto' }))
    await fireEvent.click(screen.getByRole('tab', { name: 'Original' }))

    expect(await screen.findByTestId('work-original-viewer')).toBeInTheDocument()
    expect(pdfMock.getDocument).toHaveBeenCalledTimes(1)
  })

  it('hides the mounted Original tab from focus and the accessibility tree', async () => {
    backend()
    render(BibliographyWorkView, { props })

    const viewer = await screen.findByTestId('work-original-viewer')
    const section = viewer.closest('section')
    expect(section).not.toBeNull()

    await fireEvent.click(screen.getByRole('tab', { name: 'Texto' }))

    // Still mounted (the document stays open) but gone for users, screen
    // readers and the tab order.
    expect(document.querySelector('[data-testid="work-original-viewer"]')).not.toBeNull()
    expect(section).toHaveClass('is-hidden')
    expect(section).not.toBeVisible()
    expect(screen.queryByRole('region', { name: 'Original' })).toBeNull()
    expect(section?.contains(document.activeElement)).toBe(false)

    await fireEvent.click(screen.getByRole('tab', { name: 'Original' }))
    expect(section).toBeVisible()
  })

  it('pins the hidden-tab CSS and the paused viewer wiring in the source', () => {
    const source = readFileSync(
      resolve(import.meta.dirname, 'BibliographyWorkView.svelte'),
      'utf-8'
    )
    // The hidden tab must actually be invisible: `.work-section` sets
    // display: flex, which would beat the UA rule behind the `hidden`
    // attribute without this rule.
    const hiddenRule = source.match(
      new RegExp('\\n {2}\\.work-section--original\\.is-hidden \\{([^}]*)\\}')
    )?.[1]
    expect(hiddenRule ?? '').toMatch(/display: none;/)
    // Only the Biblioteca viewer opts into paused rendering while hidden.
    const viewerTag = source.match(/<DocumentViewer[\s\S]*?\/>/)?.[0] ?? ''
    expect(viewerTag).toContain('pauseWhenHidden')
  })

  it('lists the extracted page texts in the Texto tab', async () => {
    backend()
    render(BibliographyWorkView, { props })

    await screen.findByTestId('work-original-viewer')
    await fireEvent.click(screen.getByRole('tab', { name: 'Texto' }))

    expect(screen.getByText('Página 1')).toBeInTheDocument()
    expect(screen.getByText('Primera página.')).toBeInTheDocument()
  })

  it('says the text is pending when nothing was extracted yet', async () => {
    backend({ open: opened({ pages: [], snapshotText: '', extracted: false }) })
    render(BibliographyWorkView, { props })

    await screen.findByTestId('work-original-viewer')
    await fireEvent.click(screen.getByRole('tab', { name: 'Texto' }))

    expect(
      screen.getByText('El texto extraído aparecerá cuando esté disponible.')
    ).toBeInTheDocument()
  })

  it('renders the stored snapshot text for an HTML original', async () => {
    backend({
      open: opened({
        originalKind: 'html',
        originalPath: null,
        pages: [],
        snapshotText: 'Texto de la captura.',
        extracted: true,
      }),
    })
    render(BibliographyWorkView, { props })

    expect(await screen.findByText('Texto de la captura.')).toBeInTheDocument()
    expect(screen.getByText('Se muestra el texto guardado de la captura HTML.')).toBeInTheDocument()
  })

  it('shows why the original cannot open, as a notice', async () => {
    backend({
      open: opened({
        originalKind: null,
        originalPath: null,
        openError: 'linked_file_missing: linked file moved or unreadable',
      }),
    })
    render(BibliographyWorkView, { props })

    expect(
      await screen.findByText('linked_file_missing: linked file moved or unreadable')
    ).toBeInTheDocument()
    expect(screen.queryByTestId('work-original-viewer')).toBeNull()
  })

  it('lists the attachments under Metadatos with a way to open each', async () => {
    backend({
      detail: detail({
        item: {
          ...detail().item,
          attachments: [
            attachment(),
            attachment({ attachmentKey: 'ATT2', filename: 'apendice.pdf' }),
          ],
        },
      }),
      open: (key: string) => opened({ attachmentKey: key }),
    })
    render(BibliographyWorkView, { props })

    await screen.findByTestId('work-original-viewer')
    await fireEvent.click(screen.getByRole('tab', { name: 'Metadatos' }))

    // Scoped to the Metadatos region: the now-mounted (hidden) Original tab
    // also names the current attachment.
    const metadata = within(screen.getByRole('region', { name: 'Metadatos' }))
    expect(metadata.getByText('10.0000/synth')).toBeInTheDocument()
    expect(metadata.getByText('oficio.pdf')).toBeInTheDocument()
    expect(metadata.getByText('apendice.pdf')).toBeInTheDocument()

    await fireEvent.click(screen.getAllByRole('button', { name: 'Ver en Original' })[1]!)
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('bibliography_open_work_attachment', {
        itemId: 'item-1',
        attachmentKey: 'ATT2',
      })
    })
  })

  it('offers the attachment picker in the Original tab when several are cataloged', async () => {
    backend({
      detail: detail({
        item: {
          ...detail().item,
          attachments: [
            attachment(),
            attachment({ attachmentKey: 'ATT2', filename: 'apendice.pdf' }),
          ],
        },
      }),
      open: (key: string) => opened({ attachmentKey: key }),
    })
    render(BibliographyWorkView, { props })

    const trigger = await screen.findByRole('button', { name: /Elegir adjunto/ })
    await fireEvent.click(trigger)
    await fireEvent.click(await screen.findByRole('menuitemradio', { name: /apendice\.pdf/ }))

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('bibliography_open_work_attachment', {
        itemId: 'item-1',
        attachmentKey: 'ATT2',
      })
    })
  })

  it('says when the work could not be loaded, with a retry', async () => {
    backend({ detailError: true })
    render(BibliographyWorkView, { props })

    expect(await screen.findByText('No se pudo cargar la obra.')).toBeInTheDocument()
    await fireEvent.click(screen.getByRole('button', { name: 'Reintentar' }))
    await waitFor(() => {
      expect(callsFor('bibliography_work_detail').length).toBeGreaterThan(1)
    })
  })
})
