<script lang="ts">
  import type { IconButtonProps } from './IconButton.types'

  let {
    variant = 'ghost',
    size = 'md',
    label,
    active = false,
    disabled = false,
    type = 'button',
    children,
    class: className = '',
    ...rest
  }: IconButtonProps = $props()
</script>

<button
  class="icon-button icon-button--{variant} icon-button--{size} {className}"
  class:icon-button--active={active}
  aria-label={label}
  aria-pressed={active ? 'true' : undefined}
  {disabled}
  {type}
  {...rest}
>
  {#if children}
    {@render children()}
  {/if}
</button>

<style>
  .icon-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    color: var(--color-text-secondary);
    font-family: var(--font-sans);
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base),
      color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .icon-button:hover:not(:disabled) {
    color: var(--color-text-primary);
  }

  .icon-button:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .icon-button:disabled {
    cursor: not-allowed;
    opacity: 0.48;
  }

  .icon-button--xs {
    width: 24px;
    height: 24px;
  }

  .icon-button--sm {
    width: 28px;
    height: 28px;
  }

  .icon-button--md {
    width: 32px;
    height: 32px;
  }

  .icon-button--lg {
    width: var(--control-height-lg);
    height: var(--control-height-lg);
  }

  .icon-button--ghost {
    background: transparent;
  }

  .icon-button--ghost:hover:not(:disabled),
  .icon-button--ghost.icon-button--active {
    background: var(--color-accent-faint);
    border-color: var(--border-subtle);
  }

  .icon-button--secondary {
    background: var(--surface-card);
    border-color: var(--border-subtle);
  }

  .icon-button--secondary:hover:not(:disabled),
  .icon-button--secondary.icon-button--active {
    border-color: var(--border-panel);
    background: var(--surface-card);
  }

  .icon-button--primary {
    background: var(--color-accent);
    border-color: var(--color-accent);
    color: var(--color-bg);
  }

  .icon-button--danger {
    background: var(--color-danger-soft);
    border-color: color-mix(in srgb, var(--color-danger) 24%, transparent);
    color: var(--color-danger);
  }

  .icon-button :global(svg) {
    flex-shrink: 0;
    pointer-events: none;
  }
</style>
