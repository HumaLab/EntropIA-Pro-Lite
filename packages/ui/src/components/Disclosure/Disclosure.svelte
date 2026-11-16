<script lang="ts">
  /**
   * A header button that shows and hides the region under it. Replaces raw
   * details/summary, whose native marker, focus outline and typography differ
   * in every view (finding N11).
   */
  import ActionIcon from '../Button/ActionIcon.svelte'
  import type { DisclosureProps } from './Disclosure.types'

  let {
    open = $bindable(false),
    summary,
    meta,
    children,
    variant = 'plain',
    size = 'md',
    disabled = false,
    ontoggle,
    class: className,
  }: DisclosureProps = $props()

  const uid = $props.id()
  const headerId = `${uid}-header`
  const regionId = `${uid}-region`

  function toggle() {
    if (disabled) return
    open = !open
    ontoggle?.(open)
  }
</script>

<div
  class={['disclosure', `disclosure--${variant}`, `disclosure--${size}`, className]}
  class:disclosure--open={open}
>
  <button
    id={headerId}
    type="button"
    class="disclosure__header"
    aria-expanded={open ? 'true' : 'false'}
    aria-controls={regionId}
    {disabled}
    onclick={toggle}
  >
    <span class="disclosure__chevron" aria-hidden="true">
      <ActionIcon name="chevron-right" size={size === 'sm' ? 12 : 14} />
    </span>
    <span class="disclosure__summary">
      {#if typeof summary === 'function'}
        {@render summary()}
      {:else if summary}
        {summary}
      {/if}
    </span>
    {#if meta}
      <span class="disclosure__meta">{@render meta()}</span>
    {/if}
  </button>
  <div
    id={regionId}
    class="disclosure__region"
    role="region"
    aria-labelledby={headerId}
    hidden={!open}
  >
    {#if children}{@render children()}{/if}
  </div>
</div>

<style>
  .disclosure {
    min-width: 0;
    color: var(--color-text-primary);
    font-family: var(--font-ui);
  }

  .disclosure--card {
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-card);
  }

  .disclosure__header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    inline-size: 100%;
    box-sizing: border-box;
    margin: 0;
    border: 0;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-secondary);
    font: inherit;
    font-weight: var(--font-weight-medium);
    line-height: var(--line-height-base);
    text-align: start;
    cursor: pointer;
    outline: none;
    transition:
      background-color var(--transition-base),
      color var(--transition-base),
      box-shadow var(--transition-base);
  }

  /* The header grows when the summary wraps; it never drops below its token. */
  .disclosure--sm .disclosure__header {
    min-height: var(--control-height-sm);
    padding: 0 var(--space-2);
    font-size: var(--font-size-xs);
  }

  .disclosure--md .disclosure__header {
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    font-size: var(--font-size-sm);
  }

  .disclosure--card .disclosure__header {
    border-radius: var(--radius-surface);
  }

  .disclosure__header:hover:not(:disabled),
  .disclosure--open .disclosure__header {
    color: var(--color-text-primary);
  }

  .disclosure__header:hover:not(:disabled) {
    background-color: var(--surface-toolbar);
  }

  .disclosure__header:focus-visible {
    box-shadow: var(--focus-ring);
  }

  .disclosure__header:disabled {
    cursor: not-allowed;
    opacity: var(--opacity-disabled);
  }

  .disclosure__chevron {
    display: inline-flex;
    flex: none;
    transition: transform var(--transition-base);
  }

  .disclosure--open .disclosure__chevron {
    transform: rotate(90deg);
  }

  .disclosure__summary {
    flex: 1 1 auto;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .disclosure__meta {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
    margin-inline-start: auto;
    color: var(--color-text-muted);
    font-weight: var(--font-weight-normal);
  }

  .disclosure__region {
    min-width: 0;
  }

  .disclosure--plain .disclosure__region {
    padding: var(--space-2) 0 0;
  }

  .disclosure--card .disclosure__region {
    padding: 0 var(--space-3) var(--space-3);
  }

  .disclosure--sm.disclosure--card .disclosure__region {
    padding: 0 var(--space-2) var(--space-2);
  }

  @media (prefers-reduced-motion: reduce) {
    .disclosure__header,
    .disclosure__chevron {
      transition: none;
    }
  }
</style>
