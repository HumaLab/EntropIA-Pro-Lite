import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
  notifyDocumentExplorerCollectionChanged,
} from './document-explorer'

/** What the backend emits after an assistant wrote the corpus. */
export const CORPUS_CHANGED_EVENT = 'corpus:changed'

/**
 * The set of collections changed outside the Colecciones view (an assistant
 * created or renamed one). Distinct from the sidebar's own event, which the
 * view itself announces after its writes, so the view does not reload twice.
 */
export const COLLECTIONS_CHANGED_EXTERNALLY_EVENT = 'entropia:collections-changed-externally'

interface CorpusChange {
  kind: 'collection' | 'item'
  id: string
  collectionId: string
}

function isCorpusChange(value: unknown): value is CorpusChange {
  const change = value as Partial<CorpusChange> | null
  return (
    !!change &&
    (change.kind === 'collection' || change.kind === 'item') &&
    typeof change.id === 'string' &&
    typeof change.collectionId === 'string'
  )
}

/**
 * Turn the backend's `corpus:changed` into the window events the sidebar and
 * the Colecciones view already refresh on (odd/tasks/agent-corpus-tools.md S6).
 * An open item view does not reload on it yet: it may hold unsaved edits.
 */
export async function startCorpusEventBridge(): Promise<UnlistenFn> {
  return listen(CORPUS_CHANGED_EVENT, (event) => {
    const change = event.payload
    if (!isCorpusChange(change)) return
    if (change.kind === 'collection') {
      window.dispatchEvent(new CustomEvent(DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT))
      window.dispatchEvent(new CustomEvent(COLLECTIONS_CHANGED_EXTERNALLY_EVENT))
    } else {
      notifyDocumentExplorerCollectionChanged(change.collectionId, change.id)
    }
  })
}
