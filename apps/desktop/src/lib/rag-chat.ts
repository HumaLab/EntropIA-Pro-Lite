/**
 * RAG chat store for the desktop app.
 * Module-level state so the conversation survives navigation and unmounts:
 * in-flight answers land in the store, not in a destroyed component.
 * Mirrors the NavigationStore subscription idiom.
 */

import { t } from './i18n'
import {
  ragAsk,
  ragDeleteConversation,
  ragRenameConversation,
  ragGetConversation,
  ragListConversations,
  type RagConversationSummary,
  type RagMessage,
  type RagSource,
} from './rag'
import { settingsDelete, settingsGet, settingsSet, SETTINGS_KEYS } from './settings'

export interface UiMessage {
  role: 'user' | 'assistant'
  content: string
  sources?: RagSource[]
}

export interface RagChatSnapshot {
  conversations: RagConversationSummary[]
  activeConversationId: string | null
  messages: UiMessage[]
  loading: boolean
  error: string | null
  draft: string
  initialized: boolean
}

type RagChatSubscriber = (snapshot: RagChatSnapshot) => void
type RagChatErrorSource = 'initialize' | 'send' | 'remove' | 'rename'

function describeError(error: unknown): string {
  if (typeof error === 'string' && error.trim()) return error
  if (error instanceof Error && error.message) return error.message
  return t('ragChat.errorGeneric')
}

function toUiMessage(message: RagMessage): UiMessage {
  return {
    role: message.role,
    content: message.content,
    sources: message.sources ?? [],
  }
}

export class RagChatStore {
  private _conversations: RagConversationSummary[] = []
  private _activeConversationId: string | null = null
  private _messages: UiMessage[] = []
  private _loading = false
  private _error: string | null = null
  private _errorSource: RagChatErrorSource | null = null
  private _draft = ''
  private _initialized = false
  private _initPromise: Promise<void> | null = null
  private _conversationGeneration = 0
  private readonly _conversationTitleOverrides = new Map<string, string>()
  private _requestId = 0
  private readonly _subscribers = new Set<RagChatSubscriber>()

  subscribe(run: RagChatSubscriber): () => void {
    this._subscribers.add(run)
    run(this.snapshot())
    return () => {
      this._subscribers.delete(run)
    }
  }

  private snapshot(): RagChatSnapshot {
    return {
      conversations: [...this._conversations],
      activeConversationId: this._activeConversationId,
      messages: [...this._messages],
      loading: this._loading,
      error: this._error,
      draft: this._draft,
      initialized: this._initialized,
    }
  }

  private emit(): void {
    const snapshot = this.snapshot()
    this._subscribers.forEach((run) => run(snapshot))
  }

  /**
   * Idempotent bootstrap: loads the conversation list, restores the active
   * conversation id from settings (falling back to the most recent one) and
   * rehydrates its messages. Subsequent calls are no-ops.
   */
  initialize(): Promise<void> {
    this._initPromise ??= this.doInitialize()
    return this._initPromise
  }

