import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import SearchBar from '../SearchBar.svelte'

describe('SearchBar', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('renders with placeholder text', () => {
    render(SearchBar, { props: { placeholder: 'Search collections...' } })
    const input = screen.getByPlaceholderText('Search collections...')
    expect(input).toBeInTheDocument()
  })

  it('uses an accessible name for the searchbox', () => {
    render(SearchBar, { props: { placeholder: 'Search collections...' } })

    expect(screen.getByRole('searchbox', { name: 'Search collections...' })).toBeInTheDocument()
  })

  it('renders the search icon', () => {
    render(SearchBar, { props: {} })
    expect(screen.getByTestId('search-icon')).toBeInTheDocument()
  })

  it('emits onsearch after debounce delay', async () => {
    const onsearch = vi.fn()
    render(SearchBar, { props: { onsearch, debounceMs: 300 } })
    const input = screen.getByRole('searchbox')

    await fireEvent.input(input, { target: { value: 'test query' } })
    // Should NOT have fired yet
    expect(onsearch).not.toHaveBeenCalled()

    // Advance past debounce
    vi.advanceTimersByTime(300)
    expect(onsearch).toHaveBeenCalledOnce()
    expect(onsearch).toHaveBeenCalledWith('test query')
  })

  it('rapid typing triggers single search after last keystroke', async () => {
    const onsearch = vi.fn()
    render(SearchBar, { props: { onsearch, debounceMs: 300 } })
    const input = screen.getByRole('searchbox')

    await fireEvent.input(input, { target: { value: 't' } })
    vi.advanceTimersByTime(100)
    await fireEvent.input(input, { target: { value: 'te' } })
    vi.advanceTimersByTime(100)
    await fireEvent.input(input, { target: { value: 'tes' } })
    vi.advanceTimersByTime(100)
    await fireEvent.input(input, { target: { value: 'test' } })

    // Still within debounce window from last input
    expect(onsearch).not.toHaveBeenCalled()

    vi.advanceTimersByTime(300)
    expect(onsearch).toHaveBeenCalledOnce()
    expect(onsearch).toHaveBeenCalledWith('test')
  })

  it('keeps local typed value before debounce emits onsearch', async () => {
    const onsearch = vi.fn()
    render(SearchBar, { props: { value: 'seed', onsearch, debounceMs: 300 } })
    const input = screen.getByRole('searchbox') as HTMLInputElement

    await fireEvent.input(input, { target: { value: 'seed local' } })

    expect(input.value).toBe('seed local')
    expect(onsearch).not.toHaveBeenCalled()

    vi.advanceTimersByTime(300)
    expect(onsearch).toHaveBeenCalledWith('seed local')
  })

  it('re-syncs input when parent updates external value', async () => {
    const view = render(SearchBar, { props: { value: 'initial' } })
    const input = screen.getByRole('searchbox') as HTMLInputElement

    await fireEvent.input(input, { target: { value: 'local typing' } })
    expect(input.value).toBe('local typing')

    await view.rerender({ value: '' })
    expect(input.value).toBe('')
  })

  it('shows clear button when input has value', async () => {
    render(SearchBar, { props: { value: 'something' } })
    const clearButton = screen.getByRole('button', { name: 'Clear search' })
    expect(clearButton).toHaveAttribute('data-tooltip', 'Clear search')
    expect(clearButton).toHaveClass(
      'icon-button',
      'icon-button--ghost',
      'icon-button--sm',
      'search-clear-button'
    )
    expect(clearButton).toHaveAttribute('data-testid', 'search-clear')
  })

  it('hides clear button when input is empty', () => {
    render(SearchBar, { props: {} })
    expect(screen.queryByTestId('search-clear')).not.toBeInTheDocument()
  })

  it('clears input and calls onclear when clear button clicked', async () => {
    const onclear = vi.fn()
    const onsearch = vi.fn()
    render(SearchBar, { props: { value: 'test', onclear, onsearch } })

    const clearBtn = screen.getByTestId('search-clear')
    await fireEvent.click(clearBtn)

    expect(onclear).toHaveBeenCalledOnce()
  })

  it('uses custom debounce delay', async () => {
    const onsearch = vi.fn()
    render(SearchBar, { props: { onsearch, debounceMs: 500 } })
    const input = screen.getByRole('searchbox')

    await fireEvent.input(input, { target: { value: 'query' } })

    vi.advanceTimersByTime(300)
    expect(onsearch).not.toHaveBeenCalled()

    vi.advanceTimersByTime(200)
    expect(onsearch).toHaveBeenCalledOnce()
    expect(onsearch).toHaveBeenCalledWith('query')
  })

  it('forwards id, disabled, autocomplete and spellcheck to the input', () => {
    render(SearchBar, {
      props: { id: 'q', disabled: true, autocomplete: 'off', spellcheck: false },
    })
    const input = screen.getByRole('searchbox')
    expect(input).toHaveAttribute('id', 'q')
    expect(input).toBeDisabled()
    expect(input).toHaveAttribute('autocomplete', 'off')
    expect(input).toHaveAttribute('spellcheck', 'false')
  })

  it('disables only the clear button with clearDisabled', () => {
    render(SearchBar, { props: { value: 'abc', clearDisabled: true } })
    expect(screen.getByRole('searchbox')).not.toBeDisabled()
    expect(screen.getByTestId('search-clear')).toBeDisabled()
  })
})

describe('SearchBar size and combobox wiring', () => {
  const source = () => readFileSync(resolve(import.meta.dirname, '../SearchBar.svelte'), 'utf-8')

  it('is md by default and caps the sm size at its token', () => {
    const { container, unmount } = render(SearchBar, { props: { ariaLabel: 'A' } })
    expect(container.querySelector('.search-bar')).toHaveClass('search-bar--md')
    unmount()

    const sm = render(SearchBar, { props: { ariaLabel: 'B', size: 'sm' } })
    expect(sm.container.querySelector('.search-bar')).toHaveClass('search-bar--sm')
    expect(source()).toMatch(
      /\.search-bar--sm\s*\{[^}]*min-height:\s*var\(--control-height-sm\);[^}]*max-height:\s*var\(--control-height-sm\);/
    )
  })

  it('carries combobox semantics onto the input', () => {
    render(SearchBar, {
      props: {
        ariaLabel: 'Search documents',
        role: 'combobox',
        'aria-expanded': true,
        'aria-controls': 'results',
        'aria-autocomplete': 'list',
        'aria-activedescendant': 'result-2',
      },
    })

    const input = screen.getByRole('combobox', { name: 'Search documents' })
    expect(input).toHaveAttribute('aria-expanded', 'true')
    expect(input).toHaveAttribute('aria-controls', 'results')
    expect(input).toHaveAttribute('aria-autocomplete', 'list')
    expect(input).toHaveAttribute('aria-activedescendant', 'result-2')
  })
})
