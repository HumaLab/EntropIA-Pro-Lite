import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import DocumentViewer from '../DocumentViewer.svelte'
import documentViewerSource from '../DocumentViewer.svelte?raw'

type ResizeObserverCallback = globalThis.ResizeObserverCallback

class MockResizeObserver {
  static instances: MockResizeObserver[] = []

  callback: ResizeObserverCallback
  observedTargets = new Set<Element>()

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback
    MockResizeObserver.instances.push(this)
  }

  observe = vi.fn((target: Element) => {
    this.observedTargets.add(target)
  })
  disconnect = vi.fn()

  trigger(target: Element) {
    this.callback(
      [
        {
          target,
          contentRect: target.getBoundingClientRect(),
        } as ResizeObserverEntry,
      ],
      this as unknown as ResizeObserver
    )
  }
}

vi.stubGlobal('ResizeObserver', MockResizeObserver)

let rafId = 0
let rafQueue: Array<{ id: number; callback: FrameRequestCallback }> = []

vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
  const id = ++rafId
  rafQueue.push({ id, callback })
  return id
})

vi.stubGlobal('cancelAnimationFrame', (id: number) => {
  rafQueue = rafQueue.filter((entry) => entry.id !== id)
})

vi.stubGlobal('getComputedStyle', window.getComputedStyle.bind(window))

const pdfMock = vi.hoisted(() => {
  const createPage = (width = 800, height = 600) => ({
    getViewport: vi.fn(({ scale }: { scale: number }) => ({
      width: width * scale,
      height: height * scale,
      scale,
    })),
    render: vi.fn(() => ({ promise: Promise.resolve(), cancel: vi.fn() })),
  })

  const mockPage = createPage()
  const mockDocument = {
    numPages: 3,
    getPage: vi.fn(() => Promise.resolve(mockPage)),
  }

  const createLoadingTask = (document: unknown = mockDocument) => ({
    promise: Promise.resolve(document),
    destroy: vi.fn(() => Promise.resolve()),
  })

  return {
    createPage,
    createLoadingTask,
    getDocument: vi.fn(() => createLoadingTask()),
    mockDocument,
    mockPage,
  }
})

// Mock pdfjs-dist for test environment
vi.mock('pdfjs-dist', () => {
  return {
    getDocument: pdfMock.getDocument,
    GlobalWorkerOptions: { workerSrc: '' },
  }
})

