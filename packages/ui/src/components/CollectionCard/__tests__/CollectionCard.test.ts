import { render, screen, fireEvent } from '@testing-library/svelte'
import { afterEach, describe, it, expect, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import CollectionCard from '../CollectionCard.svelte'

function readCollectionCardSource(): string {
  return readFileSync(resolve(import.meta.dirname, '../CollectionCard.svelte'), 'utf-8')
}

describe('CollectionCard', () => {
  const baseProps = {
    id: 'col-1',
    name: 'My Collection',
    itemCount: 5,
    updatedAt: Date.now() - 2 * 24 * 60 * 60 * 1000, // 2 days ago
  }

  it('renders the collection name as bold text', () => {
    render(CollectionCard, { props: baseProps })
    const nameEl = screen.getByRole('heading', { name: 'My Collection' })
    expect(nameEl).toBeInTheDocument()
    expect(nameEl.tagName).toBe('H3')
  })

  it('renders the item count badge', () => {
    render(CollectionCard, { props: baseProps })
    expect(screen.getByText('5 items')).toBeInTheDocument()
  })

  it('renders description when provided', () => {
    render(CollectionCard, {
      props: { ...baseProps, description: 'A test description for the collection' },
    })
    expect(screen.getByText('A test description for the collection')).toBeInTheDocument()
  })

  it('uses the collection name as the description when not provided', () => {
    render(CollectionCard, { props: baseProps })
    expect(screen.getByTestId('collection-description')).toHaveTextContent('My Collection')
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it.each([
    ['en', 1, '1 hour ago'],
    ['en', 11, '11 hours ago'],
    ['es', 1, 'hace 1 hora'],
    ['es', 11, 'hace 11 horas'],
  ])('renders %s relative hours with the correct singular or plural form', (locale, hours, expected) => {
    vi.useFakeTimers()
    const now = new Date('2026-07-19T12:00:00Z')
    vi.setSystemTime(now)

    render(CollectionCard, {
      props: { ...baseProps, locale, updatedAt: now.getTime() - hours * 60 * 60 * 1000 },
    })

    expect(screen.getByTestId('collection-date')).toHaveTextContent(expected)
  })

  it('calls onclick when clicked', async () => {
    const onclick = vi.fn()
    render(CollectionCard, { props: { ...baseProps, onclick } })
    const card = screen.getByRole('button')
    await fireEvent.click(card)
    expect(onclick).toHaveBeenCalledOnce()
  })

  it('renders singular "item" for count of 1', () => {
    render(CollectionCard, { props: { ...baseProps, itemCount: 1 } })
    expect(screen.getByText('1 item')).toBeInTheDocument()
  })

  it('shows edit button when onedit is provided', () => {
    render(CollectionCard, { props: { ...baseProps, onedit: vi.fn() } })
    expect(screen.getByRole('button', { name: 'Edit collection' })).toBeInTheDocument()
  })

  it('shows delete button when ondelete is provided', () => {
    render(CollectionCard, { props: { ...baseProps, ondelete: vi.fn() } })
    expect(screen.getByRole('button', { name: 'Delete collection' })).toBeInTheDocument()
  })

  it('does not show edit/delete buttons when callbacks are not provided', () => {
    render(CollectionCard, { props: baseProps })
    expect(screen.queryByTestId('edit-button')).not.toBeInTheDocument()
    expect(screen.queryByTestId('delete-button')).not.toBeInTheDocument()
  })

  it('calls onedit without triggering card onclick', async () => {
    const onclick = vi.fn()
    const onedit = vi.fn()
    render(CollectionCard, { props: { ...baseProps, onclick, onedit } })
    await fireEvent.click(screen.getByTestId('edit-button'))
    expect(onedit).toHaveBeenCalledOnce()
    expect(onclick).not.toHaveBeenCalled()
  })

  it('calls ondelete without triggering card onclick', async () => {
    const onclick = vi.fn()
    const ondelete = vi.fn()
    render(CollectionCard, { props: { ...baseProps, onclick, ondelete } })
    await fireEvent.click(screen.getByTestId('delete-button'))
    expect(ondelete).toHaveBeenCalledOnce()
    expect(onclick).not.toHaveBeenCalled()
  })

  it('declares single-line ellipsis styles for the description', () => {
    const source = readCollectionCardSource()
    expect(source).toMatch(
      /\.collection-card__description\s*\{[^}]*overflow:\s*hidden;[^}]*white-space:\s*nowrap;[^}]*text-overflow:\s*ellipsis;/s
    )
  })
})
