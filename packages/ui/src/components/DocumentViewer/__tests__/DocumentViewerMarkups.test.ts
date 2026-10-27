import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import DocumentViewer from '../DocumentViewer.svelte'
import { createMarkupViewport } from '../../PdfMarkup/geometry'
import type { PdfMarkup } from '../../PdfMarkup/types'

class MockResizeObserver {
  observe = vi.fn()
  disconnect = vi.fn()
}
vi.stubGlobal('ResizeObserver', MockResizeObserver)
vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
  callback(0)
  return 1
})
vi.stubGlobal('cancelAnimationFrame', () => {})

const pdf = vi.hoisted(() => {
  const textLayers: { container: HTMLElement; cancel: ReturnType<typeof vi.fn> }[] = []
  class TextLayer {
    container: HTMLElement
    cancel = vi.fn()
    constructor(options: { container: HTMLElement }) {
      this.container = options.container
      textLayers.push(this)
    }
    render() {
      const span = document.createElement('span')
      span.textContent = 'Some text'
      this.container.append(span)
      return Promise.resolve()
    }
  }
  return { textLayers, TextLayer, viewportFactory: { current: null as null | ((scale: number) => unknown) } }
})

vi.mock('pdfjs-dist', () => {
  const page = {
    getViewport: ({ scale }: { scale: number }) => pdf.viewportFactory.current?.(scale),
    render: () => ({ promise: Promise.resolve(), cancel: () => {} }),
    streamTextContent: () => ({}),
  }
  const doc = { numPages: 3, getPage: () => Promise.resolve(page) }
  return {
    getDocument: () => ({ promise: Promise.resolve(doc), destroy: () => Promise.resolve() }),
    GlobalWorkerOptions: { workerSrc: '' },
    TextLayer: pdf.TextLayer,
  }
})

const markups: PdfMarkup[] = [
  {
    id: 'HIGH2345',
    type: 'highlight',
    color: '#ffd400',
    pageIndex: 0,
    rects: [
      [100, 500, 300, 512],
      [100, 480, 250, 492],
    ],
    nextPageRects: [[100, 580, 200, 592]],
    text: 'two lines',
  },
  { id: 'NOTE2345', type: 'note', color: '#5fb236', pageIndex: 1, rects: [[50, 50, 72, 72]] },
]

const BIBLIOTECA_TOOLS = ['select', 'highlight', 'underline', 'note', 'image', 'text'] as const

function baseProps(overrides: Record<string, unknown> = {}) {
  return {
    path: '/doc.pdf',
    type: 'pdf' as const,
    assetUrl: 'asset://localhost/doc.pdf',
    readOnly: true,
    ...overrides,
  }
}

