/**
 * The contrast level, beside the theme (plan-editor.md §18).
 *
 * # Why a level and not a slider
 *
 * Three named steps are a decision someone can make and undo. A continuous
 * value leaves people at settings nobody chose and nobody can describe, and
 * every one of them would have to be proven readable — which for a control over
 * text is not a detail: `tokens.css` holds three measured sets, and
 * `contrast-floor.test.ts` measures every one of them back out of the file.
 *
 * # Why the palette is not here
 *
 * This decides *which* level is on. What each level looks like belongs to the
 * tokens, per theme, because the themes do not have equal room: primary text
 * sits at 16:1 on the dark theme and muted text is already at the AA floor.
 */

export type ContrastLevel = 'soft' | 'normal' | 'high'

/** In the order the control walks them, so forward is always more contrast. */
export const CONTRAST_CYCLE: ContrastLevel[] = ['soft', 'normal', 'high']

export const CONTRAST_DEFAULT: ContrastLevel = 'normal'

export const CONTRAST_STORAGE_KEY = 'entropia-contrast'

/**
 * The next level, wrapping at the top.
 *
 * Wrapping rather than stopping, because a cycling button that silently does
 * nothing at one end is a button people press twice and then distrust.
 */
export function nextContrast(current: ContrastLevel): ContrastLevel {
  const at = CONTRAST_CYCLE.indexOf(current)
  return CONTRAST_CYCLE[(at + 1) % CONTRAST_CYCLE.length] ?? CONTRAST_DEFAULT
}

/**
 * A stored value read back, or the default.
 *
 * `localStorage` holds strings that anything may have written — an older build,
 * a hand edit, a half-finished migration. An unrecognised one means the default
 * rather than an attribute nothing in the stylesheet answers, which would leave
 * the application looking like the theme had failed to load.
 */
export function readContrast(stored: string | null): ContrastLevel {
  return CONTRAST_CYCLE.includes(stored as ContrastLevel)
    ? (stored as ContrastLevel)
    : CONTRAST_DEFAULT
}

/**
 * The value for the root element's `data-contrast`, or null to remove it.
 *
 * The default level has no block in `tokens.css` — it is what each theme
 * already declares — so it is expressed by the attribute's absence. That is
 * also what keeps a theme added later working before anyone tunes it.
 */
export function contrastAttribute(level: ContrastLevel): string | null {
  return level === CONTRAST_DEFAULT ? null : level
}
