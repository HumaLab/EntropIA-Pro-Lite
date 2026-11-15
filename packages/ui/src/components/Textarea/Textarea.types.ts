import type { HTMLTextareaAttributes } from 'svelte/elements'

/** Typeface family of the text the field holds. */
export type TextareaFont = 'ui' | 'reading' | 'mono'

export interface TextareaProps extends Omit<HTMLTextareaAttributes, 'value' | 'rows' | 'class'> {
  value?: string
  /** Visible lines; sets the field's minimum height. Defaults to 3. */
  rows?: number
  /** Grow with the content instead of scrolling. */
  autosize?: boolean
  /** With `autosize`, the line count after which the field scrolls. */
  maxRows?: number
  label?: string
  hint?: string
  error?: string
  /** `ui` (default) for interface text, `reading` for prose, `mono` for code and paths. */
  font?: TextareaFont
  disabled?: boolean
  /** Extra classes for the native textarea. */
  class?: string
  /** The native textarea, for `bind:element`. */
  element?: HTMLTextAreaElement
}
