import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import WritingEditor from './WritingEditor.svelte'
import { WRITING_SCHEMA_VERSION, type CanonicalDocument } from './document-contract'

/**
 * Dictation in the manuscript (the same capture NoteEditor has).
 *
 * The recorder is faked the way NoteEditor's own dictation test fakes it: the
 * final chunk arrives on stop, so a click on "stop" is a complete recording.
 */
class FakeMediaRecorder {
  static isTypeSupported = vi.fn((mimeType: string) => mimeType === 'audio/webm;codecs=opus')

  public state: 'inactive' | 'recording' = 'inactive'
  public mimeType = 'audio/webm'
  public onstart: (() => void) | null = null
  public ondataavailable: ((event: { data: Blob }) => void) | null = null
  public onstop: (() => void) | null = null
  public onerror: ((event: Event) => void) | null = null

  constructor(_stream: MediaStream, options?: { mimeType?: string }) {
    this.mimeType = options?.mimeType ?? 'audio/webm'
  }

  start() {
    this.state = 'recording'
    this.onstart?.()
  }

  stop() {
    this.state = 'inactive'
    this.ondataavailable?.({ data: new Blob(['audio'], { type: this.mimeType }) })
    this.onstop?.()
  }

  requestData() {}
}

function manuscript(text: string): CanonicalDocument {
  return {
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] },
  }
}

const surface = () => document.querySelector('.writing-editor__surface') as HTMLElement

describe('WritingEditor dictation', () => {
  const stopTrack = vi.fn()

  beforeEach(() => {
    stopTrack.mockReset()
    Object.defineProperty(globalThis, 'MediaRecorder', {
      configurable: true,
      value: FakeMediaRecorder,
    })
    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      configurable: true,
      value: {
        getUserMedia: vi.fn().mockResolvedValue({
          getTracks: () => [{ stop: stopTrack }],
        }),
      },
    })
    Object.defineProperty(window, 'AudioContext', { configurable: true, value: undefined })
    Object.defineProperty(globalThis.navigator, 'userAgent', {
      configurable: true,
      value:
        'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/126.0.0.0 Safari/537.36',
    })
    Object.defineProperty(globalThis.navigator, 'platform', {
      configurable: true,
      value: 'MacIntel',
    })
    Reflect.deleteProperty(window, '__TAURI_INTERNALS__')
  })

  it('has no microphone button when nothing can transcribe', () => {
    render(WritingEditor, { props: { document: manuscript('Hola') } })

    expect(screen.queryByRole('button', { name: 'Iniciar dictado' })).not.toBeInTheDocument()
  })

  it('has no microphone button when the toolbar is hidden', () => {
    render(WritingEditor, {
      props: { document: manuscript('Hola'), toolbar: false, ondictate: vi.fn() },
    })

    expect(screen.queryByRole('button', { name: 'Iniciar dictado' })).not.toBeInTheDocument()
  })

  it('puts the microphone last in the toolbar’s first row, after find', () => {
    render(WritingEditor, { props: { document: manuscript('Hola'), ondictate: vi.fn() } })

    const row = screen.getByRole('toolbar').querySelector('[data-toolbar-row="first"]')!
    const buttons = [...row.querySelectorAll('button')]
    const names = buttons.map((button) => button.getAttribute('aria-label'))

    expect(names.at(-1)).toBe('Iniciar dictado')
    expect(names.at(-2)).toBe('Buscar')
  })

  it('records, transcribes, and inserts the text at the caret', async () => {
    const ondictate = vi.fn().mockResolvedValue('texto dictado')
    const onchange = vi.fn()
    const { component } = render(WritingEditor, {
      props: { document: manuscript('Hola mundo'), ondictate, onchange },
    })

    // Caret right after "Hola": the paragraph opens at 0, its text starts at 1.
    component.goToPosition(5)

    await fireEvent.click(screen.getByRole('button', { name: 'Iniciar dictado' }))
    expect(screen.getByRole('button', { name: 'Detener dictado' })).toBeInTheDocument()
    expect(screen.getByTestId('writing-editor-dictation-timer')).toHaveTextContent('0:00 / 5:00')

    await fireEvent.click(screen.getByRole('button', { name: 'Detener dictado' }))

    await waitFor(() => {
      expect(ondictate).toHaveBeenCalledOnce()
      expect(surface()).toHaveTextContent('Hola texto dictado mundo')
    })

    // Autosave hears about it like any other edit.
    const last = onchange.mock.calls.at(-1)?.[0] as CanonicalDocument
    expect(JSON.stringify(last.doc)).toContain('Hola texto dictado mundo')
    expect(screen.getByTestId('writing-editor-dictation-message')).toHaveTextContent(
      'Texto insertado desde el micrófono.'
    )
    expect(stopTrack).toHaveBeenCalledOnce()
  })

  it('is one undo step', async () => {
    const ondictate = vi.fn().mockResolvedValue('texto dictado')
    const { component } = render(WritingEditor, {
      props: { document: manuscript('Hola mundo'), ondictate },
    })
    component.goToPosition(5)

    await fireEvent.click(screen.getByRole('button', { name: 'Iniciar dictado' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Detener dictado' }))
    await waitFor(() => expect(surface()).toHaveTextContent('Hola texto dictado mundo'))

    await fireEvent.click(screen.getByRole('button', { name: 'Deshacer' }))

    expect(surface()).toHaveTextContent('Hola mundo')
    expect(surface()).not.toHaveTextContent('dictado')
  })
})
