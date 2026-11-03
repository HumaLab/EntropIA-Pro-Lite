import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  COLLECTIONS_CHANGED_EXTERNALLY_EVENT,
  CORPUS_CHANGED_EVENT,
  startCorpusEventBridge,
} from './corpus-events'
import {
  DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
  DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
} from './document-explorer'
import { workspace } from './workspace'

/**
 * A write an assistant made through the endpoint reaches the views that show
 * collections and items (odd/tasks/agent-corpus-tools.md S6): the backend
 * emits `corpus:changed`, and the bridge turns it into the window events the
 * sidebar and the Colecciones view already listen to.
 */

type Listener = (event: { payload: unknown }) => void
const { listeners } = vi.hoisted(() => ({ listeners: new Map<string, Listener>() }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((name: string, callback: Listener) => {
    listeners.set(name, callback)
    return Promise.resolve(() => listeners.delete(name))
  }),
}))

function recorder(...names: string[]) {
  const seen: Array<{ name: string; detail: unknown }> = []
  const handlers = names.map((name) => {
    const handler = (event: Event) =>
      seen.push({ name, detail: (event as CustomEvent).detail ?? null })
    window.addEventListener(name, handler)
    return () => window.removeEventListener(name, handler)
  })
  return { seen, stop: () => handlers.forEach((remove) => remove()) }
}

describe('startCorpusEventBridge', () => {
  afterEach(() => {
    listeners.clear()
    vi.restoreAllMocks()
  })

  it('tells the sidebar and Colecciones when a collection changed', async () => {
    const stop = await startCorpusEventBridge()
    const { seen, stop: unwatch } = recorder(
      DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
      COLLECTIONS_CHANGED_EXTERNALLY_EVENT,
      DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT
    )
    listeners.get(CORPUS_CHANGED_EVENT)?.({
      payload: { kind: 'collection', id: 'col_1', collectionId: 'col_1' },
    })
    expect(seen.map((event) => event.name)).toEqual([
      DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
      COLLECTIONS_CHANGED_EXTERNALLY_EVENT,
    ])
    unwatch()
    stop()
    expect(listeners.has(CORPUS_CHANGED_EVENT)).toBe(false)
  })

  it('refreshes the collection that holds a changed item', async () => {
    const stop = await startCorpusEventBridge()
    const { seen, stop: unwatch } = recorder(
      DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
      DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT
    )
    listeners.get(CORPUS_CHANGED_EVENT)?.({
      payload: { kind: 'item', id: 'ite_1', collectionId: 'col_1' },
    })
    expect(seen).toEqual([
      {
        name: DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
        detail: { collectionId: 'col_1', itemId: 'ite_1' },
      },
    ])
    unwatch()
    stop()
  })

  it('drops a document an assistant deleted from its collection and from history', async () => {
    const forgetItem = vi.spyOn(workspace, 'forgetItem')
    const stop = await startCorpusEventBridge()
    const { seen, stop: unwatch } = recorder(
      DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
      DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
      COLLECTIONS_CHANGED_EXTERNALLY_EVENT
    )
    listeners.get(CORPUS_CHANGED_EVENT)?.({
      payload: { kind: 'item_deleted', id: 'ite_1', collectionId: 'col_1' },
    })
    // The app's own item delete forgets the open document the same way, so an
    // assistant's approved delete lands on the same screen instead of a row
    // that no longer exists.
    expect(forgetItem).toHaveBeenCalledWith('ite_1')
    // The sidebar refreshes that collection, and Colecciones re-reads its card
    // because its document count changed.
    expect(seen).toEqual([
      {
        name: DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
        detail: { collectionId: 'col_1', itemId: 'ite_1' },
      },
      { name: COLLECTIONS_CHANGED_EXTERNALLY_EVENT, detail: null },
    ])
    unwatch()
    stop()
  })

  it('drops a deleted collection from the lists and from history', async () => {
    const forgetCollection = vi.spyOn(workspace, 'forgetCollection')
    const stop = await startCorpusEventBridge()
    const { seen, stop: unwatch } = recorder(
      DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
      COLLECTIONS_CHANGED_EXTERNALLY_EVENT,
      DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT
    )
    listeners.get(CORPUS_CHANGED_EVENT)?.({
      payload: { kind: 'collection_deleted', id: 'col_1', collectionId: 'col_1' },
    })
    expect(forgetCollection).toHaveBeenCalledWith('col_1')
    expect(seen.map((event) => event.name)).toEqual([
      DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
      COLLECTIONS_CHANGED_EXTERNALLY_EVENT,
    ])
    unwatch()
    stop()
  })

  it('ignores a payload it does not understand', async () => {
    const stop = await startCorpusEventBridge()
    const { seen, stop: unwatch } = recorder(
      DOCUMENT_EXPLORER_COLLECTIONS_CHANGED_EVENT,
      DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT
    )
    listeners.get(CORPUS_CHANGED_EVENT)?.({ payload: { kind: 'item' } })
    listeners.get(CORPUS_CHANGED_EVENT)?.({ payload: null })
    expect(seen).toEqual([])
    unwatch()
    stop()
  })
})
