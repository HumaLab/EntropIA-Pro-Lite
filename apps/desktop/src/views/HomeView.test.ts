/** @vitest-environment jsdom */

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import type { HomeSnapshot, HomeActivityEntry } from '$lib/home'
import type { SyncStatus } from '$lib/sync'
import type { BatchGlobalSummary, BatchSummary } from '$lib/batch-processing'
import HomeView from './HomeView.svelte'

const EMPTY_BATCH_SUMMARY: BatchGlobalSummary = {
  init: null,
  initError: null,
  active: [],
  recoveredBatches: 0,
}

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
    summary: {
      init: null,
      initError: null,
      active: [],
      recoveredBatches: 0,
    } as BatchGlobalSummary,
    subscribers: new Set<(summary: BatchGlobalSummary) => void>(),
    requestFocus: vi.fn(),
  },
}))

vi.mock('$lib/home', async (importOriginal) => {
  const actual = await importOriginal<typeof import('$lib/home')>()
  return {
    ...actual,
    loadHomeSnapshot: homeRef.loadHomeSnapshot,
  }
})

vi.mock('$lib/navigation', () => ({
  navigation: navigationRef,
}))

vi.mock('$lib/batch-processing', () => ({
  batchStore: {
    snapshot: () => batchStoreRef.summary,
    subscribe: (run: (summary: BatchGlobalSummary) => void) => {
      batchStoreRef.subscribers.add(run)
      run(batchStoreRef.summary)
      return () => batchStoreRef.subscribers.delete(run)
    },
    initialize: vi.fn().mockResolvedValue(undefined),
    requestFocus: batchStoreRef.requestFocus,
  },
}))

function setBatchSummary(summary: BatchGlobalSummary) {
  batchStoreRef.summary = summary
  batchStoreRef.subscribers.forEach((run) => run(summary))
}

