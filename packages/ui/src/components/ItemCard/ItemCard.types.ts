export type AssetType = 'image' | 'pdf' | 'audio'

export interface ItemCardProps {
  id: string
  title: string
  assetCount: number
  thumbnailPath?: string
  primaryAssetType?: AssetType
  metadataPreview?: string
  /** A short line under the title: why this card is here, when that needs saying. */
  note?: string
  onclick?: () => void
  onDelete?: (e: MouseEvent) => void
  deleteAriaLabel?: string
}
