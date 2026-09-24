/**
 * Coalesces "something changed, reload" signals for the DB browser.
 *
 * The view subscribes to several independent change sources — the batch
 * queue, document/page import or deletion, and sync completion — each of
 * which may fire several times in a short burst (a running OCR batch emits a
 * `processing:changed` tick per task settling, for instance). Reloading the
 * grid on every single signal would refetch the schema and rows dozens of
 * times during one batch; this collapses a burst into a single trailing
 * reload, and never runs two reloads at once.
 *
 * Pure timer/promise bookkeeping: no Svelte, no Tauri. The view supplies
 * `onReload` (its existing `refreshSchema` → `loadTables(selectedTable)`
 * path) and calls `notify()` from each subscription/listener.
 */

export interface DbBrowserAutoReloadOptions {
  /** Re-reads the open table's schema and rows. May return a promise. */
  onReload: () => unknown
  /** Trailing debounce window: how long a quiet period must last before a
   *  burst of `notify()` calls collapses into one reload. */
  delayMs: number
}

export interface DbBrowserAutoReload {
  /** Records a change. Resets the trailing debounce window. */
  notify: () => void
  /** Stops the timer and drops the pending reload, if any. Idempotent. */
  dispose: () => void
}

export function createDbBrowserAutoReload({
  onReload,
  delayMs,
}: DbBrowserAutoReloadOptions): DbBrowserAutoReload {
  let timer: ReturnType<typeof setTimeout> | null = null
  let reloadInFlight = false
  let reloadQueued = false
  let disposed = false

  function clearTimer() {
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
  }

  function runReload() {
    reloadInFlight = true
    // Called synchronously (not deferred through a microtask) so a
    // synchronous `onReload` reports its call before the next tick — timer
    // tests assert on `onReload` right after advancing past the debounce.
    void Promise.resolve(onReload())
      .catch(() => {
        // The view's own onReload already turns a failure into its `error`
        // state; this coalescer only owns scheduling, not error reporting.
      })
      .finally(() => {
        reloadInFlight = false
        if (disposed) return
        if (reloadQueued) {
          reloadQueued = false
          runReload()
        }
      })
  }

  function fire() {
    timer = null
    if (disposed) return

    // Never overlap: a change that arrives while a reload is still running
    // is not lost, it is queued for exactly one follow-up reload once the
    // current one settles.
    if (reloadInFlight) {
      reloadQueued = true
      return
    }

    runReload()
  }

  function notify() {
    if (disposed) return
    clearTimer()
    timer = setTimeout(fire, delayMs)
  }

  function dispose() {
    disposed = true
    reloadQueued = false
    clearTimer()
  }

  return { notify, dispose }
}
