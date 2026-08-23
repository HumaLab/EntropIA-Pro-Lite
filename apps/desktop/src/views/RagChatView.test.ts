import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import type { RagAnswer, RagConversation, RagConversationSummary } from '$lib/rag'
import { ragChat } from '$lib/rag-chat'
import RagChatView from './RagChatView.svelte'

const { navigateMock } = vi.hoisted(() => ({
  navigateMock: vi.fn(),
}))

vi.mock('$lib/navigation', () => ({
  navigation: {
    navigate: navigateMock,
  },
}))

const mockInvoke = vi.mocked(invoke)

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })

  return { promise, resolve, reject }
}

interface BackendState {
  storedActiveId: string | null
  summaries: RagConversationSummary[]
  conversations: Record<string, RagConversation>
  ask: (args: { question: string; conversationId?: string }) => Promise<RagAnswer> | RagAnswer
}

function setupBackend(overrides: Partial<BackendState> = {}): BackendState {
  const state: BackendState = {
    storedActiveId: null,
    summaries: [],
    conversations: {},
    ask: () => {
      throw new Error('unexpected rag_ask')
    },
    ...overrides,
  }

  mockInvoke.mockImplementation((async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case 'settings_get':
        return args?.key === 'rag_active_conversation' ? state.storedActiveId : null
      case 'settings_set':
      case 'settings_delete':
        return undefined
      case 'rag_list_conversations':
        return state.summaries
      case 'rag_get_conversation': {
        const found = state.conversations[args?.conversationId as string]
        if (!found) throw 'No se encontró la conversación.'
        return found
      }
      case 'rag_ask':
        return state.ask(args as { question: string; conversationId?: string })
      case 'rag_delete_conversation':
        return undefined
      default:
        throw new Error(`unexpected command: ${command}`)
    }
  }) as typeof invoke)

  return state
}

function callsFor(command: string): unknown[][] {
  return mockInvoke.mock.calls.filter(([cmd]) => cmd === command)
}

const answerWithSources: RagAnswer = {
  answer: 'La huelga comenzó en junio de 1966 [1].',
  sources: [
    {
      index: 1,
      assetId: 'asset-1',
      itemId: 'item-1',
      itemTitle: 'Entrevista 12',
      collectionId: 'col-1',
      collectionName: 'Historia oral',
      snippet: 'la huelga comenzó cuando los obreros del SOIP...',
      score: 0.91,
      startSeconds: 65,
      endSeconds: 80,
      provenance: null,
    },
  ],
  model: 'test-model',
  conversationId: 'conv-new',
}

const storedConversation: RagConversation = {
  id: 'conv-1',
  title: '¿Cuándo comenzó la huelga?',
  messages: [
    {
      id: 'msg-1',
      role: 'user',
      content: '¿Cuándo comenzó la huelga?',
      sources: [],
      createdAt: 1700000000000,
    },
    {
      id: 'msg-2',
      role: 'assistant',
      content: 'La huelga comenzó en junio de 1966 [1].',
      sources: answerWithSources.sources,
      createdAt: 1700000001000,
    },
  ],
}

const conversationSummaries: RagConversationSummary[] = [
  {
    id: 'conv-1',
    title: '¿Cuándo comenzó la huelga?',
    createdAt: 1700000000000,
    updatedAt: 1700000001000,
    messageCount: 2,
  },
  {
    id: 'conv-2',
    title: 'Salarios del SOIP',
    createdAt: 1600000000000,
    updatedAt: 1600000001000,
    messageCount: 2,
  },
]

function getComposer() {
  return screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })
}

async function sendQuestion(question: string) {
  const composer = getComposer()
  await fireEvent.input(composer, { target: { value: question } })
  await fireEvent.keyDown(composer, { key: 'Enter' })
}

beforeEach(() => {
  locale.set('es')
  navigateMock.mockReset()
  mockInvoke.mockReset()
  ragChat.reset()
})

