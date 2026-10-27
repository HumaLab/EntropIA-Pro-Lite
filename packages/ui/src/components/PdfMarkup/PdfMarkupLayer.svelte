<script lang="ts">
  import { untrack } from 'svelte'
  import Button from '../Button/Button.svelte'
  import ActionIcon from '../Button/ActionIcon.svelte'
  import type {
    MarkupTool,
    MarkupViewport,
    PdfMarkup,
    PdfMarkupDraft,
    PdfMarkupGeometryChange,
    PdfMarkupLabels,
    PdfRect,
  } from './types'
  import { TEXT_FONT_SIZE_PT } from './types'
  import {
    movePdfRect,
    noteRectAt,
    resizePdfRect,
    selectionToPdfRects,
    toPdfRect,
    toViewportBox,
    type ViewportBox,
  } from './geometry'

  interface Props {
    viewport: MarkupViewport
    /** Zero-based index of the page shown. */
    pageIndex: number
    markups: PdfMarkup[]
    tool: MarkupTool
    color: string
    palette: readonly string[]
    selectedId: string | null
    labels: PdfMarkupLabels
    /** Receives the element pdf.js renders the text layer into. */
    onTextLayerMount?: (element: HTMLDivElement | null) => void
    onSelect?: (id: string | null) => void
    onCreate?: (draft: PdfMarkupDraft) => void
    onGeometryChange?: (change: PdfMarkupGeometryChange) => void
    onDelete?: (id: string) => void
  }

  let {
    viewport,
    pageIndex,
    markups,
    tool,
    color,
    palette,
    selectedId,
    labels,
    onTextLayerMount = () => {},
    onSelect = () => {},
    onCreate = () => {},
    onGeometryChange = () => {},
    onDelete = () => {},
  }: Props = $props()

  const MIN_AREA_PX = 6

  let rootEl: HTMLDivElement | undefined = $state()
  let textLayerEl: HTMLDivElement | undefined = $state()

  // Only the element is tracked: the parent stores it in its own state, and
  // reading that state here would make this effect re-run on its own write.
  $effect(() => {
    const element = textLayerEl ?? null
    untrack(() => onTextLayerMount(element))
    return () => untrack(() => onTextLayerMount(null))
  })

  interface Shape {
    markup: PdfMarkup
    boxes: ViewportBox[]
  }

  /** Markups of this page, plus highlights continuing from the previous one. */
  const shapes = $derived.by(() => {
    const result: Shape[] = []
    for (const markup of markups) {
      let rects: PdfRect[] = []
      if (markup.pageIndex === pageIndex) rects = markup.rects
      else if (markup.pageIndex === pageIndex - 1 && markup.nextPageRects?.length) {
        rects = markup.nextPageRects
      }
      if (rects.length === 0 && !(markup.pageIndex === pageIndex && markup.paths?.length)) continue
      result.push({ markup, boxes: rects.map((rect) => toViewportBox(viewport, rect)) })
    }
    return result
  })

  const capturesPointer = $derived(tool === 'note' || tool === 'image' || tool === 'text')

  function isMovable(markup: PdfMarkup) {
    return (
      !markup.readOnly &&
      (markup.type === 'note' || markup.type === 'image' || markup.type === 'text') &&
      markup.pageIndex === pageIndex
    )
  }

  function localPoint(event: PointerEvent | MouseEvent) {
    const bounds = rootEl?.getBoundingClientRect()
    return { x: event.clientX - (bounds?.left ?? 0), y: event.clientY - (bounds?.top ?? 0) }
  }

  function inkPoints(path: number[]) {
    const points: string[] = []
    for (let index = 0; index + 1 < path.length; index += 2) {
      const [x = 0, y = 0] = viewport.convertToViewportPoint(path[index] ?? 0, path[index + 1] ?? 0)
      points.push(`${x},${y}`)
    }
    return points.join(' ')
  }

  // ── Drawing new note / area / text markups ─────────────────────────
  let areaDraft = $state<{ startX: number; startY: number; x: number; y: number } | null>(null)

  function handleOverlayPointerDown(event: PointerEvent) {
    if (!capturesPointer || event.button !== 0) return
    const point = localPoint(event)
    if (tool === 'image') {
      areaDraft = { startX: point.x, startY: point.y, x: point.x, y: point.y }
      ;(event.currentTarget as Element).setPointerCapture?.(event.pointerId)
      return
    }
    if (tool === 'note') {
      onCreate({ type: 'note', color, pageIndex, rects: [noteRectAt(viewport, point.x, point.y)] })
      return
    }
    const height = TEXT_FONT_SIZE_PT * 1.6 * viewport.scale
    const box = { x: point.x, y: point.y - height / 2, width: 160 * viewport.scale, height }
    onCreate({
      type: 'text',
      color,
      pageIndex,
      rects: [toPdfRect(viewport, box)],
      fontSize: TEXT_FONT_SIZE_PT,
    })
  }

  function handleOverlayPointerMove(event: PointerEvent) {
    if (areaDraft) {
      const point = localPoint(event)
      areaDraft = { ...areaDraft, x: point.x, y: point.y }
      return
    }
    if (drag) moveDrag(event)
  }

  function handleOverlayPointerUp(event: PointerEvent) {
    if (areaDraft) {
      const box = draftBox(areaDraft)
      areaDraft = null
      if (box.width >= MIN_AREA_PX && box.height >= MIN_AREA_PX) {
        onCreate({ type: 'image', color, pageIndex, rects: [toPdfRect(viewport, box)] })
      }
      return
    }
    if (drag) finishDrag(event)
  }

  function draftBox(draft: { startX: number; startY: number; x: number; y: number }): ViewportBox {
    return {
      x: Math.min(draft.startX, draft.x),
      y: Math.min(draft.startY, draft.y),
      width: Math.abs(draft.x - draft.startX),
      height: Math.abs(draft.y - draft.startY),
    }
  }

  // ── Selecting, moving and resizing ─────────────────────────────────
  let drag = $state<{
    id: string
    mode: 'move' | 'resize'
    startX: number
    startY: number
    dx: number
    dy: number
    rect: PdfRect
  } | null>(null)

  function handleShapePointerDown(event: PointerEvent, shape: Shape) {
    if (capturesPointer || event.button !== 0) return
    event.stopPropagation()
    onSelect(shape.markup.id)
    const rect = shape.markup.rects[0]
    if (tool !== 'select' || !isMovable(shape.markup) || !rect) return
    const target = event.target as Element | null
    const mode = target?.getAttribute('data-testid') === 'pdf-markup-resize' ? 'resize' : 'move'
    const point = localPoint(event)
    drag = {
      id: shape.markup.id,
      mode,
      startX: point.x,
      startY: point.y,
      dx: 0,
      dy: 0,
      rect,
    }
    ;(event.currentTarget as Element).setPointerCapture?.(event.pointerId)
  }

  function moveDrag(event: PointerEvent) {
    if (!drag) return
    const point = localPoint(event)
    drag = { ...drag, dx: point.x - drag.startX, dy: point.y - drag.startY }
  }

  function finishDrag(event: PointerEvent) {
    if (!drag) return
    moveDrag(event)
    const done = drag
    drag = null
    if (Math.abs(done.dx) < 1 && Math.abs(done.dy) < 1) return
    const rect =
      done.mode === 'move'
        ? movePdfRect(viewport, done.rect, done.dx, done.dy)
        : resizePdfRect(viewport, done.rect, done.dx, done.dy)
    onGeometryChange({ id: done.id, rects: [rect] })
  }

  function liveBox(shape: Shape, box: ViewportBox): ViewportBox {
    if (!drag || drag.id !== shape.markup.id) return box
    if (drag.mode === 'move') return { ...box, x: box.x + drag.dx, y: box.y + drag.dy }
    return { ...box, width: Math.max(8, box.width + drag.dx), height: Math.max(8, box.height + drag.dy) }
  }

  function handleShapeKeyDown(event: KeyboardEvent, shape: Shape) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      onSelect(shape.markup.id)
    } else if ((event.key === 'Delete' || event.key === 'Backspace') && !shape.markup.readOnly) {
      event.preventDefault()
      onDelete(shape.markup.id)
    }
  }

  // ── Text selection → highlight / underline ─────────────────────────
  let popover = $state<{ x: number; y: number; text: string; boxes: ViewportBox[] } | null>(null)

  function readSelection() {
    const selection = typeof window !== 'undefined' ? window.getSelection() : null
    if (!selection || selection.isCollapsed || !textLayerEl || !rootEl) return null
    const range = selection.getRangeAt(0)
    if (!textLayerEl.contains(range.commonAncestorContainer)) return null
    const bounds = rootEl.getBoundingClientRect()
    const boxes = Array.from(range.getClientRects()).map((rect) => ({
      x: rect.left - bounds.left,
      y: rect.top - bounds.top,
      width: rect.width,
      height: rect.height,
    }))
    const text = selection.toString().replace(/\s+/g, ' ').trim()
    if (!text || boxes.length === 0) return null
    return { text, boxes }
  }

  function createFromSelection(type: 'highlight' | 'underline', chosenColor: string, picked = popover) {
    if (!picked) return
    const rects = selectionToPdfRects(viewport, picked.boxes)
    popover = null
    window.getSelection()?.removeAllRanges()
    if (rects.length === 0) return
    onCreate({ type, color: chosenColor, pageIndex, rects, text: picked.text })
  }

  function handleTextPointerUp() {
    const picked = readSelection()
    if (!picked) {
      popover = null
      return
    }
    if (tool === 'highlight' || tool === 'underline') {
      createFromSelection(tool, color, { x: 0, y: 0, ...picked })
      return
    }
    const last = picked.boxes[picked.boxes.length - 1]
    if (!last) return
    popover = { x: last.x + last.width, y: last.y + last.height + 6, ...picked }
  }

  let popoverColor = $state<string | null>(null)
  const activePopoverColor = $derived(popoverColor ?? color)
