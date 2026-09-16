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

/**
 * Removing a document is reversible on purpose. The schema's `status` column
 * already carries `trashed`, and the list only ever asks for `active`, so the
 * document leaves the workspace with its manuscript, versions and journal
 * intact — nothing in the database is destroyed by a click in the list.
 */
describe('writing store - discarding a document', () => {
  it('moves it to the trash and drops it from the list', async () => {
    const { store } = makeStore()
    await store.listDocuments()
    expect(store.snapshot.documents).toHaveLength(1)

    await store.trashDocument('d1')

    expect(mockInvoke).toHaveBeenCalledWith('writing_set_status', {
      id: 'd1',
      status: 'trashed',
    })
    expect(store.snapshot.documents).toHaveLength(0)
    expect(store.snapshot.error).toBeNull()
  })

  it('keeps the document listed when the command fails', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'writing_list_documents') return [ROW] as never
      if (command === 'writing_set_status') throw { code: 'document_not_found', message: 'gone' }
      return undefined as never
    })
    const { store } = makeStore()
    await store.listDocuments()

    await store.trashDocument('d1')

    expect(store.snapshot.documents).toHaveLength(1)
    expect(store.snapshot.error?.code).toBe('document_not_found')
  })

  /**
   * Discarding the document being edited has to close it first: the autosave
   * loop holds the open document, and a pending write landing after the status
   * change would put it back in front of the writer.
   */
  it('closes the document first when it is the one open', async () => {
    const { store } = makeStore()
    await store.openDocument('d1')
    await store.listDocuments()

    await store.trashDocument('d1')

    expect(store.snapshot.open).toBeNull()
    expect(store.snapshot.content).toBeNull()
    store.dispose()
  })
})

/**
 * A failed save must not be a dead end (plan-editor.md 16.2). The content is
 * still in memory, so the work is not lost — but without a way to try again the
 * writer can only provoke another attempt by typing more, which is not a
 * recovery, it is a superstition.
 */
describe('writing store - retrying a failed save', () => {
  async function failedSave() {
    let attempt = 0
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'writing_load_document') return ROW as never
      if (command === 'writing_save_document') {
        attempt += 1
        if (attempt === 1) throw { code: 'sql_error', message: 'database is locked' }
        return 4 as never
      }
      if (command === 'writing_append_journal') return 1 as never
      return undefined as never
    })
    const { store, now } = makeStore()
    await store.openDocument('d1')
    store.applyEdit({
      schemaVersion: 1,
      doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'x' }] }] },
    })
    now.value += 5_000
    await store.flush()
    return { store, attempts: () => attempt }
  }

  it('leaves the failure visible with the content still held', async () => {
    const { store } = await failedSave()

    expect(store.snapshot.status).toBe('error')
    expect(store.snapshot.error?.code).toBe('sql_error')
    expect(store.snapshot.content).not.toBeNull()
    expect(store.canRetrySave).toBe(true)
    store.dispose()
  })

  it('saves on the second attempt and clears the error', async () => {
    const { store } = await failedSave()

    await store.retrySave()

    expect(store.snapshot.status).toBe('saved')
    expect(store.snapshot.revision).toBe(4)
    expect(store.snapshot.error).toBeNull()
    store.dispose()
  })

  /**
   * A conflict is not a transient failure: another window advanced the
   * revision, so the same expected revision can only fail again, and forcing it
   * through would overwrite work this store never saw. Retry is withheld rather
   * than offered as a button that cannot succeed.
   */
  it('withholds retry when another window won the revision', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'writing_load_document') return ROW as never
      if (command === 'writing_save_document') {
        throw { code: 'revision_conflict', message: 'expected 3' }
      }
      if (command === 'writing_append_journal') return 1 as never
      return undefined as never
    })
    const { store, now } = makeStore()
    await store.openDocument('d1')
    store.applyEdit({
      schemaVersion: 1,
      doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'y' }] }] },
    })
    now.value += 5_000
    await store.flush()

    expect(store.snapshot.error?.code).toBe('revision_conflict')
    expect(store.canRetrySave).toBe(false)
    store.dispose()
  })

  it('does nothing when there is no failure to retry', async () => {
    const { store } = makeStore()
    await store.openDocument('d1')

    await store.retrySave()

    expect(mockInvoke).not.toHaveBeenCalledWith('writing_save_document', expect.anything())
    store.dispose()
  })
})

/**
 * The citation projection travels with the save it belongs to.
 *
 * `save_document` replaces the document's citation rows inside the same
 * transaction that writes the content, so what the store sends has to be
 * derived from the content it is sending — not from a list kept alongside it
 * that some operation forgot to patch.
 */
describe('writing store - the citation projection', () => {
  const WITH_CITATION = {
    schemaVersion: 1,
    doc: {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            { type: 'text', text: 'segun ' },
            {
              type: 'documentCitation',
              attrs: { citationNodeId: 'c1', assetId: 'as1', pageNumber: 12 },
            },
          ],
        },
      ],
    },
  }

  it('sends the rows derived from the content it is saving', async () => {
    const { store, now } = makeStore()
    await store.openDocument('d1')
    store.applyEdit(WITH_CITATION)
    now.value += 5_000
    await store.flush()

    const save = mockInvoke.mock.calls.find(([command]) => command === 'writing_save_document')
    const citations = (save?.[1] as { save: { citations: unknown[] } }).save.citations
    expect(citations).toEqual([
      {
        id: 'c1',
        citation_node_id: 'c1',
        collection_id: null,
        item_id: null,
        asset_id: 'as1',
        page_number: 12,
        start_char: null,
        end_char: null,
        quoted_text: null,
        source_text_hash: null,
        metadata_snapshot_json: '{}',
      },
    ])
    store.dispose()
  })

  /** Deleting the citation must send an empty projection, not omit the field. */
  it('sends an empty projection once the citation is gone', async () => {
    const { store, now } = makeStore()
    await store.openDocument('d1')
    store.applyEdit(WITH_CITATION)
    now.value += 5_000
    await store.flush()

    store.applyEdit({
      schemaVersion: 1,
      doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'ya no' }] }] },
    })
    now.value += 10_000
    await store.flush()

    const saves = mockInvoke.mock.calls.filter(([command]) => command === 'writing_save_document')
    const last = saves.at(-1)?.[1] as { save: { citations: unknown[] } }
    expect(last.save.citations).toEqual([])
    store.dispose()
  })
})
