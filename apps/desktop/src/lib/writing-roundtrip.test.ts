import { Editor } from '@tiptap/core'
import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createWritingExtensions, emptyDocument, WRITING_SCHEMA_VERSION } from '@entropia/ui'
import { WritingStore, type WritingDocumentRow } from './writing'
import { DEFAULT_SCHEDULER } from './writing-scheduler'

/**
 * The editor emits a footnote correctly and the database ends up with the
 * marker but no footnote block. This drives the real editor through the real
 * store and inspects the payload that would be persisted.
 */

const mockInvoke = vi.mocked(invoke)
let saved: string | null = null

const ROW: WritingDocumentRow = {
  id: 'd1',
  title: 'Doc',
  document_type: 'article',
  status: 'active',
  schema_version: 1,
  current_content_json: JSON.stringify(emptyDocument()),
  revision: 0,
  created_at: 1,
  updated_at: 1,
}

beforeEach(() => {
  saved = null
  mockInvoke.mockReset()
  mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
    if (command === 'writing_is_ready') return true as never
    if (command === 'writing_load_document') return ROW as never
    if (command === 'writing_save_document') {
      saved = ((args as { save: { content_json: string } }).save).content_json
      return 1 as never
    }
    return undefined as never
  })
})

describe('editor to database, with a footnote', () => {
  it('persists the footnotes block the editor produced', async () => {
    const now = { value: 0 }
    const store = new WritingStore(DEFAULT_SCHEDULER, () => now.value)
    await store.openDocument('d1')

    const element = document.createElement('div')
    document.body.appendChild(element)
    const editor = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: store.snapshot.content!.doc,
      onUpdate: ({ editor: instance }) => {
        store.applyEdit({ schemaVersion: WRITING_SCHEMA_VERSION, doc: instance.getJSON() })
      },
    })

    editor.chain().focus().insertContent('una afirmacion').run()
    editor.chain().focus().addFootnote().run()

    // What the editor holds right now, for comparison.
    expect(JSON.stringify(editor.getJSON())).toContain('"footnotes"')

    now.value = DEFAULT_SCHEDULER.saveDebounceMs
    await store.tick()
    store.dispose()
    editor.destroy()

    expect(saved, 'nothing was saved').not.toBeNull()
    expect(saved, 'the footnotes block did not survive the round trip').toContain('"footnotes"')
  })
})
