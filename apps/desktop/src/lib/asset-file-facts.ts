import { invoke } from '@tauri-apps/api/core'

/** What the current file of an asset is, as the backend reads it. */
export interface AssetFileFacts {
  /** EXIF orientation, 1 to 8. 1 for a file with no tag and for anything that is not an image. */
  orientation: number
  /** Pixel size as the image is drawn, with the orientation applied. `null` when the file is not an image. */
  width: number | null
  height: number | null
  /** SHA-256 of the file, lowercase hex. `null` unless it was asked for. */
  sha256: string | null
}

/**
 * Reads the facts of the file `assets.path` names for this asset right now.
 *
 * The hash reads the whole file, so it is off unless `withSha` asks for it: pass
 * it only when there is a `source_sha256` to compare with. The backend remembers
 * each hash while the file keeps its size and modification time.
 */
export async function assetFileFacts(
  assetId: string,
  options: { withSha?: boolean } = {}
): Promise<AssetFileFacts> {
  return invoke<AssetFileFacts>('asset_file_facts', {
    assetId,
    withSha: options.withSha ?? false,
  })
}
