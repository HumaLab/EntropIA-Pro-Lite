import { Editor } from '@tiptap/core'
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { tick } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import NoteEditor from '../NoteEditor.svelte'

/**
 * Content the note editor is handed from outside — the stored note, a note
 * replaced elsewhere — is loaded, not typed. Loading it must not raise the
 * editor's update event with a changed document, which is what the component
 * treats as the writer's edits. Saving reloads the editor too: emptied for the
 * next note, or set to the HTML just saved; neither is an edit either.
 */

afterEach(() => {
  vi.restoreAllMocks()
})

/** Update events whose transaction changed the document, raised from now on. */
function watchEdits() {
  const emit = vi.spyOn(Editor.prototype, 'emit')
  return () =>
    emit.mock.calls.filter(
      ([event, payload]) =>
        event === 'update' &&
        (payload as { transaction: { docChanged: boolean } }).transaction.docChanged
    )
}

describe('NoteEditor external loads', () => {
  it('replaces its content from the prop without an update that changes it', async () => {
    const { rerender } = render(NoteEditor, {
      props: { content: '<p>Nota original</p>', clearOnSave: false },
    })
    const edits = watchEdits()

    await rerender({ content: '<p>Nota cambiada en otro lado</p>', clearOnSave: false })
    await tick()

    expect(screen.getByRole('textbox')).toHaveTextContent('Nota cambiada en otro lado')
    expect(edits()).toHaveLength(0)
  })

  it('empties itself after a save without an update that changes it', async () => {
    const onsave = vi.fn().mockResolvedValue(undefined)
    render(NoteEditor, { props: { content: '<p>Nota nueva</p>', onsave } })
    const edits = watchEdits()

    await fireEvent.click(screen.getByTestId('note-save'))

    await waitFor(() => expect(onsave).toHaveBeenCalledWith('<p>Nota nueva</p>'))
    await waitFor(() => expect(screen.getByRole('textbox')).toHaveTextContent(''))
    expect(edits()).toHaveLength(0)
  })

  it('reloads the saved HTML when it keeps the note, without an update that changes it', async () => {
    // As the app's parent does: the saved note comes back down as the prop.
    const onsave = vi.fn(async (html: string) => {
      await rerender({ content: html, onsave, clearOnSave: false })
    })
    const created = vi.spyOn(Editor.prototype, 'emit')
    const { rerender } = render(NoteEditor, {
      props: { content: '<p>Nota original</p>', onsave, clearOnSave: false },
    })
    const editor = created.mock.contexts[0] as Editor
    created.mockRestore()

    // The writer's edit. Strikethrough is a mark the editor holds but a stored
    // note does not (note-content.ts), so reloading the saved HTML really
    // changes the document: an identical one raises no update whatever the
    // option says.
    editor.commands.setContent('<p>Nota <s>actualizada</s></p>')
    await waitFor(() => expect(screen.getByTestId('note-save')).not.toBeDisabled())
    const edits = watchEdits()

    await fireEvent.click(screen.getByTestId('note-save'))

    const textbox = screen.getByRole('textbox')
    await waitFor(() => expect(onsave).toHaveBeenCalledWith('<p>Nota actualizada</p>'))
    await waitFor(() => expect(textbox.querySelector('s')).toBeNull())
    expect(textbox).toHaveTextContent('Nota actualizada')
    expect(edits()).toHaveLength(0)
  })
})
