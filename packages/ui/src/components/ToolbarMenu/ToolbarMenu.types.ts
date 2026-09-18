import type { Snippet } from 'svelte'
import type { ActionIconName } from '../Button/ActionIcon.types'
import type { MenuAlign } from './menu-placement'

/**
 * `action` is a plain command (role menuitem), `checkbox` a toggle that shows
 * its state (menuitemcheckbox), `radio` one choice of several
 * (menuitemradio).
 */
export type ToolbarMenuItemKind = 'action' | 'checkbox' | 'radio'

export interface ToolbarMenuEntry {
  kind?: ToolbarMenuItemKind
  id: string
  label: string
  icon?: ActionIconName
  /** For `checkbox` and `radio`. */
  checked?: boolean
  disabled?: boolean
  onselect: () => void
}

export interface ToolbarMenuSeparator {
  kind: 'separator'
  id: string
}

export type ToolbarMenuItem = ToolbarMenuEntry | ToolbarMenuSeparator

export type ToolbarMenuCloseReason = 'escape' | 'select' | 'outside' | 'blur' | 'toggle'

/**
 * What the trigger snippet spreads onto its button. `aria-pressed` is cleared
 * on purpose: a menu button announces itself as expanded or collapsed, and a
 * trigger that is also styled `active` (as IconButton maps it) would otherwise
 * be read as a toggle.
 */
export interface ToolbarMenuTriggerProps {
  'aria-haspopup': 'menu'
  'aria-expanded': 'true' | 'false'
  'aria-controls': string | undefined
  'aria-pressed': undefined
  onclick: (event: MouseEvent) => void
  onkeydown: (event: KeyboardEvent) => void
}

export interface ToolbarMenuContentApi {
  /** Closes the menu; focus goes back to the trigger unless told otherwise. */
  close: (options?: { returnFocus?: boolean }) => void
}

export interface ToolbarMenuProps {
  /** Accessible name of the menu. */
  label: string
  open?: boolean
  /** A list of commands. Rendered before `children` when both are given. */
  items?: readonly ToolbarMenuItem[]
  /**
   * Arbitrary content — a swatch grid, a set of options. Anything focusable
   * that carries a `menuitem`, `menuitemcheckbox` or `menuitemradio` role takes
   * part in the arrow-key navigation, and Enter or Space clicks it.
   */
  children?: Snippet<[ToolbarMenuContentApi]>
  /** Renders the trigger; spread the props onto the button. */
  trigger: Snippet<[ToolbarMenuTriggerProps, { open: boolean }]>
  /** Which trigger edge the menu lines up with before it flips. */
  align?: MenuAlign
  onclose?: (reason: ToolbarMenuCloseReason) => void
}
