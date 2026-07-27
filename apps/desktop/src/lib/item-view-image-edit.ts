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

export function createImageUpdatedPayload({
  itemId,
  assetId,
  path,
}: {
  itemId: string
  assetId: string
  path: string
}) {
  return { itemId, assetId, path }
}
