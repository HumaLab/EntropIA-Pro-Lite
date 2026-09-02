import { describe, it, expect, vi, beforeEach } from 'vitest'

const setZoomMock = vi.fn()
const settingsGetMock = vi.fn()
const settingsSetMock = vi.fn()

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ setZoom: setZoomMock }),
}))

vi.mock('./settings', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./settings')>()
  return { ...actual, settingsGet: settingsGetMock, settingsSet: settingsSetMock }
})

type ZoomModule = typeof import('./zoom')

let zoom: ZoomModule

beforeEach(async () => {
  vi.clearAllMocks()
  setZoomMock.mockResolvedValue(undefined)
  settingsSetMock.mockResolvedValue(undefined)
  settingsGetMock.mockResolvedValue(null)
  // Module-level state (the current factor) has to start fresh per test.
  vi.resetModules()
  zoom = await import('./zoom')
})

describe('clampZoom', () => {
  it('keeps a factor that already sits on a step', () => {
    expect(zoom.clampZoom(1.1)).toBe(1.1)
    expect(zoom.clampZoom(0.9)).toBe(0.9)
  })

  it('clamps to the ±25% range', () => {
    expect(zoom.clampZoom(0.1)).toBe(zoom.ZOOM_MIN)
    expect(zoom.clampZoom(4)).toBe(zoom.ZOOM_MAX)
  })

  it('snaps off-step factors to the nearest 5%', () => {
    expect(zoom.clampZoom(1.03)).toBe(1.05)
    expect(zoom.clampZoom(0.921)).toBe(0.9)
  })

  it('falls back to 100% for values that are not usable numbers', () => {
    expect(zoom.clampZoom(Number.NaN)).toBe(zoom.ZOOM_DEFAULT)
    expect(zoom.clampZoom(Number.POSITIVE_INFINITY)).toBe(zoom.ZOOM_DEFAULT)
  })
})

describe('zoomIn / zoomOut / resetZoom', () => {
  it('steps up by 5% and pushes the factor to the webview', async () => {
    await expect(zoom.zoomIn()).resolves.toBe(1.05)
    expect(setZoomMock).toHaveBeenCalledWith(1.05)
    expect(zoom.getZoom()).toBe(1.05)
  })

  it('steps down by 5%', async () => {
    await expect(zoom.zoomOut()).resolves.toBe(0.95)
    expect(setZoomMock).toHaveBeenCalledWith(0.95)
  })

  it('does not accumulate floating point drift across steps', async () => {
    await zoom.zoomOut()
    await zoom.zoomOut()
    await zoom.zoomOut()
    expect(zoom.getZoom()).toBe(0.85)
  })

  it('stops at the upper bound', async () => {
    for (let i = 0; i < 10; i++) await zoom.zoomIn()
    expect(zoom.getZoom()).toBe(zoom.ZOOM_MAX)
  })

  it('stops at the lower bound', async () => {
    for (let i = 0; i < 10; i++) await zoom.zoomOut()
    expect(zoom.getZoom()).toBe(zoom.ZOOM_MIN)
  })

  it('resets back to 100%', async () => {
    await zoom.zoomIn()
    await expect(zoom.resetZoom()).resolves.toBe(zoom.ZOOM_DEFAULT)
    expect(setZoomMock).toHaveBeenLastCalledWith(zoom.ZOOM_DEFAULT)
  })

  it('persists the applied factor', async () => {
    await zoom.zoomIn()
    expect(settingsSetMock).toHaveBeenCalledWith(zoom.ZOOM_SETTING_KEY, '1.05')
  })

  it('keeps working when persistence fails', async () => {
    settingsSetMock.mockRejectedValue(new Error('db down'))
    await expect(zoom.zoomIn()).resolves.toBe(1.05)
    expect(zoom.getZoom()).toBe(1.05)
  })

  it('reverts and does not persist when the webview refuses the zoom', async () => {
    setZoomMock.mockRejectedValue(new Error('no webview'))
    await expect(zoom.zoomIn()).resolves.toBe(zoom.ZOOM_DEFAULT)
    expect(zoom.getZoom()).toBe(zoom.ZOOM_DEFAULT)
    expect(settingsSetMock).not.toHaveBeenCalled()
  })
})

describe('initZoom', () => {
  it('restores the persisted factor', async () => {
    settingsGetMock.mockResolvedValue('1.15')
    await expect(zoom.initZoom()).resolves.toBe(1.15)
    expect(settingsGetMock).toHaveBeenCalledWith(zoom.ZOOM_SETTING_KEY)
    expect(setZoomMock).toHaveBeenCalledWith(1.15)
  })

  it('applies 100% when nothing was persisted, so a stale webview zoom is cleared', async () => {
    await expect(zoom.initZoom()).resolves.toBe(zoom.ZOOM_DEFAULT)
    expect(setZoomMock).toHaveBeenCalledWith(zoom.ZOOM_DEFAULT)
  })

  it('clamps a corrupt persisted factor', async () => {
    settingsGetMock.mockResolvedValue('9')
    await expect(zoom.initZoom()).resolves.toBe(zoom.ZOOM_MAX)
  })

  it('does not write the setting back during restore', async () => {
    settingsGetMock.mockResolvedValue('0.9')
    await zoom.initZoom()
    expect(settingsSetMock).not.toHaveBeenCalled()
  })

  it('falls back to 100% when the setting cannot be read', async () => {
    settingsGetMock.mockRejectedValue(new Error('db down'))
    await expect(zoom.initZoom()).resolves.toBe(zoom.ZOOM_DEFAULT)
  })
})

describe('zoomFactor store', () => {
  function collect(): { values: number[]; stop: () => void } {
    const values: number[] = []
    const stop = zoom.zoomFactor.subscribe((value) => values.push(value))
    return { values, stop }
  }

  it('publishes the current factor on subscribe', () => {
    const { values, stop } = collect()
    expect(values).toEqual([zoom.ZOOM_DEFAULT])
    stop()
  })

  it('publishes every applied change, and nothing when the factor is unchanged', async () => {
    const { values, stop } = collect()
    await zoom.zoomIn()
    await zoom.zoomOut()
    // Already at 100%, so this reset moves nothing and must stay silent.
    await zoom.resetZoom()
    expect(values).toEqual([1, 1.05, 1])
    stop()
  })

  it('publishes the restored factor on init', async () => {
    settingsGetMock.mockResolvedValue('0.9')
    const { values, stop } = collect()
    await zoom.initZoom()
    expect(values.at(-1)).toBe(0.9)
    stop()
  })

  it('does not publish a factor the webview refused', async () => {
    setZoomMock.mockRejectedValue(new Error('no webview'))
    const { values, stop } = collect()
    await zoom.zoomIn()
    expect(values).toEqual([zoom.ZOOM_DEFAULT])
    stop()
  })
})
