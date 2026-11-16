<script lang="ts">
  import type { SwitchProps } from './Switch.types'

  let {
    checked = $bindable(false),
    label,
    ariaLabel,
    description,
    disabled = false,
    size = 'md',
    element = $bindable(),
    onchange,
    class: className = '',
    ...rest
  }: SwitchProps = $props()

  const uid = $props.id()
  const descriptionId = `${uid}-description`

  function handleChange(event: Event & { currentTarget: HTMLInputElement }) {
    checked = event.currentTarget.checked
    onchange?.(checked)
  }
</script>

<div class="switch switch--{size} {className}" class:switch--disabled={disabled}>
  <label class="switch__row">
    <input
      bind:this={element}
      class="switch__control"
      type="checkbox"
      role="switch"
      {checked}
      {disabled}
      aria-label={ariaLabel}
      aria-describedby={description ? descriptionId : undefined}
      onchange={handleChange}
      {...rest}
    />
    {#if label}<span class="switch__label">{label}</span>{/if}
  </label>
  {#if description}
    <span class="switch__description" id={descriptionId}>{description}</span>
  {/if}
</div>

<style>
  .switch {
    --switch-track-width: 36px;
    --switch-track-height: 20px;
    --switch-thumb-size: 14px;
    --switch-border-width: 1px;
    /* The gap between the thumb and the track's inner edge. */
    --switch-thumb-inset: calc(
      (var(--switch-track-height) - 2 * var(--switch-border-width) - var(--switch-thumb-size)) / 2
    );
    display: inline-flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .switch--sm {
    --switch-track-width: 28px;
    --switch-track-height: 16px;
    --switch-thumb-size: 10px;
  }

  .switch--md {
    --switch-track-width: 36px;
    --switch-track-height: 20px;
    --switch-thumb-size: 14px;
  }

  .switch__row {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    cursor: pointer;
  }

  /* The native control brings the system blue and its own focus ring. */
  .switch__control {
    appearance: none;
    -webkit-appearance: none;
    position: relative;
    flex: 0 0 auto;
    box-sizing: border-box;
    inline-size: var(--switch-track-width);
    block-size: var(--switch-track-height);
    margin: 0;
    border: var(--switch-border-width) solid var(--color-hairline);
    border-radius: var(--radius-full);
    background: var(--color-surface-sunken);
    cursor: inherit;
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base);
  }

  .switch__control::before {
    content: '';
    position: absolute;
    top: var(--switch-thumb-inset);
    left: var(--switch-thumb-inset);
    inline-size: var(--switch-thumb-size);
    block-size: var(--switch-thumb-size);
    border-radius: var(--radius-full);
    background: var(--color-surface-elevated);
    transition: transform var(--transition-base);
  }

  .switch__control:checked {
    border-color: transparent;
    background: var(--color-accent);
  }

  .switch__control:checked::before {
    transform: translateX(
      calc(
        var(--switch-track-width) - 2 * var(--switch-border-width) - var(--switch-thumb-size) - 2 *
          var(--switch-thumb-inset)
      )
    );
  }

  .switch__control:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .switch__label {
    min-width: 0;
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
  }

  .switch__description {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .switch--disabled {
    opacity: var(--opacity-disabled);
  }

  .switch--disabled .switch__row {
    cursor: not-allowed;
  }

  @media (prefers-reduced-motion: reduce) {
    .switch__control,
    .switch__control::before {
      transition: none;
    }
  }
</style>