</script>

<div
  class="pdf-markup-layer"
  class:pdf-markup-layer--capture={capturesPointer}
  bind:this={rootEl}
  data-testid="pdf-markup-layer"
>
  <div
    class="pdf-markup-layer__text textLayer"
    class:pdf-markup-layer__text--inert={capturesPointer}
    data-testid="pdf-text-layer"
    bind:this={textLayerEl}
    onpointerup={handleTextPointerUp}
    role="presentation"
  ></div>

  <svg
    class="pdf-markup-layer__overlay"
    data-testid="pdf-markup-overlay"
    width={viewport.width}
    height={viewport.height}
    viewBox={`0 0 ${viewport.width} ${viewport.height}`}
    role="application"
    aria-label={labels.toolbarAriaLabel}
    onpointerdown={handleOverlayPointerDown}
    onpointermove={handleOverlayPointerMove}
    onpointerup={handleOverlayPointerUp}
  >
    {#each shapes as shape (shape.markup.id + ':' + shape.markup.pageIndex)}
      {@const markup = shape.markup}
      {@const selected = markup.id === selectedId}
      <g
        class="pdf-markup"
        class:pdf-markup--selected={selected}
        data-testid="pdf-markup"
        data-markup-id={markup.id}
        data-markup-type={markup.type}
        role="button"
        tabindex="0"
        aria-pressed={selected}
        aria-label={labels.markupAriaLabel(markup.type, markup.text ?? markup.comment ?? '')}
        onkeydown={(event) => handleShapeKeyDown(event, shape)}
        onpointerdown={(event) => handleShapePointerDown(event, shape)}
      >
        {#if markup.type === 'ink'}
          {#each markup.paths ?? [] as path, index (index)}
            <polyline
              class="pdf-markup__ink"
              points={inkPoints(path)}
              stroke={markup.color}
              stroke-width={(markup.width ?? 2) * viewport.scale}
              fill="none"
            />
          {/each}
        {/if}
        {#each shape.boxes as rawBox, index (index)}
          {@const box = liveBox(shape, rawBox)}
          {#if markup.type === 'highlight'}
            <rect
              class="pdf-markup__highlight"
              x={box.x}
              y={box.y}
              width={box.width}
              height={box.height}
              fill={markup.color}
            />
          {:else if markup.type === 'underline'}
            <rect
              class="pdf-markup__hitbox"
              x={box.x}
              y={box.y}
              width={box.width}
              height={box.height}
            />
            <line
              class="pdf-markup__underline"
              x1={box.x}
              x2={box.x + box.width}
              y1={box.y + box.height}
              y2={box.y + box.height}
              stroke={markup.color}
              stroke-width={Math.max(1.5, viewport.scale)}
            />
          {:else if markup.type === 'note'}
            <rect
              class="pdf-markup__note"
              x={box.x}
              y={box.y}
              width={box.width}
              height={box.height}
              rx={3}
              fill={markup.color}
            />
          {:else if markup.type === 'image'}
            <rect
              class="pdf-markup__area"
              x={box.x}
              y={box.y}
              width={box.width}
              height={box.height}
              stroke={markup.color}
            />
          {:else if markup.type === 'text'}
            <rect
              class="pdf-markup__text-box"
              x={box.x}
              y={box.y}
              width={box.width}
              height={box.height}
            />
            <text
              class="pdf-markup__text"
              x={box.x + 2}
              y={box.y + (markup.fontSize ?? TEXT_FONT_SIZE_PT) * viewport.scale}
              fill={markup.color}
              font-size={(markup.fontSize ?? TEXT_FONT_SIZE_PT) * viewport.scale}
            >{markup.comment ?? ''}</text>
          {/if}
          {#if selected && isMovable(markup) && tool === 'select' && index === 0}
            <rect
              class="pdf-markup__resize"
              data-testid="pdf-markup-resize"
              x={box.x + box.width - 5}
              y={box.y + box.height - 5}
              width={10}
              height={10}
            />
          {/if}
        {/each}
      </g>
    {/each}
    {#if areaDraft}
      {@const box = draftBox(areaDraft)}
      <rect
        class="pdf-markup__area pdf-markup__area--draft"
        data-testid="pdf-markup-draft"
        x={box.x}
        y={box.y}
        width={box.width}
        height={box.height}
        stroke={color}
      />
    {/if}
  </svg>

  {#if popover}
    <div
      class="pdf-markup-popover"
      data-testid="pdf-selection-popover"
      role="toolbar"
      aria-label={labels.popoverAriaLabel}
      style={`left:${popover.x}px;top:${popover.y}px;`}
    >
      <div class="pdf-markup-popover__colors">
        {#each palette as swatch (swatch)}
          <button
            type="button"
            class="pdf-markup-popover__swatch"
            class:pdf-markup-popover__swatch--active={swatch === activePopoverColor}
            style={`background:${swatch};`}
            aria-label={labels.colorAriaLabel(swatch)}
            aria-pressed={swatch === activePopoverColor}
            onclick={() => (popoverColor = swatch)}
          ></button>
        {/each}
      </div>
      <Button
        variant="ghost"
        size="sm"
        iconOnly
        aria-label={labels.popoverHighlight}
        title={labels.popoverHighlight}
        onclick={() => createFromSelection('highlight', activePopoverColor)}
      >
        <ActionIcon name="highlight" size={20} />
      </Button>
      <Button
        variant="ghost"
        size="sm"
        iconOnly
        aria-label={labels.popoverUnderline}
        title={labels.popoverUnderline}
        onclick={() => createFromSelection('underline', activePopoverColor)}
      >
        <ActionIcon name="underline" size={20} />
      </Button>
    </div>
  {/if}
</div>

<style>
  .pdf-markup-layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  .pdf-markup-layer__text {
    position: absolute;
    inset: 0;
    overflow: hidden;
    line-height: 1;
    pointer-events: auto;
    text-size-adjust: none;
    forced-color-adjust: none;
    transform-origin: 0 0;
    z-index: 1;
  }

  .pdf-markup-layer__text--inert {
    pointer-events: none;
  }

  .pdf-markup-layer__text :global(span),
  .pdf-markup-layer__text :global(br) {
    position: absolute;
    color: transparent;
    white-space: pre;
    cursor: text;
    transform-origin: 0% 0%;
  }

  .pdf-markup-layer__text :global(::selection) {
    background: color-mix(in srgb, var(--color-accent) 35%, transparent);
  }

  /* The parts of pdf.js 4.10 `pdf_viewer.css` the text layer relies on. */
  .pdf-markup-layer__text :global(br::selection) {
    background: transparent;
  }

  .pdf-markup-layer__text :global(span.markedContent) {
    top: 0;
    height: 0;
  }

  .pdf-markup-layer__text :global(.endOfContent) {
    display: block;
    position: absolute;
    inset: 100% 0 0;
    cursor: default;
    user-select: none;
  }

  .pdf-markup-layer__overlay {
    position: absolute;
    inset: 0;
    z-index: 2;
    pointer-events: none;
    overflow: visible;
  }

  .pdf-markup-layer--capture .pdf-markup-layer__overlay {
    pointer-events: all;
    cursor: crosshair;
  }

  .pdf-markup :is(rect, line, polyline) {
    pointer-events: auto;
    cursor: pointer;
  }

  .pdf-markup:focus-visible {
    outline: none;
  }

  .pdf-markup:focus-visible :is(rect, polyline) {
    stroke: var(--color-accent);
    stroke-width: 2;
  }

  .pdf-markup__highlight {
    opacity: 0.4;
    mix-blend-mode: multiply;
  }

  .pdf-markup__hitbox {
    fill: transparent;
  }

  .pdf-markup__note {
    opacity: 0.9;
    stroke: color-mix(in srgb, var(--color-text-primary) 30%, transparent);
  }

  .pdf-markup__area {
    fill: transparent;
    stroke-width: 2;
  }

  .pdf-markup__area--draft {
    stroke-dasharray: 4 3;
  }

  .pdf-markup__text-box {
    fill: transparent;
    stroke: color-mix(in srgb, var(--color-text-primary) 25%, transparent);
    stroke-dasharray: 3 2;
  }

  .pdf-markup__text {
    pointer-events: none;
    font-family: var(--font-ui);
  }

  .pdf-markup--selected :is(.pdf-markup__highlight, .pdf-markup__note, .pdf-markup__area, .pdf-markup__text-box) {
    stroke: var(--color-accent);
    stroke-width: 2;
  }

  .pdf-markup__resize {
    fill: var(--color-surface);
    stroke: var(--color-accent);
    cursor: nwse-resize;
  }

  .pdf-markup-popover {
    position: absolute;
    z-index: 3;
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-surface);
    box-shadow: var(--shadow-md);
    pointer-events: auto;
    transform: translateX(-50%);
  }

  .pdf-markup-popover__colors {
    display: flex;
    gap: 4px;
    padding-right: var(--space-1);
    border-right: 1px solid var(--color-border);
  }

  .pdf-markup-popover__swatch {
    width: 16px;
    height: 16px;
    padding: 0;
    border: 1px solid color-mix(in srgb, var(--color-text-primary) 20%, transparent);
    border-radius: 50%;
    cursor: pointer;
  }

  .pdf-markup-popover__swatch--active {
    outline: 2px solid var(--color-accent);
    outline-offset: 1px;
  }
</style>
