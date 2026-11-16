import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import Switch from '../Switch.svelte'
import SwitchHarness from './SwitchHarness.svelte'

const source = readFileSync(resolve(import.meta.dirname, '../Switch.svelte'), 'utf-8')

describe('Switch', () => {
  it('is a native checkbox exposed as a switch, named by its visible label', async () => {
    const onchange = vi.fn()
    render(Switch, { props: { label: 'Allow external assistants', onchange } })

    const control = screen.getByRole('switch', { name: 'Allow external assistants' })
    expect(control.tagName).toBe('INPUT')
    expect(control).toHaveAttribute('type', 'checkbox')
    expect(screen.getByText('Allow external assistants')).toBeVisible()
    expect(control).not.toBeChecked()

    await fireEvent.click(control)
    expect(control).toBeChecked()
    expect(onchange).toHaveBeenCalledWith(true)

    await fireEvent.click(control)
    expect(control).not.toBeChecked()
    expect(onchange).toHaveBeenLastCalledWith(false)
  })

  it('takes an accessible name without a visible label', () => {
    render(Switch, { props: { ariaLabel: 'Dark mode' } })

    const control = screen.getByRole('switch', { name: 'Dark mode' })
    expect(control).toHaveAttribute('aria-label', 'Dark mode')
    expect(screen.queryByText('Dark mode')).toBeNull()
  })

  it('links its description to the control', () => {
    render(Switch, { props: { label: 'Sync', description: 'Runs every five minutes' } })

    const control = screen.getByRole('switch', { name: 'Sync' })
    expect(control).toHaveAccessibleDescription('Runs every five minutes')
  })

  it('starts from the checked prop', () => {
    render(Switch, { props: { label: 'Sync', checked: true } })
    expect(screen.getByRole('switch', { name: 'Sync' })).toBeChecked()
  })

  it('writes back through bind:checked and hands out the native input', async () => {
    render(SwitchHarness)

    expect(screen.getByTestId('bound')).toHaveTextContent('off')
    expect(screen.getByTestId('element')).toHaveTextContent('INPUT')

    await fireEvent.click(screen.getByRole('switch', { name: 'Sync' }))
    expect(screen.getByTestId('bound')).toHaveTextContent('on')
  })

  it('does not change while disabled', async () => {
    const onchange = vi.fn()
    render(Switch, { props: { label: 'Sync', disabled: true, onchange } })

    const control = screen.getByRole('switch', { name: 'Sync' })
    expect(control).toBeDisabled()
    await fireEvent.click(control)
    expect(control).not.toBeChecked()
    expect(onchange).not.toHaveBeenCalled()
  })

  it('submits its value like any checkbox', async () => {
    const form = document.createElement('form')
    document.body.append(form)
    render(Switch, { target: form, props: { label: 'Sync', name: 'sync', value: 'yes' } })

    expect(new FormData(form).get('sync')).toBeNull()
    await fireEvent.click(screen.getByRole('switch', { name: 'Sync' }))
    expect(new FormData(form).get('sync')).toBe('yes')
    form.remove()
  })

  it('applies the requested size', () => {
    const { container } = render(Switch, { props: { label: 'Sync', size: 'sm' } })
    expect(container.querySelector('.switch')).toHaveClass('switch--sm')
  })

  it('draws itself from the design tokens', () => {
    // The platform checkbox paints itself with the system accent; the switch
    // replaces the native look instead of tinting it.
    expect(source).toContain('appearance: none;')
    expect(source).not.toContain('accent-color')
    expect(source).toContain('background: var(--color-accent);')
    expect(source).toContain('opacity: var(--opacity-disabled);')
    expect(source).toContain('box-shadow: var(--focus-ring);')
    expect(source).toContain('transition: transform var(--transition-base)')
    // Geometry comes from the component's own properties, not bare pixels.
    expect(source).toContain('inline-size: var(--switch-track-width);')
    expect(source).toContain('block-size: var(--switch-track-height);')
    expect(source).toMatch(/@media \(prefers-reduced-motion: reduce\)[\s\S]*transition: none;/)
  })
})
