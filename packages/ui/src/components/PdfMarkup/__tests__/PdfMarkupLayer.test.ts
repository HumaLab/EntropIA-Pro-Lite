import { fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import PdfMarkupLayer from '../PdfMarkupLayer.svelte'
import { createMarkupViewport } from '../geometry'
import { DEFAULT_MARKUP_LABELS, ZOTERO_COLORS, type MarkupTool, type PdfMarkup } from '../types'

const viewport = createMarkupViewport([0, 0, 612, 792], 1, 0)

function props(overrides: Record<string, unknown> = {}) {
  return {
    viewport,
    pageIndex: 0,
    markups: [] as PdfMarkup[],
    tool: 'select' as MarkupTool,
    color: '#ffd400',
    palette: ZOTERO_COLORS,
    selectedId: null,
    labels: DEFAULT_MARKUP_LABELS,
    ...overrides,
  } as {
    viewport: typeof viewport
    pageIndex: number
    markups: PdfMarkup[]
    tool: MarkupTool
    color: string
    palette: readonly string[]
    selectedId: string | null
    labels: typeof DEFAULT_MARKUP_LABELS
  }
}

/** Fakes a text selection inside the text layer. */
function selectText(textLayer: HTMLElement, text: string) {
  const node = document.createTextNode(text)
  textLayer.append(node)
  const removeAllRanges = vi.fn()
  vi.spyOn(window, 'getSelection').mockReturnValue({
    isCollapsed: false,
    toString: () => text,
    removeAllRanges,
    getRangeAt: () => ({
      commonAncestorContainer: node,
      getClientRects: () => [
        { left: 10, top: 100, width: 80, height: 12 },
        { left: 10, top: 120, width: 40, height: 12 },
      ],
    }),
  } as unknown as Selection)
  return removeAllRanges
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('PdfMarkupLayer', () => {
  it('offers a popover for a selection and creates a highlight with the chosen color', async () => {
    const onCreate = vi.fn()
    render(PdfMarkupLayer, { props: props({ onCreate }) })
    const textLayer = screen.getByTestId('pdf-text-layer')
    selectText(textLayer, 'Selected words')
    await fireEvent.pointerUp(textLayer)
    const popover = screen.getByTestId('pdf-selection-popover')
    await fireEvent.click(screen.getByRole('button', { name: 'Color #5fb236' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Highlight' }))
    expect(onCreate).toHaveBeenCalledOnce()
    const draft = onCreate.mock.calls[0]?.[0]
    expect(draft.type).toBe('highlight')
    expect(draft.color).toBe('#5fb236')
    expect(draft.text).toBe('Selected words')
    expect(draft.rects).toHaveLength(2)
    expect(popover).not.toBeInTheDocument()
  })

  it('creates an underline at once when the underline tool is active', async () => {
    const onCreate = vi.fn()
    render(PdfMarkupLayer, { props: props({ tool: 'underline', onCreate }) })
    const textLayer = screen.getByTestId('pdf-text-layer')
    const removeAllRanges = selectText(textLayer, 'Line')
    await fireEvent.pointerUp(textLayer)
    expect(onCreate.mock.calls[0]?.[0].type).toBe('underline')
    expect(removeAllRanges).toHaveBeenCalled()
    expect(screen.queryByTestId('pdf-selection-popover')).not.toBeInTheDocument()
  })

  it('lets the overlay take the pointer only for note, area and text tools', () => {
    const view = render(PdfMarkupLayer, { props: props({ tool: 'highlight' }) })
    expect(screen.getByTestId('pdf-markup-layer').classList.contains('pdf-markup-layer--capture')).toBe(false)
    view.unmount()
    render(PdfMarkupLayer, { props: props({ tool: 'image' }) })
    expect(screen.getByTestId('pdf-markup-layer').classList.contains('pdf-markup-layer--capture')).toBe(true)
  })

  it('draws an area by dragging and ignores a click', async () => {
    const onCreate = vi.fn()
    render(PdfMarkupLayer, { props: props({ tool: 'image', onCreate }) })
    const overlay = screen.getByTestId('pdf-markup-overlay')
    await fireEvent.pointerDown(overlay, { button: 0, clientX: 10, clientY: 10 })
    await fireEvent.pointerUp(overlay, { clientX: 12, clientY: 12 })
    expect(onCreate).not.toHaveBeenCalled()
    await fireEvent.pointerDown(overlay, { button: 0, clientX: 10, clientY: 10 })
    await fireEvent.pointerMove(overlay, { clientX: 110, clientY: 60 })
    await fireEvent.pointerUp(overlay, { clientX: 110, clientY: 60 })
    const draft = onCreate.mock.calls[0]?.[0]
    expect(draft.type).toBe('image')
    const [x1, y1, x2, y2] = draft.rects[0]
    expect([x1, y1, x2, y2]).toEqual([10, 732, 110, 782])
  })

  it('moves a selected note and reports its new rect', async () => {
    const onGeometryChange = vi.fn()
    const note: PdfMarkup = { id: 'NOTE2345', type: 'note', color: '#ffd400', pageIndex: 0, rects: [[100, 700, 122, 722]] }
    render(PdfMarkupLayer, {
      props: props({ markups: [note], selectedId: 'NOTE2345', onGeometryChange }),
    })
    const shape = screen.getByTestId('pdf-markup')
    const overlay = screen.getByTestId('pdf-markup-overlay')
    await fireEvent.pointerDown(shape, { button: 0, clientX: 105, clientY: 75 })
    await fireEvent.pointerMove(overlay, { clientX: 125, clientY: 85 })
    await fireEvent.pointerUp(overlay, { clientX: 125, clientY: 85 })
    expect(onGeometryChange).toHaveBeenCalledWith({ id: 'NOTE2345', rects: [[120, 690, 142, 712]] })
  })

  it('never moves a highlight', async () => {
    const onGeometryChange = vi.fn()
    const onSelect = vi.fn()
    const highlight: PdfMarkup = { id: 'HIGH2345', type: 'highlight', color: '#ffd400', pageIndex: 0, rects: [[100, 700, 200, 712]] }
    render(PdfMarkupLayer, {
      props: props({ markups: [highlight], selectedId: 'HIGH2345', onGeometryChange, onSelect }),
    })
    const shape = screen.getByTestId('pdf-markup')
    const overlay = screen.getByTestId('pdf-markup-overlay')
    await fireEvent.pointerDown(shape, { button: 0, clientX: 105, clientY: 85 })
    await fireEvent.pointerMove(overlay, { clientX: 150, clientY: 120 })
    await fireEvent.pointerUp(overlay, { clientX: 150, clientY: 120 })
    expect(onSelect).toHaveBeenCalledWith('HIGH2345')
    expect(onGeometryChange).not.toHaveBeenCalled()
  })
})
