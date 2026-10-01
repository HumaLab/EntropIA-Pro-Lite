import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { locale } from '$lib/i18n'
import { navegadorSession } from '$lib/navegador'
import { navegadorStore } from '$lib/navegador-store'
import type { DownloadDraft } from '$lib/navegador-capture'
import type { CaptureDetail, SourceDetail, SourceSummary } from '$lib/navegador-sources'
import type { BrowserState } from '$lib/navegador-tabs'
import NavegadorView from './NavegadorView.svelte'

// The corpus viewer (pdf.js) needs a real engine: what matters here is what it is
// given, so the same stand-in the other viewer dialogs use takes its place.
vi.mock('@entropia/ui', async () => {
  const actual = await vi.importActual<typeof import('@entropia/ui')>('@entropia/ui')
  const MockDocumentViewer = (await import('./__mocks__/MockDocumentViewer.svelte')).default
  return { ...actual, DocumentViewer: MockDocumentViewer }
})

const SHA = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
const PDF_PATH = 'C:/data/web-captures/beta/c3.pdf'

const summary = (id: string, patch: Partial<SourceSummary> = {}): SourceSummary => ({
  id,
  title: `Title of ${id}`,
  finalUrl: `https://www.${id}.example.org/page`,
  siteName: null,
  updatedAt: 1_790_000_000_000,
  captureCount: 1,
  kinds: ['page'],
  ...patch,
})

const capture = (id: string, patch: Partial<CaptureDetail> = {}): CaptureDetail => ({
  id,
  kind: 'page',
  mimeType: 'text/html',
  accessedAt: '2026-09-30T12:00:00Z',
  finalUrl: 'https://alpha.example.org/page',
  title: 'Alpha',
  sha256: SHA,
  hashOf: 'html',
  sizeBytes: 2048,
  textPreview: null,
  textInFile: false,
  quotePrefix: null,
  quoteSuffix: null,
  filePresent: true,
  createdAt: 1,
  ...patch,
})

const pdfCapture = (id: string, patch: Partial<CaptureDetail> = {}) =>
  capture(id, {
    kind: 'pdf',
    hashOf: 'pdf',
    mimeType: 'application/pdf',
    finalUrl: 'https://www.beta.example.org/articulos/227/descargar',
    ...patch,
  })

const detailOf = (id: string, captures: CaptureDetail[]): SourceDetail => ({
  id,
  originalUrl: `https://www.${id}.example.org/page`,
  finalUrl: `https://www.${id}.example.org/page`,
  canonicalUrl: null,
  title: `Title of ${id}`,
  siteName: null,
  firstAccessedAt: '2026-09-29T08:00:00Z',
  createdAt: 1,
  updatedAt: 2,
  captures,
})

const browser = (url: string | null): BrowserState => ({
  tabs: url ? [{ id: 1, url, title: 'A page', blocked: null }] : [],
  active: url ? 1 : null,
  revision: 0,
})

const download = (id: string, patch: Partial<DownloadDraft> = {}): DownloadDraft => ({
  id,
  url: 'https://www.beta.example.org/articulos/227/descargar',
  fileName: 'paper.pdf',
  size: 1536,
  sha256: SHA,
  savedTo: null,
  accessedAt: '2026-09-30T12:00:00Z',
  status: 'ready',
  reason: null,
  tab: null,
  pageUrl: 'https://www.beta.example.org/page',
  pageTitle: 'Title of beta',
  alreadySavedIn: null,
  ...patch,
})

let respond: (command: string, args?: unknown) => unknown
let details: Record<string, SourceDetail | null>
let handlers: Record<string, (event: { payload: unknown }) => void>

const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command)

