import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, it, expect, vi } from 'vitest'
import SplitDivider from './SplitDivider.svelte'

// `height` defaults to `width` so a horizontal-orientation test gets a
// real, two-pane-fitting measurement on its own axis instead of silently
// falling through to the coarse [0.4, 0.6] fallback for an unmeasured
// container — the same fallback that made an earlier, narrower mock (a
// fixed 400px height, regardless of the requested width) pass by
// coincidence rather than by exercising real clamp geometry.
function withMeasuredParent(width: number, height: number = width) {
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, value: width })
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
    configurable: true,
    value: height,
  })
  Object.defineProperty(HTMLElement.prototype, 'getBoundingClientRect', {
    configurable: true,
    value: () => ({ left: 0, top: 0, width, height, right: width, bottom: height }),
  })
}

describe('SplitDivider', () => {
  it('renders as a vertical separator, with the ratio as its value', () => {
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

  it('a cancelled pointer (touch gesture interruption, pen lift) stops the drag: a later pointermove is a no-op', async () => {
    withMeasuredParent(1000)
    const onratiochange = vi.fn()
    render(SplitDivider, { ratio: 0.5, onratiochange })

    const el = screen.getByRole('separator')
    await fireEvent.pointerDown(el, { pointerId: 1, clientX: 500, clientY: 200 })
    await fireEvent.pointerCancel(el, { pointerId: 1 })
    await fireEvent.pointerMove(el, { pointerId: 1, clientX: 700, clientY: 200 })

    expect(onratiochange).not.toHaveBeenCalled()
  })
  // Final review item 7: the ratio is persisted once per gesture, not once
  // per pointermove. Live moves only report; the end of a gesture commits.
  describe('committing the ratio', () => {
    it('reports every pointermove live but commits only once, when the drag ends', async () => {
      withMeasuredParent(2000)
      const onratiochange = vi.fn()
      const onratiocommit = vi.fn()
      render(SplitDivider, { ratio: 0.5, onratiochange, onratiocommit })

      const el = screen.getByRole('separator')
      await fireEvent.pointerDown(el, { pointerId: 1, clientX: 1000, clientY: 200 })
      await fireEvent.pointerMove(el, { pointerId: 1, clientX: 1080, clientY: 200 })
      await fireEvent.pointerMove(el, { pointerId: 1, clientX: 1160, clientY: 200 })

      expect(onratiochange).toHaveBeenCalledTimes(2)
      expect(onratiocommit).not.toHaveBeenCalled()

      await fireEvent.pointerUp(el, { pointerId: 1, clientX: 1160, clientY: 200 })

      expect(onratiocommit).toHaveBeenCalledTimes(1)
      expect(onratiocommit).toHaveBeenCalledWith(0.58)
    })

    it('commits nothing for a press that never moved', async () => {
      withMeasuredParent(1000)
      const onratiocommit = vi.fn()
      render(SplitDivider, { ratio: 0.5, onratiochange: vi.fn(), onratiocommit })

      const el = screen.getByRole('separator')
      await fireEvent.pointerDown(el, { pointerId: 1, clientX: 500, clientY: 200 })
      await fireEvent.pointerUp(el, { pointerId: 1, clientX: 500, clientY: 200 })

      expect(onratiocommit).not.toHaveBeenCalled()
    })

    it('commits each keyboard step and the double-click reset', async () => {
      withMeasuredParent(1000)
      const onratiocommit = vi.fn()
      render(SplitDivider, { ratio: 0.5, onratiochange: vi.fn(), onratiocommit })

      const el = screen.getByRole('separator')
      await fireEvent.keyDown(el, { key: 'ArrowRight' })
      expect(onratiocommit).toHaveBeenLastCalledWith(0.52)

      await fireEvent.dblClick(el)
      expect(onratiocommit).toHaveBeenLastCalledWith(0.5)
    })
  })
})
