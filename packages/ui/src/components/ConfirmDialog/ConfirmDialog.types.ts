import type { Snippet } from 'svelte'
import type { ActionIconName } from '../Button'

export type ConfirmDialogVariant = 'default' | 'destructive'

/**
 * `compact` (default) is the 440 px confirmation box. `wide` is for dialogs
 * that browse lists (e.g. an import catalog): it widens the box and caps its
 * height to the viewport so the body can scroll instead of the page.
 */
export type ConfirmDialogSize = 'compact' | 'wide'

type ConfirmDialogBaseProps = {
  title: string
  message?: string
  titleId?: string
  error?: string | null
  cancelLabel: string
  variant?: ConfirmDialogVariant
  size?: ConfirmDialogSize
  confirming?: boolean
  confirmDisabled?: boolean
  cancelDisabled?: boolean
  /**
   * Whether a click on the overlay cancels. Defaults to true. Turn it off when
   * the dialog holds something a stray click must not throw away (typed input,
   * a result the user has not read yet). Escape and the buttons still cancel.
   */
  dismissOnOverlay?: boolean
  confirmFirst?: boolean
  confirmTitle?: string
  oncancel: () => void
  /** Required whenever a confirm action is rendered; omit it with the label. */
  onconfirm?: () => void
  children?: Snippet
  errorContent?: Snippet
}

/**
 * The confirm action is optional: a dialog with neither `confirmLabel` nor
 * `confirmIcon` renders the cancel/close action alone (informational phases).
 */
export type ConfirmDialogProps = ConfirmDialogBaseProps &
  (
    | {
        confirmLabel?: string
        confirmIcon?: never
        confirmAriaLabel?: string
      }
    | {
        confirmLabel?: never
        confirmIcon: ActionIconName
        confirmAriaLabel: string
      }
  )
