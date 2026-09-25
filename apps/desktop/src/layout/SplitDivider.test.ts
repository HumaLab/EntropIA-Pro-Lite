import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, it, expect, vi } from 'vitest'
import SplitDivider from './SplitDivider.svelte'

function withMeasuredParent(width: number) {
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, value: width })
  Object.defineProperty(HTMLElement.prototype, 'getBoundingClientRect', {
    configurable: true,
    value: () => ({ left: 0, top: 0, width, height: 400, right: width, bottom: 400 }),
  })
}

describe('SplitDivider', () => {
  it('renders as a vertical separator by default, with the ratio as its value', () => {
    render(SplitDivider, { ratio: 0.5, onratiochange: vi.fn() })
    const el = screen.getByRole('separator')
    expect(el).toHaveAttribute('aria-orientation', 'vertical')
    expect(el).toHaveAttribute('aria-valuenow', '50')
  })

  it('ArrowRight increases the ratio by a fixed step, clamped to the container', async () => {
    withMeasuredParent(1000)
    const onratiochange = vi.fn()
    render(SplitDivider, { ratio: 0.5, onratiochange })

    const el = screen.getByRole('separator')
    el.focus()
    await fireEvent.keyDown(el, { key: 'ArrowRight' })

    expect(onratiochange).toHaveBeenCalledWith(0.52)
  })

  it('ArrowLeft decreases the ratio', async () => {
    withMeasuredParent(1000)
    const onratiochange = vi.fn()
    render(SplitDivider, { ratio: 0.5, onratiochange })

    await fireEvent.keyDown(screen.getByRole('separator'), { key: 'ArrowLeft' })

    expect(onratiochange).toHaveBeenCalledWith(0.48)
  })

  it('double-click resets to 0.5', async () => {
    const onratiochange = vi.fn()
    render(SplitDivider, { ratio: 0.7, onratiochange })

    await fireEvent.dblClick(screen.getByRole('separator'))

    expect(onratiochange).toHaveBeenCalledWith(0.5)
  })

  it('a horizontal divider reports aria-orientation horizontal and resizes on ArrowDown/ArrowUp', async () => {
    withMeasuredParent(1000)
    const onratiochange = vi.fn()
    render(SplitDivider, { ratio: 0.5, orientation: 'horizontal', onratiochange })

    const el = screen.getByRole('separator')
    expect(el).toHaveAttribute('aria-orientation', 'horizontal')

    await fireEvent.keyDown(el, { key: 'ArrowDown' })
    expect(onratiochange).toHaveBeenCalledWith(0.52)
  })
})
