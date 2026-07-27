<script lang="ts">
  import ActionIcon from '../Button/ActionIcon.svelte'
  import type { ActionIconName } from '../Button/ActionIcon.types'
  import type { AnnotationTool, EditTool } from '../DocumentViewer/DocumentViewer.types'

  export interface AnnotationColorOption {
    value: string
    label: string
  }

  interface AnnotationToolbarProps {
    tool: AnnotationTool
    editTool: EditTool
    color: string
    hasSelection: boolean
    canUndo?: boolean
    canRedo?: boolean
    panActive?: boolean
    colors: AnnotationColorOption[]
    onToolChange?: (tool: AnnotationTool) => void
    onEditToolChange?: (tool: EditTool) => void
    onPanToggle?: () => void
    onColorChange?: (color: string) => void
    onDeleteSelected?: () => void
    onRotateLeft?: () => void
    onRotateRight?: () => void
    fineRotationDegrees?: number | null
    canFineRotateLeft?: boolean
    canFineRotateRight?: boolean
    onFineRotate?: (deltaDegrees: number) => void
    onFineRotateCommit?: () => void | Promise<void>
    onUndo?: () => void
    onRedo?: () => void
    zoomPercent?: number | null
    canZoomOut?: boolean
    canZoomIn?: boolean
    onZoomOut?: () => void
    onZoomIn?: () => void
    labels?: Partial<AnnotationToolbarLabels>
  }

  interface AnnotationToolbarLabels {
    expandToolbar: string
    expandToolbarTitle: string
    collapseToolbar: string
    collapseToolbarTitle: string
    toolbarAriaLabel: string
    undo: string
    undoTitle: string
    redo: string
    redoTitle: string
    panTool: string
    rectangleTool: string
    underlineTool: string
    cropTool: string
    eraseTool: string
    rotateLeft: string
    rotateRight: string
    fineRotateLeft: string
    fineRotateRight: string
    fineRotationAngle: (degrees: number) => string
    zoomOut: string
    zoomIn: string
    deleteSelected: string
    colorAriaLabel: (label: string) => string
  }

  const defaultLabels: AnnotationToolbarLabels = {
    expandToolbar: 'Expand annotation toolbar',
    expandToolbarTitle: 'Expand toolbar',
    collapseToolbar: 'Collapse annotation toolbar',
    collapseToolbarTitle: 'Collapse toolbar',
    toolbarAriaLabel: 'Document editing tools',
    undo: 'Undo last edit',
    undoTitle: 'Undo',
    redo: 'Redo last edit',
    redoTitle: 'Redo',
    panTool: 'Pan image (hand tool)',
    rectangleTool: 'Rectangle annotation tool',
    underlineTool: 'Underline annotation tool',
    cropTool: 'Crop to selection',
    eraseTool: 'Erase region (white fill)',
    rotateLeft: 'Rotate 90° left',
    rotateRight: 'Rotate 90° right',
    fineRotateLeft: 'Fine rotation left, degree by degree',
    fineRotateRight: 'Fine rotation right, degree by degree',
    fineRotationAngle: (degrees: number) => `Fine rotation ${formatSignedDegrees(degrees)}`,
    zoomOut: 'Zoom out',
    zoomIn: 'Zoom in',
    deleteSelected: 'Delete selected annotation',
    colorAriaLabel: (label: string) => `${label} annotation color`,
  }

  let {
    tool,
    editTool,
    color,
    hasSelection,
    canUndo = false,
    canRedo = false,
    panActive = false,
    colors,
    onToolChange = () => {},
    onEditToolChange = () => {},
    onPanToggle = () => {},
    onColorChange = () => {},
    onDeleteSelected = () => {},
    onRotateLeft = () => {},
    onRotateRight = () => {},
    fineRotationDegrees = null,
    canFineRotateLeft = true,
    canFineRotateRight = true,
    onFineRotate = () => {},
    onFineRotateCommit = () => {},
    onUndo = () => {},
    onRedo = () => {},
    zoomPercent = null,
    canZoomOut = false,
    canZoomIn = false,
    onZoomOut = () => {},
    onZoomIn = () => {},
    labels: labelsProp = {},
  }: AnnotationToolbarProps = $props()

  const labels = $derived({ ...defaultLabels, ...labelsProp })

  let collapsed = $state(false)
  let toolbarEl: HTMLDivElement | undefined = $state()
  let toolbarAvailableHeight = $state(720)
  let fineRotationDrag = $state<{
    pointerId: number
    direction: -1 | 1
    startClientX: number
    startClientY: number
    lastSteps: number
    applied: boolean
  } | null>(null)
  let suppressFineRotationClick = false

  const TOOLBAR_BASE_CONTROL_SIZE = 30
  const TOOLBAR_BASE_PADDING = 6
  const TOOLBAR_BASE_GAP = 4
  const TOOLBAR_SAFE_INSET = 12
  const TOOLBAR_MIN_SINGLE_COLUMN_SCALE = 0.78
  const TOOLBAR_MIN_SCALE = 0.64
  const FINE_ROTATION_DRAG_PX_PER_DEGREE = 12

  function toolbarHeightForRows(rows: number) {
    return (
      rows * TOOLBAR_BASE_CONTROL_SIZE +
      Math.max(0, rows - 1) * TOOLBAR_BASE_GAP +
      TOOLBAR_BASE_PADDING * 2
    )
  }

  const toolOptions = $derived.by(
    (): Array<{
      value: Exclude<AnnotationTool, 'select'>
      label: string
      icon: ActionIconName
    }> => [
      { value: 'rectangle', label: labels.rectangleTool, icon: 'rectangle' },
      { value: 'underline', label: labels.underlineTool, icon: 'underline' },
    ]
  )

  const editToolOptions = $derived.by(
    (): Array<{
      value: Exclude<EditTool, 'none'>
      label: string
      icon: ActionIconName
    }> => [
      { value: 'crop', label: labels.cropTool, icon: 'crop' },
      { value: 'erase', label: labels.eraseTool, icon: 'eraser' },
    ]
  )

  const visibleToolbarItemCount = $derived(
    3 +
      toolOptions.length +
      editToolOptions.length +
      2 +
      (fineRotationDegrees !== null ? 3 : 0) +
      (zoomPercent !== null ? 3 : 0) +
      colors.length +
      2
  )
  const singleColumnNaturalHeight = $derived(toolbarHeightForRows(visibleToolbarItemCount))
  const toolbarColumns = $derived(
    singleColumnNaturalHeight * TOOLBAR_MIN_SINGLE_COLUMN_SCALE <= toolbarAvailableHeight ? 1 : 2
  )
  const toolbarRows = $derived(Math.ceil(visibleToolbarItemCount / toolbarColumns))
  const toolbarNaturalHeight = $derived(toolbarHeightForRows(toolbarRows))
  const toolbarMinScale = $derived(
    toolbarColumns === 1 ? TOOLBAR_MIN_SINGLE_COLUMN_SCALE : TOOLBAR_MIN_SCALE
  )
  const toolbarScale = $derived(
    Math.max(
      toolbarMinScale,
      Math.min(1, toolbarAvailableHeight / Math.max(toolbarNaturalHeight, 1))
    )
  )

  $effect(() => {
    if (!toolbarEl) return
    const container = toolbarEl.closest('.document-viewer--editable') ?? toolbarEl.parentElement
    if (!container) return

    function updateAvailableHeight() {
      if (!container) return
      const nextHeight = container.getBoundingClientRect().height
      if (nextHeight <= 0) return
      toolbarAvailableHeight = Math.max(0, nextHeight - TOOLBAR_SAFE_INSET)
    }

    updateAvailableHeight()
    const observer = new ResizeObserver(updateAvailableHeight)
    observer.observe(container)

    return () => observer.disconnect()
  })

  function handleToolClick(option: (typeof toolOptions)[number]) {
    if (tool === option.value) {
      onToolChange('select')
    } else {
      onToolChange(option.value)
    }
  }

  function handleEditToolClick(option: (typeof editToolOptions)[number]) {
    if (editTool === option.value) {
      onEditToolChange('none')
    } else {
      onEditToolChange(option.value)
    }
  }

  function formatSignedDegrees(degrees: number) {
    if (degrees > 0) return `+${degrees}°`
    return `${degrees}°`
  }

  function canApplyFineRotation(direction: -1 | 1) {
    return direction === -1 ? canFineRotateLeft : canFineRotateRight
  }

  function applyFineRotation(direction: -1 | 1, steps = 1) {
    if (!canApplyFineRotation(direction)) return
    onFineRotate(direction * steps)
  }

  function commitFineRotation() {
    void onFineRotateCommit()
  }

  function startFineRotationDrag(direction: -1 | 1, event: PointerEvent) {
    if (!canApplyFineRotation(direction)) return
    event.preventDefault()
    ;(event.currentTarget as HTMLButtonElement).setPointerCapture?.(event.pointerId)
    fineRotationDrag = {
      pointerId: event.pointerId,
      direction,
      startClientX: event.clientX,
      startClientY: event.clientY,
      lastSteps: 0,
      applied: false,
    }
  }

  function handleFineRotationPointerMove(event: PointerEvent) {
    if (!fineRotationDrag || fineRotationDrag.pointerId !== event.pointerId) return
    event.preventDefault()

    const distance = Math.hypot(
      event.clientX - fineRotationDrag.startClientX,
      event.clientY - fineRotationDrag.startClientY
    )
    const nextSteps = Math.floor(distance / FINE_ROTATION_DRAG_PX_PER_DEGREE)
    const deltaSteps = nextSteps - fineRotationDrag.lastSteps

    if (deltaSteps <= 0) return

    applyFineRotation(fineRotationDrag.direction, deltaSteps)
    fineRotationDrag.lastSteps = nextSteps
    fineRotationDrag.applied = true
  }

  function finishFineRotationDrag(event: PointerEvent, applyOnRelease = true) {
    if (!fineRotationDrag || fineRotationDrag.pointerId !== event.pointerId) return

    const target = event.currentTarget as HTMLButtonElement
    if (target.hasPointerCapture?.(event.pointerId)) {
      target.releasePointerCapture?.(event.pointerId)
    }

    if (applyOnRelease && !fineRotationDrag.applied) {
      applyFineRotation(fineRotationDrag.direction, 1)
    }

    if (applyOnRelease) {
      commitFineRotation()
    }

    fineRotationDrag = null
    suppressFineRotationClick = true
    setTimeout(() => {
      suppressFineRotationClick = false
    }, 0)
  }

  function handleFineRotationClick(direction: -1 | 1) {
    if (suppressFineRotationClick) {
      suppressFineRotationClick = false
      return
    }

    applyFineRotation(direction, 1)
    commitFineRotation()
  }

  function handleFineRotationWheel(direction: -1 | 1, event: WheelEvent) {
    if (!canApplyFineRotation(direction)) return
    event.preventDefault()
    const wheelDistance = Math.max(Math.abs(event.deltaY), Math.abs(event.deltaX))
    const steps = Math.max(1, Math.round(wheelDistance / 100))
    applyFineRotation(direction, steps)
    commitFineRotation()
  }

