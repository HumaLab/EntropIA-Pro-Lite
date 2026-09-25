<script lang="ts">
  import { tooltip } from '../Tooltip/tooltip'
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
    title,
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
  use:tooltip={title}
>
  {#if children}
    {@render children()}
  {/if}
</button>

<style>
  /* A fixed square: every side reads one property, so no flex or grid parent
     can stretch or squeeze it (control-block-size.test.ts). A caller that
     needs another size sets `--icon-button-size`, not width and height. */
  .icon-button {
    --icon-button-size: 32px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    box-sizing: border-box;
    width: var(--icon-button-size);
    min-width: var(--icon-button-size);
    max-width: var(--icon-button-size);
    height: var(--icon-button-size);
    min-height: var(--icon-button-size);
    max-height: var(--icon-button-size);
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    color: var(--color-text-secondary);
    font-family: var(--font-ui);
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
    --icon-button-size: 24px;
  }

  .icon-button--sm {
    --icon-button-size: 28px;
  }

  .icon-button--md {
    --icon-button-size: 32px;
  }

  .icon-button--lg {
    --icon-button-size: var(--control-height-lg);
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
    background: var(--control-primary-bg);
    border-color: var(--control-primary-border);
    color: var(--control-primary-text);
  }

  .icon-button--primary:hover:not(:disabled) {
    background: var(--control-primary-bg-hover);
    color: var(--control-primary-text);
  }

  .icon-button--primary:active:not(:disabled) {
    background: var(--control-primary-bg-active);
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
