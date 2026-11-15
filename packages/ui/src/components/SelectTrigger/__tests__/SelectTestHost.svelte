<script lang="ts">
  import Select from '../Select.svelte'
  import type { SelectOption } from '../SelectTrigger.types'

  let {
    options,
    initial,
    onchange,
  }: {
    options: SelectOption[]
    initial: string
    onchange?: (value: string) => void
  } = $props()

  // svelte-ignore state_referenced_locally
  let value = $state(initial) // seeded once; the select owns it from then on
</script>

<Select label="Library type" aria-label="Library type" {options} {onchange} bind:value />
<span data-testid="bound">{value}</span>
