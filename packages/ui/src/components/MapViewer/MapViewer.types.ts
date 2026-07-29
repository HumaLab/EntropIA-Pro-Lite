export interface MapMarker {
  entityId: string
  label: string
  latitude: number
  longitude: number
  itemId?: string
  itemTitle?: string
  hasManualLocation?: boolean
}

export interface MapViewerLabels {
  empty: string
  location: string
  edit: string
  save: string
  cancel: string
  reset: string
  dragHint: string
  saveError: string
}

export interface MapViewerProps {
  markers: MapMarker[]
  height?: string
  visible?: boolean
  onmarkerclick?: (marker: MapMarker) => void
  onlocationchange?: (entityId: string, latitude: number, longitude: number) => void | Promise<void>
  onresetlocation?: (entityId: string) => void | Promise<void>
  labels?: Partial<MapViewerLabels>
}
