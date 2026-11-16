import type { HTMLAttributes } from 'svelte/elements'

export type ProgressBarTone = 'accent' | 'success' | 'warning' | 'danger'
export type ProgressBarSize = 'sm' | 'md'

export interface ProgressBarProps extends Omit<
  HTMLAttributes<HTMLDivElement>,
  'role' | 'aria-valuemin' | 'aria-valuemax' | 'aria-valuenow' | 'aria-valuetext' | 'aria-label'
> {
  /** Current progress, clamped to 0..max. Ignored while indeterminate. */
  value?: number
  /** Upper bound of `value`. Defaults to 100. */
  max?: number
  /** Unknown progress: no aria-valuenow, an animated slide instead of a fill. */
  indeterminate?: boolean
  /** Accessible name. Pass `aria-labelledby` instead when a visible label exists. */
  label?: string
  /** Human-readable value (e.g. "3 of 12 files"), read instead of the number. */
  valueText?: string
  tone?: ProgressBarTone
  /** Track thickness: sm 4px, md 8px (the `--progress-bar-height` property). */
  size?: ProgressBarSize
  class?: string
}
