import { invoke } from '@tauri-apps/api/core'
import type { Collection, Item, StoreApi } from '@entropia/store'

/**
 * Routes the app's collection and item writes through the Rust corpus service
 * (`src-tauri/src/corpus`), the same one the assistant endpoint uses, so both
 * share validation, revision fencing and the search refresh
 * (odd/tasks/agent-corpus-tools.md D8).
 *
 * The store keeps its shape: every view and dialog still calls
 * `store.collections.create`, `store.collections.update` and
 * `store.items.update`; only what runs underneath changes. Called once, right
 * after the store is built in `initDb()`.
 */
export function routeCorpusWrites(store: StoreApi): void {
  store.collections.create = (data) =>
    invoke<Collection>('corpus_collection_create', {
      name: data.name,
      description: data.description ?? null,
      id: data.id ?? null,
    })

  store.collections.update = (id, data) => {
    const patch: { name?: string; description?: string | null } = {}
    if (data.name !== undefined) patch.name = data.name
    // An explicit null clears the description; an absent key leaves it.
    if ('description' in data) patch.description = data.description ?? null
    return invoke<Collection>('corpus_collection_update', {
      id,
      patch,
      expectedUpdatedAt: null,
    })
  }

  store.items.update = (id, data) => {
    const patch: { title?: string; metadataJson?: string } = {}
    if (data.title !== undefined) patch.title = data.title
    // The app sends the JSON it built, reserved keys included.
    if (data.metadata !== undefined && data.metadata !== null) patch.metadataJson = data.metadata
    return invoke<Item>('corpus_item_update', { id, patch, expectedUpdatedAt: null })
  }
}