describe('RagChatView', () => {

  it('keeps bottom spacing below the research chat composer', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'RagChatView.svelte'), 'utf-8')

    expect(source).toMatch(
      /\.rag-chat\s*\{[\s\S]*?padding-block-end: var\(--space-4\);/
    )
  })
  it('renders the empty state with header copy, composer controls and empty sidebar', async () => {
    setupBackend()
    render(RagChatView)

    expect(screen.getByRole('heading', { name: 'Chat de investigación' })).toBeInTheDocument()
    expect(
      screen.getByText('Consultá la base de conocimiento de transcripciones y documentos OCR')
    ).toBeInTheDocument()
    expect(
      screen.getByText(
        'Hacé una pregunta sobre tus transcripciones y documentos. Las respuestas citan las fuentes.'
      )
    ).toBeInTheDocument()
    expect(getComposer()).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Enviar' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Nueva conversación' })).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Conversaciones' })).toBeInTheDocument()

    await waitFor(() => {
      expect(screen.getByText('Sin conversaciones todavía')).toBeInTheDocument()
    })
  })

  it('sends a question with Enter and renders the answer with its sources', async () => {
    const state = setupBackend({
      ask: () => {
        state.summaries = [
          {
            id: 'conv-new',
            title: '¿Cuándo comenzó la huelga?',
            createdAt: 1700000000000,
            updatedAt: 1700000001000,
            messageCount: 2,
          },
        ]
        return answerWithSources
      },
    })

    render(RagChatView)
    await sendQuestion('¿Cuándo comenzó la huelga?')

    expect(callsFor('rag_ask')).toEqual([
      [
        'rag_ask',
        { question: '¿Cuándo comenzó la huelga?', conversationId: undefined, topK: undefined },
      ],
    ])

    // The question shows up as the user bubble (and later as the sidebar title).
    expect(screen.getAllByText('¿Cuándo comenzó la huelga?').length).toBeGreaterThan(0)

    await waitFor(() => {
      expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
    })

    expect(screen.getByText('Fuentes')).toBeInTheDocument()
    expect(screen.getByText('[1]')).toBeInTheDocument()
    expect(screen.getByText('Entrevista 12 (Historia oral)')).toBeInTheDocument()
    expect(screen.getByText('1:05–1:20')).toBeInTheDocument()
    expect(screen.getByText('la huelga comenzó cuando los obreros del SOIP...')).toBeInTheDocument()

    // The active conversation id was persisted and the sidebar refreshed.
    expect(callsFor('settings_set')).toEqual([
      ['settings_set', { key: 'rag_active_conversation', value: 'conv-new' }],
    ])
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /¿Cuándo comenzó la huelga\?/ })).toHaveAttribute(
        'aria-current',
        'true'
      )
    })
  })

  it('does not send when Shift+Enter inserts a newline', async () => {
    setupBackend()
    render(RagChatView)

    const composer = getComposer()
    await fireEvent.input(composer, { target: { value: 'pregunta larga' } })
    await fireEvent.keyDown(composer, { key: 'Enter', shiftKey: true })

    expect(callsFor('rag_ask')).toHaveLength(0)
  })

  it('does not send on Enter while IME composition is active', async () => {
    setupBackend()
    render(RagChatView)

    const composer = getComposer()
    await fireEvent.input(composer, { target: { value: 'にほんご' } })
    await fireEvent.keyDown(composer, { key: 'Enter', isComposing: true })

    expect(callsFor('rag_ask')).toHaveLength(0)
  })

  it('navigates to the cited item when a source is clicked', async () => {
    setupBackend({ ask: () => answerWithSources })

    render(RagChatView)
    await sendQuestion('¿Cuándo comenzó la huelga?')

    const sourceButton = await screen.findByRole('button', {
      name: 'Abrir fuente: [1] Entrevista 12',
    })
    await fireEvent.click(sourceButton)

    expect(navigateMock).toHaveBeenCalledWith({
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Historia oral',
      itemId: 'item-1',
      itemTitle: 'Entrevista 12',
      assetId: 'asset-1',
    })
  })

  it('omits the timestamp when startSeconds is null', async () => {
    setupBackend({
      ask: () => ({
        ...answerWithSources,
        sources: [{ ...answerWithSources.sources[0]!, startSeconds: null, endSeconds: null }],
      }),
    })

    render(RagChatView)
    await sendQuestion('¿Cuándo comenzó la huelga?')

    await waitFor(() => {
      expect(screen.getByText('Entrevista 12 (Historia oral)')).toBeInTheDocument()
    })
    expect(screen.queryByText('1:05–1:20')).not.toBeInTheDocument()
  })

  it('shows the no-results copy as an assistant message without sources', async () => {
    setupBackend({
      ask: () => ({ answer: '', sources: [], model: 'test-model', conversationId: 'conv-new' }),
    })

    render(RagChatView)
    await sendQuestion('¿Algo sin contexto?')

    await waitFor(() => {
      expect(
        screen.getByText(
          'No encontré contenido relevante en la base de conocimiento para esa pregunta.'
        )
      ).toBeInTheDocument()
    })
    expect(screen.queryByText('Fuentes')).not.toBeInTheDocument()
  })

  it('shows backend errors inline as an alert', async () => {
    const backendError = 'Falta la API key de OpenRouter. Configurala en Configuración.'
    setupBackend({ ask: () => Promise.reject(backendError) })

    render(RagChatView)
    await sendQuestion('¿Cuándo comenzó la huelga?')

    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent(backendError)
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })

  it('shows the thinking row and disables the composer while loading', async () => {
    const pending = deferred<RagAnswer>()
    setupBackend({ ask: () => pending.promise })

    render(RagChatView)
    await sendQuestion('¿Cuándo comenzó la huelga?')

    expect(screen.getByRole('status')).toHaveTextContent('Buscando en la base de conocimiento…')
    expect(getComposer()).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Enviar' })).toBeDisabled()

    pending.resolve(answerWithSources)

    await waitFor(() => {
      expect(screen.queryByRole('status')).not.toBeInTheDocument()
    })
    expect(getComposer()).toBeEnabled()
  })

  it('rehydrates the persisted active conversation and lists past conversations', async () => {
    setupBackend({
      storedActiveId: 'conv-1',
      summaries: conversationSummaries,
      conversations: { 'conv-1': storedConversation },
    })

    render(RagChatView)

    await waitFor(() => {
      expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
    })
    expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()

    const activeRow = screen.getByRole('button', { name: /¿Cuándo comenzó la huelga\?/ })
    expect(activeRow).toHaveAttribute('aria-current', 'true')
  })

  it('switches conversations when a sidebar row is clicked', async () => {
    setupBackend({
      storedActiveId: 'conv-1',
      summaries: conversationSummaries,
      conversations: {
        'conv-1': storedConversation,
        'conv-2': {
          id: 'conv-2',
          title: 'Salarios del SOIP',
          messages: [
            {
              id: 'msg-3',
              role: 'user',
              content: '¿Cuánto ganaban en el SOIP?',
              sources: [],
              createdAt: 1600000000000,
            },
            {
              id: 'msg-4',
              role: 'assistant',
              content: 'El jornal rondaba los 200 pesos.',
              sources: [],
              createdAt: 1600000001000,
            },
          ],
        },
      },
    })

    render(RagChatView)
    await waitFor(() => {
      expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('button', { name: /Salarios del SOIP/ }))

    await waitFor(() => {
      expect(screen.getByText('El jornal rondaba los 200 pesos.')).toBeInTheDocument()
    })
    expect(callsFor('rag_get_conversation')).toEqual([
      ['rag_get_conversation', { conversationId: 'conv-1' }],
      ['rag_get_conversation', { conversationId: 'conv-2' }],
    ])
    expect(screen.queryByText('La huelga comenzó en junio de 1966 [1].')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Salarios del SOIP/ })).toHaveAttribute(
      'aria-current',
      'true'
    )
  })

  it('asks for confirmation before deleting a conversation', async () => {
    const state = setupBackend({
      storedActiveId: 'conv-1',
      summaries: conversationSummaries,
      conversations: { 'conv-1': storedConversation },
    })

    render(RagChatView)
    await waitFor(() => {
      expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()
    })

    const deleteButtons = screen.getAllByRole('button', { name: 'Eliminar conversación' })
    await fireEvent.click(deleteButtons[1]!)

    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(screen.getByText('¿Eliminar esta conversación?')).toBeInTheDocument()
    expect(
      screen.getByText(
        'Se va a eliminar la conversación y sus mensajes. Esta acción no se puede deshacer.'
      )
    ).toBeInTheDocument()
    expect(callsFor('rag_delete_conversation')).toHaveLength(0)

    state.summaries = [conversationSummaries[0]!]
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar' }))

    await waitFor(() => {
      expect(callsFor('rag_delete_conversation')).toEqual([
        ['rag_delete_conversation', { conversationId: 'conv-2' }],
      ])
    })
    await waitFor(() => {
      expect(screen.queryByText('Salarios del SOIP')).not.toBeInTheDocument()
    })
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    // The active conversation was not the deleted one — messages stay.
    expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
  })

  it('cancels the delete dialog without deleting anything', async () => {
    setupBackend({
      storedActiveId: 'conv-1',
      summaries: conversationSummaries,
      conversations: { 'conv-1': storedConversation },
    })

    render(RagChatView)
    await waitFor(() => {
      expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getAllByRole('button', { name: 'Eliminar conversación' })[0]!)
    await fireEvent.click(screen.getByRole('button', { name: 'Cancelar' }))

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(callsFor('rag_delete_conversation')).toHaveLength(0)
    expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()
  })

  it('starts a new conversation and ignores stale in-flight responses', async () => {
    const pending = deferred<RagAnswer>()
    setupBackend({ ask: () => pending.promise })

    render(RagChatView)
    await sendQuestion('¿Cuándo comenzó la huelga?')

    await fireEvent.click(screen.getByRole('button', { name: 'Nueva conversación' }))

    pending.resolve(answerWithSources)
    await Promise.resolve()

    expect(
      screen.getByText(
        'Hacé una pregunta sobre tus transcripciones y documentos. Las respuestas citan las fuentes.'
      )
    ).toBeInTheDocument()
    expect(screen.queryByText('La huelga comenzó en junio de 1966 [1].')).not.toBeInTheDocument()
    expect(screen.queryByText('¿Cuándo comenzó la huelga?')).not.toBeInTheDocument()
  })

  it('preserves the conversation and draft across unmount/remount', async () => {
    setupBackend({ ask: () => answerWithSources })

    const first = render(RagChatView)
    await sendQuestion('¿Cuándo comenzó la huelga?')
    await waitFor(() => {
      expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
    })

    await fireEvent.input(getComposer(), { target: { value: 'borrador a medio escribir' } })
    first.unmount()

    render(RagChatView)

    expect(screen.getByText('¿Cuándo comenzó la huelga?')).toBeInTheDocument()
    expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
    expect(getComposer()).toHaveValue('borrador a medio escribir')
  })

  it('receives an in-flight answer that resolves while the view is unmounted', async () => {
    const pending = deferred<RagAnswer>()
    setupBackend({ ask: () => pending.promise })

    const first = render(RagChatView)
    await sendQuestion('¿Cuándo comenzó la huelga?')
    first.unmount()

    pending.resolve(answerWithSources)
    await Promise.resolve()

    render(RagChatView)

    await waitFor(() => {
      expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
    })
  })
})
