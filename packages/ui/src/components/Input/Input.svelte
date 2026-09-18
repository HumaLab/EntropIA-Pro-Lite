<script lang="ts">
  import type { InputProps } from './Input.types'

  let {
    value = $bindable(''),
    type = 'text',
    placeholder = '',
    disabled = false,
    label,
    error,
    hint,
  }: InputProps = $props()

  const inputId = $props.id()
  let hasError = $derived(!!error)

  function handleInput(e: Event) {
    const target = e.target as HTMLInputElement
    value = target.value
  }
</script>

<div class="input-field" class:input-field--disabled={disabled} class:input-field--error={hasError}>
  {#if label}
    <label class="input-field__label" for={inputId}>
      {label}
    </label>
  {/if}

  <input
    id={inputId}
    class="input-field__input"
    {type}
    {placeholder}
    {disabled}
    {value}
    oninput={handleInput}
    aria-invalid={hasError}
    aria-describedby={error ? `${inputId}-error` : hint ? `${inputId}-hint` : undefined}
  />

  {#if error}
    <span class="input-field__error" id={`${inputId}-error`} role="alert">
      {error}
    </span>
  {:else if hint}
    <span class="input-field__hint" id={`${inputId}-hint`}>
      {hint}
    </span>
  {/if}
</div>

<style>
  .input-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
  }

  .input-field__label {
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
    user-select: none;
  }

  .input-field__input {
    width: 100%;
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    background-color: color-mix(in srgb, var(--color-surface-sunken) 88%, transparent);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    outline: none;
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      background-color var(--transition-smooth);
    box-sizing: border-box;
    line-height: var(--line-height-base);
  }

  .input-field__input::placeholder {
    color: var(--color-text-muted);
  }

  .input-field__input:focus,
  .input-field__input:focus-visible {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background-color: color-mix(in srgb, var(--color-surface) 92%, transparent);
  }

  .input-field__input:disabled {
    cursor: not-allowed;
    opacity: 0.56;
    background-color: var(--color-surface-raised);
  }

  .input-field--error .input-field__input {
    border-color: var(--color-danger);
  }

  .input-field--error .input-field__input:focus,
  .input-field--error .input-field__input:focus-visible {
    box-shadow: var(--focus-ring-danger);
  }

  .input-field__error {
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    color: var(--color-danger);
  }

  .input-field__hint {
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .input-field--disabled .input-field__label {
    opacity: 0.5;
  }
</style>
