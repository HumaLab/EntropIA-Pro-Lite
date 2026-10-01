import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { locale } from '$lib/i18n'
import { navegadorSession } from '$lib/navegador'
import { navegadorStore } from '$lib/navegador-store'
import type { CaptureDraft, DownloadDraft } from '$lib/navegador-capture'
import type { BrowserState, BrowserTab } from '$lib/navegador-tabs'
import NavegadorView from './NavegadorView.svelte'

const SHA = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'

const page: CaptureDraft = {
  id: 'draft-1',
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
  savedTo: null,
  accessedAt: '2026-09-30T12:00:00Z',
  status: 'ready',
  reason: null,
  tab: null,
  pageUrl: null,
  pageTitle: null,
}

const tab = (id: number, patch: Partial<BrowserTab> = {}): BrowserTab => ({
  id,
  url: null,
  title: null,
  blocked: null,
  ...patch,
})

/** A revision of 0 is "not versioned": the store always takes it. */
const browser = (tabs: BrowserTab[], active: number | null, revision = 0): BrowserState => ({
  tabs,
  active,
  revision,
})

const articleTab = tab(1, { url: 'https://example.com/article', title: 'An article' })
const EMPTY = browser([], null)

let handlers: Record<string, (event: { payload: unknown }) => void> = {}
let respond: (command: string) => unknown

beforeEach(async () => {
  locale.set('es')
  handlers = {}
  respond = (command) => (command === 'navegador_state' ? browser([articleTab], 1) : undefined)
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    const answer = respond(command)
    if (answer instanceof Error) throw answer
    return answer
  })
  vi.mocked(listen).mockImplementation(async (name: string, handler: unknown) => {
    handlers[name] = handler as (event: { payload: unknown }) => void
    return () => {}
  })
  // The browser session and the store outlive a view: start every test clean.
  await navegadorSession.close()
  navegadorStore.reset()
  vi.mocked(invoke).mockClear()
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

async function openPageView() {
  const view = render(NavegadorView)
  const address = screen.getByRole('textbox', { name: 'Dirección' })
  await fireEvent.input(address, { target: { value: 'example.com' } })
  await fireEvent.submit(address.closest('form')!)
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('navegador_open', expect.anything()))
  await waitFor(() => expect(screen.getByLabelText('Capturar página')).toBeEnabled())
  return view
}

