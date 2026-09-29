import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { writing } from '$lib/writing'
import { CLOSING_EVENT, FLUSH_ACK_COMMAND, start, stop } from './app-close'

vi.mock('$lib/writing', () => ({
  writing: { flush: vi.fn() },
}))

// Mocks are set up in test-setup.ts:
//   @tauri-apps/api/core → invoke vi.fn()
//   @tauri-apps/api/event → listen vi.fn() returning Promise<vi.fn()>

const { invoke } = await import('@tauri-apps/api/core')
const { listen } = await import('@tauri-apps/api/event')

function closingHandler(): () => void {
  const handler = vi.mocked(listen).mock.calls.at(-1)?.[1] as unknown as () => void
  expect(typeof handler).toBe('function')
  return handler
}

describe('app-close handshake', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    stop()
  })

  it('awaits writing.flush() before acking the close', async () => {
    const order: string[] = []
    let resolveFlush: (() => void) | null = null
    vi.mocked(writing.flush).mockImplementation(() => {
      order.push('flush:start')
      return new Promise<void>((resolve) => {
        resolveFlush = () => {
          order.push('flush:end')
          resolve()
        }
      })
    })
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      order.push(`invoke:${command}`)
      return undefined
    })
    vi.mocked(listen).mockImplementation(() => Promise.resolve(vi.fn()))

    await start()
    closingHandler()()

    // The canonical save starts at once; the ack must not precede its settle.
    await Promise.resolve()
    expect(order).toEqual(['flush:start'])
    expect(invoke).not.toHaveBeenCalled()

    resolveFlush!()
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith(FLUSH_ACK_COMMAND)
    })
    expect(order).toEqual(['flush:start', 'flush:end', `invoke:${FLUSH_ACK_COMMAND}`])
  })

  it('acks exactly once even when writing.flush() rejects', async () => {
    vi.mocked(writing.flush).mockRejectedValue(new Error('save failed'))
    const acks: string[] = []
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      acks.push(command)
      return undefined
    })
    vi.mocked(listen).mockImplementation(() => Promise.resolve(vi.fn()))

    await start()
    closingHandler()()
    await vi.waitFor(() => {
      expect(acks).toEqual([FLUSH_ACK_COMMAND])
    })

    // A duplicate close event must not flush or ack a second time.
    closingHandler()()
    await Promise.resolve()
    expect(acks).toEqual([FLUSH_ACK_COMMAND])
    expect(vi.mocked(writing.flush)).toHaveBeenCalledTimes(1)
  })

  it('stop() removes the close listener', async () => {
    const unlisten = vi.fn()
    vi.mocked(listen).mockResolvedValue(unlisten)

    await start()
    expect(listen).toHaveBeenCalledTimes(1)
    expect(vi.mocked(listen).mock.calls[0]?.[0]).toBe(CLOSING_EVENT)

    stop()
    expect(unlisten).toHaveBeenCalledTimes(1)
  })

  it('stop() during start() unlistens the late registration', async () => {
    const unlisten = vi.fn()
    let resolveListen: ((fn: UnlistenFn) => void) | null = null
    vi.mocked(listen).mockImplementation(
      () =>
        new Promise<UnlistenFn>((resolve) => {
          resolveListen = resolve
        }),
    )

    const started = start()
    stop()
    resolveListen!(unlisten)
    await started

    expect(unlisten).toHaveBeenCalledTimes(1)
  })
})
