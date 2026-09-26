import { resolveDesktopPlatform, type DesktopPlatform } from './platform'

/**
 * Upper bound on waiting for the main view's first painted frame before the
 * splash hands over (src-tauri/src/splash.rs), on macOS and Linux only.
 *
 * The main window starts hidden. WKWebView (macOS) and WebKitGTK (Linux) never
 * run frames for a window that is not on screen, so without a bound the
 * handover waited for the 20 s splash watchdog. WebView2 (Windows) does run
 * requestAnimationFrame for a hidden window, so Windows keeps waiting for the
 * real frames: a timeout there could only reveal the window before it painted
 * (a white flash) on a slow first launch.
 */
export const FIRST_PAINT_TIMEOUT_MS = 250

/**
 * Resolve once the view has painted (two animation frames). On platforms whose
 * engine delivers no frames to a hidden window, also resolve after
 * {@link FIRST_PAINT_TIMEOUT_MS}, whichever comes first.
 */
export function waitForFirstPaint(
  platform: DesktopPlatform = resolveDesktopPlatform()
): Promise<void> {
  return new Promise((resolve) => {
    const timer = platform === 'windows' ? null : setTimeout(resolve, FIRST_PAINT_TIMEOUT_MS)
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        if (timer !== null) clearTimeout(timer)
        resolve()
      })
    )
  })
}
