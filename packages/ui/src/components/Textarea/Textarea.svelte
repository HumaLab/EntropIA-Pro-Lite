<script lang="ts">
  import type { TextareaProps } from './Textarea.types'

  let {
    value = $bindable(''),
    rows = 3,
    autosize = false,
    maxRows,
    label,
    hint,
    error,
    font = 'ui',
    disabled = false,
    class: className = '',
    element = $bindable(),
    ...rest
  }: TextareaProps = $props()

  const textareaId = $props.id()
  let hasError = $derived(!!error)
  let describedBy = $derived(
    error ? `${textareaId}-error` : hint ? `${textareaId}-hint` : undefined
  )

  const px = (raw: string) => parseFloat(raw) || 0

  /** Fit the height to the content, capped at `maxRows` lines when given. */
  function fitHeight(field: HTMLTextAreaElement) {
    field.style.height = 'auto'
    const style = getComputedStyle(field)
    const borders = px(style.borderTopWidth) + px(style.borderBottomWidth)
    const content = field.scrollHeight + borders
    let height = content
    if (maxRows) {
      const lineHeight = px(style.lineHeight)
      if (lineHeight > 0) {
        const padding = px(style.paddingTop) + px(style.paddingBottom)
        height = Math.min(content, lineHeight * maxRows + padding + borders)
      }
    }
    field.style.height = `${height}px`
    field.style.overflowY = height < content ? 'auto' : 'hidden'
  }

  $effect(() => {
    // Re-fit whenever the value changes, typed or set by the caller.
    void value
    if (autosize && element) fitHeight(element)
  })

  function handleInput(e: Event) {
    value = (e.target as HTMLTextAreaElement).value
  }
</script>

<div
  class="textarea-field"
  class:textarea-field--disabled={disabled}
  class:textarea-field--error={hasError}
>
  {#if label}
    <label class="textarea-field__label" for={textareaId}>
      {label}
    </label>
  {/if}

  <textarea
    {...rest}
    bind:this={element}
    id={textareaId}
    class="textarea-field__control textarea-field__control--{font} {className}"
    class:textarea-field__control--autosize={autosize}
    {rows}
    {disabled}
    {value}
    oninput={(e) => {
      handleInput(e)
      rest.oninput?.(e)
    }}
    aria-invalid={hasError}
    aria-describedby={describedBy}></textarea>

  {#if error}
    <span class="textarea-field__error" id={`${textareaId}-error`} role="alert">
      {error}
    </span>
  {:else if hint}
    <span class="textarea-field__hint" id={`${textareaId}-hint`}>
      {hint}
    </span>
  {/if}
</div>

<style>
  /* Same visual family as Input: border, radius, surface, focus and error. */
  .textarea-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
  }

  .textarea-field__label {
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
    user-select: none;
  }

  .textarea-field__control {
    width: 100%;
    /* The height comes from `rows`; the user may only make it taller. */
    resize: vertical;
    padding: var(--space-2) var(--space-3);
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
  }

  .textarea-field__control--ui {
    font-family: var(--font-ui);
    line-height: var(--line-height-base);
  }

  .textarea-field__control--reading {
    font-family: var(--font-reading);
    line-height: var(--line-height-reading);
  }

  .textarea-field__control--mono {
    font-family: var(--font-mono);
    line-height: var(--line-height-base);
  }

  .textarea-field__control--autosize {
    resize: none;
    overflow-y: hidden;
  }

  .textarea-field__control::placeholder {
    color: var(--color-text-muted);
  }

  .textarea-field__control:focus,
  .textarea-field__control:focus-visible {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background-color: color-mix(in srgb, var(--color-surface) 92%, transparent);
  }

  .textarea-field__control:disabled {
    cursor: not-allowed;
    resize: none;
    opacity: var(--opacity-disabled);
    background-color: var(--color-surface-raised);
  }

  .textarea-field--error .textarea-field__control {
    border-color: var(--color-danger);
  }

  .textarea-field--error .textarea-field__control:focus,
  .textarea-field--error .textarea-field__control:focus-visible {
    box-shadow: var(--focus-ring-danger);
  }

  .textarea-field__error {
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    color: var(--color-danger);
  }

  .textarea-field__hint {
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .textarea-field--disabled .textarea-field__label {
    opacity: var(--opacity-disabled);
  }

  @media (prefers-reduced-motion: reduce) {
    .textarea-field__control {
      transition: none;
    }
  }
</style>
