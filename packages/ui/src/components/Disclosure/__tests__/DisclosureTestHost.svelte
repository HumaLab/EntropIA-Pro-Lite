<script lang="ts">
  import Disclosure from '../Disclosure.svelte'

  let {
    initial = false,
    disabled = false,
    variant,
    ontoggle,
  }: {
    initial?: boolean
    disabled?: boolean
    variant?: 'plain' | 'card'
    ontoggle?: (open: boolean) => void
  } = $props()

  // svelte-ignore state_referenced_locally
  let open = $state(initial) // seeded once; the disclosure owns it from then on
</script>

<Disclosure summary="Technical detail" {disabled} {variant} {ontoggle} bind:open>
  {#snippet meta()}<span data-testid="meta">3 hits</span>{/snippet}
  <p>Body text</p>
</Disclosure>
<span data-testid="bound">{open ? 'open' : 'closed'}</span>
<button type="button" onclick={() => (open = !open)}>External toggle</button>
