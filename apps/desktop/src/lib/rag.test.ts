import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import {
  ragAsk,
  ragDeleteConversation,
  ragGetConversation,
  ragRenameConversation,
  ragListConversations,
  type RagAnswer,
  type RagConversation,
  type RagConversationSummary,
} from './rag'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

const mockInvoke = vi.mocked(invoke)

const sampleAnswer: RagAnswer = {
  answer: 'La huelga comenzó en 1966 [1].',
  sources: [
    {
      index: 1,
      assetId: 'asset-1',
      itemId: 'item-1',
      itemTitle: 'Entrevista 12',
      collectionId: 'col-1',
      collectionName: 'Historia oral',
      snippet: 'la huelga comenzó cuando...',
      score: 0.91,
      startSeconds: 65,
      endSeconds: 80,
      provenance: null,
    },
  ],
  model: 'test-model',
  conversationId: 'conv-1',
}

beforeEach(() => {
  vi.clearAllMocks()
})

describe('ragAsk', () => {
  it('invokes rag_ask with the exact command and payload', async () => {
    mockInvoke.mockResolvedValueOnce(sampleAnswer)

    const result = await ragAsk('¿Quién la lideró?', 'conv-1')

    expect(mockInvoke).toHaveBeenCalledTimes(1)
    expect(mockInvoke).toHaveBeenCalledWith('rag_ask', {
      question: '¿Quién la lideró?',
      conversationId: 'conv-1',
      topK: undefined,
    })
    expect(result).toEqual(sampleAnswer)
  })

  it('sends an undefined conversationId for a brand-new conversation', async () => {
    mockInvoke.mockResolvedValueOnce(sampleAnswer)

    await ragAsk('¿Cuándo comenzó la huelga?')

    expect(mockInvoke).toHaveBeenCalledWith('rag_ask', {
      question: '¿Cuándo comenzó la huelga?',
      conversationId: undefined,
      topK: undefined,
    })
  })

  it('forwards topK when provided', async () => {
    mockInvoke.mockResolvedValueOnce(sampleAnswer)

    await ragAsk('pregunta', 'conv-1', 8)

    expect(mockInvoke).toHaveBeenCalledWith('rag_ask', {
      question: 'pregunta',
      conversationId: 'conv-1',
      topK: 8,
    })
  })

  it('propagates backend rejections untouched', async () => {
    const backendError = 'Falta la API key de OpenRouter. Configurala en Configuración.'
    mockInvoke.mockRejectedValueOnce(backendError)

    await expect(ragAsk('pregunta')).rejects.toBe(backendError)
  })
})

describe('ragListConversations', () => {
  it('invokes rag_list_conversations without payload and returns the summaries', async () => {
    const summaries: RagConversationSummary[] = [
      {
        id: 'conv-2',
        title: 'Última',
        createdAt: 1700000200000,
        updatedAt: 1700000300000,
        messageCount: 4,
      },
      {
        id: 'conv-1',
        title: 'Primera',
        createdAt: 1700000000000,
        updatedAt: 1700000100000,
        messageCount: 2,
      },
    ]
    mockInvoke.mockResolvedValueOnce(summaries)

    const result = await ragListConversations()

    expect(mockInvoke).toHaveBeenCalledTimes(1)
    expect(mockInvoke).toHaveBeenCalledWith('rag_list_conversations')
    expect(result).toEqual(summaries)
  })
})

describe('ragGetConversation', () => {
  it('invokes rag_get_conversation with the conversation id', async () => {
    const conversation: RagConversation = {
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
          content: 'La huelga comenzó en 1966 [1].',
          sources: sampleAnswer.sources,
          createdAt: 1700000001000,
        },
      ],
    }
    mockInvoke.mockResolvedValueOnce(conversation)

    const result = await ragGetConversation('conv-1')

    expect(mockInvoke).toHaveBeenCalledTimes(1)
    expect(mockInvoke).toHaveBeenCalledWith('rag_get_conversation', { conversationId: 'conv-1' })
    expect(result).toEqual(conversation)
  })
})

describe('ragDeleteConversation', () => {
  it('invokes rag_delete_conversation with the conversation id', async () => {
    mockInvoke.mockResolvedValueOnce(undefined)

    await ragDeleteConversation('conv-1')

    expect(mockInvoke).toHaveBeenCalledTimes(1)
    expect(mockInvoke).toHaveBeenCalledWith('rag_delete_conversation', { conversationId: 'conv-1' })
  })

  it('propagates backend rejections untouched', async () => {
    const backendError = 'No se encontró la conversación.'
    mockInvoke.mockRejectedValueOnce(backendError)

    await expect(ragDeleteConversation('conv-x')).rejects.toBe(backendError)
  })
})

describe('ragRenameConversation', () => {
  it('invokes rag_update_conversation_title with the trimmed title', async () => {
    mockInvoke.mockResolvedValueOnce(undefined)

    await ragRenameConversation('conv-1', '  Título nuevo  ')

    expect(mockInvoke).toHaveBeenCalledWith('rag_update_conversation_title', {
      conversationId: 'conv-1',
      title: 'Título nuevo',
    })
  })

  it('propagates backend rejections untouched', async () => {
    const backendError = 'La conversación no existe o fue eliminada.'
    mockInvoke.mockRejectedValueOnce(backendError)

    await expect(ragRenameConversation('conv-x', 'Nuevo')).rejects.toBe(backendError)
  })
})
