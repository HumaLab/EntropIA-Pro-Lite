<script lang="ts">
  import { tooltip } from '../Tooltip/tooltip'
  import type { IconButtonProps } from './IconButton.types'

  let {
    variant = 'ghost',
    size = 'md',
    hoverTone,
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
  class:icon-button--hover-danger={hoverTone === 'danger'}
  class:icon-button--hover-success={hoverTone === 'success'}
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
    opacity: var(--opacity-disabled);
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

  /* Hover tones come after the variants, which they override by order.
     Danger may tint the surface, its colour being its meaning; success only
     colours the icon and keeps the variant's own hover surface. */
  .icon-button--hover-danger:hover:not(:disabled) {
    border-color: color-mix(in srgb, var(--color-danger) 24%, transparent);
    background: var(--color-danger-soft);
    color: var(--color-danger);
  }

  .icon-button--hover-success:hover:not(:disabled) {
    color: var(--color-success);
  }

  /* The strip that opens and closes a side panel: 20px wide and as tall as
     the panel edge. It is the one IconButton that is not a square. It comes
     after the size rules, so it wins over them by order inside this
     stylesheet, whatever `size` the caller passes. */
  .icon-button--edge {
    --icon-button-size: 20px;
    height: auto;
    min-height: 0;
    max-height: none;
    border-radius: var(--radius-dialog);
    background: var(--surface-input);
    border-color: var(--border-subtle);
    color: var(--color-text-muted);
  }

  .icon-button--edge:hover:not(:disabled) {
    color: var(--color-accent);
    background: var(--color-accent-soft);
  }

  .icon-button :global(svg) {
    flex-shrink: 0;
    pointer-events: none;
  }

  @media (prefers-reduced-motion: reduce) {
    .icon-button {
      transition: none;
    }
  }
</style>
