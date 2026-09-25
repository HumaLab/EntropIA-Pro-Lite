import { fireEvent, render, screen } from '@testing-library/svelte'
import { tick } from 'svelte'
import { describe, expect, it, vi } from 'vitest'
import ConfirmDialog from '../ConfirmDialog.svelte'

describe('ConfirmDialog', () => {
  const baseProps = {
    title: 'Delete item',
    message: 'This cannot be undone.',
    cancelLabel: 'Cancel',
    confirmLabel: 'Delete',
    oncancel: vi.fn(),
    onconfirm: vi.fn(),
  }

  it('renders an accessible modal dialog labelled by the title', () => {
    render(ConfirmDialog, { props: baseProps })

    const dialog = screen.getByRole('dialog', { name: 'Delete item' })
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(screen.getByText('This cannot be undone.')).toBeInTheDocument()
  })

  // A `.work-pane` is a CSS size container, which makes it the containing block
  // of every `position: fixed` descendant: an overlay left inside it would be
  // clipped to the pane instead of covering the window.
  it('floats its overlay out of the pane it was opened from', () => {
    const pane = document.createElement('div')
    pane.className = 'work-pane'
    document.body.appendChild(pane)

    render(ConfirmDialog, { props: baseProps, target: pane })

    const overlay = document.querySelector('.confirm-dialog__overlay')
    expect(overlay).not.toBeNull()
    expect(pane.contains(overlay)).toBe(false)
    expect(overlay?.parentElement).toBe(document.body)
    pane.remove()
  })

  it('cancels when the overlay is clicked', async () => {
    const oncancel = vi.fn()
    render(ConfirmDialog, { props: { ...baseProps, oncancel } })

    await fireEvent.click(document.querySelector('.confirm-dialog__overlay') as Element)

    expect(oncancel).toHaveBeenCalledOnce()
  })

  it('ignores overlay clicks when dismissOnOverlay is false, but still cancels on Escape', async () => {
    const oncancel = vi.fn()
    render(ConfirmDialog, {
      props: { ...baseProps, oncancel, dismissOnOverlay: false },
    })

    await fireEvent.click(document.querySelector('.confirm-dialog__overlay') as Element)
    expect(oncancel).not.toHaveBeenCalled()

    await fireEvent.keyDown(window, { key: 'Escape' })
    expect(oncancel).toHaveBeenCalledOnce()
  })

  it('cancels Escape and stops propagation', async () => {
    const oncancel = vi.fn()
    const propagated = vi.fn()
    const dialogKeydown = new KeyboardEvent('keydown', {
      key: 'Escape',
      bubbles: true,
      cancelable: true,
    })
    const dialog = render(ConfirmDialog, { props: { ...baseProps, oncancel } }).getByRole('dialog')
    document.body.addEventListener('keydown', propagated)

    dialog.dispatchEvent(dialogKeydown)

    expect(oncancel).toHaveBeenCalledOnce()
    expect(propagated).not.toHaveBeenCalled()
    document.body.removeEventListener('keydown', propagated)
  })

  it('cancels on Escape even when focus stays outside the dialog', async () => {
    const oncancel = vi.fn()
    render(ConfirmDialog, { props: { ...baseProps, oncancel } })

    await fireEvent.keyDown(document.body, { key: 'Escape' })

    expect(oncancel).toHaveBeenCalledOnce()
  })

  it('moves focus to the cancel button when opened', async () => {
    render(ConfirmDialog, { props: baseProps })
    await tick()

    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus()
  })

  it('traps Tab and Shift+Tab within the dialog', async () => {
    render(ConfirmDialog, { props: baseProps })
    await tick()

    const cancelButton = screen.getByRole('button', { name: 'Cancel' })
    const confirmButton = screen.getByRole('button', { name: 'Delete' })

    confirmButton.focus()
    await fireEvent.keyDown(confirmButton, { key: 'Tab' })
    expect(cancelButton).toHaveFocus()

    await fireEvent.keyDown(cancelButton, { key: 'Tab', shiftKey: true })
    expect(confirmButton).toHaveFocus()
  })

  it('restores focus to the previously focused element on close', async () => {
    const trigger = document.createElement('button')
    document.body.appendChild(trigger)
    trigger.focus()

    const { unmount } = render(ConfirmDialog, { props: baseProps })
    await tick()
    expect(trigger).not.toHaveFocus()

    unmount()

    expect(trigger).toHaveFocus()
    trigger.remove()
  })

  it('supports destructive icon-only confirmation', () => {
    render(ConfirmDialog, {
      props: {
        title: 'Delete asset',
        message: 'Delete file.pdf?',
        cancelLabel: 'Cancel',
        confirmIcon: 'delete',
        confirmAriaLabel: 'Delete asset',
        variant: 'destructive',
        oncancel: vi.fn(),
        onconfirm: vi.fn(),
      },
    })

    expect(screen.getByRole('button', { name: 'Delete asset' })).toHaveClass(
      'confirm-dialog__confirm-icon--destructive'
    )
  })

  it('displays error text as an alert', () => {
    render(ConfirmDialog, { props: { ...baseProps, error: 'Delete failed' } })

    expect(screen.getByRole('alert')).toHaveTextContent('Delete failed')
  })
})
