<script lang="ts">
  /**
   * One option of a RadioGroup.
   *
   * The radio is a real `input[type=radio]`, visually hidden; the card draws
   * the state instead. Selection is marked by a check glyph as well as by
   * contrast, so it never rests on colour alone, and the card draws the one
   * focus ring, from the radio.
   *
   * The `<label>` is `display: contents`, so its parts lay out in the card
   * itself while a click anywhere on them still chooses the radio. An
   * `embedded` control sits beside that label, never inside it: a text field's
   * value would otherwise become part of the radio's accessible name.
   */
  import { getContext } from 'svelte'
  import { ActionIcon } from '../Button'
  import { RADIO_GROUP_CONTEXT, type RadioGroupContext } from './RadioGroup.svelte'
  import type { RadioCardProps } from './RadioGroup.types'

  const uid = $props.id()

  let {
    value,
    label,
    description,
    wrapDescription = false,
    disabled = false,
    children,
    embedded,
  }: RadioCardProps = $props()

  const group = getContext<RadioGroupContext>(RADIO_GROUP_CONTEXT)
  if (!group) throw new Error('RadioCard must be rendered inside a RadioGroup')

  const selected = $derived(group.value === value)
  let input: HTMLInputElement | undefined = $state()

  function choose() {
    if (disabled) return
    group.select(value)
  }

  /** The card's own padding is outside the label: a click there still chooses. */
  function chooseFromCard(event: MouseEvent) {
    if (event.target !== event.currentTarget || !input || input.disabled) return
    input.click()
    input.focus()
  }
</script>

<!-- The click handler only widens the label's hit area to the card padding;
     the keyboard reaches the option through its native radio. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div
  class="radio-card radio-card--{group.variant}"
  class:radio-card--selected={selected}
  class:radio-card--disabled={disabled}
  class:radio-card--rich={!!children}
  class:radio-card--wrap-description={wrapDescription}
  onclick={chooseFromCard}
>
  <label class="radio-card__hit">
    <input
      bind:this={input}
      class="radio-card__input"
      type="radio"
      name={group.name}
      {value}
      checked={selected}
      {disabled}
      aria-labelledby="{uid}-label"
      aria-describedby={description ? `${uid}-description` : undefined}
      onchange={choose}
    />

    <span class="radio-card__head">
      <span class="radio-card__label" id="{uid}-label">{label}</span>
      <span class="radio-card__check" aria-hidden="true">
        {#if selected}<ActionIcon name="check" size={14} />{/if}
      </span>
    </span>

    {#if children}
      <span class="radio-card__body">{@render children()}</span>
    {/if}

    {#if description}
      <span class="radio-card__description" id="{uid}-description">
        {#if typeof description === 'string'}{description}{:else}{@render description()}{/if}
      </span>
    {/if}
  </label>

  {#if embedded}
    <div class="radio-card__embedded" onfocusin={choose}>
      {@render embedded()}
    </div>
  {/if}
</div>

<style>
  .radio-card {
    --radio-card-head-size: 20px;
    --radio-card-check-size: 14px;
    position: relative;
    display: grid;
    align-content: center;
    gap: var(--space-2);
    min-inline-size: 0;
    min-block-size: var(--control-height-sm);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base),
      color var(--transition-base);
  }

  /* A card with a preview or sample under its head gets room around it. */
  .radio-card--rich {
    padding: var(--space-2) var(--space-3) var(--space-3);
  }

  .radio-card:hover {
    border-color: var(--color-border-hover);
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
  }

  /* Selected reads as contrast, never as saturation. */
  .radio-card--selected,
  .radio-card--selected:hover {
    border-color: var(--color-border-strong);
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
  }

  /* The whole card draws the ring, so there is one visible focus, not two. */
  .radio-card:has(.radio-card__input:focus-visible) {
    box-shadow: var(--focus-ring);
  }

  .radio-card--disabled {
    cursor: not-allowed;
    opacity: var(--opacity-disabled);
  }

  .radio-card--disabled:hover {
    border-color: var(--border-subtle);
    background: transparent;
    color: var(--color-text-secondary);
  }

  /* A row of a destination list: no border of its own, the check first. */
  .radio-card--list {
    display: flex;
    align-items: center;
    padding: var(--space-1) var(--space-2);
    border-color: transparent;
  }

  .radio-card--list:hover,
  .radio-card--list.radio-card--selected,
  .radio-card--list.radio-card--disabled:hover {
    border-color: transparent;
  }

  .radio-card__hit {
    display: contents;
    cursor: inherit;
  }

  .radio-card__input {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }

  .radio-card__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-inline-size: 0;
    min-block-size: var(--radio-card-head-size);
  }

  .radio-card--list .radio-card__head {
    flex-direction: row-reverse;
    justify-content: flex-end;
    flex: 0 1 auto;
  }

  /* Wrapped, not cut: a long label takes two lines. */
  .radio-card__label {
    flex: 1;
    min-inline-size: 0;
    line-height: var(--line-height-tight);
    overflow-wrap: anywhere;
  }

  .radio-card--rich .radio-card__label {
    font-weight: var(--font-weight-semibold);
  }

  .radio-card__check {
    display: inline-flex;
    flex-shrink: 0;
    inline-size: var(--radio-card-check-size);
    color: var(--color-text-primary);
  }

  .radio-card__body {
    display: grid;
    min-inline-size: 0;
  }

  .radio-card__description {
    overflow: hidden;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-normal);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .radio-card--wrap-description .radio-card__description {
    overflow: visible;
    line-height: var(--line-height-base);
    text-overflow: clip;
    white-space: normal;
  }

  .radio-card__embedded {
    display: flex;
    flex: 1;
    min-inline-size: 0;
    cursor: auto;
  }

  @media (prefers-reduced-motion: reduce) {
    .radio-card {
      transition: none;
    }
  }
</style>
