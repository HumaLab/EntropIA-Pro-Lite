<script lang="ts">
  import { tooltip } from '../Tooltip/tooltip'
  import type { MapViewerLabels, MapViewerProps } from './MapViewer.types'
  import { onMount, onDestroy, tick } from 'svelte'
  import ActionIcon from '../Button/ActionIcon.svelte'
  import Button from '../Button/Button.svelte'
  import IconButton from '../IconButton/IconButton.svelte'
  import Select from '../SelectTrigger/Select.svelte'
  import type { SelectOption } from '../SelectTrigger/SelectTrigger.types'
  import L from 'leaflet'
  import 'leaflet/dist/leaflet.css'
  import markerIconUrl from 'leaflet/dist/images/marker-icon.png'
  import markerIcon2xUrl from 'leaflet/dist/images/marker-icon-2x.png'
  import markerShadowUrl from 'leaflet/dist/images/marker-shadow.png'

  const defaultLabels: MapViewerLabels = {
    empty: 'No geocoded locations',
    location: 'Location',
    edit: 'Edit location',
    create: 'Create location',
    save: 'Save',
    cancel: 'Cancel',
    reset: 'Reset automatic location',
    dragHint: 'Drag the marker to its correct location, then save the change.',
    saveError: 'Could not save the location.',
  }

  let {
    markers = [],
    locationOptions = [],
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
  let availableLocations = $derived(
    locationOptions.length > 0
      ? locationOptions
      : markers.map((marker) => ({ entityId: marker.entityId, label: marker.label }))
  )
  let selectedLocation = $derived(
    availableLocations.find((location) => location.entityId === selectedEntityId) ?? null
  )
  let selectedMarker = $derived(
    markers.find((marker) => marker.entityId === selectedEntityId) ?? null
  )
  /** The location picker: the package Select (a SelectTrigger opening a
   *  ToolbarMenu of radio entries), never a native select — the operating
   *  system would draw that outside the app's theme. */
  let locationSelectOptions = $derived<SelectOption[]>(
    availableLocations.map((location) => ({ value: location.entityId, label: location.label }))
  )

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
        captureDraggedLocation(marker.entityId, leafletMarker)
      })
    }

    if (bounds.length > 0) {
      const group = L.latLngBounds(bounds)
      map.fitBounds(group, { padding: [30, 30], maxZoom: 12 })
    }
  }

  function captureDraggedLocation(entityId: string, leafletMarker: L.Marker) {
    if (editingEntityId !== entityId) return
    const next = leafletMarker.getLatLng()
    draftLocation = { latitude: next.lat, longitude: next.lng }
    actionError = null
  }

  function startLocationEdit() {
    if (!selectedLocation || !map || !markerLayer) return

    editingEntityId = selectedLocation.entityId
    draftLocation = null
    actionError = null

    if (selectedMarker) {
      originalLocation = {
        latitude: selectedMarker.latitude,
        longitude: selectedMarker.longitude,
      }
      leafletMarkers.get(selectedMarker.entityId)?.dragging?.enable()
      return
    }

    originalLocation = null
    leafletMarkers.get(selectedLocation.entityId)?.remove()
    const leafletMarker = L.marker(map.getCenter(), { draggable: true }).addTo(markerLayer)
    leafletMarkers.set(selectedLocation.entityId, leafletMarker)
    leafletMarker.bindPopup(`<strong>${selectedLocation.label}</strong>`).openPopup()
    leafletMarker.on('dragend', () =>
      captureDraggedLocation(selectedLocation.entityId, leafletMarker)
    )
  }

  function cancelLocationEdit() {
    if (editingEntityId && originalLocation) {
      const leafletMarker = leafletMarkers.get(editingEntityId)
      leafletMarker?.setLatLng([originalLocation.latitude, originalLocation.longitude])
      leafletMarker?.dragging?.disable()
    } else if (editingEntityId) {
      leafletMarkers.get(editingEntityId)?.remove()
      leafletMarkers.delete(editingEntityId)
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

  function selectLocation(entityId: string) {
    selectedEntityId = entityId
    actionError = null
  }

  $effect(() => {
    const firstMarkerId = availableLocations[0]?.entityId ?? null
    if (
      !selectedEntityId ||
      !availableLocations.some((location) => location.entityId === selectedEntityId)
    ) {
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

  {#if availableLocations.length === 0}
    <div class="map-viewer__empty">
      <p>{ui.empty}</p>
    </div>
  {:else if onlocationchange}
    <div class="map-viewer__editor">
      <div class="map-viewer__location-row">
        <span
          class="map-viewer__location-icon"
          role="img"
          aria-label={ui.location}
          use:tooltip={ui.location}
        >
          <ActionIcon name="map-pin" size={16} />
        </span>

        {#if availableLocations.length > 1}
          <div class="map-viewer__location-picker">
            <Select
              label={ui.location}
              aria-label={ui.location}
              value={selectedEntityId ?? ''}
              options={locationSelectOptions}
              size="sm"
              block
              disabled={editingEntityId !== null || saving}
              onchange={selectLocation}
            />
          </div>
        {:else if selectedLocation}
          <strong>{selectedLocation.label}</strong>
        {/if}

        {#if !editingEntityId}
          {#if selectedMarker}
            <IconButton
              variant="secondary"
              size="md"
              label={ui.edit}
              title={ui.edit}
              onclick={startLocationEdit}
              disabled={!selectedLocation || saving}
            >
              <ActionIcon name="map-pin-pen" size={16} />
            </IconButton>
          {:else}
            <Button
              variant="secondary"
              size="sm"
              onclick={startLocationEdit}
              disabled={!selectedLocation || saving}
            >
              {ui.create}
            </Button>
          {/if}
          {#if selectedMarker?.hasManualLocation && onresetlocation}
            <Button
              variant="secondary"
              size="sm"
              onclick={resetAutomaticLocation}
              disabled={saving}
            >
              {ui.reset}
            </Button>
          {/if}
        {/if}
      </div>

      {#if editingEntityId}
        <p>{ui.dragHint}</p>
        {#if draftLocation}
          <output>{draftLocation.latitude.toFixed(6)}, {draftLocation.longitude.toFixed(6)}</output>
        {/if}
        <div class="map-viewer__actions">
          <Button
            variant="primary"
            size="sm"
            onclick={saveLocationEdit}
            disabled={!draftLocation || saving}
          >
            {ui.save}
          </Button>
          <Button variant="secondary" size="sm" onclick={cancelLocationEdit} disabled={saving}>
            {ui.cancel}
          </Button>
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
    border-radius: var(--radius-surface);
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
    font-size: var(--font-size-sm);
  }

  /* Leaflet stacks its own zoom and attribution controls at 1000 in the same
     stacking context, so the editor takes the dialog layer to stay above them. */
  .map-viewer__editor {
    position: absolute;
    z-index: var(--z-dialog);
    top: var(--space-2);
    right: var(--space-2);
    display: grid;
    gap: var(--space-2);
    width: min(19rem, calc(100% - 2 * var(--space-2)));
    box-sizing: border-box;
    padding: var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-surface);
    background: color-mix(in srgb, var(--color-surface) 94%, transparent);
    box-shadow: var(--shadow-md);
    backdrop-filter: blur(8px);
  }

  .map-viewer__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
  }

  .map-viewer__location-row {
    display: flex;
    flex-wrap: nowrap;
    gap: var(--space-2);
    align-items: center;
    width: 100%;
    min-width: 0;
  }

  .map-viewer__location-icon {
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    justify-content: center;
    color: var(--color-text-primary);
  }

  /* The picker's cell takes the row's free width and may shrink below its
     content, so a long place name ellipsizes inside the narrow editor instead
     of pushing the edit control out. The Select spans the cell (block). */
  .map-viewer__location-picker {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    flex: 1 1 auto;
    min-width: 0;
  }

  .map-viewer__location-row strong {
    flex: 1 1 auto;
    min-width: 0;
  }

  .map-viewer__editor p,
  .map-viewer__editor output {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .map-viewer__editor output {
    font-family: var(--font-mono);
  }

  .map-viewer__editor .map-viewer__error {
    color: var(--color-danger);
  }
</style>
