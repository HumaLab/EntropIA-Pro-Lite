import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { get } from 'svelte/store'
import { TOOLTIP_ID, tooltip, tooltipState } from '../tooltip'

/**
 * The action is plain DOM work, so it is exercised on a plain element rather
 * than through a host component: what is under test is the contract every
 * trigger in the application relies on.
 */
function trigger(text?: string | null, attrs: Record<string, string> = {}) {
  const node = document.createElement('button')
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v)
  document.body.append(node)
  return { node, handle: tooltip(node, text) }
}

beforeEach(() => {
  vi.useFakeTimers()
  tooltipState.set(null)
})

afterEach(() => {
  vi.useRealTimers()
  document.body.innerHTML = ''
})

describe('the tooltip action', () => {
  it('marks the trigger with its label before anything is hovered', () => {
    // The trace is what a test and the repository guard can see; the bubble
    // itself exists only while a pointer is inside the element.
    const { node } = trigger('Exportar JSON')

    expect(node.getAttribute('data-tooltip')).toBe('Exportar JSON')
    expect(get(tooltipState)).toBeNull()
  })

  it('waits before showing, so sweeping a toolbar does not strobe every label', () => {
    const { node } = trigger('Buscar')

    node.dispatchEvent(new Event('pointerenter'))
    expect(get(tooltipState)).toBeNull()

    vi.advanceTimersByTime(300)
    expect(get(tooltipState)?.text).toBe('Buscar')
  })

  it('never shows the one that was left before the delay elapsed', () => {
    const { node } = trigger('Buscar')

    node.dispatchEvent(new Event('pointerenter'))
    node.dispatchEvent(new Event('pointerleave'))
    vi.advanceTimersByTime(300)

    expect(get(tooltipState)).toBeNull()
  })

  it('describes the control when the label says more than its name', () => {
    const { node } = trigger('Buscar evidencia en contra', { 'aria-label': 'Contraevidencia' })

    node.dispatchEvent(new Event('pointerenter'))
    vi.advanceTimersByTime(300)

    expect(node.getAttribute('aria-describedby')).toBe(TOOLTIP_ID)
  })

  it('stays silent to a screen reader when it only repeats the name', () => {
    // Most icon buttons pass the same string as label and as tooltip. Pointing
    // aria-describedby at a bubble that repeats the accessible name makes the
    // control announce itself twice; the bubble still appears.
    const { node } = trigger('Buscar', { 'aria-label': 'Buscar' })

    node.dispatchEvent(new Event('pointerenter'))
    vi.advanceTimersByTime(300)

    expect([get(tooltipState)?.text, node.getAttribute('aria-describedby')]).toEqual([
      'Buscar',
      null,
    ])
  })

  it('lets go on pointer leave, on a press, and on Escape', () => {
    for (const leave of [
      new Event('pointerleave'),
      new Event('pointerdown'),
      new KeyboardEvent('keydown', { key: 'Escape' }),
    ]) {
      const { node, handle } = trigger('Guardar nota', { 'aria-label': 'Guardar' })
      node.dispatchEvent(new Event('pointerenter'))
      vi.advanceTimersByTime(300)
      expect(get(tooltipState)).not.toBeNull()

      node.dispatchEvent(leave)

      expect([get(tooltipState), node.getAttribute('aria-describedby')]).toEqual([null, null])
      handle.destroy()
    }
  })

  it('follows a control that relabels itself while its tooltip is up', () => {
    // A rotation button whose label carries the current angle, a play/pause —
    // a stale bubble is worse than none.
    const { node, handle } = trigger('Rotar +1°', { 'aria-label': 'Rotar' })
    node.dispatchEvent(new Event('pointerenter'))
    vi.advanceTimersByTime(300)

    handle.update?.('Rotar +2°')

    expect([get(tooltipState)?.text, node.getAttribute('data-tooltip')]).toEqual([
      'Rotar +2°',
      'Rotar +2°',
    ])
  })

  it('attaches nothing when there is nothing to say', () => {
    // Callers forward an optional prop; an action with no text should not need
    // an {#if} around its element.
    const { node } = trigger(undefined)

    node.dispatchEvent(new Event('pointerenter'))
    vi.advanceTimersByTime(300)

    expect([node.hasAttribute('data-tooltip'), get(tooltipState)]).toEqual([false, null])
  })

  it('takes its bubble and its trace with it when the element goes', () => {
    const { node, handle } = trigger('Cerrar')
    node.dispatchEvent(new Event('pointerenter'))
    vi.advanceTimersByTime(300)

    handle.destroy()

    expect([get(tooltipState), node.hasAttribute('data-tooltip')]).toEqual([null, false])
  })
})
