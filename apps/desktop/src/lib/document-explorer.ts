import { navigation } from './navigation'

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

/**
 * Tell the document explorer sidebar that a collection's items changed
 * (an import added documents, for instance). Shared by every caller that
 * imports into a collection — `CollectionView`'s own toolbar/drop import and
 * the "Importar fuentes" dialog on Inicio — so the sidebar refresh stays
 * identical regardless of where the import started.
 */
export function notifyDocumentExplorerCollectionChanged(
  collectionId: string,
  itemId?: string
): void {
  window.dispatchEvent(
    new CustomEvent<DocumentExplorerCollectionChangedDetail>(
      DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
      { detail: { collectionId, itemId } }
    )
  )
}

/** Tells CollectionsView to open its own create-collection form (T5). */
export const CREATE_COLLECTION_EVENT = 'entropia:create-collection'

/**
 * Opens Colecciones with its create-collection form already open — the exact
 * flow the sidebar's own "new collection" button uses (AppShell), reused by
 * Inicio's first-run "Crear colección" action so both open the identical
 * form the identical way instead of each carrying its own copy of the wiring.
 *
 * `alreadyOnCollections` is the caller's call: switching sections needs a
 * tick for CollectionsView to mount and attach its listener before the event
 * fires; already being there does not, so the event goes out immediately.
 */
export function requestCreateCollection(alreadyOnCollections: boolean): void {
  if (alreadyOnCollections) {
    window.dispatchEvent(new CustomEvent(CREATE_COLLECTION_EVENT))
    return
  }
  navigation.navigate({ name: 'collections' })
  setTimeout(() => {
    window.dispatchEvent(new CustomEvent(CREATE_COLLECTION_EVENT))
  }, 200)
}
