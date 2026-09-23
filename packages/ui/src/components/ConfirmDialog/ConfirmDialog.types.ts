import type { Snippet } from 'svelte'
import type { ActionIconName } from '../Button'

export type ConfirmDialogVariant = 'default' | 'destructive'

type ConfirmDialogBaseProps = {
  title: string
  message?: string
  titleId?: string
  error?: string | null
  cancelLabel: string
  variant?: ConfirmDialogVariant
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
  onconfirm: () => void
  children?: Snippet
  errorContent?: Snippet
}

export type ConfirmDialogProps = ConfirmDialogBaseProps &
  (
    | {
        confirmLabel: string
        confirmIcon?: never
        confirmAriaLabel?: string
      }
    | {
        confirmLabel?: never
        confirmIcon: ActionIconName
        confirmAriaLabel: string
      }
  )
