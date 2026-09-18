<script lang="ts">
  import type { TabButtonProps } from './Tabs.types'

  let {
    active = false,
    type = 'button',
    children,
    class: className = '',
    ...rest
  }: TabButtonProps = $props()
</script>

<button
  class="tab-button {className}"
  class:tab-button--active={active}
  role="tab"
  aria-selected={active}
  {type}
  {...rest}
>
  {#if children}
    {@render children()}
  {/if}
</button>

<style>
  .tab-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    min-height: 28px;
    padding: 0 var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-secondary);
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    line-height: var(--line-height-tight);
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base),
      color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .tab-button:hover:not(:disabled) {
    color: var(--color-text-primary);
    background: var(--color-accent-faint);
  }

  .tab-button:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .tab-button:disabled {
    cursor: not-allowed;
    opacity: 0.48;
  }

  .tab-button--active {
    color: var(--color-text-primary);
    background: var(--surface-card);
    border-color: var(--border-panel);
    box-shadow: none;
  }
</style>
