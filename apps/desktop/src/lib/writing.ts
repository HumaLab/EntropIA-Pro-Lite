/**
 * The writing workspace's state (plan-editor.md §6, §16).
 *
 * A plain TypeScript class implementing the store contract by hand, like every
 * other global store here (`RagChatStore`, `BatchStore`, `NavigationStore`) —
 * deliberately not a `.svelte.ts`, so the whole thing is unit-testable without
 * mounting a component.
 *
 * Two rules from the spikes are enforced here rather than left to callers:
 *
 * 1. **Nothing is mounted unvalidated.** S1 measured that one unknown node or
 *    mark empties the whole Tiptap document with no throw, and that a naive
 *    autosave then writes that emptiness over the real manuscript. So a load
 *    that fails the contract yields an error state and *no document*, and the
 *    autosave loop refuses to run without one.
 *
 * 2. **Journalling is not saving.** `Guardado` appears only against a revision
 *    persistence acknowledged (§16.2); a journal write leaves the status on
 *    `Cambios pendientes`.
 */

import { invoke } from '@tauri-apps/api/core'
import {
  WRITING_SCHEMA_VERSION,
  emptyDocument,
  parseCanonical,
  type CanonicalDocument,
  type ValidationFailure,
} from '@entropia/ui'
import {
  CLEAN_STATE,
  DEFAULT_SCHEDULER,
  decide,
  onEdit,
  onJournalled,
  onSaved,
  type SchedulerConfig,
  type SchedulerState,
} from './writing-scheduler'

export interface WritingDocumentRow {
  id: string
  title: string
  document_type: string
  status: string
  schema_version: number
  current_content_json: string
  revision: number
  created_at: number
  updated_at: number
}

/** The shape `WritingError` serialises as. Branch on `code`, show `message`. */
export interface WritingCommandError {
  code: string
  message: string
}

export type SaveStatus =
  | 'saved'
  | 'saving'
  | 'pending'
  | 'error'
  | 'recovery-available'

export interface WritingSnapshot {
  ready: boolean
  loading: boolean
  documents: WritingDocumentRow[]
  /** The open document's metadata, or null when the list is showing. */
  open: WritingDocumentRow | null
  /** Its content, or null when it could not be mounted. */
  content: CanonicalDocument | null
  /** Why the open document could not be mounted, if that is what happened. */
  refusal: ValidationFailure | null
  revision: number
  status: SaveStatus
  error: WritingCommandError | null
}

const EMPTY: WritingSnapshot = {
  ready: false,
  loading: false,
  documents: [],
  open: null,
  content: null,
  refusal: null,
  revision: 0,
  status: 'saved',
  error: null,
}

function asCommandError(error: unknown): WritingCommandError {
  if (typeof error === 'object' && error !== null && 'code' in error && 'message' in error) {
    const shaped = error as WritingCommandError
    return { code: String(shaped.code), message: String(shaped.message) }
  }
  return { code: 'unknown', message: error instanceof Error ? error.message : String(error) }
}

type Subscriber = (value: WritingSnapshot) => void

export class WritingStore {
  #state: WritingSnapshot = { ...EMPTY }
  #subscribers = new Set<Subscriber>()
  #schedule: SchedulerState = { ...CLEAN_STATE }
  #timer: ReturnType<typeof setTimeout> | null = null
  #config: SchedulerConfig
  #now: () => number

  constructor(config: SchedulerConfig = DEFAULT_SCHEDULER, now: () => number = Date.now) {
    this.#config = config
    this.#now = now
  }

