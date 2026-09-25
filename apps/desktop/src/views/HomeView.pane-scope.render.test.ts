/** @vitest-environment jsdom */

/**
 * HomeView.pane-scope.test.ts proves the `-{paneId}` suffix is present in
 * the source. This file proves the runtime consequence: split view can
 * open a second Home tab beside the first (the spec's own example of a
 * duplicate-id path that's reachable today), so two HomeView instances,
 * each under its own pane-context provider, must never emit the same DOM
 * id twice in one document.
 *
 * `$lib/pane-context` is deliberately left unmocked here (every other
 * HomeView test mocks it) so `setPaneNavigation`/`getPaneId` run for real —
 * that's the exact wiring under test.
 */
import { cleanup, render, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import { NavigationStore } from '$lib/navigation'
import type { HomeSnapshot } from '$lib/home'
import type { SyncStatus } from '$lib/sync'
import type { BatchGlobalSummary } from '$lib/batch-processing'
import HomeViewPaneHost from './__fixtures__/HomeViewPaneHost.svelte'

const EMPTY_BATCH_SUMMARY: BatchGlobalSummary = {
  init: null,
  initError: null,
  active: [],
  recoveredBatches: 0,
}

const { homeRef, workspaceRef, syncStoreRef, batchStoreRef, writingRef, ragChatRef } = vi.hoisted(
  () => ({
    homeRef: {
      loadHomeSnapshot: vi.fn(),
    },
    workspaceRef: {
      navigateActive: vi.fn(),
    },
    writingRef: {
      createDocument: vi.fn(),
    },
    ragChatRef: {
      initialize: vi.fn(),
      startNew: vi.fn(),
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
  })
)

vi.mock('$lib/home', async (importOriginal) => {
  const actual = await importOriginal<typeof import('$lib/home')>()
  return {
    ...actual,
    loadHomeSnapshot: homeRef.loadHomeSnapshot,
  }
})

vi.mock('$lib/workspace', () => ({
  workspace: workspaceRef,
}))

vi.mock('$lib/writing', () => ({
  writing: writingRef,
}))

vi.mock('$lib/rag-chat', () => ({
  ragChat: ragChatRef,
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

function makeSnapshot(): HomeSnapshot {
  return {
    stats: {
      collections: 1,
      items: 1,
      ocr: 0,
      ocrUniverse: 0,
      stt: 0,
      sttUniverse: 0,
      text: 0,
      textUniverse: 0,
      embeddings: 0,
      pendingOcr: 0,
      pendingEmbeddings: 0,
    },
    continuar: [],
    activity: [
      {
        id: 'item-1',
        title: 'Doc',
        collectionName: 'Col',
        createdAt: new Date('2026-09-23T12:00:00Z'),
        view: {
          name: 'item',
          collectionId: 'col-1',
          collectionName: 'Col',
          itemId: 'item-1',
          itemTitle: 'Doc',
        },
      },
    ],
    isFirstRun: false,
    errors: {},
  }
}

describe('two HomeView panes mounted at once', () => {
  beforeEach(() => {
    locale.set('es')
    workspaceRef.navigateActive.mockReset()
    syncStoreRef.status = { state: 'disabled' } as SyncStatus
    syncStoreRef.subscribers.clear()
    homeRef.loadHomeSnapshot.mockReset().mockResolvedValue(makeSnapshot())
    writingRef.createDocument.mockReset()
    ragChatRef.initialize.mockReset().mockResolvedValue(undefined)
    ragChatRef.startNew.mockReset()
    batchStoreRef.requestFocus.mockReset()
    batchStoreRef.summary = { ...EMPTY_BATCH_SUMMARY }
    batchStoreRef.subscribers.clear()
  })

  afterEach(() => {
    cleanup()
    locale.set('es')
  })

  it('gives every view-owned id a distinct value per pane, with no id repeated in the document', async () => {
    render(HomeViewPaneHost, { navigation: new NavigationStore(), paneId: 'pane-a' })
    render(HomeViewPaneHost, { navigation: new NavigationStore(), paneId: 'pane-b' })

    await waitFor(() => {
      expect(document.getElementById('home-activity-title-pane-a')).not.toBeNull()
      expect(document.getElementById('home-activity-title-pane-b')).not.toBeNull()
    })

    const ids = [...document.querySelectorAll('[id]')].map((el) => el.id)
    const seen = new Set<string>()
    const duplicates = ids.filter((id) => {
      if (seen.has(id)) return true
      seen.add(id)
      return false
    })

    expect(duplicates).toEqual([])
  })
})
