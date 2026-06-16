import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { badgeVariantForState, SyncStore } from './sync-store'
import type { SyncStatus } from './sync'

const mockInvoke = vi.mocked(invoke)

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

/** A fake `listen` that captures the registered callback so tests can drive events. */
function makeListen() {
  let captured: ((e: { payload: unknown }) => void) | null = null
  const unlisten = vi.fn()
  const listen = vi.fn(async (_event: string, cb: (e: { payload: unknown }) => void) => {
    captured = cb
    return unlisten
  })
  return {
    listen,
    unlisten,
    emit(payload: SyncStatus) {
      captured?.({ payload })
    },
  }
}

describe('SyncStore', () => {
  beforeEach(() => {
    mockInvoke.mockReset()
  })

  it('starts in the disabled snapshot and pushes it to new subscribers', () => {
    const store = new SyncStore()
    const seen: SyncStatus[] = []
    store.subscribe((s) => seen.push(s))
    expect(seen).toHaveLength(1)
    expect(seen.at(0)?.state).toBe('disabled')
  })

  it('bootstraps from sync_status and adopts the snapshot', async () => {
    mockInvoke.mockResolvedValue(status({ state: 'idle', pending: 2 }))
    const store = new SyncStore()
    const seen: SyncStatus[] = []
    store.subscribe((s) => seen.push(s))

    const { listen } = makeListen()
    await store.initialize(listen)

    expect(mockInvoke).toHaveBeenCalledWith('sync_status')
    expect(store.status.state).toBe('idle')
    expect(store.status.pending).toBe(2)
    // Subscriber saw the disabled snapshot then the bootstrapped one.
    expect(seen.at(-1)?.state).toBe('idle')
  })

  it('updates state from sync:status events', async () => {
    mockInvoke.mockResolvedValue(status({ state: 'idle' }))
    const store = new SyncStore()
    const seen: SyncStatus[] = []
    store.subscribe((s) => seen.push(s))

    const events = makeListen()
    await store.initialize(events.listen)

    events.emit(status({ state: 'syncing', pending: 5 }))
    expect(store.status.state).toBe('syncing')
    expect(store.status.pending).toBe(5)
    expect(seen.at(-1)?.state).toBe('syncing')

    events.emit(status({ state: 'offline', message: 'sin conexión' }))
    expect(store.status.state).toBe('offline')
    expect(store.status.message).toBe('sin conexión')
  })

  it('is idempotent: initialize only attaches once', async () => {
    mockInvoke.mockResolvedValue(status())
    const store = new SyncStore()
    const events = makeListen()

    await store.initialize(events.listen)
    await store.initialize(events.listen)

    expect(events.listen).toHaveBeenCalledTimes(1)
    expect(mockInvoke).toHaveBeenCalledTimes(1)
  })

  it('keeps the disabled snapshot and drops the memo when the bootstrap fetch fails', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    mockInvoke.mockRejectedValueOnce(new Error('offline'))
    const store = new SyncStore()
    const events = makeListen()

    await store.initialize(events.listen)
    expect(store.status.state).toBe('disabled')

    // The memo was dropped: a second initialize retries the fetch.
    mockInvoke.mockResolvedValueOnce(status({ state: 'idle' }))
    await store.initialize(events.listen)
    expect(store.status.state).toBe('idle')
    warn.mockRestore()
  })

  it('refresh re-fetches the snapshot', async () => {
    const store = new SyncStore()
    mockInvoke.mockResolvedValue(status({ state: 'error', message: 'boom' }))
    await store.refresh()
    expect(store.status.state).toBe('error')
    expect(store.status.message).toBe('boom')
  })

  it('reset returns to the disabled snapshot and tears down the listener', async () => {
    mockInvoke.mockResolvedValue(status({ state: 'idle' }))
    const store = new SyncStore()
    const events = makeListen()
    await store.initialize(events.listen)
    expect(store.status.state).toBe('idle')

    store.reset()
    expect(events.unlisten).toHaveBeenCalledTimes(1)
    expect(store.status.state).toBe('disabled')
  })
})

describe('badgeVariantForState', () => {
  it('maps states to badge variants (DESIGN §11)', () => {
    expect(badgeVariantForState('idle', 0)).toBe('success')
    expect(badgeVariantForState('disabled', 0)).toBe('success')
    expect(badgeVariantForState('syncing', 0)).toBe('info')
    expect(badgeVariantForState('offline', 0)).toBe('warning')
    expect(badgeVariantForState('error', 0)).toBe('danger')
  })

  it('unacknowledged conflicts dominate any state', () => {
    expect(badgeVariantForState('idle', 3)).toBe('danger')
    expect(badgeVariantForState('syncing', 1)).toBe('danger')
  })
})
