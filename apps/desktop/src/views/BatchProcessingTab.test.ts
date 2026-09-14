import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import BatchProcessingTab from './BatchProcessingTab.svelte'
import { batchStore } from '$lib/batch-processing'

const mockInvoke = vi.mocked(invoke)

const { storeRef } = vi.hoisted(() => ({
  storeRef: {
    current: {
      collections: {
        findAll: vi.fn(),
        countItems: vi.fn(),
      },
    },
  },
}))

vi.mock('$lib/db', () => ({
  getStore: () => storeRef.current,
  getProcessingInitState: () => ({ summary: null, error: null }),
}))

const COLLECTIONS = [
  { id: 'c1', name: 'Legajo 1', description: null, createdAt: 1, updatedAt: 1 },
  { id: 'c2', name: 'Fotos', description: null, createdAt: 2, updatedAt: 2 },
]

function draftSnapshot() {
  return {
    id: 'b-draft',
    requestId: 'req-1',
    origin: 'user',
    state: 'preparing',
    desiredState: 'pause',
    operations: ['ocr'],
    planningCursor: 0,
    planningDone: false,
    revision: 0,
    createdAt: 1,
    updatedAt: 1,
    startedAt: null,
    finishedAt: null,
    lastError: null,
    membersTotal: 4,
    membersClassified: 1,
    tasksByState: [],
    tasksByKind: [],
    collections: [{ id: 'c1', name: 'Legajo 1' }],
  }
}

function runningSnapshot() {
  return {
    ...draftSnapshot(),
    id: 'b-run',
    state: 'running',
    desiredState: 'run',
    planningDone: true,
    revision: 2,
    membersClassified: 4,
    tasksByState: [
      { name: 'pending', count: 1 },
      { name: 'failed', count: 1 },
      { name: 'succeeded', count: 2 },
    ],
    tasksByKind: [{ name: 'ocr', count: 4 }],
  }
}

function failedTask() {
  return {
    taskId: 'ocr-a9',
    kind: 'ocr',
    assetId: 'a9',
    state: 'failed',
    stage: '',
    progressDone: 0,
    progressTotal: 1,
    outcome: '',
    attemptCount: 3,
    retryCycle: 0,
    nextRetryAt: null,
    errorCode: 'corrupt_pdf',
    errorMessage: 'encrypted and locked',
    updatedAt: 5,
    requestState: 'active',
    dependencyTaskId: null,
  }
}

beforeEach(() => {
  storeRef.current.collections.findAll.mockResolvedValue(COLLECTIONS)
  storeRef.current.collections.countItems.mockResolvedValue(3)
  mockInvoke.mockImplementation(async (command: string) => {
    switch (command) {
      case 'processing_list_batches':
        return { batches: [], nextCursor: null }
      default:
        return undefined
    }
  })
})

afterEach(() => {
  batchStore.destroy()
  vi.clearAllMocks()
})

describe('BatchProcessingTab batch controls', () => {
  it('analyzes the selected collections and offers to start the draft', async () => {
    mockInvoke.mockImplementation(async (command: string, ...rest: unknown[]) => {
      const args = rest[0] as Record<string, unknown> | undefined
      if (command === 'processing_list_batches') return { batches: [], nextCursor: null }
      if (command === 'processing_prepare') {
        expect(args?.['collectionIds']).toEqual(['c1'])
        expect(args?.['operations']).toEqual(['ocr', 'embeddings'])
        return { batchId: 'b-draft', created: true, members: 4 }
      }
      if (command === 'processing_get_batch') return draftSnapshot()
      return undefined
    })
    render(BatchProcessingTab)

    await screen.findByText('Legajo 1 (3)')
    await fireEvent.click(screen.getByText('Legajo 1 (3)'))
    await fireEvent.click(screen.getByRole('button', { name: 'Analizar selección' }))

    await screen.findByText('Iniciar lote')
    expect(mockInvoke).toHaveBeenCalledWith(
      'processing_prepare',
      expect.objectContaining({ collectionIds: ['c1'] })
    )
  })

  it('starts the draft and opens its detail', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'processing_list_batches') return { batches: [], nextCursor: null }
      if (command === 'processing_prepare') return { batchId: 'b-draft', created: true, members: 4 }
      if (command === 'processing_get_batch') return draftSnapshot()
      if (command === 'processing_start') return runningSnapshot()
      if (command === 'processing_list_tasks') return { tasks: [failedTask()], nextCursor: null }
      return undefined
    })
    render(BatchProcessingTab)

    await screen.findByText('Legajo 1 (3)')
    await fireEvent.click(screen.getByText('Legajo 1 (3)'))
    await fireEvent.click(screen.getByRole('button', { name: 'Analizar selección' }))
    await fireEvent.click(await screen.findByText('Iniciar lote'))

    expect(mockInvoke).toHaveBeenCalledWith(
      'processing_start',
      expect.objectContaining({ batchId: 'b-draft' })
    )
    await screen.findByText('Detalle del lote')
  })

  it('pauses a running batch with its revision', async () => {
    const running = runningSnapshot()
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'processing_list_batches') {
        return {
          batches: [
            {
              id: 'b-run',
              state: 'running',
              desiredState: 'run',
              operations: ['ocr'],
              revision: 2,
              createdAt: 1,
              updatedAt: 2,
              activeUnits: 1,
              failedUnits: 1,
              succeededUnits: 2,
            },
          ],
          nextCursor: null,
        }
      }
      if (command === 'processing_get_batch') return running
      if (command === 'processing_list_tasks') return { tasks: [failedTask()], nextCursor: null }
      if (command === 'processing_control') return { affected: 1, snapshot: running }
      return undefined
    })
    render(BatchProcessingTab)

    const pause = await screen.findByRole('button', { name: 'Pausar' })
    await fireEvent.click(pause)

    expect(mockInvoke).toHaveBeenCalledWith(
      'processing_control',
      expect.objectContaining({
        request: expect.objectContaining({
          action: 'pause',
          batchId: 'b-run',
          expectedRevision: 2,
        }),
      })
    )
  })

  it('retries one failed unit without touching the rest', async () => {
    const running = runningSnapshot()
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'processing_list_batches') return { batches: [], nextCursor: null }
      if (command === 'processing_prepare') return { batchId: 'b-draft', created: true, members: 4 }
      if (command === 'processing_get_batch')
        return running.id === 'b-run' ? running : draftSnapshot()
      if (command === 'processing_start') return running
      if (command === 'processing_list_tasks') return { tasks: [failedTask()], nextCursor: null }
      if (command === 'processing_retry') return { reopened: 1, operationId: 'req-x' }
      return undefined
    })
    render(BatchProcessingTab)

    await screen.findByText('Legajo 1 (3)')
    await fireEvent.click(screen.getByText('Legajo 1 (3)'))
    await fireEvent.click(screen.getByRole('button', { name: 'Analizar selección' }))
    await fireEvent.click(await screen.findByText('Iniciar lote'))
    await fireEvent.click(await screen.findByText('Reintentar'))

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        'processing_retry',
        expect.objectContaining({ batchId: 'b-run', taskId: 'ocr-a9' })
      )
    })
  })
})
