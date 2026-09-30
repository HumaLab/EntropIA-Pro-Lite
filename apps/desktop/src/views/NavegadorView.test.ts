import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { locale } from '$lib/i18n'
import type { CaptureDraft, DownloadDraft } from '$lib/navegador-capture'
import NavegadorView from './NavegadorView.svelte'

const SHA = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'

const page: CaptureDraft = {
  kind: 'page',
  finalUrl: 'https://example.com/article',
  title: 'An article',
  canonicalUrl: null,
  siteName: null,
  lang: 'en',
  text: 'The readable text of the page',
  quote: null,
  quotePrefix: null,
  quoteSuffix: null,
  htmlBytes: 2048,
  hashOf: 'html',
  sha256: SHA,
  truncated: false,
  accessedAt: '2026-09-30T12:00:00Z',
}

const pdf: DownloadDraft = {
  id: 'd1',
  url: 'https://example.com/paper.pdf',
  fileName: 'paper.pdf',
  size: 1536,
  sha256: SHA,
  accessedAt: '2026-09-30T12:00:00Z',
  status: 'ready',
  reason: null,
}

let handlers: Record<string, (event: { payload: unknown }) => void> = {}
let respond: (command: string) => unknown

beforeEach(() => {
  locale.set('es')
  handlers = {}
  respond = (command) =>
    command === 'navegador_state'
      ? { url: 'https://example.com/article', title: 'An article', blocked: null }
      : undefined
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    const answer = respond(command)
    if (answer instanceof Error) throw answer
    return answer
  })
  vi.mocked(listen).mockImplementation(async (name: string, handler: unknown) => {
    handlers[name] = handler as (event: { payload: unknown }) => void
    return () => {}
  })
  // happy-dom lays nothing out: give the page area a real size.
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

async function openPage() {
  render(NavegadorView)
  const address = screen.getByRole('textbox', { name: 'Dirección' })
  await fireEvent.input(address, { target: { value: 'example.com' } })
  await fireEvent.submit(address.closest('form')!)
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('navegador_open', expect.anything()))
  await waitFor(() => expect(screen.getByLabelText('Capturar página')).toBeEnabled())
}

