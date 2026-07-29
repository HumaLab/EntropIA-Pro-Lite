<script lang="ts">
  let {
    markers = [],
    locationOptions = [],
    height = '280px',
    visible = true,
    onlocationchange,
    onresetlocation,
  } = $props()
</script>

<div data-testid="mock-map-viewer" data-height={height} data-visible={visible}>
  {#each locationOptions as location (location.entityId)}
    <div data-testid={`mock-map-location-${location.entityId}`}>{location.label}</div>
  {/each}
  {#each markers as marker (marker.entityId)}
    <div data-testid={`mock-map-marker-${marker.entityId}`}>{marker.label}</div>
    <button
      type="button"
      data-testid={`mock-map-save-${marker.entityId}`}
      onclick={() => onlocationchange?.(marker.entityId, -34.615, -58.433)}
    >Save map location</button>
    <button
      type="button"
      data-testid={`mock-map-reset-${marker.entityId}`}
      onclick={() => onresetlocation?.(marker.entityId)}
    >Reset map location</button>
  {/each}
</div>
