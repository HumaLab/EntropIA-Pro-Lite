import { render, screen } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import BatchStatusIndicator from './BatchStatusIndicator.svelte'
import { locale } from '$lib/i18n'

/**
 * The statusbar badge (P3): while the derived backlog of a library sync —
 * fichas and pasajes — still holds work, the footer carries a compact
 * bibliography line with the real counts. Everything else keeps the badge it
 * always had, and an idle footer stays silent.
 */

type SyncStatus = {
  state: string
  errorCode: string | null
  errorMessage: string | null
  progressDone: number
  progressTotal: number | null
  itemsSeen: number | null
  remoteTotal: number | null
  newProfiles: number
  newExtractions: number
  profilesDone: number
  profilesTotal: number
  extractionsDone: number
  extractionsTotal: number
  etaMs: number | null
}

function syncStatus(overrides: Partial<SyncStatus> = {}): SyncStatus {
  return {
    state: 'succeeded',
    errorCode: null,
    errorMessage: null,
    progressDone: 40,
    progressTotal: 40,
    itemsSeen: 40,
    remoteTotal: 40,
    newProfiles: 0,
    newExtractions: 0,
    profilesDone: 0,
    profilesTotal: 0,
    extractionsDone: 0,
    extractionsTotal: 0,
    etaMs: null,
    ...overrides,
  }
}

const {
  batchStoreMock,
  writingZoteroMock,
  setActiveBatches,
  setBibliographyProgress,
  navigateActiveMock,
  requestSettingsTabMock,
} = vi.hoisted(() => {
  type BatchSummaryLike = {
    id: string
    state: string
    desiredState: string
    operations: string[]
    revision: number
    priority: number
    createdAt: number
    updatedAt: number
    activeUnits: number
    failedUnits: number
    succeededUnits: number
  }
  let active: BatchSummaryLike[] = []
  const batchSubscribers = new Set<(summary: unknown) => void>()
  const batchSummary = () => ({
    init: { ready: true },
    initError: null,
    active,
    recoveredBatches: 0,
  })
  let bibliographyProgress: unknown = null
  const zoteroSubscribers = new Set<(snapshot: unknown) => void>()
  return {
    navigateActiveMock: vi.fn(),
    requestSettingsTabMock: vi.fn(),
    setActiveBatches(next: BatchSummaryLike[]) {
      active = next
      batchSubscribers.forEach((run) => run(batchSummary()))
    },
    setBibliographyProgress(next: unknown) {
      bibliographyProgress = next
      zoteroSubscribers.forEach((run) => run({ bibliographyProgress }))
    },
    batchStoreMock: {
      snapshot: () => batchSummary(),
      subscribe(run: (summary: unknown) => void) {
        batchSubscribers.add(run)
        run(batchSummary())
        return () => batchSubscribers.delete(run)
      },
      initialize: vi.fn().mockResolvedValue(undefined),
      requestFocus: vi.fn(),
    },
    writingZoteroMock: {
      followBibliographyBacklog: vi.fn().mockResolvedValue(undefined),
      subscribe(run: (snapshot: unknown) => void) {
        zoteroSubscribers.add(run)
        run({ bibliographyProgress })
        return () => zoteroSubscribers.delete(run)
      },
    },
  }
})

vi.mock('$lib/batch-processing', async (importOriginal) => ({
  ...(await importOriginal<typeof import('$lib/batch-processing')>()),
  batchStore: batchStoreMock,
}))

vi.mock('$lib/writing-zotero', async (importOriginal) => ({
  ...(await importOriginal<typeof import('$lib/writing-zotero')>()),
  writingZotero: writingZoteroMock,
}))

vi.mock('$lib/workspace', () => ({
  workspace: { navigateActive: navigateActiveMock },
}))

vi.mock('$lib/settings-tab-request', () => ({
  requestSettingsTab: requestSettingsTabMock,
}))

const ACTIVE_BATCH = {
  id: 'batch-1',
  state: 'running',
  desiredState: 'run',
  operations: ['ocr'],
  revision: 1,
  priority: 0,
  createdAt: 1,
  updatedAt: 1,
  activeUnits: 2,
  failedUnits: 0,
  succeededUnits: 0,
}

beforeEach(() => {
  locale.set('es')
  setActiveBatches([])
  setBibliographyProgress(null)
  writingZoteroMock.followBibliographyBacklog.mockClear()
})

describe('BatchStatusIndicator', () => {
  it('renders nothing while no batch work and no bibliography backlog is active', () => {
    render(BatchStatusIndicator)

    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })

  it('shows a compact bibliography line while the derived backlog drains', () => {
    setBibliographyProgress({
      status: syncStatus({
        newProfiles: 450,
        newExtractions: 400,
        profilesDone: 120,
        profilesTotal: 450,
        extractionsDone: 30,
        extractionsTotal: 400,
        etaMs: 720_000,
      }),
      unreadable: null,
    })

    render(BatchStatusIndicator)

    expect(screen.getByText('Bibliografía: fichas 120/450 · pasajes 30/400')).toBeInTheDocument()
  })

  it('keeps the generic batch badge for user batches with no bibliography backlog', () => {
    setActiveBatches([ACTIVE_BATCH])

    render(BatchStatusIndicator)

    expect(screen.getByText('Procesando 1')).toBeInTheDocument()
    expect(screen.queryByText(/Bibliografía/)).not.toBeInTheDocument()
  })

  it('resumes the bibliography backlog follower at startup', () => {
    render(BatchStatusIndicator)

    // After a restart no sync was requested in this session, so the footer
    // itself asks the store to find the backlog that may still be draining.
    expect(writingZoteroMock.followBibliographyBacklog).toHaveBeenCalledTimes(1)
  })

  it('follows again when batch work appears and not on every refresh', () => {
    render(BatchStatusIndicator)
    expect(writingZoteroMock.followBibliographyBacklog).toHaveBeenCalledTimes(1)

    // New batch work (a bibliography sync among it) may carry a backlog the
    // session knows nothing about yet.
    setActiveBatches([ACTIVE_BATCH])
    expect(writingZoteroMock.followBibliographyBacklog).toHaveBeenCalledTimes(2)

    // Steady progress of the same active set is not a new reason to look.
    setActiveBatches([{ ...ACTIVE_BATCH, revision: 2 }])
    expect(writingZoteroMock.followBibliographyBacklog).toHaveBeenCalledTimes(2)
  })

  it('drops the bibliography line again once the backlog drains', () => {
    setBibliographyProgress({
      status: syncStatus({
        profilesDone: 450,
        profilesTotal: 450,
        extractionsDone: 400,
        extractionsTotal: 400,
        etaMs: 0,
      }),
      unreadable: null,
    })

    render(BatchStatusIndicator)

    expect(screen.queryByRole('button')).not.toBeInTheDocument()
    expect(screen.queryByText(/Bibliografía/)).not.toBeInTheDocument()
  })
})
