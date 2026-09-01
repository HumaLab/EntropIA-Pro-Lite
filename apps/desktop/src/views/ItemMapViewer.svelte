<script lang="ts">
  import { MapViewer } from '@entropia/ui/components/MapViewer'
  import type { Entity, MapMarker } from '@entropia/ui'
  import type { I18nKey, I18nParams } from '$lib/i18n'

  let {
    entities,
    geoMarkers,
    visible,
    height,
    translate,
    onSaveMapLocation,
    onResetMapLocation,
  }: {
    entities: Entity[]
    geoMarkers: MapMarker[]
    visible: boolean
    height: string
    translate: (key: I18nKey, params?: I18nParams) => string
    onSaveMapLocation: (entityId: string, latitude: number, longitude: number) => void | Promise<void>
    onResetMapLocation: (entityId: string) => void | Promise<void>
  } = $props()

  let locationOptions = $derived(
    entities
      .filter((entity) => entity.entityType === 'place')
      .map((entity) => ({ entityId: entity.id, label: entity.value }))
  )
</script>

<div class="item-map-viewer" style:height data-testid="item-map-viewer" data-height={height}>
  <MapViewer
    markers={geoMarkers}
    {locationOptions}
    height="100%"
    {visible}
    labels={{
      empty: translate('item.map.empty'),
      location: translate('item.map.location'),
      edit: translate('item.map.edit'),
      create: translate('item.map.create'),
      save: translate('item.map.save'),
      cancel: translate('item.map.cancel'),
      reset: translate('item.map.reset'),
      dragHint: translate('item.map.dragHint'),
      saveError: translate('item.map.saveError'),
    }}
    onlocationchange={onSaveMapLocation}
    onresetlocation={onResetMapLocation}
  />
</div>

<style>
  .item-map-viewer {
    width: 100%;
    min-width: 0;
    min-height: 0;
  }

  .item-map-viewer :global(.map-viewer) {
    box-sizing: border-box;
  }
</style>
