import { invoke } from '@tauri-apps/api/core'
import type { AssetFileFacts } from './asset-file-facts'
import {
  extractionVersionNeeds,
  extractionVersionVerdict,
  type ExtractionVersionSource,
} from './extraction-version'
import {
  layoutVersionNeeds,
  layoutVersionVerdict,
  type LayoutVersionSource,
} from './layout-version'

/**
 * The versions of an image asset that are on disk: the original and every
 * `<stem>_vN.<ext>` an edit left next to it.
 *
 * Whether a version "corresponds" to the stored layout or text is not decided
 * here. Each version carries the same facts the version check reads of the
 * current file, and the two rules of that check (`layout-version.ts`,
 * `extraction-version.ts`) are asked about each of them: a version is marked
 * exactly when the viewer would draw the layout, or not warn about the text,
 * with that version on screen.
 */

// The family only exists for image assets, and both rules check only those.
const IMAGE_ASSET_TYPE = 'image'

/**
 * Only an image that is its own document has a version family: a PDF is edited
 * through another path, and a page of a PDF belongs to its container.
 */
export function assetHasImageVersions(asset: {
  type: string
  parentAssetId?: string | null
}): boolean {
  return asset.type === IMAGE_ASSET_TYPE && !asset.parentAssetId
}

/** One file of the family, as the backend lists it. */
export interface ImageVersionFile extends AssetFileFacts {
  /** The value to store in `assets.path` to show this file. */
  path: string
  /** `N` of `<stem>_vN`; 0 for the original. */
  version: number
  /** Modification time, milliseconds since the Unix epoch. */
  modifiedMs: number
  /** Size in bytes. */
  size: number
  /** Whether `assets.path` points at this file. */
  current: boolean
}

export interface ImageVersion extends ImageVersionFile {
  /** The stored layout was computed on this file. */
  matchesLayout: boolean
  /** The stored OCR text was read from this file. */
  matchesText: boolean
}

/** The stored rows the versions are compared with; `null` when there is none. */
export interface ImageVersionDerived {
  layout: LayoutVersionSource | null
  extraction: ExtractionVersionSource | null
}

/**
 * Hashing reads every file of the family whole, so it is asked for only when a
 * stored row carries a stamp to compare the hashes with.
 */
export function imageVersionsNeedSha({ layout, extraction }: ImageVersionDerived): boolean {
  return (
    (layout !== null && layoutVersionNeeds(IMAGE_ASSET_TYPE, layout) === 'facts_with_sha') ||
    (extraction !== null &&
      extractionVersionNeeds(IMAGE_ASSET_TYPE, extraction) === 'facts_with_sha')
  )
}

export function markImageVersions(
  files: ImageVersionFile[],
  { layout, extraction }: ImageVersionDerived
): ImageVersion[] {
  // A text with no stamp has an unknown origin: the rule says nothing about
  // it, so no version is said to be its source.
  const textIsChecked =
    extraction !== null && extractionVersionNeeds(IMAGE_ASSET_TYPE, extraction) !== 'nothing'

  return files.map((file) => ({
    ...file,
    matchesLayout:
      layout !== null && layoutVersionVerdict(IMAGE_ASSET_TYPE, layout, file).status === 'matches',
    matchesText:
      textIsChecked && extractionVersionVerdict(IMAGE_ASSET_TYPE, extraction, file) === 'matches',
  }))
}

/**
 * Lists the version family of an image asset, the original first and the
 * versions by number, each marked against the stored layout and text.
 *
 * The backend finds the family from `assets.path`; no path is sent.
 */
export async function listImageVersions(
  assetId: string,
  derived: ImageVersionDerived
): Promise<ImageVersion[]> {
  const files = await invoke<ImageVersionFile[]>('list_image_versions', {
    assetId,
    withSha: imageVersionsNeedSha(derived),
  })
  return markImageVersions(files ?? [], derived)
}
