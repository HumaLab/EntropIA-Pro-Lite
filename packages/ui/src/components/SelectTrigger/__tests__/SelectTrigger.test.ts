import { fireEvent, render, screen } from '@testing-library/svelte'
import { tick } from 'svelte'
import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import SelectTrigger from '../SelectTrigger.svelte'
import Select from '../Select.svelte'
import SelectTestHost from './SelectTestHost.svelte'

const triggerSource = readFileSync(resolve(import.meta.dirname, '../SelectTrigger.svelte'), 'utf-8')
const selectSource = readFileSync(resolve(import.meta.dirname, '../Select.svelte'), 'utf-8')

const OPTIONS = [
  { value: 'user', label: 'Personal' },
  { value: 'group', label: 'Group' },
  { value: 'shared', label: 'Shared', disabled: true },
]

describe('SelectTrigger', () => {
  it('is a plain button announcing a collapsed menu by default', () => {
    render(SelectTrigger, { props: { label: 'Personal', 'aria-label': 'Library' } })
    const button = screen.getByRole('button', { name: 'Library' })
    expect(button).toHaveAttribute('type', 'button')
    expect(button).toHaveAttribute('aria-haspopup', 'menu')
    expect(button).toHaveAttribute('aria-expanded', 'false')
    expect(button).toHaveTextContent('Personal')
  })

  it('reflects the open state', () => {
    render(SelectTrigger, { props: { label: 'Personal', open: true, 'aria-label': 'Library' } })
    const button = screen.getByRole('button', { name: 'Library' })
    expect(button).toHaveAttribute('aria-expanded', 'true')
    expect(button).toHaveClass('select-trigger--open')
  })

  it('shows the placeholder when nothing is chosen', () => {
    render(SelectTrigger, { props: { placeholder: 'Choose…', 'aria-label': 'Library' } })
    const button = screen.getByRole('button', { name: 'Library' })
    expect(button).toHaveTextContent('Choose…')
    expect(button.querySelector('.select-trigger__value--placeholder')).not.toBeNull()
  })

  it('maps size and variant to classes, md field by default', () => {
    const { unmount } = render(SelectTrigger, { props: { 'aria-label': 'A' } })
    const plain = screen.getByRole('button', { name: 'A' })
    expect(plain).toHaveClass('select-trigger--md', 'select-trigger--field')
    unmount()

    render(SelectTrigger, { props: { size: 'sm', variant: 'quiet', 'aria-label': 'B' } })
    expect(screen.getByRole('button', { name: 'B' })).toHaveClass(
      'select-trigger--sm',
      'select-trigger--quiet'
    )
  })

  it('draws its chevron through ActionIcon', () => {
    expect(triggerSource).toMatch(/<ActionIcon\s+name="chevron-down"/)
    expect(triggerSource).not.toContain('@tabler/')
  })

  it('does not fire while disabled', async () => {
    const onclick = vi.fn()
    render(SelectTrigger, { props: { disabled: true, onclick, 'aria-label': 'Library' } })
    const button = screen.getByRole('button', { name: 'Library' })
    expect(button).toBeDisabled()
    await fireEvent.click(button)
    expect(onclick).not.toHaveBeenCalled()
  })

  it('caps each size at its token and dims with the disabled token', () => {
    expect(triggerSource).toMatch(
      /\.select-trigger--sm\s*\{[^}]*min-height:\s*var\(--control-height-sm\);[^}]*max-height:\s*var\(--control-height-sm\);/
    )
    expect(triggerSource).toMatch(
      /\.select-trigger--md\s*\{[^}]*min-height:\s*var\(--control-height-md\);[^}]*max-height:\s*var\(--control-height-md\);/
    )
    expect(triggerSource).toMatch(/:disabled\s*\{[^}]*opacity:\s*var\(--opacity-disabled\)/)
    expect(triggerSource).toMatch(/:focus-visible\s*\{[^}]*box-shadow:\s*var\(--focus-ring\)/)
    expect(triggerSource).toContain('@media (prefers-reduced-motion: reduce)')
  })

  it('never renders a native select', () => {
    expect(triggerSource).not.toMatch(/<select\b/)
    expect(selectSource).not.toMatch(/<select\b/)
  })
})

const trigger = () => screen.getByRole('button', { name: 'Library type' })
const menu = () => screen.queryByRole('menu', { name: 'Library type' })

describe('Select', () => {
  it('shows the chosen option and opens a radio menu', async () => {
    render(Select, {
      props: {
        label: 'Library type',
        options: OPTIONS,
        value: 'group',
        'aria-label': 'Library type',
      },
    })
    expect(trigger()).toHaveTextContent('Group')
    expect(menu()).toBeNull()

    await fireEvent.click(trigger())
    await tick()

    expect(trigger()).toHaveAttribute('aria-expanded', 'true')
    const radios = screen.getAllByRole('menuitemradio')
    expect(radios.map((r) => r.textContent?.trim())).toEqual(['Personal', 'Group', 'Shared'])
    expect(screen.getByRole('menuitemradio', { name: 'Group' })).toHaveAttribute(
      'aria-checked',
      'true'
    )
    expect(screen.getByRole('menuitemradio', { name: 'Personal' })).toHaveAttribute(
      'aria-checked',
      'false'
    )
  })

  it('updates the bound value, closes and gives the focus back on choose', async () => {
    const onchange = vi.fn()
    render(SelectTestHost, { props: { options: OPTIONS, initial: 'user', onchange } })

    await fireEvent.click(trigger())
    await tick()
    await fireEvent.click(screen.getByRole('menuitemradio', { name: 'Group' }))
    await tick()

    expect(menu()).toBeNull()
    expect(trigger()).toHaveAttribute('aria-expanded', 'false')
    expect(document.activeElement).toBe(trigger())
    expect(onchange).toHaveBeenCalledWith('group')
    expect(screen.getByTestId('bound')).toHaveTextContent('group')
    expect(trigger()).toHaveTextContent('Group')
  })

  it('ignores a disabled option', async () => {
    const onchange = vi.fn()
    render(SelectTestHost, { props: { options: OPTIONS, initial: 'user', onchange } })

    await fireEvent.click(trigger())
    await tick()
    await fireEvent.click(screen.getByRole('menuitemradio', { name: 'Shared' }))
    await tick()

    expect(onchange).not.toHaveBeenCalled()
    expect(screen.getByTestId('bound')).toHaveTextContent('user')
  })

  it('shows the placeholder for a value no option holds', () => {
    render(Select, {
      props: {
        label: 'Library type',
        options: OPTIONS,
        value: '',
        placeholder: 'Pick one',
        'aria-label': 'Library type',
      },
    })
    expect(trigger()).toHaveTextContent('Pick one')
  })

  it('does not open while disabled', async () => {
    render(Select, {
      props: {
        label: 'Library type',
        options: OPTIONS,
        value: 'user',
        disabled: true,
        'aria-label': 'Library type',
      },
    })
    await fireEvent.click(trigger())
    await tick()
    expect(menu()).toBeNull()
  })
})