</script>

{#if collapsed}
  <button
    type="button"
    class="annotation-toolbar__fab"
    data-testid="annotation-toolbar-fab"
    aria-label={labels.expandToolbar}
    title={labels.expandToolbarTitle}
    onclick={() => (collapsed = false)}
  >
    <ActionIcon name="pencil" size={18} />
  </button>
{:else}
  <div
    bind:this={toolbarEl}
    class="annotation-toolbar"
    class:annotation-toolbar--multi-column={toolbarColumns > 1}
    data-testid="annotation-toolbar"
    role="toolbar"
    aria-orientation="vertical"
    aria-label={labels.toolbarAriaLabel}
    style={`--annotation-toolbar-scale:${toolbarScale};grid-template-rows:repeat(${toolbarRows},max-content);grid-template-columns:repeat(${toolbarColumns},max-content);`}
  >
      <button
        type="button"
        class="annotation-toolbar__button"
        aria-label={labels.undo}
        title={labels.undoTitle}
        disabled={!canUndo}
        onclick={onUndo}
      >
        <ActionIcon name="undo" size={18} />
      </button>
      <button
        type="button"
        class="annotation-toolbar__button"
        aria-label={labels.redo}
        title={labels.redoTitle}
        disabled={!canRedo}
        onclick={onRedo}
      >
        <ActionIcon name="redo" size={18} />
      </button>
      <button
        type="button"
        class="annotation-toolbar__button"
        class:annotation-toolbar__button--active={panActive}
        aria-label={labels.panTool}
        aria-pressed={panActive}
        title={labels.panTool}
        onclick={onPanToggle}
      >
        <ActionIcon name="hand" size={18} />
      </button>
      {#each toolOptions as option (option.value)}
        <button
          type="button"
          class="annotation-toolbar__button"
          class:annotation-toolbar__button--active={tool === option.value}
          aria-label={option.label}
          aria-pressed={tool === option.value}
          title={option.label}
          onclick={() => handleToolClick(option)}
        >
          <ActionIcon name={option.icon} size={18} />
        </button>
      {/each}
      {#each editToolOptions as option (option.value)}
        <button
          type="button"
          class="annotation-toolbar__button"
          class:annotation-toolbar__button--active={editTool === option.value}
          aria-label={option.label}
          aria-pressed={editTool === option.value}
          title={option.label}
          onclick={() => handleEditToolClick(option)}
        >
          <ActionIcon name={option.icon} size={18} />
        </button>
      {/each}
      <button
        type="button"
        class="annotation-toolbar__button"
        aria-label={labels.rotateLeft}
        title={labels.rotateLeft}
        onclick={onRotateLeft}
      >
        <ActionIcon name="rotate-ccw" size={18} />
      </button>

      <button
        type="button"
        class="annotation-toolbar__button"
        aria-label={labels.rotateRight}
        title={labels.rotateRight}
        onclick={onRotateRight}
      >
        <ActionIcon name="rotate-cw" size={18} />
      </button>

      {#if fineRotationDegrees !== null}
        <button
          type="button"
          class="annotation-toolbar__button"
          class:annotation-toolbar__button--active={fineRotationDrag?.direction === -1}
          aria-label={labels.fineRotateLeft}
          title={`${labels.fineRotateLeft} · ${labels.fineRotationAngle(fineRotationDegrees)}`}
          disabled={!canFineRotateLeft}
          onclick={() => handleFineRotationClick(-1)}
          onpointerdown={(event) => startFineRotationDrag(-1, event)}
          onpointermove={handleFineRotationPointerMove}
          onpointerup={(event) => finishFineRotationDrag(event)}
          onpointercancel={(event) => finishFineRotationDrag(event, false)}
          onwheel={(event) => handleFineRotationWheel(-1, event)}
        >
          <ActionIcon name="rotate-fine-ccw" size={18} />
        </button>

        <span
          class="annotation-toolbar__rotation"
          data-testid="toolbar-fine-rotation-info"
          title={labels.fineRotationAngle(fineRotationDegrees)}
          >{formatSignedDegrees(fineRotationDegrees)}</span
        >

        <button
          type="button"
          class="annotation-toolbar__button"
          class:annotation-toolbar__button--active={fineRotationDrag?.direction === 1}
          aria-label={labels.fineRotateRight}
          title={`${labels.fineRotateRight} · ${labels.fineRotationAngle(fineRotationDegrees)}`}
          disabled={!canFineRotateRight}
          onclick={() => handleFineRotationClick(1)}
          onpointerdown={(event) => startFineRotationDrag(1, event)}
          onpointermove={handleFineRotationPointerMove}
          onpointerup={(event) => finishFineRotationDrag(event)}
          onpointercancel={(event) => finishFineRotationDrag(event, false)}
          onwheel={(event) => handleFineRotationWheel(1, event)}
        >
          <ActionIcon name="rotate-fine-cw" size={18} />
        </button>
      {/if}

      {#if zoomPercent !== null}
        <button
          type="button"
          class="annotation-toolbar__button"
          aria-label={labels.zoomOut}
          title={labels.zoomOut}
          disabled={!canZoomOut}
          onclick={onZoomOut}
        >
          <svg
            class="annotation-toolbar__icon"
            viewBox="0 0 24 24"
            aria-hidden="true"
            focusable="false"
          >
            <circle cx="11" cy="11" r="6.5" fill="none" stroke="currentColor" stroke-width="1.8" />
            <path
              d="M16 16 21 21"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
            />
            <path
              d="M8.5 11h5"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
            />
          </svg>
        </button>

        <span class="annotation-toolbar__zoom" data-testid="toolbar-zoom-info">{zoomPercent}%</span>

        <button
          type="button"
          class="annotation-toolbar__button"
          aria-label={labels.zoomIn}
          title={labels.zoomIn}
          disabled={!canZoomIn}
          onclick={onZoomIn}
        >
          <svg
            class="annotation-toolbar__icon"
            viewBox="0 0 24 24"
            aria-hidden="true"
            focusable="false"
          >
            <circle cx="11" cy="11" r="6.5" fill="none" stroke="currentColor" stroke-width="1.8" />
            <path
              d="M16 16 21 21"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
            />
            <path
              d="M8.5 11h5"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
            />
            <path
              d="M11 8.5v5"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
            />
          </svg>
        </button>
      {/if}

      {#each colors as option (option.value)}
        <button
          type="button"
          class="annotation-toolbar__swatch"
          class:annotation-toolbar__swatch--active={color === option.value}
          aria-label={labels.colorAriaLabel(option.label)}
          aria-pressed={color === option.value}
          title={option.label}
          onclick={() => onColorChange(option.value)}
        >
          <span class="annotation-toolbar__swatch-fill" style={`background:${option.value}`}></span>
        </button>
      {/each}

    <button
      type="button"
      class="annotation-toolbar__button annotation-toolbar__button--danger"
      aria-label={labels.deleteSelected}
      title={labels.deleteSelected}
      disabled={!hasSelection}
      onclick={onDeleteSelected}
    >
      <ActionIcon name="delete" size={18} />
    </button>

    <button
      type="button"
      class="annotation-toolbar__button annotation-toolbar__button--collapse"
      aria-label={labels.collapseToolbar}
      title={labels.collapseToolbarTitle}
      onclick={() => (collapsed = true)}
    >
      <ActionIcon name="chevron-up" size={18} />
    </button>
  </div>
{/if}

<style>
  .annotation-toolbar,
  .annotation-toolbar__fab {
    --annotation-toolbar-scale: 1;
    --annotation-toolbar-control-size: calc(30px * var(--annotation-toolbar-scale));
    --annotation-toolbar-icon-size: calc(17px * var(--annotation-toolbar-scale));
    --annotation-toolbar-padding: calc(6px * var(--annotation-toolbar-scale));
    --annotation-toolbar-gap: calc(4px * var(--annotation-toolbar-scale));
    --annotation-toolbar-swatch-size: calc(13px * var(--annotation-toolbar-scale));
    --annotation-toolbar-radius: calc(8px * var(--annotation-toolbar-scale));
  }

  .annotation-toolbar {
    display: grid;
    grid-auto-flow: column;
    align-items: center;
    justify-items: center;
    gap: var(--annotation-toolbar-gap);
    width: max-content;
    max-width: max-content;
    overflow: visible;
    box-sizing: border-box;
    padding: var(--annotation-toolbar-padding);
    border: 1px solid var(--color-border);
    border-radius: var(--annotation-toolbar-radius);
    background: color-mix(in srgb, var(--color-surface-raised) 92%, transparent);
    box-shadow: var(--shadow-md);
    backdrop-filter: blur(10px);
    pointer-events: auto;
  }

  .annotation-toolbar--multi-column {
    column-gap: calc(var(--annotation-toolbar-gap) * 1.35);
  }

  @media (max-height: 760px) {
    .annotation-toolbar,
    .annotation-toolbar__fab {
      --annotation-toolbar-scale: 0.9;
    }
  }

  @media (max-height: 680px) {
    .annotation-toolbar,
    .annotation-toolbar__fab {
      --annotation-toolbar-scale: 0.82;
    }
  }

  @media (max-height: 600px), (max-width: 520px) {
    .annotation-toolbar,
    .annotation-toolbar__fab {
      --annotation-toolbar-scale: 0.78;
    }

    .annotation-toolbar--multi-column {
      column-gap: calc(var(--annotation-toolbar-gap) * 1.25);
    }
  }

  @media (max-height: 480px) {
    .annotation-toolbar,
    .annotation-toolbar__fab {
      --annotation-toolbar-scale: 0.68;
    }
  }

  @media (max-height: 420px) {
    .annotation-toolbar,
    .annotation-toolbar__fab {
      --annotation-toolbar-scale: 0.6;
    }
  }

  .annotation-toolbar__fab {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--annotation-toolbar-control-size);
    height: var(--annotation-toolbar-control-size);
    padding: 0;
    border: 1px solid var(--color-border);
    border-radius: var(--annotation-toolbar-radius);
    background: color-mix(in srgb, var(--color-surface-raised) 90%, transparent);
    box-shadow: var(--shadow-sm);
    backdrop-filter: blur(10px);
    color: var(--color-text-secondary);
    cursor: pointer;
    font-size: var(--font-size-md);
    line-height: 1;
    pointer-events: auto;
    transition:
      background-color var(--transition-base),
      color var(--transition-base);
  }

  .annotation-toolbar__fab:hover {
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
  }

  .annotation-toolbar__zoom,
  .annotation-toolbar__rotation {
    min-width: calc(var(--annotation-toolbar-control-size) + 2px);
    max-width: calc(var(--annotation-toolbar-control-size) + 8px);
    padding-inline: 1px;
    text-align: center;
    font-family: var(--font-mono);
    font-size: calc(0.78rem * var(--annotation-toolbar-scale));
    color: var(--color-text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .annotation-toolbar__button,
  .annotation-toolbar__swatch {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--annotation-toolbar-control-size);
    height: var(--annotation-toolbar-control-size);
    flex: 0 0 var(--annotation-toolbar-control-size);
    padding: 0;
    border: 1px solid var(--color-border);
    border-radius: var(--annotation-toolbar-radius);
    background: var(--color-surface);
    color: var(--color-text-primary);
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base),
      transform var(--transition-base);
  }

  .annotation-toolbar__button:hover:not(:disabled),
  .annotation-toolbar__swatch:hover:not(:disabled) {
    background: var(--color-surface-raised);
    border-color: var(--color-text-secondary);
  }

  .annotation-toolbar__icon {
    width: var(--annotation-toolbar-icon-size);
    height: var(--annotation-toolbar-icon-size);
    display: block;
  }

  .annotation-toolbar__button :global(svg),
  .annotation-toolbar__fab :global(svg) {
    width: var(--annotation-toolbar-icon-size);
    height: var(--annotation-toolbar-icon-size);
  }

  .annotation-toolbar__button:disabled,
  .annotation-toolbar__swatch:disabled {
    opacity: 0.48;
    cursor: not-allowed;
  }

  .annotation-toolbar__button:focus-visible,
  .annotation-toolbar__swatch:focus-visible,
  .annotation-toolbar__fab:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .annotation-toolbar__button--active,
  .annotation-toolbar__swatch--active {
    border-color: var(--color-accent);
    box-shadow: inset 0 0 0 1px var(--color-accent);
  }

  .annotation-toolbar__button--danger:disabled {
    border-color: var(--color-border);
  }

  .annotation-toolbar__button--danger:not(:disabled) {
    color: var(--color-danger);
  }

  .annotation-toolbar__button--collapse {
    color: var(--color-text-secondary);
    font-size: var(--font-size-lg);
  }

  .annotation-toolbar__button--collapse:hover {
    color: var(--color-text-primary);
  }

  .annotation-toolbar__swatch-fill {
    width: var(--annotation-toolbar-swatch-size);
    height: var(--annotation-toolbar-swatch-size);
    border-radius: var(--radius-full);
    border: 1px solid color-mix(in srgb, var(--color-text-primary) 35%, transparent);
  }

  @media (max-width: 420px), (max-height: 520px) {
    .annotation-toolbar {
      box-shadow: var(--shadow-sm);
    }
  }
</style>
