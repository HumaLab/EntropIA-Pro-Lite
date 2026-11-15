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
    class: className = '',
    size = 'md',
    font = 'ui',
    element = $bindable(),
    leading,
    trailing,
    id,
    oninput,
    ...rest
  }: InputProps = $props()

  const generatedId = $props.id()
  let inputId = $derived(id ?? generatedId)
  let hasError = $derived(!!error)
  // The types refuse a search box; a caller that forces one past them still
  // gets a plain text field, because search fields are always SearchBar.
  let inputType = $derived((type as string) === 'search' ? 'text' : type)

  function handleInput(e: Event & { currentTarget: EventTarget & HTMLInputElement }) {
    value = e.currentTarget.value
    oninput?.(e)
  }
</script>

<div
  class="input-field input-field--{size}"
  class:input-field--disabled={disabled}
  class:input-field--error={hasError}
>
  {#if label}
    <label class="input-field__label" for={inputId}>
      {label}
    </label>
  {/if}

  {#snippet field()}
    <input
      {...rest}
      bind:this={element}
      id={inputId}
      class="input-field__input {className}"
      class:input-field__input--mono={font === 'mono'}
      type={inputType}
      {placeholder}
      {disabled}
      {value}
      oninput={handleInput}
      aria-invalid={hasError}
      aria-describedby={error ? `${inputId}-error` : hint ? `${inputId}-hint` : undefined}
    />
  {/snippet}

  {#if leading || trailing}
    <div
      class="input-field__control"
      class:input-field__control--leading={!!leading}
      class:input-field__control--trailing={!!trailing}
    >
      {#if leading}
        <span class="input-field__leading">{@render leading()}</span>
      {/if}
      {@render field()}
      {#if trailing}
        <span class="input-field__trailing">{@render trailing()}</span>
      {/if}
    </div>
  {:else}
    {@render field()}
  {/if}

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
    /* Capped at its token: a flex or grid parent never makes it taller. */
    min-height: var(--control-height-md);
    max-height: var(--control-height-md);
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

  .input-field__input--mono {
    font-family: var(--font-mono);
  }

  .input-field--sm .input-field__input {
    min-height: var(--control-height-sm);
    max-height: var(--control-height-sm);
    padding: 0 var(--space-2);
    font-size: var(--font-size-xs);
  }

  /* Adornments sit over the input's edges, as the search field's magnifier
     does: the input keeps its own border, focus ring and height, and only
     owes the adornment its inline padding. They are icon-sized. */
  .input-field__control {
    --input-field-adornment-size: 16px;
    --input-field-adornment-inset: calc(
      var(--space-3) + var(--input-field-adornment-size) + var(--space-2)
    );
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
  }

  .input-field__leading,
  .input-field__trailing {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: var(--input-field-adornment-size);
    color: var(--color-text-secondary);
  }

  .input-field__leading {
    inset-inline-start: var(--space-3);
    pointer-events: none;
  }

  .input-field__trailing {
    inset-inline-end: var(--space-3);
  }

  .input-field__control--leading .input-field__input {
    padding-inline-start: var(--input-field-adornment-inset);
  }

  .input-field__control--trailing .input-field__input {
    padding-inline-end: var(--input-field-adornment-inset);
  }

  .input-field--disabled .input-field__leading,
  .input-field--disabled .input-field__trailing {
    opacity: var(--opacity-disabled);
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
    opacity: var(--opacity-disabled);
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
    opacity: var(--opacity-disabled);
  }

  @media (prefers-reduced-motion: reduce) {
    .input-field__input {
      transition: none;
    }
  }
</style>
