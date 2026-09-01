<script lang="ts">
  let {
    itemId = '',
    collectionId = '',
  }: { itemId?: string; collectionId?: string } = $props()
</script>

<div data-testid="lazy-route">{itemId}:{collectionId}</div>
