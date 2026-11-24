import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { render, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import LogsTab from './LogsTab.svelte'

const logsMocks = vi.hoisted(() => ({
  getLogs: vi.fn(),
  onLogEntry: vi.fn(),
}))

vi.mock('$lib/logs', () => ({
  clearLogs: vi.fn(),
  formatLogEntry: (entry: unknown) => String(entry),
  getLogs: logsMocks.getLogs,
  onLogEntry: logsMocks.onLogEntry,
  openLogsDir: vi.fn(),
}))

describe('LogsTab', () => {
  // The log listener registers only after the initial refresh. A tab torn
  // down meanwhile (switching Settings tabs) used to store the unlisten on a
  // dead component and never call it, leaving the listener live for the rest
  // of the session (drop-dup fix, same leak class as CollectionView's).
  it('releases a log listener that finishes registering after the tab is gone', async () => {
    logsMocks.getLogs.mockResolvedValue([])
    let resolveListener: ((unlisten: () => void) => void) | undefined
    logsMocks.onLogEntry.mockImplementation(
      () =>
        new Promise<() => void>((resolve) => {
          resolveListener = resolve
        })
    )

    const { unmount } = render(LogsTab)
    await waitFor(() => expect(resolveListener).toBeDefined())
    unmount()

    const unlisten = vi.fn()
    resolveListener!(unlisten)
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(unlisten).toHaveBeenCalledOnce()
  })

  it('registers no log listener when the tab is gone before the initial refresh settles', async () => {
    let resolveLogs: ((entries: unknown[]) => void) | undefined
    logsMocks.getLogs.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveLogs = resolve
        })
    )
    logsMocks.onLogEntry.mockReset().mockResolvedValue(vi.fn())

    const { unmount } = render(LogsTab)
    await waitFor(() => expect(resolveLogs).toBeDefined())
    unmount()
    resolveLogs!([])
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(logsMocks.onLogEntry).not.toHaveBeenCalled()
  })

  // Release 1.8.7 findings: new lines did arrive live, but the list stayed
  // where it was, so they landed below the fold and looked missing until
  // "Refrescar". The list now follows the newest line, unless the user
  // scrolled up to read an older one.
  function entry(id: number) {
    return { id, timestamp_ms: 0, level: 'info', source: 'test', message: `line ${id}` }
  }

  function fakeScroller(body: HTMLElement, clientHeight: number) {
    let height = 0
    Object.defineProperty(body, 'clientHeight', { configurable: true, get: () => clientHeight })
    Object.defineProperty(body, 'scrollHeight', { configurable: true, get: () => height })
    return {
      grow: (by: number) => {
        height += by
      },
    }
  }

  async function renderWithListener() {
    let push: ((value: ReturnType<typeof entry>) => void) | undefined
    logsMocks.onLogEntry.mockReset().mockImplementation(async (callback) => {
      push = callback
      return vi.fn()
    })
    const view = render(LogsTab)
    await waitFor(() => expect(push).toBeDefined())
    const body = view.container.querySelector('.logs-tab__body') as HTMLElement
    return { body, push: (value: ReturnType<typeof entry>) => push!(value) }
  }

  it('opens on the newest line', async () => {
    logsMocks.getLogs.mockResolvedValue([entry(1), entry(2)])
    const scrollHeight = vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockReturnValue(900)
    try {
      const { body } = await renderWithListener()
      await waitFor(() => expect(body.scrollTop).toBe(900))
    } finally {
      scrollHeight.mockRestore()
    }
  })

  it('follows a live line when the list is at its end', async () => {
    logsMocks.getLogs.mockResolvedValue([])
    const { body, push } = await renderWithListener()
    const scroller = fakeScroller(body, 300)
    scroller.grow(600)
    body.scrollTop = 300
    push(entry(3))
    scroller.grow(40)
    await waitFor(() => expect(body.scrollTop).toBe(640))
  })

  it('leaves the list alone when the user scrolled up to read', async () => {
    logsMocks.getLogs.mockResolvedValue([])
    const { body, push } = await renderWithListener()
    const scroller = fakeScroller(body, 300)
    scroller.grow(600)
    body.scrollTop = 100
    push(entry(4))
    scroller.grow(40)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(body.scrollTop).toBe(100)
  })

  it('keeps the last line clear of the floating help button', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'LogsTab.svelte'), 'utf-8')
    const rule = source.match(/\.logs-tab__body\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(rule).toMatch(/padding-block-end:\s*calc\(56px/)
  })
})
