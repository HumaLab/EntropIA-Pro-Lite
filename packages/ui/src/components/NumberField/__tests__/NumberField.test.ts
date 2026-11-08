import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import NumberField from '../NumberField.svelte'

const source = readFileSync(resolve(import.meta.dirname, '../NumberField.svelte'), 'utf-8')

const labels = { decrementLabel: 'Disminuir', incrementLabel: 'Aumentar' }

function field() {
  return screen.getByRole('spinbutton') as HTMLInputElement
}

describe('NumberField', () => {
  it('steps by one by default and fires the native input and change events', async () => {
    const oninput = vi.fn()
    const onchange = vi.fn((event: Event) => (event.currentTarget as HTMLInputElement).value)
    render(NumberField, { props: { ...labels, value: 4, label: 'Términos', oninput, onchange } })

    expect(screen.getByRole('spinbutton', { name: 'Términos' })).toHaveValue(4)
    await fireEvent.click(screen.getByRole('button', { name: 'Aumentar' }))
    expect(field()).toHaveValue(5)
    expect(oninput).toHaveBeenCalledTimes(1)
    expect(onchange).toHaveBeenCalledTimes(1)
    expect(onchange.mock.results[0]!.value).toBe('5')

    await fireEvent.click(screen.getByRole('button', { name: 'Disminuir' }))
    expect(field()).toHaveValue(4)
    expect(onchange).toHaveBeenCalledTimes(2)
  })

  it('keeps a declared step and rounds to its decimals', async () => {
    render(NumberField, { props: { ...labels, value: 0.1, step: 0.05, min: 0, max: 1 } })
    await fireEvent.click(screen.getByRole('button', { name: 'Aumentar' }))
    expect(field().value).toBe('0.15')
    await fireEvent.click(screen.getByRole('button', { name: 'Aumentar' }))
    expect(field().value).toBe('0.2')
  })

  it('clamps to the bounds and disables the button at each bound', async () => {
    const onchange = vi.fn()
    render(NumberField, { props: { ...labels, value: 9, min: 1, max: 10, step: 5, onchange } })
    const increment = screen.getByRole('button', { name: 'Aumentar' })
    await fireEvent.click(increment)
    expect(field()).toHaveValue(10)
    expect(increment).toBeDisabled()
    await fireEvent.click(increment)
    expect(onchange).toHaveBeenCalledTimes(1)
    const decrement = screen.getByRole('button', { name: 'Disminuir' })
    await fireEvent.click(decrement)
    await fireEvent.click(decrement)
    expect(field()).toHaveValue(1)
    expect(decrement).toBeDisabled()
  })

  it('starts an empty field from its minimum, or zero', async () => {
    render(NumberField, { props: { ...labels, value: null, min: 3 } })
    await fireEvent.click(screen.getByRole('button', { name: 'Aumentar' }))
    expect(field()).toHaveValue(3)
  })

  it('does nothing while disabled', async () => {
    const onchange = vi.fn()
    render(NumberField, { props: { ...labels, value: 2, disabled: true, onchange } })
    expect(field()).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Aumentar' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Disminuir' })).toBeDisabled()
    expect(onchange).not.toHaveBeenCalled()
  })

  it('passes the field attributes through and keeps the buttons out of the tab order', () => {
    render(NumberField, {
      props: { ...labels, id: 'years', value: 1990, min: 0, max: 9999, class: 'ia-field__input' },
    })
    expect(field()).toHaveAttribute('id', 'years')
    expect(field()).toHaveAttribute('min', '0')
    expect(field()).toHaveAttribute('max', '9999')
    expect(field()).toHaveAttribute('type', 'number')
    for (const name of ['Aumentar', 'Disminuir'])
      expect(screen.getByRole('button', { name })).toHaveAttribute('tabindex', '-1')
  })

  it('hides the native spin buttons and draws its icons through ActionIcon', () => {
    expect(source).toContain('::-webkit-inner-spin-button')
    expect(source).toContain('appearance: textfield')
    expect(source).toContain('name="add"')
    expect(source).toContain('name="subtract"')
  })
})
