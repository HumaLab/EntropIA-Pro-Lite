import { invoke } from '@tauri-apps/api/core'
import { buildContext, evidenceOf, sentRecord, type ContextPiece } from './agent-context'

/**
 * The agent panel's state (plan-editor.md §14).
 *
 * # Nothing here writes into a manuscript
 *
 * §14.2 opens with the rule: the agent never silently changes the text. This
 * store records proposals and asks the backend whether an acceptance may
 * proceed; the actual edit is made by the caller, and only when it is told to.
 *
 * # Why the backend decides, and not this
 *
 * The decision to apply is a state transition on a row, checked by its affected
 * row count. Deciding here — "it looked pending a moment ago" — would let two
 * clicks both believe they were first, and a paragraph would be inserted twice.
 * So the answer comes back from the write that made it.
 */

export type Requirement = 'chat' | 'retrieval'

export interface AgentAction {
  id: string
  requires: Requirement
  available: boolean
  /** `no_chat_model` or `retrieval_unavailable`, empty when available. */
  unavailable_reason: string
}

export interface SuggestionRow {
  id: string
  document_id: string
  selection_anchor_json: string | null
  source_revision: number
  selected_content_hash: string
  action_type: string
  original_text: string | null
  suggested_text: string | null
  rationale: string | null
  evidence_json: string
  status: string
  provider: string | null
  model: string | null
  created_at: number
  resolved_at: number | null
}

export interface Resolution {
  /** True only for the acceptance that actually changed the status. */
  apply: boolean
  status: string
  suggested_text: string | null
}

export interface AgentSnapshot {
  actions: AgentAction[]
  pending: SuggestionRow[]
  /** What would be sent for the current selection, exactly as it would go. */
  context: ReturnType<typeof buildContext> | null
  error: string | null
  busy: boolean
}

const EMPTY: AgentSnapshot = {
  actions: [],
  pending: [],
  context: null,
  error: null,
  busy: false,
}

type Subscriber = (value: AgentSnapshot) => void

function message(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'message' in error) {
    return String((error as { message: unknown }).message)
  }
  return error instanceof Error ? error.message : String(error)
}

function codeOf(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'code' in error) {
    return String((error as { code: unknown }).code)
  }
  return 'unknown'
}

export class WritingAgentStore {
  #state: AgentSnapshot = { ...EMPTY }
  #subscribers = new Set<Subscriber>()

  subscribe(run: Subscriber): () => void {
    this.#subscribers.add(run)
    run(this.#state)
    return () => this.#subscribers.delete(run)
  }

  #set(patch: Partial<AgentSnapshot>) {
    this.#state = { ...this.#state, ...patch }
    for (const run of this.#subscribers) run(this.#state)
  }

  get snapshot(): AgentSnapshot {
    return this.#state
  }

  /**
   * Loads the capability matrix (§14.1, G9).
   *
   * Both answers are the caller's to supply. Whether a chat model is configured
   * is a question for the settings, and whether retrieval works is a question
   * about the agent crate — deciding either here would be inventing a fact.
   */
  async loadActions(hasChat: boolean, hasRetrieval: boolean): Promise<void> {
    try {
      const actions = await invoke<AgentAction[]>('writing_agent_actions', {
        hasChat,
        hasRetrieval,
      })
      this.#set({ actions, error: null })
    } catch (error) {
      // An unreadable matrix must not be read as "everything works". Nothing is
      // offered rather than something that will fail when it is used.
      this.#set({ actions: [], error: message(error) })
    }
  }

  async loadPending(documentId: string): Promise<void> {
    try {
      const pending = await invoke<SuggestionRow[]>('writing_agent_pending', { documentId })
      this.#set({ pending, error: null })
    } catch (error) {
      this.#set({ error: message(error) })
    }
  }

  /** Assembles the context for a selection and holds it, so it can be shown. */
  prepare(pieces: ContextPiece[]): void {
    this.#set({ context: buildContext(pieces) })
  }

  /** What the held context rests on, for recording with a proposal (§14.2). */
  evidence() {
    const context = this.#state.context
    return context ? evidenceOf(context) : null
  }

  /** The record of what was sent (§14.3), from the same object that was sent. */
  sent() {
    const context = this.#state.context
    return context ? sentRecord(context) : []
  }

  /**
   * Resolves a suggestion and reports whether its text should now be applied.
   *
   * `currentContentHash` is the target as it stands now. A suggestion whose
   * target moved is refused rather than applied to whatever is there — except
   * for discarding it, which is exactly what someone does about that.
   */
  async resolve(
    id: string,
    status: 'accepted' | 'inserted_below' | 'discarded',
    currentContentHash: string | null
  ): Promise<Resolution | { code: string; message: string }> {
    this.#set({ busy: true })
    try {
      const resolution = await invoke<Resolution>('writing_agent_resolve', {
        id,
        status,
        currentContentHash,
      })
      this.#set({
        busy: false,
        error: null,
        pending: this.#state.pending.filter((row) => row.id !== id),
      })
      return resolution
    } catch (error) {
      this.#set({ busy: false })
      // The code is what the panel branches on: a target that moved needs
      // different words from a suggestion that was already resolved.
      return { code: codeOf(error), message: message(error) }
    }
  }
}

export function isResolution(value: unknown): value is Resolution {
  return typeof value === 'object' && value !== null && 'apply' in value
}

export const writingAgent = new WritingAgentStore()
