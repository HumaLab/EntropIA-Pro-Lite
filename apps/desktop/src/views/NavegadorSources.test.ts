import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { locale } from '$lib/i18n'
import { navegadorSession } from '$lib/navegador'
import { navegadorStore } from '$lib/navegador-store'
import type { CaptureDetail, SourceDetail, SourceSummary } from '$lib/navegador-sources'
import type { BrowserState } from '$lib/navegador-tabs'
import NavegadorView from './NavegadorView.svelte'

const SHA = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'

const summary = (id: string, patch: Partial<SourceSummary> = {}): SourceSummary => ({
  id,
  title: `Title of ${id}`,
  finalUrl: `https://www.${id}.example.org/page`,
  siteName: null,
  updatedAt: 1_790_000_000_000,
  captureCount: 2,
  kinds: ['page', 'selection'],
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
  textPreview: 'The readable text of the page',
  textInFile: false,
  quotePrefix: null,
  quoteSuffix: null,
  filePresent: true,
  filePending: false,
  createdAt: 1,
  ...patch,
})

const detailOf = (id: string, captures: CaptureDetail[]): SourceDetail => ({
  id,
  originalUrl: `https://start.${id}.example.org/`,
  finalUrl: `https://www.${id}.example.org/page`,
  canonicalUrl: `https://canon.${id}.example.org/`,
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

let respond: (command: string, args?: unknown) => unknown
let sources: SourceSummary[]
let details: Record<string, SourceDetail | null>
const clipboard = { writeText: vi.fn() }

const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command)

beforeEach(async () => {
  locale.set('es')
  sources = [summary('alpha'), summary('beta', { kinds: ['pdf'], captureCount: 1 })]
  details = {
    alpha: detailOf('alpha', [
      capture('c2', {
        kind: 'selection',
        hashOf: 'quote',
        textPreview: 'the exact quote',
        quotePrefix: 'before ',
        quoteSuffix: ' after',
        filePresent: null,
        accessedAt: '2026-09-30T13:00:00Z',
      }),
      capture('c1'),
    ]),
    beta: detailOf('beta', [
      capture('c3', {
        kind: 'pdf',
        hashOf: 'pdf',
        mimeType: 'application/pdf',
        textPreview: null,
        filePresent: false,
      }),
    ]),
  }
  respond = (command, args) => {
    if (command === 'navegador_state') return browser(null)
    if (command === 'navegador_list_sources') return sources
    if (command === 'navegador_source_detail') {
      return details[(args as { sourceId: string }).sourceId] ?? null
    }
    return undefined
  }
  vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
    const answer = respond(command, args)
    if (answer instanceof Error) throw answer
    return answer
  })
  vi.mocked(listen).mockImplementation(async () => () => {})
  await navegadorSession.close()
  navegadorStore.reset()
  vi.mocked(invoke).mockClear()
  clipboard.writeText = vi.fn().mockResolvedValue(undefined)
  Object.defineProperty(navigator, 'clipboard', { value: clipboard, configurable: true })
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

async function openDrawer() {
  render(NavegadorView)
  await fireEvent.click(toggle())
  await screen.findByRole('complementary', { name: 'Fuentes guardadas' })
  await waitFor(() => expect(calls('navegador_list_sources').length).toBeGreaterThan(0))
}

async function openDetail(title: string) {
  await openDrawer()
  await fireEvent.click(await within(drawer()).findByRole('button', { name: new RegExp(title) }))
  await within(drawer()).findByRole('button', { name: 'Volver a la lista' })
}

describe('saved sources drawer', () => {
  it('is closed until asked for and does not read the archive meanwhile', () => {
    render(NavegadorView)
    expect(toggle()).toBeInTheDocument()
    expect(screen.queryByRole('complementary', { name: 'Fuentes guardadas' })).toBeNull()
    expect(calls('navegador_list_sources')).toHaveLength(0)
  })

  it('opens beside the page area, never inside it, and closes again', async () => {
    await openDrawer()
    const area = screen.getByRole('region', { name: 'Área de la página web' })
    // The native webview draws above HTML: the drawer must not sit in its rect.
    expect(area.contains(drawer())).toBe(false)
    expect(drawer().contains(area)).toBe(false)

    await fireEvent.click(within(drawer()).getByLabelText('Cerrar las fuentes guardadas'))
    expect(screen.queryByRole('complementary', { name: 'Fuentes guardadas' })).toBeNull()
  })

  it('lists the sources with their host, capture count and kinds', async () => {
    await openDrawer()
    const alpha = await within(drawer()).findByRole('button', { name: /Title of alpha/ })
    expect(within(alpha).getByText('alpha.example.org')).toBeInTheDocument()
    expect(within(alpha).getByText('Capturas: 2')).toBeInTheDocument()
    expect(within(alpha).getByText('Página')).toBeInTheDocument()
    expect(within(alpha).getByText('Selección')).toBeInTheDocument()
    const beta = within(drawer()).getByRole('button', { name: /Title of beta/ })
    expect(within(beta).getByText('PDF')).toBeInTheDocument()
    expect(calls('navegador_list_sources')[0]![1]).toEqual({ query: null, limit: null })
  })

  it('says so when nothing was saved yet', async () => {
    sources = []
    await openDrawer()
    expect(
      await within(drawer()).findByText(/Todavía no guardaste ninguna fuente/)
    ).toBeInTheDocument()
  })

  it('says why a list could not be read', async () => {
    respond = (command) =>
      command === 'navegador_list_sources' ? new Error('db_error: database is locked') : undefined
    await openDrawer()
    expect(
      await within(drawer()).findByText(
        'No se pudieron leer las fuentes guardadas: db_error: database is locked'
      )
    ).toBeInTheDocument()
  })

  it('renders what a page said as text, never as markup', async () => {
    const hostile = '<img src=x onerror=alert(1)><b>bold</b>'
    sources = [summary('evil', { title: hostile })]
    await openDrawer()
    expect(await within(drawer()).findByText(hostile)).toBeInTheDocument()
    expect(drawer().querySelector('img')).toBeNull()
    expect(drawer().querySelector('b')).toBeNull()
  })

  it('refreshes the list when a capture is saved while it is open', async () => {
    await openDrawer()
    const before = calls('navegador_list_sources').length
    sources = [summary('fresh'), ...sources]
    const listing = respond
    respond = (command, args) =>
      command === 'navegador_save_capture'
        ? { sourceId: 'fresh', captureId: 'c9' }
        : listing(command, args)
    await navegadorStore.saveCapture('draft-9')
    expect(
      await within(drawer()).findByRole('button', { name: /Title of fresh/ })
    ).toBeInTheDocument()
    expect(calls('navegador_list_sources').length).toBeGreaterThan(before)
  })
})

describe('saved sources search', () => {
  it('asks the backend for what was typed, once typing pauses', async () => {
    await openDrawer()
    const search = within(drawer()).getByRole('searchbox', {
      name: 'Buscar en las fuentes guardadas',
    })
    sources = [summary('alpha')]
    await fireEvent.input(search, { target: { value: 'tra' } })
    await fireEvent.input(search, { target: { value: 'trabajo' } })

    await waitFor(() =>
      expect(calls('navegador_list_sources').at(-1)![1]).toEqual({ query: 'trabajo', limit: null })
    )
    // The pause collapses the keystrokes into one request.
    expect(
      calls('navegador_list_sources').filter(
        ([, args]) => (args as { query: string | null }).query === 'tra'
      )
    ).toHaveLength(0)
    expect(within(drawer()).queryByRole('button', { name: /Title of beta/ })).toBeNull()
  })

  it('says when nothing matches', async () => {
    await openDrawer()
    sources = []
    const search = within(drawer()).getByRole('searchbox', {
      name: 'Buscar en las fuentes guardadas',
    })
    await fireEvent.input(search, { target: { value: 'nothing' } })
    expect(
      await within(drawer()).findByText('Ninguna fuente coincide con la búsqueda.')
    ).toBeInTheDocument()
    expect(within(drawer()).queryByText(/Todavía no guardaste/)).toBeNull()
  })

  it('ignores a slow answer to an older search', async () => {
    await openDrawer()
    let release!: (value: SourceSummary[]) => void
    const slow = new Promise<SourceSummary[]>((resolve) => (release = resolve))
    vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
      if (command === 'navegador_list_sources') {
        const query = (args as { query: string | null }).query
        return query === 'old' ? slow : [summary('newer')]
      }
      return respond(command, args)
    })
    const search = within(drawer()).getByRole('searchbox', {
      name: 'Buscar en las fuentes guardadas',
    })
    await fireEvent.input(search, { target: { value: 'old' } })
    await waitFor(() =>
      expect(calls('navegador_list_sources').at(-1)![1]).toEqual({ query: 'old', limit: null })
    )
    await fireEvent.input(search, { target: { value: 'new' } })
    await within(drawer()).findByRole('button', { name: /Title of newer/ })

    release([summary('stale')])
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(within(drawer()).queryByRole('button', { name: /Title of stale/ })).toBeNull()
    expect(within(drawer()).getByRole('button', { name: /Title of newer/ })).toBeInTheDocument()
  })
})

