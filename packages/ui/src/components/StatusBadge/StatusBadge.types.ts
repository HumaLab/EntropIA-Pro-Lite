import type { Snippet } from 'svelte'
import type { HTMLAttributes } from 'svelte/elements'

export type StatusBadgeTone =
  'neutral' | 'stored' | 'success' | 'warning' | 'danger' | 'info' | 'ai' | 'evidence'
/** A tone, or `count`: a numeric pill whose colour comes from `tone`. */
export type StatusBadgeVariant = StatusBadgeTone | 'count'
export type StatusBadgeSize = 'xs' | 'sm' | 'md'

export interface StatusBadgeProps extends HTMLAttributes<HTMLSpanElement> {
  variant?: StatusBadgeVariant
  size?: StatusBadgeSize
  /** Colour of a `count` badge; `danger` (solid fill) by default. Ignored otherwise. */
  tone?: StatusBadgeTone
  /** Number a `count` badge shows instead of `children`. */
  count?: number
  /** Above this, a count shows as `max+` (e.g. "99+"). */
  max?: number
  children?: Snippet
}
