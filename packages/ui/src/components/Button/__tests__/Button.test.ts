import { render, screen } from '@testing-library/svelte'
import { describe, expect, it } from 'vitest'
import ButtonTestHost from './ButtonTestHost.svelte'

describe('Button', () => {
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
