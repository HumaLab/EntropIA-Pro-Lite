import type { Snippet } from 'svelte'

export type DisclosureVariant = 'plain' | 'card'
export type DisclosureSize = 'sm' | 'md'

export interface DisclosureProps {
  /** Whether the region is shown. Bindable, so callers can use `bind:open`. */
  open?: boolean
  /** Header label: plain text, or a snippet for richer content. */
  summary?: string | Snippet
  /** Trailing header content (a count, a status), aligned to the right. */
  meta?: Snippet
  /** The region the header shows and hides. */
  children?: Snippet
  /** `plain` sits in the flow; `card` draws a surface, a border and a radius. */
  variant?: DisclosureVariant
  /** Header height: `--control-height-sm` or `--control-height-md`. */
  size?: DisclosureSize
  disabled?: boolean
  /** Fires after the header flips the state, with the new value. */
  ontoggle?: (open: boolean) => void
  class?: string
}
