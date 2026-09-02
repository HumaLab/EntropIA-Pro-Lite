import type { Snippet } from 'svelte'
import type { HTMLAttributes } from 'svelte/elements'

export type StatusBadgeVariant =
  | 'neutral'
  | 'stored'
  | 'success'
  | 'warning'
  | 'danger'
  | 'info'
  | 'ai'
  | 'evidence'
export type StatusBadgeSize = 'sm' | 'md'

export interface StatusBadgeProps extends HTMLAttributes<HTMLSpanElement> {
  variant?: StatusBadgeVariant
  size?: StatusBadgeSize
  children?: Snippet
}
