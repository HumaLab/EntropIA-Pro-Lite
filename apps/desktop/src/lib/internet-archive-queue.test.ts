import { describe, expect, it, vi } from 'vitest'
import {
  MAX_BATCH_FILES,
  MAX_BATCH_RECORDS,
  buildBatch,
  createQueue,
  finishBatch,
  parseQueue,
  queueProgress,
  type IaQueue,
} from './internet-archive-queue'

const records = (count: number, prefix = 'id') =>
  Array.from({ length: count }, (_, index) => ({
    identifier: `${prefix}-${index + 1}`,
    title: `Title ${index + 1}`,
  }))

const queueOf = (count: number): IaQueue =>
  createQueue({
    collectionId: 'col-1',
    label: 'anarquismo',
    kinds: ['pdf'],
    records: records(count),
  })

describe('createQueue / parseQueue', () => {
  it('starts every record pending and round-trips through JSON', () => {
    const queue = queueOf(3)
    expect(queue.records.map((record) => record.state)).toEqual(['pending', 'pending', 'pending'])
    expect(queue.currentJobId).toBeNull()
    expect(parseQueue(JSON.stringify(queue))).toEqual(queue)
  })

  it('drops repeated identifiers, keeping the first', () => {
    const queue = createQueue({
      collectionId: 'col-1',
      label: 'x',
      kinds: ['pdf'],
      records: [...records(2), { identifier: 'id-1', title: 'again' }],
    })
    expect(queue.records.map((record) => record.identifier)).toEqual(['id-1', 'id-2'])
  })

  it('reads anything that is not a queue as no queue', () => {
    expect(parseQueue(null)).toBeNull()
    expect(parseQueue('{}')).toBeNull()
    expect(parseQueue('{"version":2,"records":[]}')).toBeNull()
    expect(parseQueue('not json')).toBeNull()
  })
})

describe('queueProgress', () => {
  it('counts finished records (done, empty, failed) against the total', () => {
    const queue = queueOf(5)
    queue.records[0]!.state = 'done'
    queue.records[1]!.state = 'empty'
    queue.records[2]!.state = 'failed'
    queue.records[3]!.state = 'batched'
    expect(queueProgress(queue)).toEqual({ total: 5, finished: 3, pending: 2 })
  })
})

