import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import Disclosure from '../Disclosure.svelte'
import DisclosureTestHost from './DisclosureTestHost.svelte'

const source = readFileSync(resolve(import.meta.dirname, '../Disclosure.svelte'), 'utf-8')

const header = () => screen.getByRole('button', { name: /Technical detail/ })

describe('Disclosure', () => {
  it('starts closed with its region hidden', () => {
    render(DisclosureTestHost)
    expect(header()).toHaveAttribute('type', 'button')
    expect(header()).toHaveAttribute('aria-expanded', 'false')
    expect(screen.getByText('Body text').closest('[role="region"]')).not.toBeVisible()
  })

  it('wires the header and the region to each other', () => {
    render(DisclosureTestHost, { props: { initial: true } })
    const region = screen.getByRole('region', { name: /Technical detail/ })
    expect(header()).toHaveAttribute('aria-controls', region.id)
    expect(region).toHaveAttribute('aria-labelledby', header().id)
    expect(region).toContainElement(screen.getByText('Body text'))
  })

  it('toggles on click and reports the new state', async () => {
    const ontoggle = vi.fn()
    render(DisclosureTestHost, { props: { ontoggle } })

    await fireEvent.click(header())
    expect(header()).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByRole('region')).toBeVisible()
    expect(ontoggle).toHaveBeenLastCalledWith(true)

    await fireEvent.click(header())
    expect(header()).toHaveAttribute('aria-expanded', 'false')
    expect(ontoggle).toHaveBeenLastCalledWith(false)
  })

  it('toggles from the keyboard with Enter and Space', async () => {
    // The header is a native button: the platform turns Enter and Space into a
    // click (detail 0). The component must leave those keys alone, or a manual
    // handler would toggle twice in a real webview.
    render(DisclosureTestHost)
    header().focus()
    expect(header()).toHaveFocus()

    for (const key of ['Enter', ' ']) {
      const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true })
      header().dispatchEvent(event)
      expect(event.defaultPrevented).toBe(false)
    }

    await fireEvent.click(header(), { detail: 0 })
    expect(header()).toHaveAttribute('aria-expanded', 'true')

    await fireEvent.click(header(), { detail: 0 })
    expect(header()).toHaveAttribute('aria-expanded', 'false')
  })

  it('keeps `open` bound both ways', async () => {
    render(DisclosureTestHost)
    await fireEvent.click(header())
    expect(screen.getByTestId('bound')).toHaveTextContent('open')

    await fireEvent.click(screen.getByRole('button', { name: 'External toggle' }))
    expect(header()).toHaveAttribute('aria-expanded', 'false')
  })

  it('does not toggle while disabled', async () => {
    const ontoggle = vi.fn()
    render(DisclosureTestHost, { props: { disabled: true, ontoggle } })
    expect(header()).toBeDisabled()

    await fireEvent.click(header())
    expect(header()).toHaveAttribute('aria-expanded', 'false')
    expect(ontoggle).not.toHaveBeenCalled()
  })

  it('renders the meta snippet inside the header and the card variant class', () => {
    const { container } = render(DisclosureTestHost, { props: { variant: 'card' } })
    expect(header()).toContainElement(screen.getByTestId('meta'))
    expect(container.querySelector('.disclosure--card')).not.toBeNull()
  })

  it('defaults to the plain variant', () => {
    const { container } = render(Disclosure, { props: { summary: 'Plain' } })
    expect(container.querySelector('.disclosure--plain')).not.toBeNull()
  })

  it('draws the chevron through ActionIcon and drops motion when asked', () => {
    expect(source).toContain("import ActionIcon from '../Button/ActionIcon.svelte'")
    expect(source).not.toContain('@tabler/icons-svelte-runes')
    expect(source).toContain('transition: transform var(--transition-base)')
    expect(source).toContain('@media (prefers-reduced-motion: reduce)')
    expect(source).toContain('box-shadow: var(--focus-ring);')
    expect(source).toContain('opacity: var(--opacity-disabled);')
    expect(source).toContain('min-height: var(--control-height-sm);')
    expect(source).toContain('min-height: var(--control-height-md);')
  })
})
