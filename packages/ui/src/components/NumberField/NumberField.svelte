<!--
  A numeric field with its own − and + buttons in place of the native spin
  buttons, which every engine draws differently and none matches the theme.

  The input stays a real `type="number"`: the browser keeps parsing, keyboard
  stepping (ArrowUp/ArrowDown) and the min/max/step validity, and the field
  keeps its `spinbutton` role and label. The buttons are spans with a button
  role, not <button>s: a caller often wraps the field in a <label>, and a label
  names its first labelable descendant, which would then be the − button. They
  stay out of the tab order; the keyboard steps the field with the arrows. A button press writes the input and
  fires the same `input` and `change` events typing does, so a caller's
  `bind:value`, `oninput` and `onchange` see a click exactly as an edit.
-->
<script lang="ts">
  import ActionIcon from '../Button/ActionIcon.svelte'
  import type { NumberFieldProps } from './NumberField.types'

  let {
    value = $bindable(null),
    min,
    max,
    step = 1,
    disabled = false,
    decrementLabel,
    incrementLabel,
    label,
    size = 'md',
    class: className = '',
    ...rest
  }: NumberFieldProps = $props()

  let input: HTMLInputElement | undefined = $state()

  const bound = (raw: number | string | undefined): number | null => {
    if (raw === undefined || raw === '') return null
    const parsed = Number(raw)
    return Number.isFinite(parsed) ? parsed : null
  }

  const lower = $derived(bound(min))
  const upper = $derived(bound(max))
  const increment = $derived.by(() => {
    const parsed = bound(step)
    return parsed !== null && parsed > 0 ? parsed : 1
  })
  const current = $derived.by(() => {
    if (value === null || value === undefined || value === '') return null
    const parsed = Number(value)
    return Number.isFinite(parsed) ? parsed : null
  })
  const atLower = $derived(current !== null && lower !== null && current <= lower)
  const atUpper = $derived(current !== null && upper !== null && current >= upper)

  /** Decimal places a number is written with, so 0.1 + 0.05 lands on 0.15. */
  function decimals(n: number): number {
    const text = String(n)
    const exponent = text.match(/e-(\d+)$/)
    if (exponent) return Number(exponent[1])
    const dot = text.indexOf('.')
    return dot < 0 ? 0 : text.length - dot - 1
  }

  /** A press must not move focus away from the field it edits. */
  function keepFocus(event: MouseEvent) {
    event.preventDefault()
  }

  function press(event: MouseEvent, direction: 1 | -1, blocked: boolean) {
    // Inside a wrapping <label>, a click would also be forwarded to the input.
    event.preventDefault()
    event.stopPropagation()
    if (!blocked) nudge(direction)
  }

  /** Enter and Space press a focused button, as on a native <button>. */
  function key(event: KeyboardEvent, direction: 1 | -1, blocked: boolean) {
    if (event.key !== 'Enter' && event.key !== ' ') return
    event.preventDefault()
    if (!blocked) nudge(direction)
  }

  function nudge(direction: 1 | -1) {
    if (!input || disabled) return
    const typed = input.value === '' ? NaN : Number(input.value)
    let next = Number.isFinite(typed) ? typed + direction * increment : (lower ?? 0)
    if (lower !== null) next = Math.max(next, lower)
    if (upper !== null) next = Math.min(next, upper)
    next = Number(next.toFixed(decimals(increment)))
    if (next === typed) return
    input.value = String(next)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new Event('change', { bubbles: true }))
  }
</script>

<div class="number-field number-field--{size} {className}" class:number-field--disabled={disabled}>
  <span
    role="button"
    class="number-field__step"
    tabindex="-1"
    aria-label={decrementLabel}
    aria-disabled={disabled || atLower}
    onmousedown={keepFocus}
    onclick={(event) => press(event, -1, disabled || atLower)}
    onkeydown={(event) => key(event, -1, disabled || atLower)}
  >
    <ActionIcon name="subtract" size={14} />
  </span>
  <input
    bind:this={input}
    bind:value
    class="number-field__input"
    type="number"
    {min}
    {max}
    {step}
    {disabled}
    aria-label={label}
    {...rest}
  />
  <span
    role="button"
    class="number-field__step"
    tabindex="-1"
    aria-label={incrementLabel}
    aria-disabled={disabled || atUpper}
    onmousedown={keepFocus}
    onclick={(event) => press(event, 1, disabled || atUpper)}
    onkeydown={(event) => key(event, 1, disabled || atUpper)}
  >
    <ActionIcon name="add" size={14} />
  </span>
</div>

<style>
  .number-field {
    display: flex;
    align-items: stretch;
    width: 100%;
    /* One width for every numeric field; a caller widens it with the variable. */
    max-width: var(--number-field-max-width, 9.5rem);
    min-width: 0;
    height: var(--control-height-md);
    box-sizing: border-box;
    background-color: color-mix(in srgb, var(--color-surface-sunken) 88%, transparent);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    overflow: hidden;
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      background-color var(--transition-smooth);
  }

  .number-field--sm {
    height: var(--control-height-sm);
  }

  .number-field:focus-within {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background-color: color-mix(in srgb, var(--color-surface) 92%, transparent);
  }

  .number-field--disabled {
    opacity: 0.56;
    background-color: var(--color-surface-raised);
  }

  .number-field__input {
    flex: 1 1 auto;
    width: 100%;
    min-width: 3ch;
    margin: 0;
    padding: 0 var(--space-2);
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    font-variant-numeric: tabular-nums;
    text-align: center;
    color: var(--color-text-primary);
    background: transparent;
    border: 0;
    outline: none;
    appearance: textfield;
    -moz-appearance: textfield;
  }

  .number-field__input::-webkit-inner-spin-button,
  .number-field__input::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .number-field__input:disabled {
    cursor: not-allowed;
  }

  .number-field__step {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--control-height-sm);
    padding: 0;
    color: var(--color-text-secondary);
    background: transparent;
    border: 0;
    cursor: pointer;
    user-select: none;
    transition:
      color var(--transition-smooth),
      background-color var(--transition-smooth);
  }

  .number-field__step:first-child {
    border-right: 1px solid var(--color-hairline);
  }

  .number-field__step:last-child {
    border-left: 1px solid var(--color-hairline);
  }

  .number-field__step:hover:not([aria-disabled='true']) {
    color: var(--color-text-primary);
    background-color: color-mix(in srgb, var(--color-surface-raised) 80%, transparent);
  }

  .number-field__step:active:not([aria-disabled='true']) {
    background-color: var(--color-surface-raised);
  }

  .number-field__step[aria-disabled='true'] {
    cursor: default;
    color: var(--color-text-muted);
    opacity: 0.5;
  }

  @media (prefers-reduced-motion: reduce) {
    .number-field,
    .number-field__step {
      transition: none;
    }
  }
</style>
