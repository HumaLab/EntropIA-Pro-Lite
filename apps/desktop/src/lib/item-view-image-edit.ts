import type { ViewerAnnotation } from '@entropia/ui'

export interface ImageEditUndoEntry {
  path: string
  page: number
  width: number
  height: number
  annotations: ViewerAnnotation[]
}

export function cloneViewerAnnotations(annotations: ViewerAnnotation[]): ViewerAnnotation[] {
  return annotations.map((annotation) => ({ ...annotation }))
}

export function createImageEditUndoEntry({
  path,
  page = 1,
  width,
  height,
  annotations,
}: {
  path: string
  page?: number
  width: number
  height: number
  annotations: ViewerAnnotation[]
}): ImageEditUndoEntry {
  return {
    path,
    page,
    width,
    height,
    annotations: cloneViewerAnnotations(annotations),
  }
}

export function appendImageEditUndoEntry(
  undoStack: ImageEditUndoEntry[],
  entry: ImageEditUndoEntry
): ImageEditUndoEntry[] {
  return [...undoStack, entry]
}

export function discardLatestImageEditUndoEntry(
  undoStack: ImageEditUndoEntry[]
): ImageEditUndoEntry[] {
  return undoStack.slice(0, -1)
}

export function getLatestImageEditUndoEntry(
  undoStack: ImageEditUndoEntry[]
): ImageEditUndoEntry | null {
  return undoStack[undoStack.length - 1] ?? null
}

export function updateAssetPathInList<TAsset extends { id: string; path: string }>(
  assets: TAsset[],
  assetId: string,
  path: string
): TAsset[] {
  return assets.map((asset) => (asset.id === assetId ? { ...asset, path } : asset))
}

/** What `asset:image-updated` carries to every view that shows the asset. */
export interface ImageUpdatedPayload {
  itemId: string
  assetId: string
  path: string
  /**
   * The view that made the edit. Tauri hands an event to every listener, its
   * emitter included, and that view already holds the new state.
   */
  sourceId?: string
}

export function createImageUpdatedPayload({
  itemId,
  assetId,
  path,
  sourceId,
}: ImageUpdatedPayload): ImageUpdatedPayload {
  return { itemId, assetId, path, sourceId }
}

/**
 * Whether a view must reload after an `asset:image-updated`: the edit was made
 * by another view, on an asset this one shows.
 */
export function isImageUpdateFromAnotherView(
  payload: ImageUpdatedPayload,
  view: { sourceId: string; assetIds: readonly string[] }
): boolean {
  return payload.sourceId !== view.sourceId && view.assetIds.includes(payload.assetId)
}
