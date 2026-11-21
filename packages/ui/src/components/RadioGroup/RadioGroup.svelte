<script lang="ts" module>
  import type { RadioGroupVariant } from './RadioGroup.types'

  /** What a RadioCard reads from the group around it. */
  export interface RadioGroupContext {
    readonly name: string
    readonly value: string | undefined
    readonly variant: RadioGroupVariant
    select(value: string): void
  }

  export const RADIO_GROUP_CONTEXT = Symbol('radio-group')
</script>

<script lang="ts">
  /**
   * A group of mutually exclusive options drawn as cards or rows.
   *
   * The options are native radios sharing one `name`, visually hidden inside
   * each `RadioCard`: arrow keys move the choice and the group is announced
   * as one, with no key handling of our own. The group is a `<fieldset>`, so
   * `disabled` reaches every option and any control embedded in one.
   */
  import { setContext } from 'svelte'
  import type { RadioGroupProps } from './RadioGroup.types'

  const uid = $props.id()

  let {
    name = `${uid}-radio`,
    value = $bindable(),
    variant = 'card',
    label,
    hideLabel = false,
    disabled = false,
    columnMin,
    onchange,
    children,
    class: className = '',
  }: RadioGroupProps = $props()

  setContext<RadioGroupContext>(RADIO_GROUP_CONTEXT, {
    get name() {
      return name
    },
    get value() {
      return value
    },
    get variant() {
      return variant
    },
    select(next: string) {
      if (next === value) return
      value = next
      onchange?.(next)
    },
  })
</script>

<fieldset
  class="radio-group radio-group--{variant} {className}"
  role="radiogroup"
  aria-labelledby="{uid}-legend"
  {disabled}
  style:--radio-group-column-min={columnMin}
>
  <legend
    id="{uid}-legend"
    class="radio-group__legend"
    class:radio-group__legend--hidden={hideLabel}
  >
    {label}
  </legend>
  <div class="radio-group__options">
    {@render children()}
  </div>
</fieldset>

<style>
  .radio-group {
    --radio-group-column-min: 168px;
    min-inline-size: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }

  .radio-group__legend {
    margin: 0 0 var(--space-1);
    padding: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: var(--letter-spacing-caps);
    text-transform: uppercase;
  }

  .radio-group__legend--hidden {
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

  /* Columns sized from the group's own box: the panels it sits in resize
     independently of the viewport, so a media query would be wrong here. */
  .radio-group--card .radio-group__options {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, var(--radio-group-column-min)), 1fr));
    gap: var(--space-2);
  }

  .radio-group--list .radio-group__options {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
</style>
