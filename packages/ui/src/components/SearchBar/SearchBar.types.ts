export interface SearchBarProps {
  value?: string
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
