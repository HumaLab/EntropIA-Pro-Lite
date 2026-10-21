import { Editor } from '@tiptap/core'
import { render, screen } from '@testing-library/svelte'
import { tick } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import NoteEditor from '../NoteEditor.svelte'

/**
 * Content the note editor is handed from outside — the stored note, a note
 * replaced elsewhere — is loaded, not typed. Loading it must not raise the
 * editor's update event with a changed document, which is what the component
 * treats as the writer's edits.
 */

afterEach(() => {
  vi.restoreAllMocks()
})

describe('NoteEditor external loads', () => {
  it('replaces its content from the prop without an update that changes it', async () => {
    const { rerender } = render(NoteEditor, {
      props: { content: '<p>Nota original</p>', clearOnSave: false },
    })
    const emit = vi.spyOn(Editor.prototype, 'emit')

    await rerender({ content: '<p>Nota cambiada en otro lado</p>', clearOnSave: false })
    await tick()

    const edits = emit.mock.calls.filter(
      ([event, payload]) =>
        event === 'update' &&
        (payload as { transaction: { docChanged: boolean } }).transaction.docChanged
    )
    expect(screen.getByRole('textbox')).toHaveTextContent('Nota cambiada en otro lado')
    expect(edits).toHaveLength(0)
  })
})
