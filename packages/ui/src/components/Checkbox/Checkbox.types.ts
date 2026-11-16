import type { Snippet } from 'svelte'

/** md is the default row; sm is the compact one (tighter padding, xs type). */
export type CheckboxSize = 'sm' | 'md'
/** row is a plain row; card is a bordered surface that lifts off a sunken list. */
export type CheckboxVariant = 'row' | 'card'

export interface CheckboxProps {
  /** Checked state. Bindable, so callers can use `bind:checked`. */
  checked?: boolean
  /** Label content. Rendered inside the same `<label>` as the control. */
  children?: Snippet
  disabled?: boolean
  /** Mixed state (some, not all, of a group): drawn as a dash, read as mixed. */
  indeterminate?: boolean
  /** Fires after the control settles on its new value. */
  onchange?: (checked: boolean) => void
  /** Accessible name when the row renders no visible label. */
  label?: string
  size?: CheckboxSize
  variant?: CheckboxVariant
  class?: string
}
