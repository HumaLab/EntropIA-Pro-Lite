import type { HTMLInputAttributes } from 'svelte/elements'

/** `md` is the field of a page or panel; `sm` sits in a toolbar beside `sm` buttons. */
export type SearchBarSize = 'sm' | 'md'

/** The input's combobox wiring, for a search that drives a list of results. */
export type SearchBarComboboxProps = Pick<
  HTMLInputAttributes,
  'role' | 'aria-expanded' | 'aria-controls' | 'aria-autocomplete' | 'aria-activedescendant'
>

export interface SearchBarProps extends SearchBarComboboxProps {
  value?: string
  size?: SearchBarSize
  id?: string
  disabled?: boolean
  /** Disables only the clear button (e.g. while a request the clear would race is in flight). */
  clearDisabled?: boolean
  autocomplete?: 'on' | 'off'
  spellcheck?: boolean
  placeholder?: string
  debounceMs?: number
  ariaLabel?: string
  clearAriaLabel?: string
  emitSearch?: boolean
  onvaluechange?: (query: string, event: Event) => void
  onsearch?: (query: string) => void
  onclear?: () => void
  oninput?: (event: Event) => void
  onfocus?: (event: FocusEvent) => void
  onblur?: (event: FocusEvent) => void
  onkeydown?: (event: KeyboardEvent) => void
  inputRef?: (element: HTMLInputElement | undefined) => void
}
