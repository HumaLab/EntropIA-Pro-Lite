<script lang="ts">
  import Switch from '../Switch.svelte'

  let { initial = false }: { initial?: boolean } = $props()

  // svelte-ignore state_referenced_locally
  let checked = $state(initial) // the harness seeds once, then the switch owns it
  let element = $state<HTMLInputElement>()
</script>

<Switch label="Sync" bind:checked bind:element />
<output data-testid="bound">{checked ? 'on' : 'off'}</output>
<output data-testid="element">{element?.tagName ?? 'none'}</output>
