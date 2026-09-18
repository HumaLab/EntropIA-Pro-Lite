/**
 * Design token constants for programmatic access.
 * These mirror the CSS Custom Properties defined in tokens.css.
 */

export const colors = {
  bg: 'var(--color-bg)',
  surface: 'var(--color-surface)',
  surfaceRaised: 'var(--color-surface-raised)',
  border: 'var(--color-border)',
  textPrimary: 'var(--color-text-primary)',
  textSecondary: 'var(--color-text-secondary)',
  textMuted: 'var(--color-text-muted)',
  accent: 'var(--color-accent)',
  accentHover: 'var(--color-accent-hover)',
  danger: 'var(--color-danger)',
  success: 'var(--color-success)',
  warning: 'var(--color-warning)',
} as const

export const spacing = {
  1: 'var(--space-1)',
  2: 'var(--space-2)',
  3: 'var(--space-3)',
  4: 'var(--space-4)',
  5: 'var(--space-5)',
  6: 'var(--space-6)',
  7: 'var(--space-7)',
  8: 'var(--space-8)',
} as const

export const typography = {
  fontSans: 'var(--font-ui)',
  fontMono: 'var(--font-mono)',
  sizeXs: 'var(--font-size-xs)',
  sizeSm: 'var(--font-size-sm)',
  sizeMd: 'var(--font-size-md)',
  sizeLg: 'var(--font-size-lg)',
  sizeXl: 'var(--font-size-xl)',
  weightNormal: 'var(--font-weight-normal)',
  weightMedium: 'var(--font-weight-medium)',
  weightBold: 'var(--font-weight-bold)',
} as const

export const radius = {
  sm: 'var(--radius-sm)',
  md: 'var(--radius-md)',
  lg: 'var(--radius-lg)',
  full: 'var(--radius-full)',
} as const

export const shadows = {
  sm: 'var(--shadow-sm)',
  md: 'var(--shadow-md)',
  lg: 'var(--shadow-lg)',
} as const
