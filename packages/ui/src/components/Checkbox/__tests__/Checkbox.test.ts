import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import Checkbox from '../Checkbox.svelte'

const source = readFileSync(resolve(import.meta.dirname, '../Checkbox.svelte'), 'utf-8')

describe('Checkbox', () => {
  it('reports the new value to its caller', async () => {
    const onchange = vi.fn()
    render(Checkbox, { props: { label: 'OCR', onchange } })

    const control = screen.getByRole('checkbox', { name: 'OCR' })
    expect(control).not.toBeChecked()

    await fireEvent.click(control)
    expect(control).toBeChecked()
    expect(onchange).toHaveBeenCalledWith(true)

    await fireEvent.click(control)
    expect(onchange).toHaveBeenLastCalledWith(false)
  })

  it('shows a mixed state when asked', async () => {
    render(Checkbox, { props: { label: 'All', indeterminate: true } })
    const control = screen.getByRole('checkbox', { name: 'All' })
    expect(control).toBePartiallyChecked()
  })

  it('does not change while disabled', async () => {
    const onchange = vi.fn()
    render(Checkbox, { props: { label: 'Embeddings', disabled: true, onchange } })

    const control = screen.getByRole('checkbox', { name: 'Embeddings' })
    expect(control).toBeDisabled()
    await fireEvent.click(control)
    expect(onchange).not.toHaveBeenCalled()
  })

  it('replaces the native control instead of tinting it', () => {
    // The point of the component: the platform checkbox paints itself with the
    // system accent, which is blue on every desktop the app ships to.
    expect(source).toContain('appearance: none;')
    expect(source).toContain('-webkit-appearance: none;')
    expect(source).not.toContain('accent-color')
    // Selected state reads as contrast on the neutral surfaces, not as hue.
    expect(source).toContain('border-color: var(--color-text-primary);')
    expect(source).toContain('background: var(--surface-toolbar);')
    // One focus ring for the row, drawn with the published token.
    expect(source).toContain('box-shadow: var(--focus-ring);')
    expect(source).toContain('outline: none;')
  })
})

const styles = source.split(/<style[^>]*>/)[1]!.replace(/\/\*[\s\S]*?\*\//g, '')

function rule(selector: string): string {
  const at = styles.indexOf(`${selector} {`)
  expect(at, `${selector} is not in the stylesheet`).toBeGreaterThan(-1)
  return styles.slice(at, styles.indexOf('}', at))
}

/**
 * Two looks views used to get by restyling the row with `:global`: a compact
 * row (size sm) and a bordered card that lifts off a sunken list (variant
 * card). The component owns both; the defaults stay the plain md row.
 */
describe('Checkbox size and variant', () => {
  it('defaults to the md row', () => {
    render(Checkbox, { props: { label: 'Plain' } })
    const row = screen.getByRole('checkbox', { name: 'Plain' }).closest('label')!
    expect(row).toHaveClass('checkbox', 'checkbox--row', 'checkbox--md')
  })

  it('carries the size and variant classes it is given', () => {
    render(Checkbox, { props: { label: 'Card', size: 'sm', variant: 'card' } })
    const row = screen.getByRole('checkbox', { name: 'Card' }).closest('label')!
    expect(row).toHaveClass('checkbox--card', 'checkbox--sm')
  })

  it('size sm is the compact row: tighter padding, gap and type', () => {
    const sm = rule('.checkbox--sm')
    expect(sm).toMatch(/gap:\s*var\(--space-2\)/)
    expect(sm).toMatch(/padding:\s*var\(--space-1\) var\(--space-2\)/)
    expect(sm).toMatch(/font-size:\s*var\(--font-size-xs\)/)
  })

  it('variant card is a bordered surface whose checked border is firmer than its resting one', () => {
    const card = rule('.checkbox--card')
    expect(card).toMatch(/background:\s*var\(--surface-panel\)/)
    expect(card).toMatch(/border-color:\s*var\(--border-subtle\)/)
    expect(rule('.checkbox--card:hover')).toMatch(/background:\s*var\(--surface-toolbar\)/)
    const checked = rule('.checkbox--card:has(input:checked)')
    expect(checked).toMatch(/background:\s*var\(--surface-toolbar\)/)
    expect(checked).toMatch(/border-color:\s*var\(--border-panel\)/)
  })

  it('states the card rules after the row rules, so they win by order', () => {
    expect(styles.indexOf('.checkbox--card {')).toBeGreaterThan(
      styles.indexOf('.checkbox:has(input:checked) {')
    )
  })
})
