import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import {
  NotificationStore,
  formatRelativeTime,
  severityVariant,
  type NotificationState,
} from './notification-store'
import type { NotificationItem, SyncUsage } from './sync'

const mockInvoke = vi.mocked(invoke)

function notif(overrides: Partial<NotificationItem> = {}): NotificationItem {
  return {
    id: 'ntf-1',
    kind: 'plan',
    category: '',
    severity: 'info',
    title: 'Título',
    body: 'Cuerpo',
    created_at: 1_700_000_000,
    read_at: null,
    ...overrides,
  }
}

function usage(overrides: Partial<SyncUsage> = {}): SyncUsage {
  return {
    rows: 0,
    blobs_count: 0,
    blobs_bytes: 0,
    quota_bytes: 0,
    plan_name: 'Free',
    expires_at: null,
    unread_notifications: 0,
    pending_plan_request: null,
    ...overrides,
  }
}

const describeError = (e: unknown) => (e instanceof Error ? e.message : String(e))

describe('NotificationStore', () => {
  beforeEach(() => {
    mockInvoke.mockReset()
  })

  it('starts empty and pushes the snapshot to new subscribers', () => {
    const store = new NotificationStore()
    const seen: NotificationState[] = []
    store.subscribe((s) => seen.push(s))
    expect(seen).toHaveLength(1)
    expect(seen.at(0)).toMatchObject({ unread: 0, items: [], pendingPlanRequest: null })
  })

  it('refreshFromUsage adopts the unread count and pending plan from /v1/usage', async () => {
    mockInvoke.mockResolvedValue(usage({ unread_notifications: 3, pending_plan_request: '5 GB' }))
    const store = new NotificationStore()
    await store.refreshFromUsage()
    expect(mockInvoke).toHaveBeenCalledWith('sync_get_usage')
    expect(store.state.unread).toBe(3)
    expect(store.state.pendingPlanRequest).toBe('5 GB')
  })

  it('refreshFromUsage swallows a usage failure and keeps the last badge', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    mockInvoke.mockResolvedValueOnce(usage({ unread_notifications: 2 }))
    const store = new NotificationStore()
    await store.refreshFromUsage()
    expect(store.state.unread).toBe(2)

    mockInvoke.mockRejectedValueOnce(new Error('offline'))
    await store.refreshFromUsage()
    expect(store.state.unread).toBe(2) // unchanged
    warn.mockRestore()
  })

  it('loadNotifications fetches the list and clears loading', async () => {
    mockInvoke.mockResolvedValue([notif({ id: 'a' }), notif({ id: 'b' })])
    const store = new NotificationStore()
    await store.loadNotifications(describeError)
    expect(mockInvoke).toHaveBeenCalledWith('sync_list_notifications', {
      since: undefined,
      limit: undefined,
    })
    expect(store.state.items).toHaveLength(2)
    expect(store.state.loading).toBe(false)
    expect(store.state.error).toBeNull()
  })

  it('loadNotifications surfaces a mapped error on failure', async () => {
    mockInvoke.mockRejectedValue(new Error('boom'))
    const store = new NotificationStore()
    await store.loadNotifications(describeError)
    expect(store.state.error).toBe('boom')
    expect(store.state.loading).toBe(false)
  })

  it('markRead marks the item read, lowers the badge, and reconciles via usage', async () => {
    // list → mark → usage reconcile
    mockInvoke
      .mockResolvedValueOnce([notif({ id: 'a' }), notif({ id: 'b' })]) // list
    const store = new NotificationStore()
    await store.loadNotifications(describeError)
    store['_state'].unread = 2 // seed badge as if usage reported 2

    mockInvoke
      .mockResolvedValueOnce(undefined) // sync_mark_notification_read
      .mockResolvedValueOnce(usage({ unread_notifications: 1 })) // reconcile
    await store.markRead('a')

    expect(mockInvoke).toHaveBeenCalledWith('sync_mark_notification_read', { id: 'a' })
    const a = store.state.items.find((n) => n.id === 'a')
    expect(a?.read_at).not.toBeNull()
    expect(store.state.unread).toBe(1)
  })

  it('markRead is a no-op when the item is already read locally', async () => {
    mockInvoke.mockResolvedValueOnce([notif({ id: 'a', read_at: 123 })])
    const store = new NotificationStore()
    await store.loadNotifications(describeError)
    mockInvoke.mockClear()

    await store.markRead('a')
    expect(mockInvoke).not.toHaveBeenCalled()
  })

  it('markAllRead marks every unread item and reconciles once', async () => {
    mockInvoke.mockResolvedValueOnce([
      notif({ id: 'a' }),
      notif({ id: 'b', read_at: 5 }),
      notif({ id: 'c' }),
    ])
    const store = new NotificationStore()
    await store.loadNotifications(describeError)
    store['_state'].unread = 2

    mockInvoke
      .mockResolvedValueOnce(undefined) // mark a
      .mockResolvedValueOnce(undefined) // mark c
      .mockResolvedValueOnce(usage({ unread_notifications: 0 })) // reconcile
    await store.markAllRead()

    expect(store.state.items.every((n) => n.read_at !== null)).toBe(true)
    expect(store.state.unread).toBe(0)
  })

  it('markAllRead is a no-op when there are no unread items', async () => {
    mockInvoke.mockResolvedValueOnce([notif({ id: 'a', read_at: 1 })])
    const store = new NotificationStore()
    await store.loadNotifications(describeError)
    mockInvoke.mockClear()

    await store.markAllRead()
    expect(mockInvoke).not.toHaveBeenCalled()
  })
})

describe('severityVariant', () => {
  it('maps severities to colour tokens', () => {
    expect(severityVariant('info')).toBe('info')
    expect(severityVariant('warning')).toBe('warning')
    expect(severityVariant('critical')).toBe('danger')
    expect(severityVariant('error')).toBe('danger')
    expect(severityVariant('UNKNOWN')).toBe('info') // defaults to info
  })
})

describe('formatRelativeTime', () => {
  const now = 1_700_000_000_000 // ms

  it('formats seconds-epoch input (server may send seconds)', () => {
    // 2 minutes ago, in seconds
    const out = formatRelativeTime(1_700_000_000 - 120, 'es', now)
    expect(out).toMatch(/2/)
  })

  it('formats ms-epoch input', () => {
    const out = formatRelativeTime(now - 3 * 3600 * 1000, 'en', now)
    expect(out.toLowerCase()).toMatch(/hour/)
  })
})