describe('saved source detail', () => {
  it('shows the addresses and every capture newest first, in local time and UTC', async () => {
    await openDetail('Title of alpha')
    const panel = drawer()
    expect(within(panel).getByText('https://start.alpha.example.org/')).toBeInTheDocument()
    expect(within(panel).getByText('https://canon.alpha.example.org/')).toBeInTheDocument()
    expect(within(panel).getByText('2026-09-29T08:00:00Z')).toBeInTheDocument()
    expect(calls('navegador_source_detail')[0]![1]).toEqual({ sourceId: 'alpha' })

    const items = within(panel).getAllByRole('listitem')
    expect(items).toHaveLength(2)
    expect(within(items[0]!).getByText('Selección')).toBeInTheDocument()
    expect(within(items[1]!).getByText('Página')).toBeInTheDocument()
    expect(within(items[0]!).getByText('UTC: 2026-09-30T13:00:00Z')).toBeInTheDocument()
    expect(within(items[1]!).getByText('ba7816bf8f01')).toBeInTheDocument()
    expect(within(items[1]!).getByText('Tamaño: 2.0 KB')).toBeInTheDocument()
    expect(within(items[1]!).getByText('The readable text of the page')).toBeInTheDocument()
  })

  it('shows a selection as its quote with the text around it', async () => {
    await openDetail('Title of alpha')
    const quote = within(drawer()).getByText('the exact quote')
    expect(quote.closest('blockquote')).toHaveTextContent('before the exact quote after')
  })

  it('says a snapshot exists but does not open it, and warns when its file is gone', async () => {
    await openDetail('Title of alpha')
    expect(within(drawer()).getByText('Copia HTML guardada en este equipo.')).toBeInTheDocument()
    expect(drawer().querySelector('iframe, webview, object, embed')).toBeNull()
    expect(within(drawer()).queryByRole('button', { name: /Abrir copia/ })).toBeNull()

    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Volver a la lista' }))
    await fireEvent.click(await within(drawer()).findByRole('button', { name: /Title of beta/ }))
    expect(
      await within(drawer()).findByText('El archivo guardado no está disponible en este equipo.')
    ).toBeInTheDocument()
  })

  it('says a file that sync is still downloading is on its way, not lost', async () => {
    details.beta = detailOf('beta', [
      capture('c3', {
        kind: 'pdf',
        hashOf: 'pdf',
        mimeType: 'application/pdf',
        textPreview: null,
        filePresent: false,
        filePending: true,
      }),
    ])
    await openDetail('Title of beta')
    expect(
      await within(drawer()).findByText('Descargando el archivo desde tu cuenta de sincronización…')
    ).toBeInTheDocument()
    expect(
      within(drawer()).queryByText('El archivo guardado no está disponible en este equipo.')
    ).toBeNull()
  })

  it('renders captured text as text, never as markup', async () => {
    const hostile = '<img src=x onerror=alert(1)>'
    details.alpha = detailOf('alpha', [capture('c1', { textPreview: hostile, title: hostile })])
    await openDetail('Title of alpha')
    expect(drawer().querySelector('img')).toBeNull()
    expect(within(drawer()).getAllByText(hostile).length).toBeGreaterThan(0)
  })

  it('goes back to the list', async () => {
    await openDetail('Title of alpha')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Volver a la lista' }))
    expect(
      await within(drawer()).findByRole('button', { name: /Title of beta/ })
    ).toBeInTheDocument()
  })

  it('says so when the source vanished meanwhile', async () => {
    details.alpha = null
    await openDrawer()
    await fireEvent.click(await within(drawer()).findByRole('button', { name: /Title of alpha/ }))
    expect(await within(drawer()).findByText('Esta fuente ya no existe.')).toBeInTheDocument()
    expect(within(drawer()).getByRole('button', { name: 'Volver a la lista' })).toBeInTheDocument()
  })
})

