<script lang="ts">
  import { Textarea } from '@entropia/ui'

  let { itemId = '', collectionId = '' }: { itemId?: string; collectionId?: string } = $props()
  let draft = $state('')
</script>

<div data-testid="lazy-route">{itemId}:{collectionId}</div>
<Textarea aria-label="Route draft" bind:value={draft} />
