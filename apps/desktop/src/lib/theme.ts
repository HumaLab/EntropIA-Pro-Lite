/**
 * The theme, beside contrast and typography as the third startup preference.
 *
 * Previously lived inside TopBar.svelte, which meant the theme only applied
 * once the top bar mounted. It is now a `$lib` module like `contrast.ts` and
 * `typography.ts`, applied once at app startup by `appearance.ts` and
 * consumed by both the Apariencia settings tab and this module's own tests.
 */

export type AppTheme = 'dark' | 'dim' | 'light' | 'lite'

export const THEME_STORAGE_KEY = 'entropia-theme'

// Dark first because it is the default, then the two warm/pale steps, then
// Lite last: it is the quiet one, and someone cycling past it lands back on
// the default rather than on another pale theme.
export const THEME_CYCLE: AppTheme[] = ['dark', 'dim', 'light', 'lite']

export const THEME_DEFAULT: AppTheme = 'dark'

export const themeLabels: Record<AppTheme, string> = {
  dark: 'Oscuro',
  dim: 'Cálido',
  light: 'Claro',
  lite: 'Lite',
}

/**
 * The stored theme, checked against the cycle rather than against a list
 * written out again here.
 *
 * The two used to be separate, and adding a theme to the cycle left this one
 * behind: the theme could be reached by pressing the button and was forgotten
 * on the next start, which reads as the setting not saving.
 */
export function readPersistedTheme(): AppTheme {
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY)
    return THEME_CYCLE.includes(stored as AppTheme) ? (stored as AppTheme) : THEME_DEFAULT
  } catch {
    return THEME_DEFAULT
  }
}

/** The next theme in the cycle, wrapping at the top. */
export function nextTheme(current: AppTheme): AppTheme {
  const idx = THEME_CYCLE.indexOf(current)
  return THEME_CYCLE[(idx + 1) % THEME_CYCLE.length] ?? THEME_DEFAULT
}

/**
 * Puts a theme on the root element and remembers it.
 *
 * The default theme removes the attribute rather than setting it: `tokens.css`
 * declares the default as the bare `:root`, so `data-theme="dark"` would match
 * nothing extra and would quietly become a lie the day the default changes.
 */
export function applyTheme(theme: AppTheme): void {
  if (theme === THEME_DEFAULT) {
    delete document.documentElement.dataset.theme
  } else {
    document.documentElement.dataset.theme = theme
  }

  try {
    localStorage.setItem(THEME_STORAGE_KEY, theme)
  } catch {
    // Storage can be unavailable outright. The theme still applies for this
    // session; it just will not be remembered.
  }
}

/** Applies the stored theme, or the default, and returns which one it was. */
export function restoreTheme(): AppTheme {
  const theme = readPersistedTheme()
  applyTheme(theme)
  return theme
}
