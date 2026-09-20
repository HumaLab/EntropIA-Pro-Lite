import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
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

function taskDetail() {
  const { requestState: _state, dependencyTaskId: _dependency, ...summary } = failedTask()
  return {
    ...summary,
    checkpoints: [],
    attempts: [
      {
        attemptNumber: 3,
        leaseEpoch: 1,
        startedAt: 1_000,
        finishedAt: 2_500,
        outcome: 'failed',
        retryable: true,
        errorCode: 'corrupt_pdf',
        errorMessage: null,
      },
    ],
    sharedWithBatches: ['b-run'],
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
      if (command === 'processing_get_task') return taskDetail()
      if (command === 'processing_retry') return { reopened: 1, operationId: 'req-x' }
      return undefined
    })
    render(BatchProcessingTab)

    await screen.findByText('Legajo 1')
    await fireEvent.click(screen.getByText('Legajo 1'))
    await fireEvent.click(screen.getByRole('button', { name: 'Analizar selección' }))
    await fireEvent.click(await screen.findByText('Iniciar lote'))
    // Retry lives inside the unit's own detail now: the message that explains
    // why it failed is right there, and the table stays four short columns
    // wide so two or three of them can stand side by side.
    await fireEvent.click(await screen.findByRole('button', { name: 'Ver intentos de ocr · a9' }))
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

describe('the batch detail is compact and dark all the way down', () => {
  function succeededTask() {
    return {
      ...failedTask(),
      taskId: 'ocr-a1',
      assetId: 'a1',
      state: 'succeeded',
      progressDone: 1,
      attemptCount: 1,
      errorCode: null,
      errorMessage: null,
    }
  }

  async function openDetail() {
    const result = render(BatchProcessingTab)
    await fireEvent.click(await screen.findByText('b-run'))
    await screen.findByText('Detalle del lote')
    return result
  }

  beforeEach(() => {
    const running = runningSnapshot()
    mockInvoke.mockImplementation(async (command: string, ...rest: unknown[]) => {
      if (command === 'processing_list_batches') {
        const args = rest[0] as Record<string, unknown> | undefined
        const states = args?.['states']
        const wantsActive = Array.isArray(states) && states.includes('running')
        if (!wantsActive) return { batches: [], nextCursor: null }
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
      if (command === 'processing_list_tasks') {
        return { tasks: [succeededTask(), failedTask()], nextCursor: null }
      }
      if (command === 'processing_get_task') return taskDetail()
      return undefined
    })
  })

  it('opens the state filter as the app menu, never a native select', async () => {
    // The native popup is drawn by the operating system — white surface, blue
    // focus ring — and no stylesheet in this app can reach it.
    const { container } = await openDetail()

    expect(container.querySelector('select')).toBeNull()

    const trigger = screen.getByRole('button', { name: 'Estado Todos' })
    expect(trigger).toHaveAttribute('aria-haspopup', 'menu')

    await fireEvent.click(trigger)

    const options = screen.getAllByRole('menuitemradio')
    expect(options[0]).toHaveAccessibleName('Todos')
    expect(options[0]).toHaveAttribute('aria-checked', 'true')
  })

  it('reloads the tasks through the chosen state', async () => {
    await openDetail()
    await fireEvent.click(screen.getByRole('button', { name: 'Estado Todos' }))

    await fireEvent.click(screen.getByRole('menuitemradio', { name: 'Fallidos' }))

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        'processing_list_tasks',
        expect.objectContaining({ batchId: 'b-run', state: 'failed' })
      )
    })
    expect(screen.getByRole('button', { name: 'Estado Fallidos' })).toBeInTheDocument()
  })

  it('gives the tasks a column each instead of a running sentence', async () => {
    await openDetail()

    const table = screen.getByRole('table', { name: 'Tareas del lote' })
    const headers = within(table)
      .getAllByRole('columnheader')
      .map((cell) => cell.textContent?.trim())

    // No Mensaje column: it is empty on every row that succeeded, which is
    // nearly all of them, and a permanently blank column is exactly the width
    // that stops a second table fitting beside this one.
    expect(headers).toEqual(['Operación', 'Estado', 'Progreso', 'Intentos'])
  })

  it('sums the batch into one row of stats', async () => {
    await openDetail()

    // runningSnapshot counts 1 pending, 1 failed and 2 succeeded: settled is
    // everything that will not move again, so pending is the only one left.
    for (const [value, label] of [
      ['3', 'Resueltos'],
      ['1', 'Pendientes'],
      ['1', 'Errores'],
      ['4', 'Total'],
    ] as const) {
      expect(screen.getByText(label).closest('span')).toHaveTextContent(`${value} ${label}`)
    }
  })

  it('offers no chevron on a unit that worked the first time', async () => {
    await openDetail()

    // succeededTask() is one attempt, no retry cycle, no error: opening it
    // would only repeat Estado, Progreso and Intentos back.
    expect(
      screen.queryByRole('button', { name: 'Ver intentos de ocr · a1' })
    ).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Ver intentos de ocr · a9' })).toBeInTheDocument()
  })

  it('says how an attempt ended in Spanish, and how long it took', async () => {
    await openDetail()

    await fireEvent.click(screen.getByRole('button', { name: 'Ver intentos de ocr · a9' }))

    // The duration is the one thing about an attempt the row cannot show —
    // and `failed` was reaching a Spanish interface untranslated.
    const attempt = await screen.findByText(/#3/)
    expect(attempt).toHaveTextContent('Fallido')
    expect(attempt).toHaveTextContent('1.5 s')
    expect(screen.queryByText(/failed/)).not.toBeInTheDocument()
  })

  it('reaches the error message through the row rather than a blank column', async () => {
    await openDetail()

    // Hovering the state says what went wrong without a column standing empty
    // on every other row…
    const rows = screen.getAllByRole('row')
    const failed = rows.find((row) => within(row).queryByText('Fallidos'))
    expect(within(failed!).getByText('Fallidos')).toHaveAttribute(
      'data-tooltip',
      'encrypted and locked'
    )

    // …and the chevron opens the whole of it.
    await fireEvent.click(screen.getByRole('button', { name: 'Ver intentos de ocr · a9' }))

    expect(await screen.findByText('encrypted and locked')).toBeInTheDocument()
  })
})

