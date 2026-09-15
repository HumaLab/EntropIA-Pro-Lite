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
