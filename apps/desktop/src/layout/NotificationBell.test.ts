import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import NotificationBell from './NotificationBell.svelte'
import { locale } from '$lib/i18n'
import type { SyncStatus } from '$lib/sync'
import type { NotificationState } from '$lib/notification-store'

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

function notifState(overrides: Partial<NotificationState> = {}): NotificationState {
  return {
    unread: 0,
    pendingPlanRequest: null,
    items: [],
    loading: false,
    error: null,
    ...overrides,
  }
}

// ── Sync store mock (drives session presence) ──
const { syncStoreMock, setSyncState, initializeMock } = vi.hoisted(() => {
  let current: SyncStatus = {
    state: 'disabled',
    last_sync_at: null,
    pending: 0,
    blobs_pending: 0,
    pending_blob_bytes: 0,
    conflicts: 0,
    clock_warning: false,
  }
  const subs = new Set<(v: SyncStatus) => void>()
  return {
    initializeMock: vi.fn().mockResolvedValue(undefined),
    syncStoreMock: {
      get status() {
        return current
      },
      subscribe(run: (v: SyncStatus) => void) {
        subs.add(run)
        run(current)
        return () => subs.delete(run)
      },
      initialize: (...a: unknown[]) => initializeMock(...a),
    },
    setSyncState(v: SyncStatus) {
      current = v
      subs.forEach((run) => run(current))
    },
  }
})

// ── Notification store mock (drives badge + list) ──
const { notifStoreMock, setNotifState, refreshFromUsageMock, loadNotificationsMock, markReadMock, deleteNotificationMock, markAllReadMock } =
  vi.hoisted(() => {
    let current: NotificationState = {
      unread: 0,
      pendingPlanRequest: null,
      items: [],
      loading: false,
      error: null,
    }
    const subs = new Set<(v: NotificationState) => void>()
    const setNotifState = (v: NotificationState) => {
      current = v
      subs.forEach((run) => run(current))
    }
    return {
      refreshFromUsageMock: vi.fn().mockResolvedValue(undefined),
      loadNotificationsMock: vi.fn().mockResolvedValue(undefined),
      markReadMock: vi.fn().mockResolvedValue(undefined),
      deleteNotificationMock: vi.fn().mockResolvedValue(undefined),
      markAllReadMock: vi.fn().mockResolvedValue(undefined),
      setNotifState,
      notifStoreMock: {
        get state() {
          return current
        },
        subscribe(run: (v: NotificationState) => void) {
          subs.add(run)
          run(current)
          return () => subs.delete(run)
        },
        refreshFromUsage: (...a: unknown[]) => refreshFromUsageMock(...a),
        loadNotifications: (...a: unknown[]) => loadNotificationsMock(...a),
        markRead: (...a: unknown[]) => markReadMock(...a),
        deleteNotification: (...a: unknown[]) => deleteNotificationMock(...a),
        markAllRead: (...a: unknown[]) => markAllReadMock(...a),
      },
    }
  })

vi.mock('$lib/sync-store', () => ({
  syncStore: syncStoreMock,
}))

vi.mock('$lib/notification-store', async () => {
  const actual =
    await vi.importActual<typeof import('$lib/notification-store')>('$lib/notification-store')
  return {
    severityVariant: actual.severityVariant,
    formatRelativeTime: actual.formatRelativeTime,
    notificationStore: notifStoreMock,
  }
})

describe('NotificationBell', () => {
  beforeEach(() => {
    locale.set('es')
    initializeMock.mockClear()
    refreshFromUsageMock.mockClear()
    loadNotificationsMock.mockClear()
    markReadMock.mockClear()
    deleteNotificationMock.mockClear()
    markAllReadMock.mockClear()
    setSyncState(status({ state: 'disabled' }))
    setNotifState(notifState())
  })

  afterEach(() => {
    setSyncState(status({ state: 'disabled' }))
    setNotifState(notifState())
  })

  it('renders nothing when there is no sync session (disabled)', () => {
    render(NotificationBell)
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })

  it('shows the bell (no badge) when there is a session and 0 unread', async () => {
    render(NotificationBell)
    setSyncState(status({ state: 'idle' }))
    await waitFor(() => expect(screen.getByRole('button')).toBeInTheDocument())
    expect(screen.queryByTestId('notif-badge')).not.toBeInTheDocument()
  })

  it('shows the unread badge with the count when unread > 0', async () => {
    render(NotificationBell)
    setSyncState(status({ state: 'idle' }))
    setNotifState(notifState({ unread: 4 }))
    await waitFor(() => {
      const badge = screen.getByTestId('notif-badge')
      expect(badge).toBeInTheDocument()
      expect(badge.textContent?.trim()).toBe('4')
    })
  })

  it('caps the badge at 99+', async () => {
    render(NotificationBell)
    setSyncState(status({ state: 'idle' }))
    setNotifState(notifState({ unread: 250 }))
    await waitFor(() => {
      expect(screen.getByTestId('notif-badge').textContent?.trim()).toBe('99+')
    })
  })

  it('opens the panel on click and loads notifications lazily', async () => {
    render(NotificationBell)
    setSyncState(status({ state: 'idle' }))
    await waitFor(() => screen.getByRole('button'))

    await fireEvent.click(screen.getByRole('button'))
    expect(loadNotificationsMock).toHaveBeenCalledTimes(1)
    await waitFor(() => expect(screen.getByRole('dialog')).toBeInTheDocument())
  })

  it('renders server-rendered title + body plus mark-read and delete controls', async () => {
    render(NotificationBell)
    setSyncState(status({ state: 'idle' }))
    setNotifState(
      notifState({
        unread: 1,
        items: [
          {
            id: 'n1',
            kind: 'plan',
            category: '',
            severity: 'warning',
            title: 'Cuota casi llena',
            body: 'Estás usando el 92% de tu espacio.',
            created_at: 1_700_000_000,
            read_at: null,
          },
        ],
      })
    )
    await waitFor(() => screen.getByRole('button'))
    await fireEvent.click(screen.getByRole('button'))

    expect(await screen.findByText('Cuota casi llena')).toBeInTheDocument()
    expect(screen.getByText('Estás usando el 92% de tu espacio.')).toBeInTheDocument()

    // Mark-all + per-item mark read both available while there is an unread item.
    await fireEvent.click(screen.getByLabelText('Marcar como leída'))
    expect(markReadMock).toHaveBeenCalledWith('n1')

    await fireEvent.click(screen.getByLabelText('Eliminar notificación'))
    expect(deleteNotificationMock).toHaveBeenCalledWith('n1')
  })

  it('mark all read triggers the store action', async () => {
    render(NotificationBell)
    setSyncState(status({ state: 'idle' }))
    setNotifState(
      notifState({
        unread: 2,
        items: [
          {
            id: 'n1',
            kind: 'plan',
            category: '',
            severity: 'info',
            title: 'Uno',
            body: '',
            created_at: 1_700_000_000,
            read_at: null,
          },
        ],
      })
    )
    await waitFor(() => screen.getByRole('button'))
    await fireEvent.click(screen.getByRole('button'))

    await fireEvent.click(await screen.findByText('Marcar todas como leídas'))
    expect(markAllReadMock).toHaveBeenCalledTimes(1)
  })

  it('shows the empty state when there are no notifications', async () => {
    render(NotificationBell)
    setSyncState(status({ state: 'idle' }))
    setNotifState(notifState({ items: [] }))
    await waitFor(() => screen.getByRole('button'))
    await fireEvent.click(screen.getByRole('button'))
    expect(await screen.findByText('No tenés notificaciones.')).toBeInTheDocument()
  })
})