describe('buildBatch', () => {
  it('takes pending records in order up to the record limit', async () => {
    const queue = queueOf(MAX_BATCH_RECORDS + 5)
    const { queue: next, selections } = await buildBatch(queue, async (id) => [`${id}.pdf`])
    expect(selections).toHaveLength(MAX_BATCH_RECORDS)
    expect(selections[0]).toEqual({ identifier: 'id-1', fileNames: ['id-1.pdf'] })
    expect(next.records.filter((record) => record.state === 'batched')).toHaveLength(
      MAX_BATCH_RECORDS
    )
    expect(next.records.at(-1)!.state).toBe('pending')
    // The input queue is never mutated.
    expect(queue.records.every((record) => record.state === 'pending')).toBe(true)
  })

  it('stops before a record would push the batch past the file limit', async () => {
    const queue = queueOf(3)
    const sizes: Record<string, number> = { 'id-1': 600, 'id-2': 500, 'id-3': 10 }
    const { queue: next, selections } = await buildBatch(queue, async (id) =>
      Array.from({ length: sizes[id]! }, (_, index) => `${id}-${index}.pdf`)
    )
    expect(selections.map((group) => group.identifier)).toEqual(['id-1'])
    expect(next.records.map((record) => record.state)).toEqual(['batched', 'pending', 'pending'])
  })

  it('keeps the first files of a record larger than one batch', async () => {
    const queue = queueOf(1)
    const { selections } = await buildBatch(queue, async () =>
      Array.from({ length: MAX_BATCH_FILES + 20 }, (_, index) => `f-${index}.pdf`)
    )
    expect(selections[0]!.fileNames).toHaveLength(MAX_BATCH_FILES)
  })

  it('marks records without files as empty and failed lookups as failed', async () => {
    const queue = queueOf(3)
    const { queue: next, selections } = await buildBatch(queue, async (id) => {
      if (id === 'id-1') return []
      if (id === 'id-2') throw new Error('network')
      return [`${id}.pdf`]
    })
    expect(next.records.map((record) => record.state)).toEqual(['empty', 'failed', 'batched'])
    expect(selections).toEqual([{ identifier: 'id-3', fileNames: ['id-3.pdf'] }])
  })

  it('skips records that are not pending and looks up a few at a time', async () => {
    const queue = queueOf(10)
    queue.records[0]!.state = 'done'
    let inFlight = 0
    let peak = 0
    const resolve = vi.fn(async (id: string) => {
      inFlight += 1
      peak = Math.max(peak, inFlight)
      await new Promise((done) => setTimeout(done, 1))
      inFlight -= 1
      return [`${id}.pdf`]
    })
    const { selections } = await buildBatch(queue, resolve)
    expect(resolve).not.toHaveBeenCalledWith('id-1')
    expect(selections).toHaveLength(9)
    expect(peak).toBeGreaterThan(1)
    expect(peak).toBeLessThanOrEqual(4)
  })

  it('reports how many records of the batch it has checked', async () => {
    const queue = queueOf(10)
    const progress: Array<[number, number]> = []
    await buildBatch(
      queue,
      async (id) => [`${id}.pdf`],
      (checked, total) => progress.push([checked, total])
    )
    expect(progress).toEqual([
      [0, 10],
      [4, 10],
      [8, 10],
      [10, 10],
    ])
  })

  it('counts the batch total against the record limit', async () => {
    const queue = queueOf(MAX_BATCH_RECORDS + 50)
    const totals = new Set<number>()
    await buildBatch(
      queue,
      async (id) => [`${id}.pdf`],
      (_checked, total) => totals.add(total)
    )
    expect([...totals]).toEqual([MAX_BATCH_RECORDS])
  })

  it('grows the batch total for records without files instead of showing it full early', async () => {
    const queue = queueOf(MAX_BATCH_RECORDS + 5)
    const empty = new Set(['id-1', 'id-2', 'id-3'])
    const progress: Array<[number, number]> = []
    const { selections } = await buildBatch(
      queue,
      async (id) => (empty.has(id) ? [] : [`${id}.pdf`]),
      (checked, total) => progress.push([checked, total])
    )
    expect(selections).toHaveLength(MAX_BATCH_RECORDS)
    expect(progress.at(-1)).toEqual([MAX_BATCH_RECORDS + 3, MAX_BATCH_RECORDS + 3])
    expect(progress.slice(0, -1).every(([checked, total]) => checked < total)).toBe(true)
  })

  it('reports the batch as fully checked when the file limit ends it', async () => {
    const queue = queueOf(3)
    const sizes: Record<string, number> = { 'id-1': 600, 'id-2': 500, 'id-3': 10 }
    const progress: Array<[number, number]> = []
    await buildBatch(
      queue,
      async (id) => Array.from({ length: sizes[id]! }, (_, index) => `${id}-${index}.pdf`),
      (checked, total) => progress.push([checked, total])
    )
    expect(progress).toEqual([
      [0, 3],
      [1, 1],
    ])
  })

  it('returns no selections once nothing is pending', async () => {
    const queue = queueOf(1)
    queue.records[0]!.state = 'done'
    const { selections } = await buildBatch(queue, async () => ['x.pdf'])
    expect(selections).toEqual([])
  })
})

describe('finishBatch', () => {
  it('marks the batch done and forgets its job', () => {
    const queue = { ...queueOf(2), currentJobId: 'job-1' }
    queue.records[0]!.state = 'batched'
    const next = finishBatch(queue, 'done')
    expect(next.records.map((record) => record.state)).toEqual(['done', 'pending'])
    expect(next.currentJobId).toBeNull()
  })

  it('puts a cancelled batch back as pending', () => {
    const queue = { ...queueOf(2), currentJobId: 'job-1' }
    queue.records[0]!.state = 'batched'
    queue.records[1]!.state = 'done'
    const next = finishBatch(queue, 'requeue')
    expect(next.records.map((record) => record.state)).toEqual(['pending', 'done'])
    expect(next.currentJobId).toBeNull()
  })
})
