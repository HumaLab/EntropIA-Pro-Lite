<script lang="ts">
  import type { MapViewerLabels, MapViewerProps } from './MapViewer.types'
  import { onMount, onDestroy, tick } from 'svelte'
  import L from 'leaflet'
  import 'leaflet/dist/leaflet.css'
  import markerIconUrl from 'leaflet/dist/images/marker-icon.png'
  import markerIcon2xUrl from 'leaflet/dist/images/marker-icon-2x.png'
  import markerShadowUrl from 'leaflet/dist/images/marker-shadow.png'

  const defaultLabels: MapViewerLabels = {
    empty: 'No geocoded locations',
    location: 'Location',
    edit: 'Edit location',
    save: 'Save',
    cancel: 'Cancel',
    reset: 'Reset automatic location',
    dragHint: 'Drag the marker to its correct location, then save the change.',
    saveError: 'Could not save the location.',
  }

  let {
    markers = [],
    height = '300px',
    visible = true,
    onmarkerclick,
    onlocationchange,
    onresetlocation,
    labels = {},
  }: MapViewerProps = $props()

  let rootEl: HTMLDivElement | undefined = $state()
  let mapContainer: HTMLDivElement | undefined = $state()
  let map: L.Map | null = null
  let markerLayer: L.LayerGroup | null = null
  let resizeObserver: ResizeObserver | null = null
  let invalidateScheduled = false
  let renderedMarkers: MapViewerProps['markers'] | null = null
  let selectedEntityId = $state<string | null>(null)
  let editingEntityId = $state<string | null>(null)
  let originalLocation = $state<{ latitude: number; longitude: number } | null>(null)
  let draftLocation = $state<{ latitude: number; longitude: number } | null>(null)
  let saving = $state(false)
  let actionError = $state<string | null>(null)
  const leafletMarkers = new Map<string, L.Marker>()

  let ui = $derived({ ...defaultLabels, ...labels })
  let selectedMarker = $derived(markers.find((marker) => marker.entityId === selectedEntityId) ?? null)

  const defaultCenter: L.LatLngExpression = [-34.6, -58.4]
  const defaultZoom = 3

  function configureDefaultMarkerIcon() {
    delete (L.Icon.Default.prototype as unknown as { _getIconUrl?: unknown })._getIconUrl
    L.Icon.Default.mergeOptions({
      iconRetinaUrl: markerIcon2xUrl,
      iconUrl: markerIconUrl,
      shadowUrl: markerShadowUrl,
      iconSize: [25, 41],
      iconAnchor: [12, 41],
      popupAnchor: [1, -34],
      tooltipAnchor: [16, -28],
      shadowSize: [41, 41],
      shadowAnchor: [12, 41],
    })
  }

  configureDefaultMarkerIcon()

  async function invalidateMapSize() {
    if (!map || !rootEl || !visible) return

    const rect = rootEl.getBoundingClientRect()
    if (rect.width === 0 || rect.height === 0) return

    await tick()

    requestAnimationFrame(() => {
      if (!map || !rootEl || !visible) return

      const nextRect = rootEl.getBoundingClientRect()
      if (nextRect.width === 0 || nextRect.height === 0) return

      map.invalidateSize(false)
    })
  }

  function scheduleInvalidateMapSize() {
    if (invalidateScheduled) return

    invalidateScheduled = true

    queueMicrotask(async () => {
      invalidateScheduled = false
      await invalidateMapSize()
    })
  }

  onMount(() => {
    if (!mapContainer) return

    map = L.map(mapContainer).setView(defaultCenter, defaultZoom)

    L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', {
      attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a>',
      maxZoom: 19,
    }).addTo(map)

    markerLayer = L.layerGroup().addTo(map)
    renderedMarkers = markers
    updateMarkers(markers)

    if (rootEl) {
      resizeObserver = new ResizeObserver((entries) => {
        const entry = entries[0]
        if (!entry) return

        if (entry.contentRect.width > 0 && entry.contentRect.height > 0) {
          scheduleInvalidateMapSize()
        }
      })

      resizeObserver.observe(rootEl)
    }

    scheduleInvalidateMapSize()
  })

  onDestroy(() => {
    resizeObserver?.disconnect()
    resizeObserver = null
    leafletMarkers.clear()

    if (map) {
      map.remove()
      map = null
    }
  })

  function updateMarkers(nextMarkers: MapViewerProps['markers']) {
    if (!map || !markerLayer) return

    markerLayer.clearLayers()
    leafletMarkers.clear()

    const bounds: L.LatLng[] = []

    for (const marker of nextMarkers) {
      const latLng = L.latLng(marker.latitude, marker.longitude)
      bounds.push(latLng)

      const leafletMarker = L.marker(latLng, { draggable: false }).addTo(markerLayer)
      leafletMarkers.set(marker.entityId, leafletMarker)

      const popupContent = marker.itemTitle
        ? `<strong>${marker.label}</strong><br><em>${marker.itemTitle}</em>`
        : `<strong>${marker.label}</strong>`
      leafletMarker.bindPopup(popupContent)
      leafletMarker.on('click', () => {
        if (!editingEntityId) selectedEntityId = marker.entityId
        onmarkerclick?.(marker)
      })
      leafletMarker.on('dragend', () => {
        if (editingEntityId !== marker.entityId) return
        const next = leafletMarker.getLatLng()
        draftLocation = { latitude: next.lat, longitude: next.lng }
        actionError = null
      })
    }

    if (bounds.length > 0) {
      const group = L.latLngBounds(bounds)
      map.fitBounds(group, { padding: [30, 30], maxZoom: 12 })
    }
  }

  function startLocationEdit() {
    if (!selectedMarker) return

    editingEntityId = selectedMarker.entityId
    originalLocation = {
      latitude: selectedMarker.latitude,
      longitude: selectedMarker.longitude,
    }
    draftLocation = null
    actionError = null
    leafletMarkers.get(selectedMarker.entityId)?.dragging?.enable()
  }

  function cancelLocationEdit() {
    if (editingEntityId && originalLocation) {
      const leafletMarker = leafletMarkers.get(editingEntityId)
      leafletMarker?.setLatLng([originalLocation.latitude, originalLocation.longitude])
      leafletMarker?.dragging?.disable()
    }

    editingEntityId = null
    originalLocation = null
    draftLocation = null
    actionError = null
  }

  async function saveLocationEdit() {
    if (!editingEntityId || !draftLocation || !onlocationchange) return

    saving = true
    actionError = null
    try {
      await onlocationchange(editingEntityId, draftLocation.latitude, draftLocation.longitude)
      leafletMarkers.get(editingEntityId)?.dragging?.disable()
      editingEntityId = null
      originalLocation = null
      draftLocation = null
    } catch {
      actionError = ui.saveError
    } finally {
      saving = false
    }
  }

  async function resetAutomaticLocation() {
    if (!selectedMarker || !onresetlocation) return

    saving = true
    actionError = null
    try {
      await onresetlocation(selectedMarker.entityId)
    } catch {
      actionError = ui.saveError
    } finally {
      saving = false
    }
  }

  $effect(() => {
    const firstMarkerId = markers[0]?.entityId ?? null
    if (!selectedEntityId || !markers.some((marker) => marker.entityId === selectedEntityId)) {
      selectedEntityId = firstMarkerId
    }
  })

  $effect(() => {
    const nextMarkers = markers
    if (!map || nextMarkers === renderedMarkers) return

    renderedMarkers = nextMarkers
    updateMarkers(nextMarkers)
  })

  $effect(() => {
    void visible

    if (!visible) return

    scheduleInvalidateMapSize()
  })