  private async doInitialize(): Promise<void> {
    const conversationGeneration = this._conversationGeneration
    let conversations: RagConversationSummary[]
    try {
      conversations = await ragListConversations()
    } catch (error) {
      if (conversationGeneration !== this._conversationGeneration) return
      // Bootstrap fallido: no memoizamos la promesa para que el próximo
      // mount reintente en vez de quedar roto para siempre.
      this._conversations = []
      this._error = describeError(error)
      this._errorSource = 'initialize'
      this._initPromise = null
      this.emit()
      return
    }

    if (conversationGeneration !== this._conversationGeneration) return
    this._conversations = this.mergeConversationTitles(conversations)

    // Si el usuario ya interactuó (envío en vuelo o mensajes optimistas),
    // la rehidratación no debe pisar su estado: cerramos solo la carga del
    // listado y marcamos el bootstrap como hecho.
    if (this._loading || this._messages.length > 0) {
      this._initialized = true
      this.emit()
      return
    }

    const storedId = await settingsGet(SETTINGS_KEYS.RAG_ACTIVE_CONVERSATION).catch(() => null)
    if (conversationGeneration !== this._conversationGeneration) return

    const candidateId =
      storedId && this._conversations.some((conversation) => conversation.id === storedId)
        ? storedId
        : (this._conversations[0]?.id ?? null)

    if (candidateId) {
      try {
        const conversation = await ragGetConversation(candidateId)
        if (conversationGeneration !== this._conversationGeneration) return

        this._activeConversationId = candidateId
        this._messages = conversation.messages.map(toUiMessage)
        if (candidateId !== storedId) {
          await this.persistActiveConversation(candidateId)
        }
      } catch (error) {
        if (conversationGeneration !== this._conversationGeneration) return

        console.warn('[RagChatStore] Failed to rehydrate conversation:', error)
        this._activeConversationId = null
        this._messages = []
        await this.persistActiveConversation(null)
      }
    } else if (storedId) {
      // Lista vacía pero quedó un id guardado de una conversación que ya no
      // existe: limpiamos la clave para no arrastrarla para siempre.
      void this.persistActiveConversation(null)
    }

    if (conversationGeneration !== this._conversationGeneration) return

    this._initialized = true
    this.emit()
  }

  /**
   * Sends a question in the active conversation (or starts a new one when
   * none is active). The response is applied only if the conversation context
   * is still current when it arrives.
   */
  async send(rawQuestion: string): Promise<void> {
    const question = rawQuestion.trim()
    if (!question || this._loading) return

    // El bootstrap es barato (promesa memoizada) y cierra la carrera entre
    // la rehidratación y el primer envío: el id de conversación que
    // capturamos abajo ya es el definitivo.
    await this.initialize()
    if (this._loading) return

    const requestId = ++this._requestId
    const requestConversationId = this._activeConversationId

    this._messages = [...this._messages, { role: 'user', content: question }]
    this._draft = ''
    this._error = null
    this._errorSource = null
    this._loading = true
    this.emit()

    try {
      const response = await ragAsk(question, requestConversationId ?? undefined)
      if (!this.isRequestCurrent(requestId, requestConversationId)) {
        // La respuesta llegó tarde para esta vista, pero la base SÍ cambió:
        // refrescamos el listado para no mostrar conversaciones fantasma.
        void this.refreshConversations()
        return
      }

      // conversationId null = la persistencia falló después de una respuesta
      // exitosa del LLM: mostramos la respuesta igual y no adoptamos ningún id.
      if (response.conversationId) {
        this._activeConversationId = response.conversationId
      }
      this._messages = [
        ...this._messages,
        { role: 'assistant', content: response.answer, sources: response.sources },
      ]
      this.emit()
      if (response.conversationId) {
        await this.persistActiveConversation(response.conversationId)
      }
      await this.refreshConversations()
    } catch (error) {
      if (!this.isRequestCurrent(requestId, requestConversationId)) return
      this._error = describeError(error)
      this._errorSource = 'send'
      this.emit()
    } finally {
      // El reset de loading está desacoplado de aplicar la respuesta: si
      // ningún request más nuevo tomó posesión del loading (select/startNew
      // hacen su propio reset), lo soltamos acá aunque la respuesta se haya
      // descartado.
      if (this._requestId === requestId) {
        this._loading = false
        this.emit()
      }
    }
  }

  /** Loads a past conversation and makes it the active one. */
  async select(conversationId: string): Promise<void> {
    if (conversationId === this._activeConversationId) return

    const requestId = ++this._requestId
    this._loading = true
    this._error = null
    this._errorSource = null
    this.emit()

    try {
      const conversation = await ragGetConversation(conversationId)
      if (requestId !== this._requestId) return

      this._activeConversationId = conversationId
      this._messages = conversation.messages.map(toUiMessage)
      this._loading = false
      this.emit()
      await this.persistActiveConversation(conversationId)
    } catch (error) {
      if (requestId !== this._requestId) return

      console.warn('[RagChatStore] Failed to load conversation:', error)
      this._activeConversationId = null
      this._messages = []
      this._loading = false
      this.emit()
      await this.persistActiveConversation(null)
      await this.refreshConversations()
    }
  }

