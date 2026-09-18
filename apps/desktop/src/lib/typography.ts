/**
 * The typography preset, the third axis beside theme and contrast.
 *
 * # Where the fonts are decided
 *
 * Not here. Each preset is a block in `packages/ui/src/tokens/tokens.css` that
 * sets three semantic tokens — `--font-ui`, `--font-reading`, `--font-mono` —
 * and components only ever name those tokens. Switching presets is therefore
 * one attribute on the root element, and everything restyles at once.
 *
 * The family names below exist to describe a preset to the person choosing it.
 * `typography-tokens.test.ts` reads both files and fails if they disagree, and
 * checks that `fonts.css` actually ships every family named.
 *
 * # Adding a preset
 *
 * Install the Fontsource packages, add their faces to `fonts.css`, add a block
 * to `tokens.css`, add an entry here and its label in `i18n.ts`. The selector
 * is generated from this list.
 */

import type { I18nKey } from './i18n'

export type FontPresetId = 'academic' | 'modern' | 'editorial' | 'archive'

export interface FontPreset {
  id: FontPresetId
  label: I18nKey
  /** The first family of each token's stack, as `@font-face` declares it. */
  families: { ui: string; reading: string; mono: string }
}

export const FONT_PRESETS: readonly FontPreset[] = [
  {
    id: 'academic',
    label: 'typography.academic',
    families: {
      ui: 'Source Sans 3 Variable',
      reading: 'Source Serif 4 Variable',
      mono: 'JetBrains Mono Variable',
    },
  },
  {
    id: 'modern',
    label: 'typography.modern',
    families: {
      ui: 'Inter Variable',
      reading: 'Literata Variable',
      mono: 'IBM Plex Mono',
    },
  },
  {
    id: 'editorial',
    label: 'typography.editorial',
    families: {
      ui: 'IBM Plex Sans Variable',
      reading: 'Lora Variable',
      mono: 'IBM Plex Mono',
    },
  },
  {
    id: 'archive',
    label: 'typography.archive',
    families: {
      ui: 'Atkinson Hyperlegible Next Variable',
      reading: 'Noto Serif Variable',
      mono: 'Source Code Pro Variable',
    },
  },
]

export const FONT_PRESET_DEFAULT: FontPresetId = 'academic'

export const FONT_STORAGE_KEY = 'entropia-font'

/**
 * A stored value read back, or the default.
 *
 * Anything may be in `localStorage` — an older build, a hand edit. An
 * unrecognised value means the default rather than an attribute no stylesheet
 * answers.
 */
export function readFontPreset(stored: string | null): FontPresetId {
  const match = FONT_PRESETS.find((preset) => preset.id === stored)
  return match ? match.id : FONT_PRESET_DEFAULT
}

/**
 * Puts a preset on the root element and remembers it.
 *
 * Unlike `data-contrast`, the default is written out: `data-font` is always
 * present, so what is on screen can always be read off the element.
 */
export function applyFontPreset(id: FontPresetId): void {
  document.documentElement.dataset.font = id
  try {
    localStorage.setItem(FONT_STORAGE_KEY, id)
  } catch {
    // Storage can be unavailable outright. The preset still applies for this
    // session; it just will not be remembered.
  }
}

/** Applies the stored preset, or the default, and returns which one it was. */
export function restoreFontPreset(): FontPresetId {
  let stored: string | null = null
  try {
    stored = localStorage.getItem(FONT_STORAGE_KEY)
  } catch {
    // Unavailable storage reads as nothing stored.
  }
  const id = readFontPreset(stored)
  applyFontPreset(id)
  return id
}