beforeEach(async () => {
  locale.set('es')
  handlers = {}
  details = {
    alpha: detailOf('alpha', [capture('c1')]),
    beta: detailOf('beta', [pdfCapture('c3')]),
    gamma: detailOf('gamma', [pdfCapture('c4', { filePresent: false })]),
  }
  respond = (command, args) => {
    if (command === 'navegador_state') return browser(null)
    if (command === 'navegador_list_sources') {
      return [
        summary('alpha'),
        summary('beta', { kinds: ['pdf'] }),
        summary('gamma', { kinds: ['pdf'] }),
      ]
    }
    if (command === 'navegador_source_detail') {
      return details[(args as { sourceId: string }).sourceId] ?? null
    }
    if (command === 'navegador_pdf_file') return PDF_PATH
    return undefined
  }
  vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
    const answer = respond(command, args)
    if (answer instanceof Error) throw answer
    return answer
  })
  vi.mocked(listen).mockImplementation(async (name: string, handler: unknown) => {
    handlers[name] = handler as (event: { payload: unknown }) => void
    return () => {}
  })
  await navegadorSession.close()
  navegadorStore.reset()
  vi.mocked(invoke).mockClear()
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
    x: 0,
    y: 100,
    left: 0,
    top: 100,
    width: 800,
    height: 500,
    right: 800,
    bottom: 600,
    toJSON: () => ({}),
  })
})

afterEach(() => {
  vi.restoreAllMocks()
})

const toggle = () => screen.getByRole('button', { name: 'Fuentes guardadas' })
const drawer = () => screen.getByRole('complementary', { name: 'Fuentes guardadas' })

async function openDetail(title: string) {
  render(NavegadorView)
  await fireEvent.click(toggle())
  await fireEvent.click(await within(drawer()).findByRole('button', { name: new RegExp(title) }))
  await within(drawer()).findByRole('button', { name: 'Volver a la lista' })
}

/** The browser is opened the usual way, from the address bar, then the drawer. */
async function openBrowserThenDetail(title: string) {
  render(NavegadorView)
  const address = screen.getByRole('textbox', { name: 'Dirección' })
  await fireEvent.input(address, { target: { value: 'example.com' } })
  await fireEvent.submit(address.closest('form')!)
  await waitFor(() => expect(calls('navegador_open')).toHaveLength(1))
  await fireEvent.click(toggle())
  await fireEvent.click(await within(drawer()).findByRole('button', { name: new RegExp(title) }))
  await within(drawer()).findByRole('button', { name: 'Volver a la lista' })
}

const viewerRegion = () => screen.getByRole('region', { name: 'PDF guardado' })

describe('source actions by kind', () => {
  it('a source of PDFs opens its page of origin, a page source keeps "Abrir en el navegador"', async () => {
    await openDetail('Title of beta')
    expect(within(drawer()).getByRole('button', { name: 'Abrir página de origen' })).toBeEnabled()
    expect(within(drawer()).queryByRole('button', { name: 'Abrir en el navegador' })).toBeNull()

    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Volver a la lista' }))
    await fireEvent.click(await within(drawer()).findByRole('button', { name: /Title of alpha/ }))
    await within(drawer()).findByRole('button', { name: 'Abrir en el navegador' })
    expect(within(drawer()).queryByRole('button', { name: 'Abrir página de origen' })).toBeNull()
  })

  it('"Abrir página de origen" loads the page address and never fetches the PDF', async () => {
    await openDetail('Title of beta')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Abrir página de origen' }))

    await waitFor(() => expect(calls('navegador_open')).toHaveLength(1))
    expect(calls('navegador_open')[0]![1]).toMatchObject({
      url: 'https://www.beta.example.org/page',
    })
    expect(calls('navegador_pdf_file')).toHaveLength(0)
  })
})

