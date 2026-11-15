import type { Snippet } from 'svelte'
import type { ClassValue } from 'svelte/elements'

/**
 * Panel width: `sm` is the 440 px confirmation box, `md` a 640 px form, `lg`
 * a 1120 px browser of lists. `lg` also caps the panel to the viewport so its
 * body scrolls instead of the page.
 */
export type DialogSize = 'sm' | 'md' | 'lg'

export type DialogProps = {
  /** Whether the dialog is shown. Escape and backdrop clicks set it to false. */
  open?: boolean
  /** Visible heading; names the dialog through `aria-labelledby`. */
  title: string
  /** Id of the heading. Generated when omitted. */
  titleId?: string
  /** Short text under the heading; describes the dialog through `aria-describedby`. */
  description?: string
  size?: DialogSize
  /**
   * Whether Escape and backdrop clicks close the dialog. Defaults to true. A
   * dialog that is not dismissible still swallows Escape, so the key never
   * reaches the page behind it.
   */
  dismissible?: boolean
  /**
   * Whether a backdrop click closes a dismissible dialog. Defaults to true.
   * Turn it off when a stray click must not throw away what the dialog holds;
   * Escape still closes it.
   */
  dismissOnBackdrop?: boolean
  /**
   * Opened over another dialog: it sits on the `--z-dialog-nested` layer
   * instead of `--z-dialog`. Only the topmost open dialog answers the keyboard.
   */
  nested?: boolean
  /** Extra classes for the panel (the element with `role="dialog"`). */
  class?: ClassValue
  /** Extra classes for the backdrop. */
  overlayClass?: ClassValue
  /** Called after Escape or a backdrop click closed the dialog. */
  onclose?: () => void
  children?: Snippet
  /**
   * Actions row. The element carrying `data-dialog-initial-focus` gets focus
   * when the dialog opens; without one, the first focusable element does.
   */
  footer?: Snippet
}
