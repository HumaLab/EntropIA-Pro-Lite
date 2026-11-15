import type { Snippet } from 'svelte'
import type { HTMLButtonAttributes } from 'svelte/elements'

/** `link` is text-only: accent text, underline on hover and focus, sized to its text. */
export type ButtonVariant =
  'primary' | 'secondary' | 'outline' | 'ghost' | 'subtle' | 'danger' | 'link'
export type ButtonSize = 'sm' | 'md' | 'lg'

export interface ButtonProps extends HTMLButtonAttributes {
  variant?: ButtonVariant
  size?: ButtonSize
  iconOnly?: boolean
  disabled?: boolean
  loading?: boolean
  type?: 'button' | 'submit' | 'reset'
  children?: Snippet
}
