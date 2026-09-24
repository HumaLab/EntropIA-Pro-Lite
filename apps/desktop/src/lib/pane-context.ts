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
 *
 * `getNavigation()` and `getPaneId()` are init-only: call them once, at the
 * top level of a component's `<script>`, and keep the returned value in a
 * local binding for anything that runs later (an event handler, a
 * `setTimeout`, a promise callback). Calling either getter again from inside
 * one of those is a bug, not a lazy-lookup convenience — see the getter
 * doc comments below for why, and `pane-context-fixtures/ProbeFixture.svelte`
 * for the pattern to copy.
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

/**
 * Must be called during component initialisation (the top level of a
 * `<script>` block), like `getContext` itself. Svelte's component context
 * is only available synchronously while a component is being set up: after
 * that — inside an event handler, a `setTimeout`, a promise `.then`, an
 * async continuation — Svelte no longer has a "current component" to look
 * the context up against, so a lazy call here cannot tell "no pane above me"
 * apart from "called too late, context lookup no longer applies", and both
 * fall through to the workspace's *active* tab, which is silently wrong for
 * a pane that is not the active one.
 *
 * Capture the result once, at init, and reuse that binding:
 * `const navigation = getNavigation()` — see
 * `pane-context-fixtures/ProbeFixture.svelte`.
 */
export function getNavigation(): NavigationStore {
  return paneContext()?.navigation ?? workspace.activeNavigation
}

/** Init-only, for the same reason as {@link getNavigation}: capture the
 *  result once and reuse that binding, never call this from a handler or
 *  async callback. */
export function getPaneId(): string {
  return paneContext()?.paneId ?? workspace.activeTabId
}
