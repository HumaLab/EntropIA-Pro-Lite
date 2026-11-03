import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { StoreApi } from '@entropia/store'
import { routeCorpusWrites } from './corpus'

/**
 * The app's collection and item writes go through the Rust corpus service
 * (odd/tasks/agent-corpus-tools.md D8). The store keeps its shape, so every
 * view keeps calling `store.collections.create` and friends; only the path
 * underneath changes. These tests pin the IPC payloads.
 */

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

function fakeStore() {
  const sqlCreate = vi.fn()
  const sqlCollectionUpdate = vi.fn()
  const sqlItemUpdate = vi.fn()
  const sqlCollectionDelete = vi.fn()
  const sqlItemDelete = vi.fn()
  const forgetVocabulary = vi.fn()
  const store = {
    collections: {
      create: sqlCreate,
      update: sqlCollectionUpdate,
      delete: sqlCollectionDelete,
    },
    items: { update: sqlItemUpdate, deleteWithCascade: sqlItemDelete },
    fts: { forgetVocabulary },
  } as unknown as StoreApi
  return {
    store,
    sqlCreate,
    sqlCollectionUpdate,
    sqlItemUpdate,
    sqlCollectionDelete,
    sqlItemDelete,
    forgetVocabulary,
  }
}

const COLLECTION = {
  id: 'col_1',
  name: 'Cartas',
  description: null,
  createdAt: 1,
  updatedAt: 1,
}

const ITEM = {
  id: 'ite_1',
  title: 'Carta',
  collectionId: 'col_1',
  metadata: '{"autor":"Sarmiento"}',
  createdAt: 1,
  updatedAt: 2,
}

describe('routeCorpusWrites', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('creates a collection through the backend, keeping an id the app chose', async () => {
    const { store, sqlCreate } = fakeStore()
    routeCorpusWrites(store)
    invokeMock.mockResolvedValue(COLLECTION)

    await expect(store.collections.create({ name: 'Cartas' })).resolves.toEqual(COLLECTION)
    expect(invokeMock).toHaveBeenLastCalledWith('corpus_collection_create', {
      name: 'Cartas',
      description: null,
      id: null,
    })

    await store.collections.create({ id: 'sample', name: 'Ejemplo', description: 'Demo' })
    expect(invokeMock).toHaveBeenLastCalledWith('corpus_collection_create', {
      name: 'Ejemplo',
      description: 'Demo',
      id: 'sample',
    })
    expect(sqlCreate).not.toHaveBeenCalled()
  })

  it('sends only the collection fields the caller changed', async () => {
    const { store, sqlCollectionUpdate } = fakeStore()
    routeCorpusWrites(store)
    invokeMock.mockResolvedValue(COLLECTION)

    await store.collections.update('col_1', { name: 'Nuevo nombre' })
    expect(invokeMock).toHaveBeenLastCalledWith('corpus_collection_update', {
      id: 'col_1',
      patch: { name: 'Nuevo nombre' },
      expectedUpdatedAt: null,
    })

    await store.collections.update('col_1', { name: 'X', description: null })
    expect(invokeMock).toHaveBeenLastCalledWith('corpus_collection_update', {
      id: 'col_1',
      patch: { name: 'X', description: null },
      expectedUpdatedAt: null,
    })
    expect(sqlCollectionUpdate).not.toHaveBeenCalled()
  })

  it('updates an item with the metadata JSON the app built', async () => {
    const { store, sqlItemUpdate } = fakeStore()
    routeCorpusWrites(store)
    invokeMock.mockResolvedValue(ITEM)

    await expect(
      store.items.update('ite_1', { metadata: '{"autor":"Sarmiento"}' })
    ).resolves.toEqual(ITEM)
    expect(invokeMock).toHaveBeenLastCalledWith('corpus_item_update', {
      id: 'ite_1',
      patch: { metadataJson: '{"autor":"Sarmiento"}' },
      expectedUpdatedAt: null,
    })

    await store.items.update('ite_1', { title: 'Retitulado' })
    expect(invokeMock).toHaveBeenLastCalledWith('corpus_item_update', {
      id: 'ite_1',
      patch: { title: 'Retitulado' },
      expectedUpdatedAt: null,
    })
    expect(sqlItemUpdate).not.toHaveBeenCalled()
  })

  it('passes a backend refusal to the caller', async () => {
    const { store } = fakeStore()
    routeCorpusWrites(store)
    invokeMock.mockRejectedValue(
      'invalid_argument: the collection name must be 1 to 200 characters'
    )
    await expect(store.collections.create({ name: '   ' })).rejects.toBe(
      'invalid_argument: the collection name must be 1 to 200 characters'
    )
  })

  it('deletes an item through the backend delete service', async () => {
    const { store, sqlItemDelete, forgetVocabulary } = fakeStore()
    routeCorpusWrites(store)
    invokeMock.mockResolvedValue(undefined)

    await expect(store.items.deleteWithCascade('ite_1')).resolves.toBeUndefined()
    expect(invokeMock).toHaveBeenLastCalledWith('corpus_items_delete', {
      itemIds: ['ite_1'],
    })
    expect(sqlItemDelete).not.toHaveBeenCalled()
    expect(forgetVocabulary).toHaveBeenCalledTimes(1)
  })

  it('deletes a collection through the backend delete service', async () => {
    const { store, sqlCollectionDelete, forgetVocabulary } = fakeStore()
    routeCorpusWrites(store)
    invokeMock.mockResolvedValue(undefined)

    await expect(store.collections.delete('col_1')).resolves.toBeUndefined()
    expect(invokeMock).toHaveBeenLastCalledWith('corpus_collection_delete', {
      collectionId: 'col_1',
    })
    expect(sqlCollectionDelete).not.toHaveBeenCalled()
    expect(forgetVocabulary).toHaveBeenCalledTimes(1)
  })

  it('keeps the renderer search vocabulary when a delete fails', async () => {
    const { store, forgetVocabulary } = fakeStore()
    routeCorpusWrites(store)
    invokeMock.mockRejectedValue('not_found: no item ite_missing')

    await expect(store.items.deleteWithCascade('ite_missing')).rejects.toBe(
      'not_found: no item ite_missing'
    )
    expect(forgetVocabulary).not.toHaveBeenCalled()
  })

  it('passes a backend delete refusal to the caller', async () => {
    const { store } = fakeStore()
    routeCorpusWrites(store)
    invokeMock.mockRejectedValue('not_found: no item ite_missing')

    await expect(store.items.deleteWithCascade('ite_missing')).rejects.toBe(
      'not_found: no item ite_missing'
    )
  })
})
