<script lang="ts">
  import type { ButtonProps } from './Button.types'

  let {
    variant = 'primary',
    size = 'md',
    iconOnly = false,
    disabled = false,
    loading = false,
    type = 'button',
    children,
    class: className = '',
    ...rest
  }: ButtonProps = $props()

  let isDisabled = $derived(disabled || loading)
</script>

<button
  class="btn btn--{variant} btn--{size} {className}"
  class:btn--icon-only={iconOnly}
  class:btn--loading={loading}
  {type}
  disabled={isDisabled}
  aria-busy={loading}
  {...rest}
>
  {#if loading}
    <span class="btn__spinner" aria-hidden="true"></span>
  {/if}
  <span class="btn__label" class:btn__label--hidden={loading}>
    {#if children}
      {@render children()}
    {/if}
  </span>
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    min-height: var(--control-height-md);
    padding: 0 var(--space-4);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    line-height: var(--line-height-tight);
    cursor: pointer;
    transition:
      background-color var(--transition-smooth),
      border-color var(--transition-smooth),
      color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      opacity var(--transition-smooth);
    position: relative;
    white-space: nowrap;
    user-select: none;
    box-shadow: none;
  }

  .btn:disabled {
    cursor: not-allowed;
    opacity: 0.48;
    transform: none;
  }

  .btn:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  /* ─── Variants ─── */
  .btn--primary {
    background: var(--color-accent);
    color: var(--color-bg);
    border-color: var(--color-accent);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.14);
  }
  .btn--primary:hover:not(:disabled) {
    background: var(--color-accent-hover);
    border-color: var(--color-accent-hover);
  }

  .btn--secondary {
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
    border-color: var(--color-hairline);
  }
  .btn--secondary:hover:not(:disabled) {
    background: var(--color-surface-elevated);
    border-color: var(--color-border-strong);
  }

  .btn--outline {
    background: transparent;
    color: var(--color-text-primary);
    border-color: var(--color-border);
  }
  .btn--outline:hover:not(:disabled) {
    background: var(--color-accent-faint);
    border-color: var(--color-border-hover);
  }

  .btn--ghost {
    background-color: transparent;
    color: var(--color-text-secondary);
    border-color: transparent;
  }
  .btn--ghost:hover:not(:disabled) {
    background-color: var(--color-accent-faint);
    color: var(--color-text-primary);
  }

  .btn--subtle {
    background: var(--color-accent-faint);
    color: var(--color-text-primary);
    border-color: transparent;
  }
  .btn--subtle:hover:not(:disabled) {
    background: var(--color-accent-soft);
    border-color: color-mix(in srgb, var(--color-accent) 18%, transparent);
  }

  .btn--danger {
    background: var(--color-danger-soft);
    color: var(--color-danger);
    border-color: color-mix(in srgb, var(--color-danger) 34%, transparent);
    box-shadow: none;
  }
  .btn--danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--color-danger) 16%, transparent);
    border-color: color-mix(in srgb, var(--color-danger) 48%, transparent);
    color: var(--color-danger-hover);
  }

  /* ─── Sizes ─── */
  .btn--sm {
    min-height: var(--control-height-sm);
    padding: 0 var(--space-3);
    font-size: var(--font-size-xs);
  }
  .btn--md {
    min-height: var(--control-height-md);
    padding: 0 var(--space-4);
    font-size: var(--font-size-sm);
  }
  .btn--lg {
    min-height: var(--control-height-lg);
    padding: 0 var(--space-5);
    font-size: var(--font-size-sm);
  }

  /* ─── Spinner ─── */
  .btn__spinner {
    width: 14px;
    height: 14px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: var(--radius-full);
    animation: spin 0.6s linear infinite;
    position: absolute;
  }

  .btn__label {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .btn__label--hidden {
    visibility: hidden;
  }

  .btn--loading {
    cursor: wait;
  }

  .btn--icon-only {
    gap: 0;
    aspect-ratio: 1;
    padding: 0;
    flex-shrink: 0;
  }

  .btn--icon-only.btn--sm {
    width: var(--control-height-sm);
  }

  .btn--icon-only.btn--md {
    width: var(--control-height-md);
  }

  .btn--icon-only.btn--lg {
    width: var(--control-height-lg);
  }

  .btn--icon-only :global(svg) {
    flex-shrink: 0;
    pointer-events: none;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
