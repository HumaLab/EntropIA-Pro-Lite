/**
 * Which of an item's assets a person ever sees.
 *
 * # The rule
 *
 * A PDF is imported as one asset per page plus the container it came from, and
 * the pages point at the container through `parentAssetId`. The viewer shows
 * the pages and hides the container, because a container has nothing to
 * display — opening it would show the same document the pages already are.
 *
 * # Why it is a module
 *
 * Because the rule was written inside the viewer, and anything else that had to
 * pick "an asset of this item" had no way to apply it. Filing a note against
 * `findByItem(...)[0]` put it on the container of a scanned legajo: the note was
 * written, the confirmation appeared, and it showed up on none of the pages,
 * because the one asset holding it is the one asset never drawn.
 *
 * A thing attached to something invisible is indistinguishable from a thing
 * that was never saved. So the rule lives here and both callers ask it.
 */

export interface AssetLike {
  id: string
  parentAssetId?: string | null
}

/**
 * The assets that are actually shown, in the order they were given.
 *
 * An asset is hidden when some other asset names it as its parent. Derived from
 * the set rather than from a flag on the row, which is what makes it correct
 * for an item whose PDF was never split: with no children, nothing names it,
 * and the container is shown because it is all there is.
 */
export function visibleAssets<T extends AssetLike>(assets: T[]): T[] {
  const parents = new Set(
    assets.map((asset) => asset.parentAssetId).filter((id): id is string => Boolean(id))
  )
  return assets.filter((asset) => !parents.has(asset.id))
}

/**
 * The first asset of an item that someone could actually open, or null.
 *
 * "First" is the caller's order — `AssetRepo.findByItem` sorts by path — so the
 * answer is the same asset every time rather than whichever the database
 * returned.
 */
export function firstVisibleAsset<T extends AssetLike>(assets: T[]): T | null {
  return visibleAssets(assets)[0] ?? null
}

/**
 * Where in the shown assets a viewer should open, given the asset the item's
 * collection card shows (`AssetRepo.primaryOf`).
 *
 * The card shows a root asset. When that asset is itself shown, the viewer
 * opens on it; when it is a hidden container, on its first page. Without a
 * usable card asset the viewer opens on the first one.
 */
export function openingAssetIndex<T extends AssetLike>(
  shown: T[],
  cardAsset: AssetLike | null
): number {
  if (!cardAsset) return 0
  const index = shown.findIndex(
    (asset) => asset.id === cardAsset.id || asset.parentAssetId === cardAsset.id
  )
  return Math.max(index, 0)
}
