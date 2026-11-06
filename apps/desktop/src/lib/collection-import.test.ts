import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  importClassifiedPathsIntoCollection,
  formatImportStageError,
  buildImportSummary,
} from './collection-import'

/**
 * The app's own import runs on the Rust ingest service
 * (`odd/tasks/agent-corpus-tools.md` D10): the renderer classifies, then asks
 * `corpus_import_path` once per classified file. These tests pin the IPC
 * payloads and what each answer becomes in the result the views read.
 */

const { invokeMock, storeRef, fileImportRef } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  storeRef: {
    current: {
      items: { create: vi.fn(), update: vi.fn(), delete: vi.fn() },
      assets: {
        create: vi.fn(),
        findByItem: vi.fn().mockResolvedValue([]),
        deleteWithCascade: vi.fn(),
      },
    },
  },
  fileImportRef: { classifyFiles: vi.fn() },
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

vi.mock('./db', () => ({
  getStore: () => storeRef.current,
}))

vi.mock('./file-import', () => ({
  classifyFiles: fileImportRef.classifyFiles,
}))

function classifyA(sourcePath = '/src/a.png') {
  fileImportRef.classifyFiles.mockReturnValue({
    classified: [{ sourcePath, name: 'a.png', type: 'image' }],
    rejected: [],
  })
}

const IMPORTED = { outcome: 'imported', itemId: 'item-1', title: 'a', assets: 1 }

beforeEach(() => {
  vi.clearAllMocks()
  storeRef.current = {
    items: { create: vi.fn(), update: vi.fn(), delete: vi.fn() },
    assets: {
      create: vi.fn(),
      findByItem: vi.fn().mockResolvedValue([]),
      deleteWithCascade: vi.fn(),
    },
  }
  fileImportRef.classifyFiles.mockReturnValue({ classified: [], rejected: [] })
  invokeMock.mockResolvedValue(IMPORTED)
})

