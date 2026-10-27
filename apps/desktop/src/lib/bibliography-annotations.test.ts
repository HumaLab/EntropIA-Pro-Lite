import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import {
  createAnnotation,
  deleteAnnotation,
  existenceReason,
  isVisibleInViewer,
  listAnnotations,
  parsePosition,
  resolveAnnotationConflict,
  restoreAnnotation,
  syncAnnotations,
  updateAnnotation,
  type AnnotationView,
} from './bibliography-annotations'

const mockInvoke = vi.mocked(invoke)

beforeEach(() => {
  mockInvoke.mockReset()
})

function view(overrides: Partial<AnnotationView> = {}): AnnotationView {
  return {
    key: 'ABCD2345',
    parentKey: 'PARENT22',
    data: { annotationType: 'highlight', annotationPosition: '{"pageIndex":2,"rects":[[1,2,3,4]]}' },
    syncState: 'synced',
    conflictFields: [],
    conflictRemote: null,
    remoteTrashed: false,
    lastError: null,
    updatedAt: 0,
    ...overrides,
  }
}

describe('bibliography-annotations', () => {
  it('names the library row and the key on the wire, never a credential', async () => {
    mockInvoke.mockResolvedValue({ status: 'ready', annotations: [] })
    await listAnnotations('lib-1', 'PARENT22')
    expect(mockInvoke).toHaveBeenLastCalledWith('bibliography_annotations_list', {
      libraryRowId: 'lib-1',
      parentKey: 'PARENT22',
    })
    await createAnnotation('lib-1', 'PARENT22', { annotationType: 'note' })
    expect(mockInvoke).toHaveBeenLastCalledWith('bibliography_annotations_create', {
      libraryRowId: 'lib-1',
      parentKey: 'PARENT22',
      data: { annotationType: 'note' },
    })
    await updateAnnotation('lib-1', 'ABCD2345', { annotationComment: 'x' })
    expect(mockInvoke).toHaveBeenLastCalledWith('bibliography_annotations_update', {
      libraryRowId: 'lib-1',
      key: 'ABCD2345',
      data: { annotationComment: 'x' },
    })
    await deleteAnnotation('lib-1', 'ABCD2345')
    expect(mockInvoke).toHaveBeenLastCalledWith('bibliography_annotations_delete', {
      libraryRowId: 'lib-1',
      key: 'ABCD2345',
      confirmed: false,
    })
    await restoreAnnotation('lib-1', 'ABCD2345')
    expect(mockInvoke).toHaveBeenLastCalledWith('bibliography_annotations_restore', {
      libraryRowId: 'lib-1',
      key: 'ABCD2345',
    })
    await resolveAnnotationConflict('lib-1', 'ABCD2345', { __existence: 'keep', annotationComment: 'zotero' })
    expect(mockInvoke).toHaveBeenLastCalledWith('bibliography_annotations_resolve', {
      libraryRowId: 'lib-1',
      key: 'ABCD2345',
      decisions: { __existence: 'keep', annotationComment: 'zotero' },
    })
    await syncAnnotations('lib-1', 'open')
    expect(mockInvoke).toHaveBeenLastCalledWith('bibliography_annotations_sync', {
      libraryRowId: 'lib-1',
      reason: 'open',
    })
  })

  it('parses the wire position and refuses an unreadable one', () => {
    expect(parsePosition(view().data)).toEqual({ pageIndex: 2, rects: [[1, 2, 3, 4]] })
    expect(parsePosition({ annotationPosition: { pageIndex: 0 } })).toEqual({ pageIndex: 0 })
    expect(parsePosition({ annotationPosition: 'not json' })).toBeNull()
    expect(parsePosition({ annotationPosition: '{"rects":[]}' })).toBeNull()
    expect(parsePosition(null)).toBeNull()
  })

  it('hides trashed annotations from the viewer and reads the existence reason', () => {
    expect(isVisibleInViewer(view())).toBe(true)
    expect(isVisibleInViewer(view({ data: { deleted: 1 } }))).toBe(false)
    expect(existenceReason(view({ conflictRemote: { __existence: 'trashed' } }))).toBe('trashed')
    expect(existenceReason(view())).toBeNull()
  })
})
