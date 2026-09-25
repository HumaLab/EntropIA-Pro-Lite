<script lang="ts">
  import { onDestroy } from 'svelte'
  import { clampSplitRatio } from '$lib/split-ratio'

  let {
    ratio,
    orientation = 'vertical',
    onratiochange,
    onratiocommit,
  }: {
    ratio: number
    orientation?: 'vertical' | 'horizontal'
    /** Every live change, including each pointermove of a drag. */
    onratiochange: (ratio: number) => void
    /** The settled value of a gesture: a drag's end, a key step, a reset. */
    onratiocommit?: (ratio: number) => void
  } = $props()

  let handleEl: HTMLElement | undefined = $state()
  let dragging = $state(false)
  // The last ratio a drag reported, committed once when the drag ends; null
  // while no drag has moved, so a press without a move commits nothing.
  let dragRatio: number | null = null

  const KEY_STEP = 0.02

  /**
   * The parent's box along the active axis, as both a size and an origin.
   * Keyboard resize only needs the size (matching `clampSplitRatio`'s
   * `containerSize`); pointer drag also needs the origin to turn a client
   * coordinate into a 0..1 position. One measurement backs both so they
   * cannot drift apart — `clientWidth` and `getBoundingClientRect().width`
   * can disagree by a scrollbar or a fractional-pixel rounding.
   */
  function parentBox(): { origin: number; size: number } {
    const parent = handleEl?.parentElement
    if (!parent) return { origin: 0, size: 0 }
    const rect = parent.getBoundingClientRect()
    return orientation === 'vertical'
      ? { origin: rect.left, size: rect.width }
      : { origin: rect.top, size: rect.height }
  }

  function ratioFromPointer(clientX: number, clientY: number): number {
    const { origin, size } = parentBox()
    const position = orientation === 'vertical' ? clientX - origin : clientY - origin
    return clampSplitRatio(size > 0 ? position / size : ratio, size)
  }

  function endDrag() {
    dragging = false
    document.removeEventListener('pointerup', endDrag)
    document.removeEventListener('pointercancel', endDrag)
    if (dragRatio !== null) {
      const settled = dragRatio
      dragRatio = null
      onratiocommit?.(settled)
    }
  }

  function change(next: number) {
    onratiochange(next)
    onratiocommit?.(next)
  }

  function handlePointerDown(event: PointerEvent) {
    dragging = true
    handleEl?.setPointerCapture?.(event.pointerId)
    // Pointer capture keeps `pointermove`/`pointerup` targeted at the handle
    // even once the pointer leaves it, but a system-level cancellation (a
    // touch gesture taken over by the OS, a pen lifted off the digitizer)
    // does not always reach an element that no longer exists — e.g. split
    // view toggled off mid-drag, which unmounts this component. A document
    // listener guarantees the drag still ends and is removed on destroy so
    // it never outlives the component.
    document.addEventListener('pointerup', endDrag)
    document.addEventListener('pointercancel', endDrag)
  }

  function handlePointerMove(event: PointerEvent) {
    if (!dragging) return
    dragRatio = ratioFromPointer(event.clientX, event.clientY)
    onratiochange(dragRatio)
  }

  function handlePointerUp(event: PointerEvent) {
    if (!dragging) return
    endDrag()
    handleEl?.releasePointerCapture?.(event.pointerId)
  }

  function handleDoubleClick() {
    change(0.5)
  }

  function handleKeydown(event: KeyboardEvent) {
    const decreaseKey = orientation === 'vertical' ? 'ArrowLeft' : 'ArrowUp'
    const increaseKey = orientation === 'vertical' ? 'ArrowRight' : 'ArrowDown'
    const { size } = parentBox()
    if (event.key === decreaseKey) {
      event.preventDefault()
      change(clampSplitRatio(ratio - KEY_STEP, size))
    } else if (event.key === increaseKey) {
      event.preventDefault()
      change(clampSplitRatio(ratio + KEY_STEP, size))
    }
  }

  onDestroy(endDrag)
</script>

<!--
  The linter reads `separator` as non-interactive, and for a plain one it is
  right: a rule between two sections takes no input. ARIA splits the role in
  two, though, and a *focusable* separator is the widget half — the window
  splitter pattern, defined by exactly the `aria-valuenow`/`min`/`max` trio
  below. Removing the tabindex to satisfy the warning would delete the
  keyboard resize the spec asks for, so the warning is silenced and the
  reason written down rather than the control being made worse to quieten it.
-->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  bind:this={handleEl}
  class="split-divider"
  class:split-divider--horizontal={orientation === 'horizontal'}
  role="separator"
  aria-orientation={orientation}
  aria-valuenow={Math.round(ratio * 100)}
  aria-valuemin={0}
  aria-valuemax={100}
  tabindex="0"
  onpointerdown={handlePointerDown}
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
  onpointercancel={handlePointerUp}
  ondblclick={handleDoubleClick}
  onkeydown={handleKeydown}
></div>

<style>
  .split-divider {
    flex: 0 0 6px;
    cursor: col-resize;
    background: var(--border-subtle);
    touch-action: none;
  }

  .split-divider--horizontal {
    cursor: row-resize;
  }

  .split-divider:hover,
  .split-divider:focus-visible {
    background: var(--color-accent);
    outline: none;
  }
</style>
