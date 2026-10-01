import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import type { WebCaptureProvenance } from './item-metadata'
import {
  CopyError,
  copyCaptureToCollection,
  copyFileName,
  copyOverrides,
  copyTitle,
  findExistingCopy,
} from './navegador-copy'

const { storeRef, importRef } = vi.hoisted(() => ({
  storeRef: {
    current: {
      items: { findByWebCapture: vi.fn() },
    },
  },
  importRef: { importClassifiedPathsIntoCollection: vi.fn() },
}))

vi.mock('./db', () => ({ getStore: () => storeRef.current }))
vi.mock('./collection-import', () => ({
  importClassifiedPathsIntoCollection: importRef.importClassifiedPathsIntoCollection,
}))

const SHA = 'a'.repeat(64)

const provenance = (patch: Partial<WebCaptureProvenance> = {}): WebCaptureProvenance => ({
  sourceId: 's1',
  captureId: 'c1',
  originalUrl: 'https://example.org/articles/227',
  finalUrl: 'https://example.org/articles/227/descargar',
  pageTitle: 'La cuestión social: una lectura',
  accessedAt: '2026-10-01T12:00:00Z',
  sha256: SHA,
  ...patch,
})

describe('the title and file name of a copy', () => {
  it('titles the item with the page title', () => {
    expect(copyTitle(provenance())).toBe('La cuestión social: una lectura')
  })

  it('falls back to the file name in the address when there is no title', () => {
    expect(
      copyTitle(
        provenance({ pageTitle: null, finalUrl: 'https://e.org/files/Acta%20N%C2%BA3.pdf' })
      )
    ).toBe('Acta Nº3')
    expect(
      copyTitle(provenance({ pageTitle: '   ', finalUrl: 'https://e.org/files/acta.PDF' }))
    ).toBe('acta')
  })

  it('falls back to the host when the address has no file name', () => {
    expect(copyTitle(provenance({ pageTitle: null, finalUrl: 'https://www.e.org/' }))).toBe('e.org')
  })

  it('never returns an empty title', () => {
    expect(copyTitle(provenance({ pageTitle: null, finalUrl: 'not a url' }))).toBe('PDF')
  })

  it('names the stored file after the title, with a pdf extension', () => {
    expect(copyFileName(provenance())).toBe('La cuestión social una lectura.pdf')
  })

  it.each([
    ['a/b\\c:d*e?f"g<h>i|j', 'abcdefghij.pdf'],
    ['  .. dots and spaces ..  ', 'dots and spaces.pdf'],
    ['tab\tand\nnewline', 'tab and newline.pdf'],
    ['bidi\u202etext', 'biditext.pdf'],
    ['CON', '_CON.pdf'],
    ['nul.txt', '_nul.txt.pdf'],
    ['', 'PDF.pdf'],
    ['////', 'PDF.pdf'],
  ])('sanitises %j into a safe file name', (title, expected) => {
    expect(copyFileName(provenance({ pageTitle: title, finalUrl: 'not a url' }))).toBe(expected)
  })

  it('keeps a long title to a bounded file name', () => {
    const name = copyFileName(provenance({ pageTitle: 'x'.repeat(500) }))
    expect(name.length).toBeLessThanOrEqual(100)
    expect(name.endsWith('.pdf')).toBe(true)
  })
})

describe('copyOverrides', () => {
  it('carries the title, the file name and the provenance under the reserved key', () => {
    const overrides = copyOverrides(provenance())

    expect(overrides.title).toBe('La cuestión social: una lectura')
    expect(overrides.fileName).toBe('La cuestión social una lectura.pdf')
    expect(overrides.extraMetadata).toEqual({ __entropia_web_capture: provenance() })
  })

  it('leaves duplicates to the flow that asked the person first', () => {
    expect(copyOverrides(provenance()).allowDuplicate).toBe(true)
  })
})

describe('findExistingCopy', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('asks the store for the copies of that capture in that collection', async () => {
    storeRef.current.items.findByWebCapture.mockResolvedValue({ id: 'i1', title: 'T' })

    await expect(findExistingCopy('col-1', 'c1')).resolves.toEqual({ id: 'i1', title: 'T' })
    expect(storeRef.current.items.findByWebCapture).toHaveBeenCalledWith('col-1', 'c1')
  })

  it('says there is none rather than failing the copy when the lookup fails', async () => {
    storeRef.current.items.findByWebCapture.mockRejectedValue(new Error('locked'))

    await expect(findExistingCopy('col-1', 'c1')).resolves.toBeNull()
  })
})

