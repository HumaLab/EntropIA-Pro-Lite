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

export function ragAsk(
  question: string,
  conversationId?: string,
  topK?: number
): Promise<RagAnswer> {
  return invoke<RagAnswer>('rag_ask', { question, conversationId, topK })
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

export function ragRenameConversation(conversationId: string, title: string): Promise<void> {
  return invoke<void>('rag_update_conversation_title', {
    conversationId,
    title: title.trim(),
  })
}
