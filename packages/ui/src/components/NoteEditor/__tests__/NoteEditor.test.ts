import { render, screen, fireEvent, waitFor } from '@testing-library/svelte'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import NoteEditor from '../NoteEditor.svelte'

describe('NoteEditor', () => {
  beforeEach(() => {
    vi.spyOn(HTMLInputElement.prototype, 'focus').mockImplementation(() => undefined)
    vi.spyOn(HTMLInputElement.prototype, 'select').mockImplementation(() => undefined)
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('renders a rich text textbox with toolbar', () => {
    render(NoteEditor, { props: {} })
    expect(screen.getByRole('textbox')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Bold' })).toBeInTheDocument()
  })

  it('renders with initial content', () => {
    render(NoteEditor, { props: { content: 'Hello world' } })
    expect(screen.getByRole('textbox')).toHaveTextContent('Hello world')
  })

  it('renders placeholder when provided', () => {
    render(NoteEditor, { props: { placeholder: 'Write a note...' } })
    expect(screen.getByRole('textbox')).toHaveAttribute('aria-placeholder', 'Write a note...')
  })

  it('save button is disabled when content is empty', () => {
    render(NoteEditor, { props: {} })
    const saveBtn = screen.getByTestId('note-save')
    expect(saveBtn).toBeDisabled()
  })

  it('save button is enabled when the editor starts with content', async () => {
    render(NoteEditor, { props: { content: 'New content' } })
    const saveBtn = screen.getByTestId('note-save')
    expect(saveBtn).not.toBeDisabled()
  })

  it('renders save as an icon-only control with a localized tooltip and accessible name', () => {
    render(NoteEditor, {
      props: {
        content: 'Nota',
        saveLabel: 'Guardar nota',
      },
    })

    const saveBtn = screen.getByRole('button', { name: 'Guardar nota' })

    expect(saveBtn).toHaveAttribute('title', 'Guardar nota')
    expect(saveBtn.querySelector('svg')).not.toBeNull()
    expect(saveBtn.textContent?.trim()).toBe('')
  })

  it('disables save in edit mode when content was not changed', () => {
    render(NoteEditor, {
      props: {
        content: '<p>Nota original</p>',
        oncancel: vi.fn(),
        clearOnSave: false,
      },
    })

    expect(screen.getByTestId('note-save')).toBeDisabled()
  })

  it('enables save in edit mode after the note content changes', async () => {
    render(NoteEditor, {
      props: {
        content: '<p>Nota original</p>',
        oncancel: vi.fn(),
        clearOnSave: false,
      },
    })

    const textbox = screen.getByRole('textbox')
    const saveBtn = screen.getByTestId('note-save')

    textbox.innerHTML = '<p>Nota actualizada</p>'
    await fireEvent.input(textbox)

    await waitFor(() => {
      expect(saveBtn).not.toBeDisabled()
    })
  })

  it('renders grouped formatting actions with separate link and unlink controls', () => {
    render(NoteEditor, { props: {} })

    expect(screen.getByRole('group', { name: 'Text style' })).toBeInTheDocument()
    expect(screen.getByRole('group', { name: 'Structure' })).toBeInTheDocument()
    expect(screen.getByRole('group', { name: 'Insert' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Add link' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Remove link' })).toBeInTheDocument()
  })

  it('renders icon-only toolbar buttons with localized tooltips and accessible names', () => {
    render(NoteEditor, {
      props: {
        labels: {
          bulletList: 'Lista con viñetas',
          orderedList: 'Lista ordenada',
          quote: 'Cita',
          addLink: 'Agregar link',
          removeLink: 'Quitar link',
        },
      },
    })

    const addLink = screen.getByRole('button', { name: 'Agregar link' })
    const removeLink = screen.getByRole('button', { name: 'Quitar link' })
    const bulletList = screen.getByRole('button', { name: 'Lista con viñetas' })

    expect(addLink).toHaveAttribute('title', 'Agregar link')
    expect(removeLink).toHaveAttribute('title', 'Quitar link')
    expect(bulletList).toHaveAttribute('title', 'Lista con viñetas')

    expect(addLink.querySelector('svg')).not.toBeNull()
    expect(addLink.textContent?.trim()).toBe('')
    expect(bulletList.querySelector('svg')).not.toBeNull()
    expect(bulletList.textContent?.trim()).toBe('')
  })

  it('shows concise helper text for note editing affordances', () => {
    render(NoteEditor, { props: {} })

    expect(screen.getByText('Tip: select text to apply formatting or links.')).toBeInTheDocument()
  })

  it('opens a custom link modal instead of using a browser prompt', async () => {
    const promptSpy = vi.spyOn(window, 'prompt')

    render(NoteEditor, { props: {} })

    await fireEvent.click(screen.getByRole('button', { name: 'Add link' }))

    expect(promptSpy).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog', { name: 'Insert link' })).toBeInTheDocument()
    expect(screen.getByTestId('note-editor-link-input')).toHaveAttribute(
      'placeholder',
      'https://...'
    )
  })

  it('shows subtle validation feedback for invalid link URLs', async () => {
    render(NoteEditor, { props: {} })

    await fireEvent.click(screen.getByRole('button', { name: 'Add link' }))
    await fireEvent.input(screen.getByTestId('note-editor-link-input'), {
      target: { value: 'not a url' },
    })
    await fireEvent.click(screen.getByTestId('note-editor-link-submit'))

    expect(screen.getByTestId('note-editor-link-error')).toHaveTextContent(
      'Enter a valid URL, for example https://entropia.app.'
    )
  })

  it('closes the link modal when escape is pressed', async () => {
    render(NoteEditor, { props: {} })

    await fireEvent.click(screen.getByRole('button', { name: 'Add link' }))

    const dialog = screen.getByRole('dialog', { name: 'Insert link' })
    await fireEvent.keyDown(dialog, { key: 'Escape' })

    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: 'Insert link' })).not.toBeInTheDocument()
    })
  })

  it('calls onsave with sanitized html when save is clicked', async () => {
    const onsave = vi.fn()
    render(NoteEditor, { props: { onsave, content: 'My note' } })

    const saveBtn = screen.getByTestId('note-save')
    await fireEvent.click(saveBtn)
    expect(onsave).toHaveBeenCalledOnce()
    expect(onsave).toHaveBeenCalledWith('<p>My note</p>')
  })

  it('clears the editor after successful save by default', async () => {
    const onsave = vi.fn().mockResolvedValue(undefined)
    render(NoteEditor, { props: { onsave, content: 'A note to save' } })

    const saveBtn = screen.getByTestId('note-save')
    await fireEvent.click(saveBtn)

    await waitFor(() => {
      expect(screen.getByRole('textbox')).not.toHaveTextContent('A note to save')
      expect(saveBtn).toBeDisabled()
    })
  })

  it('keeps the current content after saving when clearOnSave is false', async () => {
    const onsave = vi.fn().mockResolvedValue(undefined)
    render(NoteEditor, { props: { onsave, content: 'A note to save', clearOnSave: false } })

    const saveBtn = screen.getByTestId('note-save')
    await fireEvent.click(saveBtn)

    expect(screen.getByRole('textbox')).toHaveTextContent('A note to save')
  })

  it('does not clear editor when onsave rejects', async () => {
    const onsave = vi.fn().mockRejectedValue(new Error('Save failed'))
    render(NoteEditor, { props: { onsave, content: 'A note that fails' } })

    const saveBtn = screen.getByTestId('note-save')
    await fireEvent.click(saveBtn)

    expect(screen.getByRole('textbox')).toHaveTextContent('A note that fails')
  })

  it('does not render a cancel button', () => {
    render(NoteEditor, { props: {} })
    expect(screen.queryByTestId('note-cancel')).not.toBeInTheDocument()
  })
})
