export const DOCUMENT_EXPLORER_ASSET_SELECTED_EVENT = 'entropia:document-explorer-asset-selected'
export const DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT =
  'entropia:document-explorer-collection-changed'
/** The set of collections itself changed — one was created, renamed, or
 *  deleted. The sidebar tree owns its own copy of that list, so it has to be
 *  told; the per-collection event below only refreshes a collection's items. */
export const DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT =
  'entropia:document-explorer-collections-changed'
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
