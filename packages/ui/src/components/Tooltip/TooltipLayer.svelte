<script lang="ts">
  /**
   * The single tooltip bubble, mounted once at the shell root.
   *
   * Rendered here rather than beside each trigger so it escapes every
   * `overflow: hidden` panel in the application and needs exactly one z-index.
   */
  import { TOOLTIP_ID, tooltipState, type TooltipAnchor } from './tooltip'

  /** Breathing room between the bubble and the control it describes. */
  const OFFSET = 8
  /** Kept off the window edge by this much when a long label has to slide. */
  const EDGE = 8

  let bubble = $state<HTMLDivElement | null>(null)
  let left = $state(0)
  let top = $state(0)
  let below = $state(false)

  /**
   * Placed above the trigger, flipped below when there is no room up there, and
   * slid sideways rather than allowed off-screen. Measured after the text is in
   * the DOM, because the width of the label is what decides all three.
   */
  function place(anchor: TooltipAnchor, box: DOMRect) {
    const wantedTop = anchor.top - box.height - OFFSET
    below = wantedTop < EDGE
    top = below ? anchor.top + anchor.height + OFFSET : wantedTop

    const centred = anchor.left + anchor.width / 2 - box.width / 2
    const rightmost = window.innerWidth - box.width - EDGE
    left = Math.max(EDGE, Math.min(centred, rightmost))
  }

  $effect(() => {
    const state = $tooltipState
    if (!state || !bubble) return
    place(state.anchor, bubble.getBoundingClientRect())
  })
</script>

{#if $tooltipState}
  <div
    bind:this={bubble}
    id={TOOLTIP_ID}
    class="tooltip"
    class:tooltip--below={below}
    role="tooltip"
    style="left: {left}px; top: {top}px;"
  >
    {$tooltipState.text}
  </div>
{/if}

<style>
  .tooltip {
    position: fixed;
    /* Above every menu, panel and dialog in the app; the highest z-index in use
       elsewhere is 1200. A tooltip that a dropdown can cover is not a tooltip. */
    z-index: 1300;
    max-width: 280px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    background: var(--surface-toolbar);
    color: var(--color-text-secondary);
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-normal);
    line-height: var(--line-height-tight);
    /* The label is never the target — pointer events on it would mean leaving
       the trigger, which hides it, which puts the pointer back. */
    pointer-events: none;
    box-shadow: var(--shadow-sm);
    animation: tooltip-in var(--transition-base) ease-out;
  }

  @keyframes tooltip-in {
    from {
      opacity: 0;
      transform: translateY(2px);
    }
  }

  .tooltip--below {
    animation-name: tooltip-in-below;
  }

  @keyframes tooltip-in-below {
    from {
      opacity: 0;
      transform: translateY(-2px);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .tooltip {
      animation: none;
    }
  }
</style>
