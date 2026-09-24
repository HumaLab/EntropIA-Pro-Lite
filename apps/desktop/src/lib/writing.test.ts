import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  WritingStore,
  citationsForAsset,
  type PendingProvenance,
  type WritingDocumentRow,
} from './writing'
import { DEFAULT_SCHEDULER } from './writing-scheduler'
import { navigation } from './navigation'

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
            doc: { type: 'doc', content: [{ type: 'holographicMarginalia' }] },
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
      doc: {
        type: 'doc',
        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Hola' }] }],
      },
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
      doc: {
        type: 'doc',
        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'x' }] }],
      },
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
      doc: {
        type: 'doc',
        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'x' }] }],
      },
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
   * Regression: `back()` could land on a discarded document's own screen
   * once it no longer existed. Only a successful trash prunes it — a failed
   * one leaves history alone, matching that it also leaves the document
   * listed.
   */
  it('prunes history for the document once the trash succeeds', async () => {
    const forgetWriting = vi.spyOn(navigation, 'forgetWriting')
    const { store } = makeStore()
    await store.listDocuments()

    await store.trashDocument('d1')

    expect(forgetWriting).toHaveBeenCalledWith('d1')
    forgetWriting.mockRestore()
  })

  it('does not prune history when the trash command fails', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'writing_list_documents') return [ROW] as never
      if (command === 'writing_set_status') throw { code: 'document_not_found', message: 'gone' }
      return undefined as never
    })
    const forgetWriting = vi.spyOn(navigation, 'forgetWriting')
    const { store } = makeStore()
    await store.listDocuments()

    await store.trashDocument('d1')

    expect(forgetWriting).not.toHaveBeenCalled()
    forgetWriting.mockRestore()
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
      doc: {
        type: 'doc',
        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'x' }] }],
      },
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
      doc: {
        type: 'doc',
        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'y' }] }],
      },
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
      doc: {
        type: 'doc',
        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'ya no' }] }],
      },
    })
    now.value += 10_000
    await store.flush()

    const saves = mockInvoke.mock.calls.filter(([command]) => command === 'writing_save_document')
    const last = saves.at(-1)?.[1] as { save: { citations: unknown[] } }
    expect(last.save.citations).toEqual([])
    store.dispose()
  })
})

/**
 * Provenance travels with the save that commits it (plan-editor.md §9.6, §10.1).
 *
 * The node, the projection and the event must land together or not at all, and
 * `save_document` is the one transaction that can do that. What is asserted
 * here is the failure half of that promise: a save that fails must leave the
 * event still waiting, not swallowed and not half-written.
 */
describe('writing store - pending provenance', () => {
  const EVENT: PendingProvenance = {
    id: 'pv1',
    origin_type: 'corpus',
    operation_type: 'insert',
    range_anchor_json: JSON.stringify({ citationNodeId: 'c1' }),
    source_reference_json: JSON.stringify({ assetId: 'as1' }),
    model_provider: null,
    model_name: null,
  }

  const EDIT = {
    schemaVersion: 1,
    doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'x' }] }] },
  }

  it('sends the queued events with the save and stops waiting once it lands', async () => {
    const { store, now } = makeStore()
    await store.openDocument('d1')
    store.queueProvenance(EVENT)
    store.applyEdit(EDIT)
    now.value += 5_000
    await store.flush()

    const save = mockInvoke.mock.calls.find(([command]) => command === 'writing_save_document')
    const sent = (save?.[1] as { save: { provenance: unknown[] } }).save.provenance
    expect(sent).toEqual([EVENT])
    expect(store.pendingProvenance).toEqual([])
    store.dispose()
  })

  /** §10.1: on failure, keep the draft and the error, and confirm nothing. */
  it('keeps the event queued when the save fails', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'writing_load_document') return ROW as never
      if (command === 'writing_save_document') throw { code: 'sql_error', message: 'locked' }
      if (command === 'writing_append_journal') return 1 as never
      return undefined as never
    })
    const { store, now } = makeStore()
    await store.openDocument('d1')
    store.queueProvenance(EVENT)
    store.applyEdit(EDIT)
    now.value += 5_000
    await store.flush()

    expect(store.snapshot.status).toBe('error')
    expect(store.pendingProvenance).toEqual([EVENT])
    expect(store.snapshot.content).toEqual(EDIT)
    store.dispose()
  })

  /**
   * A citation inserted while a save is in flight belongs to the next one. If
   * the queue were simply emptied on success, its event would be dropped
   * without ever having been sent.
   */
  it('does not clear an event queued while the save was in flight', async () => {
    let releaseSave: (value: never) => void = () => {}
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'writing_load_document') return ROW as never
      if (command === 'writing_append_journal') return 1 as never
      if (command === 'writing_save_document') {
        return (await new Promise((resolve) => {
          releaseSave = resolve
        })) as never
      }
      return undefined as never
    })
    const { store, now } = makeStore()
    await store.openDocument('d1')
    store.queueProvenance(EVENT)
    store.applyEdit(EDIT)
    now.value += 5_000
    const saving = store.flush()

    const later = { ...EVENT, id: 'pv2' }
    store.queueProvenance(later)
    releaseSave(4 as never)
    await saving

    expect(store.pendingProvenance).toEqual([later])
    store.dispose()
  })
})

