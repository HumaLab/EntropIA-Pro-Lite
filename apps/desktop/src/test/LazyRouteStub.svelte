<script lang="ts">
  let { itemId = '', collectionId = '' }: { itemId?: string; collectionId?: string } = $props()
  let draft = $state('')
</script>

<div data-testid="lazy-route">{itemId}:{collectionId}</div>
<textarea aria-label="Route draft" bind:value={draft}></textarea>
