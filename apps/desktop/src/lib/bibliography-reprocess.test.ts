import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { UnlistenFn } from '@tauri-apps/api/event'
import {
  bibliographyReprocessCandidates,
  bibliographyReprocessConfirm,
  bibliographyReprocessPreview,
  bibliographyReprocessPreviewCancel,
  formatEstimatedUsd,
  isPdfAttachment,
  onBibliographyReprocessPreviewProgress,
  previewUnitPercent,
  REPROCESS_PREVIEW_PROGRESS_EVENT,
  type ReprocessPreviewProgress,
} from './bibliography-reprocess'

// Mocks are set up in test-setup.ts:
//   @tauri-apps/api/core → invoke vi.fn()
//   @tauri-apps/api/event → listen vi.fn() returning Promise<vi.fn()>
const { invoke } = await import('@tauri-apps/api/core')
const { listen } = await import('@tauri-apps/api/event')

const mockInvoke = vi.mocked(invoke)
const mockListen = vi.mocked(listen)

describe('bibliography reprocess wrappers', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockInvoke.mockResolvedValue(undefined)
    mockListen.mockResolvedValue(vi.fn() as unknown as UnlistenFn)
  })

  it('reads the candidate list through the read-only command', async () => {
    const candidates = [
      {
        attachmentId: 'att-1',
        itemId: 'item-1',
        title: 'El oficio de historiador',
        filename: 'oficio.pdf',
        reasons: ['garbled_stored_pages'],
        flaggedPages: 3,
      },
    ]
    mockInvoke.mockResolvedValueOnce(candidates)

    await expect(bibliographyReprocessCandidates()).resolves.toEqual(candidates)
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_reprocess_candidates')
  })

  it('previews exactly the given attachments by id', async () => {
    const preview = { attachments: [], totals: {}, cancelled: false }
    mockInvoke.mockResolvedValueOnce(preview)

    await expect(bibliographyReprocessPreview(['att-1', 'att-2'])).resolves.toEqual(preview)
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_reprocess_preview', {
      attachmentIds: ['att-1', 'att-2'],
    })
  })

  it('cancels the running preview', async () => {
    await bibliographyReprocessPreviewCancel()
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_reprocess_preview_cancel')
  })

  it('confirms entries as exactly {attachmentId, planHash}', async () => {
    const answer = {
      batchId: 'batch-1',
      results: [{ attachmentId: 'att-1', status: 'queued' }],
    }
    mockInvoke.mockResolvedValueOnce(answer)

    await expect(
      bibliographyReprocessConfirm([{ attachmentId: 'att-1', planHash: 'hash-1' }])
    ).resolves.toEqual(answer)
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_reprocess_confirm', {
      entries: [{ attachmentId: 'att-1', planHash: 'hash-1' }],
    })
  })

  it('subscribes to the preview progress event and passes its payload on', async () => {
    const handler = vi.fn()
    const unlisten = vi.fn() as unknown as UnlistenFn
    mockListen.mockResolvedValueOnce(unlisten)

    const dispose = await onBibliographyReprocessPreviewProgress(handler)

    expect(mockListen).toHaveBeenCalledWith(REPROCESS_PREVIEW_PROGRESS_EVENT, expect.any(Function))
    const listener = mockListen.mock.calls[0]![1] as (event: {
      payload: ReprocessPreviewProgress
    }) => void
    listener({ payload: { done: 2, total: 5, unitsDone: 7, unitsTotal: 21 } })
    expect(handler).toHaveBeenCalledWith({ done: 2, total: 5, unitsDone: 7, unitsTotal: 21 })
    expect(dispose).toBe(unlisten)
  })
})

describe('previewUnitPercent', () => {
  it('floors the share of the attachment being read', () => {
    expect(previewUnitPercent({ done: 1, total: 1, unitsDone: 37, unitsTotal: 100 })).toBe(37)
    expect(previewUnitPercent({ done: 1, total: 1, unitsDone: 38, unitsTotal: 103 })).toBe(36)
    expect(previewUnitPercent({ done: 1, total: 1, unitsDone: 7, unitsTotal: 7 })).toBe(100)
  })

  it('clamps to 0..100 and reads 0 while the unit total is unknown', () => {
    expect(previewUnitPercent({ done: 1, total: 1, unitsDone: 9, unitsTotal: 7 })).toBe(100)
    expect(previewUnitPercent({ done: 1, total: 1, unitsDone: -2, unitsTotal: 10 })).toBe(0)
    expect(previewUnitPercent({ done: 0, total: 3, unitsDone: 0, unitsTotal: 0 })).toBe(0)
  })
})

describe('formatEstimatedUsd', () => {
  it('formats the estimate with the locale decimal separator', () => {
    expect(formatEstimatedUsd(0.24, 'es')).toBe('0,24')
    expect(formatEstimatedUsd(0.24, 'en')).toBe('0.24')
    expect(formatEstimatedUsd(1.5, 'es')).toBe('1,50')
  })

  it('keeps small estimates distinguishable from zero', () => {
    expect(formatEstimatedUsd(0, 'es')).toBe('0,00')
    // 82 pages × 3000 tokens × 0.03 / 1M: the real per-page cost is tiny.
    expect(formatEstimatedUsd(0.00738, 'es')).toBe('0,0074')
  })
})

describe('isPdfAttachment', () => {
  it('matches the backend rule: pdf content type or .pdf filename', () => {
    expect(isPdfAttachment('application/pdf', null)).toBe(true)
    expect(isPdfAttachment('APPLICATION/PDF', 'x')).toBe(true)
    expect(isPdfAttachment(null, 'Oficio.PDF')).toBe(true)
    expect(isPdfAttachment('text/html', 'snap.html')).toBe(false)
    expect(isPdfAttachment(null, null)).toBe(false)
  })
})
