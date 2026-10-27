/**
 * Zotero-style PDF markups (docs/zotero-annotations-plan.md, sections 7-9).
 *
 * Kept apart from Collections' `ViewerAnnotation` on purpose: a markup is one
 * entity with any number of rects, its geometry lives in the page's PDF user
 * space (what Zotero stores, before rotation), and highlights and underlines
 * keep their geometry once created.
 */

/** `[x1, y1, x2, y2]` in PDF user space, origin bottom-left, as Zotero stores it. */
export type PdfRect = [number, number, number, number]

/** `[x, y]` in PDF user space. */
export type PdfPoint = [number, number]

export type PdfMarkupType = 'highlight' | 'underline' | 'note' | 'image' | 'text' | 'ink'

/** Tools of the markup toolbar. `image` is Zotero's "area". */
export type MarkupTool = 'select' | 'highlight' | 'underline' | 'note' | 'image' | 'text'

export const MARKUP_TOOLS: readonly MarkupTool[] = [
  'select',
  'highlight',
  'underline',
  'note',
  'image',
  'text',
]

export interface PdfMarkup {
  /** The Zotero annotation key. */
  id: string
  type: PdfMarkupType
  color: string
  /** Zero-based, as Zotero's `pageIndex`. */
  pageIndex: number
  rects: PdfRect[]
  /** Rects of a highlight that continues on the next page. */
  nextPageRects?: PdfRect[]
  /** Ink strokes: each a flat `[x1, y1, x2, y2, …]` list in PDF space. */
  paths?: number[][]
  width?: number
  fontSize?: number
  rotation?: number
  /** The selected text of a highlight or underline. */
  text?: string
  comment?: string
  /** Shown but not editable (another format, or a geometry-less ink). */
  readOnly?: boolean
}

/** What the viewer hands back when the user draws or selects a new markup. */
export interface PdfMarkupDraft {
  type: Exclude<PdfMarkupType, 'ink'>
  color: string
  pageIndex: number
  rects: PdfRect[]
  text?: string
  fontSize?: number
  /** Zotero's `annotationSortIndex`, computed by the viewer from the page text. */
  sortIndex?: string
}

/** A geometry change of a movable markup (note, image, text). */
export interface PdfMarkupGeometryChange {
  id: string
  rects: PdfRect[]
  sortIndex?: string
}

/**
 * The part of pdf.js `PageViewport` the markup layer needs. Tests pass a
 * plain object with the same contract.
 */
export interface MarkupViewport {
  width: number
  height: number
  scale: number
  rotation: number
  /** The page's CropBox `[x1, y1, x2, y2]`, origin included. */
  viewBox: number[]
  convertToViewportRectangle(rect: number[]): number[]
  convertToViewportPoint(x: number, y: number): number[]
  convertToPdfPoint(x: number, y: number): number[]
}

export interface PdfMarkupLabels {
  toolbarAriaLabel: string
  toolSelect: string
  toolHighlight: string
  toolUnderline: string
  toolNote: string
  toolImage: string
  toolText: string
  colorAriaLabel: (color: string) => string
  popoverAriaLabel: string
  popoverHighlight: string
  popoverUnderline: string
  markupAriaLabel: (type: PdfMarkupType, text: string) => string
  deleteMarkup: string
}

export const DEFAULT_MARKUP_LABELS: PdfMarkupLabels = {
  toolbarAriaLabel: 'Annotation tools',
  toolSelect: 'Select',
  toolHighlight: 'Highlight',
  toolUnderline: 'Underline',
  toolNote: 'Note',
  toolImage: 'Area',
  toolText: 'Text',
  colorAriaLabel: (color: string) => `Color ${color}`,
  popoverAriaLabel: 'Annotate selection',
  popoverHighlight: 'Highlight',
  popoverUnderline: 'Underline',
  markupAriaLabel: (type: PdfMarkupType, text: string) => (text ? `${type}: ${text}` : type),
  deleteMarkup: 'Delete annotation',
}

/** Zotero's eight annotation colors, in its order. */
export const ZOTERO_COLORS = [
  '#ffd400',
  '#ff6666',
  '#5fb236',
  '#2ea8e5',
  '#a28ae5',
  '#e56eee',
  '#f19837',
  '#aaaaaa',
] as const

/** Side of the square rect Zotero gives a note, in PDF points. */
export const NOTE_SIZE_PT = 22

/** Default font size of a text annotation, in PDF points. */
export const TEXT_FONT_SIZE_PT = 14
