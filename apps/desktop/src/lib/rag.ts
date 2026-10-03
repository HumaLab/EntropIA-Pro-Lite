import { invoke } from '@tauri-apps/api/core'

export interface RagSourceProvenance {
  retrievalUnit: string
  sourceKind: string
  sourceId: string
  chunkIds: string[]
  startChar: number
  endChar: number
}

export interface RagSource {
  /** 1-based index matching [n] citations in the answer text. */
  index: number
  assetId: string
  itemId: string
  itemTitle: string
  collectionId: string
  collectionName: string
  snippet: string
  score: number
  startSeconds: number | null
  endSeconds: number | null
  provenance: RagSourceProvenance | null
  /**
   * Present only for a passage of a Zotero library (Biblioteca scope). Absent
   * or null means a corpus source, which is also what every conversation
   * persisted before Biblioteca existed reads as.
   */
  bibliography?: RagBibliographySource | null
}

/** `pages` for a PDF, `paragraphs` for an HTML snapshot (it has no real pages). */
export interface RagBibliographyLocation {
  kind: 'pages' | 'paragraphs'
  from: number
  to: number
}

export interface RagBibliographySource {
  /** Reopens the passage on this device; the catalog is local-only. */
  chunkId: string
  itemKey: string
  libraryName: string
  libraryType: string
  libraryNativeId: string
  /** CSL family names, comma-separated; may be empty. */
  authors: string
  year: number | null
  location: RagBibliographyLocation | null
}

/** Where a question looks. Corpus is what the chat always did. */
export type RagScope = 'corpus' | 'biblioteca' | 'both'

/** One Zotero library as the backend names it (`user`/`group` + native id). */
export interface RagLibraryRef {
  libraryType: string
  libraryId: string
}

export interface RagAnswer {
  answer: string
  sources: RagSource[]
  model: string
  /**
   * Id real de la conversación persistida. `null` cuando la persistencia
   * falló después de una respuesta exitosa del LLM: la respuesta vale,
   * pero no hay id que adoptar.
   */
  conversationId: string | null
  /**
   * Why the Biblioteca leg contributed nothing (`no_library_synced`,
   * `no_embeddings`, `embedding_unavailable`, `failed`). Absent when it ran
   * normally or the scope did not ask for it.
   */
  bibliographyNotice?: string | null
}

export interface RagConversationSummary {
  id: string
  title: string
  createdAt: number
  updatedAt: number
  messageCount: number
}

export interface RagMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
  sources: RagSource[]
  createdAt: number
}

export interface RagConversation {
  id: string
  title: string
  messages: RagMessage[]
}

export interface RagAskOptions {
  scope?: RagScope
  /** Libraries to search; absent or empty means every synced one. */
  libraries?: RagLibraryRef[] | null
}

export function ragAsk(
  question: string,
  conversationId?: string,
  topK?: number,
  options: RagAskOptions = {}
): Promise<RagAnswer> {
  return invoke<RagAnswer>('rag_ask', {
    question,
    conversationId,
    topK,
    scope: options.scope,
    libraries: options.libraries ?? undefined,
  })
}

/** List persisted conversations ordered by updatedAt DESC. */
export function ragListConversations(): Promise<RagConversationSummary[]> {
  return invoke<RagConversationSummary[]>('rag_list_conversations')
}

/** Search persisted conversation titles and user/assistant message text. */
export function ragSearchConversations(query: string): Promise<RagConversationSummary[]> {
  return invoke<RagConversationSummary[]>('rag_search_conversations', {
    query: query.trim(),
  })
}

/** Fetch one conversation with its messages in order. */
export function ragGetConversation(conversationId: string): Promise<RagConversation> {
  return invoke<RagConversation>('rag_get_conversation', { conversationId })
}

export function ragDeleteConversation(conversationId: string): Promise<void> {
  return invoke<void>('rag_delete_conversation', { conversationId })
}

/**
 * Asks the backend to auto-title a freshly created conversation with the same
 * OpenRouter configuration the chat itself uses. Resolves to the persisted
 * title, or `null` when nothing was written (no API key, timeout, provider
 * error, invalid/generic title, or a title the user already owns). A `null`
 * result is the expected fallback path, not a failure.
 */
export function ragGenerateConversationTitle(conversationId: string): Promise<string | null> {
  return invoke<string | null>('rag_generate_conversation_title', { conversationId })
}

export function ragRenameConversation(conversationId: string, title: string): Promise<void> {
  return invoke<void>('rag_update_conversation_title', {
    conversationId,
    title: title.trim(),
  })
}
