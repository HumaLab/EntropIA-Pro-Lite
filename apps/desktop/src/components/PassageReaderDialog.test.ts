import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import PassageReaderDialog from './PassageReaderDialog.svelte'

// pdf.js needs a real engine: what matters is the file and page it is given.
vi.mock('@entropia/ui', async () => {
  const actual = await vi.importActual<typeof import('@entropia/ui')>('@entropia/ui')
  const MockDocumentViewer = (await import('../views/__mocks__/MockDocumentViewer.svelte')).default
  return { ...actual, DocumentViewer: MockDocumentViewer }
})

const mockInvoke = vi.mocked(invoke)

const pageText = 'Antes del pasaje. LO CITADO AQUÍ. Después del pasaje.'
const start = pageText.indexOf('LO CITADO')
const end = start + 'LO CITADO AQUÍ.'.length

const context = (patch: Record<string, unknown> = {}) => ({
  chunkId: 'chunk-2',
  itemId: 'item-2',
  itemKey: 'ABCD1234',
  title: 'Apología',
  text: 'LO CITADO AQUÍ.',
  spans: [[3, start, end]],
  pages: [{ pageNumber: 3, text: pageText, highlights: [[start, end]] }],
  originalKind: 'pdf',
  originalPath: null,
  openError: null,
  ...patch,
})

function backend(open: () => unknown, passage: unknown = context()) {
  mockInvoke.mockImplementation((async (command: string) => {
    if (command === 'bibliography_passage_context') return passage
    if (command === 'bibliography_open_passage') return open()
    throw new Error(`unexpected command: ${command}`)
  }) as typeof invoke)
}

async function renderReader() {
  render(PassageReaderDialog, {
    chunkId: 'chunk-2',
    title: 'Apología',
    heading: 'Autor · 2020 · p. 3 · Mi biblioteca',
    fallbackSnippet: 'fragmento',
    onclose: vi.fn(),
  })
  const dialog = await screen.findByRole('dialog')
  await waitFor(() => expect(within(dialog).getByText('LO CITADO AQUÍ.')).toBeInTheDocument())
  return dialog
}

describe('PassageReaderDialog original', () => {
  beforeEach(() => {
    locale.set('es')
    mockInvoke.mockReset()
  })

  it('opens the PDF inside the app at the cited page, through the granted asset URL', async () => {
    const path = String.raw`C:\Users\a\Zotero\storage\KEY\obra, 2020.pdf`
    backend(() => context({ originalPath: path }))
    const dialog = await renderReader()

    await fireEvent.click(within(dialog).getByRole('button', { name: 'Abrir original' }))

    const viewer = await screen.findByTestId('mock-document-viewer')
    expect(viewer).toHaveAttribute('data-path', path)
    expect(viewer).toHaveAttribute('data-asset-url', convertFileSrc(path))
    expect(screen.getByTestId('viewer-type')).toHaveTextContent('pdf')
    expect(screen.getByTestId('viewer-current-page')).toHaveTextContent('3')
    expect(screen.getByTestId('viewer-current-page')).not.toHaveTextContent('1')
    // The cited text is marked in the page text beside the PDF.
    const marks = document.querySelectorAll('mark')
    expect([...marks].map((mark) => mark.textContent)).toContain('LO CITADO AQUÍ.')
  })

  it('shows an HTML snapshot as stored text scrolled to the cited paragraph, with no file', async () => {
    backend(() => context({ originalKind: 'html', originalPath: null }))
    const dialog = await renderReader()

    await fireEvent.click(within(dialog).getByRole('button', { name: 'Abrir original' }))

    await waitFor(() => expect(screen.getAllByText('LO CITADO AQUÍ.').length).toBeGreaterThan(0))
    expect(screen.queryByTestId('mock-document-viewer')).toBeNull()
    expect(screen.getByText(/Antes del pasaje\./)).toBeInTheDocument()
  })

  it('says why when the original cannot be shown, and never shows a viewer', async () => {
    backend(() =>
      context({ originalKind: null, openError: 'not_a_file: not a regular file: C:/Zotero' })
    )
    const dialog = await renderReader()

    await fireEvent.click(within(dialog).getByRole('button', { name: 'Abrir original' }))

    expect(await screen.findByText(/No se pudo abrir el original: not_a_file/)).toBeVisible()
    expect(screen.queryByTestId('mock-document-viewer')).toBeNull()
  })

  it('closes the viewer back to the reader', async () => {
    backend(() => context({ originalPath: 'C:\\x\\a.pdf' }))
    const dialog = await renderReader()
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Abrir original' }))
    await screen.findByTestId('mock-document-viewer')

    await fireEvent.click(screen.getByRole('button', { name: 'Cerrar original' }))

    await waitFor(() => expect(screen.queryByTestId('mock-document-viewer')).toBeNull())
    expect(screen.getByRole('button', { name: 'Abrir original' })).toBeInTheDocument()
  })
})
