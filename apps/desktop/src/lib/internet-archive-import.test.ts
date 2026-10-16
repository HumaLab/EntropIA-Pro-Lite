/**
 * The Internet Archive import orchestrator: verified files go through
 * `importClassifiedPathsIntoCollection` with the payload Rust handed over
 * (or are confirmed as already imported, or have their partial remnant
 * discarded), then each file is confirmed or reported as failed. One file's
 * failure never stops the others; cancel is checked between files.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { IaReconcileEntry, IaVerifiedFile } from './internet-archive'
import {
  IaImportError,
  importVerifiedFiles,
  resolveReconcileLeftovers,
  type IaImportProgress,
} from './internet-archive-import'

const { iaRef, importRef } = vi.hoisted(() => ({
  iaRef: {
    iaGetImportStatus: vi.fn(),
    iaListVerifiedFiles: vi.fn(),
    iaBeginFileImport: vi.fn(),
    iaConfirmFileImported: vi.fn(),
    iaFailFileImport: vi.fn(),
    iaListReconcile: vi.fn(),
    iaFinishReconcile: vi.fn(),
  },
  importRef: {
    importClassifiedPathsIntoCollection: vi.fn(),
    discardFailedImport: vi.fn(),
  },
}))

vi.mock('./internet-archive', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./internet-archive')>()
  return {
    ...actual,
    iaGetImportStatus: iaRef.iaGetImportStatus,
    iaListVerifiedFiles: iaRef.iaListVerifiedFiles,
    iaBeginFileImport: iaRef.iaBeginFileImport,
    iaConfirmFileImported: iaRef.iaConfirmFileImported,
    iaFailFileImport: iaRef.iaFailFileImport,
    iaListReconcile: iaRef.iaListReconcile,
    iaFinishReconcile: iaRef.iaFinishReconcile,
  }
})

vi.mock('./collection-import', () => ({
  importClassifiedPathsIntoCollection: importRef.importClassifiedPathsIntoCollection,
  discardFailedImport: importRef.discardFailedImport,
}))

const EXTRA = { __entropia_ia: { identifier: 'some-id' } }

const file = (index: number, patch: Partial<IaVerifiedFile> = {}): IaVerifiedFile => ({
  index,
  identifier: 'some-id',
  fileName: `file-${index}.pdf`,
  displayFileName: `File ${index}.pdf`,
  itemTitle: `Title ${index}`,
  stagingPath: `/staging/file-${index}.pdf`,
  pageCount: 10,
  sha256: 'a'.repeat(64),
  extraMetadata: EXTRA,
  ...patch,
})

const status = (targetCollectionId: string | null = 'col-1') => ({
  id: 'job-1',
  state: 'ready' as const,
  targetCollectionId,
  files: [],
  running: false,
})

const imported = (id: string, title: string) => ({
  classifiedCount: 1,
  rejected: [],
  createdItems: [{ id, title }],
  importErrors: [],
  alreadyImported: [],
})

beforeEach(() => {
  vi.clearAllMocks()
  iaRef.iaGetImportStatus.mockResolvedValue(status())
  iaRef.iaListVerifiedFiles.mockResolvedValue([])
  iaRef.iaBeginFileImport.mockResolvedValue({ action: 'import', file: file(0) })
  iaRef.iaConfirmFileImported.mockResolvedValue(undefined)
  iaRef.iaFailFileImport.mockResolvedValue(undefined)
  iaRef.iaListReconcile.mockResolvedValue([])
  iaRef.iaFinishReconcile.mockResolvedValue(undefined)
  importRef.importClassifiedPathsIntoCollection.mockResolvedValue(imported('item-1', 'Title 0'))
  importRef.discardFailedImport.mockResolvedValue(undefined)
})

describe('importVerifiedFiles', () => {
  it('imports the begin payload with the exact overrides and confirms the created item', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0)])

    const outcome = await importVerifiedFiles('job-1')

    expect(iaRef.iaGetImportStatus).toHaveBeenCalledWith('job-1')
    expect(iaRef.iaBeginFileImport).toHaveBeenCalledWith('job-1', 0)
    expect(importRef.importClassifiedPathsIntoCollection).toHaveBeenCalledWith(
      ['/staging/file-0.pdf'],
      'col-1',
      expect.objectContaining({
        overrides: {
          title: 'Title 0',
          fileName: 'File 0.pdf',
          extraMetadata: EXTRA,
          allowDuplicate: true,
        },
      })
    )
    expect(iaRef.iaConfirmFileImported).toHaveBeenCalledWith('job-1', 0, 'item-1')
    expect(iaRef.iaFailFileImport).not.toHaveBeenCalled()
    expect(outcome).toEqual({
      imported: [{ index: 0, itemId: 'item-1' }],
      confirmedExisting: [],
      failed: [],
      notProcessed: [],
      cancelled: false,
    })
  })

  it('falls back to the listed file when begin carries no payload', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(3)])
    iaRef.iaBeginFileImport.mockResolvedValue({ action: 'import' })

    await importVerifiedFiles('job-1')

    const [paths, , options] = importRef.importClassifiedPathsIntoCollection.mock.calls[0]!
    expect(paths).toEqual(['/staging/file-3.pdf'])
    expect(options.overrides.title).toBe('Title 3')
    expect(iaRef.iaConfirmFileImported).toHaveBeenCalledWith('job-1', 3, 'item-1')
  })

  it('confirms an existing item without importing anything', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0)])
    iaRef.iaBeginFileImport.mockResolvedValue({ action: 'confirm_existing', itemId: 'item-7' })

    const outcome = await importVerifiedFiles('job-1')

    expect(importRef.importClassifiedPathsIntoCollection).not.toHaveBeenCalled()
    expect(importRef.discardFailedImport).not.toHaveBeenCalled()
    expect(iaRef.iaConfirmFileImported).toHaveBeenCalledWith('job-1', 0, 'item-7')
    expect(iaRef.iaFailFileImport).not.toHaveBeenCalled()
    expect(outcome.confirmedExisting).toEqual([{ index: 0, itemId: 'item-7' }])
    expect(outcome.failed).toEqual([])
  })

  it('discards the partial remnant and begins again before importing', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0)])
    iaRef.iaBeginFileImport
      .mockResolvedValueOnce({ action: 'discard_partial', itemId: 'item-old' })
      .mockResolvedValueOnce({ action: 'import', file: file(0) })

    const outcome = await importVerifiedFiles('job-1')

    expect(importRef.discardFailedImport).toHaveBeenCalledWith('col-1', 'item-old')
    expect(iaRef.iaBeginFileImport).toHaveBeenCalledTimes(2)
    expect(iaRef.iaBeginFileImport).toHaveBeenNthCalledWith(1, 'job-1', 0)
    expect(iaRef.iaBeginFileImport).toHaveBeenNthCalledWith(2, 'job-1', 0)
    expect(importRef.importClassifiedPathsIntoCollection).toHaveBeenCalledTimes(1)
    expect(iaRef.iaConfirmFileImported).toHaveBeenCalledWith('job-1', 0, 'item-1')
    expect(outcome.imported).toEqual([{ index: 0, itemId: 'item-1' }])
  })

  it('records partial_remains when the remnant is still there after discarding once', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0)])
    iaRef.iaBeginFileImport.mockResolvedValue({ action: 'discard_partial', itemId: 'item-old' })

    const outcome = await importVerifiedFiles('job-1')

    expect(importRef.discardFailedImport).toHaveBeenCalledTimes(1)
    expect(importRef.discardFailedImport).toHaveBeenCalledWith('col-1', 'item-old')
    expect(iaRef.iaBeginFileImport).toHaveBeenCalledTimes(2)
    expect(importRef.importClassifiedPathsIntoCollection).not.toHaveBeenCalled()
    expect(iaRef.iaFailFileImport).not.toHaveBeenCalled()
    expect(outcome.failed).toEqual([{ index: 0, code: 'partial_remains' }])
  })

  it('reports the failure and still imports the other files when the importer rejects', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0), file(1)])
    iaRef.iaBeginFileImport.mockImplementation(async (_job: string, index: number) => ({
      action: 'import',
      file: file(index),
    }))
    importRef.importClassifiedPathsIntoCollection
      .mockRejectedValueOnce(new Error('io: disk full'))
      .mockResolvedValueOnce(imported('item-1', 'Title 1'))

    const outcome = await importVerifiedFiles('job-1')

    expect(importRef.importClassifiedPathsIntoCollection).toHaveBeenCalledTimes(2)
    expect(iaRef.iaFailFileImport).toHaveBeenCalledWith('job-1', 0)
    expect(iaRef.iaFailFileImport).toHaveBeenCalledTimes(1)
    expect(iaRef.iaConfirmFileImported).toHaveBeenCalledWith('job-1', 1, 'item-1')
    expect(outcome.failed).toEqual([{ index: 0, code: 'io' }])
    expect(outcome.imported).toEqual([{ index: 1, itemId: 'item-1' }])
  })

  it('discards every created item when the importer resolves without exactly one item', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0), file(1)])
    iaRef.iaBeginFileImport.mockImplementation(async (_job: string, index: number) => ({
      action: 'import',
      file: file(index),
    }))
    importRef.importClassifiedPathsIntoCollection
      .mockResolvedValueOnce({
        classifiedCount: 1,
        rejected: [],
        createdItems: [
          { id: 'item-a', title: 'Title 0' },
          { id: 'item-b', title: 'Title 0' },
        ],
        importErrors: [],
        alreadyImported: [],
      })
      .mockResolvedValueOnce(imported('item-1', 'Title 1'))

    const outcome = await importVerifiedFiles('job-1')

    expect(importRef.discardFailedImport).toHaveBeenCalledWith('col-1', 'item-a')
    expect(importRef.discardFailedImport).toHaveBeenCalledWith('col-1', 'item-b')
    expect(iaRef.iaFailFileImport).toHaveBeenCalledWith('job-1', 0)
    expect(iaRef.iaConfirmFileImported).toHaveBeenCalledWith('job-1', 1, 'item-1')
    expect(outcome.failed).toEqual([{ index: 0, code: 'unknown' }])
    expect(outcome.imported).toEqual([{ index: 1, itemId: 'item-1' }])
  })

  it('reports the failure when the importer resolves with no item', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0), file(1)])
    iaRef.iaBeginFileImport.mockImplementation(async (_job: string, index: number) => ({
      action: 'import',
      file: file(index),
    }))
    importRef.importClassifiedPathsIntoCollection
      .mockResolvedValueOnce({
        classifiedCount: 1,
        rejected: ['/staging/file-0.pdf'],
        createdItems: [],
        importErrors: [],
        alreadyImported: [],
      })
      .mockResolvedValueOnce(imported('item-1', 'Title 1'))

    const outcome = await importVerifiedFiles('job-1')

    expect(iaRef.iaFailFileImport).toHaveBeenCalledWith('job-1', 0)
    expect(iaRef.iaConfirmFileImported).toHaveBeenCalledWith('job-1', 1, 'item-1')
    expect(outcome.failed).toEqual([{ index: 0, code: 'unknown' }])
    expect(outcome.imported).toEqual([{ index: 1, itemId: 'item-1' }])
  })

  it('logs the collected import errors when the importer resolves with no item', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0)])
    importRef.importClassifiedPathsIntoCollection.mockResolvedValueOnce({
      classifiedCount: 1,
      rejected: [],
      createdItems: [],
      importErrors: ['Import (importing File 0.pdf): forbidden path', 'second cause'],
      alreadyImported: [],
    })
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})

    try {
      const outcome = await importVerifiedFiles('job-1')

      expect(iaRef.iaFailFileImport).toHaveBeenCalledWith('job-1', 0)
      expect(outcome.failed).toEqual([{ index: 0, code: 'unknown' }])
      expect(warn).toHaveBeenCalledWith(
        '[internet-archive] the import failed:',
        'Import (importing File 0.pdf): forbidden path; second cause'
      )
    } finally {
      warn.mockRestore()
    }
  })

  it('records the parsed code when confirm rejects, without reporting a failure', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0)])
    iaRef.iaConfirmFileImported.mockRejectedValue(new Error('incomplete_import: pages missing'))

    const outcome = await importVerifiedFiles('job-1')

    expect(importRef.discardFailedImport).toHaveBeenCalledWith('col-1', 'item-1')
    expect(iaRef.iaFailFileImport).not.toHaveBeenCalled()
    expect(outcome.failed).toEqual([{ index: 0, code: 'incomplete_import' }])
    expect(outcome.imported).toEqual([])
  })

  it('keeps the confirm code and continues when discarding after a refused confirm fails', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0), file(1)])
    iaRef.iaBeginFileImport.mockImplementation(async (_job: string, index: number) => ({
      action: 'import',
      file: file(index),
    }))
    iaRef.iaConfirmFileImported.mockRejectedValueOnce(new Error('incomplete_import: pages missing'))
    importRef.discardFailedImport.mockRejectedValueOnce(new Error('io: cannot discard'))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})

    try {
      const outcome = await importVerifiedFiles('job-1')

      expect(iaRef.iaFailFileImport).not.toHaveBeenCalled()
      expect(outcome.failed).toEqual([{ index: 0, code: 'incomplete_import' }])
      expect(outcome.imported).toEqual([{ index: 1, itemId: 'item-1' }])
      expect(warn).toHaveBeenCalled()
    } finally {
      warn.mockRestore()
    }
  })

  it('still reports the failure and continues when discarding after a bad import fails', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0), file(1)])
    iaRef.iaBeginFileImport.mockImplementation(async (_job: string, index: number) => ({
      action: 'import',
      file: file(index),
    }))
    importRef.importClassifiedPathsIntoCollection
      .mockResolvedValueOnce({
        classifiedCount: 1,
        rejected: [],
        createdItems: [
          { id: 'item-a', title: 'Title 0' },
          { id: 'item-b', title: 'Title 0' },
        ],
        importErrors: [],
        alreadyImported: [],
      })
      .mockResolvedValueOnce(imported('item-1', 'Title 1'))
    importRef.discardFailedImport.mockRejectedValue(new Error('io: cannot discard'))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})

    try {
      const outcome = await importVerifiedFiles('job-1')

      expect(iaRef.iaFailFileImport).toHaveBeenCalledWith('job-1', 0)
      expect(outcome.failed).toEqual([{ index: 0, code: 'unknown' }])
      expect(outcome.imported).toEqual([{ index: 1, itemId: 'item-1' }])
      expect(warn).toHaveBeenCalled()
    } finally {
      warn.mockRestore()
    }
  })

  it('records partial_remains without beginning again when the partial discard fails', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0), file(1)])
    iaRef.iaBeginFileImport.mockImplementation(async (_job: string, index: number) =>
      index === 0
        ? { action: 'discard_partial', itemId: 'item-old' }
        : { action: 'import', file: file(index) }
    )
    importRef.discardFailedImport.mockRejectedValueOnce(new Error('io: cannot discard'))
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})

    try {
      const outcome = await importVerifiedFiles('job-1')

      expect(iaRef.iaBeginFileImport).toHaveBeenCalledTimes(2)
      expect(iaRef.iaBeginFileImport).toHaveBeenNthCalledWith(1, 'job-1', 0)
      expect(iaRef.iaBeginFileImport).toHaveBeenNthCalledWith(2, 'job-1', 1)
      expect(importRef.importClassifiedPathsIntoCollection).toHaveBeenCalledTimes(1)
      expect(iaRef.iaFailFileImport).not.toHaveBeenCalled()
      expect(outcome.failed).toEqual([{ index: 0, code: 'partial_remains' }])
      expect(outcome.imported).toEqual([{ index: 1, itemId: 'item-1' }])
      expect(warn).toHaveBeenCalled()
    } finally {
      warn.mockRestore()
    }
  })

  it('records the parsed code when begin rejects and continues with the rest', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0), file(1)])
    iaRef.iaBeginFileImport
      .mockRejectedValueOnce(new Error('staging_missing: staging file gone'))
      .mockResolvedValueOnce({ action: 'import', file: file(1) })

    const outcome = await importVerifiedFiles('job-1')

    expect(outcome.failed).toEqual([{ index: 0, code: 'staging_missing' }])
    expect(outcome.imported).toEqual([{ index: 1, itemId: 'item-1' }])
    expect(iaRef.iaFailFileImport).not.toHaveBeenCalled()
  })

  it('stops before the next file when asked to cancel, leaving the rest unprocessed', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0), file(1), file(2)])
    iaRef.iaBeginFileImport.mockImplementation(async (_job: string, index: number) => ({
      action: 'import',
      file: file(index),
    }))
    const seen = [false, true, true]
    const shouldCancel = vi.fn(() => seen.shift() ?? true)

    const outcome = await importVerifiedFiles('job-1', { shouldCancel })

    expect(shouldCancel).toHaveBeenCalled()
    expect(iaRef.iaBeginFileImport).toHaveBeenCalledTimes(1)
    expect(importRef.importClassifiedPathsIntoCollection).toHaveBeenCalledTimes(1)
    expect(outcome.imported).toEqual([{ index: 0, itemId: 'item-1' }])
    expect(outcome.notProcessed).toEqual([1, 2])
    expect(outcome.cancelled).toBe(true)
  })

  it('fails with a typed error when the job names no target collection', async () => {
    iaRef.iaGetImportStatus.mockResolvedValue(status(null))

    const failure = await importVerifiedFiles('job-1').catch((reason) => reason)

    expect(failure).toBeInstanceOf(IaImportError)
    expect(failure.code).toBe('missing_collection')
    expect(iaRef.iaListVerifiedFiles).not.toHaveBeenCalled()
    expect(iaRef.iaBeginFileImport).not.toHaveBeenCalled()
    expect(importRef.importClassifiedPathsIntoCollection).not.toHaveBeenCalled()
  })

  it('reports progress for the current file and forwards the importer stage', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0), file(1)])
    iaRef.iaBeginFileImport.mockImplementation(async (_job: string, index: number) => ({
      action: 'import',
      file: file(index),
    }))
    importRef.importClassifiedPathsIntoCollection.mockImplementation(
      async (_paths, _collection, options) => {
        options.onProgress?.({
          total: 1,
          completed: 0,
          imported: 0,
          failed: 0,
          skipped: 0,
          currentFileName: 'File 0.pdf',
          stage: 'copyingFile',
        })
        return imported('item-1', 'Title 0')
      }
    )
    const seen: IaImportProgress[] = []

    await importVerifiedFiles('job-1', { onProgress: (progress) => seen.push(progress) })

    expect(seen.length).toBeGreaterThan(0)
    expect(seen[0]).toMatchObject({ index: 0, displayFileName: 'File 0.pdf' })
    const forwarded = seen.find((progress) => progress.stage === 'copyingFile')
    expect(forwarded).toMatchObject({ index: 0, displayFileName: 'File 0.pdf' })
    expect(seen.find((progress) => progress.index === 1)).toMatchObject({
      displayFileName: 'File 1.pdf',
      imported: 1,
    })
  })

  it('passes the translated base message through to the importer', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0)])

    await importVerifiedFiles('job-1', { baseErrorMessage: 'Importar archivo' })

    const [, , options] = importRef.importClassifiedPathsIntoCollection.mock.calls[0]!
    expect(options.baseErrorMessage).toBe('Importar archivo')
  })

  it('carries a neutral English default instead of inventing UI copy', async () => {
    iaRef.iaListVerifiedFiles.mockResolvedValue([file(0)])

    await importVerifiedFiles('job-1')

    const [, , options] = importRef.importClassifiedPathsIntoCollection.mock.calls[0]!
    expect(options.baseErrorMessage).toMatch(/[A-Za-z]/)
  })
})

describe('resolveReconcileLeftovers', () => {
  const entry = (patch: Partial<IaReconcileEntry>): IaReconcileEntry => ({
    jobId: 'job-1',
    index: 0,
    identifier: 'some-id',
    fileName: 'file-0.pdf',
    action: 'none',
    ...patch,
  })

  it('discards the remnant before releasing a discard entry', async () => {
    iaRef.iaListReconcile.mockResolvedValue([entry({ action: 'discard', itemId: 'item-old' })])

    const outcome = await resolveReconcileLeftovers()

    expect(iaRef.iaGetImportStatus).toHaveBeenCalledWith('job-1')
    expect(importRef.discardFailedImport).toHaveBeenCalledWith('col-1', 'item-old')
    expect(iaRef.iaFinishReconcile).toHaveBeenCalledWith('job-1', 0)
    expect(outcome).toEqual({ released: 1, failed: [] })
  })

  it('releases keep and none entries straight away', async () => {
    iaRef.iaListReconcile.mockResolvedValue([
      entry({ action: 'keep', index: 0, itemId: 'item-1' }),
      entry({ action: 'none', index: 1 }),
    ])

    const outcome = await resolveReconcileLeftovers()

    expect(importRef.discardFailedImport).not.toHaveBeenCalled()
    expect(iaRef.iaGetImportStatus).not.toHaveBeenCalled()
    expect(iaRef.iaFinishReconcile).toHaveBeenCalledWith('job-1', 0)
    expect(iaRef.iaFinishReconcile).toHaveBeenCalledWith('job-1', 1)
    expect(outcome).toEqual({ released: 2, failed: [] })
  })

  it('records a failing finish and releases the rest', async () => {
    iaRef.iaListReconcile.mockResolvedValue([
      entry({ action: 'none', index: 0 }),
      entry({ action: 'none', index: 1 }),
    ])
    iaRef.iaFinishReconcile
      .mockRejectedValueOnce(new Error('partial_remains: still there'))
      .mockResolvedValueOnce(undefined)

    const outcome = await resolveReconcileLeftovers()

    expect(outcome).toEqual({
      released: 1,
      failed: [{ jobId: 'job-1', index: 0, code: 'partial_remains' }],
    })
  })
})
