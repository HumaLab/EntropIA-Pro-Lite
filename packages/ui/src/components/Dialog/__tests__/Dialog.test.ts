import { fireEvent, render, screen } from '@testing-library/svelte'
import { createRawSnippet, tick } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import Dialog from '../Dialog.svelte'

const snippet = (html: string) => createRawSnippet(() => ({ render: () => html }))

const overlay = () => document.querySelector('.dialog__overlay') as HTMLElement | null

describe('Dialog', () => {
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('renders nothing while closed', () => {
    render(Dialog, { props: { open: false, title: 'Settings' } })

    expect(screen.queryByRole('dialog')).toBeNull()
    expect(overlay()).toBeNull()
  })

  it('renders a modal dialog labelled by its title and described by its description', () => {
    render(Dialog, {
      props: { open: true, title: 'Rename', description: 'Pick a new name.' },
    })

    const dialog = screen.getByRole('dialog', { name: 'Rename' })
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog).toHaveAccessibleDescription('Pick a new name.')
  })

  it('has no description reference when no description is given', () => {
    render(Dialog, { props: { open: true, title: 'Rename' } })

    expect(screen.getByRole('dialog')).not.toHaveAttribute('aria-describedby')
  })

  it('renders its body and footer snippets', () => {
    render(Dialog, {
      props: {
        open: true,
        title: 'Rename',
        children: snippet('<p>Body text</p>'),
        footer: snippet('<button type="button">Save</button>'),
      },
    })

    expect(screen.getByText('Body text')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Save' }).closest('.dialog__footer')).not.toBeNull()
  })

  it('floats its overlay out of the pane it was opened from', () => {
    const pane = document.createElement('div')
    pane.className = 'work-pane'
    document.body.appendChild(pane)

    render(Dialog, { props: { open: true, title: 'Rename' }, target: pane })

    expect(overlay()).not.toBeNull()
    expect(pane.contains(overlay())).toBe(false)
    expect(overlay()?.parentElement).toBe(document.body)
  })

  it.each([
    ['sm', 'dialog--sm'],
    ['md', 'dialog--md'],
    ['lg', 'dialog--lg'],
  ] as const)('applies the %s width', (size, className) => {
    render(Dialog, { props: { open: true, title: 'Rename', size } })

    expect(screen.getByRole('dialog')).toHaveClass(className)
  })

  it('defaults to the small width', () => {
    render(Dialog, { props: { open: true, title: 'Rename' } })

    expect(screen.getByRole('dialog')).toHaveClass('dialog--sm')
  })

  it('passes extra classes to the panel and the overlay', () => {
    render(Dialog, {
      props: { open: true, title: 'Rename', class: 'my-panel', overlayClass: 'my-overlay' },
    })

    expect(screen.getByRole('dialog')).toHaveClass('my-panel')
    expect(overlay()).toHaveClass('my-overlay')
  })

  it('closes on Escape, reports it and stops the key from reaching the page', async () => {
    const onclose = vi.fn()
    const propagated = vi.fn()
    render(Dialog, { props: { open: true, title: 'Rename', onclose } })
    document.body.addEventListener('keydown', propagated)

    const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    screen.getByRole('dialog').dispatchEvent(event)
    await tick()

    expect(onclose).toHaveBeenCalledOnce()
    expect(event.defaultPrevented).toBe(true)
    expect(propagated).not.toHaveBeenCalled()
    expect(screen.queryByRole('dialog')).toBeNull()
    document.body.removeEventListener('keydown', propagated)
  })

  it('closes on a backdrop click but not on a click inside the panel', async () => {
    const onclose = vi.fn()
    render(Dialog, { props: { open: true, title: 'Rename', onclose } })

    await fireEvent.click(screen.getByRole('dialog'))
    expect(onclose).not.toHaveBeenCalled()

    await fireEvent.click(overlay() as HTMLElement)
    expect(onclose).toHaveBeenCalledOnce()
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('stays open on Escape and backdrop clicks when it is not dismissible', async () => {
    const onclose = vi.fn()
    render(Dialog, { props: { open: true, title: 'Rename', dismissible: false, onclose } })

    await fireEvent.keyDown(window, { key: 'Escape' })
    await fireEvent.click(overlay() as HTMLElement)

    expect(onclose).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog')).toBeInTheDocument()
  })

  it('can ignore backdrop clicks while still closing on Escape', async () => {
    const onclose = vi.fn()
    render(Dialog, {
      props: { open: true, title: 'Rename', dismissOnBackdrop: false, onclose },
    })

    await fireEvent.click(overlay() as HTMLElement)
    expect(onclose).not.toHaveBeenCalled()

    await fireEvent.keyDown(window, { key: 'Escape' })
    expect(onclose).toHaveBeenCalledOnce()
  })

  it('focuses the element marked for initial focus', async () => {
    render(Dialog, {
      props: {
        open: true,
        title: 'Rename',
        children: snippet('<input aria-label="Name" />'),
        footer: snippet('<button type="button" data-dialog-initial-focus>Cancel</button>'),
      },
    })
    await tick()

    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus()
  })

  it('focuses the first focusable element without a mark, and the panel without any', async () => {
    const { unmount } = render(Dialog, {
      props: { open: true, title: 'Rename', children: snippet('<input aria-label="Name" />') },
    })
    await tick()
    expect(screen.getByRole('textbox', { name: 'Name' })).toHaveFocus()
    unmount()

    render(Dialog, { props: { open: true, title: 'Notice' } })
    await tick()
    expect(screen.getByRole('dialog')).toHaveFocus()
  })

  it('traps Tab and Shift+Tab within the dialog', async () => {
    render(Dialog, {
      props: {
        open: true,
        title: 'Rename',
        children: snippet('<input aria-label="Name" />'),
        footer: snippet('<button type="button">Save</button>'),
      },
    })
    await tick()
    const input = screen.getByRole('textbox', { name: 'Name' })
    const save = screen.getByRole('button', { name: 'Save' })

    save.focus()
    await fireEvent.keyDown(save, { key: 'Tab' })
    expect(input).toHaveFocus()

    await fireEvent.keyDown(input, { key: 'Tab', shiftKey: true })
    expect(save).toHaveFocus()
  })

  it('returns focus to the element that had it when it closes', async () => {
    const trigger = document.createElement('button')
    document.body.appendChild(trigger)
    trigger.focus()

    const { rerender } = render(Dialog, { props: { open: true, title: 'Rename' } })
    await tick()
    expect(trigger).not.toHaveFocus()

    await rerender({ open: false })

    expect(screen.queryByRole('dialog')).toBeNull()
    expect(trigger).toHaveFocus()
  })

  it('layers a nested dialog above a regular one', () => {
    render(Dialog, { props: { open: true, title: 'Outer' } })
    render(Dialog, { props: { open: true, title: 'Inner', nested: true } })

    const overlays = document.querySelectorAll('.dialog__overlay')
    expect(overlays[0]).not.toHaveClass('dialog__overlay--nested')
    expect(overlays[1]).toHaveClass('dialog__overlay--nested')
  })

  it('lets only the topmost dialog answer Escape', async () => {
    const outerClose = vi.fn()
    const innerClose = vi.fn()
    render(Dialog, { props: { open: true, title: 'Outer', onclose: outerClose } })
    render(Dialog, { props: { open: true, title: 'Inner', nested: true, onclose: innerClose } })

    await fireEvent.keyDown(window, { key: 'Escape' })
    expect(innerClose).toHaveBeenCalledOnce()
    expect(outerClose).not.toHaveBeenCalled()

    await fireEvent.keyDown(window, { key: 'Escape' })
    expect(outerClose).toHaveBeenCalledOnce()
  })
})

describe('Dialog header and viewer size', () => {
  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('shows an eyebrow above the title, outside the accessible name', () => {
    render(Dialog, { props: { open: true, title: 'Carta', eyebrow: 'Original passage' } })

    const eyebrow = screen.getByText('Original passage')
    expect(eyebrow).toHaveClass('dialog__eyebrow')
    expect(screen.getByRole('dialog', { name: 'Carta' })).toBeInTheDocument()
    const title = screen.getByRole('heading', { name: 'Carta' })
    expect(eyebrow.compareDocumentPosition(title) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('renders no eyebrow when none is given', () => {
    render(Dialog, { props: { open: true, title: 'Carta' } })

    expect(document.querySelector('.dialog__eyebrow')).toBeNull()
  })

  it('places header actions in the header, beside the heading, not in the footer', () => {
    render(Dialog, {
      props: {
        open: true,
        title: 'Carta',
        headerActions: snippet('<button type="button">Close</button>'),
      },
    })

    const action = screen.getByRole('button', { name: 'Close' })
    expect(action.closest('.dialog__header-actions')?.closest('.dialog__header')).not.toBeNull()
    expect(action.closest('.dialog__footer')).toBeNull()
  })

  it('applies the viewer size', () => {
    render(Dialog, { props: { open: true, title: 'Carta', size: 'viewer' } })

    expect(screen.getByRole('dialog')).toHaveClass('dialog--viewer')
  })

  it('lets a header action take initial focus', async () => {
    render(Dialog, {
      props: {
        open: true,
        title: 'Carta',
        children: snippet('<button type="button">Inside</button>'),
        headerActions: snippet('<button type="button" data-dialog-initial-focus>Close</button>'),
      },
    })
    await tick()

    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Close' }))
  })
})