describe('NavegadorView capture', () => {
  it('cannot capture before a page is open, and shows no panel', () => {
    render(NavegadorView)
    expect(screen.getByLabelText('Capturar página')).toBeDisabled()
    expect(screen.getByLabelText('Capturar selección')).toBeDisabled()
    expect(screen.queryByLabelText(/Captura \(borrador/)).not.toBeInTheDocument()
  })

  it('captures the page and shows the draft outside the page area', async () => {
    await openPage()
    respond = (command) => (command === 'navegador_capture_page' ? page : undefined)
    await fireEvent.click(screen.getByLabelText('Capturar página'))

    const panel = await screen.findByLabelText(/Captura \(borrador/)
    expect(within(panel).getByText('An article')).toBeInTheDocument()
    expect(within(panel).getByText('https://example.com/article')).toBeInTheDocument()
    expect(within(panel).getByText('2026-09-30T12:00:00Z')).toBeInTheDocument()
    expect(within(panel).getByText('ba7816bf8f01')).toBeInTheDocument()
    expect(within(panel).getByText(/Texto: 29 caracteres/)).toBeInTheDocument()
    expect(within(panel).getByText('The readable text of the page')).toBeInTheDocument()
    // The native webview draws over anything that overlaps it: the panel must
    // not be inside the placeholder that marks where the page goes.
    const area = screen.getByRole('region', { name: 'Área de la página web' })
    expect(area.contains(panel)).toBe(false)
    expect(invoke).toHaveBeenCalledWith('navegador_capture_page')
  })

  it('captures the selection with its own command', async () => {
    await openPage()
    respond = (command) =>
      command === 'navegador_capture_selection'
        ? {
            ...page,
            kind: 'selection',
            quote: 'quoted words',
            text: 'quoted words',
            hashOf: 'quote',
          }
        : undefined
    await fireEvent.click(screen.getByLabelText('Capturar selección'))
    const panel = await screen.findByLabelText(/Captura \(borrador/)
    expect(within(panel).getByText('quoted words')).toBeInTheDocument()
    expect(within(panel).getByText('SHA-256 de la cita')).toBeInTheDocument()
    expect(invoke).toHaveBeenCalledWith('navegador_capture_selection')
  })

  it('explains a failed capture in the current language', async () => {
    await openPage()
    respond = (command) =>
      command === 'navegador_capture_selection' ? new Error('no_selection') : undefined
    await fireEvent.click(screen.getByLabelText('Capturar selección'))
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'No hay texto seleccionado en la página.'
    )

    locale.set('en')
    respond = (command) => (command === 'navegador_capture_page' ? new Error('timeout') : undefined)
    await fireEvent.click(await screen.findByLabelText('Capture page'))
    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent('The page did not answer in time.')
    )
  })

  it('shows the message of an error it does not know', async () => {
    await openPage()
    respond = (command) =>
      command === 'navegador_capture_page' ? new Error('something odd') : undefined
    await fireEvent.click(screen.getByLabelText('Capturar página'))
    expect(await screen.findByRole('alert')).toHaveTextContent('No se pudo capturar: something odd')
  })

  it('warns when the content was cut', async () => {
    await openPage()
    respond = (command) =>
      command === 'navegador_capture_page' ? { ...page, truncated: true } : undefined
    await fireEvent.click(screen.getByLabelText('Capturar página'))
    expect(await screen.findByText('El contenido superó el tope y se cortó.')).toBeInTheDocument()
  })

  it('lets the person dismiss a draft', async () => {
    await openPage()
    respond = (command) => (command === 'navegador_capture_page' ? page : undefined)
    await fireEvent.click(screen.getByLabelText('Capturar página'))
    await screen.findByLabelText(/Captura \(borrador/)
    await fireEvent.click(screen.getByLabelText('Descartar'))
    expect(screen.queryByLabelText(/Captura \(borrador/)).not.toBeInTheDocument()
  })

  it('renders what a page said as text, never as markup', async () => {
    await openPage()
    const hostile = '<img src=x onerror=alert(1)><b>bold</b>'
    respond = (command) =>
      command === 'navegador_capture_page' ? { ...page, title: hostile, text: hostile } : undefined
    await fireEvent.click(screen.getByLabelText('Capturar página'))
    const panel = await screen.findByLabelText(/Captura \(borrador/)
    expect(panel.querySelector('img')).toBeNull()
    expect(panel.querySelector('b')).toBeNull()
    expect(within(panel).getAllByText(hostile).length).toBeGreaterThan(0)
  })
})

describe('NavegadorView downloads', () => {
  it('listens for downloads and lists them', async () => {
    render(NavegadorView)
    await waitFor(() => expect(handlers['navegador://download']).toBeDefined())
    handlers['navegador://download']!({
      payload: { ...pdf, status: 'downloading', size: null, sha256: null },
    })
    const panel = await screen.findByLabelText(/Captura \(borrador/)
    expect(within(panel).getByText('paper.pdf')).toBeInTheDocument()
    expect(within(panel).getByText('Descargando…')).toBeInTheDocument()

    handlers['navegador://download']!({ payload: pdf })
    await waitFor(() => expect(within(panel).getByText('PDF verificado')).toBeInTheDocument())
    expect(within(panel).getByText('1.5 KB')).toBeInTheDocument()
    expect(within(panel).getByText('ba7816bf8f01')).toBeInTheDocument()
    expect(within(panel).getAllByText('paper.pdf')).toHaveLength(1)
  })

  it('says why a download was rejected', async () => {
    render(NavegadorView)
    await waitFor(() => expect(handlers['navegador://download']).toBeDefined())
    handlers['navegador://download']!({
      payload: {
        ...pdf,
        id: 'd2',
        fileName: 'page.pdf',
        status: 'rejected',
        reason: 'not_pdf',
        size: null,
        sha256: null,
      },
    })
    expect(await screen.findByText('No es un PDF.')).toBeInTheDocument()
    expect(screen.getByText('Rechazado')).toBeInTheDocument()
  })
})
