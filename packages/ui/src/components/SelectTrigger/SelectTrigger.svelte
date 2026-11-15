<script lang="ts">
  /**
   * The button that opens a list of choices: it shows the current one and a
   * chevron. Never a native select element, which cannot be themed. Spread a
   * ToolbarMenu trigger's props onto it, or use Select, which does so.
   */
  import ActionIcon from '../Button/ActionIcon.svelte'
  import type { SelectTriggerProps } from './SelectTrigger.types'

  let {
    label,
    placeholder,
    size = 'md',
    variant = 'field',
    open = false,
    disabled = false,
    element = $bindable(),
    class: className,
    ...rest
  }: SelectTriggerProps = $props()

  const empty = $derived(!label)
</script>

<button
  {...rest}
  bind:this={element}
  type="button"
  class={['select-trigger', `select-trigger--${size}`, `select-trigger--${variant}`, className]}
  class:select-trigger--open={open}
  aria-haspopup="menu"
  aria-expanded={open ? 'true' : 'false'}
  {disabled}
>
  <span class="select-trigger__value" class:select-trigger__value--placeholder={empty}>
    {empty ? (placeholder ?? '') : label}
  </span>
  <ActionIcon name="chevron-down" size={size === 'sm' ? 12 : 14} />
</button>

<style>
  .select-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    max-width: 100%;
    box-sizing: border-box;
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-primary);
    font-family: var(--font-ui);
    line-height: var(--line-height-base);
    cursor: pointer;
    outline: none;
    transition:
      border-color var(--transition-base),
      background-color var(--transition-base),
      box-shadow var(--transition-base);
  }

  /* Capped at its token: a flex or grid parent never makes it taller. */
  .select-trigger--sm {
    min-height: var(--control-height-sm);
    max-height: var(--control-height-sm);
    padding: 0 var(--space-2);
    font-size: var(--font-size-xs);
  }

  .select-trigger--md {
    min-height: var(--control-height-md);
    max-height: var(--control-height-md);
    padding: 0 var(--space-3);
    font-size: var(--font-size-sm);
  }

  /* Same well as Input. */
  .select-trigger--field {
    border-color: var(--color-hairline);
    background-color: color-mix(in srgb, var(--color-surface-sunken) 88%, transparent);
  }

  .select-trigger--field:hover:not(:disabled),
  .select-trigger--field.select-trigger--open {
    border-color: color-mix(in srgb, var(--color-accent) 40%, var(--color-hairline));
  }

  .select-trigger--quiet {
    color: var(--color-text-secondary);
  }

  .select-trigger--quiet:hover:not(:disabled),
  .select-trigger--quiet.select-trigger--open {
    background-color: var(--color-surface-raised);
    color: var(--color-text-primary);
  }

  .select-trigger:focus-visible {
    box-shadow: var(--focus-ring);
  }

  .select-trigger:disabled {
    cursor: not-allowed;
    opacity: var(--opacity-disabled);
  }

  .select-trigger__value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .select-trigger__value--placeholder {
    color: var(--color-text-muted);
  }

  @media (prefers-reduced-motion: reduce) {
    .select-trigger {
      transition: none;
    }
  }
</style>
