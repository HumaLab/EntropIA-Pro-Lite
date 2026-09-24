/**
 * Startup preferences that used to be implicit in a component's `onMount`.
 *
 * Theme and contrast were applied by TopBar.svelte, and the typography preset
 * by TypographyMenu.svelte — meaning none of the three actually applied until
 * that component mounted. Now that all three live in the Apariencia settings
 * tab (not mounted at startup), the app needs its own place to restore them,
 * exactly as before: same storage keys, same defaults.
 *
 * Zoom (`zoom.ts`'s `initZoom`) and locale (`i18n.ts`'s `initLocale`) already
 * had their own startup paths, called from App.svelte; this module does not
 * duplicate them.
 */

import { restoreTheme } from './theme'
import { restoreContrast } from './contrast'
import { restoreFontPreset } from './typography'

export function initializeAppearance(): void {
  restoreTheme()
  restoreContrast()
  restoreFontPreset()
}
