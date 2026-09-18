<script lang="ts">
  /**
   * A colour menu's content: "no colour", then the palette as a grid of
   * swatches (writing-colors.ts). It lives inside a ToolbarMenu, which owns
   * opening, closing, Enter, Escape and the up/down walk through its items;
   * the grid adds the two-dimensional part.
   *
   * The swatches are the annotation toolbar's: a ringed dot on a square
   * button, with the chosen one outlined in the accent. Each is named and
   * tooltipped with its colour's name, never with a native title.
   */
  import ActionIcon from '../Button/ActionIcon.svelte'
  import { tooltip } from '../Tooltip/tooltip'
  import {
    WRITING_COLORS,
    highlightColorVar,
    textColorVar,
    type WritingColor,
  } from './writing-colors'

  let {
    label,
    heading = false,
    kind,
    current,
    noColorLabel,
    colorLabel,
    onpick,
  }: {
    /** The group's accessible name; also its visible heading when `heading`. */
    label: string
    heading?: boolean
    kind: 'text' | 'highlight'
    current: WritingColor | null
    noColorLabel: string
    colorLabel: (name: WritingColor) => string
    onpick: (name: WritingColor | null) => void
  } = $props()

  const COLUMNS = 4
  const ITEM_SELECTOR = '[role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"]'

  let grid: HTMLDivElement | undefined = $state()

  const fill = (name: WritingColor) =>
    kind === 'text' ? textColorVar(name) : highlightColorVar(name)

  /**
   * Left and right step through the swatches and wrap; up and down move a row.
   * Past the top or bottom row the focus leaves the grid for the menu's item
   * before or after it, as the menu's own arrows would.
   */
  function onGridKeydown(event: KeyboardEvent) {
    if (!grid) return
    const swatches = [...grid.querySelectorAll<HTMLElement>(ITEM_SELECTOR)]
    const at = swatches.indexOf(document.activeElement as HTMLElement)
    if (at === -1) return
    const count = swatches.length

    let next: number | 'before' | 'after'
    switch (event.key) {
      case 'ArrowRight':
        next = (at + 1) % count
        break
      case 'ArrowLeft':
        next = (at - 1 + count) % count
        break
      case 'ArrowDown':
        next = at + COLUMNS < count ? at + COLUMNS : 'after'
        break
      case 'ArrowUp':
        next = at - COLUMNS >= 0 ? at - COLUMNS : 'before'
        break
      default:
        return
    }
    event.preventDefault()
    event.stopPropagation()

    if (typeof next === 'number') {
      swatches[next]?.focus()
      return
    }
    const menu = grid.closest('[role="menu"]')
    if (!menu) return
    const items = [...menu.querySelectorAll<HTMLElement>(ITEM_SELECTOR)].filter(
      (item) => !item.hasAttribute('disabled')
    )
    const edge = items.indexOf(next === 'before' ? swatches[0]! : swatches[count - 1]!)
    const step = next === 'before' ? -1 : 1
    items[(edge + step + items.length) % items.length]?.focus()
  }
</script>

<div class="color-palette" role="group" aria-label={label}>
  {#if heading}
    <div class="color-palette__heading" aria-hidden="true">{label}</div>
  {/if}
  <button
    type="button"
    class="color-palette__none"
    class:color-palette__none--checked={current === null}
    role="menuitemradio"
    aria-checked={current === null ? 'true' : 'false'}
    tabindex="-1"
    onclick={() => onpick(null)}
  >
    <span class="color-palette__fill color-palette__fill--none" aria-hidden="true"></span>
    <span class="color-palette__label">{noColorLabel}</span>
    <span class="color-palette__check" aria-hidden="true">
      {#if current === null}<ActionIcon name="check" size={14} />{/if}
    </span>
  </button>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="color-palette__grid" bind:this={grid} onkeydown={onGridKeydown}>
    {#each WRITING_COLORS as name (name)}
      <button
        type="button"
        class="color-palette__swatch"
        class:color-palette__swatch--active={current === name}
        role="menuitemradio"
        aria-checked={current === name ? 'true' : 'false'}
        aria-label={colorLabel(name)}
        use:tooltip={colorLabel(name)}
        tabindex="-1"
        onclick={() => onpick(name)}
      >
        <span class="color-palette__fill" style:background={fill(name)}></span>
      </button>
    {/each}
  </div>
</div>

<style>
  .color-palette {
    display: grid;
    gap: var(--space-1);
  }

  .color-palette__heading {
    padding: var(--space-1) var(--space-2) 0;
    color: var(--color-text-muted);
    font-family: var(--font-ui);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-medium);
  }

  /* The menu's own item, so "no colour" reads as one of its entries. */
  .color-palette__none {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-height: 28px;
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    text-align: start;
    white-space: nowrap;
    cursor: pointer;
  }

  .color-palette__none:hover,
  .color-palette__none--checked {
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
  }

  .color-palette__none:focus-visible {
    outline: none;
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
    box-shadow: var(--focus-ring);
  }

  .color-palette__label {
    flex: 1;
  }

  .color-palette__check {
    display: inline-flex;
    flex-shrink: 0;
    width: 14px;
  }

  .color-palette__grid {
    display: grid;
    grid-template-columns: repeat(4, 28px);
    gap: var(--space-1);
    padding: 0 var(--space-1) var(--space-1);
  }

  /* AnnotationToolbar's swatch, at the toolbar's 28px. */
  .color-palette__swatch {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-control);
    background: var(--color-surface);
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base);
  }

  .color-palette__swatch:hover {
    background: var(--color-surface-raised);
    border-color: var(--color-text-secondary);
  }

  .color-palette__swatch:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  /* Outlined twice over rather than tinted, so the choice never rests on
     colour alone. */
  .color-palette__swatch--active {
    border-color: var(--color-accent);
    box-shadow: inset 0 0 0 1px var(--color-accent);
  }

  .color-palette__fill {
    flex-shrink: 0;
    width: 14px;
    height: 14px;
    border-radius: var(--radius-full);
    border: 1px solid color-mix(in srgb, var(--color-text-primary) 35%, transparent);
  }

  /* An empty ring, struck through: the swatch for no colour at all. */
  .color-palette__fill--none {
    background: linear-gradient(
      135deg,
      transparent calc(50% - 0.5px),
      var(--color-text-muted) calc(50% - 0.5px),
      var(--color-text-muted) calc(50% + 0.5px),
      transparent calc(50% + 0.5px)
    );
  }
</style>
