import { afterEach, describe, expect, it, vi } from 'vitest'
import { createMarkupViewport } from '../geometry'
import {
  ThumbnailRenderer,
  renderThumbnail,
  thumbnailRegion,
  type ThumbnailDocument,
} from '../thumbnails'

function fakeDocument() {
  const render = vi.fn(() => ({ promise: Promise.resolve() }))
  const getPage = vi.fn(async () => ({
    getViewport: ({ scale }: { scale: number }) => createMarkupViewport([0, 0, 612, 792], scale, 0),
    render,
  }))
  return { doc: { getPage } as unknown as ThumbnailDocument, render, getPage }
}

function stubCanvas() {
  const context = {
    beginPath: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    stroke: vi.fn(),
  }
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(
    context as unknown as CanvasRenderingContext2D
  )
  vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockReturnValue('data:image/png;base64,AAA')
  return context
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('annotation thumbnails', () => {
  it('shows the bounds of the rects, or of the strokes with their width', () => {
    expect(thumbnailRegion({ id: 'a', pageIndex: 0, rects: [[10, 20, 30, 40], [5, 25, 15, 50]] })).toEqual([
      5, 20, 30, 50,
    ])
    expect(thumbnailRegion({ id: 'b', pageIndex: 0, paths: [[10, 10, 20, 20]], width: 1 })).toEqual([
      8, 8, 22, 22,
    ])
    expect(thumbnailRegion({ id: 'c', pageIndex: 0 })).toBeNull()
  })

  it('renders only the region, scaled so its longest side fits', async () => {
    stubCanvas()
    const { doc, render, getPage } = fakeDocument()
    const url = await renderThumbnail(doc, { id: 'a', pageIndex: 2, rects: [[100, 600, 420, 680]] }, 160)
    expect(url).toBe('data:image/png;base64,AAA')
    expect(getPage).toHaveBeenCalledWith(3)
    const options = (render.mock.calls[0] as unknown as [{ viewport: { scale: number }; transform: number[] }])[0]
    expect(options.viewport.scale).toBeCloseTo(0.5)
    expect(options.transform).toEqual([1, 0, 0, 1, -50, -56])
  })

  it('draws ink strokes over the page', async () => {
    const context = stubCanvas()
    const { doc } = fakeDocument()
    await renderThumbnail(doc, { id: 'ink', pageIndex: 0, paths: [[10, 10, 50, 50, 90, 10]], color: '#ff6666' })
    expect(context.moveTo).toHaveBeenCalledTimes(1)
    expect(context.lineTo).toHaveBeenCalledTimes(2)
    expect(context.stroke).toHaveBeenCalledTimes(1)
  })

  it('caches by revision and renders at most two at once', async () => {
    let active = 0
    let peak = 0
    const render = vi.fn(async () => {
      active++
      peak = Math.max(peak, active)
      await new Promise((resolve) => setTimeout(resolve, 5))
      active--
      return 'url'
    })
    const renderer = new ThumbnailRenderer({} as ThumbnailDocument, 2, render)
    const requests = ['a', 'b', 'c', 'd'].map((id) => ({ id, pageIndex: 0, rects: [[0, 0, 1, 1]] as [number, number, number, number][] }))
    await Promise.all(requests.map((request) => renderer.thumbnail(request)))
    await renderer.thumbnail(requests[0]!)
    expect(render).toHaveBeenCalledTimes(4)
    expect(peak).toBe(2)
  })
})
