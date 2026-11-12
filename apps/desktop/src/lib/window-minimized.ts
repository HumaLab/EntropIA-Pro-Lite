import { getCurrentWindow } from '@tauri-apps/api/window'

/**
 * Calls `onChange(true)` when the app window is minimized and
 * `onChange(false)` when it comes back.
 *
 * WebView2 does not mark the document hidden when its host window is
 * minimized, so `visibilitychange` alone never fires and frame loops keep
 * running at full rate (measured on the Store build 1.6.0: the Inicio
 * constellation used more CPU minimized than visible). Tauri still reports the
 * resize that a minimize causes, so the state is read back from the window.
 *
 * Outside Tauri (tests, plain browser) there is no window to watch and the
 * returned stop function is a no-op.
 */
export function watchWindowMinimized(onChange: (minimized: boolean) => void): () => void {
  let stopped = false
  let unlisteners: Array<() => void> = []
  let lastReported: boolean | null = null

  try {
    const appWindow = getCurrentWindow()

    const report = async () => {
      try {
        const minimized = await appWindow.isMinimized()
        if (stopped || minimized === lastReported) return
        lastReported = minimized
        onChange(minimized)
      } catch {
        // The window can be gone while closing; nothing to report then.
      }
    }

    void Promise.all([
      appWindow.onResized(() => void report()),
      appWindow.onFocusChanged(() => void report()),
    ])
      .then((registered) => {
        if (stopped) registered.forEach((unlisten) => unlisten())
        else unlisteners = registered
      })
      .catch(() => {})
  } catch {
    // Not running inside Tauri.
  }

  return () => {
    stopped = true
    unlisteners.forEach((unlisten) => unlisten())
    unlisteners = []
  }
}