describe('DocumentViewer', () => {
  beforeEach(() => {
    MockResizeObserver.instances = []
    rafId = 0
    rafQueue = []
    pdfMock.mockPage.getViewport.mockClear()
    pdfMock.mockPage.render.mockClear()
    pdfMock.mockDocument.getPage.mockReset()
    pdfMock.mockDocument.getPage.mockImplementation(() => Promise.resolve(pdfMock.mockPage))
    pdfMock.getDocument.mockReset()
    pdfMock.getDocument.mockImplementation(() => pdfMock.createLoadingTask())
  })

  function deferred<T>() {
    let resolve!: (value: T) => void
    let reject!: (reason?: unknown) => void
    const promise = new Promise<T>((res, rej) => {
      resolve = res
      reject = rej
    })

    return { promise, resolve, reject }
  }

  async function flushRaf() {
    const pending = [...rafQueue]
    rafQueue = []
    pending.forEach(({ callback }) => callback(performance.now()))
    await Promise.resolve()
  }

  async function triggerResizeObservers(target: Element) {
    MockResizeObserver.instances.forEach((obs) => obs.trigger(target))
    await flushRaf()
  }

  function setupImage(
    img: HTMLImageElement,
    naturalW: number,
    naturalH: number,
    displayW: number,
    displayH: number
  ) {
    // clientWidth/clientHeight reflect the CSS size (what we set via style)
    Object.defineProperty(img, 'clientWidth', { configurable: true, value: displayW })
    Object.defineProperty(img, 'clientHeight', { configurable: true, value: displayH })
    // naturalWidth/naturalHeight reflect the intrinsic image dimensions
    Object.defineProperty(img, 'naturalWidth', { configurable: true, value: naturalW })
    Object.defineProperty(img, 'naturalHeight', { configurable: true, value: naturalH })
    Object.defineProperty(img, 'complete', { configurable: true, value: true })
    img.getBoundingClientRect = vi.fn(() => ({
      x: 0,
      y: 0,
      top: 0,
      left: 0,
      right: displayW,
      bottom: displayH,
      width: displayW,
      height: displayH,
      toJSON: () => ({}),
    }))
  }

  function setupContainer(container: HTMLElement, width: number, height: number) {
    Object.defineProperty(container, 'clientWidth', { configurable: true, value: width })
    Object.defineProperty(container, 'clientHeight', { configurable: true, value: height })
    container.getBoundingClientRect = vi.fn(() => ({
      x: 0,
      y: 0,
      top: 0,
      left: 0,
      right: width,
      bottom: height,
      width,
      height,
      toJSON: () => ({}),
    }))
  }

  describe('image mode', () => {
    it('renders an img element with the asset URL', () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })
      const img = screen.getByRole('img')
      expect(img).toBeInTheDocument()
      expect(img).toHaveAttribute('src', 'asset://localhost/path/to/image.jpg')
    })

    it('renders annotation toolbar in image mode and hides pdf controls', () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      expect(screen.getByTestId('annotation-toolbar')).toBeInTheDocument()
      expect(screen.queryByTestId('pdf-controls')).not.toBeInTheDocument()
    })

    it('renders delete selected annotation as the shared trash icon button', () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: 'ann-1',
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const deleteBtn = screen.getByRole('button', { name: /delete selected annotation/i })
      expect(deleteBtn.querySelector('svg')).toBeInTheDocument()
      expect(deleteBtn).not.toHaveTextContent('✕')
    })

    it('renders image zoom controls in the top toolbar', () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      expect(screen.getByTestId('annotation-toolbar')).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Zoom in' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Zoom out' })).toBeInTheDocument()
      expect(screen.getByTestId('toolbar-zoom-info')).toHaveTextContent('100%')
    })

    it('exposes asset duplication from the editing toolbar', async () => {
      const onDuplicateAsset = vi.fn()
      render(DocumentViewer, {
        props: {
          path: '/path/to/document.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/document.pdf',
          onDuplicateAsset,
        },
      })

      await fireEvent.click(screen.getByRole('button', { name: 'Duplicate asset' }))

      expect(onDuplicateAsset).toHaveBeenCalledOnce()
    })

    it('keeps pan and zoom controls while hiding editing actions in read-only mode', () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          readOnly: true,
        },
      })

      expect(screen.getByTestId('annotation-toolbar')).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Pan image (hand tool)' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Zoom in' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Zoom out' })).toBeInTheDocument()
      expect(
        screen.queryByRole('button', { name: 'Rectangle annotation tool' })
      ).not.toBeInTheDocument()
      expect(screen.queryByRole('button', { name: 'Crop to selection' })).not.toBeInTheDocument()
      expect(screen.queryByRole('button', { name: 'Rotate 90° right' })).not.toBeInTheDocument()
      expect(screen.queryByRole('button', { name: 'Duplicate asset' })).not.toBeInTheDocument()
      expect(
        screen.queryByRole('button', { name: 'Delete selected annotation' })
      ).not.toBeInTheDocument()
    })

    it('uses 10 percent steps and preserves uniform image dimensions when zooming out', async () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      setupImage(img, 200, 100, 200, 100)
      await fireEvent.load(img)
      await triggerResizeObservers(img)

      const stageContent = img.closest('.document-viewer__image-stage-content') as HTMLElement
      const stageSizer = stageContent.parentElement as HTMLElement
      expect(stageSizer.style.width).toBe('200px')
      expect(stageSizer.style.height).toBe('100px')

      await fireEvent.click(screen.getByRole('button', { name: 'Zoom out' }))

      expect(screen.getByTestId('toolbar-zoom-info')).toHaveTextContent('90%')
      expect(img.style.width).toBe('200px')
      expect(img.style.height).toBe('100px')
      expect(stageContent).toHaveStyle({ transform: 'scale(0.9)' })
      expect(stageSizer.style.width).toBe('180px')
      expect(stageSizer.style.height).toBe('90px')
    })

    it('ignores tiny container resize noise and keeps manual zoom composed with fit sizing', async () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      const container = img.closest('.document-viewer') as HTMLElement
      const stageContent = img.closest('.document-viewer__image-stage-content') as HTMLElement
      const stageSizer = stageContent.parentElement as HTMLElement

      setupImage(img, 200, 100, 200, 100)
      setupContainer(container, 300, 240)
      await fireEvent.load(img)
      await triggerResizeObservers(container)

      expect(MockResizeObserver.instances.some((obs) => obs.observedTargets.has(img))).toBe(false)
      expect(MockResizeObserver.instances.some((obs) => obs.observedTargets.has(container))).toBe(
        true
      )
      expect(stageSizer.style.width).toBe('300px')
      expect(stageSizer.style.height).toBe('150px')

      setupContainer(container, 301, 240)
      await triggerResizeObservers(container)

      expect(stageSizer.style.width).toBe('300px')
      expect(stageSizer.style.height).toBe('150px')
      expect(screen.getByTestId('toolbar-zoom-info')).toHaveTextContent('100%')

      await fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))

      expect(screen.getByTestId('toolbar-zoom-info')).toHaveTextContent('110%')
      expect(stageSizer.style.width).toBe('330px')
      expect(stageSizer.style.height).toBe('165px')
      expect(stageContent).toHaveStyle({ transform: 'scale(1.1)' })

      setupContainer(container, 301.4, 240)
      await triggerResizeObservers(container)

      expect(stageSizer.style.width).toBe('330px')
      expect(stageSizer.style.height).toBe('165px')

      setupContainer(container, 280, 240)
      await triggerResizeObservers(container)

      expect(stageSizer.style.width).toBe('308px')
      expect(stageSizer.style.height).toBe('154px')
      expect(stageContent).toHaveStyle({ transform: 'scale(1.1)' })
    })

    it('renders layout regions and syncs hover/select callbacks', async () => {
      const onLayoutRegionHoverChange = vi.fn()
      const onLayoutRegionSelect = vi.fn()

      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          layoutRegions: [
            {
              id: 'layout-block-1::overlay',
              blockId: 'layout-block-1',
              label: 'title',
              x: 20,
              y: 10,
              width: 80,
              height: 30,
            },
          ],
          showLayoutOverlay: true,
          hoveredLayoutRegionId: 'layout-block-1::overlay',
          selectedLayoutRegionId: null,
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
          onLayoutRegionHoverChange,
          onLayoutRegionSelect,
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      setupImage(img, 200, 100, 200, 100)
      await fireEvent.load(img)
      await triggerResizeObservers(img)

      const region = await screen.findByTestId('layout-overlay-layout-block-1::overlay')
      expect(region).toHaveAttribute('x', '20')
      expect(region).toHaveAttribute('width', '80')

      await fireEvent.pointerEnter(region)
      expect(onLayoutRegionHoverChange).toHaveBeenCalledWith('layout-block-1::overlay')

      await fireEvent.click(region)
      expect(onLayoutRegionSelect).toHaveBeenCalledWith('layout-block-1::overlay')

      await fireEvent.pointerLeave(region)
      expect(onLayoutRegionHoverChange).toHaveBeenLastCalledWith(null)
    })

    it('keeps selected and hovered layout regions visually separate', async () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          layoutRegions: [
            {
              id: 'layout-block-1::overlay',
              blockId: 'layout-block-1',
              label: 'title',
              x: 20,
              y: 10,
              width: 80,
              height: 30,
            },
            {
              id: 'layout-block-2::overlay',
              blockId: 'layout-block-2',
              label: 'text',
              x: 30,
              y: 60,
              width: 90,
              height: 40,
            },
          ],
          showLayoutOverlay: true,
          hoveredLayoutRegionId: 'layout-block-2::overlay',
          selectedLayoutRegionId: 'layout-block-1::overlay',
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      setupImage(img, 200, 100, 200, 100)
      await fireEvent.load(img)
      await triggerResizeObservers(img)

      const selectedRegion = await screen.findByTestId('layout-overlay-layout-block-1::overlay')
      const hoveredRegion = await screen.findByTestId('layout-overlay-layout-block-2::overlay')

      expect(selectedRegion).toHaveAttribute('stroke', 'rgb(34, 211, 238)')
      expect(hoveredRegion).toHaveAttribute('stroke', 'rgb(250, 204, 21)')
    })

    it('creates a rectangle annotation with normalized coordinates relative to natural image size', async () => {
      const onAnnotationsChange = vi.fn()

      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'rectangle',
          annotationColor: 'var(--color-accent)',
          onAnnotationsChange,
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      // Natural 200x100, displayed at 200x100 (fitScale=1, zoom=1)
      setupImage(img, 200, 100, 200, 100)
      await fireEvent.load(img)
      // Trigger resize observers (image + container)
      await triggerResizeObservers(img)

      const overlay = await screen.findByTestId('annotation-overlay')
      overlay.getBoundingClientRect = vi.fn(() => ({
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 200,
        bottom: 100,
        width: 200,
        height: 100,
        toJSON: () => ({}),
      }))

      // Drag from (20,10) to (120,60) on a 200x100 display
      await fireEvent.pointerDown(overlay, { clientX: 20, clientY: 10, button: 0 })
      await fireEvent.pointerMove(overlay, { clientX: 120, clientY: 60, button: 0 })
      await fireEvent.pointerUp(overlay, { clientX: 120, clientY: 60, button: 0 })

      expect(onAnnotationsChange).toHaveBeenCalledTimes(1)
      const created = onAnnotationsChange.mock.calls[0]![0]![0]!
      expect(created.kind).toBe('rectangle')
      // Normalized: 20/200=0.1, 10/100=0.1, 100/200=0.5, 50/100=0.5
      expect(created.x).toBeCloseTo(0.1, 3)
      expect(created.y).toBeCloseTo(0.1, 3)
      expect(created.width).toBeCloseTo(0.5, 3)
      expect(created.height).toBeCloseTo(0.5, 3)
    })

    it('renders annotations in natural-image viewBox coordinates', async () => {
      const annotation = {
        id: 'ann-1',
        assetId: 'asset-1',
        page: 1,
        kind: 'rectangle' as const,
        color: 'var(--color-accent)',
        x: 0.25,
        y: 0.1,
        width: 0.5,
        height: 0.4,
        createdAt: 10,
        updatedAt: 10,
      }

      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [annotation],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      // Natural 200x100, displayed at 200x100
      setupImage(img, 200, 100, 200, 100)
      await fireEvent.load(img)
      await triggerResizeObservers(img)

      const shape = await screen.findByTestId('annotation-shape-ann-1')
      // ViewBox is "0 0 200 100" → normalized * natural = viewBox px
      expect(shape).toHaveAttribute('x', '50') // 0.25 * 200
      expect(shape).toHaveAttribute('y', '10') // 0.1 * 100
      expect(shape).toHaveAttribute('width', '100') // 0.5 * 200
      expect(shape).toHaveAttribute('height', '40') // 0.4 * 100
    })

    it('keeps annotation positions correct after image resize (zoom stays)', async () => {
      const annotation = {
        id: 'ann-1',
        assetId: 'asset-1',
        page: 1,
        kind: 'rectangle' as const,
        color: 'var(--color-accent)',
        x: 0.25,
        y: 0.1,
        width: 0.5,
        height: 0.4,
        createdAt: 10,
        updatedAt: 10,
      }

      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [annotation],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement

      // First: natural 200x100, displayed at 200x100
      setupImage(img, 200, 100, 200, 100)
      await fireEvent.load(img)
      await triggerResizeObservers(img)

      const shape = await screen.findByTestId('annotation-shape-ann-1')
      // ViewBox coordinates are always in natural-image space (200x100)
      expect(shape).toHaveAttribute('x', '50')
      expect(shape).toHaveAttribute('y', '10')
      expect(shape).toHaveAttribute('width', '100')
      expect(shape).toHaveAttribute('height', '40')

      // Now resize: same natural image but displayed at 400x200
      setupImage(img, 200, 100, 400, 200)
      await triggerResizeObservers(img)

      await waitFor(() => {
        // ViewBox coords don't change — they're in natural-image space (200x100)
        expect(shape).toHaveAttribute('x', '50')
        expect(shape).toHaveAttribute('y', '10')
        expect(shape).toHaveAttribute('width', '100')
        expect(shape).toHaveAttribute('height', '40')
      })
    })

    it('selects, recolors, deletes, and deselects annotations', async () => {
      const onAnnotationsChange = vi.fn()
      const onSelectedAnnotationIdChange = vi.fn()
      const onAnnotationColorChange = vi.fn()

      const annotation = {
        id: 'ann-1',
        assetId: 'asset-1',
        page: 1,
        kind: 'rectangle' as const,
        color: 'var(--color-accent)',
        x: 0.1,
        y: 0.2,
        width: 0.4,
        height: 0.3,
        createdAt: 10,
        updatedAt: 10,
      }

      const view = render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [annotation],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
          onAnnotationsChange,
          onSelectedAnnotationIdChange,
          onAnnotationColorChange,
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      setupImage(img, 200, 100, 200, 100)
      await fireEvent.load(img)
      await triggerResizeObservers(img)

      const shape = await screen.findByTestId('annotation-shape-ann-1')
      await fireEvent.click(shape)
      expect(onSelectedAnnotationIdChange).toHaveBeenCalledWith('ann-1')

      await view.rerender({
        path: '/path/to/image.jpg',
        type: 'image',
        assetUrl: 'asset://localhost/path/to/image.jpg',
        annotations: [annotation],
        selectedAnnotationId: 'ann-1',
        annotationTool: 'select',
        annotationColor: 'var(--color-accent)',
        onAnnotationsChange,
        onSelectedAnnotationIdChange,
        onAnnotationColorChange,
      })

      await fireEvent.click(screen.getByRole('button', { name: /warning annotation color/i }))
      expect(onAnnotationColorChange).toHaveBeenCalledWith('var(--color-warning)')
      expect(onAnnotationsChange).toHaveBeenCalledWith([
        expect.objectContaining({ id: 'ann-1', color: 'var(--color-warning)' }),
      ])

      await fireEvent.click(screen.getByRole('button', { name: /delete selected annotation/i }))
      expect(onAnnotationsChange).toHaveBeenCalledWith([])
      expect(onSelectedAnnotationIdChange).toHaveBeenCalledWith(null)

      const overlay = screen.getByTestId('annotation-overlay')
      overlay.getBoundingClientRect = vi.fn(() => ({
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 200,
        bottom: 100,
        width: 200,
        height: 100,
        toJSON: () => ({}),
      }))

      await fireEvent.pointerDown(overlay, { clientX: 199, clientY: 99, button: 0 })
      expect(onSelectedAnnotationIdChange).toHaveBeenLastCalledWith(null)
    })

    it('creates an underline annotation by horizontal drag with fixed stroke', async () => {
      const onAnnotationsChange = vi.fn()

      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'underline',
          annotationColor: 'var(--color-accent)',
          onAnnotationsChange,
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      setupImage(img, 200, 100, 200, 100)
      await fireEvent.load(img)
      await triggerResizeObservers(img)

      const overlay = await screen.findByTestId('annotation-overlay')
      overlay.getBoundingClientRect = vi.fn(() => ({
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 200,
        bottom: 100,
        width: 200,
        height: 100,
        toJSON: () => ({}),
      }))

      // Drag from (20,50) to (120,80) — vertical movement ignored for underline
      await fireEvent.pointerDown(overlay, { clientX: 20, clientY: 50, button: 0 })
      await fireEvent.pointerMove(overlay, { clientX: 120, clientY: 80, button: 0 })
      await fireEvent.pointerUp(overlay, { clientX: 120, clientY: 80, button: 0 })

      expect(onAnnotationsChange).toHaveBeenCalledTimes(1)
      const created = onAnnotationsChange.mock.calls[0]![0]![0]!
      expect(created.kind).toBe('underline')
      expect(created.width).toBeCloseTo(0.5, 3) // (120-20)/200 = 0.5
      expect(created.x).toBeCloseTo(0.1, 3) // 20/200 = 0.1
      expect(created.y).toBeCloseTo(0.49, 2) // startY 0.5 - 0.01
      expect(created.height).toBe(0.02)
    })

    it('renders underline annotations with non-scaling stroke', async () => {
      const annotation = {
        id: 'ann-ul',
        assetId: 'asset-1',
        page: 1,
        kind: 'underline' as const,
        color: 'var(--color-accent)',
        x: 0.1,
        y: 0.49,
        width: 0.5,
        height: 0.02,
        createdAt: 10,
        updatedAt: 10,
      }

      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [annotation],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      setupImage(img, 200, 100, 200, 100)
      await fireEvent.load(img)
      await triggerResizeObservers(img)

      const line = await screen.findByTestId('annotation-shape-ann-ul')
      // Fixed 2px stroke with non-scaling-stroke
      expect(line).toHaveAttribute('stroke-width', '2')
      expect(line).toHaveAttribute('vector-effect', 'non-scaling-stroke')
    })

    it('does not show the select/arrow tool button in the toolbar', () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      expect(
        screen.queryByRole('button', { name: /select annotation tool/i })
      ).not.toBeInTheDocument()
      expect(screen.getByTestId('annotation-toolbar')).toHaveAttribute(
        'aria-orientation',
        'vertical'
      )
      expect(screen.getByRole('button', { name: 'Pan image (hand tool)' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /rectangle annotation tool/i })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /underline annotation tool/i })).toBeInTheDocument()
      expect(
        screen
          .getByRole('button', { name: /rotate 90° left/i })
          .querySelector('svg.lucide-rotate-ccw')
      ).toBeInTheDocument()
      expect(
        screen
          .getByRole('button', { name: /rotate 90° right/i })
          .querySelector('svg.lucide-rotate-cw')
      ).toBeInTheDocument()
      expect(
        screen
          .getByRole('button', { name: /fine rotation left/i })
          .querySelector('[data-action-icon="rotate-fine-ccw"]')
      ).toBeInTheDocument()
      expect(
        screen
          .getByRole('button', { name: /fine rotation right/i })
          .querySelector('[data-action-icon="rotate-fine-cw"]')
      ).toBeInTheDocument()
      expect(screen.getByTestId('toolbar-fine-rotation-info')).toHaveTextContent('0°')
    })

    it('scales the vertical toolbar before switching cleanly to two columns', async () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      const container = img.closest('.document-viewer') as HTMLElement
      const toolbar = screen.getByTestId('annotation-toolbar')

      setupImage(img, 200, 100, 200, 100)
      setupContainer(container, 900, 650)
      await fireEvent.load(img)
      await triggerResizeObservers(container)

      expect(toolbar.getAttribute('style')).toMatch(
        /grid-template-columns:\s*repeat\(1,max-content\)/
      )
      const oneColumnScale = Number(
        toolbar.getAttribute('style')?.match(/--annotation-toolbar-scale:([^;]+)/)?.[1]
      )
      expect(oneColumnScale).toBeLessThan(1)
      expect(oneColumnScale).toBeGreaterThanOrEqual(0.78)

      setupContainer(container, 900, 520)
      await triggerResizeObservers(container)

      expect(toolbar.getAttribute('style')).toMatch(
        /grid-template-columns:\s*repeat\(2,max-content\)/
      )
      expect(toolbar.getAttribute('style')).toMatch(
        /grid-template-rows:\s*repeat\(11,max-content\)/
      )
    })

    it('fine-rotates the image in one-degree steps and clamps to thirty degrees', async () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      const container = img.closest('.document-viewer') as HTMLElement
      setupImage(img, 200, 100, 200, 100)
      setupContainer(container, 200, 100)
      await fireEvent.load(img)
      await triggerResizeObservers(container)

      const rotateLeft = screen.getByRole('button', { name: /fine rotation left/i })
      const rotateRight = screen.getByRole('button', { name: /fine rotation right/i })
      const rotationInfo = screen.getByTestId('toolbar-fine-rotation-info')
      const rotator = screen.getByTestId('image-rotator')

      await fireEvent.click(rotateRight)

      expect(rotationInfo).toHaveTextContent('+1°')
      expect(rotator.getAttribute('style')).toContain('rotate(1deg)')

      for (let i = 0; i < 40; i++) {
        await fireEvent.click(rotateRight)
      }

      expect(rotationInfo).toHaveTextContent('+30°')
      expect(rotateRight).toBeDisabled()

      for (let i = 0; i < 70; i++) {
        await fireEvent.click(rotateLeft)
      }

      expect(rotationInfo).toHaveTextContent('-30°')
      expect(rotateLeft).toBeDisabled()
      expect(rotator.getAttribute('style')).toContain('rotate(-30deg)')
    })

    it('fine-rotates from drag and wheel input on the toolbar buttons', async () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const rotateRight = screen.getByRole('button', { name: /fine rotation right/i })
      const rotationInfo = screen.getByTestId('toolbar-fine-rotation-info')

      await fireEvent.pointerDown(rotateRight, { pointerId: 2, clientX: 0, clientY: 0, button: 0 })
      await fireEvent.pointerMove(rotateRight, { pointerId: 2, clientX: 36, clientY: 0, button: 0 })
      await fireEvent.pointerUp(rotateRight, { pointerId: 2, clientX: 36, clientY: 0, button: 0 })

      expect(rotationInfo).toHaveTextContent('+3°')

      await fireEvent.wheel(rotateRight, { deltaY: 200 })

      expect(rotationInfo).toHaveTextContent('+5°')
    })

    it('commits fine rotation after click, wheel, and drag gestures with the final angle', async () => {
      const onFineRotateCommit = vi.fn()

      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
          onFineRotateCommit,
        },
      })

      const rotateRight = screen.getByRole('button', { name: /fine rotation right/i })

      await fireEvent.click(rotateRight)
      expect(onFineRotateCommit).toHaveBeenLastCalledWith(1)

      await fireEvent.wheel(rotateRight, { deltaY: 200 })
      expect(onFineRotateCommit).toHaveBeenLastCalledWith(3)

      await fireEvent.pointerDown(rotateRight, { pointerId: 2, clientX: 0, clientY: 0, button: 0 })
      await fireEvent.pointerMove(rotateRight, { pointerId: 2, clientX: 36, clientY: 0, button: 0 })
      await fireEvent.pointerUp(rotateRight, { pointerId: 2, clientX: 36, clientY: 0, button: 0 })

      expect(onFineRotateCommit).toHaveBeenLastCalledWith(6)
      expect(onFineRotateCommit).toHaveBeenCalledTimes(3)
    })

    it('toggles the hand pan tool and resets edit/annotation modes', async () => {
      const onAnnotationToolChange = vi.fn()
      const onEditToolChange = vi.fn()

      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'rectangle',
          annotationColor: 'var(--color-accent)',
          editTool: 'crop',
          onAnnotationToolChange,
          onEditToolChange,
        },
      })

      const panButton = screen.getByRole('button', { name: 'Pan image (hand tool)' })
      expect(panButton).toHaveAttribute('aria-pressed', 'false')

      await fireEvent.click(panButton)

      await waitFor(() => expect(panButton).toHaveAttribute('aria-pressed', 'true'))
      expect(onEditToolChange).toHaveBeenCalledWith('none')
      expect(onAnnotationToolChange).toHaveBeenCalledWith('select')

      await fireEvent.click(panButton)

      await waitFor(() => expect(panButton).toHaveAttribute('aria-pressed', 'false'))
    })

    it('pans the zoomed image without creating annotations while the hand tool is active', async () => {
      const onAnnotationsChange = vi.fn()

      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'rectangle',
          annotationColor: 'var(--color-accent)',
          onAnnotationsChange,
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      const container = img.closest('.document-viewer') as HTMLElement
      setupImage(img, 200, 100, 200, 100)
      setupContainer(container, 120, 80)
      await fireEvent.load(img)
      await triggerResizeObservers(container)

      await fireEvent.click(screen.getByRole('button', { name: 'Pan image (hand tool)' }))
      await waitFor(() =>
        expect(screen.getByRole('button', { name: 'Pan image (hand tool)' })).toHaveAttribute(
          'aria-pressed',
          'true'
        )
      )
      await fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))

      const overlay = await screen.findByTestId('annotation-overlay')
      overlay.getBoundingClientRect = vi.fn(() => ({
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 220,
        bottom: 110,
        width: 220,
        height: 110,
        toJSON: () => ({}),
      }))

      container.scrollLeft = 40
      container.scrollTop = 15

      await fireEvent.pointerDown(overlay, { clientX: 100, clientY: 60, button: 0, pointerId: 7 })
      await Promise.resolve()
      await fireEvent.pointerMove(overlay, { clientX: 70, clientY: 35, button: 0, pointerId: 7 })
      await fireEvent.pointerUp(overlay, { clientX: 70, clientY: 35, button: 0, pointerId: 7 })

      expect(container.scrollLeft).toBe(70)
      expect(container.scrollTop).toBe(40)
      expect(onAnnotationsChange).not.toHaveBeenCalled()
    })

    it('does not apply a crop selection when the pointer reaches the image edge before release', async () => {
      const onEditSelect = vi.fn()

      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
          editTool: 'crop',
          onEditSelect,
        },
      })

      const img = screen.getByRole('img') as HTMLImageElement
      const container = img.closest('.document-viewer') as HTMLElement
      setupImage(img, 200, 100, 200, 100)
      setupContainer(container, 200, 100)
      await fireEvent.load(img)
      await triggerResizeObservers(container)

      const overlay = await screen.findByTestId('annotation-overlay')
      overlay.getBoundingClientRect = vi.fn(() => ({
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 200,
        bottom: 100,
        width: 200,
        height: 100,
        toJSON: () => ({}),
      }))

      await fireEvent.pointerDown(overlay, { clientX: 20, clientY: 10, button: 0, pointerId: 3 })
      await fireEvent.pointerMove(overlay, { clientX: 200, clientY: 100, button: 0, pointerId: 3 })
      await fireEvent.pointerLeave(overlay, { clientX: 201, clientY: 101, pointerId: 3 })

      expect(onEditSelect).not.toHaveBeenCalled()
      expect(screen.getByTestId('edit-selection-rect')).toBeInTheDocument()
    })

    it('toggles tool off when clicking the already-active tool button', async () => {
      const onAnnotationToolChange = vi.fn()

      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'rectangle',
          annotationColor: 'var(--color-accent)',
          onAnnotationToolChange,
        },
      })

      await fireEvent.click(screen.getByRole('button', { name: /rectangle annotation tool/i }))
      expect(onAnnotationToolChange).toHaveBeenCalledWith('select')
    })

    it('collapses and expands the toolbar', async () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      expect(screen.getByTestId('annotation-toolbar')).toBeInTheDocument()

      await fireEvent.click(screen.getByRole('button', { name: /collapse annotation toolbar/i }))

      expect(screen.queryByTestId('annotation-toolbar')).not.toBeInTheDocument()
      expect(screen.getByTestId('annotation-toolbar-fab')).toBeInTheDocument()

      await fireEvent.click(screen.getByTestId('annotation-toolbar-fab'))

      expect(screen.getByTestId('annotation-toolbar')).toBeInTheDocument()
      expect(screen.queryByTestId('annotation-toolbar-fab')).not.toBeInTheDocument()
    })
  })

  describe('audio mode', () => {
    it('passes the native path to the audio fallback blob loader', async () => {
      Object.defineProperty(URL, 'createObjectURL', {
        configurable: true,
        value: vi.fn(() => 'blob:audio-fallback'),
      })
      Object.defineProperty(URL, 'revokeObjectURL', {
        configurable: true,
        value: vi.fn(),
      })
      const fetchMock = vi.fn()
      const audioFallbackBlobLoader = vi
        .fn()
        .mockResolvedValue(new Blob(['audio'], { type: 'audio/wav' }))
      vi.stubGlobal('fetch', fetchMock)

      render(DocumentViewer, {
        props: {
          path: 'C:/audio/interview.wav',
          type: 'audio',
          assetUrl: 'asset://localhost/audio/interview.wav',
          audioFallbackBlobLoader,
        },
      })
      const audio = screen.getByTestId('audio-player').querySelector('audio') as HTMLAudioElement

      await fireEvent.error(audio)

      await waitFor(() =>
        expect(audioFallbackBlobLoader).toHaveBeenCalledWith('C:/audio/interview.wav')
      )
      expect(fetchMock).not.toHaveBeenCalled()
    })
  })

  describe('pdf mode', () => {
    it('renders a canvas element for PDF', () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })
      const canvas = screen.getByTestId('pdf-canvas')
      expect(canvas).toBeInTheDocument()
    })

    it('renders the shared editing toolbar (asset navigation lives in the host view)', () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })
      expect(screen.getByTestId('annotation-toolbar')).toBeInTheDocument()
      expect(screen.queryByTestId('pdf-prev')).not.toBeInTheDocument()
      expect(screen.queryByTestId('pdf-next')).not.toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Zoom in' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Zoom out' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /rectangle annotation tool/i })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /crop to selection/i })).toBeInTheDocument()
    })

    it('uses 10 percent steps for pdf zoom controls', async () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      await waitFor(() => {
        expect(screen.getByTestId('toolbar-zoom-info')).toHaveTextContent('100%')
      })

      await fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))
      expect(screen.getByTestId('toolbar-zoom-info')).toHaveTextContent('110%')

      await fireEvent.click(screen.getByRole('button', { name: 'Zoom out' }))
      expect(screen.getByTestId('toolbar-zoom-info')).toHaveTextContent('100%')
    })

    it('fits the PDF page to the scroll container like images and re-fits on resize', async () => {
      const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext')
      getContext.mockReturnValue({} as CanvasRenderingContext2D)

      const pageRequest = deferred<typeof pdfMock.mockPage>()
      pdfMock.mockDocument.getPage.mockImplementation(() => pageRequest.promise)

      render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const scrollContainer = screen.getByTestId('pdf-scroll-container')
      await waitFor(() => expect(pdfMock.mockDocument.getPage).toHaveBeenCalled())
      // 800x600 page inside a 400x300 container -> fit scale 0.5 (100% = fit)
      setupContainer(scrollContainer, 400, 300)
      pageRequest.resolve(pdfMock.createPage(800, 600))
      await waitFor(() => {
        const canvas = screen.getByTestId('pdf-canvas') as HTMLCanvasElement
        expect(canvas.width).toBe(400)
        expect(canvas.height).toBe(300)
      })
      expect(screen.getByTestId('toolbar-zoom-info')).toHaveTextContent('100%')

      // Manual zoom composes on top of the fit (like imageZoom).
      await fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))
      await waitFor(() => {
        const canvas = screen.getByTestId('pdf-canvas') as HTMLCanvasElement
        expect(canvas.width).toBeCloseTo(440)
        expect(canvas.height).toBeCloseTo(330)
      })
      expect(screen.getByTestId('toolbar-zoom-info')).toHaveTextContent('110%')

      // Container resize re-fits at the current zoom multiplier.
      setupContainer(scrollContainer, 200, 150)
      await triggerResizeObservers(scrollContainer)
      await waitFor(() => {
        const canvas = screen.getByTestId('pdf-canvas') as HTMLCanvasElement
        expect(canvas.width).toBeCloseTo(220)
        expect(canvas.height).toBeCloseTo(165)
      })
      expect(screen.getByTestId('toolbar-zoom-info')).toHaveTextContent('110%')

      getContext.mockRestore()
    })

    it('anchors oversized PDF content to the real left scroll origin', () => {
      expect(documentViewerSource).toMatch(
        /\.document-viewer__canvas-container\s*\{[^}]*justify-content:\s*flex-start;[^}]*overflow:\s*auto;/s
      )
      expect(documentViewerSource).toMatch(
        /\.document-viewer__pdf-stage\s*\{[^}]*position:\s*relative;[^}]*flex:\s*0 0 auto;/s
      )
      expect(documentViewerSource).toMatch(
        /\.document-viewer__pdf-surface\s*>\s*canvas\s*\{[^}]*flex:\s*0 0 auto;/s
      )
    })

    it('resets PDF scroll to the left edge when changing assets', async () => {
      const view = render(DocumentViewer, {
        props: {
          path: '/path/to/doc-a.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc-a.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })
      const scrollContainer = screen.getByTestId('pdf-scroll-container')
      scrollContainer.scrollLeft = 120
      scrollContainer.scrollTop = 40

      await view.rerender({
        path: '/path/to/doc-b.pdf',
        type: 'pdf',
        assetUrl: 'asset://localhost/path/to/doc-b.pdf',
        annotations: [],
        selectedAnnotationId: null,
        annotationTool: 'select',
        annotationColor: 'var(--color-accent)',
      })

      expect(scrollContainer.scrollLeft).toBe(0)
      expect(scrollContainer.scrollTop).toBe(0)
    })

    it('ignores stale pdf renders after a newer render starts', async () => {
      const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext')
      getContext.mockReturnValue({} as CanvasRenderingContext2D)

      const initialPage = pdfMock.createPage()
      const stalePage = pdfMock.createPage()
      const latestPage = pdfMock.createPage()
      const stalePageRequest = deferred<typeof stalePage>()
      const onPageChange = vi.fn()

      pdfMock.mockDocument.getPage
        .mockResolvedValueOnce(initialPage)
        .mockReturnValueOnce(stalePageRequest.promise)
        .mockResolvedValueOnce(latestPage)

      render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
          onPageChange,
        },
      })

      await waitFor(() => expect(initialPage.render).toHaveBeenCalledTimes(1))

      await fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))
      await fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))
      await waitFor(() => expect(latestPage.render).toHaveBeenCalledTimes(1))

      stalePageRequest.resolve(stalePage)
      await Promise.resolve()

      expect(stalePage.render).not.toHaveBeenCalled()
      expect(onPageChange).toHaveBeenLastCalledWith(1, 3)
      expect(screen.getByTestId('toolbar-zoom-info')).toHaveTextContent('120%')

      getContext.mockRestore()
    })

    it('shows loading state initially', () => {
      render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })
      expect(screen.getByTestId('pdf-loading')).toBeInTheDocument()
    })

    it('creates normalized annotations on the rendered PDF page', async () => {
      const onAnnotationsChange = vi.fn()
      const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext')
      getContext.mockReturnValue({} as CanvasRenderingContext2D)

      render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'rectangle',
          annotationColor: 'var(--color-accent)',
          onAnnotationsChange,
        },
      })

      const overlay = await screen.findByTestId('annotation-overlay')
      overlay.getBoundingClientRect = vi.fn(() => ({
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 800,
        bottom: 600,
        width: 800,
        height: 600,
        toJSON: () => ({}),
      }))

      await fireEvent.pointerDown(overlay, { clientX: 80, clientY: 60, button: 0 })
      await fireEvent.pointerMove(overlay, { clientX: 400, clientY: 300, button: 0 })
      await fireEvent.pointerUp(overlay, { clientX: 400, clientY: 300, button: 0 })

      expect(onAnnotationsChange).toHaveBeenCalledWith([
        expect.objectContaining({ kind: 'rectangle', x: 0.1, y: 0.1, width: 0.4, height: 0.4 }),
      ])
      getContext.mockRestore()
    })

    it('keeps persisted PDF edits aligned while zooming and rotating the page', async () => {
      const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext')
      getContext.mockReturnValue({} as CanvasRenderingContext2D)
      const now = 10

      render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [
            {
              id: 'crop-1',
              assetId: 'asset-pdf',
              page: 1,
              kind: 'crop',
              color: '#fff',
              x: 0.25,
              y: 0.25,
              width: 0.5,
              height: 0.5,
              createdAt: now,
              updatedAt: now,
            },
            {
              id: 'erase-1',
              assetId: 'asset-pdf',
              page: 1,
              kind: 'erase',
              color: '#fff',
              x: 0.1,
              y: 0.2,
              width: 0.3,
              height: 0.2,
              createdAt: now,
              updatedAt: now,
            },
            {
              id: 'rotation-1',
              assetId: 'asset-pdf',
              page: 1,
              kind: 'rotation',
              color: '#fff',
              x: 1,
              y: 5,
              width: 0,
              height: 0,
              createdAt: now,
              updatedAt: now,
            },
          ],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      const erasure = await screen.findByTestId('document-erasure-erase-1')
      const rotator = screen.getByTestId('pdf-rotator')
      const cropFrame = rotator.querySelector('.document-viewer__pdf-crop-frame') as HTMLElement

      expect(erasure).toHaveAttribute('x', '80')
      expect(erasure).toHaveAttribute('y', '120')
      expect(rotator.getAttribute('style')).toContain('rotate(95deg)')
      expect(cropFrame.style.width).toBe('400px')
      expect(cropFrame.style.height).toBe('300px')

      await fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))

      await waitFor(() => expect(cropFrame.style.width).toBe('440px'))
      expect(cropFrame.style.height).toBe('330px')
      expect(erasure).toHaveAttribute('x', '80')
      expect(erasure).toHaveAttribute('y', '120')
      expect(rotator.getAttribute('style')).toContain('rotate(95deg)')

      getContext.mockRestore()
    })

    it('keeps the PDF edit selection aligned with the viewport after rotation', async () => {
      const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext')
      getContext.mockReturnValue({} as CanvasRenderingContext2D)

      render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [
            {
              id: 'rotation-1',
              assetId: 'asset-pdf',
              page: 1,
              kind: 'rotation',
              color: '#fff',
              x: 1,
              y: 0,
              width: 0,
              height: 0,
              createdAt: 10,
              updatedAt: 10,
            },
          ],
          editTool: 'crop',
        },
      })

      const editOverlay = await screen.findByTestId('pdf-edit-overlay')
      editOverlay.getBoundingClientRect = vi.fn(() => ({
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: 600,
        bottom: 800,
        width: 600,
        height: 800,
        toJSON: () => ({}),
      }))

      await fireEvent.pointerDown(editOverlay, { clientX: 60, clientY: 80, button: 0 })
      await fireEvent.pointerMove(editOverlay, { clientX: 300, clientY: 400, button: 0 })

      const selection = screen.getByTestId('edit-selection-rect')
      expect(selection.closest('[data-testid="pdf-rotator"]')).toBeNull()
      expect(selection).toHaveAttribute('x', '60')
      expect(selection).toHaveAttribute('y', '80')
      expect(selection).toHaveAttribute('width', '240')
      expect(selection).toHaveAttribute('height', '320')

      getContext.mockRestore()
    })

    it('commits a fine PDF rotation when returning exactly to zero degrees', async () => {
      const onFineRotateCommit = vi.fn()
      render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [
            {
              id: 'rotation-1',
              assetId: 'asset-pdf',
              page: 1,
              kind: 'rotation',
              color: '#fff',
              x: 0,
              y: 1,
              width: 0,
              height: 0,
              createdAt: 10,
              updatedAt: 10,
            },
          ],
          onFineRotateCommit,
        },
      })

      await waitFor(() =>
        expect(screen.getByTestId('toolbar-fine-rotation-info')).toHaveTextContent('+1°')
      )
      await fireEvent.click(screen.getByRole('button', { name: /fine rotation left/i }))

      expect(onFineRotateCommit).toHaveBeenLastCalledWith(0)
    })

    it('removes the old interactive overlay immediately when switching PDF assets', async () => {
      const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext')
      getContext.mockReturnValue({} as CanvasRenderingContext2D)
      const nextDocument = deferred<typeof pdfMock.mockDocument>()

      const view = render(DocumentViewer, {
        props: {
          path: '/path/to/doc-a.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc-a.pdf',
          annotations: [],
        },
      })
      await screen.findByTestId('annotation-overlay')

      pdfMock.getDocument.mockImplementationOnce(() => ({
        promise: nextDocument.promise,
        destroy: vi.fn(() => Promise.resolve()),
      }))
      await view.rerender({
        path: '/path/to/doc-b.pdf',
        type: 'pdf',
        assetUrl: 'asset://localhost/path/to/doc-b.pdf',
        annotations: [],
      })

      expect(screen.queryByTestId('annotation-overlay')).not.toBeInTheDocument()
      nextDocument.resolve(pdfMock.mockDocument)
      await screen.findByTestId('annotation-overlay')
      getContext.mockRestore()
    })

    it('shows PDF loading state when transitioning from image to pdf', async () => {
      const view = render(DocumentViewer, {
        props: {
          path: '/path/to/image.jpg',
          type: 'image',
          assetUrl: 'asset://localhost/path/to/image.jpg',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      expect(screen.queryByTestId('pdf-loading')).not.toBeInTheDocument()

      await view.rerender({
        path: '/path/to/doc.pdf',
        type: 'pdf',
        assetUrl: 'asset://localhost/path/to/doc.pdf',
        annotations: [],
        selectedAnnotationId: null,
        annotationTool: 'select',
        annotationColor: 'var(--color-accent)',
      })

      expect(screen.getByTestId('pdf-loading')).toBeInTheDocument()
    })

    it('hides PDF-only UI when transitioning from pdf to image', async () => {
      const view = render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      expect(screen.getByTestId('annotation-toolbar')).toBeInTheDocument()

      await view.rerender({
        path: '/path/to/image.jpg',
        type: 'image',
        assetUrl: 'asset://localhost/path/to/image.jpg',
        annotations: [],
        selectedAnnotationId: null,
        annotationTool: 'select',
        annotationColor: 'var(--color-accent)',
      })

      expect(screen.getByRole('img')).toHaveAttribute('src', 'asset://localhost/path/to/image.jpg')
      expect(screen.queryByTestId('annotation-toolbar')).toBeInTheDocument()
      expect(screen.queryByTestId('pdf-loading')).not.toBeInTheDocument()
    })

    it('reloads and destroys the previous document when the asset url changes while staying in pdf mode', async () => {
      const view = render(DocumentViewer, {
        props: {
          path: '/path/to/doc-a.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc-a.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      await waitFor(() => expect(pdfMock.getDocument).toHaveBeenCalledTimes(1))
      const firstTask = pdfMock.getDocument.mock.results[0]!.value

      await view.rerender({
        path: '/path/to/doc-b.pdf',
        type: 'pdf',
        assetUrl: 'asset://localhost/path/to/doc-b.pdf',
        annotations: [],
        selectedAnnotationId: null,
        annotationTool: 'select',
        annotationColor: 'var(--color-accent)',
      })

      await waitFor(() => expect(pdfMock.getDocument).toHaveBeenCalledTimes(2))
      expect(pdfMock.getDocument).toHaveBeenLastCalledWith('asset://localhost/path/to/doc-b.pdf')
      expect(firstTask.destroy).toHaveBeenCalledTimes(1)
    })

    it('destroys the pdf document when switching to image mode', async () => {
      const view = render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      await waitFor(() => expect(pdfMock.getDocument).toHaveBeenCalledTimes(1))
      const loadingTask = pdfMock.getDocument.mock.results[0]!.value

      await view.rerender({
        path: '/path/to/image.jpg',
        type: 'image',
        assetUrl: 'asset://localhost/path/to/image.jpg',
        annotations: [],
        selectedAnnotationId: null,
        annotationTool: 'select',
        annotationColor: 'var(--color-accent)',
      })

      // Exactly once — effect cleanup and resetViewerState must not double-destroy
      expect(loadingTask.destroy).toHaveBeenCalledTimes(1)
    })

    it('destroys the pdf document when the component unmounts', async () => {
      const view = render(DocumentViewer, {
        props: {
          path: '/path/to/doc.pdf',
          type: 'pdf',
          assetUrl: 'asset://localhost/path/to/doc.pdf',
          annotations: [],
          selectedAnnotationId: null,
          annotationTool: 'select',
          annotationColor: 'var(--color-accent)',
        },
      })

      await waitFor(() => expect(pdfMock.getDocument).toHaveBeenCalledTimes(1))
      const loadingTask = pdfMock.getDocument.mock.results[0]!.value

      view.unmount()

      expect(loadingTask.destroy).toHaveBeenCalledTimes(1)
    })
  })
})
