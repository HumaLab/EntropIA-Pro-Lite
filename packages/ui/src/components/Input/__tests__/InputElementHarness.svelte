<script lang="ts">
  import Input from '../Input.svelte'

  let { onelement }: { onelement: (element: HTMLInputElement | null | undefined) => void } =
    $props()
  let element = $state<HTMLInputElement | null>()

  $effect(() => onelement(element))
</script>

<Input label="Name" bind:element />
