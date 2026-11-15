<script lang="ts">
  import type { PanelProps } from './Panel.types'

  let {
    variant = 'default',
    padding = 'md',
    hoverable = false,
    active = false,
    children,
    class: className = '',
    ...rest
  }: PanelProps = $props()
</script>

<div
  class="panel panel--{variant} panel--padding-{padding} {className}"
  class:panel--hoverable={hoverable}
  class:panel--active={active}
  {...rest}
>
  {#if children}
    {@render children()}
  {/if}
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    min-width: 0;
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-surface);
    color: var(--color-text-primary);
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base),
      box-shadow var(--transition-base),
      transform var(--transition-base);
  }

  .panel--default {
    background: var(--surface-panel);
    box-shadow: var(--shadow-surface);
  }

  .panel--raised {
    background: var(--surface-card);
    border-color: var(--border-subtle);
    box-shadow: var(--shadow-surface);
  }

  .panel--glass {
    background: color-mix(in srgb, var(--surface-glass) 88%, transparent);
    border-color: var(--border-subtle);
    box-shadow: var(--shadow-surface);
    backdrop-filter: blur(10px);
  }

  .panel--sunken {
    background: var(--surface-input);
    border-color: var(--border-subtle);
    box-shadow: var(--shadow-inset);
  }

  .panel--hoverable:hover {
    border-color: var(--color-border-strong);
    box-shadow: var(--shadow-surface);
  }

  .panel--active {
    border-color: var(--border-focus);
    box-shadow: var(--shadow-surface), var(--focus-ring);
  }

  .panel--padding-none {
    padding: 0;
  }

  .panel--padding-sm {
    padding: var(--space-3);
  }

  .panel--padding-md {
    padding: var(--space-4);
  }

  .panel--padding-lg {
    padding: var(--space-6);
  }

  @media (prefers-reduced-motion: reduce) {
    .panel {
      transition: none;
    }
  }
</style>
