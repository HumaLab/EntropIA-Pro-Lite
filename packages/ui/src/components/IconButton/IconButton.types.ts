import type { Snippet } from 'svelte'
import type { HTMLButtonAttributes } from 'svelte/elements'

/**
 * One container per step of ACTION_ICON_SIZES, paired 1:1:
 *   xs -> 24 (icon 12) · sm -> 28 (icon 14) · md -> 32 (icon 16) · lg -> 40 (icon 20)
 *
 * Pick the container from the icon, never the other way round. A view that
 * needs a size not on this list is asking for a new step in the scale, not for
 * a one-off class.
 */
export type IconButtonSize = 'xs' | 'sm' | 'md' | 'lg'
export type IconButtonVariant = 'ghost' | 'secondary' | 'primary' | 'danger'

export interface IconButtonProps extends HTMLButtonAttributes {
  variant?: IconButtonVariant
  size?: IconButtonSize
  label: string
  active?: boolean
  children?: Snippet
}
