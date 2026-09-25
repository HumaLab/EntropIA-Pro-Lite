/**
 * Upper bound on waiting for the main view's first painted frame before the
 * splash hands over (src-tauri/src/splash.rs).
 *
 * The main window starts hidden. WebView2 (Windows) still runs
 * requestAnimationFrame for it, so there the two frames arrive in a few tens of
 * milliseconds and this timeout never decides anything: the view has painted
 * before the window shows, with no white flash. WKWebView (macOS) and
 * WebKitGTK (Linux) never run frames for a window that is not on screen, so
 * without a bound the handover waited for the 20 s splash watchdog.
 */
export const FIRST_PAINT_TIMEOUT_MS = 250

/**
 * Resolve once the view has painted (two animation frames), or after
 * {@link FIRST_PAINT_TIMEOUT_MS} when the engine delivers no frames to a hidden
 * window, whichever comes first.
 */
export function waitForFirstPaint(): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, FIRST_PAINT_TIMEOUT_MS)
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        clearTimeout(timer)
        resolve()
      })
    )
  })
}
