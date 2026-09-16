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
  citationProjection,
  zoteroCitationProjection,
  emptyDocument,
  parseCanonical,
  type CanonicalDocument,
  type RepairReport,
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
/**
 * Where a piece of the manuscript came from (§9.6).
 *
 * These are unions, not strings, because `writing_provenance_events` constrains
 * both columns with a `CHECK` and SQLite only complains once the transaction
 * runs — at which point the save has already failed in front of the writer.
 * `provenance-vocabulary.test.ts` keeps them equal to the migration.
 */
/** One manuscript that cites an asset, and how many times (§10.3). */
export interface AssetDependency {
  document_id: string
  document_title: string
  citation_count: number
}

/**
 * Which manuscripts cite `assetId`.
 *
 * A free function rather than a store method: the asset views that need it have
 * nothing to do with an open manuscript, and answering them should not require
 * the writing store to exist. It answers with an empty list when Escritura has
 * never been opened, so a deletion is never blocked by a missing table.
 */
export async function citationsForAsset(assetId: string): Promise<AssetDependency[]> {
  try {
    const found = await invoke<AssetDependency[]>('writing_citations_for_asset', { assetId })
    // The shape is checked, not assumed. A command that answers with anything
    // else is not a dependency list, and handing that to a caller which will
    // read `.length` turns a missing warning into a broken dialog.
    return Array.isArray(found) ? found : []
  } catch {
    // A dependency warning that cannot be produced must not stop the deletion
    // the user asked for. The snapshot on the citation is what preserves it
    // either way (§29.1).
    return []
  }
}

export type ProvenanceOrigin = 'manual' | 'corpus' | 'note' | 'zotero' | 'agent' | 'import'
export type ProvenanceOperation = 'insert' | 'replace' | 'rewrite' | 'restore' | 'other'

/** One provenance event waiting to be committed with the next save (§9.6). */
export interface PendingProvenance {
  id: string
  origin_type: ProvenanceOrigin
  operation_type: ProvenanceOperation
  range_anchor_json: string | null
  source_reference_json: string | null
  model_provider: string | null
  model_name: string | null
}

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
  /**
   * Damage healed on load, if any. In memory only — the writer is told, and
   * the next real save is what persists it (§8.3).
   */
  repair: RepairReport | null
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
  repair: null,
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
  /** Provenance events waiting for the save that will commit them (§10.1). */
  #pendingProvenance: PendingProvenance[] = []
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
    this.#set({ loading: true, open: null, content: null, refusal: null, repair: null, error: null })
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
        repair: parsed.repair.orphanFootnoteReferences > 0 ? parsed.repair : null,
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
   * Takes a document out of the workspace.
   *
   * Reversible by design. The schema's `status` column already carries
   * `trashed` and the list only ever asks for `active`, so the manuscript, its
   * versions and its journal all survive: what a click in the list removes is
   * the document's place in the workspace, not the writing.
   *
   * The open document is closed before the status changes. The autosave loop
   * holds whatever is open, and a write landing after the discard would put
   * the document back in front of the writer.
   */
  /**
   * Queues a provenance event to commit with the next save (§9.6, §10.1).
   *
   * It is not written on its own. §10.1 requires the node, the projection and
   * the event to land together or not at all, and `save_document` is the one
   * transaction that can do that — so the event waits here and travels with the
   * content it describes.
   *
   * It is cleared only once that save succeeds. A failed save leaves the draft
   * and the error standing with the event still queued, which is exactly what
   * §10.1 asks for: no partially confirmed records.
   */
  queueProvenance(event: PendingProvenance): void {
    this.#pendingProvenance = [...this.#pendingProvenance, event]
  }

  /** What is waiting to be committed. Empty once the save that carried it lands. */
  get pendingProvenance(): PendingProvenance[] {
    return this.#pendingProvenance
  }

  async trashDocument(id: string): Promise<void> {
    if (this.#state.open?.id === id) this.closeDocument()
    try {
      await invoke('writing_set_status', { id, status: 'trashed' })
      this.#set({
        documents: this.#state.documents.filter((d) => d.id !== id),
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
    this.#set({
      open: null,
      content: null,
      refusal: null,
      repair: null,
      revision: 0,
      status: 'saved',
    })
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
    // Captured before the await: an edit landing mid-save must not have its
    // event cleared by a save that never carried it.
    const sent = this.#pendingProvenance
    try {
      const revision = await invoke<number>('writing_save_document', {
        save: {
          document_id: documentId,
          expected_revision: this.#state.revision,
          content_json: JSON.stringify(content),
          schema_version: content.schemaVersion,
          plain_text_cache: null,
          // Derived from the manuscript on every save, never maintained beside
          // it. `save_document` replaces the rows inside this same transaction
          // (§8.4), so a derived projection cannot drift: copy, move, delete,
          // undo and redo all stay consistent because the document is the only
          // thing that says what the citations are.
          citations: citationProjection(content),
          zotero_citations: zoteroCitationProjection(content),
          // Append-only, and committed by the same transaction as the content
          // it describes (§9.6, §10.1).
          provenance: sent,
        },
      })
      this.#schedule = onSaved()
      // Only now: the events are committed, so they stop waiting. Anything
      // queued while this save was in flight stays queued.
      this.#pendingProvenance = this.#pendingProvenance.slice(sent.length)
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

  /**
   * Whether the visible failure is one another attempt could clear.
   *
   * A `revision_conflict` is not: another window advanced the revision, so the
   * same expected revision can only fail again, and forcing it through would
   * overwrite work this store never saw. Offering a button that cannot succeed
   * is worse than offering none, so retry is withheld and the message says what
   * actually happened.
   */
  get canRetrySave(): boolean {
    const { status, error, open, content } = this.#state
    if (status !== 'error' || !error || !open || !content) return false
    return error.code !== 'revision_conflict'
  }

  /**
   * Tries the failed save again (plan-editor.md 16.2).
   *
   * Without it the writer can only provoke another attempt by typing more,
   * which is not a recovery. The content never left memory, so this is the
   * same save, not a reconstruction of it.
   */
  async retrySave(): Promise<void> {
    if (!this.canRetrySave) return
    const open = this.#state.open
    const content = this.#state.content
    if (!open || !content) return
    this.#cancelTimer()
    await this.#save(open.id, content)
    this.#arm()
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
