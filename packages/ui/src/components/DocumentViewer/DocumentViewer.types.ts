import type {
  MarkupTool,
  PdfMarkup,
  PdfMarkupDraft,
  PdfMarkupGeometryChange,
  PdfMarkupLabels,
} from '../PdfMarkup/types'

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
  /**
   * PDF only, opt-in (Biblioteca): Zotero-style markups drawn over the page.
   * Leaving it undefined keeps the viewer exactly as Collections uses it.
   */
  markups?: PdfMarkup[]
  /** Markup tools offered; crop, erase and rotate are never among them. */
  allowedTools?: readonly MarkupTool[]
  /** Swatches of the color pickers; the default keeps Collections' tokens. */
  palette?: readonly string[]
  /** PDF only, opt-in: a pdf.js text layer so text can be selected. */
  textSelection?: boolean
  markupTool?: MarkupTool
  markupColor?: string
  selectedMarkupId?: string | null
  markupLabels?: Partial<PdfMarkupLabels>
  onMarkupCreate?: (draft: PdfMarkupDraft) => void
  /** A note, area or text markup was moved or resized. */
  onMarkupChange?: (change: PdfMarkupGeometryChange) => void
  onMarkupDelete?: (id: string) => void
  onMarkupSelect?: (id: string | null) => void
  onMarkupToolChange?: (tool: MarkupTool) => void
  onMarkupColorChange?: (color: string) => void
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