describe('importClassifiedPathsIntoCollection', () => {
  it('returns an empty result when every path is rejected', async () => {
    fileImportRef.classifyFiles.mockReturnValue({ classified: [], rejected: ['notes.exe'] })

    const result = await importClassifiedPathsIntoCollection(['notes.exe'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
    })

    expect(result).toEqual({
      classifiedCount: 0,
      rejected: ['notes.exe'],
      createdItems: [],
      importErrors: [],
      alreadyImported: [],
      missingCollection: false,
    })
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('imports each classified file through the backend, once per file', async () => {
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [
        { sourcePath: '/src/a.png', name: 'a.png', type: 'image' },
        { sourcePath: '/src/b.jpg', name: 'b.jpg', type: 'image' },
      ],
      rejected: [],
    })
    invokeMock
      .mockResolvedValueOnce(IMPORTED)
      .mockResolvedValueOnce({ outcome: 'imported', itemId: 'item-2', title: 'b', assets: 1 })

    const result = await importClassifiedPathsIntoCollection(
      ['/src/a.png', '/src/b.jpg'],
      'col-1',
      { baseErrorMessage: 'Failed to import files' }
    )

    expect(invokeMock).toHaveBeenNthCalledWith(1, 'corpus_import_path', {
      collectionId: 'col-1',
      sourcePath: '/src/a.png',
      overrides: null,
    })
    expect(invokeMock).toHaveBeenNthCalledWith(2, 'corpus_import_path', {
      collectionId: 'col-1',
      sourcePath: '/src/b.jpg',
      overrides: null,
    })
    expect(result.classifiedCount).toBe(2)
    expect(result.createdItems).toEqual([
      { id: 'item-1', title: 'a' },
      { id: 'item-2', title: 'b' },
    ])
    expect(result.importErrors).toEqual([])
  })

  it('titles the item with what the backend created', async () => {
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [{ sourcePath: '/data/web-captures/s1/c1.pdf', name: 'c1.pdf', type: 'pdf' }],
      rejected: [],
    })
    invokeMock.mockResolvedValue({
      outcome: 'imported',
      itemId: 'item-1',
      title: 'The paper',
      assets: 3,
    })

    const result = await importClassifiedPathsIntoCollection(
      ['/data/web-captures/s1/c1.pdf'],
      'col-1',
      { baseErrorMessage: 'Failed to import files' }
    )

    expect(result.createdItems).toEqual([{ id: 'item-1', title: 'The paper' }])
  })

  it('marks a batch refused because its collection is gone', async () => {
    classifyA()
    invokeMock.mockRejectedValue('not_found: no collection col-1')

    const result = await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', {
      baseErrorMessage: 'Failed to import dropped files',
    })

    expect(result.missingCollection).toBe(true)
    // The per-file error is unchanged: views that list it keep the sentence.
    expect(result.importErrors).toEqual([
      'Failed to import dropped files (importing a.png): no collection col-1',
    ])
  })

  it('does not mark any other refusal as a missing collection', async () => {
    classifyA()
    invokeMock.mockRejectedValue('not_found: no item item-1')

    const result = await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
    })

    expect(result.missingCollection).toBe(false)
  })

  it('reports an unchanged earlier import as already imported, never as an error', async () => {
    classifyA()
    invokeMock.mockResolvedValue({ outcome: 'duplicate', itemId: 'item-old' })

    const result = await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
    })

    expect(result.alreadyImported).toEqual(['a.png'])
    expect(result.createdItems).toEqual([])
    expect(result.importErrors).toEqual([])
  })

  it('collects a failed file and keeps importing the rest', async () => {
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [
        { sourcePath: '/src/broken.png', name: 'broken.png', type: 'image' },
        { sourcePath: '/src/ok.png', name: 'ok.png', type: 'image' },
      ],
      rejected: [],
    })
    invokeMock
      .mockRejectedValueOnce('unavailable: could not copy broken.png: disk full')
      .mockResolvedValueOnce({ outcome: 'imported', itemId: 'item-2', title: 'ok', assets: 1 })

    const result = await importClassifiedPathsIntoCollection(
      ['/src/broken.png', '/src/ok.png'],
      'col-1',
      { baseErrorMessage: 'Failed to import files' }
    )

    expect(result.importErrors).toEqual([
      'Failed to import files (importing broken.png): could not copy broken.png: disk full',
    ])
    expect(result.createdItems).toEqual([{ id: 'item-2', title: 'ok' }])
  })

  it('says an unsupported file the way the app has always said it', async () => {
    classifyA()
    invokeMock.mockRejectedValueOnce(
      'unsupported: /src/a.png is not a PDF, an image or an audio file'
    )

    const result = await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
    })

    expect(result.importErrors).toEqual([
      'Failed to import files (importing a.png): Unsupported file format: a.png',
    ])
  })

  it('forwards the overrides the caller gave, for a file that has its own identity', async () => {
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [{ sourcePath: '/data/web-captures/s1/c1.pdf', name: 'c1.pdf', type: 'pdf' }],
      rejected: [],
    })
    const overrides = {
      title: 'The paper',
      fileName: 'The paper.pdf',
      extraMetadata: { __entropia_web_capture: { captureId: 'c1' } },
      allowDuplicate: true,
    }

    await importClassifiedPathsIntoCollection(['/data/web-captures/s1/c1.pdf'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
      overrides,
    })

    expect(invokeMock).toHaveBeenCalledWith('corpus_import_path', {
      collectionId: 'col-1',
      sourcePath: '/data/web-captures/s1/c1.pdf',
      overrides,
    })
  })

  it('never creates the item or its assets itself', async () => {
    classifyA()

    await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
    })

    expect(storeRef.current.items.create).not.toHaveBeenCalled()
    expect(storeRef.current.items.update).not.toHaveBeenCalled()
    expect(storeRef.current.assets.create).not.toHaveBeenCalled()
    expect(storeRef.current.items.delete).not.toHaveBeenCalled()
  })

  it('reports progress from the start to each completed file', async () => {
    classifyA()
    const events: Array<{ stage: string; completed: number; currentFileName: string | null }> = []

    await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
      onProgress: (progress) => events.push(progress),
    })

    expect(events[0]).toMatchObject({
      total: 1,
      completed: 0,
      imported: 0,
      skipped: 0,
      currentFileName: null,
      stage: 'creatingDocument',
    })
    expect(
      events.some(
        (event) => event.stage === 'creatingDocument' && event.currentFileName === 'a.png'
      )
    ).toBe(true)
    expect(events.at(-1)).toMatchObject({
      completed: 1,
      imported: 1,
      currentFileName: 'a.png',
      stage: 'completed',
    })
  })

  it('counts a failed file as completed and failed, once', async () => {
    classifyA()
    invokeMock.mockRejectedValue('unavailable: could not copy a.png: disk full')
    const events: Array<{ stage: string; completed: number; failed: number }> = []

    await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
      onProgress: (progress) => events.push(progress),
    })

    expect(events.at(-1)).toMatchObject({ completed: 1, failed: 1, stage: 'completed' })
  })

  // The claim map is still the renderer's: the backend's duplicate check runs
  // inside one transaction, but two imports of the same path that started
  // together must not both report a duplicate of each other. Each call claims
  // `collectionId + path` before its request and holds it until the file
  // settles; a concurrent call finding the claim waits for that outcome and
  // reports "already imported" only if the other import actually left it in
  // the collection. If that import failed, this call claims and imports it.
  describe('two concurrent imports of the same file into one collection', () => {
    it('creates one item and reports the other as already imported', async () => {
      classifyA()
      const options = { baseErrorMessage: 'Failed to import files' }

      const [first, second] = await Promise.all([
        importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', options),
        importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', options),
      ])

      expect(invokeMock).toHaveBeenCalledOnce()
      expect([...first.createdItems, ...second.createdItems]).toHaveLength(1)
      expect([...first.alreadyImported, ...second.alreadyImported]).toEqual(['a.png'])
    })

    it('still imports the same file into two different collections', async () => {
      classifyA()
      const options = { baseErrorMessage: 'Failed to import files' }

      await Promise.all([
        importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', options),
        importClassifiedPathsIntoCollection(['/src/a.png'], 'col-2', options),
      ])

      expect(invokeMock).toHaveBeenCalledTimes(2)
    })

    it('imports the file itself when the concurrent import holding it fails', async () => {
      classifyA()
      invokeMock.mockRejectedValueOnce('unavailable: could not copy a.png: db locked')
      const options = { baseErrorMessage: 'Failed to import files' }

      const [first, second] = await Promise.all([
        importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', options),
        importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', options),
      ])

      expect(first.importErrors).toHaveLength(1)
      // Never reported as already imported: nothing was.
      expect(second.alreadyImported).toEqual([])
      expect(second.createdItems).toHaveLength(1)
    })

    it('releases the path once an import settles, even a failed one', async () => {
      classifyA()
      invokeMock.mockRejectedValueOnce('unavailable: could not copy a.png: db locked')
      const options = { baseErrorMessage: 'Failed to import files' }
      const failed = await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', options)
      expect(failed.importErrors).toHaveLength(1)

      const retried = await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', options)

      expect(retried.createdItems).toHaveLength(1)
      expect(retried.alreadyImported).toEqual([])
    })
  })
})

