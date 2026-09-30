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

/** A rectangle in logical pixels, relative to the window's content area. */
export type ViewerBounds = { x: number; y: number; width: number; height: number }

/** What the backend reports about the page; mirrors `ViewerState` in Rust. */
export type ViewerState = {
  url: string | null
  title: string | null
  blocked: string | null
}

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
 * There is one native webview and possibly several Navegador views (a split
 * pane, a tab switched away and back). The session decides who drives it:
 * the view that showed it last owns it, a hidden or replaced view cannot move
 * it, and the last view to go away closes it. Calls run one after another, so
 * a fast unmount and mount never interleave their native round trips.
 */
export function createViewerSession(api: ViewerApi) {
  const mounted = new Set<string>()
  let owner: string | null = null
  let opened = false
  let tail: Promise<unknown> = Promise.resolve()

  const enqueue = <T>(task: () => Promise<T>): Promise<T> => {
    const result = tail.then(task)
    tail = result.catch(() => undefined)
    return result
  }

  return {
    attach(id: string): void {
      mounted.add(id)
    },

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

    detach(id: string): Promise<void> {
      mounted.delete(id)
      return enqueue(async () => {
        const wasOwner = owner === id
        if (wasOwner) owner = null
        if (!opened) return
        if (mounted.size === 0) {
          opened = false
          await api.close()
        } else if (wasOwner) {
          await api.setVisible(false)
        }
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

export function navegadorNavigate(url: string): Promise<ViewerState> {
  return invoke<ViewerState>('navegador_navigate', { url })
}

export function navegadorBack(): Promise<void> {
  return invoke('navegador_back')
}

export function navegadorForward(): Promise<void> {
  return invoke('navegador_forward')
}

export function navegadorReload(): Promise<void> {
  return invoke('navegador_reload')
}

export function navegadorState(): Promise<ViewerState> {
  return invoke<ViewerState>('navegador_state')
}

/** Follow what the backend says about the page. Only the main webview hears it. */
export function onNavegadorState(handler: (state: ViewerState) => void): Promise<UnlistenFn> {
  return listen<ViewerState>(NAVEGADOR_STATE_EVENT, (event) => handler(event.payload))
}