async function openPage() {
  await openPageView()
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
    expect(invoke).toHaveBeenCalledWith('navegador_capture_page', { tab: 1 })
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
    expect(invoke).toHaveBeenCalledWith('navegador_capture_selection', { tab: 1 })
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

describe('NavegadorView saving', () => {
  const saved = { sourceId: 's1', captureId: 'c1' }
  const calls = (command: string) =>
    vi.mocked(invoke).mock.calls.filter(([name]) => name === command)

  async function captured() {
    await openPage()
    respond = (command) => {
      if (command === 'navegador_capture_page') return page
      if (command === 'navegador_save_capture') return saved
      return undefined
    }
    await fireEvent.click(screen.getByLabelText('Capturar página'))
    return await screen.findByLabelText(/Captura \(borrador/)
  }

  it('offers to save a draft and sends only its id', async () => {
    const panel = await captured()
    await fireEvent.click(within(panel).getByRole('button', { name: 'Guardar' }))

    await waitFor(() => expect(calls('navegador_save_capture')).toHaveLength(1))
    expect(calls('navegador_save_capture')[0]![1]).toEqual({ draftId: 'draft-1' })
  })

  it('shows the draft as saved afterwards and cannot save it again', async () => {
    const panel = await captured()
    await fireEvent.click(within(panel).getByRole('button', { name: 'Guardar' }))

    const done = await within(panel).findByRole('button', { name: 'Guardado' })
    expect(done).toBeDisabled()
    await fireEvent.click(done)
    expect(calls('navegador_save_capture')).toHaveLength(1)
  })

  it('says why a save failed and lets the person try again', async () => {
    const panel = await captured()
    respond = (command) =>
      command === 'navegador_save_capture' ? new Error('db_error: disk full') : undefined
    await fireEvent.click(within(panel).getByRole('button', { name: 'Guardar' }))

    expect(
      await within(panel).findByText(
        'No se pudo registrar en el archivo de datos, y no quedó nada guardado: disk full'
      )
    ).toBeInTheDocument()
    expect(within(panel).getByRole('button', { name: 'Guardar' })).toBeEnabled()

    respond = (command) => (command === 'navegador_save_capture' ? saved : undefined)
    await fireEvent.click(within(panel).getByRole('button', { name: 'Guardar' }))
    await within(panel).findByRole('button', { name: 'Guardado' })
    expect(within(panel).queryByText(/No se pudo registrar/)).not.toBeInTheDocument()
  })

  it('tells the backend to forget a draft that is dismissed', async () => {
    const panel = await captured()
    await fireEvent.click(within(panel).getByLabelText('Descartar'))

    await waitFor(() => expect(calls('navegador_discard_draft')).toHaveLength(1))
    expect(calls('navegador_discard_draft')[0]![1]).toEqual({ draftId: 'draft-1' })
  })

  it('saves a verified PDF by its download id', async () => {
    render(NavegadorView)
    await waitFor(() => expect(handlers['navegador://download']).toBeDefined())
    respond = (command) => (command === 'navegador_save_download' ? saved : undefined)
    handlers['navegador://download']!({ payload: pdf })
    const panel = await screen.findByLabelText(/Captura \(borrador/)

    await fireEvent.click(await within(panel).findByRole('button', { name: 'Guardar paper.pdf' }))

    await waitFor(() => expect(calls('navegador_save_download')).toHaveLength(1))
    expect(calls('navegador_save_download')[0]![1]).toEqual({ downloadId: 'd1' })
    expect(await within(panel).findByRole('button', { name: 'Guardado paper.pdf' })).toBeDisabled()
  })

  it('offers no save for what is not a verified PDF', async () => {
    render(NavegadorView)
    await waitFor(() => expect(handlers['navegador://download']).toBeDefined())
    for (const [id, status] of [
      ['a', 'downloading'],
      ['b', 'rejected'],
      ['c', 'failed'],
      ['d', 'saved'],
    ] as const) {
      handlers['navegador://download']!({
        payload: { ...pdf, id, fileName: `${id}.zip`, status, size: null, sha256: null },
      })
    }
    await screen.findByText('d.zip')
    expect(screen.queryByRole('button', { name: /^Guardar / })).not.toBeInTheDocument()
  })

  it('speaks English too', async () => {
    await openPage()
    locale.set('en')
    respond = (command) => (command === 'navegador_capture_page' ? page : undefined)
    await fireEvent.click(await screen.findByLabelText('Capture page'))
    const panel = await screen.findByLabelText(/Capture \(draft/)
    expect(within(panel).getByRole('button', { name: 'Save' })).toBeInTheDocument()
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

describe('NavegadorView across mounts', () => {
  const commands = () => vi.mocked(invoke).mock.calls.map(([command]) => command)

  it('hides the browser when the view goes away and never closes it', async () => {
    const view = await openPageView()
    vi.mocked(invoke).mockClear()
    view.unmount()
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('navegador_set_visible', { visible: false })
    )
    expect(commands()).not.toContain('navegador_close')
    expect(navegadorSession.isOpen()).toBe(true)
  })

  it('shows the same browser again, at the new rect, when the view comes back', async () => {
    const view = await openPageView()
    view.unmount()
    await waitFor(() => expect(commands()).toContain('navegador_set_visible'))
    vi.mocked(invoke).mockClear()

    render(NavegadorView)
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('navegador_set_bounds', {
        x: 0,
        y: 100,
        width: 800,
        height: 500,
      })
    )
    expect(invoke).toHaveBeenCalledWith('navegador_set_visible', { visible: true })
    expect(commands()).not.toContain('navegador_open')
    // The address bar shows the page the browser is still on.
    expect(await screen.findByDisplayValue('https://example.com/article')).toBeInTheDocument()
    expect(screen.getByLabelText('Capturar página')).toBeEnabled()
  })

  it('keeps the capture draft and the downloads when the view is remounted', async () => {
    const view = await openPageView()
    respond = (command) => (command === 'navegador_capture_page' ? page : undefined)
    await fireEvent.click(screen.getByLabelText('Capturar página'))
    await screen.findByLabelText(/Captura \(borrador/)
    handlers['navegador://download']!({ payload: pdf })
    await screen.findByText('PDF verificado')
    view.unmount()

    render(NavegadorView)
    const panel = await screen.findByLabelText(/Captura \(borrador/)
    expect(within(panel).getByText('An article')).toBeInTheDocument()
    expect(within(panel).getByText('paper.pdf')).toBeInTheDocument()
  })

  it('lists a download that finished while no view was mounted', async () => {
    const view = await openPageView()
    view.unmount()
    handlers['navegador://download']!({ payload: pdf })
    render(NavegadorView)
    expect(await screen.findByText('paper.pdf')).toBeInTheDocument()
    expect(screen.getByText('PDF verificado')).toBeInTheDocument()
  })
})

describe('NavegadorView clearing the panel', () => {
  const emit = (draft: Partial<DownloadDraft> & { id: string }) =>
    handlers['navegador://download']!({ payload: { ...pdf, ...draft } })

  async function withDownloads() {
    render(NavegadorView)
    await waitFor(() => expect(handlers['navegador://download']).toBeDefined())
    emit({ id: 'd1', fileName: 'one.pdf' })
    emit({ id: 'd2', fileName: 'two.pdf' })
    await screen.findByText('two.pdf')
  }

  it('removes one download from the list with its own button', async () => {
    await withDownloads()
    await fireEvent.click(screen.getByLabelText('Quitar one.pdf de la lista'))
    expect(screen.queryByText('one.pdf')).not.toBeInTheDocument()
    expect(screen.getByText('two.pdf')).toBeInTheDocument()
  })

  it('clears the whole download list with one action', async () => {
    await withDownloads()
    await fireEvent.click(screen.getByRole('button', { name: 'Limpiar' }))
    expect(screen.queryByText('one.pdf')).not.toBeInTheDocument()
    expect(screen.queryByText('two.pdf')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Limpiar' })).not.toBeInTheDocument()
  })

  it('does not bring back a removed download that reports again', async () => {
    await withDownloads()
    await fireEvent.click(screen.getByLabelText('Quitar one.pdf de la lista'))
    emit({ id: 'd1', fileName: 'one.pdf', status: 'ready' })
    await waitFor(() => expect(screen.getByText('two.pdf')).toBeInTheDocument())
    expect(screen.queryByText('one.pdf')).not.toBeInTheDocument()
  })

  it('shows the capture draft and the downloads together, each with its own dismiss', async () => {
    await openPageView()
    respond = (command) => (command === 'navegador_capture_page' ? page : undefined)
    await fireEvent.click(screen.getByLabelText('Capturar página'))
    const panel = await screen.findByLabelText(/Captura \(borrador/)
    emit({ id: 'd1', fileName: 'one.pdf' })
    await within(panel).findByText('one.pdf')
    expect(within(panel).getByText('An article')).toBeInTheDocument()

    await fireEvent.click(within(panel).getByLabelText('Quitar one.pdf de la lista'))
    expect(within(panel).queryByText('one.pdf')).not.toBeInTheDocument()
    expect(within(panel).getByText('An article')).toBeInTheDocument()

    emit({ id: 'd2', fileName: 'two.pdf' })
    await within(panel).findByText('two.pdf')
    await fireEvent.click(within(panel).getByLabelText('Descartar'))
    expect(within(panel).queryByText('An article')).not.toBeInTheDocument()
    expect(within(panel).getByText('two.pdf')).toBeInTheDocument()
  })

  it('speaks English too', async () => {
    locale.set('en')
    await withDownloads()
    expect(screen.getByLabelText('Remove one.pdf from the list')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Clear' })).toBeInTheDocument()
  })
})

describe('NavegadorView download folder', () => {
  const emit = (draft: Partial<DownloadDraft> & { id: string }) =>
    handlers['navegador://download']!({ payload: { ...pdf, ...draft } })

  async function withDownload(draft: Partial<DownloadDraft> = {}) {
    render(NavegadorView)
    await waitFor(() => expect(handlers['navegador://download']).toBeDefined())
    emit({ id: 'd1', ...draft })
    return screen.findByLabelText(/Captura \(borrador/)
  }

  beforeEach(() => {
    respond = (command) =>
      command === 'navegador_download_dir'
        ? { path: 'C:/Users/x/Downloads', isDefault: true }
        : command === 'navegador_state'
          ? EMPTY
          : undefined
  })

  it('shows the folder that non-PDF files go to, in the downloads header', async () => {
    const panel = await withDownload()
    expect(await within(panel).findByText('C:/Users/x/Downloads')).toBeInTheDocument()
    expect(within(panel).getByText(/Carpeta de descargas/)).toBeInTheDocument()
    expect(invoke).toHaveBeenCalledWith('navegador_download_dir')
  })

  it('lets the person pick another folder and saves it', async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    vi.mocked(open).mockResolvedValue('D:/Docs')
    const setDir = vi.fn(() => ({ path: 'D:/Docs', isDefault: false }))
    const base = respond
    respond = (command) => (command === 'navegador_set_download_dir' ? setDir() : base(command))
    const panel = await withDownload()
    await within(panel).findByText('C:/Users/x/Downloads')
    await fireEvent.click(within(panel).getByRole('button', { name: 'Cambiar' }))
    expect(open).toHaveBeenCalledWith(expect.objectContaining({ directory: true, multiple: false }))
    await within(panel).findByText('D:/Docs')
    expect(invoke).toHaveBeenCalledWith('navegador_set_download_dir', { path: 'D:/Docs' })
  })

  it('changes nothing when the dialog is cancelled', async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    vi.mocked(open).mockResolvedValue(null)
    const panel = await withDownload()
    await within(panel).findByText('C:/Users/x/Downloads')
    await fireEvent.click(within(panel).getByRole('button', { name: 'Cambiar' }))
    await waitFor(() => expect(open).toHaveBeenCalled())
    expect(invoke).not.toHaveBeenCalledWith('navegador_set_download_dir', expect.anything())
    expect(within(panel).getByText('C:/Users/x/Downloads')).toBeInTheDocument()
  })

  it('says why a folder was refused and keeps the old one', async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    vi.mocked(open).mockResolvedValue('D:/gone')
    const base = respond
    respond = (command) =>
      command === 'navegador_set_download_dir'
        ? new Error('That folder does not exist')
        : base(command)
    const panel = await withDownload()
    await within(panel).findByText('C:/Users/x/Downloads')
    await fireEvent.click(within(panel).getByRole('button', { name: 'Cambiar' }))
    expect(await within(panel).findByRole('alert')).toHaveTextContent(
      'No se pudo usar esa carpeta: That folder does not exist'
    )
    expect(within(panel).getByText('C:/Users/x/Downloads')).toBeInTheDocument()
  })

  it('shows a file that was saved to the folder, and where', async () => {
    const panel = await withDownload({
      fileName: 'data.zip',
      status: 'saved',
      savedTo: 'C:/Users/x/Downloads',
      size: 2048,
      sha256: null,
    })
    expect(within(panel).getByText('data.zip')).toBeInTheDocument()
    expect(within(panel).getByText('Guardado')).toBeInTheDocument()
    expect(within(panel).getByText('Guardado en C:/Users/x/Downloads')).toBeInTheDocument()
    expect(within(panel).getByText('2.0 KB')).toBeInTheDocument()
    expect(within(panel).queryByText('No es un PDF.')).not.toBeInTheDocument()
  })
})

describe('NavegadorView tabs', () => {
  const commands = () => vi.mocked(invoke).mock.calls.map(([command]) => command)
  const strip = () => screen.getByRole('group', { name: /Solapas del navegador|Browser tabs/ })

  /**
   * An open browser whose state, as the backend says it, is `tabs`/`active`
   * (the active tab may be blank, so this does not wait for a page).
   */
  async function openWith(tabs: BrowserTab[], active: number) {
    respond = (command) => (command === 'navegador_state' ? browser(tabs, active) : undefined)
    const view = render(NavegadorView)
    // The view adopts the backend's state on mount; type only after that.
    await waitFor(() => expect(strip()).toBeInTheDocument())
    const address = screen.getByRole('textbox', { name: /Dirección|Address/ })
    await fireEvent.input(address, { target: { value: 'example.com' } })
    await fireEvent.submit(address.closest('form')!)
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('navegador_open', expect.anything()))
    return view
  }

  const emitState = (state: BrowserState) => handlers['navegador://state']!({ payload: state })

  it('shows no tab strip before the browser has a page', () => {
    render(NavegadorView)
    expect(screen.queryByRole('group', { name: 'Solapas del navegador' })).not.toBeInTheDocument()
  })

  it('lists the tabs by title, then by host, and a blank one as new', async () => {
    await openWith([articleTab, tab(2, { url: 'https://docs.example.org/x' }), tab(3)], 1)
    const tabs = within(strip())
    expect(tabs.getByRole('button', { name: 'An article' })).toHaveAttribute('aria-current', 'true')
    expect(tabs.getByRole('button', { name: 'docs.example.org' })).not.toHaveAttribute(
      'aria-current'
    )
    expect(tabs.getByRole('button', { name: 'Nueva solapa' })).toBeInTheDocument()
  })

  it('opens a blank tab with the plus button and leaves the address bar ready', async () => {
    await openWith([articleTab], 1)
    const blank = browser([articleTab, tab(2)], 2)
    respond = (command) => (command === 'navegador_new_tab' ? blank : undefined)
    await fireEvent.click(within(strip()).getByRole('button', { name: 'Abrir solapa nueva' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('navegador_new_tab'))
    const address = screen.getByRole('textbox', { name: 'Dirección' })
    await waitFor(() => expect(address).toHaveValue(''))
    await waitFor(() => expect(address).toHaveFocus())
    // A blank tab has no page to go back in or capture.
    expect(screen.getByLabelText('Atrás')).toBeDisabled()
    expect(screen.getByLabelText('Recargar')).toBeDisabled()
    expect(screen.getByLabelText('Capturar página')).toBeDisabled()
  })

  it('types into the blank tab: the address goes to that tab', async () => {
    await openWith([articleTab, tab(2)], 2)
    const address = screen.getByRole('textbox', { name: 'Dirección' })
    await waitFor(() => expect(address).toHaveValue(''))
    const loaded = browser([articleTab, tab(2, { url: 'https://b.test/', title: 'B' })], 2)
    respond = (command) => (command === 'navegador_navigate' ? loaded : undefined)
    await fireEvent.input(address, { target: { value: 'b.test' } })
    await fireEvent.submit(address.closest('form')!)
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('navegador_navigate', { tab: 2, url: 'b.test' })
    )
    await waitFor(() => expect(address).toHaveValue('https://b.test/'))
  })

  it('switches tab: the address bar, status and buttons follow, and the backend is told', async () => {
    const second = tab(2, { url: 'https://b.test/', title: 'B' })
    await openWith([articleTab, second], 2)
    const address = screen.getByRole('textbox', { name: 'Dirección' })
    await waitFor(() => expect(address).toHaveValue('https://b.test/'))
    respond = (command) =>
      command === 'navegador_activate_tab' ? browser([articleTab, second], 1) : undefined
    await fireEvent.click(within(strip()).getByRole('button', { name: 'An article' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('navegador_activate_tab', { tab: 1 }))
    await waitFor(() => expect(address).toHaveValue('https://example.com/article'))
    expect(within(strip()).getByRole('button', { name: 'An article' })).toHaveAttribute(
      'aria-current',
      'true'
    )
  })

  it('acts on the active tab: navigate, back, forward, reload and both captures', async () => {
    const second = tab(2, { url: 'https://b.test/', title: 'B' })
    await openWith([articleTab, second], 2)
    vi.mocked(invoke).mockClear()
    respond = (command) =>
      command === 'navegador_navigate' ? browser([articleTab, second], 2) : undefined
    await fireEvent.click(screen.getByLabelText('Atrás'))
    await fireEvent.click(screen.getByLabelText('Adelante'))
    await fireEvent.click(screen.getByLabelText('Recargar'))
    await waitFor(() => expect(commands()).toContain('navegador_reload'))
    expect(invoke).toHaveBeenCalledWith('navegador_back', { tab: 2 })
    expect(invoke).toHaveBeenCalledWith('navegador_forward', { tab: 2 })
    expect(invoke).toHaveBeenCalledWith('navegador_reload', { tab: 2 })
    respond = () => page
    await fireEvent.click(screen.getByLabelText('Capturar página'))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('navegador_capture_page', { tab: 2 }))
    await fireEvent.click(screen.getByLabelText('Capturar selección'))
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('navegador_capture_selection', { tab: 2 })
    )
    const address = screen.getByRole('textbox', { name: 'Dirección' })
    await fireEvent.input(address, { target: { value: 'c.test' } })
    await fireEvent.submit(address.closest('form')!)
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('navegador_navigate', { tab: 2, url: 'c.test' })
    )
  })

  it('closes a tab with its own button, named after the tab', async () => {
    const second = tab(2, { url: 'https://b.test/', title: 'B' })
    await openWith([articleTab, second], 1)
    respond = (command) =>
      command === 'navegador_close_tab' ? browser([articleTab], 1) : undefined
    await fireEvent.click(within(strip()).getByLabelText('Cerrar solapa B'))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('navegador_close_tab', { tab: 2 }))
    await waitFor(() =>
      expect(within(strip()).queryByRole('button', { name: 'B' })).not.toBeInTheDocument()
    )
  })

  it('keeps the address bar ready when the last tab is closed and replaced by a blank one', async () => {
    await openWith([articleTab], 1)
    respond = (command) => (command === 'navegador_close_tab' ? browser([tab(2)], 2) : undefined)
    await fireEvent.click(within(strip()).getByLabelText('Cerrar solapa An article'))
    const address = screen.getByRole('textbox', { name: 'Dirección' })
    await waitFor(() => expect(address).toHaveValue(''))
    expect(within(strip()).getByRole('button', { name: 'Nueva solapa' })).toBeInTheDocument()
    expect(screen.getByLabelText('Capturar página')).toBeDisabled()
  })

  it('disables the plus button at four tabs and says why', async () => {
    const full = [1, 2, 3, 4].map((id) => tab(id, { url: `https://t${id}.test/` }))
    await openWith(full, 1)
    const plus = within(strip()).getByRole('button', { name: 'Solapa nueva: ya hay 4 abiertas' })
    expect(plus).toBeDisabled()
    await fireEvent.click(plus)
    expect(commands()).not.toContain('navegador_new_tab')
  })

  it('shows a tab a page opened, from the backend event, while the view is mounted', async () => {
    await openWith([articleTab], 1)
    emitState(
      browser([articleTab, tab(2, { url: 'https://popup.test/', title: 'Opened by a page' })], 1, 5)
    )
    expect(
      await within(strip()).findByRole('button', { name: 'Opened by a page' })
    ).toBeInTheDocument()
  })

  it('ignores a state older than the one it has', async () => {
    await openWith([articleTab], 1)
    emitState(browser([articleTab, tab(2, { url: 'https://b.test/', title: 'B' })], 2, 9))
    await within(strip()).findByRole('button', { name: 'B' })
    emitState(browser([articleTab], 1, 8))
    await waitFor(() =>
      expect(screen.getByRole('textbox', { name: 'Dirección' })).toHaveValue('https://b.test/')
    )
    expect(within(strip()).getByRole('button', { name: 'B' })).toBeInTheDocument()
  })

  it('keeps every tab when the view goes away and comes back', async () => {
    const second = tab(2, { url: 'https://b.test/', title: 'B' })
    const view = await openWith([articleTab, second], 2)
    view.unmount()
    render(NavegadorView)
    expect(await within(strip()).findByRole('button', { name: 'B' })).toHaveAttribute(
      'aria-current',
      'true'
    )
    expect(within(strip()).getByRole('button', { name: 'An article' })).toBeInTheDocument()
  })

  it('hears a tab a page opened while no view was mounted', async () => {
    const view = await openWith([articleTab], 1)
    view.unmount()
    const opened = browser([articleTab, tab(2, { url: 'https://b.test/', title: 'B' })], 1, 6)
    emitState(opened)
    // The backend answers a remount with the state it holds now.
    respond = (command) => (command === 'navegador_state' ? opened : undefined)
    render(NavegadorView)
    expect(await within(strip()).findByRole('button', { name: 'B' })).toBeInTheDocument()
  })

  it('tells the person why a page could not open another tab', async () => {
    await openWith([articleTab], 1)
    emitState(browser([{ ...articleTab, blocked: 'The page tried to open too many tabs' }], 1, 7))
    expect(
      await screen.findByText('Bloqueado: The page tried to open too many tabs')
    ).toBeInTheDocument()
  })

  it('says where a download came from by the page it started on', async () => {
    await openWith([articleTab], 1)
    handlers['navegador://download']!({
      payload: {
        ...pdf,
        url: 'https://files.example.org/paper.pdf',
        tab: 1,
        pageUrl: 'https://news.example.org/a',
        pageTitle: 'Article one',
      },
    })
    const panel = await screen.findByLabelText(/Captura \(borrador/)
    expect(within(panel).getByText('Desde news.example.org · Article one')).toBeInTheDocument()
  })

  it('keeps each download labelled with its own page when the tab moved on', async () => {
    await openWith([articleTab], 1)
    for (const [id, title] of [
      ['d1', 'Article one'],
      ['d2', 'Article two'],
      ['d3', 'Article three'],
    ] as const) {
      handlers['navegador://download']!({
        payload: {
          ...pdf,
          id,
          fileName: `${id}.pdf`,
          tab: 1,
          pageUrl: `https://news.example.org/${id}`,
          pageTitle: title,
        },
      })
    }
    // The tab itself now shows something else.
    emitState(browser([{ ...articleTab, title: 'Something else entirely' }], 1, 9))
    const panel = await screen.findByLabelText(/Captura \(borrador/)
    for (const title of ['Article one', 'Article two', 'Article three']) {
      expect(within(panel).getByText(`Desde news.example.org · ${title}`)).toBeInTheDocument()
    }
    expect(within(panel).queryByText(/Something else entirely/)).not.toBeInTheDocument()
  })

  it('still lists a download with only its host when the page is unknown', async () => {
    await openWith([articleTab], 1)
    handlers['navegador://download']!({
      payload: { ...pdf, url: 'https://files.example.org/paper.pdf', tab: 7 },
    })
    const panel = await screen.findByLabelText(/Captura \(borrador/)
    expect(within(panel).getByText('Desde files.example.org')).toBeInTheDocument()
  })

  it('renders a hostile tab title as text, never as markup', async () => {
    const hostile = '<img src=x onerror=alert(1)>'
    await openWith([tab(1, { url: 'https://example.com/', title: hostile })], 1)
    expect(strip().querySelector('img')).toBeNull()
    expect(within(strip()).getByRole('button', { name: hostile })).toBeInTheDocument()
  })

  it('speaks English too', async () => {
    locale.set('en')
    await openWith([articleTab, tab(2)], 1)
    expect(strip()).toHaveAccessibleName('Browser tabs')
    expect(within(strip()).getByRole('button', { name: 'New tab' })).toBeInTheDocument()
    expect(within(strip()).getByLabelText('Close tab An article')).toBeInTheDocument()
  })
})
