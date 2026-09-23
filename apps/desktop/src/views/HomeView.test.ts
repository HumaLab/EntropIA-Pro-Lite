/** @vitest-environment jsdom */

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import type { HomeSnapshot, HomeActivityEntry } from '$lib/home'
import type { SyncStatus } from '$lib/sync'
import HomeView from './HomeView.svelte'

const { homeRef, navigationRef, syncStoreRef, batchStoreRef } = vi.hoisted(() => ({
  homeRef: {
    loadHomeSnapshot: vi.fn(),
  },
  navigationRef: {
    navigate: vi.fn(),
    openRootSection: vi.fn(),
  },
  syncStoreRef: {
    status: { state: 'disabled' } as SyncStatus,
    subscribers: new Set<(status: SyncStatus) => void>(),
  },
  batchStoreRef: {
    requestFocus: vi.fn(),
  },
}))

vi.mock('$lib/home', () => ({
  loadHomeSnapshot: homeRef.loadHomeSnapshot,
}))

vi.mock('$lib/navigation', () => ({
  navigation: navigationRef,
}))

vi.mock('$lib/batch-processing', () => ({
  batchStore: batchStoreRef,
}))

vi.mock('$lib/sync-store', () => ({
  syncStore: {
    get status() {
      return syncStoreRef.status
    },
    subscribe: (run: (status: SyncStatus) => void) => {
      syncStoreRef.subscribers.add(run)
      run(syncStoreRef.status)
      return () => syncStoreRef.subscribers.delete(run)
    },
    initialize: vi.fn().mockResolvedValue(undefined),
  },
}))

function setSyncStatus(status: SyncStatus) {
  syncStoreRef.status = status
  syncStoreRef.subscribers.forEach((run) => run(status))
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((res) => {
    resolve = res
  })
  return { promise, resolve }
}

const now = new Date('2026-09-23T12:00:00Z')

const ACTIVITY_ENTRIES: HomeActivityEntry[] = [
  {
    id: 'item-1',
    title: 'Acta fundacional',
    collectionName: 'Historia argentina',
    createdAt: new Date(now.getTime() - 1 * 60 * 60 * 1000),
    view: {
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Historia argentina',
      itemId: 'item-1',
      itemTitle: 'Acta fundacional',
    },
  },
  {
    id: 'item-2',
    title: 'Carta abierta',
    collectionName: 'Filosofía antigua',
    createdAt: new Date(now.getTime() - 5 * 60 * 60 * 1000),
    view: {
      name: 'item',
      collectionId: 'col-2',
      collectionName: 'Filosofía antigua',
      itemId: 'item-2',
      itemTitle: 'Carta abierta',
    },
  },
  {
    id: 'item-3',
    title: 'Informe preliminar',
    collectionName: 'Historia argentina',
    createdAt: new Date(now.getTime() - 1 * 24 * 60 * 60 * 1000),
    view: {
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Historia argentina',
      itemId: 'item-3',
      itemTitle: 'Informe preliminar',
    },
  },
  {
    id: 'item-4',
    title: 'Nota de archivo',
    collectionName: 'Colección extra',
    createdAt: new Date(now.getTime() - 2 * 24 * 60 * 60 * 1000),
    view: {
      name: 'item',
      collectionId: 'col-3',
      collectionName: 'Colección extra',
      itemId: 'item-4',
      itemTitle: 'Nota de archivo',
    },
  },
  {
    id: 'item-5',
    title: 'Registro fotográfico',
    collectionName: 'Historia argentina',
    createdAt: new Date(now.getTime() - 3 * 24 * 60 * 60 * 1000),
    view: {
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Historia argentina',
      itemId: 'item-5',
      itemTitle: 'Registro fotográfico',
    },
  },
  {
    id: 'item-6',
    title: 'Documento excedente',
    collectionName: 'Colección extra',
    createdAt: new Date(now.getTime() - 4 * 24 * 60 * 60 * 1000),
    view: {
      name: 'item',
      collectionId: 'col-3',
      collectionName: 'Colección extra',
      itemId: 'item-6',
      itemTitle: 'Documento excedente',
    },
  },
]

