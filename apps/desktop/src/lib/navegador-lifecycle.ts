/**
 * When the browser goes away. It is hidden, not closed, whenever the Navegador
 * view unmounts (another section, another tab), so it needs a second rule for
 * the moment it should really end: its tab is closed.
 *
 * There is one browser, shared by every Navegador tab (the view shown last
 * drives it), so closing a Navegador tab ends it only when no other tab is
 * still on the Navegador. A tab that navigated away from the Navegador before
 * being closed does not end it: the browser stays hidden until a Navegador tab
 * closes or the app exits. Both are deliberate limits of this first version.
 */

import { navegadorSession } from './navegador'
import { navegadorStore } from './navegador-store'
import type { WorkspaceStore } from './workspace'

/** Whether closing a tab that was showing `closedView` should close the browser. */
export function shouldReleaseBrowser(
  closedView: string,
  remainingViews: readonly string[]
): boolean {
  return closedView === 'navegador' && !remainingViews.includes('navegador')
}

/**
 * Call `release` when the last Navegador tab closes. A failing release is
 * logged and never gets in the way of closing the tab. Returns the function
 * that stops watching.
 */
export function watchNavegadorTabs(
  workspace: Pick<WorkspaceStore, 'onTabClosed'>,
  release: () => void | Promise<void>
): () => void {
  return workspace.onTabClosed(({ view, remaining }) => {
    const remainingViews = remaining.map((tab) => tab.navigation.current.name)
    if (!shouldReleaseBrowser(view.name, remainingViews)) return
    try {
      void Promise.resolve(release()).catch((reason) => {
        console.warn('[navegador] could not close the browser:', reason)
      })
    } catch (reason) {
      console.warn('[navegador] could not close the browser:', reason)
    }
  })
}

/**
 * What the shell installs: closing the last Navegador tab closes the browser
 * and forgets its drafts and download list, so the next Navegador starts clean.
 */
export function installNavegadorLifecycle(
  workspace: Pick<WorkspaceStore, 'onTabClosed'>,
  deps: {
    session: { close(): Promise<void> }
    store: { clearAll(): void }
  } = { session: navegadorSession, store: navegadorStore }
): () => void {
  return watchNavegadorTabs(workspace, async () => {
    deps.store.clearAll()
    await deps.session.close()
  })
}
