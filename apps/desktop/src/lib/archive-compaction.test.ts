import { get } from 'svelte/store'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { COMPACTING_EVENT, compacting, start, stop } from './archive-compaction'

const { listen } = await import('@tauri-apps/api/event')

type Handler = (event: { payload: boolean }) => void

function handler(): Handler {
  const registered = vi.mocked(listen).mock.calls.at(-1)?.[1] as unknown as Handler
  expect(typeof registered).toBe('function')
  return registered
}

describe('archive compaction notice', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    stop()
  })

  it('shows while Rust compacts and clears when it ends', async () => {
    vi.mocked(listen).mockImplementation(() => Promise.resolve(vi.fn()))
    await start()
    expect(vi.mocked(listen).mock.calls[0]?.[0]).toBe(COMPACTING_EVENT)
    expect(get(compacting)).toBe(false)

    handler()({ payload: true })
    expect(get(compacting)).toBe(true)

    handler()({ payload: false })
    expect(get(compacting)).toBe(false)
  })

  it('only an explicit true shows the notice', async () => {
    vi.mocked(listen).mockImplementation(() => Promise.resolve(vi.fn()))
    await start()
    handler()({ payload: undefined as unknown as boolean })
    expect(get(compacting)).toBe(false)
  })

  it('stop() clears a visible notice and removes the listener', async () => {
    const unlisten = vi.fn()
    vi.mocked(listen).mockResolvedValue(unlisten)
    await start()
    handler()({ payload: true })

    stop()

    expect(unlisten).toHaveBeenCalledTimes(1)
    expect(get(compacting)).toBe(false)
  })

  it('stop() during start() unlistens the late registration', async () => {
    const unlisten = vi.fn()
    let resolveListen: ((fn: UnlistenFn) => void) | null = null
    vi.mocked(listen).mockImplementation(
      () =>
        new Promise<UnlistenFn>((resolve) => {
          resolveListen = resolve
        })
    )

    const started = start()
    stop()
    resolveListen!(unlisten)
    await started

    expect(unlisten).toHaveBeenCalledTimes(1)
  })
})
