<script lang="ts">
  import type { PDFDocumentLoadingTask, PDFDocumentProxy, RenderTask } from 'pdfjs-dist'
  import AnnotationToolbar from '../AnnotationToolbar/AnnotationToolbar.svelte'
  import AudioPlayer from '../AudioPlayer/AudioPlayer.svelte'
  import type { DocumentViewerLabels, DocumentViewerProps } from './DocumentViewer.types'
  import type { AnnotationTool, EditTool, ViewerAnnotation } from './DocumentViewer.types'

  let {
    path: _path,
    type,
    assetUrl,
    annotations = [],
    layoutRegions = [],
    showLayoutOverlay = false,
    selectedAnnotationId = null,
    hoveredLayoutRegionId = null,
    selectedLayoutRegionId = null,
    annotationTool = 'select',
    annotationColor = 'var(--color-accent)',
    editTool = 'none',
    canUndo = false,
    canRedo = false,
    readOnly = false,
    currentPage = 1,
    layoutReferenceWidth = 0,
    layoutReferenceHeight = 0,
    onAnnotationsChange = () => {},
    onSelectedAnnotationIdChange = () => {},
    onLayoutRegionHoverChange = () => {},
    onLayoutRegionSelect = () => {},
    onAnnotationToolChange = () => {},
    onAnnotationColorChange = () => {},
    onEditSelect = () => {},
    onEditToolChange = () => {},
    onRotateLeft = () => {},
    onRotateRight = () => {},
    onFineRotateCommit = () => {},
    onUndo = () => {},
    onRedo = () => {},
    onPageChange = () => {},
    onDimensionsChange = () => {},
    audioFallbackBlobLoader,
    labels: labelsProp = {},
    annotationToolbarLabels = {},
  }: DocumentViewerProps = $props()

  const defaultLabels: DocumentViewerLabels = {
    imageAlt: 'Document',
    imageOverlayAriaLabel: 'Image annotation overlay',
    audioSkipBack: 'Skip back 5 seconds',
    audioPlay: 'Play',
    audioPause: 'Pause',
    audioSkipForward: 'Skip forward 5 seconds',
    audioSeek: 'Seek',
    audioVolume: 'Volume',
    pdfLoading: 'Loading PDF...',
    pdfLoadError: 'Failed to load PDF',
    pdfRenderError: 'Failed to render page',
    pdfPreviousPage: 'Previous page',
    pdfNextPage: 'Next page',
    pdfZoomOut: 'Zoom out',
    pdfZoomIn: 'Zoom in',
    layoutOverlayAriaLabel: 'Document layout overlay',
    layoutRegionAriaLabel: (label: string) => `Layout region ${label}`,
    annotationAriaLabel: (id: string) => `Select annotation ${id}`,
    cropRegionAriaLabel: 'Crop region',
    eraseRegionAriaLabel: 'Erase region',
  }

  const labels = $derived({ ...defaultLabels, ...labelsProp })
  const audioPlayerFallbackBlobLoader = $derived(
    audioFallbackBlobLoader ? () => audioFallbackBlobLoader(_path) : undefined
  )

  const presetColors = [
    { value: 'var(--color-accent)', label: 'Accent' },
    { value: 'var(--color-success)', label: 'Success' },
    { value: 'var(--color-warning)', label: 'Warning' },
    { value: 'var(--color-danger)', label: 'Danger' },
  ]
  const MIN_DRAW_PX = 6
  const UNDERLINE_STROKE_PX = 2
  const UNDERLINE_HITBOX_NORMALIZED = 0.02
  const MIN_ZOOM = 0.4
  const MAX_ZOOM = 3.0
  const ZOOM_STEP = 0.1
  const MIN_FINE_ROTATION_DEGREES = -30
  const MAX_FINE_ROTATION_DEGREES = 30
  const SIZE_DEADBAND_PX = 1.5

  // PDF state
  let pdfPage = $state(1)
  let totalPages = $state(0)
  let pdfZoom = $state(1.0)
  let loading = $state(false)
  let error = $state<string | null>(null)

  let canvasEl: HTMLCanvasElement | undefined = $state()
  let pdfScrollEl: HTMLDivElement | undefined = $state()
  let imgEl: HTMLImageElement | undefined = $state()
  let containerEl: HTMLElement | undefined = $state()
  let pdfDoc: PDFDocumentProxy | null = null
  let pdfCanvasW = $state(0)
  let pdfCanvasH = $state(0)
  let renderRequestId = 0
  let activeRenderTask: RenderTask | null = null
  let loadRequestId = 0
  let activeLoadingTask: PDFDocumentLoadingTask | null = null

  // Image geometry — natural (intrinsic) dimensions of the source file
  let naturalW = $state(0)
  let naturalH = $state(0)

  // Container inner dimensions (content area, padding excluded)
  let containerW = $state(0)
  let containerH = $state(0)

  // Manual zoom multiplier applied on top of auto-fit sizing.
  let imageZoom = $state(1.0)
  let imageRotation = $state(0)
  let containerMeasureFrame = $state<number | null>(null)
  let panToolActive = $state(false)
  let isPanning = $state(false)
  let panDrag: {
    pointerId: number
    startClientX: number
    startClientY: number
    startScrollLeft: number
    startScrollTop: number
  } | null = null

  let draft = $state<{
    startX: number
    startY: number
    currentX: number
    currentY: number
    kind: Exclude<AnnotationTool, 'select'>
  } | null>(null)

  // Edit draft: temporary rectangle drawn while crop/erase is active
  let editDraft = $state<{
    startX: number
    startY: number
    currentX: number
    currentY: number
  } | null>(null)

  // ── Derived geometry ──────────────────────────────────────────────
  // fitScale: the scale that makes the image fit inside the container
  const fitScale = $derived(
    naturalW > 0 && naturalH > 0 && containerW > 0 && containerH > 0
      ? Math.min(containerW / naturalW, containerH / naturalH)
      : 1
  )

  // Display dimensions: what the image (and SVG overlay) measure on screen
  const baseDisplayW = $derived(Math.round(naturalW * fitScale))
  const baseDisplayH = $derived(Math.round(naturalH * fitScale))
  const displayW = $derived(Math.round(baseDisplayW * imageZoom))
  const displayH = $derived(Math.round(baseDisplayH * imageZoom))

  const hasRenderableBounds = $derived(naturalW > 0 && naturalH > 0)

  const canZoomIn = $derived(imageZoom < MAX_ZOOM - 0.001)
  const canZoomOut = $derived(imageZoom > MIN_ZOOM + 0.001)
  const canFineRotateLeft = $derived(imageRotation > MIN_FINE_ROTATION_DEGREES)
  const canFineRotateRight = $derived(imageRotation < MAX_FINE_ROTATION_DEGREES)

  const canPdfZoomIn = $derived(pdfZoom < MAX_ZOOM - 0.001)
  const canPdfZoomOut = $derived(pdfZoom > MIN_ZOOM + 0.001)
  const visibleAnnotations = $derived(
    annotations.filter(
      (annotation) => annotation.kind === 'rectangle' || annotation.kind === 'underline'
    )
  )
  const pdfErasures = $derived(annotations.filter((annotation) => annotation.kind === 'erase'))
  const pdfCrop = $derived(
    annotations.find((annotation) => annotation.kind === 'crop') ?? {
      x: 0,
      y: 0,
      width: 1,
      height: 1,
    }
  )
  const pdfRotationEdit = $derived(annotations.find((annotation) => annotation.kind === 'rotation'))
  const pdfQuarterTurns = $derived(Math.round(pdfRotationEdit?.x ?? 0))
  const pdfFineRotation = $derived(pdfRotationEdit?.y ?? 0)
  const pdfTotalRotation = $derived(pdfQuarterTurns * 90 + imageRotation)
  const pdfCropW = $derived(Math.max(1, Math.round(pdfCanvasW * pdfCrop.width)))
  const pdfCropH = $derived(Math.max(1, Math.round(pdfCanvasH * pdfCrop.height)))
  const pdfCropOffsetX = $derived(Math.round(pdfCanvasW * pdfCrop.x))
  const pdfCropOffsetY = $derived(Math.round(pdfCanvasH * pdfCrop.y))
  const pdfRotationRadians = $derived((pdfTotalRotation * Math.PI) / 180)
  const pdfRotatedW = $derived(
    Math.ceil(
      Math.abs(pdfCropW * Math.cos(pdfRotationRadians)) +
        Math.abs(pdfCropH * Math.sin(pdfRotationRadians))
    )
  )
  const pdfRotatedH = $derived(
    Math.ceil(
      Math.abs(pdfCropW * Math.sin(pdfRotationRadians)) +
        Math.abs(pdfCropH * Math.cos(pdfRotationRadians))
    )
  )
  const activeDisplayW = $derived(type === 'pdf' ? pdfCanvasW : displayW)
  const activeDisplayH = $derived(type === 'pdf' ? pdfCanvasH : displayH)

  const overlayCursor = $derived(
    isPanning
      ? 'grabbing'
      : panToolActive
        ? 'grab'
        : editTool !== 'none'
          ? 'crosshair'
          : annotationTool === 'select'
            ? 'default'
            : 'crosshair'
  )

  const layoutOverlayInteractive = $derived(
    annotationTool === 'select' && editTool === 'none' && !panToolActive
  )
  const layoutViewportW = $derived(type === 'image' ? naturalW : layoutReferenceWidth)
  const layoutViewportH = $derived(type === 'image' ? naturalH : layoutReferenceHeight)
  const layoutDisplayW = $derived(type === 'image' ? displayW : pdfCanvasW)
  const layoutDisplayH = $derived(type === 'image' ? displayH : pdfCanvasH)
  const canRenderLayoutOverlay = $derived(
    showLayoutOverlay &&
      layoutRegions.length > 0 &&
      layoutViewportW > 0 &&
      layoutViewportH > 0 &&
      layoutDisplayW > 0 &&
      layoutDisplayH > 0 &&
      type !== 'audio'
  )

  // ── Helpers ────────────────────────────────────────────────────────
  function clamp01(value: number) {
    return Math.max(0, Math.min(1, value))
  }

  function round(value: number) {
    return Number(value.toFixed(4))
  }

  function clampZoom(value: number) {
    return Number(Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, value)).toFixed(2))
  }

  function clampFineRotation(value: number) {
    return Math.min(MAX_FINE_ROTATION_DEGREES, Math.max(MIN_FINE_ROTATION_DEGREES, value))
  }

  function readPx(value: string) {
    const parsed = Number.parseFloat(value)
    return Number.isFinite(parsed) ? parsed : 0
  }

  function isPracticallyEqual(next: number, prev: number, epsilon = SIZE_DEADBAND_PX) {
    return Math.abs(next - prev) < epsilon
  }

  function createLocalAnnotation(
    nextTool: Exclude<AnnotationTool, 'select'>,
    x: number,
    y: number,
    width: number,
    height: number
  ): ViewerAnnotation {
    const now = Date.now()
    return {
      id: crypto.randomUUID(),
      assetId: '',
      page: 1,
      kind: nextTool,
      color: annotationColor,
      x,
      y,
      width,
      height,
      createdAt: now,
      updatedAt: now,
    }
  }

  /** Convert normalized [0,1] → natural-image pixels (SVG viewBox space) */
  function px(value: number, axis: 'x' | 'y') {
    const dimension = axis === 'x' ? naturalW : naturalH
    return Math.round(value * dimension)
  }

  function layoutPx(value: number, axis: 'x' | 'y') {
    const source = axis === 'x' ? layoutViewportW : layoutViewportH
    const target = axis === 'x' ? naturalW : naturalH
    return source > 0 ? (value / source) * target : value
  }

  /** Convert a viewport PointerEvent to normalized [0,1] coordinates.
   *  Prefer the SVG screen matrix so rotation/zoom transforms are inverted
   *  before translating the pointer to natural-image coordinates. */
  function getNormalizedPoint(event: PointerEvent) {
    const target = event.currentTarget as SVGSVGElement
    const ctm = target.getScreenCTM?.()

    if (ctm && typeof ctm.inverse === 'function' && typeof target.createSVGPoint === 'function') {
      const point = target.createSVGPoint()
      point.x = event.clientX
      point.y = event.clientY

      if (typeof point.matrixTransform === 'function') {
        const localPoint = point.matrixTransform(ctm.inverse())

        if (Number.isFinite(localPoint.x) && Number.isFinite(localPoint.y)) {
          return {
            x: clamp01(localPoint.x / Math.max(naturalW, 1)),
            y: clamp01(localPoint.y / Math.max(naturalH, 1)),
          }
        }
      }
    }

    const rect = target.getBoundingClientRect()
    if (rect.width === 0 || rect.height === 0) return null

    return {
      x: clamp01((event.clientX - rect.left) / rect.width),
      y: clamp01((event.clientY - rect.top) / rect.height),
    }
  }

  function toDraftBox(currentDraft: NonNullable<typeof draft>) {
    const x = Math.min(currentDraft.startX, currentDraft.currentX)
    const y = Math.min(currentDraft.startY, currentDraft.currentY)
    return {
      x: round(x),
      y: round(y),
      width: round(Math.abs(currentDraft.currentX - currentDraft.startX)),
      height: round(Math.abs(currentDraft.currentY - currentDraft.startY)),
    }
  }

  function meetsMinimumSize(box: { width: number; height: number }, kind: AnnotationTool) {
    // Minimum size in display pixels, converted to normalized coords via display dimensions
    const minNormW = MIN_DRAW_PX / Math.max(activeDisplayW, 1)
    const minNormH = MIN_DRAW_PX / Math.max(activeDisplayH, 1)
    if (kind === 'underline') {
      return box.width >= minNormW
    }
    return box.width >= minNormW && box.height >= minNormH
  }

  // ── Edit draft helpers ───────────────────────────────────────────────
  function toEditBox(d: NonNullable<typeof editDraft>) {
    const x = Math.min(d.startX, d.currentX)
    const y = Math.min(d.startY, d.currentY)
    return {
      x: round(x),
      y: round(y),
      width: round(Math.abs(d.currentX - d.startX)),
      height: round(Math.abs(d.currentY - d.startY)),
    }
  }

  // ── Measurement ────────────────────────────────────────────────────
  function measureImage() {
    if (!imgEl) return
    // Skip measurement if the image hasn't loaded yet (e.g. after {#key} re-creation).
    // This preserves the previous dimensions until onload fires, avoiding a flash
    // where the overlay disappears because naturalWidth/naturalHeight are 0.
    if (!imgEl.complete || imgEl.naturalWidth === 0) return
    const nextNaturalW = imgEl.naturalWidth
    const nextNaturalH = imgEl.naturalHeight

    if (nextNaturalW === naturalW && nextNaturalH === naturalH) return

    naturalW = nextNaturalW
    naturalH = nextNaturalH
    onDimensionsChange({ width: nextNaturalW, height: nextNaturalH })
  }

  function measureContainer() {
    if (!containerEl) return
    const style = getComputedStyle(containerEl)
    const padX = readPx(style.paddingLeft) + readPx(style.paddingRight)
    const padY = readPx(style.paddingTop) + readPx(style.paddingBottom)
    const rect = containerEl.getBoundingClientRect()
    const nextContainerW = Math.max(0, Number((rect.width - padX).toFixed(2)))
    const nextContainerH = Math.max(0, Number((rect.height - padY).toFixed(2)))

    const widthChanged = !isPracticallyEqual(nextContainerW, containerW)
    const heightChanged = !isPracticallyEqual(nextContainerH, containerH)

    if (!widthChanged && !heightChanged) return

    if (widthChanged) containerW = nextContainerW
    if (heightChanged) containerH = nextContainerH
  }

  function scheduleContainerMeasure() {
    if (containerMeasureFrame !== null) return

    containerMeasureFrame = requestAnimationFrame(() => {
      containerMeasureFrame = null
      measureContainer()
    })
  }

  function cancelScheduledContainerMeasure() {
    if (containerMeasureFrame === null) return
    cancelAnimationFrame(containerMeasureFrame)
    containerMeasureFrame = null
  }

  // ── Handlers ────────────────────────────────────────────────────────
  function handleToolbarToolChange(tool: AnnotationTool) {
    panToolActive = false
    panDrag = null
    isPanning = false
    onAnnotationToolChange(tool)
    // Reset edit tool when switching to annotation mode
    if (tool !== 'select') {
      onEditToolChange('none')
    }
    if (tool !== annotationTool) {
      draft = null
    }
  }

  function handleToolbarEditToolChange(tool: EditTool) {
    panToolActive = false
    panDrag = null
    isPanning = false
    onEditToolChange(tool)
  }

  function handlePanToggle() {
    const nextActive = !panToolActive
    panToolActive = nextActive
    panDrag = null
    isPanning = false
    draft = null
    editDraft = null
    if (nextActive) {
      onEditToolChange('none')
      onAnnotationToolChange('select')
      onSelectedAnnotationIdChange(null)
    }
  }

  function handleToolbarColorChange(color: string) {
    onAnnotationColorChange(color)
    if (!selectedAnnotationId) return
    onAnnotationsChange(
      annotations.map((a) =>
        a.id === selectedAnnotationId ? { ...a, color, updatedAt: Date.now() } : a
      )
    )
  }

  function getLayoutRegionFill(
    region: (typeof layoutRegions)[number],
    isSelected: boolean,
    isHovered: boolean
  ) {
    if (region.matchSource === 'block') {
      return isSelected
        ? 'rgba(251, 191, 36, 0.24)'
        : isHovered
          ? 'rgba(251, 191, 36, 0.18)'
          : 'rgba(251, 191, 36, 0.08)'
    }

    return isSelected
      ? 'rgba(34, 211, 238, 0.2)'
      : isHovered
        ? 'rgba(250, 204, 21, 0.16)'
        : 'rgba(34, 211, 238, 0.08)'
  }

  function getLayoutRegionStroke(
    region: (typeof layoutRegions)[number],
    isSelected: boolean,
    isHovered: boolean
  ) {
    if (region.matchSource === 'block') {
      return isSelected
        ? 'rgb(245, 158, 11)'
        : isHovered
          ? 'rgb(251, 191, 36)'
          : 'rgba(245, 158, 11, 0.9)'
    }

    return isSelected
      ? 'rgb(34, 211, 238)'
      : isHovered
        ? 'rgb(250, 204, 21)'
        : 'rgba(34, 211, 238, 0.8)'
  }

  function getLayoutRegionStrokeWidth(
    region: (typeof layoutRegions)[number],
    isSelected: boolean,
    isHovered: boolean
  ) {
    if (isSelected) return region.matchSource === 'block' ? 3 : 2.5
    if (isHovered) return region.matchSource === 'block' ? 2.4 : 2
    return region.matchSource === 'block' ? 1.75 : 1.25
  }

  function getLayoutRegionStrokeDasharray(
    region: (typeof layoutRegions)[number],
    isSelected: boolean
  ) {
    if (isSelected) {
      return region.matchSource === 'block' ? '10 4' : '0'
    }

    return region.matchSource === 'block' ? '10 6' : '6 4'
  }

  function handleDeleteSelected() {
    if (!selectedAnnotationId) return
    onAnnotationsChange(annotations.filter((a) => a.id !== selectedAnnotationId))
    onSelectedAnnotationIdChange(null)
  }

  function captureOverlayPointer(event: PointerEvent) {
    ;(event.currentTarget as SVGSVGElement).setPointerCapture?.(event.pointerId)
  }

  function releaseOverlayPointer(event: PointerEvent) {
    const target = event.currentTarget as SVGSVGElement
    if (!target.hasPointerCapture?.(event.pointerId)) return
    target.releasePointerCapture?.(event.pointerId)
  }

  function cancelDrafts(event: PointerEvent) {
    releaseOverlayPointer(event)
    panDrag = null
    isPanning = false
    draft = null
    editDraft = null
  }

  function handleOverlayPointerDown(event: PointerEvent) {
    if (!hasRenderableBounds || event.button !== 0) return

    const scrollViewport = type === 'pdf' ? pdfScrollEl : containerEl
    if (panToolActive && scrollViewport) {
      event.preventDefault()
      panDrag = {
        pointerId: event.pointerId,
        startClientX: event.clientX,
        startClientY: event.clientY,
        startScrollLeft: scrollViewport.scrollLeft,
        startScrollTop: scrollViewport.scrollTop,
      }
      isPanning = true
      captureOverlayPointer(event)
      return
    }

    const point = getNormalizedPoint(event)
    if (!point) return

    // Edit mode: start drawing an edit selection rectangle
    if (editTool !== 'none') {
      editDraft = {
        startX: point.x,
        startY: point.y,
        currentX: point.x,
        currentY: point.y,
      }
      captureOverlayPointer(event)
      return
    }

    if (annotationTool === 'select') {
      onSelectedAnnotationIdChange(null)
      return
    }

    draft = {
      startX: point.x,
      startY: point.y,
      currentX: point.x,
      currentY: point.y,
      kind: annotationTool,
    }
    captureOverlayPointer(event)
  }

  function handleOverlayPointerMove(event: PointerEvent) {
    const scrollViewport = type === 'pdf' ? pdfScrollEl : containerEl
    if (panDrag && scrollViewport) {
      event.preventDefault()
      scrollViewport.scrollLeft = panDrag.startScrollLeft + (panDrag.startClientX - event.clientX)
      scrollViewport.scrollTop = panDrag.startScrollTop + (panDrag.startClientY - event.clientY)
      return
    }

    if (editDraft) {
      const point = getNormalizedPoint(event)
      if (!point) return
      editDraft = {
        ...editDraft,
        currentX: point.x,
        currentY: point.y,
      }
      return
    }

    if (!draft) return
    const point = getNormalizedPoint(event)
    if (!point) return
    draft = {
      ...draft,
      currentX: point.x,
      currentY: draft.kind === 'underline' ? draft.startY : point.y,
    }
  }

  function finishDraft(event?: PointerEvent) {
    if (event) releaseOverlayPointer(event)

    if (panDrag) {
      panDrag = null
      isPanning = false
      return
    }

    // Handle edit draft completion
    if (editDraft) {
      const box = toEditBox(editDraft)
      editDraft = null
      const minSize = MIN_DRAW_PX / Math.max(activeDisplayW, 1)
      const minHeight = MIN_DRAW_PX / Math.max(activeDisplayH, 1)
      if (box.width < minSize || box.height < minHeight) return
      onEditSelect({ x: box.x, y: box.y, width: box.width, height: box.height })
      return
    }

    if (!draft) return
    const kind = draft.kind

    if (kind === 'underline') {
      const x = round(Math.min(draft.startX, draft.currentX))
      const width = round(Math.abs(draft.currentX - draft.startX))
      const y = round(clamp01(draft.startY - UNDERLINE_HITBOX_NORMALIZED / 2))
      const minWidth = MIN_DRAW_PX / Math.max(activeDisplayW, 1)
      const clampedWidth = round(Math.min(width, 1 - x))

      draft = null
      if (clampedWidth < minWidth) return

      onAnnotationsChange([
        ...annotations,
        createLocalAnnotation('underline', x, y, clampedWidth, UNDERLINE_HITBOX_NORMALIZED),
      ])
      onSelectedAnnotationIdChange(null)
      return
    }

    const box = toDraftBox(draft)
    draft = null
    if (!meetsMinimumSize(box, kind)) return

    onAnnotationsChange([
      ...annotations,
      createLocalAnnotation(kind, box.x, box.y, box.width, box.height),
    ])
    onSelectedAnnotationIdChange(null)
  }

  function handleShapeClick(annotationId: string) {
    onSelectedAnnotationIdChange(annotationId)
  }

  function handleLayoutRegionEnter(regionId: string) {
    onLayoutRegionHoverChange(regionId)
  }

  function handleLayoutRegionLeave() {
    onLayoutRegionHoverChange(null)
  }

  function handleLayoutRegionClick(regionId: string) {
    onLayoutRegionSelect(regionId)
  }

  function handleLayoutRegionKeydown(event: KeyboardEvent, regionId: string) {
    if (event.key !== 'Enter' && event.key !== ' ') return
    event.preventDefault()
    event.stopPropagation()
    handleLayoutRegionClick(regionId)
  }

  function handleShapePointerDown(event: PointerEvent, annotationId: string) {
    event.stopPropagation()
    handleShapeClick(annotationId)
  }
  function handleShapeKeydown(event: KeyboardEvent, annotationId: string) {
    if (event.key !== 'Enter' && event.key !== ' ') return
    event.preventDefault()
    event.stopPropagation()
    handleShapeClick(annotationId)
  }

  // ── Zoom (image) ──────────────────────────────────────────────────
  function imageZoomIn() {
    if (canZoomIn) imageZoom = clampZoom(imageZoom + ZOOM_STEP)
  }
  function imageZoomOut() {
    if (canZoomOut) imageZoom = clampZoom(imageZoom - ZOOM_STEP)
  }

  function adjustFineRotation(deltaDegrees: number) {
    imageRotation = clampFineRotation(imageRotation + deltaDegrees)
  }

  function commitFineRotation() {
    if (type === 'image' && imageRotation === 0) return
    void onFineRotateCommit(imageRotation)
  }

  // ── PDF ─────────────────────────────────────────────────────────────
  function cancelActiveRenderTask() {
    if (!activeRenderTask || typeof activeRenderTask.cancel !== 'function') return
    activeRenderTask.cancel()
  }

  function isRenderCancellation(err: unknown) {
    return err instanceof Error && err.name === 'RenderingCancelledException'
  }

  function destroyPdfResources() {
    const loadingTask = activeLoadingTask
    activeLoadingTask = null
    pdfDoc = null
    if (!loadingTask || typeof loadingTask.destroy !== 'function') return
    // destroy() frees the parsed document and terminates its dedicated worker.
    void loadingTask.destroy().catch(() => {
      // Aborted loads reject on destroy; nothing left to release.
    })
  }

  function teardownPdf() {
    renderRequestId++
    loadRequestId++
    cancelActiveRenderTask()
    activeRenderTask = null
    destroyPdfResources()
  }

  function resetViewerState() {
    teardownPdf()
    loading = false
    error = null
    pdfPage = 1
    totalPages = 0
    pdfZoom = 1.0
    pdfCanvasW = 0
    pdfCanvasH = 0
  }
  function activatePdfMode() {
    loading = true
    error = null
  }

  async function loadPdf(url: string) {
    const requestId = ++loadRequestId
    try {
      activatePdfMode()
      const pdfjs = await import('pdfjs-dist')
      pdfjs.GlobalWorkerOptions.workerSrc = new URL(
        'pdfjs-dist/build/pdf.worker.min.mjs',
        import.meta.url
      ).href
      if (requestId !== loadRequestId) return
      const loadingTask = pdfjs.getDocument(url)
      activeLoadingTask = loadingTask
      const doc = await loadingTask.promise
      if (requestId !== loadRequestId) return
      pdfDoc = doc
      totalPages = doc.numPages
      pdfPage = Math.min(Math.max(currentPage, 1), Math.max(doc.numPages, 1))
      await renderPage()
    } catch (err) {
      if (requestId !== loadRequestId) return
      error = err instanceof Error ? err.message : labels.pdfLoadError
    } finally {
      if (requestId === loadRequestId) {
        loading = false
      }
    }
  }

  async function renderPage() {
    if (!pdfDoc || !canvasEl) return
    const requestId = ++renderRequestId
    const requestedPage = pdfPage
    const requestedZoom = pdfZoom
    cancelActiveRenderTask()
    activeRenderTask = null

    try {
      const page = await pdfDoc.getPage(requestedPage)
      if (requestId !== renderRequestId) return

      const naturalViewport = page.getViewport({ scale: 1 })
      const viewport = page.getViewport({ scale: requestedZoom })
      const context = canvasEl.getContext('2d')
      if (!context) return
      canvasEl.width = viewport.width
      canvasEl.height = viewport.height
      pdfCanvasW = viewport.width
      pdfCanvasH = viewport.height
      naturalW = naturalViewport.width
      naturalH = naturalViewport.height
      onDimensionsChange({ width: naturalViewport.width, height: naturalViewport.height })
      const renderTask = page.render({ canvasContext: context, viewport })
      activeRenderTask = renderTask
      await renderTask.promise
      if (requestId !== renderRequestId) return
      onPageChange(requestedPage, totalPages)
    } catch (err) {
      if (requestId !== renderRequestId || isRenderCancellation(err)) return
      error = err instanceof Error ? err.message : labels.pdfRenderError
    } finally {
      if (requestId === renderRequestId) {
        activeRenderTask = null
      }
    }
  }

  function pdfZoomIn() {
    if (canPdfZoomIn) {
      pdfZoom = clampZoom(pdfZoom + ZOOM_STEP)
      renderPage()
    }
  }
  function pdfZoomOut() {
    if (canPdfZoomOut) {
      pdfZoom = clampZoom(pdfZoom - ZOOM_STEP)
      renderPage()
    }
  }

  // ── Effects ────────────────────────────────────────────────────────
  // Reset image zoom when the asset URL changes (crop, rotate, erase)
  $effect(() => {
    // Depend on assetUrl so zoom resets on every edit
    const _url = assetUrl
    if (type === 'image') {
      imageZoom = 1.0
      imageRotation = 0
    } else if (type === 'pdf') {
      imageRotation = pdfFineRotation
    }
  })

  $effect(() => {
    if (type === 'pdf' && imageRotation !== pdfFineRotation) {
      imageRotation = pdfFineRotation
    }
  })

  $effect(() => {
    if (type !== 'image') {
      naturalW = 0
      naturalH = 0
      return
    }
    if (!imgEl) return // Image element not mounted yet; don't zero dimensions
    measureImage()
  })

  $effect(() => {
    if (type !== 'image' || !containerEl) return
    scheduleContainerMeasure()
    const obs = new ResizeObserver(() => scheduleContainerMeasure())
    obs.observe(containerEl)
    return () => {
      obs.disconnect()
      cancelScheduledContainerMeasure()
    }
  })

  $effect(() => {
    if (type !== 'pdf') {
      resetViewerState()
      return
    }
    // Read synchronously so the effect re-runs when the asset changes (pdf -> pdf)
    const url = assetUrl
    pdfCanvasW = 0
    pdfCanvasH = 0
    naturalW = 0
    naturalH = 0
    if (pdfScrollEl) {
      pdfScrollEl.scrollLeft = 0
      pdfScrollEl.scrollTop = 0
    }
    activatePdfMode()
    void loadPdf(url)
    return () => {
      teardownPdf()
    }
  })

  $effect(() => {
    if (type !== 'pdf' || !pdfDoc) {
      return
    }

    const nextPage = Math.min(Math.max(currentPage, 1), Math.max(totalPages, 1))
    if (nextPage === pdfPage) {
      return
    }

    pdfPage = nextPage
    void renderPage()
  })

  // ── Draft rendering ─────────────────────────────────────────────────
  const draftBox = $derived(draft ? toDraftBox(draft) : null)
  const draftUnderline = $derived(
    draft?.kind === 'underline'
      ? {
          x1: Math.min(draft.startX, draft.currentX),
          x2: Math.max(draft.startX, draft.currentX),
          y: draft.startY,
        }
      : null
  )
