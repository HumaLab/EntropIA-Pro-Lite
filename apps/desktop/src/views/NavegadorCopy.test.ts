/**
 * "Copiar a colección" from the saved-sources drawer, end to end up to the
 * corpus import: the drawer offers it for a saved PDF, the file comes from Rust
 * by capture id, and the document the copy creates opens in the pane.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { locale } from '$lib/i18n'
import { navegadorSession } from '$lib/navegador'
import { navegadorStore } from '$lib/navegador-store'
import { workspace } from '$lib/workspace'
import type { CaptureDetail, CopyTicket, SourceDetail } from '$lib/navegador-sources'
import type { BrowserState } from '$lib/navegador-tabs'
import NavegadorView from './NavegadorView.svelte'

const { storeRef, importRef } = vi.hoisted(() => ({
  storeRef: {
    current: {
      collections: { findAll: vi.fn(), create: vi.fn(), deleteIfEmpty: vi.fn() },
      items: { findByWebCapture: vi.fn() },
    },
  },
  importRef: { importClassifiedPathsIntoCollection: vi.fn() },
}))

vi.mock('$lib/db', () => ({ getStore: () => storeRef.current }))
vi.mock('$lib/collection-import', async (importOriginal) => {
  const actual = await importOriginal<typeof import('$lib/collection-import')>()
  return {
    ...actual,
    importClassifiedPathsIntoCollection: importRef.importClassifiedPathsIntoCollection,
  }
})

const SHA = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
const FILE = 'C:/data/web-captures/beta/c3.pdf'

const pdfCapture = (id: string, patch: Partial<CaptureDetail> = {}): CaptureDetail => ({
  id,
  kind: 'pdf',
  mimeType: 'application/pdf',
  accessedAt: '2026-09-30T12:00:00Z',
  finalUrl: 'https://www.beta.example.org/articulos/227/descargar',
  title: 'La cuestión social',
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
  ...patch,
})

const detailOf = (id: string, captures: CaptureDetail[]): SourceDetail => ({
  id,
  originalUrl: `https://www.${id}.example.org/articulos/227`,
  finalUrl: `https://www.${id}.example.org/articulos/227`,
  canonicalUrl: null,
  title: `Title of ${id}`,
  siteName: null,
  firstAccessedAt: '2026-09-29T08:00:00Z',
  createdAt: 1,
  updatedAt: 2,
  captures,
})

const renderedTicket: CopyTicket = {
  path: 'C:/data/web-captures/_copy/c1-1-0.pdf',
  provenance: {
    sourceId: 'alpha',
    captureId: 'c1',
    originalUrl: 'https://www.alpha.example.org/articulos/227',
    finalUrl: 'https://www.alpha.example.org/articulos/227',
    pageTitle: 'La cuestión social',
    accessedAt: '2026-09-30T12:00:00Z',
    sha256: SHA,
    captureKind: 'page',
    rendering: 'text-pdf',
  },
}

const ticket: CopyTicket = {
  path: FILE,
  provenance: {
    sourceId: 'beta',
    captureId: 'c3',
    originalUrl: 'https://www.beta.example.org/articulos/227',
    finalUrl: 'https://www.beta.example.org/articulos/227/descargar',
    pageTitle: 'La cuestión social',
    accessedAt: '2026-09-30T12:00:00Z',
    sha256: SHA,
  },
}

const browser = (): BrowserState => ({ tabs: [], active: null, revision: 0 })

let details: Record<string, SourceDetail>
const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command)
const drawer = () => screen.getByRole('complementary', { name: 'Fuentes guardadas' })

beforeEach(async () => {
  locale.set('es')
  vi.clearAllMocks()
  details = {
    alpha: detailOf('alpha', [
      { ...pdfCapture('c1'), kind: 'page', hashOf: 'html', textPreview: 'Texto de la página' },
    ]),
    delta: detailOf('delta', [
      { ...pdfCapture('c5'), kind: 'page', hashOf: 'html', textPreview: null },
      {
        ...pdfCapture('c6'),
        kind: 'selection',
        hashOf: 'quote',
        textPreview: 'una cita',
        quotePrefix: 'antes ',
        quoteSuffix: ' después',
      },
    ]),
    beta: detailOf('beta', [pdfCapture('c3')]),
    gamma: detailOf('gamma', [pdfCapture('c4', { filePresent: false })]),
  }
  vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
    if (command === 'navegador_state') return browser()
    if (command === 'navegador_list_sources') {
      return ['alpha', 'beta', 'gamma', 'delta'].map((id) => ({
        id,
        title: `Title of ${id}`,
        finalUrl: `https://www.${id}.example.org/page`,
        siteName: null,
        updatedAt: 1_790_000_000_000,
        captureCount: 1,
        kinds: ['pdf'],
      }))
    }
    if (command === 'navegador_source_detail') {
      return details[(args as { sourceId: string }).sourceId] ?? null
    }
    if (command === 'navegador_copy_ticket') {
      return (args as { captureId: string }).captureId === 'c3' ? ticket : renderedTicket
    }
    return undefined
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
  storeRef.current = {
    collections: {
      findAll: vi
        .fn()
        .mockResolvedValue([
          { id: 'col-1', name: 'Voces', description: null, createdAt: 1, updatedAt: 1 },
        ]),
      create: vi.fn(),
      deleteIfEmpty: vi.fn(),
    },
    items: { findByWebCapture: vi.fn().mockResolvedValue(null) },
  }
  importRef.importClassifiedPathsIntoCollection.mockResolvedValue({
    classifiedCount: 1,
    rejected: [],
    createdItems: [{ id: 'item-9', title: 'La cuestión social' }],
    importErrors: [],
    alreadyImported: [],
  })
})

afterEach(() => {
  vi.restoreAllMocks()
})

async function openDetail(title: string) {
  render(NavegadorView)
  await fireEvent.click(screen.getByRole('button', { name: 'Fuentes guardadas' }))
  await fireEvent.click(await within(drawer()).findByRole('button', { name: new RegExp(title) }))
  await within(drawer()).findByRole('button', { name: 'Volver a la lista' })
}

describe('copy to collection from the saved sources', () => {
  it('is offered for a PDF whose file is on disk, and not for a missing file', async () => {
    await openDetail('Title of beta')
    expect(within(drawer()).getByRole('button', { name: 'Copiar a colección' })).toBeEnabled()

    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Volver a la lista' }))
    await fireEvent.click(await within(drawer()).findByRole('button', { name: /Title of gamma/ }))
    await within(drawer()).findByText('El archivo guardado no está disponible en este equipo.')
    expect(within(drawer()).queryByRole('button', { name: 'Copiar a colección' })).toBeNull()

    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Volver a la lista' }))
    await fireEvent.click(await within(drawer()).findByRole('button', { name: /Title of alpha/ }))
  })

  it('is offered for a page or a selection that has text, and not for a page without it', async () => {
    await openDetail('Title of alpha')
    expect(within(drawer()).getByRole('button', { name: 'Copiar a colección' })).toBeEnabled()

    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Volver a la lista' }))
    await fireEvent.click(await within(drawer()).findByRole('button', { name: /Title of delta/ }))
    await within(drawer()).findByRole('button', { name: 'Abrir en el navegador' })
    // One page with no text kept and one selection: only the selection can be copied.
    expect(within(drawer()).getAllByRole('button', { name: 'Copiar a colección' })).toHaveLength(1)
  })

  it('copies a page capture as the rendered PDF Rust names and records the rendering', async () => {
    await openDetail('Title of alpha')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Copiar a colección' }))
    await screen.findByText(/PDF con el texto de/)
    await fireEvent.click(await screen.findByLabelText(/Voces/))

    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))

    await screen.findByText(/Copia creada en «Voces»/)
    expect(calls('navegador_copy_ticket')).toEqual([['navegador_copy_ticket', { captureId: 'c1' }]])
    const [paths, , options] = importRef.importClassifiedPathsIntoCollection.mock.calls[0]!
    expect(paths).toEqual([renderedTicket.path])
    expect(options.overrides.extraMetadata).toEqual({
      __entropia_web_capture: renderedTicket.provenance,
    })
  })

  it('copies a selection by its capture id, asking before a second copy', async () => {
    storeRef.current.items.findByWebCapture.mockResolvedValue({ id: 'item-1', title: 'Cita' })
    await openDetail('Title of delta')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Copiar a colección' }))
    await fireEvent.click(await screen.findByLabelText(/Voces/))

    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))

    await screen.findByText(/ya está copiada en «Voces»/)
    expect(storeRef.current.items.findByWebCapture).toHaveBeenCalledWith('col-1', 'c6')
    expect(calls('navegador_copy_ticket')).toHaveLength(0)
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar otra vez' }))
    await screen.findByText(/Copia creada/)
    expect(calls('navegador_copy_ticket')).toEqual([['navegador_copy_ticket', { captureId: 'c6' }]])
  })

  it('copies the saved file Rust names for the capture and records where it came from', async () => {
    await openDetail('Title of beta')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Copiar a colección' }))
    await fireEvent.click(await screen.findByLabelText(/Voces/))

    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))

    await screen.findByText(/Copia creada en «Voces»/)
    expect(calls('navegador_copy_ticket')).toEqual([['navegador_copy_ticket', { captureId: 'c3' }]])
    const [paths, collectionId, options] =
      importRef.importClassifiedPathsIntoCollection.mock.calls[0]!
    expect(paths).toEqual([FILE])
    expect(collectionId).toBe('col-1')
    expect(options.overrides.title).toBe('La cuestión social')
    expect(options.overrides.extraMetadata).toEqual({ __entropia_web_capture: ticket.provenance })
  })

  it('never moves, deletes or rewrites the web source', async () => {
    await openDetail('Title of beta')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Copiar a colección' }))
    await fireEvent.click(await screen.findByLabelText(/Voces/))
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await screen.findByText(/Copia creada/)

    const commands = vi.mocked(invoke).mock.calls.map(([name]) => name)
    expect(commands).not.toContain('navegador_delete_source')
    expect(commands).not.toContain('navegador_save_capture')
    expect(commands).not.toContain('navegador_save_download')
  })

  it('opens the new document in the pane', async () => {
    const navigate = vi.spyOn(workspace.activeNavigation, 'navigate').mockImplementation(() => {})
    await openDetail('Title of beta')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Copiar a colección' }))
    await fireEvent.click(await screen.findByLabelText(/Voces/))
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await screen.findByText(/Copia creada/)

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir documento' }))

    await waitFor(() =>
      expect(navigate).toHaveBeenCalledWith({
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Voces',
        itemId: 'item-9',
        itemTitle: 'La cuestión social',
      })
    )
  })

  it('closes without copying when the person cancels', async () => {
    await openDetail('Title of beta')
    await fireEvent.click(within(drawer()).getByRole('button', { name: 'Copiar a colección' }))

    await fireEvent.click(await screen.findByRole('button', { name: 'Cancelar' }))

    expect(screen.queryByText('Copiar a una colección')).toBeNull()
    expect(calls('navegador_copy_ticket')).toHaveLength(0)
    expect(importRef.importClassifiedPathsIntoCollection).not.toHaveBeenCalled()
  })
})
