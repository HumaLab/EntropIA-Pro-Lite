<script lang="ts" generics="T">
  /**
   * A uniform, fixed-height virtualized grid.
   *
   * It does not own the scroll container. The page it lives in already scrolls,
   * and taking that over would change how the page behaves — sticky headers,
   * scroll restoration, drag targets. Instead it watches an ancestor's scroll
   * position and renders spacers so the scrollbar still describes the whole
   * collection.
   *
   * All the arithmetic lives in ./virtual-grid.ts. This file only measures,
   * applies, and keeps focus from falling on the floor.
   */
  import { onMount, tick, type Snippet } from 'svelte'
  import {
    computeVirtualGridWindow,
    resolveColumnCount,
    resolveFocusTarget,
  } from './virtual-grid'

  let {
    items,
    getKey,
    item,
    scrollElement,
    rowHeight = 0,
    minColumnWidth = 260,
    gap = 12,
    overscanRows = 2,
    ariaLabel = undefined,
    onwindowchange = undefined,
  }: {
    items: T[]
    getKey: (item: T) => string
    /** Rendered once per visible item. */
    item: Snippet<[T, number]>
    /** The scrolling ancestor. Until it is set, everything renders. */
    scrollElement?: HTMLElement | undefined
    /** Fallback row height, used until a real card can be measured. */
    rowHeight?: number
    minColumnWidth?: number
    gap?: number
    overscanRows?: number
    ariaLabel?: string | undefined
    /** Reports the rendered window, so a caller can prioritize work by what is
     *  actually on screen rather than by what has been loaded. */
    onwindowchange?: (window: { startIndex: number; endIndex: number; centerRow: number }) => void
  } = $props()

  let containerEl: HTMLDivElement | undefined = $state()
  let measuredRowHeight = $state(0)
  let containerWidth = $state(0)
  let scrollTop = $state(0)
  let viewportHeight = $state(0)
  let gridOffset = $state(0)

  let focusedKey: string | null = null
  let renderedKeys: string[] = []

  const columns = $derived(
    resolveColumnCount({ containerWidth, minColumnWidth, gap })
  )

  const effectiveRowHeight = $derived(measuredRowHeight || rowHeight)

  const windowRange = $derived(
    computeVirtualGridWindow({
      scrollTop,
      gridOffset,
      rowHeight: effectiveRowHeight,
      columns,
      totalItems: items.length,
      overscanRows,
      viewportHeight,
    })
  )

  const visible = $derived(
    items.slice(windowRange.startIndex, windowRange.endIndex).map((entry, offset) => ({
      entry,
      index: windowRange.startIndex + offset,
      key: getKey(entry),
    }))
  )

  function measure() {
    if (!containerEl) return

    containerWidth = containerEl.clientWidth
    gridOffset = containerEl.offsetTop

    const scroller = scrollElement
    if (scroller) {
      scrollTop = scroller.scrollTop
      viewportHeight = scroller.clientHeight
    }

    // Measure a real card rather than trusting the caller's constant: card
    // height changes with the theme's spacing scale and with font size.
    const firstCard = containerEl.querySelector('[data-virtual-key]') as HTMLElement | null
    if (firstCard && firstCard.offsetHeight > 0) {
      measuredRowHeight = firstCard.offsetHeight + gap
    }
  }

  /**
   * Move focus somewhere sensible when the window changes.
   *
   * Unmounting the focused element drops focus to `<body>`, which silently
   * throws away the user's place in the grid. Handing it to the nearest
   * surviving card keeps keyboard navigation continuous.
   */
  async function preserveFocus(nextKeys: string[]) {
    const previousKeys = renderedKeys
    renderedKeys = nextKeys

    if (!focusedKey || !containerEl) return

    // By the time this runs the DOM has already updated, so a focused card that
    // was just unmounted has dropped focus to <body>. That is precisely the
    // case worth rescuing, so it counts as "focus was in the grid".
    const active = document.activeElement
    const focusWasHere = !active || active === document.body || containerEl.contains(active)
    if (!focusWasHere) return

    const target = resolveFocusTarget({ focusedKey, previousKeys, nextKeys })
    if (target.kind === 'keep') return

    await tick()

    if (target.kind === 'handoff') {
      const card = containerEl.querySelector(`[data-virtual-key="${CSS.escape(target.key)}"]`)
      const focusable = card?.querySelector<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
      )
      const next = focusable ?? (card as HTMLElement | null)
      if (next) {
        focusedKey = target.key
        next.focus()
        return
      }
    }

    focusedKey = null
    containerEl.focus()
  }

  function handleFocusIn(event: FocusEvent) {
    const card = (event.target as HTMLElement | null)?.closest('[data-virtual-key]')
    focusedKey = (card as HTMLElement | null)?.dataset.virtualKey ?? null
  }

  function handleFocusOut(event: FocusEvent) {
    // Focus genuinely leaving for somewhere else means the grid no longer owns
    // it. A removal reports no relatedTarget, and that one we do want to catch.
    const next = event.relatedTarget as HTMLElement | null
    if (next && !containerEl?.contains(next)) focusedKey = null
  }

  /** Re-read geometry. The scrolling ancestor calls this from its own handler
   *  so there is a single scroll listener on the page, not one per grid. */
  export function refresh() {
    measure()
  }

  /**
   * Put focus on one card, or on the grid itself when that card is gone.
   *
   * Deleting from a dialog moves focus out of the grid first, so the grid
   * cannot infer where focus should land afterwards. The caller knows which
   * neighbour it wants and says so.
   */
  export function focusCard(key: string | null) {
    if (!containerEl) return

    const card = key
      ? containerEl.querySelector(`[data-virtual-key="${CSS.escape(key)}"]`)
      : null
    const focusable = card?.querySelector<HTMLElement>(
      'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
    )
    const next = focusable ?? (card as HTMLElement | null)

    if (next) {
      focusedKey = key
      next.focus()
      return
    }

    focusedKey = null
    containerEl.focus()
  }

  $effect(() => {
    const keys = visible.map((row) => row.key)
    void preserveFocus(keys)

    onwindowchange?.({
      startIndex: windowRange.startIndex,
      endIndex: windowRange.endIndex,
      centerRow: Math.floor((windowRange.startIndex + windowRange.endIndex) / 2),
    })
  })

  onMount(() => {
    measure()

    const scroller = scrollElement
    const onScroll = () => measure()
    scroller?.addEventListener('scroll', onScroll, { passive: true })

    // A width change re-flows the columns, which changes the window as surely
    // as scrolling does.
    const observer =
      typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => measure())
    if (containerEl) observer?.observe(containerEl)
    if (scroller) observer?.observe(scroller)

    return () => {
      scroller?.removeEventListener('scroll', onScroll)
      observer?.disconnect()
    }
  })
</script>

<div
  bind:this={containerEl}
  class="virtual-grid"
  role="group"
  tabindex="-1"
  aria-label={ariaLabel}
  onfocusin={handleFocusIn}
  onfocusout={handleFocusOut}
>
  <div class="virtual-grid__spacer" style="height: {windowRange.beforeHeight}px" aria-hidden="true"></div>

  <div
    class="virtual-grid__window"
    style="grid-template-columns: repeat({columns}, minmax(0, 1fr)); gap: {gap}px"
  >
    {#each visible as row (row.key)}
      <div class="virtual-grid__cell" data-virtual-key={row.key}>
        {@render item(row.entry, row.index)}
      </div>
    {/each}
  </div>

  <div class="virtual-grid__spacer" style="height: {windowRange.afterHeight}px" aria-hidden="true"></div>
</div>

<style>
  .virtual-grid {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .virtual-grid:focus {
    outline: none;
  }

  .virtual-grid__window {
    display: grid;
  }

  /* The spacers carry the height of the rows that are not rendered, so the
     scrollbar keeps describing the whole collection rather than the window. */
  .virtual-grid__spacer {
    flex: none;
  }

  .virtual-grid__cell {
    min-width: 0;
  }
</style>
