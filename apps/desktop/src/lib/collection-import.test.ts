import { beforeEach, describe, expect, it, vi } from 'vitest'
import { importClassifiedPathsIntoCollection, formatImportStageError } from './collection-import'

const { storeRef, fileImportRef } = vi.hoisted(() => ({
  storeRef: {
    current: {
      items: {
        create: vi.fn(),
        update: vi.fn(),
        delete: vi.fn(),
        findImportedFromSource: vi.fn().mockResolvedValue(null),
      },
      assets: {
        create: vi.fn(),
        findByItem: vi.fn().mockResolvedValue([]),
        deleteWithCascade: vi.fn(),
      },
    },
  },
  fileImportRef: {
    classifyFiles: vi.fn(),
    importSingleFile: vi.fn(),
    splitPdfPages: vi.fn(),
    readSourceFingerprint: vi.fn(),
  },
}))

vi.mock('./db', () => ({
  getStore: () => storeRef.current,
}))

vi.mock('./file-import', () => ({
  classifyFiles: fileImportRef.classifyFiles,
  importSingleFile: fileImportRef.importSingleFile,
  splitPdfPages: fileImportRef.splitPdfPages,
  readSourceFingerprint: fileImportRef.readSourceFingerprint,
}))

beforeEach(() => {
  vi.clearAllMocks()
  storeRef.current = {
    items: {
      create: vi.fn(),
      update: vi.fn(),
      delete: vi.fn(),
      findImportedFromSource: vi.fn().mockResolvedValue(null),
    },
    assets: {
      create: vi.fn(),
      findByItem: vi.fn().mockResolvedValue([]),
      deleteWithCascade: vi.fn(),
    },
  }
  fileImportRef.classifyFiles.mockReturnValue({ classified: [], rejected: [] })
  fileImportRef.splitPdfPages.mockResolvedValue([])
  fileImportRef.readSourceFingerprint.mockResolvedValue({
    originalPath: '',
    sizeBytes: 0,
    modifiedAt: null,
  })
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
    })
    expect(storeRef.current.items.create).not.toHaveBeenCalled()
  })

  it('creates one item and one asset per classified file', async () => {
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [{ sourcePath: '/src/a.png', name: 'a.png', type: 'image' }],
      rejected: [],
    })
    storeRef.current.items.create.mockResolvedValue({ id: 'item-1' })
    fileImportRef.importSingleFile.mockResolvedValue({
      originalName: 'a.png',
      originalPath: '/src/a.png',
      destPath: '/data/a.png',
      type: 'image',
      size: 100,
      originalMetadata: { originalName: 'a.png', originalPath: '/src/a.png', importedAt: 'now' },
    })

    const result = await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
    })

    expect(storeRef.current.items.create).toHaveBeenCalledWith({
      title: 'a',
      collectionId: 'col-1',
      metadata: null,
    })
    expect(storeRef.current.assets.create).toHaveBeenCalledWith({
      itemId: 'item-1',
      path: '/data/a.png',
      type: 'image',
      size: 100,
      sortIndex: 0,
    })
    expect(result.classifiedCount).toBe(1)
    expect(result.createdItems).toEqual([{ id: 'item-1', title: 'a' }])
    expect(result.importErrors).toEqual([])
  })

  it('reports progress through onProgress at each stage', async () => {
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [{ sourcePath: '/src/a.png', name: 'a.png', type: 'image' }],
      rejected: [],
    })
    storeRef.current.items.create.mockResolvedValue({ id: 'item-1' })
    fileImportRef.importSingleFile.mockResolvedValue({
      originalName: 'a.png',
      originalPath: '/src/a.png',
      destPath: '/data/a.png',
      type: 'image',
      size: 100,
      originalMetadata: { originalName: 'a.png', originalPath: '/src/a.png', importedAt: 'now' },
    })

    const stages: string[] = []
    await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
      onProgress: (p) => stages.push(p.stage),
    })

    expect(stages).toEqual([
      'creatingDocument',
      'creatingDocument',
      'copyingFile',
      'savingDocument',
      'savingDocument',
      'completed',
    ])
  })

  it('skips a file already imported from the same source and reports it separately', async () => {
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [{ sourcePath: '/src/a.png', name: 'a.png', type: 'image' }],
      rejected: [],
    })
    storeRef.current.items.findImportedFromSource.mockResolvedValue({ id: 'existing-item' })

    const result = await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
    })

    expect(storeRef.current.items.create).not.toHaveBeenCalled()
    expect(result.alreadyImported).toEqual(['a.png'])
    expect(result.createdItems).toEqual([])
  })

  it('discards the item and collects the error when importing a file fails', async () => {
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [{ sourcePath: '/src/a.png', name: 'a.png', type: 'image' }],
      rejected: [],
    })
    storeRef.current.items.create.mockResolvedValue({ id: 'item-1' })
    fileImportRef.importSingleFile.mockRejectedValue(new Error('copy failed'))

    const result = await importClassifiedPathsIntoCollection(['/src/a.png'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
    })

    expect(storeRef.current.items.delete).toHaveBeenCalledWith('item-1')
    expect(result.createdItems).toEqual([])
    expect(result.importErrors).toEqual(['Failed to import files (importing a.png): copy failed'])
  })

  it('splits a multi-page PDF into one asset per page', async () => {
    fileImportRef.classifyFiles.mockReturnValue({
      classified: [{ sourcePath: '/src/doc.pdf', name: 'doc.pdf', type: 'pdf' }],
      rejected: [],
    })
    storeRef.current.items.create.mockResolvedValue({ id: 'item-1' })
    fileImportRef.importSingleFile.mockResolvedValue({
      originalName: 'doc.pdf',
      originalPath: '/src/doc.pdf',
      destPath: '/data/doc.pdf',
      type: 'pdf',
      size: 500,
      originalMetadata: {
        originalName: 'doc.pdf',
        originalPath: '/src/doc.pdf',
        importedAt: 'now',
      },
    })
    storeRef.current.assets.create
      .mockResolvedValueOnce({ id: 'parent-asset' })
      .mockResolvedValueOnce({ id: 'page-1' })
      .mockResolvedValueOnce({ id: 'page-2' })
    fileImportRef.splitPdfPages.mockResolvedValue([
      { page_number: 1, pdf_path: '/data/doc_page_1.pdf' },
      { page_number: 2, pdf_path: '/data/doc_page_2.pdf' },
    ])

    const result = await importClassifiedPathsIntoCollection(['/src/doc.pdf'], 'col-1', {
      baseErrorMessage: 'Failed to import files',
    })

    expect(storeRef.current.assets.create).toHaveBeenCalledTimes(3)
    expect(result.createdItems).toEqual([{ id: 'item-1', title: 'doc' }])
  })
})

describe('formatImportStageError', () => {
  it('formats a base message, stage and error detail', () => {
    expect(
      formatImportStageError('Failed to import files', 'selecting files', new Error('boom'))
    ).toBe('Failed to import files (selecting files): boom')
  })
})