describe('copyCaptureToCollection', () => {
  const ticket = { path: 'C:/data/web-captures/s1/c1.pdf', provenance: provenance() }

  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'navegador_copy_ticket') return ticket
      return undefined
    })
    importRef.importClassifiedPathsIntoCollection.mockResolvedValue({
      classifiedCount: 1,
      rejected: [],
      createdItems: [{ id: 'item-9', title: 'La cuestión social: una lectura' }],
      importErrors: [],
      alreadyImported: [],
    })
  })

  it('asks Rust for the file by capture id and imports it with the provenance', async () => {
    const copied = await copyCaptureToCollection({ captureId: 'c1', collectionId: 'col-1' })

    expect(invoke).toHaveBeenCalledWith('navegador_copy_ticket', { captureId: 'c1' })
    expect(importRef.importClassifiedPathsIntoCollection).toHaveBeenCalledWith(
      ['C:/data/web-captures/s1/c1.pdf'],
      'col-1',
      expect.objectContaining({ overrides: copyOverrides(provenance()) })
    )
    expect(copied).toEqual({
      item: { id: 'item-9', title: 'La cuestión social: una lectura' },
      collectionId: 'col-1',
    })
  })

  it('imports only the path Rust answered', async () => {
    await copyCaptureToCollection({ captureId: 'c1', collectionId: 'col-1' })

    const [paths] = importRef.importClassifiedPathsIntoCollection.mock.calls[0]!
    expect(paths).toEqual([ticket.path])
  })

  it('reports progress from the import', async () => {
    const stages: string[] = []
    importRef.importClassifiedPathsIntoCollection.mockImplementation(async (_p, _c, options) => {
      options.onProgress?.({ stage: 'copyingFile' })
      return {
        classifiedCount: 1,
        rejected: [],
        createdItems: [{ id: 'i', title: 't' }],
        importErrors: [],
        alreadyImported: [],
      }
    })

    await copyCaptureToCollection({
      captureId: 'c1',
      collectionId: 'col-1',
      onStage: (stage) => stages.push(stage),
    })

    expect(stages).toEqual(['copyingFile'])
  })

  it.each(['not_found', 'not_a_pdf', 'file_missing', 'file_changed', 'invalid_id', 'db_error'])(
    'stops with the code %s when Rust refuses the capture, importing nothing',
    async (code) => {
      vi.mocked(invoke).mockRejectedValue(`${code}: why`)

      const failure = await copyCaptureToCollection({
        captureId: 'c1',
        collectionId: 'col-1',
      }).catch((reason) => reason)

      expect(failure).toBeInstanceOf(CopyError)
      expect(failure.code).toBe(code)
      expect(importRef.importClassifiedPathsIntoCollection).not.toHaveBeenCalled()
    }
  )

  it('fails with the import error when the import could not create the item', async () => {
    importRef.importClassifiedPathsIntoCollection.mockResolvedValue({
      classifiedCount: 1,
      rejected: [],
      createdItems: [],
      importErrors: ['Copy (importing x.pdf): disk full'],
      alreadyImported: [],
    })

    const failure = await copyCaptureToCollection({
      captureId: 'c1',
      collectionId: 'col-1',
    }).catch((reason) => reason)

    expect(failure).toBeInstanceOf(CopyError)
    expect(failure.code).toBe('import_failed')
    expect(failure.detail).toContain('disk full')
  })

  it('fails when nothing was created and nothing was reported', async () => {
    importRef.importClassifiedPathsIntoCollection.mockResolvedValue({
      classifiedCount: 1,
      rejected: [],
      createdItems: [],
      importErrors: [],
      alreadyImported: ['c1.pdf'],
    })

    const failure = await copyCaptureToCollection({
      captureId: 'c1',
      collectionId: 'col-1',
    }).catch((reason) => reason)

    expect(failure).toBeInstanceOf(CopyError)
    expect(failure.code).toBe('not_created')
  })
})
