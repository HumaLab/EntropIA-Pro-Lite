/**
 * The Navegador's native child webview, seen from the frontend.
 *
 * The page itself is not DOM: the backend draws it in a separate native
 * webview laid over the window. So the view only owns a placeholder element,
 * and this module turns that element's rect into the bounds the backend
 * needs and keeps the native webview in step with which view is showing.
 */

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { parseBrowserState, type BrowserState } from './navegador-tabs'

export type { BrowserState, BrowserTab } from './navegador-tabs'

/** A rectangle in logical pixels, relative to the window's content area. */
export type ViewerBounds = { x: number; y: number; width: number; height: number }

export const NAVEGADOR_STATE_EVENT = 'navegador://state'

/**
 * Where the placeholder sits, as the backend wants it. `getBoundingClientRect`
 * measures in CSS pixels of this webview; the window measures in logical
 * pixels, and the two differ by exactly the webview zoom (`lib/zoom.ts`).
 * Whole pixels, so the native webview does not straddle two device pixels.
 * `null` when there is nothing to place (collapsed, hidden, not laid out).
 */
export function computeBounds(
  rect: { left: number; top: number; width: number; height: number },
  zoom = 1
): ViewerBounds | null {
  const scale = Number.isFinite(zoom) && zoom > 0 ? zoom : 1
  const values = [rect.left, rect.top, rect.width, rect.height].map((n) => n * scale)
  if (!values.every(Number.isFinite)) return null
  const [x, y, width, height] = values.map(Math.round) as [number, number, number, number]
  if (width < 1 || height < 1) return null
  return { x, y, width, height }
}

export interface ViewerApi {
  open(url: string, bounds: ViewerBounds): Promise<void>
  setBounds(bounds: ViewerBounds): Promise<void>
  setVisible(visible: boolean): Promise<void>
  close(): Promise<void>
}

/**
 * There is one browser (its tabs are native webviews the backend keeps) and
 * possibly several Navegador views (a split pane, an app tab switched away and
 * back). The session decides who drives it: the view that showed it last owns
 * it, and a hidden or replaced view cannot move it. A view going away only
 * hides the browser: its tabs, their pages, history and sign-ins stay alive,
 * and the next view to show it puts the active tab back at its own rect. It
 * closes only when [close] is called (its app tab was closed) or the app exits
 * (the backend closes it then). Calls run one after another, so a fast unmount
 * and mount never interleave their native round trips.
 */
export function createViewerSession(api: ViewerApi) {
  let owner: string | null = null
  let opened = false
  let tail: Promise<unknown> = Promise.resolve()

  const enqueue = <T>(task: () => Promise<T>): Promise<T> => {
    const result = tail.then(task)
    tail = result.catch(() => undefined)
    return result
  }

  return {
    /** Whether the native webview exists, as of the calls already made. */
    isOpen(): boolean {
      return opened
    },

    show(id: string, url: string, bounds: ViewerBounds): Promise<void> {
      return enqueue(async () => {
        owner = id
        if (!opened) {
          await api.open(url, bounds)
          opened = true
          return
        }
        await api.setBounds(bounds)
        await api.setVisible(true)
      })
    },

    hide(id: string): Promise<void> {
      return enqueue(async () => {
        if (opened && owner === id) await api.setVisible(false)
      })
    },

    setBounds(id: string, bounds: ViewerBounds): Promise<void> {
      return enqueue(async () => {
        if (opened && owner === id) await api.setBounds(bounds)
      })
    },

    /** A view is going away: hide the browser if it was driving it, keep it alive. */
    detach(id: string): Promise<void> {
      return enqueue(async () => {
        if (owner !== id) return
        owner = null
        if (opened) await api.setVisible(false)
      })
    },

    /** Close the browser for good (its tab was closed). A no-op when none is open. */
    close(): Promise<void> {
      return enqueue(async () => {
        owner = null
        if (!opened) return
        opened = false
        await api.close()
      })
    },
  }
}

/** The real backend behind the session. */
export const navegadorApi: ViewerApi = {
  async open(url, bounds) {
    await invoke('navegador_open', { url, ...bounds })
  },
  async setBounds(bounds) {
    await invoke('navegador_set_bounds', { ...bounds })
  },
  async setVisible(visible) {
    await invoke('navegador_set_visible', { visible })
  },
  async close() {
    await invoke('navegador_close')
  },
}

export const navegadorSession = createViewerSession(navegadorApi)

// Everything that acts on a page names its tab, so it is the tab the person was
// looking at when they acted, not whichever became active in between.

export async function navegadorNavigate(tab: number, url: string): Promise<BrowserState> {
  return parseBrowserState(await invoke('navegador_navigate', { tab, url }))
}

export function navegadorBack(tab: number): Promise<void> {
  return invoke('navegador_back', { tab })
}

export function navegadorForward(tab: number): Promise<void> {
  return invoke('navegador_forward', { tab })
}

export function navegadorReload(tab: number): Promise<void> {
  return invoke('navegador_reload', { tab })
}

/** A new blank tab in front. The backend refuses it past four tabs. */
export async function navegadorNewTab(): Promise<BrowserState> {
  return parseBrowserState(await invoke('navegador_new_tab'))
}

export async function navegadorActivateTab(tab: number): Promise<BrowserState> {
  return parseBrowserState(await invoke('navegador_activate_tab', { tab }))
}

/** Close a tab and its page. The last tab is replaced by a blank one. */
export async function navegadorCloseTab(tab: number): Promise<BrowserState> {
  return parseBrowserState(await invoke('navegador_close_tab', { tab }))
}

export async function navegadorState(): Promise<BrowserState> {
  return parseBrowserState(await invoke('navegador_state'))
}

/** Follow the tabs and the active one. Only the main webview hears it. */
export function onNavegadorState(handler: (state: BrowserState) => void): Promise<UnlistenFn> {
  return listen<unknown>(NAVEGADOR_STATE_EVENT, (event) =>
    handler(parseBrowserState(event.payload))
  )
}
