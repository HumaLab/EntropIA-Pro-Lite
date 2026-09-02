/**
 * Browser-style UI zoom for the desktop app.
 *
 * The app styles everything in `px`, so scaling the root font size would move
 * almost nothing. Instead this drives the WebView's own zoom (the same knob a
 * browser's Ctrl +/- uses), which scales the whole surface — text, icons,
 * layout and canvases alike — without touching a single stylesheet.
 */

import { get, writable, type Readable } from 'svelte/store'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { settingsGet, settingsSet, SETTINGS_KEYS } from './settings'

/** 75%..125% in 5% steps: the ±25% range around the design size. */
export const ZOOM_MIN = 0.75
export const ZOOM_MAX = 1.25
export const ZOOM_STEP = 0.05
export const ZOOM_DEFAULT = 1

export const ZOOM_SETTING_KEY = SETTINGS_KEYS.UI_ZOOM_FACTOR

const zoomStore = writable(ZOOM_DEFAULT)

/**
 * The applied factor, for UI that has to stay in step with it. Read-only on
 * purpose: the keyboard and the zoom menu both change it through the actions
 * below, which are the only path that also talks to the webview.
 */
export const zoomFactor: Readable<number> = { subscribe: zoomStore.subscribe }

/**
 * Snap a factor onto the nearest 5% step inside the allowed range. Anything
 * unusable — a corrupt stored value, a division gone wrong — lands on 100%.
 */
export function clampZoom(factor: number): number {
  if (!Number.isFinite(factor)) return ZOOM_DEFAULT
  const snapped = Math.round(factor / ZOOM_STEP) * ZOOM_STEP
  const bounded = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, snapped))
  // Steps of 0.05 accumulate binary drift (0.95 - 0.05 = 0.8999...); two
  // decimals is the exact precision this scale needs.
  return Math.round(bounded * 100) / 100
}

/** The factor currently applied to the webview. */
export function getZoom(): number {
  return get(zoomStore)
}

/**
 * Push `factor` to the webview and remember it. When the webview refuses (no
 * Tauri host, permission missing) the previous factor stays in force, because
 * a remembered zoom that never took effect would lie to every later step.
 */
async function applyZoom(factor: number, persist: boolean): Promise<number> {
  const target = clampZoom(factor)
  const previous = getZoom()

  try {
    await getCurrentWebview().setZoom(target)
  } catch (e) {
    console.warn('[zoom] webview refused the zoom factor:', e)
    return previous
  }

  zoomStore.set(target)
  if (persist) {
    try {
      await settingsSet(ZOOM_SETTING_KEY, String(target))
    } catch (e) {
      // A zoom that cannot be stored still works for this session.
      console.warn('[zoom] could not persist the zoom factor:', e)
    }
  }
  return target
}

export function zoomIn(): Promise<number> {
  return applyZoom(getZoom() + ZOOM_STEP, true)
}

export function zoomOut(): Promise<number> {
  return applyZoom(getZoom() - ZOOM_STEP, true)
}

export function resetZoom(): Promise<number> {
  return applyZoom(ZOOM_DEFAULT, true)
}

/**
 * Restore the stored zoom at startup. Always applies a factor — even the
 * default — because WebView2 can carry a stale zoom across a webview restart,
 * and a fresh JS context is the only reliable moment to correct it.
 *
 * Requires the settings store, so call it after the database is initialized.
 */
export async function initZoom(): Promise<number> {
  let stored: string | null = null
  try {
    stored = await settingsGet(ZOOM_SETTING_KEY)
  } catch (e) {
    console.warn('[zoom] could not read the stored zoom factor:', e)
  }
  const factor = stored === null ? ZOOM_DEFAULT : Number.parseFloat(stored)
  return applyZoom(factor, false)
}
