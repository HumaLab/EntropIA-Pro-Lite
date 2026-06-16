import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import SyncStatusIndicator from './SyncStatusIndicator.svelte'
import { locale } from '$lib/i18n'
import type { SyncStatus } from '$lib/sync'

function status(overrides: Partial<SyncStatus> = {}): SyncStatus {
  return {
    state: 'idle',
    last_sync_at: null,
    pending: 0,
    blobs_pending: 0,
    pending_blob_bytes: 0,
    conflicts: 0,
    clock_warning: false,
    ...overrides,
  }
}

const { syncStoreMock, setSyncState, initializeMock, openRootSectionMock } = vi.hoisted(() => {
  let current: SyncStatus = {
    state: 'disabled',
    last_sync_at: null,
    pending: 0,
    blobs_pending: 0,
    pending_blob_bytes: 0,
    conflicts: 0,
    clock_warning: false,
  }
  const subscribers = new Set<(value: SyncStatus) => void>()
  return {
    initializeMock: vi.fn().mockResolvedValue(undefined),
    openRootSectionMock: vi.fn(),
    syncStoreMock: {
      get status() {
        return current
      },
      subscribe(run: (value: SyncStatus) => void) {
        subscribers.add(run)
        run(current)
        return () => subscribers.delete(run)
      },
      initialize: (...args: unknown[]) => initializeMock(...args),
    },
    setSyncState(value: SyncStatus) {
      current = value
      subscribers.forEach((run) => run(current))
    },
  }
})

vi.mock('$lib/sync-store', async () => {
  const actual = await vi.importActual<typeof import('$lib/sync-store')>('$lib/sync-store')
  return {
    badgeVariantForState: actual.badgeVariantForState,
    syncStore: syncStoreMock,
  }
})

vi.mock('$lib/navigation', () => ({
  navigation: {
    openRootSection: openRootSectionMock,
  },
}))

describe('SyncStatusIndicator', () => {
  beforeEach(() => {
    locale.set('es')
    initializeMock.mockClear()
    openRootSectionMock.mockClear()
    setSyncState(status({ state: 'disabled' }))
  })

  afterEach(() => {
    setSyncState(status({ state: 'disabled' }))
  })

  it('renders nothing while sync is disabled (opt-in footer stays intact)', () => {
    render(SyncStatusIndicator)
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })

  it('bootstraps the store on mount', () => {
    render(SyncStatusIndicator)
    expect(initializeMock).toHaveBeenCalled()
  })

  it('shows the idle label when up to date', async () => {
    render(SyncStatusIndicator)
    setSyncState(status({ state: 'idle' }))
    await waitFor(() => {
      expect(screen.getByText('Sincronización al día')).toBeInTheDocument()
    })
  })

  it('shows the syncing label and motion class while syncing', async () => {
    render(SyncStatusIndicator)
    setSyncState(status({ state: 'syncing' }))
    await waitFor(() => {
      expect(screen.getByText('Sincronizando…')).toBeInTheDocument()
    })
    expect(document.querySelector('.sync-indicator--syncing')).not.toBeNull()
  })

  it('shows the offline label', async () => {
    render(SyncStatusIndicator)
    setSyncState(status({ state: 'offline' }))
    await waitFor(() => {
      expect(screen.getByText('Sin conexión')).toBeInTheDocument()
    })
  })

  it('shows the error label', async () => {
    render(SyncStatusIndicator)
    setSyncState(status({ state: 'error', message: 'boom' }))
    await waitFor(() => {
      expect(screen.getByText('Error de sincronización')).toBeInTheDocument()
    })
  })

  it('prioritizes the conflicts label even while idle', async () => {
    render(SyncStatusIndicator)
    setSyncState(status({ state: 'idle', conflicts: 2 }))
    await waitFor(() => {
      expect(screen.getByText('2 conflictos sin resolver')).toBeInTheDocument()
    })
  })

  it('opens the sync settings section on click', async () => {
    render(SyncStatusIndicator)
    setSyncState(status({ state: 'idle' }))
    await waitFor(() => screen.getByRole('button'))

    await fireEvent.click(screen.getByRole('button'))
    expect(openRootSectionMock).toHaveBeenCalledWith({ name: 'settings' })
  })

  it('builds a tooltip with last sync, pending, and conflicts', async () => {
    render(SyncStatusIndicator)
    setSyncState(
      status({ state: 'idle', last_sync_at: 1_700_000_000_000, pending: 4, blobs_pending: 1, conflicts: 1 })
    )
    await waitFor(() => screen.getByRole('button'))

    const tooltip = screen.getByRole('button').getAttribute('title') ?? ''
    expect(tooltip).toContain('Cambios pendientes: 4')
    expect(tooltip).toContain('Archivos pendientes: 1')
    expect(tooltip).toContain('Conflictos: 1')
  })

  it('translates labels when the locale changes', async () => {
    render(SyncStatusIndicator)
    setSyncState(status({ state: 'idle' }))
    await waitFor(() => screen.getByText('Sincronización al día'))

    locale.set('en')
    await waitFor(() => {
      expect(screen.getByText('Sync up to date')).toBeInTheDocument()
    })
  })
})
