<script lang="ts">
  import RadioCard from '../RadioCard.svelte'
  import RadioGroup from '../RadioGroup.svelte'
  import type { RadioGroupVariant } from '../RadioGroup.types'

  let {
    value = $bindable('a'),
    variant = 'card',
    disabled = false,
    disableB = false,
    hideLabel = false,
    columnMin,
    wrapDescription = false,
    onchange,
  }: {
    value?: string
    variant?: RadioGroupVariant
    disabled?: boolean
    disableB?: boolean
    hideLabel?: boolean
    columnMin?: string
    wrapDescription?: boolean
    onchange?: (value: string) => void
  } = $props()

  let newName = $state('')
</script>

<RadioGroup label="Destino" bind:value {variant} {disabled} {hideLabel} {columnMin} {onchange}>
  <RadioCard value="a" label="Alfa" description="Primera opción" {wrapDescription}>
    <span data-testid="preview-a" aria-hidden="true">Vista previa</span>
  </RadioCard>
  <RadioCard value="b" label="Beta" disabled={disableB} />
  <RadioCard value="new" label="Nueva colección">
    {#snippet embedded()}
      <input aria-label="Nombre de la colección" bind:value={newName} />
    {/snippet}
  </RadioCard>
</RadioGroup>

<output data-testid="value">{value}</output>
