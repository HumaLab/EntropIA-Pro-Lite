/**
 * Svelte context for pane-scoped navigation (tabs-split-view design).
 *
 * `WorkPane.svelte` (Stage 2) calls `setPaneNavigation` once per mounted
 * pane, at component init, with that pane's own `NavigationStore`. Every
 * pane-scoped view underneath it calls `getNavigation()` instead of
 * importing the (retired) `navigation` singleton, so the same component
 * tree works unmodified whether it is the only pane or one of a split pair.
 *
 * Outside any pane (a chrome component, or a test rendering a view in
 * isolation) both getters fall back to the workspace's active tab.
 */
import { getContext, setContext } from 'svelte'
import type { NavigationStore } from './navigation'
import { workspace } from './workspace'

interface PaneContextValue {
  navigation: NavigationStore
  paneId: string
}

const PANE_CONTEXT_KEY = Symbol('entropia.pane-context')

export function setPaneNavigation(navigation: NavigationStore, paneId: string): void {
  setContext<PaneContextValue>(PANE_CONTEXT_KEY, { navigation, paneId })
}

/**
 * `getContext` (and `hasContext`) throw `lifecycle_outside_component` in
 * Svelte 5 when called outside component initialisation, rather than
 * returning `undefined` the way earlier Svelte versions did. Both public
 * getters below need to work outside any component — a chrome component
 * with no pane ancestor, or a test rendering a view in isolation — so this
 * wrapper turns that throw into the "no provider above" case they already
 * fall back from.
 */
function paneContext(): PaneContextValue | undefined {
  try {
    return getContext<PaneContextValue | undefined>(PANE_CONTEXT_KEY)
  } catch {
    return undefined
  }
}

export function getNavigation(): NavigationStore {
  return paneContext()?.navigation ?? workspace.activeNavigation
}

export function getPaneId(): string {
  return paneContext()?.paneId ?? workspace.activeTabId
}
