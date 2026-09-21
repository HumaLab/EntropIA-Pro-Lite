/**
 * The pure arithmetic behind resizing a writingImage (spec, Node View and
 * Resizing). A width below the minimum or outside a positive range is
 * refused — a null return — rather than silently floored: the node view's
 * drag handler is what keeps the visible handle from ever asking for less,
 * this function only has to refuse an out-of-contract request.
 */
export const MIN_WRITING_IMAGE_WIDTH = 80

export function clampWritingImageWidth(
  desiredWidth: number,
  aspectRatio: number,
  availableWidth: number
): { width: number; height: number } | null {
  if (
    !Number.isFinite(desiredWidth) ||
    !Number.isFinite(aspectRatio) ||
    !Number.isFinite(availableWidth)
  ) {
    return null
  }
  if (aspectRatio <= 0 || availableWidth <= 0) return null
  if (desiredWidth <= 0 || desiredWidth < MIN_WRITING_IMAGE_WIDTH) return null

  const width = Math.min(desiredWidth, availableWidth)
  return { width: Math.round(width), height: Math.round(width / aspectRatio) }
}
