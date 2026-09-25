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
})
