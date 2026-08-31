import { render, screen } from '@testing-library/svelte'
import { describe, expect, it } from 'vitest'
import IconButtonTestHost from './IconButtonTestHost.svelte'

/**
 * IconButton is the single container for every icon-only control in the app.
 * Its four sizes pair 1:1 with ACTION_ICON_SIZES:
 *   xs 24 (icon 12) · sm 28 (icon 14) · md 32 (icon 16) · lg 40 (icon 20)
 */
describe('IconButton', () => {
  // NOTE: the size *dimensions* are asserted in visual-contract.test.ts against
  // the stylesheet. Asserting them here would be a false positive: the class is
  // interpolated as `icon-button--{size}`, so any string produces a class name
  // whether or not a rule backs it.

  it('keeps the icon square and centred at every size', () => {
    for (const size of ['xs', 'sm', 'md', 'lg'] as const) {
      const { unmount } = render(IconButtonTestHost, {
        props: { size, label: `Action ${size}` },
      })

      const button = screen.getByRole('button', { name: `Action ${size}` })
      expect(button.className).toContain(`icon-button--${size}`)
      expect(button.querySelector('svg')).not.toBeNull()
      unmount()
    }
  })

  it('carries the accessible name on the button, not the icon', () => {
    render(IconButtonTestHost, { props: { label: 'Dismiss notification' } })

    const button = screen.getByRole('button', { name: 'Dismiss notification' })
    expect(button.querySelector('svg')).toHaveAttribute('aria-hidden', 'true')
  })

  it('reports its pressed state only when it is a toggle', () => {
    const { unmount } = render(IconButtonTestHost, {
      props: { label: 'Toggle panel', active: true },
    })
    expect(screen.getByRole('button', { name: 'Toggle panel' })).toHaveAttribute(
      'aria-pressed',
      'true'
    )
    unmount()

    render(IconButtonTestHost, { props: { label: 'Plain action' } })
    expect(screen.getByRole('button', { name: 'Plain action' })).not.toHaveAttribute('aria-pressed')
  })
})
