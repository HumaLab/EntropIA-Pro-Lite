/**
 * Turns a `sync:status` tick into a Writing manuscript-list refresh.
 *
 * The engine emits `sync:status` on every sync transition — idle -> syncing
 * -> idle — but only the end of a successful cycle applies documents, and
 * that moment arrives stamped with a fresh `last_sync_at`. This is the very
 * guard DbBrowserView mounts on its syncStore subscription: skip the
 * store's synchronous first snapshot (a baseline, not a change), then fire
 * once per new completion timestamp and ignore the intermediate status
 * ticks in between.
 */

/** The one field of a sync status snapshot the guard reasons about. */
export interface SyncCompletionSnapshot {
  last_sync_at: number | null
}

/**
 * Wraps `onCompleted` in the first-snapshot / `last_sync_at` guard.
 *
 * One watcher per mount: `subscribe` pushes the current snapshot
 * synchronously, and that push must never refresh — on any mount.
 */
export function createSyncCompletedWatcher(
  onCompleted: () => void
): (status: SyncCompletionSnapshot) => void {
  let sawFirstSnapshot = false
  let lastCompletedAt: number | null = null

  return (status) => {
    if (!sawFirstSnapshot) {
      sawFirstSnapshot = true
      lastCompletedAt = status.last_sync_at
      return
    }
    // Only a completed sync pass refreshes, not every intermediate status
    // tick (e.g. idle -> syncing). Any change counts, exactly as in
    // DbBrowserView — including a reset back to null.
    if (status.last_sync_at !== lastCompletedAt) {
      lastCompletedAt = status.last_sync_at
      onCompleted()
    }
  }
}
