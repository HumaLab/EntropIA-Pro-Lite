/**
 * "Import every result" queue: the records of a whole search or listing,
 * imported in batches that each fit one import job. Stored through
 * `iaQueueSave` so an unfinished queue resumes when the importer opens again.
 */

import type { IaFileKind, IaSelection } from './internet-archive'

/** One import job takes at most this many files (`MAX_PREPARE_FILES`). */
export const MAX_BATCH_FILES = 1000
/** One import job takes at most this many records (`MAX_PREPARE_SELECTIONS`). */
export const MAX_BATCH_RECORDS = 100

export type IaQueueRecordState = 'pending' | 'batched' | 'done' | 'empty' | 'failed'

export type IaQueueRecord = {
  identifier: string
  title: string | null
  state: IaQueueRecordState
}

export type IaQueue = {
  version: 1
  collectionId: string
  /** What was searched or listed, shown when offering to resume. */
  label: string
  kinds: IaFileKind[]
  records: IaQueueRecord[]
  /** Job holding the `batched` records, once prepared. */
  currentJobId: string | null
}

/** Records looked up at once while building a batch. */
const LOOKUP_CONCURRENCY = 4
const RECORD_STATES = new Set<IaQueueRecordState>(['pending', 'batched', 'done', 'empty', 'failed'])

export function createQueue(input: {
  collectionId: string
  label: string
  kinds: IaFileKind[]
  records: ReadonlyArray<{ identifier: string; title: string | null }>
}): IaQueue {
  const seen = new Set<string>()
  const records: IaQueueRecord[] = []
  for (const record of input.records) {
    if (seen.has(record.identifier)) continue
    seen.add(record.identifier)
    records.push({ identifier: record.identifier, title: record.title, state: 'pending' })
  }
  return {
    version: 1,
    collectionId: input.collectionId,
    label: input.label,
    kinds: [...input.kinds],
    records,
    currentJobId: null,
  }
}

/** The stored queue, or `null` for anything that is not a version 1 queue. */
export function parseQueue(content: string | null): IaQueue | null {
  if (content == null) return null
  let value: unknown
  try {
    value = JSON.parse(content)
  } catch {
    return null
  }
  if (typeof value !== 'object' || value === null) return null
  const queue = value as Partial<IaQueue>
  if (
    queue.version !== 1 ||
    typeof queue.collectionId !== 'string' ||
    typeof queue.label !== 'string' ||
    !Array.isArray(queue.kinds) ||
    !Array.isArray(queue.records) ||
    !(queue.currentJobId === null || typeof queue.currentJobId === 'string')
  ) {
    return null
  }
  const valid = queue.records.every(
    (record) =>
      typeof record === 'object' &&
      record !== null &&
      typeof record.identifier === 'string' &&
      (record.title === null || typeof record.title === 'string') &&
      RECORD_STATES.has(record.state)
  )
  return valid ? (queue as IaQueue) : null
}

/** Finished records (done, empty, failed) and those still to import. */
export function queueProgress(queue: IaQueue): {
  total: number
  finished: number
  pending: number
} {
  const total = queue.records.length
  const pending = queue.records.filter(
    (record) => record.state === 'pending' || record.state === 'batched'
  ).length
  return { total, finished: total - pending, pending }
}

/**
 * Next batch: pending records in order, each with the file names
 * `resolveFiles` returns, until the next record would pass
 * `MAX_BATCH_FILES` or `MAX_BATCH_RECORDS`. A record with no files is
 * `empty`, one whose lookup throws is `failed`; a single record with more
 * files than a batch keeps its first `MAX_BATCH_FILES`. Returns a new queue.
 * `onProgress` hears the records checked so far against the batch's
 * expected size (pending records, at most `MAX_BATCH_RECORDS`).
 */
export async function buildBatch(
  queue: IaQueue,
  resolveFiles: (identifier: string) => Promise<string[]>,
  onProgress?: (checked: number, total: number) => void
): Promise<{ queue: IaQueue; selections: IaSelection[] }> {
  const records = queue.records.map((record) => ({ ...record }))
  const pending = records.filter((record) => record.state === 'pending')
  const selections: IaSelection[] = []
  const total = Math.min(pending.length, MAX_BATCH_RECORDS)
  let files = 0
  let next = 0
  if (total > 0) onProgress?.(0, total)
  batch: while (next < pending.length && selections.length < MAX_BATCH_RECORDS) {
    const chunk = pending.slice(next, next + LOOKUP_CONCURRENCY)
    const found = await Promise.allSettled(chunk.map((record) => resolveFiles(record.identifier)))
    for (const [offset, record] of chunk.entries()) {
      const result = found[offset]!
      if (result.status === 'rejected') {
        record.state = 'failed'
      } else if (result.value.length === 0) {
        record.state = 'empty'
      } else {
        const names = result.value.slice(0, MAX_BATCH_FILES)
        if (selections.length >= MAX_BATCH_RECORDS) break batch
        if (selections.length > 0 && files + names.length > MAX_BATCH_FILES) break batch
        record.state = 'batched'
        selections.push({ identifier: record.identifier, fileNames: names })
        files += names.length
      }
      next += 1
    }
    onProgress?.(Math.min(next, total), total)
  }
  return { queue: { ...queue, records }, selections }
}

/** Close the current batch: `done` keeps it finished, `requeue` (a
 *  cancelled job) puts its records back as pending. */
export function finishBatch(queue: IaQueue, how: 'done' | 'requeue'): IaQueue {
  const state: IaQueueRecordState = how === 'done' ? 'done' : 'pending'
  return {
    ...queue,
    currentJobId: null,
    records: queue.records.map((record) =>
      record.state === 'batched' ? { ...record, state } : record
    ),
  }
}
