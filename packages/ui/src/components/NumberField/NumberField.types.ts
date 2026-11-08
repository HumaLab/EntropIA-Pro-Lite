import type { HTMLInputAttributes } from 'svelte/elements'

export interface NumberFieldProps extends Omit<
  HTMLInputAttributes,
  'type' | 'value' | 'min' | 'max' | 'step' | 'size' | 'class'
> {
  /**
   * Current value. Bindable; `null` while the field is empty. A string state is
   * accepted as the native input accepts it; the field writes back numbers.
   */
  value?: number | string | null
  min?: number | string
  max?: number | string
  /** Amount one button press moves the value. Defaults to 1. */
  step?: number | string
  disabled?: boolean
  /** Accessible name of the − button, in the caller's language. */
  decrementLabel: string
  /** Accessible name of the + button, in the caller's language. */
  incrementLabel: string
  /** Accessible name of the field when no visible `<label>` names it. */
  label?: string
  /** `sm` for dense forms; `md` matches the other inputs. */
  size?: 'sm' | 'md'
  /** Layout class for the whole control (width, grid placement). */
  class?: string
}
