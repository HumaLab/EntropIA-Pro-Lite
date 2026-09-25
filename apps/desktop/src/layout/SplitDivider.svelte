<script lang="ts">
  import { clampSplitRatio } from '$lib/split-ratio'

  let {
    ratio,
    orientation = 'vertical',
    onratiochange,
  }: {
    ratio: number
    orientation?: 'vertical' | 'horizontal'
    onratiochange: (ratio: number) => void
  } = $props()

  let handleEl: HTMLElement | undefined = $state()
  let dragging = $state(false)

  const KEY_STEP = 0.02

  function containerSize(): number {
    const parent = handleEl?.parentElement
    if (!parent) return 0
    return orientation === 'vertical' ? parent.clientWidth : (parent as HTMLElement).clientHeight
  }

  function ratioFromPointer(clientX: number, clientY: number): number {
    const parent = handleEl?.parentElement
    if (!parent) return ratio
    const rect = parent.getBoundingClientRect()
    const position = orientation === 'vertical' ? clientX - rect.left : clientY - rect.top
    const size = orientation === 'vertical' ? rect.width : rect.height
    return clampSplitRatio(size > 0 ? position / size : ratio, size)
  }

  function handlePointerDown(event: PointerEvent) {
    dragging = true
    handleEl?.setPointerCapture?.(event.pointerId)
  }

  function handlePointerMove(event: PointerEvent) {
    if (!dragging) return
    onratiochange(ratioFromPointer(event.clientX, event.clientY))
  }

  function handlePointerUp(event: PointerEvent) {
    dragging = false
    handleEl?.releasePointerCapture?.(event.pointerId)
  }

  function handleDoubleClick() {
    onratiochange(0.5)
  }

  function handleKeydown(event: KeyboardEvent) {
    const decreaseKey = orientation === 'vertical' ? 'ArrowLeft' : 'ArrowUp'
    const increaseKey = orientation === 'vertical' ? 'ArrowRight' : 'ArrowDown'
    if (event.key === decreaseKey) {
      event.preventDefault()
      onratiochange(clampSplitRatio(ratio - KEY_STEP, containerSize()))
    } else if (event.key === increaseKey) {
      event.preventDefault()
      onratiochange(clampSplitRatio(ratio + KEY_STEP, containerSize()))
    }
  }
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
