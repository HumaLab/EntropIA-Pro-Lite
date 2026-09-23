export type AssetType = 'image' | 'pdf' | 'audio'

export interface ItemCardProps {
  id: string
  title: string
  assetCount: number
  /**
   * The count chip, already worded by the caller ("3 páginas"). This package
   * carries no translations; without it the chip falls back to "N assets".
   */
  countLabel?: string
  thumbnailPath?: string
  primaryAssetType?: AssetType
  metadataPreview?: string
  /** A short line under the title: why this card is here, when that needs saying. */
  note?: string
  onclick?: () => void
  onDelete?: (e: MouseEvent) => void
  deleteAriaLabel?: string
}
