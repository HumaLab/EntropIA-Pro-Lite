/**
 * Cloud-sync status store (DESIGN §11). Module-level state so the sync indicator
 * survives navigation and component unmounts: status arrives on a long-lived Tauri
 * event listener, not in a destroyed component.
 *
 * Mirrors the NavigationStore subscription idiom (a `subscribe()` that immediately
 * pushes the current snapshot and returns an unsubscribe). The store is fed by the
 * `sync_status` bootstrap call plus the `sync:status` event stream
 * (SyncEventManager). It NEVER touches `settingsGet`/`settingsSet` — sync config
 * lives only in `sync_meta` + the keyring (DESIGN §11).
 */

import { listen } from '@tauri-apps/api/event'
import { SyncEventManager, syncStatus, type SyncState, type SyncStatus } from './sync'

/** The disabled snapshot: opt-in default, renders nothing in the UI (DESIGN §11). */
const DISABLED_STATUS: SyncStatus = {
  state: 'disabled',
  last_sync_at: null,
  pending: 0,
  blobs_pending: 0,
  pending_blob_bytes: 0,
  conflicts: 0,
  clock_warning: false,
}

type SyncSubscriber = (status: SyncStatus) => void

export class SyncStore {
  private _status: SyncStatus = { ...DISABLED_STATUS }
  private readonly _subscribers = new Set<SyncSubscriber>()
  private _events: SyncEventManager | null = null
  private _bootstrapPromise: Promise<void> | null = null

  /**
   * Subscribes to status changes. Immediately pushes the current snapshot, then
   * notifies on every transition. Returns an unsubscribe (NavigationStore idiom).
   */
  subscribe(run: SyncSubscriber): () => void {
    this._subscribers.add(run)
    run(this.snapshot())
    return () => {
      this._subscribers.delete(run)
    }
  }

  /** The current status snapshot (defensive copy). */
  get status(): SyncStatus {
    return this.snapshot()
  }

  private snapshot(): SyncStatus {
    return { ...this._status }
  }

  private emit(): void {
    const snapshot = this.snapshot()
    this._subscribers.forEach((run) => run(snapshot))
  }

  /**
   * Replaces the current status and notifies subscribers. A malformed snapshot
   * (e.g. an `undefined` IPC return in a test or a partial payload) is coerced to
   * the disabled snapshot rather than crashing downstream `.length` reads.
   */
  setStatus(status: SyncStatus | null | undefined): void {
    this._status =
      status && typeof status === 'object' && typeof status.state === 'string'
        ? status
        : { ...DISABLED_STATUS }
    this.emit()
  }

  /**
   * Idempotent bootstrap: registers the `sync:status` listener (so live
   * transitions flow in) and loads the initial snapshot via `sync_status`.
   * The `listen` function is injected for testability — defaults to the Tauri
   * event listener. Subsequent calls are no-ops.
   */
  initialize(
    listenFn: (
      event: string,
      callback: (e: { payload: unknown }) => void
    ) => Promise<() => void> = listen
  ): Promise<void> {
    this._bootstrapPromise ??= this.doInitialize(listenFn)
    return this._bootstrapPromise
  }

  private async doInitialize(
    listenFn: (
      event: string,
      callback: (e: { payload: unknown }) => void
    ) => Promise<() => void>
  ): Promise<void> {
    // Start the listener BEFORE the bootstrap fetch so no transition emitted
    // between the two is lost.
    this._events = new SyncEventManager((status) => this.setStatus(status))
    try {
      await this._events.startListening(listenFn)
    } catch (error) {
      console.warn('[SyncStore] Failed to start the status listener:', error)
    }

    try {
      const status = await syncStatus()
      this.setStatus(status)
    } catch (error) {
      // Bootstrap failed: keep the disabled snapshot and let the next manual
      // refresh retry. We do NOT memoize a failed listener attach above, but the
      // fetch failing here should not wedge the store forever, so drop the memo.
      console.warn('[SyncStore] Failed to bootstrap sync status:', error)
      this._bootstrapPromise = null
    }
  }

  /** Re-fetches the status snapshot on demand (e.g. after a command resolves). */
  async refresh(): Promise<void> {
    try {
      this.setStatus(await syncStatus())
    } catch (error) {
      console.warn('[SyncStore] Failed to refresh sync status:', error)
    }
  }

  /** Tears down the event listener (test isolation / teardown). */
  dispose(): void {
    this._events?.stopListening()
    this._events = null
  }

  /** Test-only: restore the pristine disabled state and drop the bootstrap memo. */
  reset(): void {
    this.dispose()
    this._status = { ...DISABLED_STATUS }
    this._bootstrapPromise = null
    this.emit()
  }
}

/** Returns the StatusBadge variant for a sync state (DESIGN §11 mapping). */
export function badgeVariantForState(
  state: SyncState,
  conflicts: number
): 'success' | 'neutral' | 'info' | 'warning' | 'danger' {
  // Unacknowledged conflicts dominate even an otherwise-idle state.
  if (conflicts > 0) return 'danger'
  switch (state) {
    case 'syncing':
      return 'info'
    case 'offline':
      return 'warning'
    case 'error':
      return 'danger'
    case 'idle':
    case 'disabled':
    default:
      return 'success'
  }
}

export const syncStore = new SyncStore()
