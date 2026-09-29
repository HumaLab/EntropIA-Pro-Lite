import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it, vi } from 'vitest'
import { createSyncCompletedWatcher } from '$lib/writing-sync-refresh'

/**
 * A background sync cycle that applied documents left Escritura's list stale
 * until an app restart: the view never subscribed to the sync store. The fix
 * mirrors DbBrowserView's subscription — skip the first snapshot, refresh
 * only when `last_sync_at` moves to a new value. Rendering WritingView needs
 * the whole store and Tauri behind it (the same reason every other
 * WritingView.*.test stays at the source level), so the guard lives in a
 * tiny pure helper tested here behaviorally, with source checks that the
 * view wires it into its mount/destroy lifecycle at all.
 */

describe('the sync-completion guard', () => {
  it('does not refresh on the first snapshot', () => {
    const refresh = vi.fn()
    const watch = createSyncCompletedWatcher(refresh)

    // `subscribe` pushes the current status synchronously; that push is a
    // baseline, not a completed cycle, and must not reload anything.
    watch({ last_sync_at: 42 })

    expect(refresh).not.toHaveBeenCalled()
  })

  it('refreshes exactly once when last_sync_at changes to a new value', () => {
    const refresh = vi.fn()
    const watch = createSyncCompletedWatcher(refresh)

    watch({ last_sync_at: null })
    watch({ last_sync_at: 100 })

    expect(refresh).toHaveBeenCalledTimes(1)
  })

  it('does not refresh while last_sync_at stays identical', () => {
    const refresh = vi.fn()
    const watch = createSyncCompletedWatcher(refresh)

    // First snapshot, then the intermediate ticks of a cycle in progress
    // (idle -> syncing -> idle): same completion stamp, no reload.
    watch({ last_sync_at: 100 })
    watch({ last_sync_at: 100 })
    watch({ last_sync_at: 100 })

    expect(refresh).not.toHaveBeenCalled()
  })

  it('refreshes once per completed pass and nothing in between', () => {
    const refresh = vi.fn()
    const watch = createSyncCompletedWatcher(refresh)

    watch({ last_sync_at: null }) // first snapshot (baseline)
    watch({ last_sync_at: 200 }) // cycle end -> refresh
    watch({ last_sync_at: 200 }) // idle -> syncing tick -> nothing
    watch({ last_sync_at: 300 }) // next cycle end -> refresh

    expect(refresh).toHaveBeenCalledTimes(2)
  })
})

describe('WritingView wires the guard into its lifecycle', () => {
  const SOURCE = readFileSync(resolve(import.meta.dirname, 'WritingView.svelte'), 'utf-8')

  it('initializes the sync store and subscribes through the guard on mount', () => {
    const init = SOURCE.indexOf('void syncStore.initialize()')
    const subscribe = SOURCE.indexOf('syncStore.subscribe(')

    expect(init, 'the sync store is no longer initialized on mount').toBeGreaterThan(-1)
    expect(subscribe, 'WritingView no longer subscribes to the sync store').toBeGreaterThan(-1)

    // DbBrowserView's ordering: the idempotent bootstrap first, then the
    // subscription, and the guard — never a bare refresh on every tick.
    expect(init).toBeLessThan(subscribe)
    expect(SOURCE).toContain('createSyncCompletedWatcher(')
  })

  it('refreshes list and notices behind the editor, guarded, and unsubscribes on destroy', () => {
    const subscribe = SOURCE.indexOf('syncStore.subscribe(')
    const onDestroy = SOURCE.indexOf('onDestroy(() => {')
    expect(subscribe, 'subscription is missing').toBeGreaterThan(-1)
    expect(onDestroy, 'onDestroy is missing').toBeGreaterThan(-1)
    expect(subscribe, 'the subscription must live in the onMount window').toBeLessThan(onDestroy)

    const callback = SOURCE.slice(subscribe, onDestroy)
    expect(callback, 'the refresh callback must respect the destroyed guard').toMatch(
      /if \(destroyed\) return/
    )
    // Both halves of the reload: the document list and its sync notices.
    // The list refresh is safe behind an open editor because `listDocuments`
    // merges a patch that never touches `open`/`content` (writing.ts `#set`).
    expect(callback).toContain('store.listDocuments()')
    expect(callback).toContain('refreshSyncNotices()')

    const destroyBody = SOURCE.slice(onDestroy, SOURCE.indexOf('\n  })', onDestroy))
    expect(destroyBody, 'onDestroy must release the sync subscription').toContain(
      'unsubscribeSync?.()'
    )
  })
})
