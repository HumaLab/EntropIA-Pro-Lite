/**
 * Typed client and global snapshot store for the durable batch queue.
 *
 * The backend owns all queue state (SQLite); this module only reads
 * snapshots and sends control intents. `processing:changed` events are
 * invalidation hints — after every event (and on a bounded poll while work
 * is active) the store re-reads the snapshot, reconciles by batch revision,
 * and drops stale responses. Losing events therefore degrades to polling,
 * never to a wrong progress bar.
 */

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getProcessingInitState, type ProcessingInitSummary } from './db'

// ── DTOs (mirror the serde camelCase contracts in processing/commands.rs) ──

export interface BatchPrepareResponse {
  batchId: string
  created: boolean
  members: number
}

export interface StateCount {
  name: string
  count: number
}

export interface BatchCollectionRef {
  id: string
  name: string
}

export interface BatchSnapshot {
  id: string
  requestId: string
  origin: string
  state: string
  desiredState: string
  operations: string[]
  planningCursor: number
  planningDone: boolean
  revision: number
  createdAt: number
  updatedAt: number
  startedAt: number | null
  finishedAt: number | null
  lastError: string | null
  membersTotal: number
  membersClassified: number
  tasksByState: StateCount[]
  tasksByKind: StateCount[]
  collections: BatchCollectionRef[]
}

export interface BatchSummary {
  id: string
  state: string
  desiredState: string
  operations: string[]
  revision: number
  createdAt: number
  updatedAt: number
  activeUnits: number
  failedUnits: number
  succeededUnits: number
}

export interface BatchCursor {
  createdAt: number
  id: string
}

export interface BatchTaskSummary {
  taskId: string
  kind: string
  assetId: string
  state: string
  stage: string
  progressDone: number
  progressTotal: number
  outcome: string
  attemptCount: number
  retryCycle: number
  nextRetryAt: number | null
  errorCode: string | null
  errorMessage: string | null
  updatedAt: number
  requestState: string
  dependencyTaskId: string | null
}

export interface TaskAttempt {
  attemptNumber: number
  leaseEpoch: number
  startedAt: number
  finishedAt: number | null
  outcome: string
  retryable: boolean
  errorCode: string | null
  errorMessage: string | null
}

export interface TaskCheckpoint {
  unitKey: string
  checksum: string
  createdAt: number
}

export interface BatchTaskDetail extends Omit<
  BatchTaskSummary,
  'requestState' | 'dependencyTaskId'
> {
  checkpoints: TaskCheckpoint[]
  attempts: TaskAttempt[]
  sharedWithBatches: string[]
}

export type BatchControlAction = 'pause' | 'resume' | 'cancel'

// ── Command wrappers ────────────────────────────────────────────────────────

export function processingPrepare(
  requestId: string,
  collectionIds: string[],
  operations: string[]
): Promise<BatchPrepareResponse> {
  return invoke<BatchPrepareResponse>('processing_prepare', {
    requestId,
    collectionIds,
    operations,
  })
}

export function processingStart(
  batchId: string,
  expectedRevision?: number
): Promise<BatchSnapshot> {
  return invoke<BatchSnapshot>('processing_start', {
    batchId,
    expectedRevision: expectedRevision ?? null,
  })
}

export function processingControl(
  action: BatchControlAction,
  batchId?: string,
  expectedRevision?: number
): Promise<{ affected: number; snapshot: BatchSnapshot | null }> {
  return invoke('processing_control', {
    request: {
      batchId: batchId ?? null,
      action,
      expectedRevision: expectedRevision ?? null,
    },
  })
}

export function processingRetry(
  requestId: string,
  batchId: string,
  taskId?: string,
  failedOnly = false
): Promise<{ reopened: number; operationId: string }> {
  return invoke('processing_retry', {
    requestId,
    batchId,
    taskId: taskId ?? null,
    failedOnly,
  })
}

export function processingListBatches(
  options: {
    states?: string[]
    cursorCreatedAt?: number
    cursorId?: string
    limit?: number
  } = {}
): Promise<{ batches: BatchSummary[]; nextCursor: BatchCursor | null }> {
  return invoke('processing_list_batches', {
    states: options.states ?? null,
    cursorCreatedAt: options.cursorCreatedAt ?? null,
    cursorId: options.cursorId ?? null,
    limit: options.limit ?? 50,
  })
}

export function processingGetBatch(batchId: string): Promise<BatchSnapshot> {
  return invoke<BatchSnapshot>('processing_get_batch', { batchId })
}

export function processingListTasks(options: {
  batchId: string
  state?: string
  kind?: string
  afterTaskId?: string
  limit?: number
}): Promise<{ tasks: BatchTaskSummary[]; nextCursor: string | null }> {
  return invoke('processing_list_tasks', {
    batchId: options.batchId,
    state: options.state ?? null,
    kind: options.kind ?? null,
    afterTaskId: options.afterTaskId ?? null,
    limit: options.limit ?? 50,
  })
}

export function processingGetTask(
  batchId: string,
  taskId: string,
  attemptLimit = 20
): Promise<BatchTaskDetail> {
  return invoke<BatchTaskDetail>('processing_get_task', { batchId, taskId, attemptLimit })
}

export function newBatchRequestId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) return crypto.randomUUID()
  return `req-${Date.now()}-${Math.floor(Math.random() * 1_000_000)}`
}

// ── Progress math (mirrors plan-lote.md §10.2) ─────────────────────────────

