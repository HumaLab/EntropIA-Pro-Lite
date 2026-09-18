/**
 * The one tooltip in EntropIA.
 *
 * # Why an action and a single layer, rather than a component per trigger
 *
 * A `<Tooltip>` wrapper would have to wrap its trigger, and a wrapper is a box:
 * it lands in flex rows and grid cells that were measured without it. An action
 * attaches to the element that already exists and changes no layout at all.
 *
 * And there is exactly ONE floating element for the whole application, rendered
 * by `TooltipLayer` at the shell root. That is what keeps a tooltip out of the
 * `overflow: hidden` panel its trigger lives in, and what makes one z-index
 * enough. Two tooltips are never visible at once, so one element is all there
 * ever needs to be — which is also why `aria-describedby` can point at a fixed
 * id rather than a generated one.
 */
import { writable } from 'svelte/store'

/** The id `TooltipLayer` puts on the bubble and the action points at. */
export const TOOLTIP_ID = 'entropia-tooltip'

/** Where the trigger is, in viewport coordinates. */
export interface TooltipAnchor {
  top: number
  left: number
  width: number
  height: number
}

export interface TooltipState {
  text: string
  anchor: TooltipAnchor
}

export const tooltipState = writable<TooltipState | null>(null)

/**
 * Long enough that sweeping a pointer across a toolbar does not strobe every
 * label on the way, short enough to feel like an answer rather than a wait.
 */
const HOVER_DELAY_MS = 260

function anchorOf(node: HTMLElement): TooltipAnchor {
  const { top, left, width, height } = node.getBoundingClientRect()
  return { top, left, width, height }
}

/**
 * `use:tooltip={text}` — show `text` while the element is hovered or has
 * keyboard focus.
 *
 * Pass `undefined` to attach nothing: callers commonly forward an optional
 * prop, and an action that only sometimes has something to say should not need
 * an `{#if}` around its element.
 */
export function tooltip(node: HTMLElement, text?: string | null) {
  let current = text ?? ''
  let owned = false
  let timer: ReturnType<typeof setTimeout> | undefined

  /**
   * The label, left on the element whether or not it is showing.
   *
   * Two jobs. It is what a test can assert — a tooltip that exists only while a
   * pointer is inside the element is otherwise invisible to one. And it is what
   * the repository guard greps for, to tell a deliberate tooltip apart from a
   * stray `title` someone reached for out of habit.
   */
  function trace() {
    if (current) node.setAttribute('data-tooltip', current)
    else node.removeAttribute('data-tooltip')
  }

  trace()

  /**
   * Whether the bubble tells a screen reader anything its accessible name does
   * not already say.
   *
   * Most icon buttons pass the same string as `label` and as the tooltip — the
   * label IS the description. Pointing `aria-describedby` at a bubble that
   * repeats the accessible name makes the control announce itself twice. The
   * bubble still appears; it simply stops being narrated as extra information.
   */
  function addsSomething(): boolean {
    const name = (node.getAttribute('aria-label') ?? node.textContent ?? '').trim().toLowerCase()
    return name !== current.trim().toLowerCase()
  }

  function show() {
    if (!current) return
    owned = true
    tooltipState.set({ text: current, anchor: anchorOf(node) })
    if (addsSomething()) node.setAttribute('aria-describedby', TOOLTIP_ID)
  }

  function hide() {
    if (timer) clearTimeout(timer)
    timer = undefined
    node.removeAttribute('aria-describedby')
    if (owned) tooltipState.set(null)
    owned = false
  }

  function showAfterDelay() {
    if (!current || timer) return
    timer = setTimeout(() => {
      timer = undefined
      show()
    }, HOVER_DELAY_MS)
  }

  /**
   * Keyboard focus shows it at once — someone tabbing has already committed to
   * this control. A pointer click that moves focus here has not, and
   * `:focus-visible` is the browser's own answer to "did they arrive by
   * keyboard", so it is asked rather than guessed at.
   */
  function onFocus() {
    if (node.matches(':focus-visible')) show()
  }

  /** A press has its own answer; the label stops being the interesting thing. */
  function onPointerDown() {
    hide()
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.key === 'Escape') hide()
  }

  node.addEventListener('pointerenter', showAfterDelay)
  node.addEventListener('pointerleave', hide)
  node.addEventListener('pointerdown', onPointerDown)
  node.addEventListener('focusin', onFocus)
  node.addEventListener('focusout', hide)
  node.addEventListener('keydown', onKeyDown)

  return {
    update(next?: string | null) {
      current = next ?? ''
      trace()
      // A control that relabels itself while its tooltip is up — a toggle, a
      // play/pause — should follow or go, never sit there stale.
      if (!owned) return
      if (current) show()
      else hide()
    },
    destroy() {
      hide()
      node.removeAttribute('data-tooltip')
      node.removeEventListener('pointerenter', showAfterDelay)
      node.removeEventListener('pointerleave', hide)
      node.removeEventListener('pointerdown', onPointerDown)
      node.removeEventListener('focusin', onFocus)
      node.removeEventListener('focusout', hide)
      node.removeEventListener('keydown', onKeyDown)
    },
  }
}
