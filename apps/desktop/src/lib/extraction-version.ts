import type { Asset, Extraction } from '@entropia/store'
import { assetFileFacts, type AssetFileFacts } from './asset-file-facts'

/**
 * Was a stored OCR text read from the file its asset shows right now?
 *
 * The sibling of the layout rule (`layout-version.ts`), and deliberately
 * narrower, because a text is kept and only warned about:
 *
 * - Only image assets are checked. A PDF asset is never warned about and its
 *   file is never read for this.
 * - A text stamped with `sourceSha256` belongs to the file with that hash.
 * - A text without a stamp has an unknown origin: nothing is said about it and
 *   nothing is read.
 */

/** The column of an `extractions` row the rule reads. */
export type ExtractionVersionSource = Pick<Extraction, 'sourceSha256'>

export type ExtractionVersionVerdict = 'matches' | 'other_version'

/** What has to be read from the current file to decide. */
export type ExtractionVersionNeeds = 'nothing' | 'facts_with_sha'

const IMAGE_ASSET_TYPE = 'image'

function isChecked(assetType: string, extraction: ExtractionVersionSource): boolean {
  return (
    assetType === IMAGE_ASSET_TYPE &&
    typeof extraction.sourceSha256 === 'string' &&
    extraction.sourceSha256.length > 0
  )
}

/** The hash reads the whole file, so it is asked for only when there is a stamp. */
export function extractionVersionNeeds(
  assetType: string,
  extraction: ExtractionVersionSource
): ExtractionVersionNeeds {
  return isChecked(assetType, extraction) ? 'facts_with_sha' : 'nothing'
}

/** The rule itself. `facts` are those of the asset's current file. */
export function extractionVersionVerdict(
  assetType: string,
  extraction: ExtractionVersionSource,
  facts: AssetFileFacts | null
): ExtractionVersionVerdict {
  if (!isChecked(assetType, extraction)) return 'matches'
  return facts?.sha256 === extraction.sourceSha256 ? 'matches' : 'other_version'
}

/** `unknown`: the current file could not be read, so nothing can be said. */
export type ExtractionVersionCheck = ExtractionVersionVerdict | 'unknown'

/**
 * Applies the rule to an asset, hashing its current file only when the text is
 * stamped. Never throws: a file that cannot be read answers `unknown`.
 */
export async function checkExtractionVersion(
  asset: Pick<Asset, 'id' | 'type'>,
  extraction: ExtractionVersionSource
): Promise<ExtractionVersionCheck> {
  if (extractionVersionNeeds(asset.type, extraction) === 'nothing') return 'matches'

  let facts: AssetFileFacts | null
  try {
    facts = (await assetFileFacts(asset.id, { withSha: true })) ?? null
  } catch {
    facts = null
  }
  if (!facts) return 'unknown'
  return extractionVersionVerdict(asset.type, extraction, facts)
}
