import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import BibliographyWorkView from './BibliographyWorkView.svelte'

const mockInvoke = vi.mocked(invoke)

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

    expect(screen.getByText('10.0000/synth')).toBeInTheDocument()
    expect(screen.getByText('oficio.pdf')).toBeInTheDocument()
    expect(screen.getByText('apendice.pdf')).toBeInTheDocument()

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
