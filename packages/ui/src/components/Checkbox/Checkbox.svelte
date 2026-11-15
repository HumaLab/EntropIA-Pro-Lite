<script lang="ts">
  import type { CheckboxProps } from './Checkbox.types'

  let {
    checked = $bindable(false),
    children,
    disabled = false,
    indeterminate = false,
    onchange,
    label,
    size = 'md',
    variant = 'row',
    class: className = '',
  }: CheckboxProps = $props()

  function handleChange(event: Event & { currentTarget: HTMLInputElement }) {
    checked = event.currentTarget.checked
    onchange?.(checked)
  }
</script>

<label
  class="checkbox checkbox--{variant} checkbox--{size} {className}"
  class:checkbox--disabled={disabled}
>
  <input
    type="checkbox"
    {checked}
    {disabled}
    {indeterminate}
    aria-label={label}
    onchange={handleChange}
  />
  <span class="checkbox__label">
    {#if children}{@render children()}{/if}
  </span>
</label>

<style>
  .checkbox {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: var(--space-3);
    align-items: center;
    padding: var(--space-2) var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base);
  }

  .checkbox:hover {
    background: var(--surface-toolbar);
  }

  /* Selected reads as contrast, never as saturation. */
  .checkbox:has(input:checked) {
    background: var(--surface-toolbar);
    border-color: var(--border-subtle);
  }

  /* The whole row draws the ring, so there is one visible focus, not two. */
  .checkbox:has(input:focus-visible) {
    box-shadow: var(--focus-ring);
  }

  .checkbox--sm {
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    font-size: var(--font-size-xs);
  }

  /* A card lifts off a sunken list. Its resting border is --border-subtle,
     the row's checked border, so its own checked state needs a firmer one;
     these rules come after the row ones and win by order. */
  .checkbox--card {
    background: var(--surface-panel);
    border-color: var(--border-subtle);
  }

  .checkbox--card:hover {
    background: var(--surface-toolbar);
  }

  .checkbox--card:has(input:checked) {
    background: var(--surface-toolbar);
    border-color: var(--border-panel);
  }

  .checkbox--disabled {
    cursor: not-allowed;
    opacity: var(--opacity-disabled);
  }

  .checkbox--disabled:hover {
    background: transparent;
  }

  /* The native control brings the system blue and its own focus ring. */
  .checkbox input {
    appearance: none;
    -webkit-appearance: none;
    display: grid;
    place-content: center;
    inline-size: 1rem;
    block-size: 1rem;
    margin: 0;
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-xs);
    background: var(--surface-app);
    color: var(--color-text-primary);
    cursor: inherit;
    transition:
      border-color var(--transition-base),
      background-color var(--transition-base);
  }

  .checkbox input::after {
    content: '';
    inline-size: 0.625rem;
    block-size: 0.625rem;
    transform: scale(0);
    transition: transform var(--transition-base);
    background: currentColor;
    clip-path: polygon(14% 44%, 0 65%, 50% 100%, 100% 16%, 80% 0%, 43% 62%);
  }

  .checkbox input:checked {
    border-color: var(--color-text-primary);
  }

  .checkbox input:checked::after {
    transform: scale(1);
  }

  .checkbox input:indeterminate {
    border-color: var(--color-text-primary);
  }

  .checkbox input:indeterminate::after {
    transform: scale(1);
    clip-path: inset(40% 10%);
  }

  .checkbox input:focus-visible {
    outline: none;
  }

  .checkbox__label {
    min-width: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .checkbox,
    .checkbox input,
    .checkbox input::after {
      transition: none;
    }
  }
</style>
