/**
 * A statusbar shortcut asking Settings to open on a given tab.
 *
 * The route is just `{ name: 'settings' }`, so the tab travels beside it. The
 * request is consumed by the first Settings view that takes it: a later plain
 * visit to Settings opens its default tab again.
 */
export type RequestableSettingsTab = 'sync'

let pending: RequestableSettingsTab | null = null
const subscribers = new Set<(tab: RequestableSettingsTab) => void>()

/** Call before navigating to Settings. */
export function requestSettingsTab(tab: RequestableSettingsTab): void {
  if (subscribers.size === 0) {
    pending = tab
    return
  }
  pending = null
  subscribers.forEach((run) => run(tab))
}

/**
 * Hands `run` a request made before this Settings view mounted, then every
 * request made while it stays mounted. Returns the unsubscribe function.
 */
export function onSettingsTabRequest(run: (tab: RequestableSettingsTab) => void): () => void {
  if (pending) {
    const tab = pending
    pending = null
    run(tab)
  }
  subscribers.add(run)
  return () => {
    subscribers.delete(run)
  }
}
