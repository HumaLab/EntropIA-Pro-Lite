import type { Snippet } from 'svelte'

export interface CheckboxProps {
  /** Checked state. Bindable, so callers can use `bind:checked`. */
  checked?: boolean
  /** Label content. Rendered inside the same `<label>` as the control. */
  children?: Snippet
  disabled?: boolean
  /** Fires after the control settles on its new value. */
  onchange?: (checked: boolean) => void
  /** Accessible name when the row renders no visible label. */
  label?: string
  class?: string
}
