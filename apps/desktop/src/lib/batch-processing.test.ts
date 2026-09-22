import { describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import {
  batchProgress,
  batchStore,
  processingListActiveBatches,
  isTerminalBatchState,
  type BatchSnapshot,
  type BatchSummary,
  type BatchTaskSummary,
} from './batch-processing'

function snapshot(
  states: Array<[string, number]>,
  aggregate: { done?: number; total?: number | null; unknownTasks?: number } = {}
): BatchSnapshot {
  const taskTotal = states.reduce((sum, [, count]) => sum + count, 0)
  return {
    id: 'b1',
    requestId: 'req-1',
    origin: 'user',
    state: 'running',
    desiredState: 'run',
    operations: ['ocr'],
    planningCursor: 0,
    planningDone: true,
    revision: 1,
    createdAt: 1,
    updatedAt: 2,
    startedAt: 1,
    finishedAt: null,
    lastError: null,
    membersTotal: 0,
    membersClassified: 0,
    progressDone: aggregate.done ?? 0,
    progressTotal: aggregate.total ?? null,
    progressUnknownTasks: aggregate.unknownTasks ?? taskTotal,
    priority: 0,
    tasksByState: states.map(([name, count]) => ({ name, count })),
    tasksByKind: [],
    collections: [],
  }
}

describe('batchProgress', () => {
  it('reports no total instead of dividing by zero', () => {
    expect(batchProgress(snapshot([]))).toEqual({
      total: 0,
      settled: 0,
      succeeded: 0,
      failed: 0,
      unitsDone: 0,
      unitsTotal: null,
      unknownUnitTasks: 0,
      settledRatio: null,
      unitRatio: null,
      basis: null,
      ratio: null,
    })
  })

  it('counts settled separately from successes', () => {
    const progress = batchProgress(
      snapshot([
        ['pending', 1],
        ['running', 1],
        ['succeeded', 2],
        ['failed', 3],
        ['cancelled', 1],
      ])
    )
    expect(progress.total).toBe(8)
    expect(progress.settled).toBe(6)
    expect(progress.succeeded).toBe(2)
    expect(progress.failed).toBe(3)
    expect(progress.settledRatio).toBeCloseTo(0.75)
    expect(progress.basis).toBe('tasks')
    expect(progress.ratio).toBeCloseTo(0.75)
  })

  it('uses durable page/chunk units when every linked task declares a total', () => {
    const progress = batchProgress(
      snapshot(
        [
          ['running', 1],
          ['succeeded', 1],
        ],
        { done: 7, total: 10, unknownTasks: 0 }
      )
    )

    expect(progress.unitsDone).toBe(7)
    expect(progress.unitsTotal).toBe(10)
    expect(progress.unitRatio).toBeCloseTo(0.7)
    expect(progress.settledRatio).toBeCloseTo(0.5)
    expect(progress.basis).toBe('units')
    expect(progress.ratio).toBeCloseTo(0.7)
  })

  it('falls back to settled tasks when any linked task has an unknown total', () => {
    const progress = batchProgress(
      snapshot(
        [
          ['running', 1],
          ['succeeded', 1],
        ],
        { done: 7, total: 10, unknownTasks: 1 }
      )
    )

    expect(progress.unitsDone).toBe(7)
    expect(progress.unitsTotal).toBe(10)
    expect(progress.unknownUnitTasks).toBe(1)
    expect(progress.unitRatio).toBeNull()
    expect(progress.basis).toBe('tasks')
    expect(progress.ratio).toBeCloseTo(0.5)
  })

  it('never treats bibliography item totals as page or chunk totals', () => {
    const bibliography = snapshot([['running', 1]], {
      done: 1,
      total: 100,
      unknownTasks: 0,
    })
    bibliography.tasksByKind = [{ name: 'bibliography_sync', count: 1 }]

    const progress = batchProgress(bibliography)

    expect(progress.unitsDone).toBe(1)
    expect(progress.unitsTotal).toBe(100)
    expect(progress.unknownUnitTasks).toBe(1)
    expect(progress.unitRatio).toBeNull()
    expect(progress.basis).toBe('tasks')
    expect(progress.ratio).toBe(0)
  })

  it('never presents failures as success', () => {
    const progress = batchProgress(snapshot([['failed', 2]]))
    expect(progress.ratio).toBe(1)
    expect(progress.succeeded).toBe(0)
  })
})

describe('isTerminalBatchState', () => {
  it('matches the backend terminal states', () => {
    expect(isTerminalBatchState('completed')).toBe(true)
    expect(isTerminalBatchState('completed_with_errors')).toBe(true)
    expect(isTerminalBatchState('cancelled')).toBe(true)
    expect(isTerminalBatchState('running')).toBe(false)
    expect(isTerminalBatchState('paused')).toBe(false)
  })
})

describe('durable batch navigation', () => {
  it('delivers focus to a settings view mounted after navigation', () => {
    batchStore.requestFocus('old-paused-batch')
    const received: Array<string | null> = []
    const unsubscribe = batchStore.subscribeFocus((focus) => received.push(focus.batchId))
    expect(received).toEqual(['old-paused-batch'])
    unsubscribe()
  })

  it('keeps an older active batch beyond the first page of results', async () => {
    const rows = Array.from(
      { length: 51 },
      (_, index) =>
        ({
          id: `b-${index}`,
          state: 'paused',
          desiredState: 'pause',
          operations: ['ocr'],
          revision: 0,
          priority: 0,
          createdAt: 100 - index,
          updatedAt: 1,
          activeUnits: 1,
          failedUnits: 0,
          succeededUnits: 0,
        }) satisfies BatchSummary
    )
    vi.mocked(invoke).mockImplementation(async (_command, args) => {
      const request = args as { states: string[]; cursorId: string | null }
      if (request.states.includes('completed'))
        throw new Error('history must not consume active pages')
      return request.cursorId
        ? { batches: rows.slice(50), nextCursor: null }
        : { batches: rows.slice(0, 50), nextCursor: { createdAt: 51, id: 'b-49' } }
    })
    const active = await processingListActiveBatches()
    expect(active.map((row) => row.id)).toEqual(rows.map((row) => row.id))
    vi.mocked(invoke).mockReset()
  })
})

describe('e2a-3 additive subject identity', () => {
  it('carries corpus/asset identity alongside assetId with old fixtures unaffected', () => {
    // New additive fields are accepted where tasks flow; documentary rows
    // mirror assetId in subjectId. No behavior/rendering change — this only
    // pins the type contract.
    const withSubject: BatchTaskSummary = {
      taskId: 't1',
      kind: 'ocr',
      assetId: 'a1',
      domain: 'corpus',
      subjectKind: 'asset',
      subjectId: 'a1',
      state: 'pending',
      stage: '',
      progressDone: 0,
      progressTotal: 0,
      outcome: '',
      attemptCount: 0,
      retryCycle: 0,
      nextRetryAt: null,
      errorCode: null,
      errorMessage: null,
      updatedAt: 1,
      requestState: 'active',
      dependencyTaskId: null,
    }
    expect(withSubject.domain).toBe('corpus')
    expect(withSubject.subjectId).toBe(withSubject.assetId)
    // Old fixtures without the new fields still compile and still drive
    // the unchanged progress math.
    const legacy: BatchTaskSummary = {
      taskId: 't0',
      kind: 'ocr',
      assetId: 'a0',
      state: 'pending',
      stage: '',
      progressDone: 0,
      progressTotal: 0,
      outcome: '',
      attemptCount: 0,
      retryCycle: 0,
      nextRetryAt: null,
      errorCode: null,
      errorMessage: null,
      updatedAt: 1,
      requestState: 'active',
      dependencyTaskId: null,
    }
    expect(legacy.domain).toBeUndefined()
    expect(batchProgress).toBeDefined()
  })
})