  /**
   * Clears the active conversation. No DB row is created here — the backend
   * creates one on the first send.
   */
  startNew(): void {
    this._requestId++
    this._activeConversationId = null
    this._messages = []
    this._error = null
    this._errorSource = null
    this._draft = ''
    this._loading = false
    this.emit()
    void this.persistActiveConversation(null)
  }

  /** Deletes a conversation; if it was active, behaves like startNew(). */
  async remove(conversationId: string): Promise<void> {
    try {
      await ragDeleteConversation(conversationId)
    } catch (error) {
      this._error = describeError(error)
      this._errorSource = 'remove'
      this.emit()
      return
    }

    this._conversationTitleOverrides.delete(conversationId)
    if (this._activeConversationId === conversationId) {
      this.startNew()
    }
    await this.refreshConversations()
  }

  /** Persists a title and updates only the matching local conversation summary. */
  async rename(conversationId: string, title: string): Promise<void> {
    const normalizedTitle = title.trim()
    try {
      await ragRenameConversation(conversationId, normalizedTitle)
    } catch (error) {
      this._error = describeError(error)
      this._errorSource = 'rename'
      this.emit()
      throw error
    }

    this._conversations = this._conversations.map((conversation) =>
      conversation.id === conversationId ? { ...conversation, title: normalizedTitle } : conversation,
    )
    this._conversationTitleOverrides.set(conversationId, normalizedTitle)
    if (this._errorSource === 'rename') {
      this._error = null
      this._errorSource = null
    }
    this.emit()
  }

  /** Keeps the composer draft at module scope so it survives navigation. */
  setDraft(value: string): void {
    this._draft = value
    this.emit()
  }

  /** Test-only: restore pristine state so suites can isolate the singleton. */
  reset(): void {
    this._conversationGeneration += 1
    this._requestId++
    this._conversationTitleOverrides.clear()
    this._conversations = []
    this._activeConversationId = null
    this._messages = []
    this._loading = false
    this._error = null
    this._errorSource = null
    this._draft = ''
    this._initialized = false
    this._initPromise = null
    this.emit()
  }

  private isRequestCurrent(requestId: number, requestConversationId: string | null): boolean {
    return requestId === this._requestId && this._activeConversationId === requestConversationId
  }

  private async persistActiveConversation(conversationId: string | null): Promise<void> {
    try {
      if (conversationId) {
        await settingsSet(SETTINGS_KEYS.RAG_ACTIVE_CONVERSATION, conversationId)
      } else {
        await settingsDelete(SETTINGS_KEYS.RAG_ACTIVE_CONVERSATION)
      }
    } catch (error) {
      console.warn('[RagChatStore] Failed to persist active conversation:', error)
    }
  }

  private async refreshConversations(): Promise<void> {
    const conversationGeneration = this._conversationGeneration
    try {
      const conversations = await ragListConversations()
      if (conversationGeneration !== this._conversationGeneration) return
      this._conversations = this.mergeConversationTitles(conversations)
      this.emit()
    } catch (error) {
      console.warn('[RagChatStore] Failed to refresh conversations:', error)
    }
  }

  private mergeConversationTitles(
    conversations: RagConversationSummary[],
  ): RagConversationSummary[] {
    const returnedIds = new Set(conversations.map((conversation) => conversation.id))
    const merged = conversations.map((conversation) => {
      const override = this._conversationTitleOverrides.get(conversation.id)
      if (override === undefined) return conversation
      if (override === conversation.title) {
        this._conversationTitleOverrides.delete(conversation.id)
        return conversation
      }
      return { ...conversation, title: override }
    })

    for (const conversationId of this._conversationTitleOverrides.keys()) {
      if (!returnedIds.has(conversationId)) {
        this._conversationTitleOverrides.delete(conversationId)
      }
    }
    return merged
  }
}

export const ragChat = new RagChatStore()
