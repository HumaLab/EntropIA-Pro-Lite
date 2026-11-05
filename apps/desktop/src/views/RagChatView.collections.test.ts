import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { get } from 'svelte/store'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import type { RagAnswer } from '$lib/rag'
import { ragChat } from '$lib/rag-chat'
import RagChatView from './RagChatView.svelte'

const { storeMock } = vi.hoisted(() => ({
  storeMock: {
    collections: {
      findAll: vi.fn(),
      countItems: vi.fn(),
    },
  },
}))

vi.mock('$lib/db', () => ({ getStore: () => storeMock }))

vi.mock('$lib/pane-context', () => ({
  getNavigation: () => ({ navigate: vi.fn(), openRootSection: vi.fn() }),
  getPaneId: () => 'pane-test',
}))

vi.mock('$lib/rag-chat-export', () => ({ downloadRagConversationPdf: vi.fn() }))

const mockInvoke = vi.mocked(invoke)

const answer: RagAnswer = {
  answer: 'Respuesta.',
  sources: [],
  model: 'test-model',
  conversationId: 'conv-new',
}

function setupBackend(ask: (args: Record<string, unknown>) => RagAnswer = () => answer) {
  const askMock = vi.fn(ask)
  mockInvoke.mockImplementation((async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case 'settings_get':
      case 'rag_generate_conversation_title':
        return null
      case 'settings_set':
      case 'settings_delete':
        return undefined
      case 'rag_list_conversations':
        return []
      case 'rag_chunk_coverage':
        return { assetsWithText: 0, assetsWithChunks: 0 }
      case 'rag_ask':
        return askMock(args as Record<string, unknown>)
      case 'bibliography_library_status':
        return { libraries: [], vectorReady: false }
      default:
        throw new Error(`unexpected command: ${command}`)
    }
  }) as typeof invoke)
  return askMock
}

async function ask(question: string) {
  const composer = screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })
  await fireEvent.input(composer, { target: { value: question } })
  await fireEvent.keyDown(composer, { key: 'Enter' })
}

beforeEach(() => {
  locale.set('es')
  mockInvoke.mockReset()
  ragChat.reset()
  storeMock.collections.findAll.mockReset()
  storeMock.collections.countItems.mockReset()
  storeMock.collections.findAll.mockResolvedValue([
    { id: 'col-1', name: 'Archivo General' },
    { id: 'col-2', name: 'Hemeroteca' },
  ])
  storeMock.collections.countItems.mockImplementation(async (id: string) =>
    id === 'col-1' ? 12 : 3
  )
})

describe('RagChatView collection choice', () => {
  it('offers every collection on the Corpus and sends no collections while "all" is chosen', async () => {
    const askMock = setupBackend()
    render(RagChatView)

    expect(await screen.findByRole('button', { name: /Todas las colecciones/ })).toBeVisible()
    await ask('¿Qué pasó?')

    await waitFor(() => expect(askMock).toHaveBeenCalled())
    expect(askMock.mock.calls[0]![0].collections).toBeUndefined()
  })

  it('narrows the question to the collections left checked', async () => {
    const askMock = setupBackend()
    render(RagChatView)

    await fireEvent.click(await screen.findByRole('button', { name: /Todas las colecciones/ }))
    const archive = await screen.findByRole('menuitemcheckbox', { name: /Archivo General · 12/ })
    expect(archive).toHaveAttribute('aria-checked', 'true')
    await fireEvent.click(archive)

    await waitFor(() =>
      expect(screen.getByRole('button', { name: /1 de 2 colecciones/ })).toBeInTheDocument()
    )
    await ask('¿Qué pasó?')

    await waitFor(() => expect(askMock).toHaveBeenCalled())
    expect(askMock.mock.calls[0]![0].collections).toEqual(['col-2'])
  })

  it('keeps the collection choice with both scopes and hides it on the Biblioteca', async () => {
    setupBackend()
    render(RagChatView)
    await screen.findByRole('button', { name: /Todas las colecciones/ })

    await fireEvent.click(screen.getByRole('tab', { name: 'Ambos' }))
    expect(screen.getByRole('button', { name: /Todas las colecciones/ })).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('tab', { name: 'Biblioteca' }))
    expect(screen.queryByRole('button', { name: /colecciones/ })).toBeNull()
  })

  it('forgets a chosen collection that no longer exists', async () => {
    ragChat.setCollections(['col-gone', 'col-2'])
    setupBackend()
    render(RagChatView)

    expect(await screen.findByRole('button', { name: /1 de 2 colecciones/ })).toBeVisible()
    await waitFor(() => expect(get(ragChat).collections).toEqual(['col-2']))
  })

  it('hides the choice when the corpus has no collections', async () => {
    storeMock.collections.findAll.mockResolvedValue([])
    setupBackend()
    render(RagChatView)

    await waitFor(() => expect(storeMock.collections.findAll).toHaveBeenCalled())
    expect(screen.queryByRole('button', { name: /colecciones/ })).toBeNull()
  })
})
