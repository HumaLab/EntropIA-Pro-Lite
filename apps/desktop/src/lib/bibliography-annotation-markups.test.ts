import { describe, expect, it } from 'vitest'
import {
  compareViews,
  draftToData,
  viewPage,
  viewToMarkup,
  withField,
  withGeometry,
} from './bibliography-annotation-markups'
import type { AnnotationView } from './bibliography-annotations'

function view(data: AnnotationView['data'], overrides: Partial<AnnotationView> = {}): AnnotationView {
  return {
    key: 'ABCD2345',
    parentKey: 'PARENT22',
    data,
    syncState: 'synced',
    conflictFields: [],
    conflictRemote: null,
    remoteTrashed: false,
    lastError: null,
    updatedAt: 0,
    ...overrides,
  }
}

const highlight = {
  annotationType: 'highlight' as const,
  annotationColor: '#ff6666',
  annotationText: 'quoted',
  annotationComment: 'note',
  annotationSortIndex: '00001|000010|00100',
  annotationPosition:
    '{"pageIndex":1,"rects":[[1,2,3,4],[5,6,7,8]],"nextPageRects":[[9,9,10,10]],"future":true}',
}

describe('bibliography-annotation-markups', () => {
  it('maps a stored highlight to a multi-rect markup', () => {
    expect(viewToMarkup(view(highlight))).toEqual({
      id: 'ABCD2345',
      type: 'highlight',
      color: '#ff6666',
      pageIndex: 1,
      rects: [
        [1, 2, 3, 4],
        [5, 6, 7, 8],
      ],
      nextPageRects: [[9, 9, 10, 10]],
      text: 'quoted',
      comment: 'note',
    })
  })

  it('draws no trashed, unknown or position-less annotation', () => {
    expect(viewToMarkup(view({ ...highlight, deleted: 1 }))).toBeNull()
    expect(viewToMarkup(view({ ...highlight, annotationType: undefined }))).toBeNull()
    expect(viewToMarkup(view({ annotationType: 'note' }))).toBeNull()
  })

  it('keeps ink and conflicted annotations read-only', () => {
    const ink = view({ annotationType: 'ink', annotationPosition: '{"pageIndex":0,"paths":[[1,2,3,4]],"width":2}' })
    expect(viewToMarkup(ink)).toMatchObject({ type: 'ink', paths: [[1, 2, 3, 4]], width: 2, readOnly: true })
    expect(viewToMarkup(view(highlight, { syncState: 'conflict' }))?.readOnly).toBe(true)
  })

  it('turns a viewer draft into a Zotero data object', () => {
    expect(
      draftToData({
        type: 'highlight',
        color: '#ffd400',
        pageIndex: 2,
        rects: [[1, 2, 3, 4]],
        text: 'words',
        sortIndex: '00002|000003|00004',
      })
    ).toEqual({
      annotationType: 'highlight',
      annotationColor: '#ffd400',
      annotationComment: '',
      annotationPageLabel: '3',
      annotationPosition: { pageIndex: 2, rects: [[1, 2, 3, 4]] },
      annotationText: 'words',
      annotationSortIndex: '00002|000003|00004',
      tags: [],
    })
    const note = draftToData({ type: 'note', color: '#ffd400', pageIndex: 0, rects: [[0, 0, 22, 22]] })
    expect(note).not.toHaveProperty('annotationText')
    const text = draftToData({ type: 'text', color: '#000000', pageIndex: 0, rects: [[0, 0, 1, 1]], fontSize: 12 })
    expect(text.annotationPosition).toEqual({ pageIndex: 0, rects: [[0, 0, 1, 1]], fontSize: 12, rotation: 0 })
  })

  it('changes geometry without dropping unknown position fields', () => {
    const moved = withGeometry(highlight, { id: 'ABCD2345', rects: [[10, 20, 30, 40]], sortIndex: '00001|000000|00050' })
    expect(moved.annotationPosition).toEqual({
      pageIndex: 1,
      rects: [[10, 20, 30, 40]],
      nextPageRects: [[9, 9, 10, 10]],
      future: true,
    })
    expect(moved.annotationSortIndex).toBe('00001|000000|00050')
    expect(withField(highlight, 'annotationComment', 'x').annotationComment).toBe('x')
  })

  it('orders the list by page, then sort index', () => {
    const a = view({ ...highlight, annotationSortIndex: '00001|000020|00100' }, { key: 'AAAA2222' })
    const b = view({ ...highlight, annotationSortIndex: '00001|000010|00100' }, { key: 'BBBB3333' })
    const c = view({ ...highlight, annotationPosition: '{"pageIndex":0,"rects":[]}' }, { key: 'CCCC4444' })
    expect([a, b, c].sort(compareViews).map((entry) => entry.key)).toEqual(['CCCC4444', 'BBBB3333', 'AAAA2222'])
    expect(viewPage(c)).toBe(1)
  })
})
