import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import SearchClearButton from '../SearchClearButton.svelte'

describe('SearchClearButton', () => {
  it('renders the canonical accessible clear control and invokes its callback', async () => {
    const onclick = vi.fn()
    render(SearchClearButton, {
      props: { label: 'Limpiar búsqueda', class: 'search-clear-button--overlay', onclick },
    })

    const button = screen.getByRole('button', { name: 'Limpiar búsqueda' })
    expect(button).toHaveAttribute('aria-label', 'Limpiar búsqueda')
    expect(button).toHaveAttribute('title', 'Limpiar búsqueda')
    expect(button).toHaveAttribute('type', 'button')
    expect(button).toHaveClass(
      'icon-button',
      'icon-button--ghost',
      'icon-button--sm',
      'search-clear-button',
      'search-clear-button--overlay',
    )
    expect(button.querySelector('svg')).toHaveAttribute('width', '14')
    expect(button.querySelector('svg')).toHaveAttribute('height', '14')

    await fireEvent.click(button)
    expect(onclick).toHaveBeenCalledOnce()
  })

  it('does not activate while disabled', async () => {
    const onclick = vi.fn()
    render(SearchClearButton, { props: { label: 'Clear search', disabled: true, onclick } })

    const button = screen.getByRole('button', { name: 'Clear search' })
    expect(button).toBeDisabled()
    await fireEvent.click(button)
    expect(onclick).not.toHaveBeenCalled()
  })
})
