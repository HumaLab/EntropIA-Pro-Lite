/**
 * Notification-center store (NOTIFICATIONS.md §1). Module-level state so the bell
 * badge survives navigation and component unmounts.
 *
 * Two data paths, mirroring the S2 contract:
 *  - The UNREAD BADGE COUNT piggybacks on `GET /v1/usage` (`SyncUsage.unread_notifications`).
 *    `refreshFromUsage()` adopts that count + the `pending_plan_request` flag; the sync
 *    cycle already refreshes usage, so the badge stays cheap.
 *  - The NOTIFICATION LIST is fetched on demand (`syncListNotifications`) only when the
 *    panel opens — the list is heavier and not needed until the user looks.
 *
 * Marking read is optimistic on the local list (`read_at`) and decrements the badge,
 * then reconciles with `/v1/usage` on the next `refreshFromUsage()`. Plain TypeScript
 * (not `.svelte.ts`) for full Vitest testability — mirrors `sync-store.ts`.
 *
 * The notification CONTENT (`title`/`body`) is rendered by the server; this store and
 * its consumers NEVER build or translate notification copy (only UI labels).
 */

import {
  syncGetUsage,
  syncListNotifications,
  syncMarkNotificationRead,
  type NotificationItem,
} from './sync'

/** The reactive snapshot pushed to subscribers. */
export interface NotificationState {
  /** Unread badge count, sourced from `SyncUsage.unread_notifications`. */
  unread: number
  /** Name of the plan with a pending change request, or `null`. */
  pendingPlanRequest: string | null
  /** The loaded notification list (newest-first as the server returns it). */
  items: NotificationItem[]
  /** True while the list is being fetched. */
  loading: boolean
  /** A human (already-mapped) error from the last list fetch, or `null`. */
  error: string | null
}

const EMPTY_STATE: NotificationState = {
  unread: 0,
  pendingPlanRequest: null,
  items: [],
  loading: false,
  error: null,
}

type NotificationSubscriber = (state: NotificationState) => void

export class NotificationStore {
  private _state: NotificationState = { ...EMPTY_STATE }
  private readonly _subscribers = new Set<NotificationSubscriber>()

  /**
   * Subscribes to state changes. Immediately pushes the current snapshot, then
   * notifies on every transition. Returns an unsubscribe (NavigationStore idiom).
   */
  subscribe(run: NotificationSubscriber): () => void {
    this._subscribers.add(run)
    run(this.snapshot())
    return () => {
      this._subscribers.delete(run)
    }
  }

  /** The current snapshot (defensive copy). */
  get state(): NotificationState {
    return this.snapshot()
  }

  private snapshot(): NotificationState {
    return { ...this._state, items: [...this._state.items] }
  }

  private emit(): void {
    const snapshot = this.snapshot()
    this._subscribers.forEach((run) => run(snapshot))
  }

  private patch(partial: Partial<NotificationState>): void {
    this._state = { ...this._state, ...partial }
    this.emit()
  }

  /**
   * Adopts the unread count + pending-plan flag from `/v1/usage`. Called on the sync
   * cycle / on demand. Swallows errors (the badge simply keeps its last value) so a
   * transient usage failure never wedges the UI.
   */
  async refreshFromUsage(): Promise<void> {
    try {
      const usage = await syncGetUsage()
      this.patch({
        unread: Math.max(0, usage.unread_notifications ?? 0),
        pendingPlanRequest: usage.pending_plan_request ?? null,
      })
    } catch (error) {
      console.warn('[NotificationStore] Failed to refresh usage badge:', error)
    }
  }

  /**
   * Fetches the notification list (panel open). `describeError` maps a thrown error
   * to a human message (injected so the store stays free of i18n imports).
   */
  async loadNotifications(describeError: (error: unknown) => string): Promise<void> {
    this.patch({ loading: true, error: null })
    try {
      const items = await syncListNotifications()
      this.patch({ items, loading: false })
    } catch (error) {
      this.patch({ loading: false, error: describeError(error) })
    }
  }

  /**
   * Marks one notification read: optimistic on the local list, decrements the badge,
   * then reconciles via `refreshFromUsage()`. Idempotent server-side.
   */
  async markRead(id: string): Promise<void> {
    const target = this._state.items.find((n) => n.id === id)
    if (target && target.read_at !== null) return // already read locally — no-op
    await syncMarkNotificationRead(id)
    this.applyLocalRead([id])
    await this.refreshFromUsage()
  }

  /** Marks every currently-unread notification read, then reconciles the badge. */
  async markAllRead(): Promise<void> {
    const unreadIds = this._state.items.filter((n) => n.read_at === null).map((n) => n.id)
    if (unreadIds.length === 0) return
    await Promise.all(unreadIds.map((id) => syncMarkNotificationRead(id)))
    this.applyLocalRead(unreadIds)
    await this.refreshFromUsage()
  }

  /** Stamps `read_at` on the given ids and drops the badge by the number newly read. */
  private applyLocalRead(ids: string[]): void {
    const idSet = new Set(ids)
    const now = Date.now()
    let newlyRead = 0
    const items = this._state.items.map((n) => {
      if (idSet.has(n.id) && n.read_at === null) {
        newlyRead += 1
        return { ...n, read_at: now }
      }
      return n
    })
    this.patch({ items, unread: Math.max(0, this._state.unread - newlyRead) })
  }

  /** Test-only / teardown: restore the pristine empty state. */
  reset(): void {
    this._state = { ...EMPTY_STATE }
    this.emit()
  }
}

/** Maps a notification severity to a StatusBadge-style colour token (panel dot/border). */
export function severityVariant(severity: string): 'info' | 'warning' | 'danger' {
  switch (severity.toLowerCase()) {
    case 'critical':
    case 'error':
    case 'danger':
      return 'danger'
    case 'warning':
    case 'warn':
      return 'warning'
    case 'info':
    default:
      return 'info'
  }
}

/** Relative-time formatter for notification timestamps (server `created_at`, seconds or ms). */
export function formatRelativeTime(createdAt: number, locale: string, now = Date.now()): string {
  // The server may send seconds (10-digit) or ms (13-digit) epochs; normalise to ms.
  const ms = createdAt < 1e12 ? createdAt * 1000 : createdAt
  const diffSec = Math.round((ms - now) / 1000)
  const abs = Math.abs(diffSec)
  const rtf = new Intl.RelativeTimeFormat(locale === 'en' ? 'en' : 'es', { numeric: 'auto' })
  if (abs < 60) return rtf.format(Math.trunc(diffSec), 'second')
  if (abs < 3600) return rtf.format(Math.trunc(diffSec / 60), 'minute')
  if (abs < 86_400) return rtf.format(Math.trunc(diffSec / 3600), 'hour')
  if (abs < 2_592_000) return rtf.format(Math.trunc(diffSec / 86_400), 'day')
  if (abs < 31_536_000) return rtf.format(Math.trunc(diffSec / 2_592_000), 'month')
  return rtf.format(Math.trunc(diffSec / 31_536_000), 'year')
}

export const notificationStore = new NotificationStore()