describe('saved source actions', () => {
  it('opens the address in a new browser when none is open yet', async () => {
    await openDetail('Title of alpha')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Abrir en el navegador' }))

    await waitFor(() => expect(calls('navegador_open')).toHaveLength(1))
    expect(calls('navegador_open')[0]![1]).toMatchObject({
      url: 'https://www.alpha.example.org/page',
    })
  })

  it('navigates the active tab when the browser is already open', async () => {
    const listing = respond
    respond = (command, args) => {
      if (command === 'navegador_state') return browser('https://example.com/')
      if (command === 'navegador_navigate') return browser('https://www.alpha.example.org/page')
      return listing(command, args)
    }
    render(NavegadorView)
    // The browser is opened the usual way, from the address bar.
    const address = screen.getByRole('textbox', { name: 'Dirección' })
    await fireEvent.input(address, { target: { value: 'example.com' } })
    await fireEvent.submit(address.closest('form')!)
    await waitFor(() => expect(calls('navegador_open')).toHaveLength(1))
    await waitFor(() => expect(screen.getByLabelText('Capturar página')).toBeEnabled())

    await fireEvent.click(toggle())
    await fireEvent.click(await within(drawer()).findByRole('button', { name: /Title of alpha/ }))
    await fireEvent.click(
      await within(drawer()).findByRole('button', { name: 'Abrir en el navegador' })
    )

    await waitFor(() => expect(calls('navegador_navigate')).toHaveLength(1))
    expect(calls('navegador_navigate')[0]![1]).toEqual({
      tab: 1,
      url: 'https://www.alpha.example.org/page',
    })
    // It reused the browser: no second one was opened.
    expect(calls('navegador_open')).toHaveLength(1)
  })

  it('shows what the URL policy said when the address is refused', async () => {
    respond = (command, args) => {
      if (command === 'navegador_open')
        return new Error('blocked: loopback addresses are not allowed')
      if (command === 'navegador_state') return browser(null)
      if (command === 'navegador_list_sources') return sources
      if (command === 'navegador_source_detail')
        return details[(args as { sourceId: string }).sourceId] ?? null
      return undefined
    }
    await openDetail('Title of alpha')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Abrir en el navegador' }))
    expect(await screen.findByText(/loopback addresses are not allowed/)).toBeInTheDocument()
  })

  it('copies the address', async () => {
    await openDetail('Title of alpha')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Copiar dirección' }))
    await waitFor(() =>
      expect(clipboard.writeText).toHaveBeenCalledWith('https://www.alpha.example.org/page')
    )
    expect(await within(drawer()).findByText('Dirección copiada.')).toBeInTheDocument()
  })

  it('says when the address could not be copied', async () => {
    clipboard.writeText = vi.fn().mockRejectedValue(new Error('denied'))
    await openDetail('Title of alpha')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Copiar dirección' }))
    expect(await within(drawer()).findByText('No se pudo copiar la dirección.')).toBeInTheDocument()
  })
})

