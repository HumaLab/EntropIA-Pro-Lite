import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from './i18n'
import type { RagAnswer, RagConversation, RagConversationSummary } from './rag'
import { RagChatStore, type RagChatSnapshot } from './rag-chat'

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
  rename?: (args: { conversationId: string; title: string }) => Promise<void> | void
  /** Override opcional del listado (para diferir o fallar la carga). */
  list?: () => Promise<RagConversationSummary[]> | RagConversationSummary[]
}

function summary(id: string, title: string, updatedAt: number): RagConversationSummary {
  return { id, title, createdAt: updatedAt - 1000, updatedAt, messageCount: 2 }
}

function conversation(id: string, title: string): RagConversation {
  return {
    id,
    title,
    messages: [
      { id: `${id}-m1`, role: 'user', content: `pregunta de ${id}`, sources: [], createdAt: 1 },
      { id: `${id}-m2`, role: 'assistant', content: `respuesta de ${id}`, sources: [], createdAt: 2 },
    ],
  }
}

function answer(
  conversationId: string | null,
  text = 'La huelga comenzó en 1966 [1].'
): RagAnswer {
  return { answer: text, sources: [], model: 'test-model', conversationId }
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
        return state.list ? state.list() : state.summaries
      case 'rag_get_conversation': {
        const found = state.conversations[args?.conversationId as string]
        if (!found) throw 'No se encontró la conversación.'
        return found
      }
      case 'rag_ask':
        return state.ask(args as { question: string; conversationId?: string })
      case 'rag_delete_conversation':
        return undefined
      case 'rag_update_conversation_title':
        await state.rename?.(args as { conversationId: string; title: string })
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

function snapshotOf(store: RagChatStore): RagChatSnapshot {
  let snapshot!: RagChatSnapshot
  const unsubscribe = store.subscribe((value) => {
    snapshot = value
  })
  unsubscribe()
  return snapshot
}

beforeEach(() => {
  locale.set('es')
  mockInvoke.mockReset()
})

describe('RagChatStore.initialize', () => {
  it('restores the active id from settings and rehydrates its messages', async () => {
    setupBackend({
      storedActiveId: 'conv-1',
      summaries: [summary('conv-2', 'Última', 2000), summary('conv-1', 'Primera', 1000)],
      conversations: { 'conv-1': conversation('conv-1', 'Primera') },
    })
    const store = new RagChatStore()

    await store.initialize()

    expect(callsFor('rag_get_conversation')).toEqual([
      ['rag_get_conversation', { conversationId: 'conv-1' }],
    ])
    const snapshot = snapshotOf(store)
    expect(snapshot.initialized).toBe(true)
    expect(snapshot.activeConversationId).toBe('conv-1')
    expect(snapshot.conversations.map((c) => c.id)).toEqual(['conv-2', 'conv-1'])
    expect(snapshot.messages).toEqual([
      { role: 'user', content: 'pregunta de conv-1', sources: [] },
      { role: 'assistant', content: 'respuesta de conv-1', sources: [] },
    ])
  })

  it('falls back to the most recent conversation when no setting is stored', async () => {
    setupBackend({
      summaries: [summary('conv-2', 'Última', 2000), summary('conv-1', 'Primera', 1000)],
      conversations: { 'conv-2': conversation('conv-2', 'Última') },
    })
    const store = new RagChatStore()

    await store.initialize()

    const snapshot = snapshotOf(store)
    expect(snapshot.activeConversationId).toBe('conv-2')
    expect(snapshot.messages[0]?.content).toBe('pregunta de conv-2')
    expect(callsFor('settings_set')).toEqual([
      ['settings_set', { key: 'rag_active_conversation', value: 'conv-2' }],
    ])
  })

  it('falls back to the most recent conversation when the stored id is stale', async () => {
    setupBackend({
      storedActiveId: 'conv-gone',
      summaries: [summary('conv-2', 'Última', 2000)],
      conversations: { 'conv-2': conversation('conv-2', 'Última') },
    })
    const store = new RagChatStore()

    await store.initialize()

    expect(snapshotOf(store).activeConversationId).toBe('conv-2')
    expect(callsFor('rag_get_conversation')).toEqual([
      ['rag_get_conversation', { conversationId: 'conv-2' }],
    ])
  })

  it('starts empty when there are no conversations', async () => {
    setupBackend()
    const store = new RagChatStore()

    await store.initialize()

    const snapshot = snapshotOf(store)
    expect(snapshot.initialized).toBe(true)
    expect(snapshot.activeConversationId).toBeNull()
    expect(snapshot.messages).toEqual([])
    expect(snapshot.conversations).toEqual([])
    expect(callsFor('rag_get_conversation')).toHaveLength(0)
  })

  it('is idempotent — a second call does not reload anything', async () => {
    setupBackend({ summaries: [] })
    const store = new RagChatStore()

    await store.initialize()
    await store.initialize()

    expect(callsFor('rag_list_conversations')).toHaveLength(1)
  })

  it('retries the bootstrap after a failed list load instead of memoizing the failure', async () => {
    const state = setupBackend({
      summaries: [summary('conv-1', 'Primera', 1000)],
      conversations: { 'conv-1': conversation('conv-1', 'Primera') },
    })
    let listCalls = 0
    state.list = () => {
      listCalls += 1
      if (listCalls === 1) throw 'No se pudo cargar el listado.'
      return state.summaries
    }
    const store = new RagChatStore()

    await store.initialize()
    let snapshot = snapshotOf(store)
    expect(snapshot.initialized).toBe(false)
    expect(snapshot.error).toBe('No se pudo cargar el listado.')
    expect(snapshot.conversations).toEqual([])

    await store.initialize()
    snapshot = snapshotOf(store)
    expect(snapshot.initialized).toBe(true)
    expect(snapshot.activeConversationId).toBe('conv-1')
    expect(snapshot.messages[0]?.content).toBe('pregunta de conv-1')
    expect(callsFor('rag_list_conversations')).toHaveLength(2)
  })

  it('cleans the stale stored id when the conversation list is empty', async () => {
    setupBackend({ storedActiveId: 'conv-fantasma', summaries: [] })
    const store = new RagChatStore()

    await store.initialize()

    const snapshot = snapshotOf(store)
    expect(snapshot.initialized).toBe(true)
    expect(snapshot.activeConversationId).toBeNull()
    expect(callsFor('settings_delete')).toEqual([
      ['settings_delete', { key: 'rag_active_conversation' }],
    ])
  })
})

describe('RagChatStore.send', () => {
  it('adopts the returned conversationId, persists it and refreshes the list', async () => {
    const state = setupBackend({
      ask: ({ conversationId }) => {
        expect(conversationId).toBeUndefined()
        state.summaries = [summary('conv-new', '¿Cuándo comenzó la huelga?', 3000)]
        return answer('conv-new')
      },
    })
    const store = new RagChatStore()
    await store.initialize()

    await store.send('¿Cuándo comenzó la huelga?')

    expect(callsFor('rag_ask')).toEqual([
      [
        'rag_ask',
        { question: '¿Cuándo comenzó la huelga?', conversationId: undefined, topK: undefined },
      ],
    ])
    expect(callsFor('settings_set')).toEqual([
      ['settings_set', { key: 'rag_active_conversation', value: 'conv-new' }],
    ])
    // Initialize + post-send refresh.
    expect(callsFor('rag_list_conversations')).toHaveLength(2)

    const snapshot = snapshotOf(store)
    expect(snapshot.activeConversationId).toBe('conv-new')
    expect(snapshot.conversations.map((c) => c.id)).toEqual(['conv-new'])
    expect(snapshot.messages).toEqual([
      { role: 'user', content: '¿Cuándo comenzó la huelga?' },
      { role: 'assistant', content: 'La huelga comenzó en 1966 [1].', sources: [] },
    ])
    expect(snapshot.loading).toBe(false)
    expect(snapshot.draft).toBe('')
  })

  it('sends within the active conversation', async () => {
    const state = setupBackend({
      storedActiveId: 'conv-1',
      summaries: [summary('conv-1', 'Primera', 1000)],
      conversations: { 'conv-1': conversation('conv-1', 'Primera') },
    })
    state.ask = () => answer('conv-1', 'Liderada por la comisión interna.')
    const store = new RagChatStore()
    await store.initialize()

    await store.send('¿Quién la lideró?')

    expect(callsFor('rag_ask')).toEqual([
      ['rag_ask', { question: '¿Quién la lideró?', conversationId: 'conv-1', topK: undefined }],
    ])
    const snapshot = snapshotOf(store)
    expect(snapshot.messages).toHaveLength(4)
    expect(snapshot.messages.at(-1)).toEqual({
      role: 'assistant',
      content: 'Liderada por la comisión interna.',
      sources: [],
    })
  })

  it('waits for initialization: rehydration lands first and the optimistic message survives', async () => {
    const pendingList = deferred<RagConversationSummary[]>()
    const state = setupBackend({
      storedActiveId: 'conv-1',
      summaries: [summary('conv-1', 'Primera', 1000)],
      conversations: { 'conv-1': conversation('conv-1', 'Primera') },
    })
    let listCalls = 0
    state.list = () => {
      listCalls += 1
      return listCalls === 1 ? pendingList.promise : state.summaries
    }
    state.ask = ({ conversationId }) => {
      // El send capturó el id DESPUÉS de la rehidratación.
      expect(conversationId).toBe('conv-1')
      return answer('conv-1', 'respuesta nueva')
    }
    const store = new RagChatStore()

    // Mount + envío inmediato, antes de que el bootstrap resuelva.
    const initPromise = store.initialize()
    const sendPromise = store.send('pregunta optimista')

    pendingList.resolve(state.summaries)
    await initPromise
    await sendPromise

    const snapshot = snapshotOf(store)
    expect(snapshot.loading).toBe(false)
    expect(snapshot.activeConversationId).toBe('conv-1')
    expect(snapshot.messages).toEqual([
      { role: 'user', content: 'pregunta de conv-1', sources: [] },
      { role: 'assistant', content: 'respuesta de conv-1', sources: [] },
      { role: 'user', content: 'pregunta optimista' },
      { role: 'assistant', content: 'respuesta nueva', sources: [] },
    ])
  })

  it('keeps the answer and the active id when persistence failed (null conversationId)', async () => {
    const state = setupBackend({
      storedActiveId: 'conv-1',
      summaries: [summary('conv-1', 'Primera', 1000)],
      conversations: { 'conv-1': conversation('conv-1', 'Primera') },
    })
    state.ask = () => answer(null, 'respuesta sin persistir')
    const store = new RagChatStore()
    await store.initialize()

    await store.send('pregunta')

    const snapshot = snapshotOf(store)
    expect(snapshot.activeConversationId).toBe('conv-1')
    expect(snapshot.messages.at(-1)).toEqual({
      role: 'assistant',
      content: 'respuesta sin persistir',
      sources: [],
    })
    expect(snapshot.loading).toBe(false)
    expect(snapshot.error).toBeNull()
    // No se persiste un id que el backend no pudo crear...
    expect(callsFor('settings_set')).toHaveLength(0)
    // ...pero el listado igual se refresca (init + post-send).
    expect(callsFor('rag_list_conversations')).toHaveLength(2)
  })

  it('surfaces backend errors and stops loading', async () => {
    const state = setupBackend()
    state.ask = () => Promise.reject('Falta la API key de OpenRouter.')
    const store = new RagChatStore()
    await store.initialize()

    await store.send('pregunta')

    const snapshot = snapshotOf(store)
    expect(snapshot.error).toBe('Falta la API key de OpenRouter.')
    expect(snapshot.loading).toBe(false)
    expect(snapshot.messages).toEqual([{ role: 'user', content: 'pregunta' }])
  })

  it('discards a stale response when another conversation was selected meanwhile', async () => {
    const pendingAsk = deferred<RagAnswer>()
    const state = setupBackend({
      storedActiveId: 'conv-a',
      summaries: [summary('conv-a', 'A', 2000), summary('conv-b', 'B', 1000)],
      conversations: {
        'conv-a': conversation('conv-a', 'A'),
        'conv-b': conversation('conv-b', 'B'),
      },
    })
    state.ask = () => pendingAsk.promise
    const store = new RagChatStore()
    await store.initialize()

    const sendPromise = store.send('pregunta tardía')
    // El envío arranca de verdad (rag_ask en vuelo) antes de cambiar de
    // conversación; si no, el select se adelanta y el send se descarta solo.
    await vi.waitFor(() => expect(callsFor('rag_ask')).toHaveLength(1))
    await store.select('conv-b')

    pendingAsk.resolve(answer('conv-a', 'respuesta tardía'))
    await sendPromise

    const snapshot = snapshotOf(store)
    expect(snapshot.activeConversationId).toBe('conv-b')
    expect(snapshot.messages).toEqual([
      { role: 'user', content: 'pregunta de conv-b', sources: [] },
      { role: 'assistant', content: 'respuesta de conv-b', sources: [] },
    ])
    expect(snapshot.loading).toBe(false)
  })

  it('discards a stale brand-new conversation response after selecting another one', async () => {
    const pendingAsk = deferred<RagAnswer>()
    const state = setupBackend({
      conversations: { 'conv-b': conversation('conv-b', 'B') },
    })
    state.ask = () => pendingAsk.promise
    const store = new RagChatStore()
    await store.initialize()

    const sendPromise = store.send('pregunta nueva')
    // Igual que arriba: esperamos a que rag_ask esté en vuelo antes del select.
    await vi.waitFor(() => expect(callsFor('rag_ask')).toHaveLength(1))
    await store.select('conv-b')

    pendingAsk.resolve(answer('conv-new'))
    await sendPromise

    const snapshot = snapshotOf(store)
    expect(snapshot.activeConversationId).toBe('conv-b')
    expect(
      callsFor('settings_set').some(([, args]) => {
        return (args as { value: string }).value === 'conv-new'
      })
    ).toBe(false)
    // La base SÍ cambió aunque la respuesta se descartó: el listado se
    // refresca igual para no mostrar conversaciones fantasma (init + discard).
    expect(callsFor('rag_list_conversations')).toHaveLength(2)
  })
})

describe('RagChatStore.select', () => {
  it('clears the active conversation gracefully when it no longer exists', async () => {
    const state = setupBackend({
      storedActiveId: 'conv-1',
      summaries: [summary('conv-1', 'Primera', 1000), summary('conv-gone', 'Borrada', 500)],
      conversations: { 'conv-1': conversation('conv-1', 'Primera') },
    })
    const store = new RagChatStore()
    await store.initialize()

    state.summaries = [summary('conv-1', 'Primera', 1000)]
    await store.select('conv-gone')

    const snapshot = snapshotOf(store)
    expect(snapshot.activeConversationId).toBeNull()
    expect(snapshot.messages).toEqual([])
    expect(snapshot.loading).toBe(false)
    expect(snapshot.conversations.map((c) => c.id)).toEqual(['conv-1'])
    expect(callsFor('settings_delete')).toEqual([
      ['settings_delete', { key: 'rag_active_conversation' }],
    ])
  })
})

describe('RagChatStore.remove', () => {
  it('behaves like startNew when the active conversation is removed', async () => {
    const state = setupBackend({
      storedActiveId: 'conv-1',
      summaries: [summary('conv-1', 'Primera', 1000)],
      conversations: { 'conv-1': conversation('conv-1', 'Primera') },
    })
    const store = new RagChatStore()
    await store.initialize()
    store.setDraft('borrador pendiente')

    state.summaries = []
    await store.remove('conv-1')

    expect(callsFor('rag_delete_conversation')).toEqual([
      ['rag_delete_conversation', { conversationId: 'conv-1' }],
    ])
    const snapshot = snapshotOf(store)
    expect(snapshot.activeConversationId).toBeNull()
    expect(snapshot.messages).toEqual([])
    expect(snapshot.draft).toBe('')
    expect(snapshot.conversations).toEqual([])
    expect(callsFor('settings_delete')).toEqual([
      ['settings_delete', { key: 'rag_active_conversation' }],
    ])
  })

  it('keeps the active conversation when another one is removed', async () => {
    const state = setupBackend({
      storedActiveId: 'conv-1',
      summaries: [summary('conv-1', 'Primera', 2000), summary('conv-2', 'Otra', 1000)],
      conversations: { 'conv-1': conversation('conv-1', 'Primera') },
    })
    const store = new RagChatStore()
    await store.initialize()

    state.summaries = [summary('conv-1', 'Primera', 2000)]
    await store.remove('conv-2')

    const snapshot = snapshotOf(store)
    expect(snapshot.activeConversationId).toBe('conv-1')
    expect(snapshot.messages).toHaveLength(2)
    expect(snapshot.conversations.map((c) => c.id)).toEqual(['conv-1'])
  })
})

describe('RagChatStore.rename', () => {
  it('updates only the matching title after persistence succeeds', async () => {
    const state = setupBackend({
      storedActiveId: 'conv-1',
      summaries: [summary('conv-1', 'Título original', 1_000)],
      conversations: { 'conv-1': conversation('conv-1', 'Título original') },
    })
    const store = new RagChatStore()
    await store.initialize()
    store.setDraft('borrador')

    await store.rename('conv-1', '  Título renovado  ')

    expect(callsFor('rag_update_conversation_title')).toEqual([
      ['rag_update_conversation_title', { conversationId: 'conv-1', title: 'Título renovado' }],
    ])
    const snapshot = snapshotOf(store)
    expect(snapshot.conversations).toEqual([
      expect.objectContaining({
        id: 'conv-1',
        title: 'Título renovado',
        createdAt: 0,
        updatedAt: 1_000,
        messageCount: 2,
      }),
    ])
    expect(snapshot.activeConversationId).toBe('conv-1')
    expect(snapshot.messages).toEqual([
      { role: 'user', content: 'pregunta de conv-1', sources: [] },
      { role: 'assistant', content: 'respuesta de conv-1', sources: [] },
    ])
    expect(snapshot.draft).toBe('borrador')
  })

  it('records and rethrows persistence failures without changing the title', async () => {
    setupBackend({
      summaries: [summary('conv-1', 'Título original', 1_000)],
      rename: () => Promise.reject('No se pudo guardar el nombre.'),
    })
    const store = new RagChatStore()
    await store.initialize()

    await expect(store.rename('conv-1', 'Nuevo')).rejects.toBe('No se pudo guardar el nombre.')

    const snapshot = snapshotOf(store)
    expect(snapshot.conversations[0]?.title).toBe('Título original')
    expect(snapshot.error).toBe('No se pudo guardar el nombre.')
  })
  it('clears a previous rename error after a later rename succeeds', async () => {
    let renameAttempts = 0
    const state = setupBackend({
      summaries: [summary('conv-1', 'Título original', 1_000)],
      rename: () => {
        renameAttempts += 1
        return renameAttempts === 1
          ? Promise.reject('No se pudo guardar el nombre.')
          : undefined
      },
    })
    const store = new RagChatStore()
    await store.initialize()

    await expect(store.rename('conv-1', 'Primer intento')).rejects.toBe(
      'No se pudo guardar el nombre.',
    )
    await store.rename('conv-1', 'Segundo intento')

    expect(snapshotOf(store).error).toBeNull()
  })

  it('preserves an unrelated error when a later rename succeeds', async () => {
    const sharedError = 'No se pudo guardar el nombre.'
    let renameAttempts = 0
    const state = setupBackend({
      storedActiveId: 'conv-1',
      summaries: [summary('conv-1', 'Título original', 1_000)],
      conversations: { 'conv-1': conversation('conv-1', 'Título original') },
      ask: () => Promise.reject(sharedError),
      rename: () => {
        renameAttempts += 1
        return renameAttempts === 1 ? Promise.reject(sharedError) : undefined
      },
    })
    const store = new RagChatStore()
    await store.initialize()

    await expect(store.rename('conv-1', 'Primer intento')).rejects.toBe(sharedError)
    await store.send('pregunta')
    expect(snapshotOf(store).error).toBe(sharedError)

    await store.rename('conv-1', 'Título renovado')

    expect(snapshotOf(store).error).toBe(sharedError)
  })

  it('does not let a stale refresh overwrite a successful rename', async () => {
    const staleRefresh = deferred<RagConversationSummary[]>()
    const state = setupBackend({
      summaries: [summary('conv-1', 'Título original', 1_000)],
      conversations: { 'conv-1': conversation('conv-1', 'Título original') },
    })
    let listCalls = 0
    state.list = () => {
      listCalls += 1
      return listCalls === 1 ? state.summaries : staleRefresh.promise
    }
    const store = new RagChatStore()
    await store.initialize()

    const refreshPromise = store.remove('conv-2')
    await vi.waitFor(() => expect(callsFor('rag_list_conversations')).toHaveLength(2))

    await store.rename('conv-1', 'Título renovado')
    staleRefresh.resolve([summary('conv-1', 'Título original', 1_000)])
    await refreshPromise

    expect(snapshotOf(store).conversations[0]?.title).toBe('Título renovado')
  })

  it('does not apply a refresh that was invalidated by reset', async () => {
    const staleRefresh = deferred<RagConversationSummary[]>()
    const state = setupBackend({
      summaries: [summary('conv-1', 'Título original', 1_000)],
      conversations: { 'conv-1': conversation('conv-1', 'Título original') },
    })
    let listCalls = 0
    state.list = () => {
      listCalls += 1
      return listCalls === 1 ? state.summaries : staleRefresh.promise
    }
    const store = new RagChatStore()
    await store.initialize()

    const refreshPromise = store.remove('conv-2')
    await vi.waitFor(() => expect(callsFor('rag_list_conversations')).toHaveLength(2))

    store.reset()
    staleRefresh.resolve([summary('conv-1', 'Título original', 1_000)])
    await refreshPromise

    expect(snapshotOf(store).conversations).toEqual([])
  })

})

describe('RagChatStore persistence across unmounts', () => {
  it('keeps messages and state across subscribe/unsubscribe cycles', async () => {
    const state = setupBackend()
    state.ask = () => answer('conv-new')
    const store = new RagChatStore()
    await store.initialize()
    await store.send('¿Cuándo comenzó la huelga?')

    const first = snapshotOf(store)
    const second = snapshotOf(store)

    expect(first.messages).toHaveLength(2)
    expect(second.messages).toEqual(first.messages)
    expect(second.activeConversationId).toBe('conv-new')
  })

  it('keeps the draft across subscribe/unsubscribe cycles', async () => {
    setupBackend()
    const store = new RagChatStore()
    await store.initialize()

    store.setDraft('texto a medio escribir')

    expect(snapshotOf(store).draft).toBe('texto a medio escribir')
    expect(snapshotOf(store).draft).toBe('texto a medio escribir')
  })
})
