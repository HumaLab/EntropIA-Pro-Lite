import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { WritingStore, type WritingDocumentRow } from './writing'
import { DEFAULT_SCHEDULER } from './writing-scheduler'

const CONTENT = { schemaVersion: 1, doc: { type: 'doc', content: [{ type: 'paragraph' }] } }

const ROW: WritingDocumentRow = {
  id: 'd1',
  title: 'Articulo',
  document_type: 'article',
  status: 'active',
  schema_version: 1,
  current_content_json: JSON.stringify(CONTENT),
  revision: 3,
  created_at: 1,
  updated_at: 1,
}

const mockInvoke = vi.mocked(invoke)

/** A store with a clock we drive, so no test waits on a real timer. */
function makeStore(now = { value: 0 }) {
  return { store: new WritingStore(DEFAULT_SCHEDULER, () => now.value), now }
}

beforeEach(() => {
  mockInvoke.mockReset()
  mockInvoke.mockImplementation(async (command: string) => {
    if (command === 'writing_is_ready') return true as never
    if (command === 'writing_load_document') return ROW as never
    if (command === 'writing_list_documents') return [ROW] as never
    if (command === 'writing_save_document') return 4 as never
    if (command === 'writing_append_journal') return 1 as never
    return undefined as never
  })
})

describe('writing store — opening', () => {
  it('holds the document and its revision once it parses', async () => {
    const { store } = makeStore()
    await store.openDocument('d1')

    expect(store.snapshot.open?.id).toBe('d1')
    expect(store.snapshot.content).toEqual(CONTENT)
    expect(store.snapshot.revision).toBe(3)
    expect(store.snapshot.status).toBe('saved')
  })

  it('refuses content it cannot mount, and keeps no document to autosave', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'writing_load_document') {
        return {
          ...ROW,
          current_content_json: JSON.stringify({
            schemaVersion: 1,
            doc: { type: 'doc', content: [{ type: 'zoteroCitation' }] },
          }),
        } as never
      }
      return true as never
    })

    const { store } = makeStore()
    await store.openDocument('d1')

    expect(store.snapshot.refusal?.code).toBe('unknown-node')
    expect(store.snapshot.content).toBeNull()
    expect(store.snapshot.status).toBe('error')
  })
})

/**
 * The bug this guards: the store is a module singleton, so the section's view
 * can remount with a document still open. ProseMirror reports its own mount
 * normalisation as an update, and without this the store took that for typing
 * and autosave earned a revision for it — the number climbed on every visit.
 */
describe('writing store — an edit that changes nothing is not an edit', () => {
  it('ignores content identical to what it already holds', async () => {
    const { store } = makeStore()
    await store.openDocument('d1')

    store.applyEdit(structuredClone(CONTENT))

    expect(store.snapshot.status).toBe('saved')
    expect(store.snapshot.revision).toBe(3)
  })

  it('still accepts a genuine change', async () => {
    const { store } = makeStore()
    await store.openDocument('d1')

    store.applyEdit({
      schemaVersion: 1,
      doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Hola' }] }] },
    })

    expect(store.snapshot.status).toBe('pending')
  })

  it('ignores edits once the document was refused', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'writing_load_document') {
        return { ...ROW, current_content_json: '{"schemaVersion":1,"doc":' } as never
      }
      return true as never
    })
    const { store } = makeStore()
    await store.openDocument('d1')

    store.applyEdit({ schemaVersion: 1, doc: { type: 'doc' } })

    expect(store.snapshot.status).toBe('error')
  })
})

describe('writing store — closing', () => {
  it('clears the open document so a remount shows the list', async () => {
    const { store } = makeStore()
    await store.openDocument('d1')
    expect(store.snapshot.open).not.toBeNull()

    store.closeDocument()

    expect(store.snapshot.open).toBeNull()
    expect(store.snapshot.content).toBeNull()
    expect(store.snapshot.revision).toBe(0)
  })

  it('ignores edits after closing', async () => {
    const { store } = makeStore()
    await store.openDocument('d1')
    store.closeDocument()

    store.applyEdit({ schemaVersion: 1, doc: { type: 'doc' } })

    expect(store.snapshot.status).toBe('saved')
  })
})

describe('writing store — saving', () => {
  it('confirms Guardado only against the revision persistence returned', async () => {
    const { store, now } = makeStore()
    await store.openDocument('d1')

    store.applyEdit({
      schemaVersion: 1,
      doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'x' }] }] },
    })
    expect(store.snapshot.status).toBe('pending')

    now.value = DEFAULT_SCHEDULER.saveDebounceMs
    await store.tick()

    expect(store.snapshot.status).toBe('saved')
    expect(store.snapshot.revision).toBe(4)
    store.dispose()
  })

  it('keeps a save failure visible instead of letting it disappear', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'writing_is_ready') return true as never
      if (command === 'writing_load_document') return ROW as never
      if (command === 'writing_save_document') {
        throw { code: 'revision_conflict', message: 'another window won' }
      }
      return undefined as never
    })

    const { store, now } = makeStore()
    await store.openDocument('d1')
    store.applyEdit({
      schemaVersion: 1,
      doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'x' }] }] },
    })

    now.value = DEFAULT_SCHEDULER.saveDebounceMs
    await store.tick()

    expect(store.snapshot.status).toBe('error')
    expect(store.snapshot.error?.code).toBe('revision_conflict')
    store.dispose()
  })
})

describe('writing store — renaming', () => {
  it('updates the open document and the list without touching the revision', async () => {
    const { store } = makeStore()
    await store.openDocument('d1')
    await store.listDocuments()

    await store.renameDocument('d1', '  La sociedad de los molineros  ')

    expect(store.snapshot.open?.title).toBe('La sociedad de los molineros')
    expect(store.snapshot.documents[0]?.title).toBe('La sociedad de los molineros')
    expect(store.snapshot.revision).toBe(3)
    expect(mockInvoke).toHaveBeenCalledWith('writing_rename_document', {
      id: 'd1',
      title: 'La sociedad de los molineros',
    })
  })

  it('ignores a title that is only whitespace', async () => {
    const { store } = makeStore()
    await store.openDocument('d1')
    mockInvoke.mockClear()

    await store.renameDocument('d1', '   ')

    expect(mockInvoke).not.toHaveBeenCalled()
    expect(store.snapshot.open?.title).toBe('Articulo')
  })
})
