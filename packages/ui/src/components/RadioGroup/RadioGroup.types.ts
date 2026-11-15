import type { Snippet } from 'svelte'

/**
 * `card`: bordered cells in as many columns as fit (preset grids, choice
 * grids). `list`: borderless rows stacked one under another (destination
 * lists in dialogs).
 */
export type RadioGroupVariant = 'card' | 'list'

export interface RadioGroupProps {
  /** Shared `name` of the native radios. Defaults to a unique id. */
  name?: string
  /** The chosen option's value. Bindable, so callers can use `bind:value`. */
  value?: string
  variant?: RadioGroupVariant
  /** The group's name, rendered as the fieldset's legend. */
  label: string
  /** Keep the legend for assistive technology only, when a heading already names the group on screen. */
  hideLabel?: boolean
  /** Disables every option, embedded controls included (native fieldset). */
  disabled?: boolean
  /**
   * Narrowest column of the `card` grid, as a CSS length (default 168px). The
   * grid never overflows its box: below this width it falls to one column.
   */
  columnMin?: string
  /** Fires after the selection moves to a new option. */
  onchange?: (value: string) => void
  /** The `RadioCard` options. */
  children: Snippet
  class?: string
}

export interface RadioCardProps {
  value: string
  /** The option's accessible name, shown at its head. */
  label: string
  /** Secondary line, linked to the radio by `aria-describedby`. */
  description?: string | Snippet
  disabled?: boolean
  /**
   * Extra visual content under the head (a preview, a sample). It is not
   * part of the accessible name or description; mark it `aria-hidden` when
   * it only repeats what the label says.
   */
  children?: Snippet
  /**
   * A control shown inside the option, such as the name field of a “new
   * collection” option. Focusing it selects the option. It sits outside the
   * `<label>`, so its value never becomes part of the radio's name.
   */
  embedded?: Snippet
}
