import type { HTMLInputAttributes } from 'svelte/elements'

export type SwitchSize = 'sm' | 'md'

export interface SwitchProps extends Omit<
  HTMLInputAttributes,
  'type' | 'role' | 'checked' | 'disabled' | 'size' | 'onchange' | 'class' | 'aria-label'
> {
  /** On state. Bindable, so callers can use `bind:checked`. */
  checked?: boolean
  /** Visible label, rendered next to the control and naming it. */
  label?: string
  /** Accessible name when the switch renders no visible label. */
  ariaLabel?: string
  /** Secondary text under the label, read as the control's description. */
  description?: string
  disabled?: boolean
  /** Track and thumb size: sm 28x16, md 36x20 (the `--switch-*` properties). */
  size?: SwitchSize
  /** The native `<input>`, for focus management. Bindable via `bind:element`. */
  element?: HTMLInputElement
  /** Fires after the control settles on its new value. */
  onchange?: (checked: boolean) => void
  class?: string
}