/**
 * The dependency warning before deleting a cited asset (§10.3).
 *
 * It is a warning, never a veto: §29.1 settled the policy as deletion with a
 * preserved snapshot. So every way this can fail has to end in "no warning",
 * never in a deletion the user cannot complete.
 */
describe('citationsForAsset', () => {
  it('reports the manuscripts that cite the asset', async () => {
    const rows = [{ document_id: 'd1', document_title: 'Primero', citation_count: 2 }]
    mockInvoke.mockResolvedValue(rows as never)

    expect(await citationsForAsset('as1')).toEqual(rows)
    expect(mockInvoke).toHaveBeenCalledWith('writing_citations_for_asset', { assetId: 'as1' })
  })

  it('warns about nothing when the command fails', async () => {
    mockInvoke.mockRejectedValue(new Error('no such table'))

    expect(await citationsForAsset('as1')).toEqual([])
  })

  /**
   * The shape is checked rather than assumed. A caller reads `.length` on this,
   * so an answer that is not a list would turn a missing warning into a broken
   * confirmation dialog — which is exactly how it first failed.
   */
  it('warns about nothing when the answer is not a list', async () => {
    mockInvoke.mockResolvedValue(undefined as never)
    expect(await citationsForAsset('as1')).toEqual([])

    mockInvoke.mockResolvedValue({ unexpected: true } as never)
    expect(await citationsForAsset('as1')).toEqual([])
  })
})

/**
 * A `footnoteReference` whose `data-id` pairs with no `footnote` is what
 * `repairCanonical` prunes and counts, and what the notice reports.
 */
const ORPHAN_ROW: WritingDocumentRow = {
  ...ROW,
  id: 'd-orphan',
  current_content_json: JSON.stringify({
    schemaVersion: 1,
    doc: {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            { type: 'footnoteReference', attrs: { 'data-id': 'gone', referenceNumber: 1 } },
          ],
        },
      ],
    },
  }),
}

describe('writing store — the repair notice', () => {
  function storeOnOrphanDocument() {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'writing_is_ready') return true as never
      if (command === 'writing_load_document') return ORPHAN_ROW as never
      return undefined as never
    })
    return makeStore().store
  }

  it('reports the orphan markers the document carries', async () => {
    const store = storeOnOrphanDocument()
    await store.openDocument('d-orphan')

    expect(store.snapshot.repair).toEqual({ orphanFootnoteReferences: 1 })
  })

  it('drops the notice on dismiss without disturbing the document', async () => {
    const store = storeOnOrphanDocument()
    await store.openDocument('d-orphan')
    const { open, content, revision, status } = store.snapshot

    store.dismissRepair()

    expect(store.snapshot.repair).toBeNull()
    expect([
      store.snapshot.open,
      store.snapshot.content,
      store.snapshot.revision,
      store.snapshot.status,
    ]).toEqual([open, content, revision, status])
  })

  it('shows it again on the next open, because dismissing is not repairing', async () => {
    // This is the honest shape of the feature and the reason it is worth a
    // test: the report is derived on every open, never stored. Dismissing says
    // "I read it". The markers stay on disk until an edit saves the repaired
    // document, which is exactly what the notice tells the writer.
    const store = storeOnOrphanDocument()
    await store.openDocument('d-orphan')
    store.dismissRepair()

    await store.openDocument('d-orphan')

    expect(store.snapshot.repair).toEqual({ orphanFootnoteReferences: 1 })
  })
})
