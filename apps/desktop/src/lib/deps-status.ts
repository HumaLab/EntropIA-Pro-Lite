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

/** Colour reinforces the glyph above; it never carries the meaning alone. */
export function dependencyStatusColor(status: DependencyStatus): string {
  switch (status.type) {
    case 'installed':
      return 'var(--color-success)'
    case 'missing':
      return 'var(--color-danger)'
    case 'failed':
      return 'var(--color-warning)'
    default:
      return 'var(--color-text-muted)'
  }
}
