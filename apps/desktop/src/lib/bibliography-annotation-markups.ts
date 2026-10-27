import type { PdfMarkup, PdfMarkupDraft, PdfMarkupGeometryChange, PdfRect } from '@entropia/ui'
import {
  isVisibleInViewer,
  parsePosition,
  type AnnotationView,
  type ZoteroAnnotationData,
  type ZoteroAnnotationPosition,
  type ZoteroAnnotationType,
} from './bibliography-annotations'

/**
 * Translation between the Zotero annotation `data` object the backend
 * stores and the viewer's `PdfMarkup` (docs/zotero-annotations-plan.md,
 * section 7). Geometry stays in PDF user space on both sides; only the
 * page index and the field names change.
 */

const MARKUP_TYPES: readonly ZoteroAnnotationType[] = [
  'highlight',
  'underline',
  'note',
  'image',
  'text',
  'ink',
]

function asRect(value: unknown): PdfRect | null {
  if (!Array.isArray(value) || value.length < 4) return null
  const numbers = value.slice(0, 4).map(Number)
  return numbers.every(Number.isFinite) ? (numbers as PdfRect) : null
}

function asRects(value: unknown): PdfRect[] {
  return Array.isArray(value) ? value.map(asRect).filter((rect): rect is PdfRect => rect !== null) : []
}

/** A stored annotation as a viewer markup; `null` when it cannot be drawn. */
export function viewToMarkup(view: AnnotationView): PdfMarkup | null {
  if (!isVisibleInViewer(view)) return null
  const data = view.data
  const type = data?.annotationType
  if (!data || !type || !MARKUP_TYPES.includes(type)) return null
  const position = parsePosition(data)
  if (!position) return null
  const markup: PdfMarkup = {
    id: view.key,
    type,
    color: typeof data.annotationColor === 'string' ? data.annotationColor : '#ffd400',
    pageIndex: position.pageIndex,
    rects: asRects(position.rects),
  }
  const next = asRects(position.nextPageRects)
  if (next.length > 0) markup.nextPageRects = next
  if (Array.isArray(position.paths)) {
    markup.paths = position.paths.filter(Array.isArray).map((path) => path.map(Number))
  }
  if (typeof position.width === 'number') markup.width = position.width
  if (typeof position.fontSize === 'number') markup.fontSize = position.fontSize
  if (typeof position.rotation === 'number') markup.rotation = position.rotation
  if (typeof data.annotationText === 'string' && data.annotationText) markup.text = data.annotationText
  if (typeof data.annotationComment === 'string' && data.annotationComment) {
    markup.comment = data.annotationComment
  }
  // Ink is shown and deletable, but its strokes are not edited here.
  if (type === 'ink' || view.syncState === 'conflict') markup.readOnly = true
  return markup
}

/** The data object of a new annotation drawn or selected in the viewer. */
export function draftToData(draft: PdfMarkupDraft): ZoteroAnnotationData {
  const position: ZoteroAnnotationPosition = { pageIndex: draft.pageIndex, rects: draft.rects }
  if (draft.type === 'text') {
    position.fontSize = draft.fontSize ?? 14
    position.rotation = 0
  }
  const data: ZoteroAnnotationData = {
    annotationType: draft.type,
    annotationColor: draft.color,
    annotationComment: '',
    annotationPageLabel: String(draft.pageIndex + 1),
    annotationPosition: position,
    tags: [],
  }
  if (draft.type === 'highlight' || draft.type === 'underline') {
    data.annotationText = draft.text ?? ''
  }
  if (draft.sortIndex) data.annotationSortIndex = draft.sortIndex
  return data
}

/** The full desired object after a geometry change; unknown position fields survive. */
export function withGeometry(
  data: ZoteroAnnotationData,
  change: PdfMarkupGeometryChange
): ZoteroAnnotationData {
  const position = parsePosition(data)
  if (!position) return data
  const next: ZoteroAnnotationData = {
    ...data,
    annotationPosition: { ...position, rects: change.rects },
  }
  if (change.sortIndex) next.annotationSortIndex = change.sortIndex
  return next
}

/** The full desired object with one field changed. */
export function withField(
  data: ZoteroAnnotationData,
  field: 'annotationComment' | 'annotationColor',
  value: string
): ZoteroAnnotationData {
  return { ...data, [field]: value }
}

/** The page an annotation is listed under (one-based), or null without a position. */
export function viewPage(view: AnnotationView): number | null {
  const position = parsePosition(view.data)
  return position ? position.pageIndex + 1 : null
}

/** List order: page, then Zotero's sort index, then key. */
export function compareViews(a: AnnotationView, b: AnnotationView): number {
  const pageA = viewPage(a) ?? Number.MAX_SAFE_INTEGER
  const pageB = viewPage(b) ?? Number.MAX_SAFE_INTEGER
  if (pageA !== pageB) return pageA - pageB
  const sortA = String(a.data?.annotationSortIndex ?? '')
  const sortB = String(b.data?.annotationSortIndex ?? '')
  if (sortA !== sortB) return sortA < sortB ? -1 : 1
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0
}