describe('the unit list is paged, not grown', () => {
  const PAGE = 48

  function pagedInvoke(total: number) {
    return async (command: string, ...rest: unknown[]) => {
      if (command === 'processing_list_batches') {
        const args = rest[0] as Record<string, unknown> | undefined
        const states = args?.['states']
        const wantsActive = Array.isArray(states) && states.includes('running')
        if (!wantsActive) return { batches: [], nextCursor: null }
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
              activeUnits: 0,
              failedUnits: 0,
              succeededUnits: total,
            },
          ],
          nextCursor: null,
        }
      }
      if (command === 'processing_get_batch') {
        return {
          ...runningSnapshot(),
          tasksByState: [
            { name: 'succeeded', count: total - 10 },
            { name: 'pending', count: 10 },
          ],
        }
      }
      if (command === 'processing_list_tasks') {
        const args = rest[0] as Record<string, unknown> | undefined
        const offset = Number(args?.['offset'] ?? 0)
        const rows = Math.max(0, Math.min(PAGE, total - offset))
        return {
          tasks: Array.from({ length: rows }, (_, index) => ({
            ...failedTask(),
            taskId: `t-${offset + index}`,
            state: 'succeeded',
            errorMessage: null,
          })),
          nextCursor: null,
        }
      }
      return undefined
    }
  }

  async function openPaged(total: number) {
    mockInvoke.mockImplementation(pagedInvoke(total))
    render(BatchProcessingTab)
    await fireEvent.click(await screen.findByText('b-run'))
    await screen.findByText('Detalle del lote')
  }

  it('counts the whole result set from the snapshot, not the loaded page', async () => {
    // 482 units, 48 to a page: the total can never come from `tasks.length`,
    // and no extra query is needed for it either.
    await openPaged(482)

    expect(screen.getByText('482 resultados')).toBeInTheDocument()
    expect(screen.getByText('1–48 de 482')).toBeInTheDocument()
  })

  it('asks the backend for the page it jumped to', async () => {
    await openPaged(482)

    await fireEvent.click(screen.getByRole('button', { name: 'Última página' }))

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        'processing_list_tasks',
        expect.objectContaining({ offset: 10 * 48, limit: 48 })
      )
    })
    expect(screen.getByText('481–482 de 482')).toBeInTheDocument()
  })

  it('disables the way back on the first page and the way on at the end', async () => {
    await openPaged(482)

    expect(screen.getByRole('button', { name: 'Primera página' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Página anterior' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Página siguiente' })).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Última página' })).toBeEnabled()

    await fireEvent.click(screen.getByRole('button', { name: 'Última página' }))

    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Página siguiente' })).toBeDisabled()
    )
    expect(screen.getByRole('button', { name: 'Última página' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Primera página' })).toBeEnabled()
  })

  it('returns to the first page when the filter changes under an advanced one', async () => {
    await openPaged(482)
    await fireEvent.click(screen.getByRole('button', { name: 'Última página' }))
    await waitFor(() => expect(screen.getByText('481–482 de 482')).toBeInTheDocument())

    await fireEvent.click(screen.getByRole('button', { name: 'Estado Todos' }))
    await fireEvent.click(screen.getByRole('menuitemradio', { name: 'Pendientes' }))

    // Page 11 of an unfiltered list is past the end of a 10-unit one; staying
    // there would show an empty table with no way to tell why.
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith(
        'processing_list_tasks',
        expect.objectContaining({ state: 'pending', offset: 0 })
      )
    })
    expect(screen.getByText('10 resultados')).toBeInTheDocument()
  })

  it('leaves nowhere to page to when a single page holds everything', async () => {
    await openPaged(12)

    expect(screen.getByRole('button', { name: 'Página siguiente' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Última página' })).toBeDisabled()
    expect(screen.getByText('1–12 de 12')).toBeInTheDocument()
  })

  it('says a filter matched nothing instead of showing an empty table', async () => {
    await openPaged(482)

    await fireEvent.click(screen.getByRole('button', { name: 'Estado Todos' }))
    await fireEvent.click(screen.getByRole('menuitemradio', { name: 'Fallidos' }))

    expect(await screen.findByText('Sin resultados para este filtro')).toBeInTheDocument()
    expect(screen.getByText('0 resultados')).toBeInTheDocument()
    expect(screen.queryByRole('table', { name: 'Tareas del lote' })).not.toBeInTheDocument()
  })
})

describe('the composer keeps its operations on one strip', () => {
  const HINT = 'Genera embeddings cuando el OCR produzca texto.'

  it('holds the label, both toggles and the action in one labelled group', async () => {
    render(BatchProcessingTab)

    const operations = await screen.findByRole('group', { name: 'Operaciones' })

    expect(within(operations).getByRole('checkbox', { name: 'OCR' })).toBeInTheDocument()
    expect(within(operations).getByRole('checkbox', { name: 'Embeddings' })).toBeInTheDocument()
    expect(within(operations).getByRole('button', { name: 'Analizar selección' })).toBeVisible()
  })

  it('carries the embeddings hint on the toggle instead of a line under it', async () => {
    // The hint as its own paragraph is what a single-line strip cannot afford,
    // and dropping it outright would take the explanation with it.
    render(BatchProcessingTab)

    const embeddings = await screen.findByRole('checkbox', { name: 'Embeddings' })

    expect(embeddings.closest('[data-tooltip]')).toHaveAttribute('data-tooltip', HINT)
    expect(screen.queryByText(HINT)).not.toBeInTheDocument()
  })
})

describe('the history reads as a table', () => {
  const LONG_ID = 'b-0123456789abcdef0123456789abcdef'

  function completedBatch() {
    return {
      id: LONG_ID,
      state: 'completed',
      desiredState: 'run',
      operations: ['ocr'],
      revision: 4,
      createdAt: Date.UTC(2026, 0, 15, 12),
      updatedAt: Date.UTC(2026, 0, 15, 12),
      activeUnits: 0,
      failedUnits: 2,
      succeededUnits: 6,
    }
  }

  beforeEach(() => {
    mockInvoke.mockImplementation(async (command: string, ...rest: unknown[]) => {
      if (command !== 'processing_list_batches') return undefined
      const args = rest[0] as Record<string, unknown> | undefined
      const states = args?.['states']
      const wantsHistory = Array.isArray(states) && states.includes('completed')
      if (!wantsHistory) return { batches: [], nextCursor: null }
      return { batches: [completedBatch()], nextCursor: null }
    })
  })

  it('gives every metric its own column instead of one running sentence', async () => {
    render(BatchProcessingTab)

    const table = await screen.findByRole('table', { name: 'Historial de lotes' })
    const headers = within(table)
      .getAllByRole('columnheader')
      .map((cell) => cell.textContent?.trim())

    expect(headers).toEqual(['Lote', 'Estado', 'Fecha', 'OCR', 'Emb.', 'Total', 'Errores'])
  })

  it('names the batch state in the interface language, not the backend one', async () => {
    render(BatchProcessingTab)

    expect(await screen.findByText('Completado')).toBeInTheDocument()
    expect(screen.queryByText('completed')).not.toBeInTheDocument()
  })

  it('counts the units per column and marks which operations ran', async () => {
    render(BatchProcessingTab)
    await screen.findByRole('table', { name: 'Historial de lotes' })

    const cells = screen.getAllByRole('cell')

    // Total is summed from the unit counts: the summary carries no such field.
    expect(cells[3]).toHaveTextContent('Incluida')
    expect(cells[4]).toHaveTextContent('No incluida')
    expect(cells[5]).toHaveTextContent('8')
    expect(cells[6]).toHaveTextContent('2')
  })

  it('truncates a long id but keeps the whole of it reachable', async () => {
    render(BatchProcessingTab)

    const open = await screen.findByRole('button', { name: LONG_ID })

    expect(open).toHaveAttribute('data-tooltip', LONG_ID)
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

  it('drops the two panels to one column when their row cannot hold both', () => {
    const panels = ruleFor('.batch-tab__panels {')

    // A viewport media query reads a width these panels never get: the
    // Configuración sidebar sits beside them. `min(100%, …)` is what keeps the
    // track floor from exceeding the track once there is only one column.
    expect(panels).toMatch(
      /grid-template-columns:\s*repeat\(auto-fit,\s*minmax\(min\(100%,\s*\d+px\),\s*1fr\)\)/
    )
    // Without this an empty "Sin trabajo pendiente" panel is stretched to the
    // height of a long history — the reserved emptiness this redesign removed.
    expect(panels).toMatch(/align-items:\s*start/)
  })

  it('sends the analyze action to the end of its line without leaving the flow', () => {
    // `position: absolute` or a float would put it over the label it follows
    // the moment the strip wraps; an auto start margin cannot overlap anything.
    expect(ruleFor('.batch-ops :global(.batch-ops__action) {')).toMatch(
      /margin-inline-start:\s*auto/
    )
    expect(ruleFor('.batch-ops {')).toMatch(/flex-wrap:\s*wrap/)
  })

  it('gives a long batch id every declaration an ellipsis needs', () => {
    const open = ruleFor('.batch-table__open {')

    expect([
      /overflow:\s*hidden/.test(open),
      /text-overflow:\s*ellipsis/.test(open),
      /white-space:\s*nowrap/.test(open),
    ]).toEqual([true, true, true])
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
