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

describe('initDb and the verify-only migration gate', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockInvoke.mockResolvedValue(undefined)
    mockEnsureSyncCapture.mockResolvedValue(undefined)
    mockInitStore.mockResolvedValue(fakeStore)
  })

  it('runs initStore, sync capture and processing init without the removed migration window', async () => {
    const order: string[] = []
    mockInitStore.mockImplementation(async () => {
      order.push('initStore')
      return fakeStore
    })
    mockEnsureSyncCapture.mockImplementation(async () => {
      order.push('syncCapture')
    })
    mockInvoke.mockImplementation(async (command: string) => {
      order.push(command)
      return undefined
    })

    await initDb()

    expect(order).toEqual(['initStore', 'syncCapture', 'processing_initialize'])
    expect(mockInvoke).not.toHaveBeenCalledWith('db_migration_window_begin')
    expect(mockInvoke).not.toHaveBeenCalledWith('db_migration_window_end')
  })

  it('surfaces a migration verify failure through the existing init error path', async () => {
    mockInitStore.mockRejectedValueOnce(
      new Error('Archive is missing migrations applied by the backend: 0058_processing_ner_tasks')
    )

    await expect(initDb()).rejects.toThrow(
      'Archive is missing migrations applied by the backend: 0058_processing_ner_tasks'
    )

    expect(mockInvoke).not.toHaveBeenCalled()
    expect(mockEnsureSyncCapture).not.toHaveBeenCalled()
  })
})
