import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'

export type RgbColor = [number, number, number]

/**
 * Reads the `rgb()` / `rgba()` form `getComputedStyle` returns. A transparent
 * or unreadable colour gives `null`: handing it to the window would leave the
 * default (white) background in place anyway.
 */
export function parseCssRgb(value: string): RgbColor | null {
  const match = value.match(/^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*([\d.]+)\s*)?\)$/)
  if (!match) return null
  if (match[4] !== undefined && Number(match[4]) === 0) return null
  return [Number(match[1]), Number(match[2]), Number(match[3])]
}

/**
 * Paints the native window with the active theme's background before it is
 * shown.
 *
 * WebKitGTK (Linux) runs no frames for a hidden window, so the main window is
 * revealed before its first paint (see first-paint.ts). Until that frame, what
 * shows is the webview's own background — white, a flash on a dark theme.
 * Reading the colour from the page keeps it right for every theme.
 *
 * Best effort: a refused or failed call must never hold up the reveal.
 */
export async function matchWindowBackground(
  setBackground: (color: RgbColor) => Promise<void> = (color) =>
    getCurrentWebviewWindow().setBackgroundColor(color)
): Promise<void> {
  const color = parseCssRgb(getComputedStyle(document.documentElement).backgroundColor)
  if (!color) return
  try {
    await setBackground(color)
  } catch (error) {
    console.warn('[window-background] could not match the window background:', error)
  }
}
