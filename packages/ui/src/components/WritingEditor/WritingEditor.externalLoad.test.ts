import { Editor } from '@tiptap/core'
import { render } from '@testing-library/svelte'
import { tick } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

import WritingEditor from './WritingEditor.svelte'
import { WRITING_SCHEMA_VERSION, type CanonicalDocument } from './document-contract'

/**
 * A document that arrives from outside — a recovery replay, a version restore —
 * replaces the content without being an edit. Its load must not raise the
 * editor's update event with a changed document, which is what `onchange`
 * and autosave listen to. (`setEditable` raises an update too, with nothing
 * changed; `onchange` already ignores that by comparing what it last sent.)
 */

afterEach(() => {
  vi.restoreAllMocks()
})

function manuscript(text: string): CanonicalDocument {
  return {
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] },
  }
}

const surface = () => document.querySelector('.writing-editor__surface') as HTMLElement

describe('WritingEditor external loads', () => {
  it('replaces the document from the prop without an update that changes it', async () => {
    const { rerender } = render(WritingEditor, {
      props: { document: manuscript('primera versión') },
    })
    const emit = vi.spyOn(Editor.prototype, 'emit')

    await rerender({ document: manuscript('versión restaurada') })
    await tick()

    const edits = emit.mock.calls.filter(
      ([event, payload]) =>
        event === 'update' &&
        (payload as { transaction: { docChanged: boolean } }).transaction.docChanged
    )
    expect(surface()).toHaveTextContent('versión restaurada')
    expect(edits).toHaveLength(0)
  })
})