const SETTLED_STATES: Record<string, true> = {
  succeeded: true,
  skipped: true,
  failed: true,
  cancelled: true,
}

export function batchProgress(snapshot: BatchSnapshot): {
  total: number
  settled: number
  succeeded: number
  failed: number
  ratio: number | null
} {
  const byState = new Map(snapshot.tasksByState.map((entry) => [entry.name, entry.count]))
  const total = [...byState.values()].reduce((sum, count) => sum + count, 0)
  if (total === 0) return { total: 0, settled: 0, succeeded: 0, failed: 0, ratio: null }
  let settled = 0
  for (const [state, count] of byState) {
    if (SETTLED_STATES[state]) settled += count
  }
  const succeeded = byState.get('succeeded') ?? 0
  const failed = byState.get('failed') ?? 0
  return { total, settled, succeeded, failed, ratio: settled / total }
}

// ── Global store (survives navigation like SyncStore) ───────────────────────

export interface BatchGlobalSummary {
  init: ProcessingInitSummary | null
  initError: string | null
  active: BatchSummary[]
  recoveredBatches: number
}

export interface BatchFocusRequest {
  batchId: string | null
  nonce: number
}

type BatchSubscriber = (summary: BatchGlobalSummary) => void

const EMPTY_SUMMARY: BatchGlobalSummary = {
  init: null,
  initError: null,
  active: [],
  recoveredBatches: 0,
}

class BatchStore {
  private _summary: BatchGlobalSummary = { ...EMPTY_SUMMARY }
  private readonly _subscribers = new Set<BatchSubscriber>()
  private _unlisten: UnlistenFn | null = null
  private _bootstrap: Promise<void> | null = null
  private _pollTimer: ReturnType<typeof setInterval> | null = null
  private _revision = 0
  private _focus: BatchFocusRequest = { batchId: null, nonce: 0 }
  private readonly _focusSubscribers = new Set<(focus: BatchFocusRequest) => void>()

  subscribe(run: BatchSubscriber): () => void {
    this._subscribers.add(run)
    run(this.snapshot())
    return () => {
      this._subscribers.delete(run)
    }
  }

  snapshot(): BatchGlobalSummary {
    return { ...this._summary, active: [...this._summary.active] }
  }

  private emit(): void {
    const snapshot = this.snapshot()
    this._subscribers.forEach((run) => run(snapshot))
  }

  /**
   * Idempotent bootstrap: subscribes to `processing:changed` BEFORE the
   * first snapshot read (no missed transitions in between), then starts
   * bounded polling while work is active. Coalesces bursts: at most one
   * refresh in flight, events during a refresh schedule exactly one more.
   */
  initialize(
    listenFn: (
      event: string,
      callback: (event: { payload: unknown }) => void
    ) => Promise<UnlistenFn> = listen
  ): Promise<void> {
    if (this._bootstrap) return this._bootstrap
    this._bootstrap = (async () => {
      this._unlisten = await listenFn('processing:changed', () => {
        void this.refresh()
      })
      await this.refresh()
    })()
    return this._bootstrap
  }

  destroy(): void {
    this._unlisten?.()
    this._unlisten = null
    this._bootstrap = null
    if (this._pollTimer) {
      clearInterval(this._pollTimer)
      this._pollTimer = null
    }
  }

  private _refreshInFlight: Promise<void> | null = null
  private _refreshQueued = false

  async refresh(): Promise<void> {
    if (this._refreshInFlight) {
      this._refreshQueued = true
      return
    }
    this._refreshInFlight = this.doRefresh().finally(() => {
      this._refreshInFlight = null
      if (this._refreshQueued) {
        this._refreshQueued = false
        void this.refresh()
      }
    })
    return this._refreshInFlight
  }

  private async doRefresh(): Promise<void> {
    const revision = ++this._revision
    try {
      const { summary, error } = getProcessingInitState()
      const { batches } = await processingListBatches({ limit: 50 })
      if (revision !== this._revision) return
      const active = batches.filter((batch) => !isTerminalBatchState(batch.state))
      const recoveredBatches =
        summary?.recovered != null ? summary.pendingBatches : this._summary.recoveredBatches
      this._summary = { init: summary, initError: error, active, recoveredBatches }
      this.armPolling(active.length > 0)
    } catch (error) {
      if (revision !== this._revision) return
      this._summary = {
        ...this._summary,
        initError: error instanceof Error ? error.message : String(error),
      }
      this.armPolling(this._summary.active.length > 0)
    }
    this.emit()
  }

  private armPolling(hasWork: boolean): void {
    if (hasWork && !this._pollTimer) {
      this._pollTimer = setInterval(() => {
        void this.refresh()
      }, 3000)
    } else if (!hasWork && this._pollTimer) {
      clearInterval(this._pollTimer)
      this._pollTimer = null
    }
  }

  /** Asks the settings tab to open a batch once navigation lands there. */
  requestFocus(batchId: string | null): void {
    this._focus = { batchId, nonce: this._focus.nonce + 1 }
    this._focusSubscribers.forEach((run) => run({ ...this._focus }))
  }

  subscribeFocus(run: (focus: BatchFocusRequest) => void): () => void {
    this._focusSubscribers.add(run)
    return () => {
      this._focusSubscribers.delete(run)
    }
  }
}

export function isTerminalBatchState(state: string): boolean {
  return state === 'completed' || state === 'completed_with_errors' || state === 'cancelled'
}

export const batchStore = new BatchStore()
