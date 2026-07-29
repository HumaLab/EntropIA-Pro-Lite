export interface MapMarker {
  entityId: string
  label: string
  latitude: number
  longitude: number
  itemId?: string
  itemTitle?: string
  hasManualLocation?: boolean
}

export interface MapLocationOption {
  entityId: string
  label: string
}

export interface MapViewerLabels {
  empty: string
  location: string
  edit: string
  create: string
  save: string
  cancel: string
  reset: string
  dragHint: string
  saveError: string
}

export interface MapViewerProps {
  markers: MapMarker[]
  locationOptions?: MapLocationOption[]
  height?: string
  visible?: boolean
  onmarkerclick?: (marker: MapMarker) => void
  onlocationchange?: (entityId: string, latitude: number, longitude: number) => void | Promise<void>
  onresetlocation?: (entityId: string) => void | Promise<void>
  labels?: Partial<MapViewerLabels>
}
