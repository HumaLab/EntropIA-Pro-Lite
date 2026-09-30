import { describe, expect, it, vi } from 'vitest'
import { computeBounds, createViewerSession, type ViewerApi, type ViewerBounds } from './navegador'

const rect = (left: number, top: number, width: number, height: number) => ({
  left,
  top,
  width,
  height,
})

describe('computeBounds', () => {
  it('passes a rect through at 100% zoom, rounded to whole pixels', () => {
    expect(computeBounds(rect(10, 48, 800, 600), 1)).toEqual({
      x: 10,
      y: 48,
      width: 800,
      height: 600,
    })
    expect(computeBounds(rect(10.4, 47.6, 799.5, 600.2), 1)).toEqual({
      x: 10,
      y: 48,
      width: 800,
      height: 600,
    })
  })

  it('scales by the webview zoom: CSS pixels times zoom are the logical pixels', () => {
    expect(computeBounds(rect(100, 40, 400, 200), 1.25)).toEqual({
      x: 125,
      y: 50,
      width: 500,
      height: 250,
    })
    expect(computeBounds(rect(100, 40, 400, 200), 0.75)).toEqual({
      x: 75,
      y: 30,
      width: 300,
      height: 150,
    })
  })

  it('defaults to no zoom', () => {
    expect(computeBounds(rect(1, 2, 30, 40))).toEqual({ x: 1, y: 2, width: 30, height: 40 })
  })

  it('keeps a negative origin: a pane can start partly outside the window', () => {
    expect(computeBounds(rect(-12, -3, 300, 200), 1)).toMatchObject({ x: -12, y: -3 })
  })

  it('has nothing to place for an empty or broken rect', () => {
    expect(computeBounds(rect(0, 0, 0, 200), 1)).toBeNull()
    expect(computeBounds(rect(0, 0, 200, 0.4), 1)).toBeNull()
    expect(computeBounds(rect(0, 0, Number.NaN, 200), 1)).toBeNull()
    expect(computeBounds(rect(Number.POSITIVE_INFINITY, 0, 100, 200), 1)).toBeNull()
  })

  it('ignores an unusable zoom instead of sending garbage', () => {
    for (const zoom of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(computeBounds(rect(10, 10, 100, 100), zoom)).toEqual({
        x: 10,
        y: 10,
        width: 100,
        height: 100,
      })
    }
  })
})

function fakeApi() {
  const calls: string[] = []
  const api: ViewerApi = {
    open: vi.fn(async (url: string, b: ViewerBounds) => {
      calls.push(`open ${url} ${b.width}x${b.height}`)
    }),
    setBounds: vi.fn(async (b: ViewerBounds) => {
      calls.push(`bounds ${b.width}x${b.height}`)
    }),
    setVisible: vi.fn(async (visible: boolean) => {
      calls.push(`visible ${visible}`)
    }),
    close: vi.fn(async () => {
      calls.push('close')
    }),
  }
  return { api, calls }
}

const bounds = (width: number, height: number): ViewerBounds => ({ x: 0, y: 0, width, height })

describe('createViewerSession', () => {
  it('opens the browser the first time it is shown and reuses it afterwards', async () => {
    const { api, calls } = fakeApi()
    const session = createViewerSession(api)
    session.attach('a')
    await session.show('a', 'https://example.com/', bounds(800, 600))
    await session.show('a', 'https://example.com/', bounds(700, 500))
    expect(calls).toEqual(['open https://example.com/ 800x600', 'bounds 700x500', 'visible true'])
  })

  it('hides the browser when its view is hidden, and shows it again', async () => {
    const { api, calls } = fakeApi()
    const session = createViewerSession(api)
    session.attach('a')
    await session.show('a', 'https://example.com/', bounds(800, 600))
    await session.hide('a')
    await session.show('a', 'https://example.com/', bounds(800, 600))
    expect(calls).toEqual([
      'open https://example.com/ 800x600',
      'visible false',
      'bounds 800x600',
      'visible true',
    ])
  })

  it('closes the browser when the last view goes away', async () => {
    const { api, calls } = fakeApi()
    const session = createViewerSession(api)
    session.attach('a')
    await session.show('a', 'https://example.com/', bounds(800, 600))
    await session.detach('a')
    expect(calls).toEqual(['open https://example.com/ 800x600', 'close'])
  })

  it('keeps the browser, hidden, while another view is still mounted', async () => {
    const { api, calls } = fakeApi()
    const session = createViewerSession(api)
    session.attach('a')
    session.attach('b')
    await session.show('a', 'https://example.com/', bounds(800, 600))
    await session.detach('a')
    expect(calls).toEqual(['open https://example.com/ 800x600', 'visible false'])
    await session.detach('b')
    expect(calls.at(-1)).toBe('close')
  })

  it('lets the view shown last own the browser, and ignores the previous owner', async () => {
    const { api, calls } = fakeApi()
    const session = createViewerSession(api)
    session.attach('a')
    session.attach('b')
    await session.show('a', 'https://example.com/', bounds(800, 600))
    await session.show('b', 'https://example.com/', bounds(400, 300))
    calls.length = 0
    await session.setBounds('a', bounds(1, 1))
    await session.hide('a')
    expect(calls).toEqual([])
    await session.setBounds('b', bounds(500, 400))
    expect(calls).toEqual(['bounds 500x400'])
  })

  it('says whether a browser is open, so a second view knows to navigate it', async () => {
    const { api } = fakeApi()
    const session = createViewerSession(api)
    session.attach('a')
    expect(session.isOpen()).toBe(false)
    await session.show('a', 'https://example.com/', bounds(800, 600))
    expect(session.isOpen()).toBe(true)
    await session.detach('a')
    expect(session.isOpen()).toBe(false)
  })

  it('does not open a browser for a view that never showed one', async () => {
    const { api, calls } = fakeApi()
    const session = createViewerSession(api)
    session.attach('a')
    await session.setBounds('a', bounds(800, 600))
    await session.hide('a')
    await session.detach('a')
    expect(calls).toEqual([])
  })

  it('runs its calls in the order they were made', async () => {
    const order: string[] = []
    let releaseOpen!: () => void
    const api: ViewerApi = {
      open: () =>
        new Promise<void>((resolve) => {
          releaseOpen = () => {
            order.push('open done')
            resolve()
          }
        }),
      setBounds: async () => void order.push('bounds'),
      setVisible: async () => void order.push('visible'),
      close: async () => void order.push('close'),
    }
    const session = createViewerSession(api)
    session.attach('a')
    const shown = session.show('a', 'https://example.com/', bounds(800, 600))
    const detached = session.detach('a')
    await Promise.resolve()
    expect(order).toEqual([])
    releaseOpen()
    await Promise.all([shown, detached])
    expect(order).toEqual(['open done', 'close'])
  })

  it('survives a failing call and keeps going', async () => {
    const { api, calls } = fakeApi()
    vi.mocked(api.open).mockRejectedValueOnce(new Error('boom'))
    const session = createViewerSession(api)
    session.attach('a')
    await expect(session.show('a', 'https://example.com/', bounds(800, 600))).rejects.toThrow(
      'boom'
    )
    await session.show('a', 'https://example.com/', bounds(800, 600))
    expect(calls).toEqual(['open https://example.com/ 800x600'])
  })
})