describe('buildImportSummary', () => {
  it('reports the last created item title when nothing failed', () => {
    const summary = buildImportSummary({
      classifiedCount: 1,
      rejected: [],
      createdItems: [{ id: 'item-1', title: 'a' }],
      importErrors: [],
      alreadyImported: [],
      missingCollection: false,
    })

    expect(summary).toEqual({
      imported: 1,
      skipped: 0,
      errors: [],
      rejected: [],
      alreadyImported: [],
      lastItemTitle: 'a',
    })
  })

  it('sums rejected and alreadyImported into skipped', () => {
    const summary = buildImportSummary({
      classifiedCount: 2,
      rejected: ['b.exe'],
      createdItems: [{ id: 'item-1', title: 'a' }],
      importErrors: [],
      alreadyImported: ['c.png'],
      missingCollection: false,
    })

    expect(summary.skipped).toBe(2)
  })

  it('never reports a last item title when there was a failure, even with a created item', () => {
    const summary = buildImportSummary({
      classifiedCount: 2,
      rejected: [],
      createdItems: [{ id: 'item-1', title: 'a' }],
      importErrors: ['Failed to import files (importing b.png): boom'],
      alreadyImported: [],
      missingCollection: false,
    })

    expect(summary.lastItemTitle).toBeNull()
    expect(summary.errors).toEqual(['Failed to import files (importing b.png): boom'])
  })
})

describe('formatImportStageError', () => {
  it('formats a base message, stage and error detail', () => {
    expect(
      formatImportStageError('Failed to import files', 'selecting files', new Error('boom'))
    ).toBe('Failed to import files (selecting files): boom')
  })
})
