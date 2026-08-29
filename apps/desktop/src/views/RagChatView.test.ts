import { tick } from 'svelte'
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import type { RagAnswer, RagConversation, RagConversationSummary } from '$lib/rag'
import { ragChat } from '$lib/rag-chat'
import RagChatView from './RagChatView.svelte'

const { navigateMock, downloadRagConversationPdfMock } = vi.hoisted(() => ({
  navigateMock: vi.fn(),
  downloadRagConversationPdfMock: vi.fn(),
}))

vi.mock('$lib/navigation', () => ({
  navigation: {
    navigate: navigateMock,
  },
}))

vi.mock('$lib/rag-chat-export', () => ({
  downloadRagConversationPdf: downloadRagConversationPdfMock,
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
  searchResults?: RagConversationSummary[]
  searchPromise?: Promise<RagConversationSummary[]>
  searchError?: unknown
  renameError?: unknown
  renamePromise?: Promise<unknown>
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
      case 'rag_search_conversations':
        if (state.searchPromise) return state.searchPromise
        if (state.searchError) throw state.searchError
        return state.searchResults ?? state.summaries
      case 'rag_get_conversation': {
        const found = state.conversations[args?.conversationId as string]
        if (!found) throw 'No se encontró la conversación.'
        return found
      }
      case 'rag_update_conversation_title':
        if (state.renameError) throw state.renameError
        if (state.renamePromise) await state.renamePromise
        state.summaries = state.summaries.map((conversation) =>
          conversation.id === args?.conversationId
            ? { ...conversation, title: args?.title as string }
            : conversation,
        )
        return undefined
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
  downloadRagConversationPdfMock.mockReset()
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

    const send = screen.getByRole('button', { name: 'Enviar' })
    const newConversation = screen.getByRole('button', { name: 'Nueva conversación' })

    expect(send).toHaveAttribute('title', 'Enviar')
    expect(send.querySelector('svg')).not.toBeNull()
    expect(send.textContent?.trim()).toBe('')
    expect(newConversation).toHaveAttribute('title', 'Nueva conversación')
    expect(newConversation.querySelector('svg')).not.toBeNull()
    expect(newConversation.textContent?.trim()).toBe('')
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
  it('copies only assistant content and shows transient feedback', async () => {
    const answerWithMultipleSources: RagAnswer = {
      ...answerWithSources,
      sources: [
        answerWithSources.sources[0]!,
        {
          ...answerWithSources.sources[0]!,
          index: 2,
          assetId: 'asset-2',
          itemId: 'item-2',
          itemTitle: 'Acta & <anexo>',
          snippet: 'metadata-sentinel-snippet',
          collectionName: 'metadata-sentinel-collection',
          startSeconds: 12,
          endSeconds: 15,
          provenance: {
            retrievalUnit: 'chunk',
            sourceKind: 'ocr',
            sourceId: 'metadata-sentinel-source',
            chunkIds: ['metadata-sentinel-chunk'],
            startChar: 10,
            endChar: 20,
          },
        },
      ],
    }

    setupBackend({ ask: () => answerWithMultipleSources })
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })

    render(RagChatView)
    await sendQuestion('¿Cuándo comenzó la huelga?')

    const copy = await screen.findByRole('button', { name: 'Copiar respuesta' })
    expect(copy).toHaveAttribute('title', 'Copiar respuesta')
    expect(screen.queryByRole('button', { name: 'Copiar pregunta' })).not.toBeInTheDocument()

    await fireEvent.click(copy)

    expect(writeText).toHaveBeenCalledWith(
      'La huelga comenzó en junio de 1966 [1].\n\nFuentes:\n[1] Entrevista 12\n[2] Acta & <anexo>',
    )
    expect(writeText.mock.calls[0]?.[0]).not.toContain('metadata-sentinel-snippet')
    expect(writeText.mock.calls[0]?.[0]).not.toContain('metadata-sentinel-collection')
    expect(writeText.mock.calls[0]?.[0]).not.toContain('metadata-sentinel-source')
    expect(screen.getByText('Copiado')).toBeInTheDocument()
    expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
  })
  it('copies assistant content unchanged when there are no sources', async () => {
    setupBackend({
      ask: () => ({
        ...answerWithSources,
        answer: 'El jornal rondaba los 200 pesos.',
        sources: [],
      }),
    })
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })

    render(RagChatView)
    await sendQuestion('¿Cuánto ganaban en el SOIP?')

    const copy = await screen.findByRole('button', { name: 'Copiar respuesta' })
    await fireEvent.click(copy)

    expect(writeText).toHaveBeenCalledWith('El jornal rondaba los 200 pesos.')
  })

  it('clears copy feedback when switching to a different conversation', async () => {
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
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })

    render(RagChatView)
    const copy = await screen.findByRole('button', { name: 'Copiar respuesta' })
    await fireEvent.click(copy)
    expect(screen.getByText('Copiado')).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: /Salarios del SOIP/ }))

    await waitFor(() => {
      expect(screen.getByText('El jornal rondaba los 200 pesos.')).toBeInTheDocument()
    })
    expect(screen.queryByText('Copiado')).not.toBeInTheDocument()
  })
  it('ignores deferred copy feedback after switching conversations', async () => {
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
    const writeText = deferred<void>()
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: vi.fn(() => writeText.promise) },
    })

    render(RagChatView)
    const copy = await screen.findByRole('button', { name: 'Copiar respuesta' })
    await fireEvent.click(copy)

    await fireEvent.click(screen.getByRole('button', { name: /Salarios del SOIP/ }))
    await waitFor(() => {
      expect(screen.getByText('El jornal rondaba los 200 pesos.')).toBeInTheDocument()
    })

    writeText.resolve()
    await writeText.promise
    await tick()
    expect(screen.queryByText('Copiado')).not.toBeInTheDocument()
    expect(screen.queryByText('No se pudo copiar la respuesta.')).not.toBeInTheDocument()
  })

  it('keeps newer copy feedback when an older attempt finishes later', async () => {
    setupBackend({ ask: () => answerWithSources })
    const first = deferred<void>()
    const second = deferred<void>()
    const writeText = vi
      .fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })

    render(RagChatView)
    await sendQuestion('¿Cuándo comenzó la huelga?')
    const copy = await screen.findByRole('button', { name: 'Copiar respuesta' })
    await fireEvent.click(copy)
    await fireEvent.click(copy)

    second.resolve()
    await second.promise
    await tick()
    expect(screen.getByText('Copiado')).toBeInTheDocument()

    first.reject(new Error('older copy failed'))
    await expect(first.promise).rejects.toThrow('older copy failed')
    await tick()
    expect(screen.getByText('Copiado')).toBeInTheDocument()
    expect(screen.queryByText('No se pudo copiar la respuesta.')).not.toBeInTheDocument()
  })

  it('keeps canonical conversation rows and controls visible when the first search fails', async () => {
    setupBackend({
      storedActiveId: 'conv-1',
      summaries: conversationSummaries,
      searchError: new Error('search failed'),
      conversations: { 'conv-1': storedConversation },
    })

    render(RagChatView)
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /¿Cuándo comenzó la huelga\?/ })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: /Salarios del SOIP/ })).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Buscar conversaciones' }))
    const search = screen.getByRole('searchbox', { name: 'Buscar conversaciones' })
    await fireEvent.input(search, { target: { value: 'fallido' } })

    await waitFor(() => {
      expect(callsFor('rag_search_conversations')).toHaveLength(1)
      expect(screen.getByRole('alert')).toHaveTextContent(
        'No se pudieron buscar las conversaciones.',
      )
    })

    expect(screen.getByRole('button', { name: /¿Cuándo comenzó la huelga\?/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Salarios del SOIP/ })).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: 'Descargar conversación en PDF' })).toHaveLength(2)
    expect(screen.getAllByRole('button', { name: 'Eliminar conversación' })).toHaveLength(2)
  })

  it('keeps prior filtered conversations visible when search fails', async () => {
    const state = setupBackend({
      storedActiveId: 'conv-1',
      summaries: conversationSummaries,
      searchResults: [conversationSummaries[1]!],
      conversations: { 'conv-1': storedConversation },
    })

    render(RagChatView)
    await waitFor(() => {
      expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Buscar conversaciones' }))
    const search = screen.getByRole('searchbox', { name: 'Buscar conversaciones' })
    await fireEvent.input(search, { target: { value: 'salarios' } })
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /Salarios del SOIP/ })).toBeInTheDocument()
      expect(callsFor('rag_search_conversations')).toHaveLength(1)
    })

    state.searchError = new Error('search failed')
    await fireEvent.input(search, { target: { value: 'salarios fallido' } })
    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(
        'No se pudieron buscar las conversaciones.',
      )
    })

    expect(screen.getByRole('button', { name: /Salarios del SOIP/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Descargar conversación en PDF' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Eliminar conversación' })).toBeInTheDocument()
  })



  it('keeps retrying downloads disabled until the current attempt finishes', async () => {
    setupBackend({
      storedActiveId: 'conv-1',
      summaries: conversationSummaries,
      conversations: { 'conv-1': storedConversation },
    })
    const first = deferred<string>()
    const second = deferred<string>()
    downloadRagConversationPdfMock.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)

    render(RagChatView)
    await waitFor(() => {
      expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()
    })

    const downloadButton = screen.getAllByRole('button', {
      name: 'Descargar conversación en PDF',
    })[1]!

    vi.useFakeTimers()
    try {
      await fireEvent.click(downloadButton)
      expect(downloadButton).toBeDisabled()

      first.resolve('first.pdf')
      await vi.advanceTimersByTimeAsync(0)
      expect(downloadButton).not.toBeDisabled()

      await fireEvent.click(downloadButton)
      expect(downloadButton).toBeDisabled()

      await vi.advanceTimersByTimeAsync(1800)
      expect(downloadButton).toBeDisabled()

      second.resolve('second.pdf')
      await vi.advanceTimersByTimeAsync(0)
      expect(downloadButton).not.toBeDisabled()
    } finally {
      vi.useRealTimers()
    }
  })



  it('searches message text without changing the active conversation', async () => {
    setupBackend({
      storedActiveId: 'conv-1',
      summaries: conversationSummaries,
      searchResults: [conversationSummaries[1]!],
      conversations: { 'conv-1': storedConversation },
    })

    render(RagChatView)
    await waitFor(() => {
      expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Buscar conversaciones' }))
    const search = screen.getByRole('searchbox', { name: 'Buscar conversaciones' })
    await fireEvent.input(search, { target: { value: 'jornal' } })
    await waitFor(() => {
      expect(callsFor('rag_search_conversations')).toEqual([
        ['rag_search_conversations', { query: 'jornal' }],
      ])
    })

    expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: /¿Cuándo comenzó la huelga\?/ })
    ).not.toBeInTheDocument()
    expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
    expect(callsFor('rag_get_conversation')).toEqual([
      ['rag_get_conversation', { conversationId: 'conv-1' }],
    ])
  })
  it('clears conversation search without closing the panel or changing the active conversation', async () => {
    setupBackend({
      storedActiveId: 'conv-1',
      summaries: conversationSummaries,
      searchResults: [conversationSummaries[1]!],
      conversations: { 'conv-1': storedConversation },
    })

    render(RagChatView)
    await waitFor(() => {
      expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Buscar conversaciones' }))
    const searchInput = screen.getByRole('searchbox', { name: 'Buscar conversaciones' })
    await fireEvent.input(searchInput, { target: { value: 'research' } })
    await tick()

    const clearButton = screen.getByRole('button', { name: 'Limpiar búsqueda' })
    expect(clearButton).toHaveAttribute('title', 'Limpiar búsqueda')
    await fireEvent.click(clearButton)

    expect(searchInput).toHaveValue('')
    expect(screen.getByRole('searchbox', { name: 'Buscar conversaciones' })).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: /¿Cuándo comenzó la huelga\?/ }),
    ).toHaveAttribute('aria-current', 'true')
    expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
  })


  it('downloads the conversation attached to a non-active row', async () => {
    setupBackend({
      storedActiveId: 'conv-1',
      summaries: conversationSummaries,
      conversations: { 'conv-1': storedConversation },
    })
    downloadRagConversationPdfMock.mockResolvedValue(
      'C:/Users/test/Downloads/Salarios - conv-2.pdf'
    )

    render(RagChatView)
    await waitFor(() => {
      expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()
    })

    const downloadButtons = screen.getAllByRole('button', {
      name: 'Descargar conversación en PDF',
    })
    expect(downloadButtons[1]).toHaveAttribute('title', 'Descargar conversación en PDF')
    await fireEvent.click(downloadButtons[1]!)

    expect(downloadRagConversationPdfMock).toHaveBeenCalledWith('conv-2')
    expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
  })
  it('renders edit, download, and delete actions in that order with the shared edit icon', async () => {
    setupBackend({ summaries: conversationSummaries })
    render(RagChatView)

    const title = await screen.findByText('¿Cuándo comenzó la huelga?')
    const row = title.closest('.rag-chat__conversation')!
    const actions = row.querySelector('.rag-chat__conversation-actions')!
    const buttons = Array.from(actions.querySelectorAll('button'))

    expect(buttons).toHaveLength(3)
    expect(buttons[0]).toHaveAttribute('aria-label', 'Editar nombre de la conversación')
    expect(buttons[0]).toHaveAttribute('title', 'Editar nombre de la conversación')
    expect(buttons[0]?.querySelector('svg')).not.toBeNull()
    expect(buttons[1]).toHaveAttribute('aria-label', 'Descargar conversación en PDF')
    expect(buttons[2]).toHaveAttribute('aria-label', 'Eliminar conversación')
  })

  it('enters inline editing without selecting the conversation and saves a trimmed title with Enter', async () => {
    setupBackend({
      summaries: conversationSummaries,
      conversations: { 'conv-1': storedConversation },
    })
    render(RagChatView)

    const editButtons = await screen.findAllByRole('button', { name: 'Editar nombre de la conversación' })
    await fireEvent.click(editButtons[0]!)
    const input = screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })
    expect(input).toHaveValue('¿Cuándo comenzó la huelga?')

    await fireEvent.input(input, { target: { value: '  Título renovado  ' } })
    await fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() =>
      expect(callsFor('rag_update_conversation_title')).toEqual([
        ['rag_update_conversation_title', { conversationId: 'conv-1', title: 'Título renovado' }],
      ]),
    )
    expect(await screen.findByText('Título renovado')).toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: 'Editar nombre de la conversación' })).not.toBeInTheDocument()
    expect(callsFor('rag_get_conversation')).toHaveLength(1)
  })

  it('cancels inline editing with Escape without persisting', async () => {
    setupBackend({ summaries: conversationSummaries })
    render(RagChatView)

    const editButtons = await screen.findAllByRole('button', { name: 'Editar nombre de la conversación' })
    await fireEvent.click(editButtons[0]!)
    const input = screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })
    await fireEvent.input(input, { target: { value: 'No guardar' } })
    await fireEvent.keyDown(input, { key: 'Escape' })

    expect(screen.queryByRole('textbox', { name: 'Editar nombre de la conversación' })).not.toBeInTheDocument()
    expect(screen.getByText('¿Cuándo comenzó la huelga?')).toBeInTheDocument()
    expect(callsFor('rag_update_conversation_title')).toHaveLength(0)
  })

  it('rejects an empty title and keeps the input open', async () => {
    setupBackend({ summaries: conversationSummaries })
    render(RagChatView)

    const editButtons = await screen.findAllByRole('button', { name: 'Editar nombre de la conversación' })
    await fireEvent.click(editButtons[0]!)
    const input = screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })
    await fireEvent.input(input, { target: { value: '   ' } })
    await fireEvent.keyDown(input, { key: 'Enter' })

    expect(screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })).toHaveValue('   ')
    expect(screen.getByRole('alert')).toHaveTextContent('El nombre de la conversación no puede estar vacío.')
    expect(callsFor('rag_update_conversation_title')).toHaveLength(0)
  })

  it('keeps the entered title when SQLite persistence fails', async () => {
    setupBackend({
      summaries: conversationSummaries,
      renameError: 'No se pudo guardar el nombre de la conversación.',
    })
    render(RagChatView)

    const editButtons = await screen.findAllByRole('button', { name: 'Editar nombre de la conversación' })
    await fireEvent.click(editButtons[0]!)
    const input = screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })
    await fireEvent.input(input, { target: { value: 'Título pendiente' } })
    await fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() =>
      expect(
        screen.getByRole('textbox', { name: 'Editar nombre de la conversación' }),
      ).toHaveValue('Título pendiente'),
    )
    expect(screen.getByRole('alert')).toHaveTextContent('No se pudo guardar el nombre de la conversación.')
  })
  it('disables the title input while rename persistence is pending', async () => {
    const renameResponse = deferred<void>()
    setupBackend({
      summaries: conversationSummaries,
      renamePromise: renameResponse.promise,
    })
    render(RagChatView)

    const editButtons = await screen.findAllByRole('button', { name: 'Editar nombre de la conversación' })
    await fireEvent.click(editButtons[0]!)
    const input = screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })
    await fireEvent.input(input, { target: { value: 'Título pendiente' } })
    await fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() =>
      expect(callsFor('rag_update_conversation_title')).toHaveLength(1),
    )
    expect(screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })).toBeDisabled()
    expect(screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })).toHaveValue(
      'Título pendiente',
    )

    renameResponse.resolve()
    await renameResponse.promise
    await waitFor(() =>
      expect(
        screen.queryByRole('textbox', { name: 'Editar nombre de la conversación' }),
      ).not.toBeInTheDocument(),
    )
  })

  it('shows the title error globally while conversation search is loading', async () => {
    const searchResponse = deferred<RagConversationSummary[]>()
    const titleError = 'No se pudo guardar el nombre de la conversación.'
    setupBackend({
      summaries: conversationSummaries,
      renameError: titleError,
      searchPromise: searchResponse.promise,
    })
    render(RagChatView)

    const editButtons = await screen.findAllByRole('button', { name: 'Editar nombre de la conversación' })
    await fireEvent.click(editButtons[0]!)
    const input = screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })
    await fireEvent.input(input, { target: { value: 'Título pendiente' } })
    await fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent(titleError))

    await fireEvent.click(screen.getByRole('button', { name: 'Buscar conversaciones' }))
    await fireEvent.input(screen.getByRole('searchbox', { name: 'Buscar conversaciones' }), {
      target: { value: 'huelga' },
    })
    await waitFor(() => expect(callsFor('rag_search_conversations')).toHaveLength(1))

    expect(screen.getByText('Buscando conversaciones…')).toBeInTheDocument()
    expect(screen.getByRole('alert')).toHaveTextContent(titleError)

    searchResponse.resolve([])
    await searchResponse.promise
  })

 
  it('shows a later unrelated composer error after title persistence fails', async () => {
    const laterError = 'La consulta no pudo completarse.'
    setupBackend({
      summaries: conversationSummaries,
      renameError: 'No se pudo guardar el nombre de la conversación.',
      ask: () => Promise.reject(laterError),
    })
    render(RagChatView)

    const editButtons = await screen.findAllByRole('button', { name: 'Editar nombre de la conversación' })
    await fireEvent.click(editButtons[0]!)
    const input = screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })
    await fireEvent.input(input, { target: { value: 'Título pendiente' } })
    await fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent(
        'No se pudo guardar el nombre de la conversación.',
      ),
    )

    await sendQuestion('otra pregunta')

    await waitFor(() => {
      expect(screen.getByText(laterError)).toBeInTheDocument()
      expect(screen.getAllByRole('alert')).toHaveLength(2)
    })
  })
  it('shows a same-text composer error after title persistence fails', async () => {
    const sharedError = 'No se pudo guardar el nombre de la conversación.'
    setupBackend({
      summaries: conversationSummaries,
      renameError: sharedError,
      ask: () => Promise.reject(sharedError),
    })
    render(RagChatView)

    const editButtons = await screen.findAllByRole('button', { name: 'Editar nombre de la conversación' })
    await fireEvent.click(editButtons[0]!)
    const input = screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })
    await fireEvent.input(input, { target: { value: 'Título pendiente' } })
    await fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent(sharedError),
    )

    await sendQuestion('otra pregunta')

    await waitFor(() => expect(screen.getAllByRole('alert')).toHaveLength(2))
  })

  it.each([
    ['composition flag', { key: 'Enter', isComposing: true }],
    ['legacy IME key code', { key: 'Enter', keyCode: 229 }],
  ])('does not save the title on an IME Enter signal (%s)', async (_signal, event) => {
    setupBackend({ summaries: conversationSummaries })
    render(RagChatView)

    const editButtons = await screen.findAllByRole('button', { name: 'Editar nombre de la conversación' })
    await fireEvent.click(editButtons[0]!)
    const input = screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })
    await fireEvent.input(input, { target: { value: 'Título IME' } })
    await fireEvent.keyDown(input, event)

    expect(callsFor('rag_update_conversation_title')).toHaveLength(0)
    expect(screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })).toHaveValue(
      'Título IME',
    )
  })

  it('reconciles the renamed title while a conversation search is active', async () => {
    const state = setupBackend({
      summaries: conversationSummaries,
      searchResults: [conversationSummaries[0]!],
      conversations: { 'conv-1': storedConversation },
    })
    render(RagChatView)

    await fireEvent.click(screen.getByRole('button', { name: 'Buscar conversaciones' }))
    await fireEvent.input(screen.getByRole('searchbox', { name: 'Buscar conversaciones' }), {
      target: { value: 'huelga' },
    })
    await waitFor(() => expect(screen.getByText('¿Cuándo comenzó la huelga?')).toBeInTheDocument())

    const editButton = await screen.findByRole('button', { name: 'Editar nombre de la conversación' })
    await fireEvent.click(editButton)
    const input = screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })
    await fireEvent.input(input, { target: { value: 'Título buscable' } })
    await fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => expect(screen.getByText('Título buscable')).toBeInTheDocument())
    expect(state.summaries[0]?.id).toBe('conv-1')
  })

  it('reconciles a late search response with the canonical renamed title', async () => {
    const renameResponse = deferred<void>()
    const searchResponse = deferred<RagConversationSummary[]>()
    setupBackend({
      summaries: conversationSummaries,
      searchPromise: searchResponse.promise,
      renamePromise: renameResponse.promise,
      conversations: { 'conv-1': storedConversation },
    })
    render(RagChatView)

    const editButtons = await screen.findAllByRole('button', { name: 'Editar nombre de la conversación' })
    await fireEvent.click(editButtons[0]!)
    const input = screen.getByRole('textbox', { name: 'Editar nombre de la conversación' })
    await fireEvent.input(input, { target: { value: 'Título buscable' } })
    await fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() =>
      expect(callsFor('rag_update_conversation_title')).toHaveLength(1),
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Buscar conversaciones' }))
    const search = screen.getByRole('searchbox', { name: 'Buscar conversaciones' })
    await fireEvent.input(search, { target: { value: 'huelga' } })
    await waitFor(() => expect(callsFor('rag_search_conversations')).toHaveLength(1))

    renameResponse.resolve()
    await renameResponse.promise
    await waitFor(() =>
      expect(
        screen.queryByRole('textbox', { name: 'Editar nombre de la conversación' }),
      ).not.toBeInTheDocument(),
    )

    searchResponse.resolve([conversationSummaries[0]!])
    await searchResponse.promise

    await waitFor(() => {
      expect(screen.getByText('Título buscable')).toBeInTheDocument()
      expect(
        screen.queryByRole('button', { name: /¿Cuándo comenzó la huelga\?/ }),
      ).not.toBeInTheDocument()
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
  it('removes a deleted conversation from active search results', async () => {
    const state = setupBackend({
      storedActiveId: 'conv-1',
      summaries: conversationSummaries,
      searchResults: [conversationSummaries[1]!],
      conversations: { 'conv-1': storedConversation },
    })

    render(RagChatView)
    await waitFor(() => {
      expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Buscar conversaciones' }))
    const search = screen.getByRole('searchbox', { name: 'Buscar conversaciones' })
    await fireEvent.input(search, { target: { value: 'salarios' } })
    await waitFor(() => {
      expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()
      expect(callsFor('rag_search_conversations')).toEqual([
        ['rag_search_conversations', { query: 'salarios' }],
      ])
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar conversación' }))
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
    expect(
      screen.queryByRole('button', { name: 'Descargar conversación en PDF' }),
    ).not.toBeInTheDocument()
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