describe('deleting a saved source', () => {
  async function askToDelete() {
    await openDetail('Title of alpha')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Eliminar fuente' }))
    return await screen.findByRole('dialog')
  }

  it('asks first, naming the source and what goes with it', async () => {
    const dialog = await askToDelete()
    expect(within(dialog).getByText(/«Title of alpha»/)).toBeInTheDocument()
    expect(within(dialog).getByText(/sus 2 capturas/)).toBeInTheDocument()
    expect(
      within(dialog).getByText(/copias que hayas hecho en colecciones no se tocan/)
    ).toBeInTheDocument()
    expect(calls('navegador_delete_source')).toHaveLength(0)
  })

  it('does nothing when cancelled', async () => {
    const dialog = await askToDelete()
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Cancelar' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(calls('navegador_delete_source')).toHaveLength(0)
  })

  it('deletes by id, returns to a list without it and says it is done', async () => {
    const dialog = await askToDelete()
    respond = (command, args) => {
      if (command === 'navegador_delete_source') {
        sources = sources.filter((s) => s.id !== 'alpha')
        return { leftoverFiles: false }
      }
      if (command === 'navegador_state') return browser(null)
      if (command === 'navegador_list_sources') return sources
      if (command === 'navegador_source_detail')
        return details[(args as { sourceId: string }).sourceId] ?? null
      return undefined
    }
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Eliminar Title of alpha' }))

    await waitFor(() => expect(calls('navegador_delete_source')).toHaveLength(1))
    expect(calls('navegador_delete_source')[0]![1]).toEqual({ sourceId: 'alpha' })
    expect(await within(drawer()).findByText('Fuente eliminada.')).toBeInTheDocument()
    expect(within(drawer()).queryByRole('button', { name: /Title of alpha/ })).toBeNull()
    expect(within(drawer()).getByRole('button', { name: /Title of beta/ })).toBeInTheDocument()
  })

  it('tells the person when some files will be cleaned up at the next start', async () => {
    const dialog = await askToDelete()
    respond = (command, args) => {
      if (command === 'navegador_delete_source') return { leftoverFiles: true }
      if (command === 'navegador_state') return browser(null)
      if (command === 'navegador_list_sources') return sources.filter((s) => s.id !== 'alpha')
      if (command === 'navegador_source_detail')
        return details[(args as { sourceId: string }).sourceId] ?? null
      return undefined
    }
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Eliminar Title of alpha' }))
    expect(
      await within(drawer()).findByText(/Algunos archivos no se pudieron borrar ahora/)
    ).toBeInTheDocument()
  })

  it('keeps the source and says why when the delete failed', async () => {
    const dialog = await askToDelete()
    respond = (command, args) => {
      if (command === 'navegador_delete_source') return new Error('db_error: database is locked')
      if (command === 'navegador_state') return browser(null)
      if (command === 'navegador_list_sources') return sources
      if (command === 'navegador_source_detail')
        return details[(args as { sourceId: string }).sourceId] ?? null
      return undefined
    }
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Eliminar Title of alpha' }))

    expect(
      await screen.findByText(
        'No se pudo eliminar la fuente y no se borró nada: database is locked'
      )
    ).toBeInTheDocument()
    expect(within(drawer()).getByRole('button', { name: 'Volver a la lista' })).toBeInTheDocument()
  })
})

describe('saved sources in English', () => {
  it('speaks English too', async () => {
    locale.set('en')
    render(NavegadorView)
    await fireEvent.click(screen.getByRole('button', { name: 'Saved sources' }))
    const panel = await screen.findByRole('complementary', { name: 'Saved sources' })
    expect(
      within(panel).getByRole('searchbox', { name: 'Search saved sources' })
    ).toBeInTheDocument()
    await fireEvent.click(await within(panel).findByRole('button', { name: /Title of alpha/ }))
    expect(await within(panel).findByText('Original address')).toBeInTheDocument()
    expect(within(panel).getByRole('button', { name: 'Open in the browser' })).toBeInTheDocument()
    expect(within(panel).getByRole('button', { name: 'Delete source' })).toBeInTheDocument()
  })
})
