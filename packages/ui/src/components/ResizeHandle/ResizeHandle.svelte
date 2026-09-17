<script lang="ts">
  import { widthAfterDrag, widthAfterKey, type PanelBounds, type PanelSide } from './panel-size'

  /**
   * A draggable divider between a side panel and the work beside it (§18).
   *
   * # Why it is a separator and not a button
   *
   * `role="separator"` with `aria-valuenow` is what a screen reader announces as
   * an adjustable divider, and it is what makes the arrow keys expected rather
   * than a surprise. A `<div>` with a pointer listener is invisible to anyone
   * not using a pointer, which is precisely the failure §18 opens by naming.
   *
   * # Why the hit area is wider than the line
   *
   * The divider is drawn 1px wide because a thick bar between two panels is
   * visual noise, but a 1px pointer target is a target people miss. So the
   * element is 9px of hit area with the line painted down its middle — the same
   * trick every window manager uses, for the same reason.
   *
   * # Why pointer capture
   *
   * A drag that leaves the element mid-gesture must keep going; without capture
   * the panel stops following the pointer the moment it crosses onto the
   * document, which is where it is heading by definition.
   */

  interface Props {
    /** The panel's current width in pixels. */
    width: number
    bounds: PanelBounds
    /** Which side of the work area the panel is on. */
    side: PanelSide
    /** Named for the announcement: "Ancho del esquema", and so on. */
    label: string
    /** The panel this controls, for `aria-controls`. */
    controls?: string
    /** Raised continuously while dragging and on every key press. */
    onresize: (width: number) => void
    /** Raised once the gesture ends, for persisting the result. */
    oncommit?: (width: number) => void
  }

  let { width, bounds, side, label, controls, onresize, oncommit }: Props = $props()

  let dragging = $state(false)
  // The width the gesture began at. Every frame is computed from this rather
  // than from the previous one: accumulating drifts, and a pointer that runs
  // past the edge and comes back would not return to where it left.
  let startWidth = 0
  let startX = 0

  function onpointerdown(event: PointerEvent) {
    // Primary button only. A right-click here belongs to the context menu.
    if (event.button !== 0) return
    dragging = true
    startWidth = width
    startX = event.clientX
    event.currentTarget instanceof HTMLElement &&
      event.currentTarget.setPointerCapture(event.pointerId)
    event.preventDefault()
  }

  function onpointermove(event: PointerEvent) {
    if (!dragging) return
    onresize(widthAfterDrag(startWidth, event.clientX - startX, side, bounds))
  }

  function onpointerup(event: PointerEvent) {
    if (!dragging) return
    dragging = false
    event.currentTarget instanceof HTMLElement &&
      event.currentTarget.releasePointerCapture(event.pointerId)
    oncommit?.(width)
  }

  function onkeydown(event: KeyboardEvent) {
    const next = widthAfterKey(event.key, width, side, bounds)
    // A key this does not handle goes on to whatever else was listening —
    // Tab above all, which has to keep moving focus out of here.
    if (next === null) return
    event.preventDefault()
    onresize(next)
    oncommit?.(next)
  }
</script>

<!--
  The linter reads `separator` as non-interactive, and for a plain one it is
  right: a rule between two sections takes no input. ARIA splits the role in
  two, though, and a *focusable* separator is the widget half — the window
  splitter pattern, defined by exactly the `aria-valuenow`/`min`/`max` trio
  below. Removing the tabindex to satisfy the warning would delete the keyboard
  support §18 asks for first, so the warning is silenced and the reason written
  down rather than the control being made worse to quieten it.
-->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="resize"
  class:resize--dragging={dragging}
  role="separator"
  tabindex="0"
  aria-orientation="vertical"
  aria-label={label}
  aria-valuenow={width}
  aria-valuemin={bounds.min}
  aria-valuemax={bounds.max}
  aria-controls={controls}
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  onpointercancel={onpointerup}
  {onkeydown}
></div>

<style>
  .resize {
    flex: 0 0 9px;
    align-self: stretch;
    /* The line is painted down the middle of a target people can actually hit.
       A 1px divider is the right amount of ink and the wrong size of button. */
    background: linear-gradient(
      to right,
      transparent 4px,
      var(--border-subtle) 4px,
      var(--border-subtle) 5px,
      transparent 5px
    );
    cursor: col-resize;
    touch-action: none;
  }

  .resize:hover,
  .resize--dragging {
    background: linear-gradient(
      to right,
      transparent 4px,
      var(--color-accent) 4px,
      var(--color-accent) 5px,
      transparent 5px
    );
  }

  /* §18 asks for visible focus, and this is the one control on the page whose
     whole body is a hairline: a ring around 9px of mostly-transparent gutter is
     what tells someone arriving by Tab that the arrows will now do something. */
  .resize:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: -1px;
    border-radius: var(--radius-sm);
  }

  @media (prefers-reduced-motion: no-preference) {
    .resize {
      transition: background 120ms ease;
    }
  }
</style>
