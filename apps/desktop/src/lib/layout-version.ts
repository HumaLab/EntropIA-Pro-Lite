import type { Asset, AssetLayout } from '@entropia/store'
import { assetFileFacts, type AssetFileFacts } from './asset-file-facts'
import type { OcrMode } from './ocr'

/**
 * Does a stored layout belong to the file its asset shows right now?
 *
 * The one rule, for every reader of a layout (the viewer, the export; the
 * per-block re-OCR applies the same rule in Rust, `derived_source::layout_version`):
 *
 * - Only image assets are checked. A PDF asset is never rejected and its file
 *   is never read for this.
 * - A layout stamped with `sourceSha256` belongs to the file with that hash.
 * - A layout without a stamp predates the stamps. It belongs to the file only
 *   when the file carries no EXIF orientation (such a layout was computed in the
 *   sideways system of a tagged photo) and has the size the layout was computed on.
 *
 * `layout-version.cases.json` is the case table both implementations are tested
 * against.
 */

/** The columns of a `layouts` row the rule reads. */
export type LayoutVersionSource = Pick<AssetLayout, 'sourceSha256' | 'imageWidth' | 'imageHeight'>

export interface PixelSize {
  width: number
  height: number
}

export type LayoutVersionVerdict =
  | { status: 'matches' }
  | {
      status: 'other_version'
      /** The size the layout was computed on. */
      layout: PixelSize
      /** The size of the current file, `null` when it has none to report. */
      current: PixelSize | null
    }

/** What has to be read from the current file to decide. */
export type LayoutVersionNeeds = 'nothing' | 'facts' | 'facts_with_sha'

const IMAGE_ASSET_TYPE = 'image'

function hasStamp(layout: LayoutVersionSource): layout is LayoutVersionSource & {
  sourceSha256: string
} {
  return typeof layout.sourceSha256 === 'string' && layout.sourceSha256.length > 0
}

/**
 * The hash reads the whole file, so it is asked for only when the layout has
 * one to compare with.
 */
export function layoutVersionNeeds(
  assetType: string,
  layout: LayoutVersionSource
): LayoutVersionNeeds {
  if (assetType !== IMAGE_ASSET_TYPE) return 'nothing'
  return hasStamp(layout) ? 'facts_with_sha' : 'facts'
}

/**
 * The rule itself. `facts` are those of the asset's current file; they are not
 * looked at for anything that is not an image asset. An image layout with no
 * facts to compare with is not vouched for.
 */
export function layoutVersionVerdict(
  assetType: string,
  layout: LayoutVersionSource,
  facts: AssetFileFacts | null
): LayoutVersionVerdict {
  if (assetType !== IMAGE_ASSET_TYPE) return { status: 'matches' }

  const current =
    facts && facts.width !== null && facts.height !== null
      ? { width: facts.width, height: facts.height }
      : null
  const matches = hasStamp(layout)
    ? facts?.sha256 === layout.sourceSha256
    : facts?.orientation === 1 &&
      current !== null &&
      current.width === layout.imageWidth &&
      current.height === layout.imageHeight

  if (matches) return { status: 'matches' }
  return {
    status: 'other_version',
    layout: { width: layout.imageWidth, height: layout.imageHeight },
    current,
  }
}

/** `layouts.model` of a layout computed by GLM-OCR (the OCRH action). */
const GLM_OCR_LAYOUT_MODEL = 'glm_ocr'

/**
 * Which OCR action computes the layout of an image again: the one that computed
 * the stale layout. A layout from the local engine (OCRL) is run again there
 * when this build has it; every other case goes to GLM-OCR (OCRH), the engine
 * every build has.
 */
export function layoutRerunMode(layoutModel: string, localOcrAvailable: boolean): OcrMode {
  return localOcrAvailable && layoutModel !== GLM_OCR_LAYOUT_MODEL ? 'light' : 'high'
}

/** `unknown`: the current file could not be read, so nothing can be said. */
export type LayoutVersionCheck = LayoutVersionVerdict | { status: 'unknown' }

/**
 * Applies the rule to an asset, reading from its current file only what the
 * rule needs. Never throws: a file that cannot be read answers `unknown`, and
 * each reader decides what to do with a layout nobody can vouch for.
 */
export async function checkLayoutVersion(
  asset: Pick<Asset, 'id' | 'type'>,
  layout: LayoutVersionSource
): Promise<LayoutVersionCheck> {
  const needs = layoutVersionNeeds(asset.type, layout)
  if (needs === 'nothing') return layoutVersionVerdict(asset.type, layout, null)

  let facts: AssetFileFacts | null
  try {
    facts = (await assetFileFacts(asset.id, { withSha: needs === 'facts_with_sha' })) ?? null
  } catch {
    facts = null
  }
  if (!facts) return { status: 'unknown' }
  return layoutVersionVerdict(asset.type, layout, facts)
}
