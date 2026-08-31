import { render } from '@testing-library/svelte'
import { describe, expect, it } from 'vitest'
import ActionIcon from '../ActionIcon.svelte'
import { ACTION_ICON_NAMES, ACTION_ICON_SIZES } from '../ActionIcon.types'

describe('ActionIcon', () => {
  it('renders an svg for every name in the catalogue', () => {
    const missing: string[] = []

    for (const name of ACTION_ICON_NAMES) {
      const { container, unmount } = render(ActionIcon, { props: { name } })
      if (container.querySelector('svg') === null) missing.push(name)
      unmount()
    }

    expect(missing).toEqual([])
  })

  it('hides every icon from the accessibility tree', () => {
    const exposed: string[] = []

    for (const name of ACTION_ICON_NAMES) {
      const { container, unmount } = render(ActionIcon, { props: { name } })
      const svg = container.querySelector('svg')
      if (svg?.getAttribute('aria-hidden') !== 'true') exposed.push(name)
      unmount()
    }

    expect(exposed).toEqual([])
  })

  it('draws every icon at the canonical stroke width', () => {
    const offContract: string[] = []

    for (const name of ACTION_ICON_NAMES) {
      const { container, unmount } = render(ActionIcon, { props: { name } })
      const stroke = container.querySelector('svg')?.getAttribute('stroke-width')
      if (stroke !== null && stroke !== '2') offContract.push(`${name}=${stroke}`)
      unmount()
    }

    expect(offContract).toEqual([])
  })

  it('renders the notification bell that NotificationBell draws by hand today', () => {
    const { container } = render(ActionIcon, { props: { name: 'bell' } })

    expect(container.querySelector('svg')).not.toBeNull()
  })

  it('applies the requested size to both axes', () => {
    const { container } = render(ActionIcon, { props: { name: 'close', size: 20 } })
    const svg = container.querySelector('svg')

    expect(svg).toHaveAttribute('width', '20')
    expect(svg).toHaveAttribute('height', '20')
  })

  it('defaults to the md step when no size is given', () => {
    const { container } = render(ActionIcon, { props: { name: 'close' } })

    expect(container.querySelector('svg')).toHaveAttribute('width', '16')
  })

  it('exposes a closed size scale of even steps only', () => {
    expect(ACTION_ICON_SIZES).toEqual([12, 14, 16, 20, 24, 40])
    expect(ACTION_ICON_SIZES.every((size) => size % 2 === 0)).toBe(true)
  })

  it('no longer exposes the pencil alias of edit', () => {
    expect(ACTION_ICON_NAMES).not.toContain('pencil')
    expect(ACTION_ICON_NAMES).toContain('edit')
  })
})
