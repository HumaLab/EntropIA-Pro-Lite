import type { ActionIconName } from '@entropia/ui'
import type { DependencyStatus } from './deps'

/**
 * Presentation mapping for a dependency's status.
 *
 * These used to be typographic characters returned as strings ('✓', '✗', '⏳',
 * '⚠', '?'). A character is not an icon: it inherits the font's metrics, so it
 * neither scales nor aligns with the rest of the icon set, and it renders
 * differently per platform. They live here rather than inside the component so
 * the mapping is testable on its own.
 *
 * Shape carries the meaning and colour reinforces it — never the other way
 * round. Installed, missing and failed are three different glyphs so the state
 * survives a colour-blind reader and a greyscale screenshot.
 */
export function dependencyStatusIcon(status: DependencyStatus): ActionIconName {
  switch (status.type) {
    case 'installed':
      return 'circle-check'
    case 'missing':
      return 'circle-x'
    case 'failed':
      return 'triangle-alert'
    case 'checking':
    case 'installing':
      return 'loader'
    case 'unknown':
    default:
      return 'circle-help'
  }
}

/**
 * Modifier class for the status icon. Colour reinforces the glyph above; it
 * never carries the meaning alone. The class, not an inline style, picks the
 * token, so the stylesheet owns every colour: installed → success, missing →
 * danger, failed → warning, anything else → muted.
 */
export type DependencyStatusIconClass =
  | 'deps-row__icon--installed'
  | 'deps-row__icon--missing'
  | 'deps-row__icon--failed'
  | 'deps-row__icon--neutral'

export function dependencyStatusIconClass(status: DependencyStatus): DependencyStatusIconClass {
  switch (status.type) {
    case 'installed':
      return 'deps-row__icon--installed'
    case 'missing':
      return 'deps-row__icon--missing'
    case 'failed':
      return 'deps-row__icon--failed'
    default:
      return 'deps-row__icon--neutral'
  }
}
