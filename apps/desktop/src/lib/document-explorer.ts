export const DOCUMENT_EXPLORER_ASSET_SELECTED_EVENT = 'entropia:document-explorer-asset-selected'
export const DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT =
  'entropia:document-explorer-collection-changed'
export const DOCUMENT_ASSET_DELETED_EVENT = 'entropia:document-asset-deleted'

export interface DocumentExplorerAssetDetail {
  itemId: string
  assetId: string | null
  assetLabel?: string | null
}

export interface DocumentExplorerCollectionChangedDetail {
  collectionId: string
  itemId?: string
}

export interface DocumentAssetDeletedDetail {
  itemId: string
  assetId: string
}