describe('viewing a saved PDF', () => {
  it('offers the viewer only for a PDF capture whose file is on disk', async () => {
    await openDetail('Title of beta')
    expect(within(drawer()).getByRole('button', { name: 'Ver PDF guardado' })).toBeEnabled()

    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Volver a la lista' }))
    await fireEvent.click(await within(drawer()).findByRole('button', { name: /Title of gamma/ }))
    await within(drawer()).findByText('El archivo guardado ya no está en el disco.')
    expect(within(drawer()).queryByRole('button', { name: 'Ver PDF guardado' })).toBeNull()
  })

  it('asks for the capture by id and shows the local file in the app viewer, read only', async () => {
    await openDetail('Title of beta')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Ver PDF guardado' }))

    const viewer = await within(viewerRegion()).findByTestId('mock-document-viewer')
    expect(calls('navegador_pdf_file')).toEqual([['navegador_pdf_file', { captureId: 'c3' }]])
    expect(within(viewer).getByTestId('viewer-type')).toHaveTextContent('pdf')
    expect(viewer.dataset.path).toBe(PDF_PATH)
    expect(viewer.dataset.assetUrl).toContain(PDF_PATH)
    expect(viewer.dataset.readOnly).toBe('true')
    // Nothing was downloaded and the browser did not move.
    expect(calls('navegador_open')).toHaveLength(0)
    expect(calls('navegador_navigate')).toHaveLength(0)
  })

  it('hides the native browser while the PDF is open and shows it again after', async () => {
    await openBrowserThenDetail('Title of beta')
    vi.mocked(invoke).mockClear()

    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Ver PDF guardado' }))
    await within(viewerRegion()).findByTestId('mock-document-viewer')
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('navegador_set_visible', { visible: false })
    )
    expect(
      calls('navegador_set_visible').some(([, args]) => (args as { visible: boolean }).visible)
    ).toBe(false)

    vi.mocked(invoke).mockClear()
    await fireEvent.click(within(viewerRegion()).getByRole('button', { name: 'Cerrar el PDF' }))
    expect(screen.queryByRole('region', { name: 'PDF guardado' })).toBeNull()
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('navegador_set_visible', { visible: true })
    )
  })

  it('says clearly when the file turns out to be gone', async () => {
    const answer = respond
    respond = (command, args) =>
      command === 'navegador_pdf_file'
        ? new Error('file_missing: the saved PDF is not on disk')
        : answer(command, args)
    await openDetail('Title of beta')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Ver PDF guardado' }))

    expect(
      await within(viewerRegion()).findByText(/El archivo del PDF ya no está en este equipo/)
    ).toBeInTheDocument()
    expect(within(viewerRegion()).queryByTestId('mock-document-viewer')).toBeNull()
    // The person can still close it and get the browser back.
    await fireEvent.click(within(viewerRegion()).getByRole('button', { name: 'Cerrar el PDF' }))
    expect(screen.queryByRole('region', { name: 'PDF guardado' })).toBeNull()
  })

  it('works with no network: it only ever asks the backend for the saved file', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.reject(new TypeError('offline')))
    )
    try {
      await openDetail('Title of beta')
      await fireEvent.click(within(drawer()).getByRole('button', { name: 'Ver PDF guardado' }))
      await within(viewerRegion()).findByTestId('mock-document-viewer')
      expect(fetch).not.toHaveBeenCalled()
    } finally {
      vi.unstubAllGlobals()
    }
  })
})

describe('a download that is already in the sources', () => {
  const emit = (draft: DownloadDraft) => handlers['navegador://download']!({ payload: draft })

  it('says so instead of offering Guardar', async () => {
    render(NavegadorView)
    await waitFor(() => expect(handlers['navegador://download']).toBeDefined())
    emit(download('d1', { alreadySavedIn: 'beta' }))

    expect(await screen.findByText('Ya está en tus fuentes')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^Guardar/ })).toBeNull()
  })

  it('shows the source that holds it in the saved-sources drawer', async () => {
    render(NavegadorView)
    await waitFor(() => expect(handlers['navegador://download']).toBeDefined())
    emit(download('d1', { alreadySavedIn: 'beta' }))

    await fireEvent.click(
      await screen.findByRole('button', { name: 'Ver la fuente que ya guarda paper.pdf' })
    )

    await within(drawer()).findByRole('button', { name: 'Volver a la lista' })
    expect(calls('navegador_source_detail')).toEqual([
      ['navegador_source_detail', { sourceId: 'beta' }],
    ])
    expect(await within(drawer()).findByText('Title of beta')).toBeInTheDocument()
  })

  it('still offers Guardar for a PDF that is new', async () => {
    render(NavegadorView)
    await waitFor(() => expect(handlers['navegador://download']).toBeDefined())
    emit(download('d1'))

    expect(await screen.findByRole('button', { name: 'Guardar paper.pdf' })).toBeInTheDocument()
    expect(screen.queryByText('Ya está en tus fuentes')).toBeNull()
  })

  it('lists the same PDF downloaded again once, not once per download', async () => {
    render(NavegadorView)
    await waitFor(() => expect(handlers['navegador://download']).toBeDefined())
    emit(download('d1'))
    emit(download('d2'))
    emit(download('d3'))

    await screen.findByRole('button', { name: 'Guardar paper.pdf' })
    expect(screen.getAllByRole('button', { name: 'Guardar paper.pdf' })).toHaveLength(1)
    expect(screen.getAllByText('paper.pdf')).toHaveLength(1)
  })
})
