import { describe, expect, it } from 'vitest'
import { batchProgress, isTerminalBatchState, type BatchSnapshot } from './batch-processing'

function snapshot(states: Array<[string, number]>): BatchSnapshot {
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
    expect(progress.ratio).toBeCloseTo(0.75)
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
