import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
  notifyDocumentExplorerCollectionChanged,
} from './document-explorer'
import { workspace } from './workspace'

/** What the backend emits after an assistant wrote the corpus. */
export const CORPUS_CHANGED_EVENT = 'corpus:changed'

/**
 * The set of collections changed outside the Colecciones view (an assistant
 * created or renamed one, or deleted documents from one). Distinct from the
 * sidebar's own event, which the view itself announces after its writes, so
 * the view does not reload twice.
 */
export const COLLECTIONS_CHANGED_EXTERNALLY_EVENT = 'entropia:collections-changed-externally'

/**
 * `item` and `collection` are writes to a record that still exists; the
 * `_deleted` kinds are an approved delete, which also takes the record's
 * screen away.
 */
type CorpusChangeKind = 'collection' | 'item' | 'collection_deleted' | 'item_deleted'

interface CorpusChange {
  kind: CorpusChangeKind
  id: string
  collectionId: string
}

const CORPUS_CHANGE_KINDS: CorpusChangeKind[] = [
  'collection',
  'item',
  'collection_deleted',
  'item_deleted',
]

function isCorpusChange(value: unknown): value is CorpusChange {
  const change = value as Partial<CorpusChange> | null
  return (
    !!change &&
    CORPUS_CHANGE_KINDS.includes(change.kind as CorpusChangeKind) &&
    typeof change.id === 'string' &&
    typeof change.collectionId === 'string'
  )
}

/**
 * Turn the backend's `corpus:changed` into the window events the sidebar and
 * the Colecciones view already refresh on (odd/tasks/agent-corpus-tools.md S6).
 * A delete also forgets the deleted subject in every tab, exactly as the app's
 * own item and collection deletes do (`workspace.forgetItem`,
 * `forgetCollection`), so an open document or collection whose record an
 * assistant deleted lands on the previous screen instead of on a missing row.
 * An open item view does not reload on a mere `item` write yet: it may hold
 * unsaved edits.
 */
export async function startCorpusEventBridge(): Promise<UnlistenFn> {
  return listen(CORPUS_CHANGED_EVENT, (event) => {
    const change = event.payload
    if (!isCorpusChange(change)) return
    switch (change.kind) {
      case 'collection':
        window.dispatchEvent(new CustomEvent(DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT))
        window.dispatchEvent(new CustomEvent(COLLECTIONS_CHANGED_EXTERNALLY_EVENT))
        break
      case 'collection_deleted':
        // Forget first: the views that showed it unmount, so the refresh the
        // list events cause never races a screen for a record that is gone.
        workspace.forgetCollection(change.collectionId)
        window.dispatchEvent(new CustomEvent(DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT))
        window.dispatchEvent(new CustomEvent(COLLECTIONS_CHANGED_EXTERNALLY_EVENT))
        break
      case 'item_deleted':
        workspace.forgetItem(change.id)
        notifyDocumentExplorerCollectionChanged(change.collectionId, change.id)
        // Colecciones shows each collection's document count, which changed too.
        window.dispatchEvent(new CustomEvent(COLLECTIONS_CHANGED_EXTERNALLY_EVENT))
        break
      case 'item':
        notifyDocumentExplorerCollectionChanged(change.collectionId, change.id)
        break
    }
  })
}