</script>

<div
  class="document-viewer"
  class:document-viewer--editable={type === 'image' || type === 'pdf'}
  class:document-viewer--image={type === 'image'}
  bind:this={containerEl}
>
  {#if type === 'image' || type === 'pdf'}
    <div class="document-viewer__toolbar-anchor">
      <AnnotationToolbar
        {readOnly}
        tool={annotationTool}
        {editTool}
        panActive={panToolActive}
        color={annotationColor}
        hasSelection={selectedAnnotationId !== null}
        {canUndo}
        {canRedo}
        colors={presetColors}
        onToolChange={handleToolbarToolChange}
        onEditToolChange={handleToolbarEditToolChange}
        onPanToggle={handlePanToggle}
        onColorChange={handleToolbarColorChange}
        onDeleteSelected={handleDeleteSelected}
        {onRotateLeft}
        {onRotateRight}
        fineRotationDegrees={imageRotation}
        {canFineRotateLeft}
        {canFineRotateRight}
        onFineRotate={adjustFineRotation}
        onFineRotateCommit={commitFineRotation}
        {onUndo}
        {onRedo}
        zoomPercent={Math.round((type === 'pdf' ? pdfZoom : imageZoom) * 100)}
        canZoomOut={type === 'pdf' ? canPdfZoomOut : canZoomOut}
        canZoomIn={type === 'pdf' ? canPdfZoomIn : canZoomIn}
        onZoomOut={type === 'pdf' ? pdfZoomOut : imageZoomOut}
        onZoomIn={type === 'pdf' ? pdfZoomIn : imageZoomIn}
        labels={{
          ...annotationToolbarLabels,
          zoomOut: labels.pdfZoomOut,
          zoomIn: labels.pdfZoomIn,
        }}
      />
    </div>
  {/if}

  {#if type === 'image'}
    <div class="document-viewer__image-stage">
      <div
        class="document-viewer__image-stage-sizer"
        style={`width:${displayW}px;height:${displayH}px;`}
      >
        <div
          class="document-viewer__image-stage-content"
          style={`width:${baseDisplayW}px;height:${baseDisplayH}px;transform: scale(${imageZoom});`}
        >
          <div
            class="document-viewer__image-rotator"
            data-testid="image-rotator"
            style={`width:${baseDisplayW}px;height:${baseDisplayH}px;transform: rotate(${imageRotation}deg);`}
          >
            {#key assetUrl}
              <img
                bind:this={imgEl}
                src={assetUrl}
                alt={labels.imageAlt}
                class="document-viewer__image"
                style={`width:${baseDisplayW}px;height:${baseDisplayH}px;`}
                onload={measureImage}
              />
            {/key}

            {#if hasRenderableBounds}
              <svg
                class="document-viewer__overlay"
                data-testid="annotation-overlay"
                role="application"
                aria-label={labels.imageOverlayAriaLabel}
                width={baseDisplayW}
                height={baseDisplayH}
                viewBox={`0 0 ${naturalW} ${naturalH}`}
                style={`--overlay-cursor: ${overlayCursor}`}
                onpointerdown={handleOverlayPointerDown}
                onpointermove={handleOverlayPointerMove}
                onpointerup={finishDraft}
                onpointercancel={cancelDrafts}
              >
                {#if canRenderLayoutOverlay}
                  {#each layoutRegions as region (region.id)}
                    {@const isSelectedRegion = region.id === selectedLayoutRegionId}
                    {@const isHoveredRegion = region.id === hoveredLayoutRegionId}
                    <rect
                      data-testid={`layout-overlay-${region.id}`}
                      class:selected={isSelectedRegion}
                      class:hovered={isHoveredRegion}
                      class="document-viewer__layout-region"
                      x={region.x}
                      y={region.y}
                      width={region.width}
                      height={region.height}
                      fill={getLayoutRegionFill(region, isSelectedRegion, isHoveredRegion)}
                      stroke={getLayoutRegionStroke(region, isSelectedRegion, isHoveredRegion)}
                      stroke-width={getLayoutRegionStrokeWidth(
                        region,
                        isSelectedRegion,
                        isHoveredRegion
                      )}
                      stroke-dasharray={getLayoutRegionStrokeDasharray(region, isSelectedRegion)}
                      vector-effect="non-scaling-stroke"
                      role="button"
                      tabindex="-1"
                      aria-label={labels.layoutRegionAriaLabel(region.label)}
                      style={!layoutOverlayInteractive ? 'pointer-events:none' : ''}
                      onpointerenter={() => handleLayoutRegionEnter(region.id)}
                      onpointerleave={handleLayoutRegionLeave}
                      onclick={(event) => {
                        event.stopPropagation()
                        handleLayoutRegionClick(region.id)
                      }}
                      onkeydown={(event) => handleLayoutRegionKeydown(event, region.id)}
                    />
                  {/each}
                {/if}

                {#each visibleAnnotations as annotation (annotation.id)}
                  {#if annotation.kind === 'rectangle'}
                    <rect
                      data-testid={`annotation-shape-${annotation.id}`}
                      x={px(annotation.x, 'x')}
                      y={px(annotation.y, 'y')}
                      width={px(annotation.width, 'x')}
                      height={px(annotation.height, 'y')}
                      fill={annotation.color}
                      fill-opacity="0.2"
                      stroke={annotation.id === selectedAnnotationId
                        ? 'var(--color-text-primary)'
                        : annotation.color}
                      stroke-width={annotation.id === selectedAnnotationId ? 2 : 1.5}
                      vector-effect="non-scaling-stroke"
                      style={editTool !== 'none' || panToolActive ? 'pointer-events:none' : ''}
                      role="button"
                      tabindex="-1"
                      aria-label={labels.annotationAriaLabel(annotation.id)}
                      onclick={(event) => {
                        event.stopPropagation()
                        handleShapeClick(annotation.id)
                      }}
                      onkeydown={(event) => handleShapeKeydown(event, annotation.id)}
                      onpointerdown={(event) => handleShapePointerDown(event, annotation.id)}
                    />
                  {:else}
                    <g style={editTool !== 'none' || panToolActive ? 'pointer-events:none' : ''}>
                      <rect
                        data-testid={`annotation-hitbox-${annotation.id}`}
                        x={px(annotation.x, 'x')}
                        y={px(annotation.y, 'y')}
                        width={px(annotation.width, 'x')}
                        height={px(annotation.height, 'y')}
                        fill="transparent"
                        role="button"
                        tabindex="-1"
                        aria-label={labels.annotationAriaLabel(annotation.id)}
                        onclick={(event) => {
                          event.stopPropagation()
                          handleShapeClick(annotation.id)
                        }}
                        onkeydown={(event) => handleShapeKeydown(event, annotation.id)}
                        onpointerdown={(event) => handleShapePointerDown(event, annotation.id)}
                      />
                      <line
                        data-testid={`annotation-shape-${annotation.id}`}
                        x1={px(annotation.x, 'x')}
                        y1={px(annotation.y + annotation.height / 2, 'y')}
                        x2={px(annotation.x + annotation.width, 'x')}
                        y2={px(annotation.y + annotation.height / 2, 'y')}
                        stroke={annotation.id === selectedAnnotationId
                          ? 'var(--color-text-primary)'
                          : annotation.color}
                        stroke-width={UNDERLINE_STROKE_PX}
                        stroke-linecap="round"
                        vector-effect="non-scaling-stroke"
                        role="button"
                        tabindex="-1"
                        aria-label={labels.annotationAriaLabel(annotation.id)}
                        onclick={(event) => {
                          event.stopPropagation()
                          handleShapeClick(annotation.id)
                        }}
                        onkeydown={(event) => handleShapeKeydown(event, annotation.id)}
                        onpointerdown={(event) => handleShapePointerDown(event, annotation.id)}
                      />
                    </g>
                  {/if}
                {/each}

                {#if draftBox && draft?.kind === 'rectangle'}
                  <rect
                    x={px(draftBox.x, 'x')}
                    y={px(draftBox.y, 'y')}
                    width={px(draftBox.width, 'x')}
                    height={px(draftBox.height, 'y')}
                    fill={annotationColor}
                    fill-opacity="0.14"
                    stroke={annotationColor}
                    stroke-dasharray="6 4"
                    stroke-width="1.5"
                    vector-effect="non-scaling-stroke"
                  />
                {/if}

                {#if draftUnderline}
                  <line
                    x1={px(draftUnderline.x1, 'x')}
                    y1={px(draftUnderline.y, 'y')}
                    x2={px(draftUnderline.x2, 'x')}
                    y2={px(draftUnderline.y, 'y')}
                    stroke={annotationColor}
                    stroke-width={UNDERLINE_STROKE_PX}
                    stroke-dasharray="6 4"
                    stroke-linecap="round"
                    vector-effect="non-scaling-stroke"
                  />
                {/if}

                {#if editDraft}
                  {@const ebox = toEditBox(editDraft)}
                  {@const isCrop = editTool === 'crop'}
                  {@const editColor = isCrop
                    ? 'var(--color-success, #16a34a)'
                    : 'var(--color-danger, #dc2626)'}
                  {@const editLabel = isCrop
                    ? labels.cropRegionAriaLabel
                    : labels.eraseRegionAriaLabel}
                  {#if ebox.width > 0.001 && ebox.height > 0.001}
                    <rect
                      x={0}
                      y={0}
                      width={naturalW}
                      height={px(ebox.y, 'y')}
                      fill="rgba(0,0,0,0.35)"
                    />
                    <rect
                      x={0}
                      y={px(ebox.y, 'y')}
                      width={px(ebox.x, 'x')}
                      height={px(ebox.height, 'y')}
                      fill="rgba(0,0,0,0.35)"
                    />
                    <rect
                      x={px(ebox.x + ebox.width, 'x')}
                      y={px(ebox.y, 'y')}
                      width={naturalW - px(ebox.x + ebox.width, 'x')}
                      height={px(ebox.height, 'y')}
                      fill="rgba(0,0,0,0.35)"
                    />
                    <rect
                      x={0}
                      y={px(ebox.y + ebox.height, 'y')}
                      width={naturalW}
                      height={naturalH - px(ebox.y + ebox.height, 'y')}
                      fill="rgba(0,0,0,0.35)"
                    />
                  {/if}
                  <rect
                    data-testid="edit-selection-rect"
                    x={px(ebox.x, 'x')}
                    y={px(ebox.y, 'y')}
                    width={px(ebox.width, 'x')}
                    height={px(ebox.height, 'y')}
                    fill={isCrop ? 'rgba(22,163,74,0.08)' : 'rgba(220,38,38,0.08)'}
                    stroke={editColor}
                    stroke-width="2"
                    stroke-dasharray="8 4"
                    vector-effect="non-scaling-stroke"
                    role="img"
                    aria-label={editLabel}
                  />
                {/if}
              </svg>
            {/if}
          </div>
        </div>
      </div>
    </div>
  {:else if type === 'audio'}
    <AudioPlayer
      src={assetUrl}
      fallbackBlobLoader={audioPlayerFallbackBlobLoader}
      labels={{
        skipBack: labels.audioSkipBack,
        play: labels.audioPlay,
        pause: labels.audioPause,
        skipForward: labels.audioSkipForward,
        seek: labels.audioSeek,
        volume: labels.audioVolume,
      }}
    />
  {:else}
    {#if loading}
      <div class="document-viewer__loading" data-testid="pdf-loading">
        <span class="document-viewer__spinner" aria-hidden="true"></span>
        <span>{labels.pdfLoading}</span>
      </div>
    {/if}

    {#if error}
      <div class="document-viewer__error" data-testid="pdf-error" role="alert">{error}</div>
    {/if}

    <div
      class="document-viewer__canvas-container"
      data-testid="pdf-scroll-container"
      bind:this={pdfScrollEl}
    >
      <div
        class="document-viewer__pdf-stage"
        data-testid="pdf-stage"
        style={`width:${pdfRotatedW}px;height:${pdfRotatedH}px;`}
      >
        <div
          class="document-viewer__pdf-rotator"
          data-testid="pdf-rotator"
          style={`width:${pdfCropW}px;height:${pdfCropH}px;transform:translate(-50%,-50%) rotate(${pdfTotalRotation}deg);`}
        >
          <div
            class="document-viewer__pdf-crop-frame"
            style={`width:${pdfCropW}px;height:${pdfCropH}px;`}
          >
            <div
              class="document-viewer__pdf-surface"
              style={`width:${pdfCanvasW}px;height:${pdfCanvasH}px;transform:translate(${-pdfCropOffsetX}px,${-pdfCropOffsetY}px);`}
            >
              <canvas bind:this={canvasEl} data-testid="pdf-canvas"></canvas>

              {#if hasRenderableBounds}
                <svg
                  class="document-viewer__overlay"
                  data-testid="annotation-overlay"
                  role="application"
                  aria-label={labels.imageOverlayAriaLabel}
                  width={pdfCanvasW}
                  height={pdfCanvasH}
                  viewBox={`0 0 ${naturalW} ${naturalH}`}
                  style={`--overlay-cursor: ${overlayCursor}`}
                  onpointerdown={handleOverlayPointerDown}
                  onpointermove={handleOverlayPointerMove}
                  onpointerup={finishDraft}
                  onpointercancel={cancelDrafts}
                >
                  {#if canRenderLayoutOverlay}
                    {#each layoutRegions as region (region.id)}
                      {@const isSelectedRegion = region.id === selectedLayoutRegionId}
                      {@const isHoveredRegion = region.id === hoveredLayoutRegionId}
                      <rect
                        data-testid={`layout-overlay-${region.id}`}
                        class:selected={isSelectedRegion}
                        class:hovered={isHoveredRegion}
                        class="document-viewer__layout-region"
                        x={layoutPx(region.x, 'x')}
                        y={layoutPx(region.y, 'y')}
                        width={layoutPx(region.width, 'x')}
                        height={layoutPx(region.height, 'y')}
                        fill={getLayoutRegionFill(region, isSelectedRegion, isHoveredRegion)}
                        stroke={getLayoutRegionStroke(region, isSelectedRegion, isHoveredRegion)}
                        stroke-width={getLayoutRegionStrokeWidth(
                          region,
                          isSelectedRegion,
                          isHoveredRegion
                        )}
                        stroke-dasharray={getLayoutRegionStrokeDasharray(region, isSelectedRegion)}
                        vector-effect="non-scaling-stroke"
                        role="button"
                        tabindex="-1"
                        aria-label={labels.layoutRegionAriaLabel(region.label)}
                        style={!layoutOverlayInteractive ? 'pointer-events:none' : ''}
                        onpointerenter={() => handleLayoutRegionEnter(region.id)}
                        onpointerleave={handleLayoutRegionLeave}
                        onclick={(event) => {
                          event.stopPropagation()
                          handleLayoutRegionClick(region.id)
                        }}
                        onkeydown={(event) => handleLayoutRegionKeydown(event, region.id)}
                      />
                    {/each}
                  {/if}

                  {#each pdfErasures as erasure (erasure.id)}
                    <rect
                      data-testid={`document-erasure-${erasure.id}`}
                      x={px(erasure.x, 'x')}
                      y={px(erasure.y, 'y')}
                      width={px(erasure.width, 'x')}
                      height={px(erasure.height, 'y')}
                      fill={erasure.color || '#ffffff'}
                      pointer-events="none"
                    />
                  {/each}

                  {#each visibleAnnotations as annotation (annotation.id)}
                    {#if annotation.kind === 'rectangle'}
                      <rect
                        data-testid={`annotation-shape-${annotation.id}`}
                        x={px(annotation.x, 'x')}
                        y={px(annotation.y, 'y')}
                        width={px(annotation.width, 'x')}
                        height={px(annotation.height, 'y')}
                        fill={annotation.color}
                        fill-opacity="0.2"
                        stroke={annotation.id === selectedAnnotationId
                          ? 'var(--color-text-primary)'
                          : annotation.color}
                        stroke-width={annotation.id === selectedAnnotationId ? 2 : 1.5}
                        vector-effect="non-scaling-stroke"
                        style={editTool !== 'none' || panToolActive ? 'pointer-events:none' : ''}
                        role="button"
                        tabindex="-1"
                        aria-label={labels.annotationAriaLabel(annotation.id)}
                        onclick={(event) => {
                          event.stopPropagation()
                          handleShapeClick(annotation.id)
                        }}
                        onkeydown={(event) => handleShapeKeydown(event, annotation.id)}
                        onpointerdown={(event) => handleShapePointerDown(event, annotation.id)}
                      />
                    {:else}
                      <g style={editTool !== 'none' || panToolActive ? 'pointer-events:none' : ''}>
                        <rect
                          data-testid={`annotation-hitbox-${annotation.id}`}
                          x={px(annotation.x, 'x')}
                          y={px(annotation.y, 'y')}
                          width={px(annotation.width, 'x')}
                          height={px(annotation.height, 'y')}
                          fill="transparent"
                          role="button"
                          tabindex="-1"
                          aria-label={labels.annotationAriaLabel(annotation.id)}
                          onclick={(event) => {
                            event.stopPropagation()
                            handleShapeClick(annotation.id)
                          }}
                          onkeydown={(event) => handleShapeKeydown(event, annotation.id)}
                          onpointerdown={(event) => handleShapePointerDown(event, annotation.id)}
                        />
                        <line
                          data-testid={`annotation-shape-${annotation.id}`}
                          x1={px(annotation.x, 'x')}
                          y1={px(annotation.y + annotation.height / 2, 'y')}
                          x2={px(annotation.x + annotation.width, 'x')}
                          y2={px(annotation.y + annotation.height / 2, 'y')}
                          stroke={annotation.id === selectedAnnotationId
                            ? 'var(--color-text-primary)'
                            : annotation.color}
                          stroke-width={UNDERLINE_STROKE_PX}
                          stroke-linecap="round"
                          vector-effect="non-scaling-stroke"
                          role="button"
                          tabindex="-1"
                          aria-label={labels.annotationAriaLabel(annotation.id)}
                          onclick={(event) => {
                            event.stopPropagation()
                            handleShapeClick(annotation.id)
                          }}
                          onkeydown={(event) => handleShapeKeydown(event, annotation.id)}
                          onpointerdown={(event) => handleShapePointerDown(event, annotation.id)}
                        />
                      </g>
                    {/if}
                  {/each}

                  {#if draftBox && draft?.kind === 'rectangle'}
                    <rect
                      x={px(draftBox.x, 'x')}
                      y={px(draftBox.y, 'y')}
                      width={px(draftBox.width, 'x')}
                      height={px(draftBox.height, 'y')}
                      fill={annotationColor}
                      fill-opacity="0.14"
                      stroke={annotationColor}
                      stroke-dasharray="6 4"
                      stroke-width="1.5"
                      vector-effect="non-scaling-stroke"
                    />
                  {/if}

                  {#if draftUnderline}
                    <line
                      x1={px(draftUnderline.x1, 'x')}
                      y1={px(draftUnderline.y, 'y')}
                      x2={px(draftUnderline.x2, 'x')}
                      y2={px(draftUnderline.y, 'y')}
                      stroke={annotationColor}
                      stroke-width={UNDERLINE_STROKE_PX}
                      stroke-dasharray="6 4"
                      stroke-linecap="round"
                      vector-effect="non-scaling-stroke"
                    />
                  {/if}

                  {#if editDraft}
                    {@const ebox = toEditBox(editDraft)}
                    {@const isCrop = editTool === 'crop'}
                    {@const editColor = isCrop
                      ? 'var(--color-success, #16a34a)'
                      : 'var(--color-danger, #dc2626)'}
                    <rect
                      data-testid="edit-selection-rect"
                      x={px(ebox.x, 'x')}
                      y={px(ebox.y, 'y')}
                      width={px(ebox.width, 'x')}
                      height={px(ebox.height, 'y')}
                      fill={isCrop ? 'rgba(22,163,74,0.08)' : 'rgba(220,38,38,0.08)'}
                      stroke={editColor}
                      stroke-width="2"
                      stroke-dasharray="8 4"
                      vector-effect="non-scaling-stroke"
                      role="img"
                      aria-label={isCrop ? labels.cropRegionAriaLabel : labels.eraseRegionAriaLabel}
                    />
                  {/if}
                </svg>
              {/if}
            </div>
          </div>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .document-viewer {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background-color: transparent;
    border: 0;
    border-radius: 0;
    overflow: hidden;
  }

  .document-viewer--editable {
    position: relative;
    flex: 1;
  }

  .document-viewer--image {
    position: relative;
    flex: 1;
    overflow: auto;
    scrollbar-gutter: stable both-edges;
    padding: 0;
  }

  .document-viewer__toolbar-anchor {
    position: sticky;
    top: 0;
    left: 0;
    right: 0;
    z-index: 3;
    display: flex;
    justify-content: flex-end;
    align-items: flex-start;
    gap: var(--space-2);
    width: 100%;
    inline-size: 100%;
    max-width: 100%;
    box-sizing: border-box;
    pointer-events: none;
    height: 0;
    min-height: 0;
    padding: 0 var(--space-2) 0 0;
  }

  .document-viewer__image-stage {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin: auto;
  }

  .document-viewer__image-stage-sizer {
    position: relative;
    flex: 0 0 auto;
  }

  .document-viewer__image-stage-content {
    position: relative;
    transform-origin: top left;
  }

  .document-viewer__image-rotator {
    position: relative;
    transform-origin: center center;
    transition: transform 0.08s linear;
  }

  .document-viewer__image {
    display: block;
    flex-shrink: 0;
  }

  .document-viewer__overlay {
    position: absolute;
    inset: 0;
    cursor: var(--overlay-cursor, crosshair);
  }

  .document-viewer__layout-region {
    transition:
      fill 0.15s ease,
      stroke 0.15s ease,
      stroke-width 0.15s ease;
  }

  .document-viewer__canvas-container {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    align-items: flex-start;
    justify-content: flex-start;
    overflow: auto;
    box-sizing: border-box;
    padding: var(--space-4);
  }

  .document-viewer__pdf-stage {
    position: relative;
    flex: 0 0 auto;
  }

  .document-viewer__pdf-rotator {
    position: absolute;
    left: 50%;
    top: 50%;
    transform-origin: center;
    transition: transform 0.08s linear;
  }

  .document-viewer__pdf-crop-frame {
    position: relative;
    overflow: hidden;
  }

  .document-viewer__pdf-surface {
    position: absolute;
    left: 0;
    top: 0;
    transform-origin: top left;
  }

  .document-viewer__pdf-surface > canvas {
    display: block;
    flex: 0 0 auto;
  }

  .document-viewer__loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-6);
    color: var(--color-text-secondary);
    font-family: var(--font-sans);
    font-size: var(--font-size-md);
  }

  .document-viewer__spinner {
    width: 16px;
    height: 16px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: var(--radius-full);
    animation: spin 0.6s linear infinite;
  }

  .document-viewer__error {
    padding: var(--space-4);
    color: var(--color-danger);
    font-family: var(--font-sans);
    font-size: var(--font-size-md);
    text-align: center;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