describe('DocumentViewer markups', () => {
  let getContext: ReturnType<typeof vi.spyOn>

  beforeEach(() => {
    pdf.textLayers.length = 0
    pdf.viewportFactory.current = (scale: number) => createMarkupViewport([0, 0, 800, 600], scale, 0)
    getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext')
    getContext.mockReturnValue({ drawImage: vi.fn() } as unknown as CanvasRenderingContext2D)
  })

  afterEach(() => {
    getContext.mockRestore()
  })

  it('leaves the Collections viewer unchanged when no markups are passed', async () => {
    render(DocumentViewer, { props: baseProps({ readOnly: false }) })
    await waitFor(() => expect(screen.getByTestId('annotation-toolbar')).toBeInTheDocument())
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(screen.queryByTestId('markup-toolbar')).not.toBeInTheDocument()
    expect(screen.queryByTestId('pdf-markup-layer')).not.toBeInTheDocument()
  })

  it('offers only the allowed markup tools, never crop, erase or rotate', async () => {
    render(DocumentViewer, {
      props: baseProps({ markups: [], allowedTools: BIBLIOTECA_TOOLS }),
    })
    const toolbar = await screen.findByTestId('markup-toolbar')
    for (const tool of BIBLIOTECA_TOOLS) {
      expect(toolbar.querySelector(`[data-testid="markup-tool-${tool}"]`)).not.toBeNull()
    }
    expect(screen.queryByRole('button', { name: 'Crop to selection' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Rotate 90° right' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /erase/i })).not.toBeInTheDocument()
  })

  it('hides tools left out of allowedTools', async () => {
    render(DocumentViewer, { props: baseProps({ markups: [], allowedTools: ['select', 'highlight'] }) })
    const toolbar = await screen.findByTestId('markup-toolbar')
    expect(toolbar.querySelectorAll('[data-testid^="markup-tool-"]')).toHaveLength(2)
  })

  it('draws every rect of one markup and its continuation on the next page', async () => {
    const view = render(DocumentViewer, { props: baseProps({ markups }) })
    await waitFor(() => expect(screen.getAllByTestId('pdf-markup')).toHaveLength(1))
    const highlight = screen.getByTestId('pdf-markup')
    expect(highlight.getAttribute('data-markup-id')).toBe('HIGH2345')
    expect(highlight.querySelectorAll('.pdf-markup__highlight')).toHaveLength(2)

    await view.rerender(baseProps({ markups, currentPage: 2 }))
    await waitFor(() => {
      const ids = screen.getAllByTestId('pdf-markup').map((node) => node.getAttribute('data-markup-id'))
      expect(ids.sort()).toEqual(['HIGH2345', 'NOTE2345'])
    })
    const continued = screen
      .getAllByTestId('pdf-markup')
      .find((node) => node.getAttribute('data-markup-id') === 'HIGH2345')
    expect(continued?.querySelectorAll('.pdf-markup__highlight')).toHaveLength(1)
  })

  it('selects a markup when any of its rects is clicked', async () => {
    const onMarkupSelect = vi.fn()
    render(DocumentViewer, { props: baseProps({ markups, onMarkupSelect }) })
    const highlight = await screen.findByTestId('pdf-markup')
    const second = highlight.querySelectorAll('.pdf-markup__highlight')[1] as Element
    await fireEvent.pointerDown(second, { button: 0 })
    expect(onMarkupSelect).toHaveBeenCalledWith('HIGH2345')
  })

  it('creates a 22 pt note where the note tool clicks', async () => {
    const onMarkupCreate = vi.fn()
    render(DocumentViewer, {
      props: baseProps({ markups: [], markupTool: 'note', markupColor: '#2ea8e5', onMarkupCreate }),
    })
    const overlay = await screen.findByTestId('pdf-markup-overlay')
    await fireEvent.pointerDown(overlay, { button: 0, clientX: 100, clientY: 100 })
    await waitFor(() => expect(onMarkupCreate).toHaveBeenCalledOnce())
    const draft = onMarkupCreate.mock.calls[0]?.[0]
    expect(draft.type).toBe('note')
    expect(draft.color).toBe('#2ea8e5')
    expect(draft.pageIndex).toBe(0)
    const [x1, y1, x2, y2] = draft.rects[0]
    expect(x2 - x1).toBeCloseTo(22)
    expect(y2 - y1).toBeCloseTo(22)
    expect(draft.sortIndex).toMatch(/^00000\|\d{6}\|\d{5}$/)
  })

  it('builds the text layer for the page and rebuilds it on a same-size page change', async () => {
    const view = render(DocumentViewer, { props: baseProps({ markups: [], textSelection: true }) })
    await waitFor(() => expect(pdf.textLayers).toHaveLength(1))
    const first = pdf.textLayers[0]
    await view.rerender(baseProps({ markups: [], textSelection: true, currentPage: 2 }))
    await waitFor(() => expect(pdf.textLayers).toHaveLength(2))
    expect(first?.cancel).toHaveBeenCalled()
  })

  it('rebuilds the text layer when the attachment changes', async () => {
    const view = render(DocumentViewer, { props: baseProps({ markups: [], textSelection: true }) })
    await waitFor(() => expect(pdf.textLayers).toHaveLength(1))
    await view.rerender(
      baseProps({ markups: [], textSelection: true, assetUrl: 'asset://localhost/other.pdf' })
    )
    await waitFor(() => expect(pdf.textLayers).toHaveLength(2))
  })

  it('builds no text layer unless asked', async () => {
    render(DocumentViewer, { props: baseProps({ markups: [] }) })
    await screen.findByTestId('pdf-markup-layer')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(pdf.textLayers).toHaveLength(0)
  })
})