function makeSnapshot(overrides: Partial<HomeSnapshot> = {}): HomeSnapshot {
  return {
    stats: {
      collections: 3,
      items: 2193,
      ocr: 1800,
      embeddings: 1500,
      pendingOcr: 4,
      pendingEmbeddings: 12,
    },
    continuar: [
      {
        kind: 'collection',
        id: 'col-1',
        title: 'Historia argentina',
        size: 19,
        updatedAt: new Date(now.getTime() - 8 * 60 * 60 * 1000),
        view: { name: 'collection', id: 'col-1', collectionName: 'Historia argentina' },
      },
      {
        kind: 'writing',
        id: 'doc-1',
        title: 'Borrador de tesis',
        size: null,
        updatedAt: new Date(now.getTime() - 24 * 60 * 60 * 1000),
        view: { name: 'writing', documentId: 'doc-1', documentTitle: 'Borrador de tesis' },
      },
      {
        kind: 'research',
        id: 'job-1',
        title: 'Impacto de la reforma',
        size: null,
        updatedAt: null,
        view: { name: 'investigation', jobId: 'job-1', title: 'Impacto de la reforma' },
      },
      {
        kind: 'collection',
        id: 'col-2',
        title: 'Filosofía antigua',
        size: 7,
        updatedAt: new Date(now.getTime() - 2 * 24 * 60 * 60 * 1000),
        view: { name: 'collection', id: 'col-2', collectionName: 'Filosofía antigua' },
      },
    ],
    activity: ACTIVITY_ENTRIES,
    isFirstRun: false,
    ...overrides,
  }
}

