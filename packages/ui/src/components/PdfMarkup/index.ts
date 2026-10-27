export { default as PdfMarkupLayer } from './PdfMarkupLayer.svelte'
export { default as MarkupToolbar } from './MarkupToolbar.svelte'
export {
  DEFAULT_MARKUP_LABELS,
  MARKUP_TOOLS,
  NOTE_SIZE_PT,
  TEXT_FONT_SIZE_PT,
  ZOTERO_COLORS,
} from './types'
export type {
  MarkupTool,
  MarkupViewport,
  PdfMarkup,
  PdfMarkupDraft,
  PdfMarkupGeometryChange,
  PdfMarkupLabels,
  PdfMarkupType,
  PdfPoint,
  PdfRect,
} from './types'
export {
  boundingPdfRect,
  createMarkupViewport,
  movePdfRect,
  noteRectAt,
  pageIndexToPage,
  pageToPageIndex,
  resizePdfRect,
  selectionToPdfRects,
  toPdfPoint,
  toPdfRect,
  toViewportBox,
} from './geometry'
export type { ViewportBox } from './geometry'
export {
  charsFromTextItems,
  charsFromTextLayer,
  rectsDist,
  formatSortIndex,
  getClosestOffset,
  getSortIndex,
  getSortIndexRect,
} from './sortIndex'
export type { PageChar, SortPosition, TextItemLike, ViewportToPdf } from './sortIndex'
export { THUMBNAIL_MAX_PX, ThumbnailRenderer, renderThumbnail, thumbnailRegion } from './thumbnails'
export type { ThumbnailDocument, ThumbnailPage, ThumbnailRequest } from './thumbnails'
export { TextLayerController, sameTextLayerIdentity } from './textLayerController'
export type { TextLayerHandle, TextLayerIdentity } from './textLayerController'
