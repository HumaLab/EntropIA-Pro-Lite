export type AnnotationKind = 'rectangle' | 'underline'
export type DocumentEditKind = 'crop' | 'erase' | 'rotation'
export type ViewerAnnotationKind = AnnotationKind | DocumentEditKind
export type AnnotationTool = 'select' | AnnotationKind

export type EditTool = 'none' | 'crop' | 'erase'

export interface ImageEditResult {
  path: string
  width: number
  height: number
  format_changed: boolean
  /** Path of the file before the edit (kept on disk for undo) */
  previous_path: string
}

export interface ViewerAnnotation {
  id: string
  assetId: string
  page: number
  kind: ViewerAnnotationKind
  color: string
  x: number
  y: number
  width: number
  height: number
  createdAt: number
  updatedAt: number
}

export interface ViewerLayoutRegion {
  id: string
  blockId: string
  label: string
  x: number
  y: number
  width: number
  height: number
  matchSource?: 'region' | 'block'
}

export type ViewerType = 'image' | 'pdf' | 'audio'

export interface DocumentViewerProps {
  path: string
  type: ViewerType
  assetUrl: string
  annotations?: ViewerAnnotation[]
  selectedAnnotationId?: string | null
  annotationTool?: AnnotationTool
  annotationColor?: string
  editTool?: EditTool
  canUndo?: boolean
  canRedo?: boolean
  readOnly?: boolean
  currentPage?: number
  /**
   * Opt-in (Biblioteca): while the viewer's container measures 0×0 (the tab
   * is hidden but stays mounted), a ResizeObserver notification must not draw
   * — pdfFitScale falls back to scale 1 there and renders a full-size page
   * nobody sees. Showing the container again renders exactly once.
   */
  pauseWhenHidden?: boolean
  layoutRegions?: ViewerLayoutRegion[]
  showLayoutOverlay?: boolean
  hoveredLayoutRegionId?: string | null
  selectedLayoutRegionId?: string | null
  layoutReferenceWidth?: number
  layoutReferenceHeight?: number
  onAnnotationsChange?: (annotations: ViewerAnnotation[]) => void
  onSelectedAnnotationIdChange?: (annotationId: string | null) => void
  onLayoutRegionHoverChange?: (regionId: string | null) => void
  onLayoutRegionSelect?: (regionId: string) => void
  onAnnotationToolChange?: (tool: AnnotationTool) => void
  onAnnotationColorChange?: (color: string) => void
  onEditSelect?: (region: { x: number; y: number; width: number; height: number }) => void
  onEditToolChange?: (tool: EditTool) => void
  onRotateLeft?: () => void
  onRotateRight?: () => void
  onFineRotateCommit?: (degrees: number) => void | Promise<void>
  onUndo?: () => void
  onRedo?: () => void
  onDuplicateAsset?: () => void | Promise<void>
  duplicateAssetDisabled?: boolean
  onPageChange?: (page: number, totalPages: number) => void
  onDimensionsChange?: (dimensions: { width: number; height: number }) => void
  audioFallbackBlobLoader?: (nativePath: string) => Promise<Blob>
  /** Audio only: second to position the recording at (a citation of a transcript). */
  audioStartAtSeconds?: number | null
  labels?: Partial<DocumentViewerLabels>
  annotationToolbarLabels?: Record<string, unknown>
}

export interface DocumentViewerLabels {
  imageAlt: string
  imageOverlayAriaLabel: string
  audioSkipBack: string
  audioPlay: string
  audioPause: string
  audioSkipForward: string
  audioSeek: string
  audioVolume: string
  pdfLoading: string
  pdfLoadError: string
  pdfRenderError: string
  pdfPreviousPage: string
  pdfNextPage: string
  pdfZoomOut: string
  pdfZoomIn: string
  layoutOverlayAriaLabel: string
  layoutRegionAriaLabel: (label: string) => string
  annotationAriaLabel: (id: string) => string
  cropRegionAriaLabel: string
  eraseRegionAriaLabel: string
}