describe('HomeView', () => {
  beforeEach(() => {
    locale.set('es')
    navigationRef.navigate.mockReset()
    navigationRef.openRootSection.mockReset()
    syncStoreRef.status = { state: 'disabled' } as SyncStatus
    syncStoreRef.subscribers.clear()
    homeRef.loadHomeSnapshot.mockReset()
    batchStoreRef.requestFocus.mockReset()
  })

  afterEach(() => {
    cleanup()
    locale.set('es')
  })

  it('renders the startup page shell with the Inicio header', async () => {
    homeRef.loadHomeSnapshot.mockResolvedValue(makeSnapshot())
    const { container } = render(HomeView)

    const root = container.querySelector('.home-view')
    expect(root).not.toBeNull()
    expect(root).toHaveClass('page-shell')

    expect(screen.getByRole('heading', { level: 1, name: 'Inicio' })).toBeInTheDocument()
    expect(screen.getByText('Espacio de trabajo')).toBeInTheDocument()
    await waitFor(() => expect(homeRef.loadHomeSnapshot).toHaveBeenCalled())
  })

  it('renders the English header when the locale is English', async () => {
    homeRef.loadHomeSnapshot.mockResolvedValue(makeSnapshot())
    locale.set('en')
    render(HomeView)

    expect(screen.getByRole('heading', { level: 1, name: 'Home' })).toBeInTheDocument()
    expect(screen.getByText('Workspace')).toBeInTheDocument()
    await waitFor(() => expect(homeRef.loadHomeSnapshot).toHaveBeenCalled())
  })

  describe('loading state', () => {
    it('renders the layout skeleton without numbers while the snapshot loads', async () => {
      const { promise, resolve } = deferred<HomeSnapshot>()
      homeRef.loadHomeSnapshot.mockReturnValue(promise)

      render(HomeView)

      expect(screen.getByText('Continuar')).toBeInTheDocument()
      expect(screen.getByText('Estado del corpus')).toBeInTheDocument()
      expect(screen.queryByText('2.193')).not.toBeInTheDocument()

      resolve(makeSnapshot())
      await waitFor(() => expect(screen.getByText('2.193')).toBeInTheDocument())
    })
  })

  describe('error state', () => {
    it('shows an inline message when the snapshot fails to load', async () => {
      homeRef.loadHomeSnapshot.mockRejectedValue(new Error('boom'))
      render(HomeView)

      expect(await screen.findByRole('alert')).toBeInTheDocument()
    })
  })

  describe('in-use layout', () => {
    beforeEach(() => {
      homeRef.loadHomeSnapshot.mockResolvedValue(makeSnapshot())
    })

    it('shows at most 3 rows in Continuar, most recent first', async () => {
      const { container } = render(HomeView)

      await waitFor(() =>
        expect(container.querySelector('.home-view__continuar-list')).not.toBeNull()
      )
      const list = container.querySelector<HTMLElement>('.home-view__continuar-list')!
      const titles = within(list)
        .getAllByText(/./, { selector: '.home-view__continuar-row-title' })
        .map((el) => el.textContent)

      expect(titles).toEqual(['Historia argentina', 'Borrador de tesis', 'Impacto de la reforma'])
    })

    it('shows at most 5 rows in Actividad reciente, most recent first', async () => {
      render(HomeView)

      await screen.findByText('Registro fotográfico')
      expect(screen.queryByText('Documento excedente')).not.toBeInTheDocument()
    })

    it('shows the recently imported document collection instead of an item count', async () => {
      render(HomeView)

      const cell = await screen.findByText('Acta fundacional')
      const row = cell.closest('.home-view__recent-row')!
      expect(within(row as HTMLElement).getByText('Historia argentina')).toBeInTheDocument()
    })

    it('formats the corpus numbers with locale grouping', async () => {
      render(HomeView)

      expect(await screen.findByText('2.193')).toBeInTheDocument()
    })

    it('labels the corpus item figure and Continuar counts as Documentos, not ítems', async () => {
      const { container } = render(HomeView)

      expect(await screen.findByText('Documentos')).toBeInTheDocument()
      expect(screen.queryByText('Ítems')).not.toBeInTheDocument()

      const continuarList = container.querySelector<HTMLElement>('.home-view__continuar-list')!
      expect(within(continuarList).getByText(/19 Documentos/)).toBeInTheDocument()
    })

    it('shows a trailing arrow on every quick-access card, signalling it navigates', async () => {
      const { container } = render(HomeView)

      await waitFor(() =>
        expect(container.querySelectorAll('.home-view__quick-access-arrow')).toHaveLength(4)
      )
    })

    it('shows the pending OCR and embeddings lines when their counts are above zero', async () => {
      render(HomeView)

      expect(await screen.findByText('4 pendientes de OCR')).toBeInTheDocument()
      expect(screen.getByText('12 pendientes de embeddings')).toBeInTheDocument()
    })

    it('hides a pending line whose count is zero', async () => {
      homeRef.loadHomeSnapshot.mockResolvedValue(
        makeSnapshot({
          stats: {
            collections: 3,
            items: 2193,
            ocr: 1800,
            embeddings: 1500,
            pendingOcr: 0,
            pendingEmbeddings: 0,
          },
        })
      )
      render(HomeView)

      await screen.findByText('Estado del corpus')
      expect(screen.queryByText(/pendientes de OCR/)).not.toBeInTheDocument()
      expect(screen.queryByText(/pendientes de embeddings/)).not.toBeInTheDocument()
    })

    it('opens Lotes when a pending OCR/embeddings line is clicked', async () => {
      render(HomeView)

      await fireEvent.click(await screen.findByText('4 pendientes de OCR'))

      expect(batchStoreRef.requestFocus).toHaveBeenCalledWith(null)
      expect(navigationRef.openRootSection).toHaveBeenCalledWith({ name: 'settings' })
    })

    it('shows OCR and embeddings as a ratio of the total documents, with a computed percentage', async () => {
      render(HomeView)

      expect(await screen.findByText('1.800 / 2.193')).toBeInTheDocument()
      expect(screen.getByText(/Con OCR/)).toHaveTextContent('Con OCR · 82 %')
      expect(await screen.findByText('1.500 / 2.193')).toBeInTheDocument()
      expect(screen.getByText(/Con embeddings/)).toHaveTextContent('Con embeddings · 68 %')
    })

    it('guards the OCR/embeddings percentage against a zero total instead of dividing by zero', async () => {
      homeRef.loadHomeSnapshot.mockResolvedValue(
        makeSnapshot({
          stats: {
            collections: 0,
            items: 0,
            ocr: 0,
            embeddings: 0,
            pendingOcr: 0,
            pendingEmbeddings: 0,
          },
        })
      )
      render(HomeView)

      await screen.findByText('Estado del corpus')
      expect(screen.getByText(/Con OCR/)).toHaveTextContent('Con OCR · 0 %')
      expect(screen.getByText(/Con embeddings/)).toHaveTextContent('Con embeddings · 0 %')
    })

    it('navigates to the entry view when a Continuar row is clicked', async () => {
      const { container } = render(HomeView)

      await waitFor(() =>
        expect(container.querySelector('.home-view__continuar-list')).not.toBeNull()
      )
      const list = container.querySelector<HTMLElement>('.home-view__continuar-list')!
      const row = within(list).getByText('Historia argentina').closest('button')
      expect(row).not.toBeNull()
      await fireEvent.click(row!)

      expect(navigationRef.navigate).toHaveBeenCalledWith({
        name: 'collection',
        id: 'col-1',
        collectionName: 'Historia argentina',
      })
    })

    it('navigates to the item view when an Actividad reciente row is clicked', async () => {
      render(HomeView)

      const cell = await screen.findByText('Informe preliminar')
      const row = cell.closest('.home-view__recent-row')!
      await fireEvent.click(row)

      expect(navigationRef.navigate).toHaveBeenCalledWith({
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Historia argentina',
        itemId: 'item-3',
        itemTitle: 'Informe preliminar',
      })
    })

    it('opens the collections view from the quick-access card', async () => {
      render(HomeView)

      await fireEvent.click(await screen.findByRole('button', { name: /Colecciones/ }))

      expect(navigationRef.navigate).toHaveBeenCalledWith({ name: 'collections' })
    })

    it('opens the chat section from the quick-access card', async () => {
      render(HomeView)

      await fireEvent.click(await screen.findByRole('button', { name: /Chat/ }))

      expect(navigationRef.openRootSection).toHaveBeenCalledWith({ name: 'rag-chat' })
    })

    it('opens the research section from the header action', async () => {
      render(HomeView)

      await fireEvent.click(await screen.findByRole('button', { name: 'Nueva investigación' }))

      expect(navigationRef.openRootSection).toHaveBeenCalledWith({ name: 'research' })
    })

    it('opens the writing list from the header action', async () => {
      render(HomeView)

      await fireEvent.click(await screen.findByRole('button', { name: 'Nuevo documento' }))

      expect(navigationRef.openRootSection).toHaveBeenCalledWith({ name: 'writing' })
    })

    it('every header action button has an accessible name', async () => {
      render(HomeView)

      expect(await screen.findByRole('button', { name: 'Importar fuentes' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Nueva investigación' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Nuevo documento' })).toBeInTheDocument()
    })

    it('shows the sync line using the sync statusbar labels when sync is enabled', async () => {
      setSyncStatus({
        state: 'syncing',
        last_sync_at: null,
        pending: 0,
        blobs_pending: 0,
        pending_blob_bytes: 0,
        conflicts: 0,
        clock_warning: false,
      })
      render(HomeView)

      expect(await screen.findByText('Sincronizando…')).toBeInTheDocument()
    })

    it('hides the sync line when sync is disabled', async () => {
      render(HomeView)

      await screen.findByText('Estado del corpus')
      expect(screen.queryByText('Inactivo')).not.toBeInTheDocument()
    })
  })

  describe('first-run layout', () => {
    beforeEach(() => {
      homeRef.loadHomeSnapshot.mockResolvedValue(
        makeSnapshot({
          continuar: [],
          activity: [],
          isFirstRun: true,
          stats: {
            collections: 0,
            items: 0,
            ocr: 0,
            embeddings: 0,
            pendingOcr: 0,
            pendingEmbeddings: 0,
          },
        })
      )
    })

    it('shows "Inicio rápido" instead of Continuar', async () => {
      render(HomeView)

      expect(await screen.findByText('Empezá con EntropIA')).toBeInTheDocument()
      expect(screen.queryByText('Continuar')).not.toBeInTheDocument()
    })

    it('hides Actividad reciente entirely', async () => {
      render(HomeView)

      await screen.findByText('Empezá con EntropIA')
      expect(screen.queryByText('Actividad reciente')).not.toBeInTheDocument()
    })

    it('opens collections from the first-run "Crear colección" action', async () => {
      render(HomeView)

      await fireEvent.click(await screen.findByRole('button', { name: 'Crear colección' }))

      expect(navigationRef.navigate).toHaveBeenCalledWith({ name: 'collections' })
    })
  })
})
