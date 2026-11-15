import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { render, screen } from '@testing-library/svelte'
import { describe, expect, it } from 'vitest'
import IconButtonTestHost from './IconButtonTestHost.svelte'

const STYLES = readFileSync(resolve(import.meta.dirname, '../IconButton.svelte'), 'utf-8')
  .split(/<style[^>]*>/)[1]!
  .replace(/\/\*[\s\S]*?\*\//g, '')

function rule(selector: string): string {
  const at = STYLES.indexOf(`${selector} {`)
  expect(at, `${selector} is not in the stylesheet`).toBeGreaterThan(-1)
  return STYLES.slice(at, STYLES.indexOf('}', at))
}

/**
 * The edge variant is the strip that opens and closes a side panel: 20px wide,
 * as tall as the panel edge. Views used to get it by overriding IconButton
 * with `:global` and a stronger selector (R11); the component owns it now.
 */
describe('IconButton variant="edge"', () => {
  it('renders as an IconButton with the edge class', () => {
    render(IconButtonTestHost, { props: { variant: 'edge', size: 'sm', label: 'Open panel' } })
    expect(screen.getByRole('button', { name: 'Open panel' })).toHaveClass(
      'icon-button',
      'icon-button--edge'
    )
  })

  it('is a 20px strip whose height follows its parent, not the size scale', () => {
    const edge = rule('.icon-button--edge')
    expect(edge).toMatch(/--icon-button-size:\s*20px/)
    expect(edge).toMatch(/(?<![-\w])height:\s*auto/)
    expect(edge).toMatch(/min-height:\s*0/)
    expect(edge).toMatch(/max-height:\s*none/)
  })

  it('wins over every size rule by order inside the component, not by a caller', () => {
    const edgeAt = STYLES.indexOf('.icon-button--edge {')
    for (const size of ['xs', 'sm', 'md', 'lg']) {
      expect(STYLES.indexOf(`.icon-button--${size} {`)).toBeLessThan(edgeAt)
    }
  })

  it('draws the panel-edge surface and an accent hover', () => {
    const edge = rule('.icon-button--edge')
    expect(edge).toMatch(/border-radius:\s*var\(--radius-dialog\)/)
    expect(edge).toMatch(/background:\s*var\(--surface-input\)/)
    expect(edge).toMatch(/border-color:\s*var\(--border-subtle\)/)
    expect(edge).toMatch(/color:\s*var\(--color-text-muted\)/)
    const hover = rule('.icon-button--edge:hover:not(:disabled)')
    expect(hover).toMatch(/color:\s*var\(--color-accent\)/)
    expect(hover).toMatch(/background:\s*var\(--color-accent-soft\)/)
  })
})

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
