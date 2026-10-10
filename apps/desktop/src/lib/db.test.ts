import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { initStore } from '@entropia/store'
import { ensureSyncCapture } from '$lib/sync'
import { initDb } from './db'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))
vi.mock('@entropia/store', () => ({
  initStore: vi.fn(),
}))
vi.mock('$lib/sync', () => ({
  ensureSyncCapture: vi.fn(),
}))

const mockInvoke = vi.mocked(invoke)
const mockInitStore = vi.mocked(initStore)
const mockEnsureSyncCapture = vi.mocked(ensureSyncCapture)

const fakeStore = {} as Awaited<ReturnType<typeof initStore>>

describe('initDb and the one-shot migration window (S-02c)', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockInvoke.mockResolvedValue(undefined)
    mockEnsureSyncCapture.mockResolvedValue(undefined)
    mockInitStore.mockResolvedValue(fakeStore)
  })

  it('opens the window before the first db_* call and closes it after initStore', async () => {
    const commands: string[] = []
    mockInvoke.mockImplementation(async (command: string) => {
      commands.push(command)
      return command === 'db_execute' ? { rowsAffected: 0 } : undefined
    })
    // initStore goes through the real tauri-db-client, so its first execute is
    // a real `db_execute` invoke: the window must already be open by then.
    mockInitStore.mockImplementation(async (client) => {
      await client.execute('CREATE TABLE probe (id TEXT)', [])
      return fakeStore
    })

    await initDb()

    expect(commands).toEqual([
      'db_migration_window_begin',
      'db_execute',
      'db_migration_window_end',
      'processing_initialize',
    ])
  })

  it('a refused begin is logged and never blocks startup', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'db_migration_window_begin') {
        throw new Error('Migration window is already closed for this process')
      }
      return undefined
    })
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})

    await expect(initDb()).resolves.toBeUndefined()

    expect(warn).toHaveBeenCalledWith(
      '[db] db_migration_window_begin failed:',
      expect.any(Error),
    )
    expect(mockInitStore).toHaveBeenCalledTimes(1)
    expect(mockInvoke).toHaveBeenCalledWith('db_migration_window_end')
    expect(mockInvoke).toHaveBeenCalledWith('processing_initialize')
    warn.mockRestore()
  })

  it('closes the window even when initStore fails', async () => {
    mockInitStore.mockRejectedValueOnce(new Error('database unavailable'))

    await expect(initDb()).rejects.toThrow('database unavailable')

    expect(mockInvoke).toHaveBeenCalledWith('db_migration_window_end')
    expect(mockEnsureSyncCapture).not.toHaveBeenCalled()
    expect(mockInvoke).not.toHaveBeenCalledWith('processing_initialize')
  })
})
