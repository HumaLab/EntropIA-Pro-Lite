/**
 * "Copiar a Zotero" from the saved-sources drawer: the button on the source and
 * on a saved PDF, the list of copies with their state, and what the drawer does
 * while a copy waits for Zotero (sends it again, offers to open Zotero).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { locale } from '$lib/i18n'
import { navegadorSession } from '$lib/navegador'
import { navegadorStore } from '$lib/navegador-store'
import type { CaptureDetail, SourceDetail, SourceSummary } from '$lib/navegador-sources'
import type { ZoteroCopy } from '$lib/navegador-zotero'
import type { BrowserState } from '$lib/navegador-tabs'
import NavegadorView from './NavegadorView.svelte'

const SHA = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'

const summary = (id: string, kinds: string[]): SourceSummary => ({
  id,
  title: `Title of ${id}`,
  finalUrl: `https://www.${id}.example.org/page`,
  siteName: null,
  updatedAt: 1_790_000_000_000,
  captureCount: 1,
  kinds: kinds as SourceSummary['kinds'],
})

const pdf = (id: string): CaptureDetail => ({
  id,
  kind: 'pdf',
  mimeType: 'application/pdf',
  accessedAt: '2026-09-30T12:00:00Z',
  finalUrl: 'https://beta.example.org/a.pdf',
  title: 'Informe',
  sha256: SHA,
  hashOf: 'pdf',
  sizeBytes: 2048,
  textPreview: null,
  textInFile: false,
  quotePrefix: null,
  quoteSuffix: null,
  filePresent: true,
  filePending: false,
  createdAt: 1,
})

const detailOf = (id: string, captures: CaptureDetail[]): SourceDetail => ({
  id,
  originalUrl: `https://start.${id}.example.org/`,
  finalUrl: `https://www.${id}.example.org/page`,
  canonicalUrl: null,
  title: `Title of ${id}`,
  siteName: null,
  firstAccessedAt: '2026-09-29T08:00:00Z',
  createdAt: 1,
  updatedAt: 2,
  captures,
})

const copyRow = (patch: Partial<ZoteroCopy> = {}): ZoteroCopy => ({
  id: 'k1',
  sourceId: 'beta',
  captureId: null,
  libraryType: 'user',
  libraryId: '0',
  libraryName: null,
  state: 'waiting',
  itemKey: null,
  detail: null,
  errorCode: null,
  errorMessage: null,
  attempts: 1,
  createdAt: 1,
  updatedAt: 1,
  ...patch,
})

const browser = (): BrowserState => ({ tabs: [], active: null, revision: 0 })

let respond: (command: string, args?: unknown) => unknown
let copies: ZoteroCopy[]

const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command)

beforeEach(async () => {
  locale.set('es')
  copies = []
  respond = (command, args) => {
    switch (command) {
      case 'navegador_state':
        return browser()
      case 'navegador_list_sources':
        return [summary('beta', ['pdf'])]
      case 'navegador_source_detail':
        return (args as { sourceId: string }).sourceId === 'beta'
          ? detailOf('beta', [pdf('c3')])
          : null
      case 'navegador_zotero_copy_list':
        return copies
      case 'navegador_zotero_copy_run':
        return { reachable: false, copies }
      case 'writing_zotero_known_libraries':
        return [{ libraryType: 'user', libraryId: '0', name: null, source: 'personal' }]
      case 'navegador_zotero_copy_request':
        return copyRow({ state: 'queued', captureId: (args as { captureId: string }).captureId })
      default:
        return undefined
    }
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
  vi.useRealTimers()
  vi.restoreAllMocks()
})

const drawer = () => screen.getByRole('complementary', { name: 'Fuentes guardadas' })

async function openBeta() {
  render(NavegadorView)
  await fireEvent.click(screen.getByRole('button', { name: 'Fuentes guardadas' }))
  await fireEvent.click(await within(drawer()).findByRole('button', { name: /Title of beta/ }))
  await within(drawer()).findByRole('button', { name: 'Volver a la lista' })
}

describe('copy to Zotero from the drawer', () => {
  it('offers it on the source only, and asks by id', async () => {
    await openBeta()
    // One button for the source; a saved PDF has none of its own.
    const buttons = within(drawer()).getAllByRole('button', { name: 'Copiar a Zotero' })
    expect(buttons).toHaveLength(1)

    await fireEvent.click(buttons[0]!)
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await waitFor(() => expect(calls('navegador_zotero_copy_request')).toHaveLength(1))
    expect(calls('navegador_zotero_copy_request')[0]![1]).toMatchObject({
      sourceId: 'beta',
      captureId: null,
    })
  })

  it('shows no section when nothing was copied', async () => {
    await openBeta()
    await waitFor(() => expect(calls('navegador_zotero_copy_list').length).toBeGreaterThan(0))
    expect(within(drawer()).queryByText('Copias a Zotero')).toBeNull()
  })

  it('shows a waiting copy with its library, its state and what can be done', async () => {
    copies = [copyRow()]
    await openBeta()
    await within(drawer()).findByText('Copias a Zotero')
    expect(within(drawer()).getByText('Esperando a Zotero')).toBeInTheDocument()
    expect(within(drawer()).getByText('Biblioteca: Mi biblioteca')).toBeInTheDocument()
    expect(within(drawer()).getByRole('button', { name: 'Abrir Zotero' })).toBeInTheDocument()
    expect(within(drawer()).getByRole('button', { name: 'Cancelar copia' })).toBeInTheDocument()
  })

  it('tries to send a waiting copy as soon as the source is opened', async () => {
    copies = [copyRow()]
    await openBeta()
    await waitFor(() => expect(calls('navegador_zotero_copy_run').length).toBeGreaterThan(0))
  })

  it('does not run anything for a source whose copies are all finished', async () => {
    copies = [copyRow({ state: 'copied', itemKey: 'ABCD2345' })]
    await openBeta()
    await within(drawer()).findByText('Copiado')
    expect(calls('navegador_zotero_copy_run')).toHaveLength(0)
    expect(within(drawer()).queryByRole('button', { name: 'Abrir Zotero' })).toBeNull()
  })

  it('sends again every few seconds while something waits, and stops when it is done', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] })
    copies = [copyRow()]
    await openBeta()
    await waitFor(() => expect(calls('navegador_zotero_copy_run')).toHaveLength(1))
    await vi.advanceTimersByTimeAsync(10_000)
    await waitFor(() => expect(calls('navegador_zotero_copy_run')).toHaveLength(2))

    copies = [copyRow({ state: 'copied', itemKey: 'ABCD2345' })]
    await vi.advanceTimersByTimeAsync(10_000)
    await within(drawer()).findByText('Copiado')
    const settled = calls('navegador_zotero_copy_run').length
    await vi.advanceTimersByTimeAsync(30_000)
    expect(calls('navegador_zotero_copy_run')).toHaveLength(settled)
  })

  it('opens Zotero only when the button is pressed, and says what happened', async () => {
    copies = [copyRow()]
    respond = ((inner) => (command: string, args?: unknown) =>
      command === 'navegador_zotero_launch' ? 'started' : inner(command, args))(respond)
    await openBeta()
    const button = await within(drawer()).findByRole('button', { name: 'Abrir Zotero' })
    expect(calls('navegador_zotero_launch')).toHaveLength(0)
    await fireEvent.click(button)
    expect(await within(drawer()).findByText(/Abriendo Zotero/)).toBeInTheDocument()
    expect(calls('navegador_zotero_launch')).toHaveLength(1)
  })

  it('says when Zotero could not be found', async () => {
    copies = [copyRow()]
    respond = ((inner) => (command: string, args?: unknown) =>
      command === 'navegador_zotero_launch' ? 'not_found' : inner(command, args))(respond)
    await openBeta()
    await fireEvent.click(await within(drawer()).findByRole('button', { name: 'Abrir Zotero' }))
    expect(await within(drawer()).findByText(/No se encontró Zotero/)).toBeInTheDocument()
  })

  it('cancels a waiting copy', async () => {
    copies = [copyRow()]
    respond = ((inner) => (command: string, args?: unknown) => {
      if (command === 'navegador_zotero_copy_cancel') {
        copies = [copyRow({ state: 'cancelled' })]
        return copies[0]
      }
      return inner(command, args)
    })(respond)
    await openBeta()
    await fireEvent.click(await within(drawer()).findByRole('button', { name: 'Cancelar copia' }))
    expect(calls('navegador_zotero_copy_cancel')[0]![1]).toEqual({ copyId: 'k1' })
    expect(await within(drawer()).findByText('Cancelado')).toBeInTheDocument()
  })

  it('retries a failed copy with the same library, and explains why it failed', async () => {
    copies = [
      copyRow({
        state: 'failed',
        libraryType: 'group',
        libraryId: '6680944',
        libraryName: 'prueba',
        errorCode: 'library_unavailable',
        errorMessage: 'x',
      }),
    ]
    await openBeta()
    expect(
      await within(drawer()).findByText(/no está disponible para escribir/)
    ).toBeInTheDocument()
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Reintentar' }))
    await waitFor(() => expect(calls('navegador_zotero_copy_request')).toHaveLength(1))
    expect(calls('navegador_zotero_copy_request')[0]![1]).toEqual({
      sourceId: 'beta',
      captureId: null,
      library: { libraryType: 'group', libraryId: '6680944', libraryName: 'prueba' },
    })
  })

  it('reports what was found in an item that already existed', async () => {
    copies = [
      copyRow({
        state: 'linked',
        itemKey: 'OLDKEY22',
        detail: {
          existing: true,
          pdf: 'parent_exists',
          pendingFields: ['accessDate'],
          keptFields: ['title'],
        },
      }),
    ]
    await openBeta()
    expect(await within(drawer()).findByText('Ya estaba en Zotero')).toBeInTheDocument()
    expect(within(drawer()).getByText(/se enlazó, no se duplicó/)).toBeInTheDocument()
    expect(within(drawer()).getByText(/nunca se pisan/)).toBeInTheDocument()
  })
})