</script>

<div class="map-viewer" bind:this={rootEl} style="height: {height}">
  <div class="map-viewer__container" bind:this={mapContainer}></div>

  {#if markers.length === 0}
    <div class="map-viewer__empty">
      <p>{ui.empty}</p>
    </div>
  {:else if onlocationchange}
    <div class="map-viewer__editor">
      {#if markers.length > 1}
        <label>
          <span>{ui.location}</span>
          <select bind:value={selectedEntityId} disabled={editingEntityId !== null || saving}>
            {#each markers as marker (marker.entityId)}
              <option value={marker.entityId}>{marker.label}</option>
            {/each}
          </select>
        </label>
      {:else if selectedMarker}
        <strong>{selectedMarker.label}</strong>
      {/if}

      {#if editingEntityId}
        <p>{ui.dragHint}</p>
        {#if draftLocation}
          <output>{draftLocation.latitude.toFixed(6)}, {draftLocation.longitude.toFixed(6)}</output>
        {/if}
        <div class="map-viewer__actions">
          <button type="button" onclick={saveLocationEdit} disabled={!draftLocation || saving}>{ui.save}</button>
          <button type="button" onclick={cancelLocationEdit} disabled={saving}>{ui.cancel}</button>
        </div>
      {:else}
        <div class="map-viewer__actions">
          <button type="button" onclick={startLocationEdit} disabled={!selectedMarker || saving}>{ui.edit}</button>
          {#if selectedMarker?.hasManualLocation && onresetlocation}
            <button type="button" onclick={resetAutomaticLocation} disabled={saving}>{ui.reset}</button>
          {/if}
        </div>
      {/if}

      {#if actionError}
        <p class="map-viewer__error" role="alert">{actionError}</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .map-viewer {
    position: relative;
    width: 100%;
    min-width: 0;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm, 4px);
    overflow: hidden;
  }

  .map-viewer__container {
    width: 100%;
    height: 100%;
    min-height: 100%;
  }

  .map-viewer__empty {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--color-surface);
    pointer-events: none;
  }

  .map-viewer__empty p {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm, 0.875rem);
  }

  .map-viewer__editor {
    position: absolute;
    z-index: 1000;
    top: var(--space-2, 0.5rem);
    right: var(--space-2, 0.5rem);
    display: grid;
    gap: var(--space-2, 0.5rem);
    width: min(19rem, calc(100% - 1rem));
    padding: var(--space-3, 0.75rem);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm, 4px);
    background: color-mix(in srgb, var(--color-surface) 94%, transparent);
    box-shadow: var(--shadow-md, 0 8px 24px rgb(0 0 0 / 18%));
    backdrop-filter: blur(8px);
  }

  .map-viewer__editor label,
  .map-viewer__actions {
    display: flex;
    gap: var(--space-2, 0.5rem);
    align-items: center;
  }

  .map-viewer__editor label {
    justify-content: space-between;
  }

  .map-viewer__editor select {
    min-width: 0;
    max-width: 12rem;
  }

  .map-viewer__editor p,
  .map-viewer__editor output {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs, 0.75rem);
  }

  .map-viewer__editor output {
    font-family: var(--font-mono, monospace);
  }

  .map-viewer__actions {
    flex-wrap: wrap;
  }

  .map-viewer__actions button {
    padding: 0.35rem 0.65rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm, 4px);
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
    font-size: var(--font-size-xs, 0.75rem);
    cursor: pointer;
  }

  .map-viewer__actions button:disabled {
    cursor: not-allowed;
    opacity: 0.55;
  }

  .map-viewer__error {
    color: var(--color-danger, #b42318) !important;
  }
</style>