  subscribe(run: Subscriber): () => void {
    this.#subscribers.add(run)
    run(this.#state)
    return () => this.#subscribers.delete(run)
  }

  #set(patch: Partial<WritingSnapshot>) {
    this.#state = { ...this.#state, ...patch }
    for (const run of this.#subscribers) run(this.#state)
  }

  get snapshot(): WritingSnapshot {
    return this.#state
  }

  /** Gates the section on the migration, like the batch queue does. */
  async init(): Promise<boolean> {
    try {
      const ready = await invoke<boolean>('writing_is_ready')
      this.#set({ ready, error: null })
      return ready
    } catch (error) {
      this.#set({ ready: false, error: asCommandError(error) })
      return false
    }
  }

  async listDocuments(statuses: string[] = ['active']): Promise<void> {
    this.#set({ loading: true })
    try {
      const documents = await invoke<WritingDocumentRow[]>('writing_list_documents', { statuses })
      this.#set({ documents, loading: false, error: null })
    } catch (error) {
      this.#set({ loading: false, error: asCommandError(error) })
    }
  }

  async createDocument(title: string, documentType = 'article'): Promise<string | null> {
    const id = crypto.randomUUID()
    try {
      const row = await invoke<WritingDocumentRow>('writing_create_document', {
        input: {
          id,
          title,
          document_type: documentType,
          schema_version: WRITING_SCHEMA_VERSION,
          content_json: JSON.stringify(emptyDocument()),
        },
      })
      this.#set({ documents: [row, ...this.#state.documents], error: null })
      return row.id
    } catch (error) {
      this.#set({ error: asCommandError(error) })
      return null
    }
  }

  /**
   * Opens a document. A document whose content fails the contract yields
   * `refusal` and a null `content`, and the autosave loop will not start — so
   * there is no path from "cannot read it" to "overwrote it".
   */
  async openDocument(id: string): Promise<void> {
    this.#cancelTimer()
    this.#schedule = { ...CLEAN_STATE }
    this.#set({ loading: true, open: null, content: null, refusal: null, error: null })
    try {
      const row = await invoke<WritingDocumentRow>('writing_load_document', { id })
      const parsed = parseCanonical(row.current_content_json)
      if (!parsed.ok) {
        this.#set({
          loading: false,
          open: row,
          content: null,
          refusal: parsed,
          revision: row.revision,
          status: 'error',
        })
        return
      }
      this.#set({
        loading: false,
        open: row,
        content: parsed.document,
        refusal: null,
        revision: row.revision,
        status: 'saved',
      })
    } catch (error) {
      this.#set({ loading: false, error: asCommandError(error), status: 'error' })
    }
  }

  /**
   * Renames a document. The backend does not advance the revision for this —
   * a title is metadata, and bumping it would turn an edit in flight into a
   * spurious conflict — so neither does the local state.
   */
  async renameDocument(id: string, title: string): Promise<void> {
    const trimmed = title.trim()
    if (!trimmed) return
    try {
      await invoke('writing_rename_document', { id, title: trimmed })
      const open = this.#state.open
      this.#set({
        open: open && open.id === id ? { ...open, title: trimmed } : open,
        documents: this.#state.documents.map((d) => (d.id === id ? { ...d, title: trimmed } : d)),
        error: null,
      })
    } catch (error) {
      this.#set({ error: asCommandError(error) })
    }
  }

  /**
   * Closes the open document. The store outlives the view — it is a module
   * singleton — so leaving `open` set is what makes a remount show the editor
   * again instead of the list.
   */
  closeDocument(): void {
    this.#cancelTimer()
    this.#schedule = { ...CLEAN_STATE }
    this.#set({ open: null, content: null, refusal: null, revision: 0, status: 'saved' })
  }

  /**
   * Records an edit and arms whatever the scheduler says is due next.
   *
   * Content identical to what is already held is not an edit. Without this a
   * remount, a recovery replay or a restore would each look like typing and
   * earn a spurious revision.
   */
  applyEdit(next: CanonicalDocument): void {
    if (!this.#state.open || this.#state.refusal) return
    if (this.#state.content && JSON.stringify(this.#state.content) === JSON.stringify(next)) return
    this.#set({ content: next, status: 'pending' })
    this.#schedule = onEdit(this.#schedule, this.#now())
    this.#arm()
  }

  #cancelTimer() {
    if (this.#timer !== null) {
      clearTimeout(this.#timer)
      this.#timer = null
    }
  }

  /**
   * Honours `nextCheckInMs`. Waking only on edits looks like it works and
   * quietly breaks the ceiling — see the note in `writing-scheduler.ts`.
   */
  #arm() {
    this.#cancelTimer()
    const decision = decide(this.#schedule, this.#now(), this.#config)
    if (decision.nextCheckInMs === null) return
    this.#timer = setTimeout(() => {
      this.#timer = null
      void this.tick()
    }, Math.max(decision.nextCheckInMs, 1))
  }

  /** Runs whatever is due. Exposed so tests can drive it without timers. */
  async tick(): Promise<void> {
    const open = this.#state.open
    const content = this.#state.content
    if (!open || !content) return

    const decision = decide(this.#schedule, this.#now(), this.#config)
    if (decision.journal) {
      await this.#journal(open.id, content)
    }
    if (decision.save) {
      await this.#save(open.id, content)
    }
    this.#arm()
  }

  async #journal(documentId: string, content: CanonicalDocument): Promise<void> {
    try {
      await invoke<number>('writing_append_journal', {
        entry: {
          document_id: documentId,
          base_revision: this.#state.revision,
          schema_version: content.schemaVersion,
          delta_json: JSON.stringify(content.doc),
        },
      })
      this.#schedule = onJournalled(this.#schedule, this.#now())
      // Still pending: a journal write is not a canonical save (§16.2).
    } catch (error) {
      this.#set({ error: asCommandError(error) })
    }
  }

  async #save(documentId: string, content: CanonicalDocument): Promise<void> {
    this.#set({ status: 'saving' })
    try {
      const revision = await invoke<number>('writing_save_document', {
        save: {
          document_id: documentId,
          expected_revision: this.#state.revision,
          content_json: JSON.stringify(content),
          schema_version: content.schemaVersion,
          plain_text_cache: null,
          citations: [],
          provenance: [],
        },
      })
      this.#schedule = onSaved()
      this.#set({ revision, status: 'saved', error: null })
      void invoke('writing_prune_journal', {
        documentId,
        confirmedRevision: revision,
      }).catch(() => {
        // Pruning is housekeeping: failing it costs disk, never correctness.
      })
    } catch (error) {
      // A revision_conflict means another window won. The content stays in
      // memory and the status stays visible — §16.2 forbids a save error that
      // disappears on its own.
      this.#set({ status: 'error', error: asCommandError(error) })
    }
  }

  /** Forces a canonical save now: switching documents, closing the view. */
  async flush(): Promise<void> {
    this.#cancelTimer()
    const open = this.#state.open
    const content = this.#state.content
    if (!open || !content || this.#state.status === 'saved') return
    await this.#save(open.id, content)
  }

  /** Releases timers. Called when the view unmounts. */
  dispose(): void {
    this.#cancelTimer()
  }
}

export const writing = new WritingStore()
