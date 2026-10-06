import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import {
  bibliographyListWorks,
  bibliographyOpenWorkAttachment,
  bibliographyWorkDetail,
} from './bibliography-library'

const mockInvoke = vi.mocked(invoke)

beforeEach(() => {
  mockInvoke.mockReset()
})

describe('bibliography-library', () => {
  it('lists one page of works with the library scope on the wire', async () => {
    const page = {
      works: [
        {
          itemId: 'item-1',
          itemKey: 'AAAA1111',
          libraryRowId: 'lib-row-1',
          title: 'El oficio de historiador',
          authors: 'Bloch',
          year: 1949,
          libraryName: 'Mi biblioteca',
          libraryType: 'user',
          libraryNativeId: '0',
          cslJson: '{"id":"x"}',
        },
      ],
      total: 120,
    }
    mockInvoke.mockResolvedValueOnce(page)

    const result = await bibliographyListWorks({
      library: { libraryType: 'user', libraryId: '0' },
      offset: 50,
      limit: 50,
      query: 'oficio',
    })

    expect(result).toEqual(page)
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_list_works', {
      request: {
        offset: 50,
        limit: 50,
        query: 'oficio',
        zoteroLibraryType: 'user',
        zoteroLibraryId: '0',
      },
    })
  })

  it('lists without a scope when no library is picked', async () => {
    mockInvoke.mockResolvedValueOnce({ works: [], total: 0 })

    await bibliographyListWorks({ offset: 0, limit: 50 })

    expect(mockInvoke).toHaveBeenCalledWith('bibliography_list_works', {
      request: {
        offset: 0,
        limit: 50,
        query: null,
        zoteroLibraryType: null,
        zoteroLibraryId: null,
      },
    })
  })

  it('reads one work detail by its catalog item id', async () => {
    const detail = {
      itemId: 'item-1',
      itemKey: 'AAAA1111',
      title: 'Obra',
      authors: 'Bloch',
      year: 1949,
      libraryName: 'Mi biblioteca',
      libraryType: 'user',
      libraryNativeId: '0',
      cslJson: '{}',
      item: {
        itemKey: 'AAAA1111',
        itemType: 'book',
        title: 'Obra',
        creators: null,
        publicationTitle: null,
        publisher: null,
        date: null,
        doi: null,
        isbn: null,
        abstract: null,
        language: null,
        url: null,
        itemVersion: 3,
        collections: [],
        tags: [],
        attachments: [],
      },
    }
    mockInvoke.mockResolvedValueOnce(detail)

    const result = await bibliographyWorkDetail('item-1')

    expect(result).toEqual(detail)
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_work_detail', { itemId: 'item-1' })
  })

  it('opens one work attachment by item id and attachment key', async () => {
    const opened = {
      itemId: 'item-1',
      itemKey: 'AAAA1111',
      title: 'Obra',
      attachmentKey: 'ATT1',
      originalKind: 'pdf',
      originalPath: 'C:/zotero/storage/ATT1/doc.pdf',
      openError: null,
      pages: [{ pageNumber: 1, method: 'native', quality: 'rich', text: 'Primera página.' }],
      snapshotText: '',
      extracted: true,
    }
    mockInvoke.mockResolvedValueOnce(opened)

    const result = await bibliographyOpenWorkAttachment('item-1', 'ATT1')

    expect(result).toEqual(opened)
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_open_work_attachment', {
      itemId: 'item-1',
      attachmentKey: 'ATT1',
    })
  })
})
