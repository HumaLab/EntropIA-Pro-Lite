import { navigation } from './navigation'
import { zoomIn, zoomOut, resetZoom } from './zoom'

/**
 * Escape interceptors let views consume the global Escape key before it
 * triggers back-navigation — e.g. cancel an active editing mode or guard
 * unsaved changes. Handlers run most-recently-registered first; the first
 * one that returns true consumes the key and navigation is skipped.
 */
export type EscapeInterceptor = () => boolean

const escapeInterceptors: EscapeInterceptor[] = []

/**
 * Register an Escape interceptor. Returns an unregister function — callers
 * (views) must unregister on unmount.
 */
export function registerEscapeInterceptor(interceptor: EscapeInterceptor): () => void {
  escapeInterceptors.push(interceptor)
  return () => {
    const index = escapeInterceptors.indexOf(interceptor)
    if (index >= 0) escapeInterceptors.splice(index, 1)
  }
}

/** Run interceptors LIFO; true when one of them consumed the Escape. */
function consumeEscape(): boolean {
  for (let i = escapeInterceptors.length - 1; i >= 0; i--) {
    if (escapeInterceptors[i]!()) return true
  }
  return false
}

/**
 * Global keyboard handler for the desktop app.
 * - Ctrl/Cmd +/-/0 → browser-style UI zoom.
 * - Escape → first lets registered interceptors cancel in-progress work;
 *   otherwise navigates back.
 * Returns a cleanup function that removes the listener.
 */
export function setupKeyboardShortcuts(): () => void {
  const handler = (e: KeyboardEvent) => {
    if (handleZoomShortcut(e)) return
    if (e.key !== 'Escape' || shouldIgnoreGlobalEscape(e)) return
    if (consumeEscape()) return
    navigation.back()
  }
  window.addEventListener('keydown', handler)
  return () => window.removeEventListener('keydown', handler)
}

/**
 * Zoom deliberately skips the input/dialog guards that Escape uses: browser
 * zoom works while typing too, and users expect the same here. Returns true
 * when the key was consumed.
 */
function handleZoomShortcut(e: KeyboardEvent): boolean {
  if (e.defaultPrevented) return false
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return false

  const action = zoomActionFor(e.key)
  if (!action) return false

  // Without this the WebView applies its own built-in zoom on top of ours.
  e.preventDefault()
  void action()
  return true
}

/** Covers the main row, the shifted variants and the numeric keypad. */
function zoomActionFor(key: string): (() => Promise<number>) | null {
  switch (key) {
    case '+':
    case '=':
    case 'Add':
      return zoomIn
    case '-':
    case '_':
    case 'Subtract':
      return zoomOut
    case '0':
      return resetZoom
    default:
      return null
  }
}

function shouldIgnoreGlobalEscape(e: KeyboardEvent): boolean {
  if (e.defaultPrevented) return true

  if (document.querySelector('[role="dialog"], [aria-modal="true"]')) {
    return true
  }

  const target = e.target instanceof Element ? e.target : null
  if (!target) return false

  const tagName = target.tagName.toLowerCase()
  return (
    tagName === 'input' ||
    tagName === 'textarea' ||
    tagName === 'select' ||
    target.closest('[contenteditable="true"]') !== null
  )
}
