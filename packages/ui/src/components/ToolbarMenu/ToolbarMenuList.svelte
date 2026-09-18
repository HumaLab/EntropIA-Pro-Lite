<script lang="ts">
  /**
   * A ToolbarMenu's entries: commands, toggles, choices, separators and
   * headings. ToolbarMenu draws its `items` with it, and content that has to
   * interleave entries with something else (the writing toolbar's overflow,
   * which places colour grids between them) draws more with it, passing the
   * menu's own `select` so they run and close the same way.
   */
  import ActionIcon from '../Button/ActionIcon.svelte'
  import type { ToolbarMenuEntry, ToolbarMenuItem } from './ToolbarMenu.types'

  let {
    items,
    onselect,
  }: {
    items: readonly ToolbarMenuItem[]
    onselect: (item: ToolbarMenuEntry) => void
  } = $props()

  function roleOf(item: ToolbarMenuEntry): string {
    if (item.kind === 'checkbox') return 'menuitemcheckbox'
    if (item.kind === 'radio') return 'menuitemradio'
    return 'menuitem'
  }
</script>

{#each items as item (item.id)}
  {#if item.kind === 'separator'}
    <div class="toolbar-menu__separator" role="separator"></div>
  {:else if item.kind === 'heading'}
    <div class="toolbar-menu__heading" role="presentation">{item.label}</div>
  {:else}
    {@const checkable = item.kind === 'checkbox' || item.kind === 'radio'}
    <button
      type="button"
      class="toolbar-menu__item"
      class:toolbar-menu__item--checked={checkable && item.checked}
      role={roleOf(item)}
      aria-checked={checkable ? (item.checked ? 'true' : 'false') : undefined}
      disabled={item.disabled}
      tabindex="-1"
      onclick={() => onselect(item)}
    >
      <span class="toolbar-menu__icon" aria-hidden="true">
        {#if item.icon}<ActionIcon name={item.icon} size={14} />{/if}
      </span>
      <span class="toolbar-menu__label">{item.label}</span>
      <!-- Checked is marked by a glyph as well as by contrast, so it never
           rests on colour alone. -->
      <span class="toolbar-menu__check" aria-hidden="true">
        {#if checkable && item.checked}<ActionIcon name="check" size={14} />{/if}
      </span>
    </button>
  {/if}
{/each}

<style>
  .toolbar-menu__item {
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

  .toolbar-menu__item:hover:not(:disabled),
  .toolbar-menu__item--checked {
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
  }

  .toolbar-menu__item:focus-visible {
    outline: none;
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
    box-shadow: var(--focus-ring);
  }

  .toolbar-menu__item:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .toolbar-menu__icon,
  .toolbar-menu__check {
    display: inline-flex;
    flex-shrink: 0;
    width: 14px;
  }

  .toolbar-menu__label {
    flex: 1;
  }

  .toolbar-menu__separator {
    height: 1px;
    margin: 0 var(--space-1);
    background: var(--border-subtle);
  }

  /* The colour palettes' own heading (ColorPalette.svelte), so every caption
     in a menu reads the same. */
  .toolbar-menu__heading {
    padding: var(--space-1) var(--space-2) 0;
    color: var(--color-text-muted);
    font-family: var(--font-ui);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-medium);
  }
</style>
