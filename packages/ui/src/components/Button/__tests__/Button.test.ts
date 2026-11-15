import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import ButtonTestHost from './ButtonTestHost.svelte'

describe('Button', () => {
  it('marks a button whose label may wrap', () => {
    render(ButtonTestHost, { props: { wrap: true, label: 'Find evidence against' } })

    expect(screen.getByRole('button', { name: 'Find evidence against' })).toHaveClass('btn--wrap')
  })

  it('keeps a label on one line by default', () => {
    render(ButtonTestHost, { props: { label: 'Save' } })

    expect(screen.getByRole('button', { name: 'Save' })).not.toHaveClass('btn--wrap')
  })

  it('keeps its own classes when the caller adds one', () => {
    render(ButtonTestHost, {
      props: {
        label: 'Delete',
        class: 'item-card__delete',
      },
    })

    const button = screen.getByRole('button', { name: 'Delete' })
    expect(button.className).toContain('item-card__delete')
    expect(button.className).toContain('btn')
    expect(button.className).toContain('btn--primary')
  })

  it('renders icon-only buttons with an accessible name', () => {
    render(ButtonTestHost, {
      props: {
        iconOnly: true,
        label: 'Edit collection',
      },
    })

    const button = screen.getByRole('button', { name: 'Edit collection' })
    expect(button).toBeInTheDocument()
    expect(button.className).toContain('btn--icon-only')
  })

  it('keeps icon-only controls square across sizes', () => {
    render(ButtonTestHost, {
      props: {
        iconOnly: true,
        size: 'sm',
        label: 'Delete collection',
      },
    })

    expect(screen.getByRole('button', { name: 'Delete collection' }).className).toContain('btn--sm')
  })
})

describe('Button variant="link"', () => {
  it('renders a native button with the link variant class', () => {
    render(ButtonTestHost, { props: { variant: 'link', label: 'Open settings' } })

    const button = screen.getByRole('button', { name: 'Open settings' })
    expect(button.tagName).toBe('BUTTON')
    expect(button).toHaveAttribute('type', 'button')
    expect(button.className).toContain('btn--link')
    expect(button.className).not.toContain('btn--primary')
  })

  it('disables the link like any other button', async () => {
    const onclick = vi.fn()
    render(ButtonTestHost, {
      props: { variant: 'link', disabled: true, label: 'Dismiss', onclick },
    })

    const button = screen.getByRole('button', { name: 'Dismiss' })
    expect(button).toBeDisabled()
    await fireEvent.click(button)
    expect(onclick).not.toHaveBeenCalled()
  })
})
