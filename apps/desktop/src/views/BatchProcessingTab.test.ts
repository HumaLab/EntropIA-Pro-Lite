import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
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
  vi.useRealTimers()
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

    await screen.findByText('Legajo 1')
    await fireEvent.click(screen.getByText('Legajo 1'))
    await fireEvent.click(screen.getByRole('button', { name: 'Analizar selección' }))

    expect(await screen.findByRole('button', { name: 'Iniciar lote' })).toBeDisabled()
  })

  it('keeps watching the draft until background planning finishes', async () => {
    // processing_prepare returns as soon as the batch row exists; classifying
    // its members and flipping planning_done happens on the supervisor thread
    // afterwards, and no event announces it. The tab has to keep looking.
    vi.useFakeTimers()
    // At mount there is no work at all, so nothing is polling yet — that is
    // precisely the state the draft has to wake up from.
    let prepared = false
    let planningDone = false
    mockInvoke.mockImplementation(async (command: string, ...rest: unknown[]) => {
      const args = rest[0] as Record<string, unknown> | undefined
      if (command === 'processing_list_batches') {
        const states = args?.['states']
        const wantsActive = Array.isArray(states) && states.includes('preparing')
        if (!wantsActive || !prepared) return { batches: [], nextCursor: null }
        return {
          batches: [
            {
              id: 'b-draft',
              state: planningDone ? 'ready' : 'preparing',
              desiredState: 'pause',
              operations: ['ocr'],
              revision: 0,
              createdAt: 1,
              updatedAt: 1,
              activeUnits: 0,
              failedUnits: 0,
              succeededUnits: 0,
            },
          ],
          nextCursor: null,
        }
      }
      if (command === 'processing_prepare') {
        prepared = true
        return { batchId: 'b-draft', created: true, members: 4 }
      }
      if (command === 'processing_get_batch') {
        return planningDone
          ? {
              ...draftSnapshot(),
              state: 'ready',
              planningDone: true,
              planningCursor: 4,
              membersClassified: 4,
            }
          : draftSnapshot()
      }
      return undefined
    })
    render(BatchProcessingTab)

    await screen.findByText('Legajo 1')
    await fireEvent.click(screen.getByText('Legajo 1'))
    await fireEvent.click(screen.getByRole('button', { name: 'Analizar selección' }))
    expect(await screen.findByRole('button', { name: 'Iniciar lote' })).toBeDisabled()

    planningDone = true
    await vi.advanceTimersByTimeAsync(3000)

    await waitFor(() => expect(screen.getByRole('button', { name: 'Iniciar lote' })).toBeEnabled())
    vi.useRealTimers()
  })

  it('starts the draft and opens its detail', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'processing_list_batches') return { batches: [], nextCursor: null }
      if (command === 'processing_prepare') return { batchId: 'b-draft', created: true, members: 4 }
      if (command === 'processing_get_batch')
        return { ...draftSnapshot(), state: 'ready', planningDone: true }
      if (command === 'processing_start') return runningSnapshot()
      if (command === 'processing_list_tasks') return { tasks: [failedTask()], nextCursor: null }
      return undefined
    })
    render(BatchProcessingTab)

    await screen.findByText('Legajo 1')
    await fireEvent.click(screen.getByText('Legajo 1'))
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

    await screen.findByText('Legajo 1')
    await fireEvent.click(screen.getByText('Legajo 1'))
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

  it('does not offer pause or resume while cancellation is converging', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'processing_list_batches') {
        return {
          batches: [
            {
              id: 'b-cancelling',
              state: 'cancelling',
              desiredState: 'cancel',
              operations: ['ocr'],
              revision: 3,
              createdAt: 1,
              updatedAt: 3,
              activeUnits: 1,
              failedUnits: 0,
              succeededUnits: 0,
            },
          ],
          nextCursor: null,
        }
      }
      return undefined
    })
    render(BatchProcessingTab)

    await screen.findAllByText('b-cancelling')
    expect(screen.queryByRole('button', { name: 'Pausar' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Reanudar' })).not.toBeInTheDocument()
  })

  it('offers to start a prepared draft that outlived its panel', async () => {
    // The draft panel is component state: reload the app, or leave the tab, and
    // a batch prepared but never started is only reachable through the active
    // list. Without a resume there, its only exit is cancellation.
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'processing_list_batches') {
        return {
          batches: [
            {
              id: 'b-ready',
              state: 'ready',
              desiredState: 'pause',
              operations: ['ocr'],
              revision: 0,
              createdAt: 1,
              updatedAt: 2,
              activeUnits: 0,
              failedUnits: 0,
              succeededUnits: 0,
            },
          ],
          nextCursor: null,
        }
      }
      return undefined
    })
    render(BatchProcessingTab)

    await screen.findAllByText('b-ready')
    await fireEvent.click(screen.getByRole('button', { name: 'Reanudar' }))

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        'processing_control',
        expect.objectContaining({
          request: expect.objectContaining({ action: 'resume', batchId: 'b-ready' }),
        })
      )
    })
  })
})

/**
 * jsdom performs no layout, so the two ways this picker breaks silently are the
 * two it cannot see: a track rule that stops reflowing, and a name that stops
 * truncating. Both are asserted against the stylesheet itself.
 */
describe('the collection picker reflows and truncates', () => {
  // Comments are stripped: the rules below are explained in prose that names
  // the very declarations under test, and a check that reads its own
  // documentation is a check that proves nothing.
  const STYLES = readFileSync(
    resolve(import.meta.dirname, 'BatchProcessingTab.svelte'),
    'utf-8'
  ).replace(/\/\*[\s\S]*?\*\//g, '')

  function ruleFor(selector: string): string {
    const at = STYLES.indexOf(selector)
    expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
    const rule = STYLES.slice(at)
    return rule.slice(0, rule.indexOf('}'))
  }

  it('sizes its columns from the space available, not from a column count', () => {
    const list = ruleFor('.batch-field__scope-list {')

    // A fixed count — repeat(4, 1fr) — renders fine and stops reflowing, which
    // is exactly the regression no rendering test would catch.
    expect(list).toMatch(
      /grid-template-columns:\s*repeat\(auto-(fit|fill),\s*minmax\(\d+px,\s*1fr\)\)/
    )
  })

  it('gives a long collection name every declaration an ellipsis needs', () => {
    const name = ruleFor('.batch-field__scope-name {')

    // min-width is the one that gets dropped as redundant and is not: without
    // it a flex item refuses to shrink below its content, so the text never
    // overflows its box, the ellipsis never appears, and the card widens.
    expect([
      /min-width:\s*0/.test(name),
      /overflow:\s*hidden/.test(name),
      /text-overflow:\s*ellipsis/.test(name),
      /white-space:\s*nowrap/.test(name),
    ]).toEqual([true, true, true, true])
  })
})
