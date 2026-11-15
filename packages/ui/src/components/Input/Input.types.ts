import type { Snippet } from 'svelte'
import type { HTMLInputAttributes } from 'svelte/elements'

/** A search box is never an Input: search fields are always SearchBar. */
export type InputType = 'text' | 'password' | 'email' | 'url' | 'tel' | 'number'
export type InputSize = 'sm' | 'md'

export interface InputProps extends Omit<
  HTMLInputAttributes,
  'type' | 'value' | 'size' | 'class' | 'children'
> {
  value?: string
  type?: InputType
  placeholder?: string
  disabled?: boolean
  label?: string
  error?: string
  hint?: string
  /** Merged with the component's own class on the native input. */
  class?: string
  /** Control height: `md` (default) or `sm`, both capped at their token. */
  size?: InputSize
  /** The native input, for `bind:element` (focus, selection, measuring). */
  element?: HTMLInputElement | null | undefined
  /** Icon-sized content on the input's leading edge (a magnifier, a unit). */
  leading?: Snippet
  /** Icon-sized content on the input's trailing edge (a clear or reveal button). */
  trailing?: Snippet
}