function makeActiveBatch(overrides: Partial<BatchSummary> = {}): BatchSummary {
  return {
    id: 'batch-1',
    state: 'running',
    desiredState: 'running',
    operations: ['ocr'],
    revision: 1,
    createdAt: 0,
    updatedAt: 0,
    activeUnits: 816,
    failedUnits: 0,
    succeededUnits: 428,
    ...overrides,
  }
}

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
      collections: 14,
      items: 2193,
      ocr: 618,
      stt: 24,
      text: 1087,
      embeddings: 630,
      pendingOcr: 4,
      pendingEmbeddings: 12,
    },
    continuar: [
      {
        kind: 'collection',
        id: 'col-1',
        title: 'Historia argentina',
        size: 19,
        wordCount: null,
        updatedAt: new Date(now.getTime() - 8 * 60 * 60 * 1000),
        view: { name: 'collection', id: 'col-1', collectionName: 'Historia argentina' },
      },
      {
        kind: 'writing',
        id: 'doc-1',
        title: 'Borrador de tesis',
        size: null,
        wordCount: null,
        updatedAt: new Date(now.getTime() - 24 * 60 * 60 * 1000),
        view: { name: 'writing', documentId: 'doc-1', documentTitle: 'Borrador de tesis' },
      },
      {
        kind: 'research',
        id: 'job-1',
        title: 'Impacto de la reforma',
        size: null,
        wordCount: null,
        updatedAt: null,
        view: { name: 'investigation', jobId: 'job-1', title: 'Impacto de la reforma' },
      },
      {
        kind: 'collection',
        id: 'col-2',
        title: 'Filosofía antigua',
        size: 7,
        wordCount: null,
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
    batchStoreRef.summary = { ...EMPTY_BATCH_SUMMARY, active: [] }
    batchStoreRef.subscribers.clear()
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
            collections: 14,
            items: 2193,
            ocr: 618,
            stt: 24,
            text: 1087,
            embeddings: 630,
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

    it('shows the corpus as an OCR/STT -> Texto -> Embeddings pipeline, each stage a ratio with a computed percentage', async () => {
      render(HomeView)

      // OCR and STT are a ratio of the total documents.
      expect(await screen.findByText('618 / 2.193')).toBeInTheDocument()
      expect(screen.getByText(/con OCR/)).toHaveTextContent('con OCR · 28 %')
      expect(await screen.findByText('24 / 2.193')).toBeInTheDocument()
      expect(screen.getByText(/con STT/)).toHaveTextContent('con STT · 1 %')
      // Texto is a ratio of the total documents.
      expect(await screen.findByText('1.087 / 2.193')).toBeInTheDocument()
      expect(screen.getByText(/con texto/)).toHaveTextContent('con texto · 50 %')
      // Embeddings is a ratio of documents WITH TEXT, not of the total.
      expect(await screen.findByText('630 / 1.087')).toBeInTheDocument()
      expect(screen.getByText(/con embeddings/)).toHaveTextContent('con embeddings · 58 %')
    })

    it('guards every stage percentage against a zero denominator instead of dividing by zero', async () => {
      homeRef.loadHomeSnapshot.mockResolvedValue(
        makeSnapshot({
          stats: {
            collections: 0,
            items: 0,
            ocr: 0,
            stt: 0,
            text: 0,
            embeddings: 0,
            pendingOcr: 0,
            pendingEmbeddings: 0,
          },
        })
      )
      render(HomeView)

      await screen.findByText('Estado del corpus')
      expect(screen.getByText(/con OCR/)).toHaveTextContent('con OCR · 0 %')
      expect(screen.getByText(/con STT/)).toHaveTextContent('con STT · 0 %')
      expect(screen.getByText(/con texto/)).toHaveTextContent('con texto · 0 %')
      expect(screen.getByText(/con embeddings/)).toHaveTextContent('con embeddings · 0 %')
    })

    it('draws a distinct icon for each corpus stage, through ActionIcon only', async () => {
      const { container } = render(HomeView)

      await screen.findByText('Estado del corpus')
      const corpusPanel = container.querySelector('.home-view__corpus')!
      const icons = [...corpusPanel.querySelectorAll('[data-action-icon]')].map((el) =>
        el.getAttribute('data-action-icon')
      )

      expect(icons).toEqual(
        expect.arrayContaining(['folder', 'file', 'scan', 'mic', 'file-text', 'nodes'])
      )
    })

    describe('Continuar meta and title formatting', () => {
      // Pins `Date.now()`/`new Date()` to `now` so `formatRelativeDate`
      // produces a deterministic label; real timers stay untouched, so
      // `waitFor`/`findByText` still poll in real time.
      beforeEach(() => {
        vi.useFakeTimers({ toFake: ['Date'] })
        vi.setSystemTime(now)
      })

      afterEach(() => {
        vi.useRealTimers()
      })

      it('shows a writing entry meta as type · relative time · word count', async () => {
        homeRef.loadHomeSnapshot.mockResolvedValue(
          makeSnapshot({
            continuar: [
              {
                kind: 'writing',
                id: 'doc-1',
                title: 'Borrador de tesis',
                size: null,
                wordCount: 4280,
                updatedAt: new Date(now.getTime() - 3 * 60 * 60 * 1000),
                view: { name: 'writing', documentId: 'doc-1', documentTitle: 'Borrador de tesis' },
              },
            ],
          })
        )
        const { container } = render(HomeView)

        await waitFor(() =>
          expect(container.querySelector('.home-view__continuar-list')).not.toBeNull()
        )
        const meta = container.querySelector('.home-view__continuar-row-meta')
        expect(meta?.textContent).toBe('Escritura · hace 3 horas · 4.280 palabras')
      })

      it('shows a singular word count', async () => {
        homeRef.loadHomeSnapshot.mockResolvedValue(
          makeSnapshot({
            continuar: [
              {
                kind: 'writing',
                id: 'doc-1',
                title: 'Borrador',
                size: null,
                wordCount: 1,
                updatedAt: new Date(now.getTime() - 3 * 60 * 60 * 1000),
                view: { name: 'writing', documentId: 'doc-1', documentTitle: 'Borrador' },
              },
            ],
          })
        )
        const { container } = render(HomeView)

        await waitFor(() =>
          expect(container.querySelector('.home-view__continuar-list')).not.toBeNull()
        )
        expect(container.querySelector('.home-view__continuar-row-meta')?.textContent).toBe(
          'Escritura · hace 3 horas · 1 palabra'
        )
      })

      it('shows a research entry meta with no trailing datum', async () => {
        homeRef.loadHomeSnapshot.mockResolvedValue(
          makeSnapshot({
            continuar: [
              {
                kind: 'research',
                id: 'job-1',
                title: 'Impacto de la reforma',
                size: null,
                wordCount: null,
                updatedAt: null,
                view: { name: 'investigation', jobId: 'job-1', title: 'Impacto de la reforma' },
              },
            ],
          })
        )
        const { container } = render(HomeView)

        await waitFor(() =>
          expect(container.querySelector('.home-view__continuar-list')).not.toBeNull()
        )
        expect(container.querySelector('.home-view__continuar-row-meta')?.textContent).toBe(
          'Investigación'
        )
      })

      it('shows the collection meta as type · relative time · document count', async () => {
        homeRef.loadHomeSnapshot.mockResolvedValue(
          makeSnapshot({
            continuar: [
              {
                kind: 'collection',
                id: 'col-1',
                title: 'Historia argentina',
                size: 19,
                wordCount: null,
                updatedAt: new Date(now.getTime() - 8 * 60 * 60 * 1000),
                view: { name: 'collection', id: 'col-1', collectionName: 'Historia argentina' },
              },
            ],
          })
        )
        const { container } = render(HomeView)

        await waitFor(() =>
          expect(container.querySelector('.home-view__continuar-list')).not.toBeNull()
        )
        expect(container.querySelector('.home-view__continuar-row-meta')?.textContent).toBe(
          'Colecciones · hace 8 horas · 19 Documentos'
        )
      })

      it('shows the English word count with English thousands grouping', async () => {
        locale.set('en')
        homeRef.loadHomeSnapshot.mockResolvedValue(
          makeSnapshot({
            continuar: [
              {
                kind: 'writing',
                id: 'doc-1',
                title: 'Thesis draft',
                size: null,
                wordCount: 4280,
                updatedAt: new Date(now.getTime() - 3 * 60 * 60 * 1000),
                view: { name: 'writing', documentId: 'doc-1', documentTitle: 'Thesis draft' },
              },
            ],
          })
        )
        const { container } = render(HomeView)

        await waitFor(() =>
          expect(container.querySelector('.home-view__continuar-list')).not.toBeNull()
        )
        expect(container.querySelector('.home-view__continuar-row-meta')?.textContent).toBe(
          'Writing · 3 hours ago · 4,280 words'
        )
      })

      it('shows the untitled placeholder for a writing document with an empty stored title', async () => {
        homeRef.loadHomeSnapshot.mockResolvedValue(
          makeSnapshot({
            continuar: [
              {
                kind: 'writing',
                id: 'doc-1',
                title: '',
                size: null,
                wordCount: null,
                updatedAt: new Date(now.getTime() - 3 * 60 * 60 * 1000),
                view: { name: 'writing', documentId: 'doc-1', documentTitle: '' },
              },
            ],
          })
        )
        render(HomeView)

        expect(await screen.findByText('Documento sin título')).toBeInTheDocument()
      })

      it('shows the untitled placeholder for a writing document still at its default stored title', async () => {
        homeRef.loadHomeSnapshot.mockResolvedValue(
          makeSnapshot({
            continuar: [
              {
                kind: 'writing',
                id: 'doc-1',
                title: 'Sin título',
                size: null,
                wordCount: null,
                updatedAt: new Date(now.getTime() - 3 * 60 * 60 * 1000),
                view: { name: 'writing', documentId: 'doc-1', documentTitle: 'Sin título' },
              },
            ],
          })
        )
        render(HomeView)

        expect(await screen.findByText('Documento sin título')).toBeInTheDocument()
        expect(screen.queryByText('Sin título')).not.toBeInTheDocument()
      })

      it('never shows the untitled placeholder for a collection named "Sin título"', async () => {
        homeRef.loadHomeSnapshot.mockResolvedValue(
          makeSnapshot({
            continuar: [
              {
                kind: 'collection',
                id: 'col-1',
                title: 'Sin título',
                size: 3,
                wordCount: null,
                updatedAt: new Date(now.getTime() - 8 * 60 * 60 * 1000),
                view: { name: 'collection', id: 'col-1', collectionName: 'Sin título' },
              },
            ],
          })
        )
        render(HomeView)

        expect(await screen.findByText('Sin título')).toBeInTheDocument()
        expect(screen.queryByText('Documento sin título')).not.toBeInTheDocument()
      })
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

  describe('active-process band', () => {
    beforeEach(() => {
      homeRef.loadHomeSnapshot.mockResolvedValue(makeSnapshot())
    })

    it('takes no space when no batch is active', async () => {
      render(HomeView)

      await screen.findByText('Estado del corpus')
      expect(screen.queryByRole('status')).not.toBeInTheDocument()
    })

    it('shows the running OCR batch with its progress', async () => {
      setBatchSummary({ ...EMPTY_BATCH_SUMMARY, active: [makeActiveBatch()] })
      render(HomeView)

      const band = await screen.findByRole('status')
      // Scoped to the band: the corpus panel's pipeline also has an "OCR"
      // stage label, so an unscoped query would be ambiguous.
      expect(within(band).getByText('OCR')).toBeInTheDocument()
      expect(within(band).getByText('428 / 1.244 páginas · 34 %')).toBeInTheDocument()
      expect(within(band).getByRole('button', { name: 'Ver lote →' })).toBeInTheDocument()
    })

    it('labels an embeddings batch accordingly', async () => {
      setBatchSummary({
        ...EMPTY_BATCH_SUMMARY,
        active: [makeActiveBatch({ operations: ['embeddings'] })],
      })
      render(HomeView)

      const band = await screen.findByRole('status')
      // Scoped for the same reason: the corpus panel also has an
      // "Embeddings" stage label.
      expect(within(band).getByText('Embeddings')).toBeInTheDocument()
    })

    it('opens Lotes with the active batch focused when "Ver lote" is clicked', async () => {
      setBatchSummary({ ...EMPTY_BATCH_SUMMARY, active: [makeActiveBatch()] })
      render(HomeView)

      await fireEvent.click(await screen.findByRole('button', { name: 'Ver lote →' }))

      expect(batchStoreRef.requestFocus).toHaveBeenCalledWith('batch-1')
      expect(navigationRef.openRootSection).toHaveBeenCalledWith({ name: 'settings' })
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
            stt: 0,
            text: 0,
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

    it('does not duplicate "Importar fuentes": the header omits it and the first-run block carries it', async () => {
      render(HomeView)

      await screen.findByText('Empezá con EntropIA')
      const importButtons = screen.getAllByRole('button', { name: 'Importar fuentes' })
      expect(importButtons).toHaveLength(1)

      await fireEvent.click(importButtons[0]!)
      expect(navigationRef.navigate).toHaveBeenCalledWith({ name: 'collections' })
    })

    it('still shows Nueva investigación and Nuevo documento in the header', async () => {
      render(HomeView)

      await screen.findByText('Empezá con EntropIA')
      expect(screen.getByRole('button', { name: 'Nueva investigación' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Nuevo documento' })).toBeInTheDocument()
    })
  })
})
