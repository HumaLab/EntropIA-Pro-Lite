import type { HTMLButtonAttributes } from 'svelte/elements'
import type { MenuAlign } from '../ToolbarMenu/menu-placement'

export type SelectTriggerSize = 'sm' | 'md'

/** `field` is bordered like Input; `quiet` drops the border for toolbars and headers. */
export type SelectTriggerVariant = 'field' | 'quiet'

export interface SelectTriggerProps extends Omit<
  HTMLButtonAttributes,
  'type' | 'aria-haspopup' | 'aria-expanded' | 'children'
> {
  /** The chosen option's text. */
  label?: string
  /** Shown, muted, while `label` is empty. */
  placeholder?: string
  size?: SelectTriggerSize
  variant?: SelectTriggerVariant
  /** Spans its container's width, like an Input in a form column. */
  block?: boolean
  /**
   * Id of the element showing the chosen value, so `aria-labelledby` can name
   * the button by its visible label followed by the value.
   */
  valueId?: string
  /** Whether the menu it controls is open; drives `aria-expanded`. */
  open?: boolean
  /** The rendered button, for callers that need to focus it. */
  element?: HTMLButtonElement
}

export interface SelectOption {
  value: string
  label: string
  disabled?: boolean
}

export interface SelectProps extends Omit<
  SelectTriggerProps,
  'label' | 'open' | 'element' | 'onchange'
> {
  /** Accessible name of the menu. */
  label: string
  value?: string
  options: readonly SelectOption[]
  /** Which trigger edge the menu lines up with before it flips. */
  align?: MenuAlign
  /** Called with the newly chosen value; not called when the same one is picked again. */
  onchange?: (value: string) => void
}
