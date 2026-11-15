import { render, screen } from '@testing-library/svelte'
import { createRawSnippet } from 'svelte'
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import StatusBadge from '../StatusBadge.svelte'

const source = readFileSync(resolve(import.meta.dirname, '../StatusBadge.svelte'), 'utf-8')

const text = (value: string) => createRawSnippet(() => ({ render: () => `<span>${value}</span>` }))

const badge = (container: HTMLElement) => container.querySelector<HTMLElement>('.status-badge')!

describe('StatusBadge', () => {
  it('keeps sm and neutral as the defaults', () => {
    const { container } = render(StatusBadge, { props: { children: text('Draft') } })

    const el = badge(container)
    expect(el).toHaveTextContent('Draft')
    expect(el).toHaveClass('status-badge--sm', 'status-badge--neutral')
    expect(el).not.toHaveClass('status-badge--count')
  })

  it('renders the xs size on the 2xs type step with a tokenised height', () => {
    const { container } = render(StatusBadge, {
      props: { size: 'xs', variant: 'info', children: text('Beta') },
    })

    expect(badge(container)).toHaveClass('status-badge--xs', 'status-badge--info')
    const xsRule = source.match(/\.status-badge--xs\s*\{[^}]*\}/)?.[0] ?? ''
    expect(xsRule).toContain('font-size: var(--font-size-2xs)')
    expect(xsRule).toContain('--status-badge-height:')
    expect(source).toContain('min-height: var(--status-badge-height)')
  })

  it('shows a count as a danger pill by default', () => {
    const { container } = render(StatusBadge, { props: { variant: 'count', count: 7 } })

    const el = badge(container)
    expect(el).toHaveTextContent('7')
    expect(el).toHaveClass('status-badge--count', 'status-badge--danger')
    expect(source).toMatch(
      /\.status-badge--count\.status-badge--danger\s*\{[^}]*background: var\(--color-danger\);[^}]*color: var\(--color-on-danger\);/
    )
    const countRule = source.match(/\.status-badge--count\s*\{[^}]*\}/)?.[0] ?? ''
    expect(countRule).toContain('border-radius: var(--radius-full)')
    expect(countRule).toContain('font-variant-numeric: tabular-nums')
  })

  it('lets a count take any of the existing tones', () => {
    const { container } = render(StatusBadge, {
      props: { variant: 'count', tone: 'ai', count: 3 },
    })

    const el = badge(container)
    expect(el).toHaveClass('status-badge--count', 'status-badge--ai')
    expect(el).not.toHaveClass('status-badge--danger')
  })

  it('caps a count above max as "max+"', () => {
    const { container } = render(StatusBadge, { props: { variant: 'count', count: 150, max: 99 } })
    expect(badge(container)).toHaveTextContent('99+')
  })

  it('shows a count at or below max as is', () => {
    const { container } = render(StatusBadge, { props: { variant: 'count', count: 99, max: 99 } })
    expect(badge(container)).toHaveTextContent(/^99$/)
  })

  it('names a bare count through aria-label', () => {
    render(StatusBadge, {
      props: { variant: 'count', count: 4, 'aria-label': '4 unread notifications' },
    })

    expect(screen.getByRole('img', { name: '4 unread notifications' })).toHaveTextContent('4')
  })

  it('keeps an explicit role over the default count role', () => {
    render(StatusBadge, {
      props: { variant: 'count', count: 2, role: 'status', 'aria-label': '2 pending' },
    })

    expect(screen.getByRole('status', { name: '2 pending' })).toBeInTheDocument()
  })

  it('passes class and extra attributes through', () => {
    const { container } = render(StatusBadge, {
      props: { class: 'extra', title: 'Saved', variant: 'stored', children: text('Saved') },
    })

    const el = badge(container)
    expect(el).toHaveClass('extra', 'status-badge--stored')
    expect(el).toHaveAttribute('title', 'Saved')
  })

  it('removes nothing from the existing tone vocabulary', () => {
    expect(source).toContain('var(--state-ai-soft)')
    expect(source).toContain('var(--state-evidence-soft)')
  })
})
