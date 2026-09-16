import { getStore } from '$lib/db'
import type { NoteSearchHit } from '@entropia/store'

/**
 * The Notas tab's state (plan-editor.md §6.3, §13).
 *
 * Two things this holds that the note repository cannot. The first is scope:
 * the manuscript's own collections narrow the search, because a writer looking
 * for evidence for *this* article is not looking through the whole archive —
 * §13.1 says the associations "ayudarán a filtrar el selector, pero no
 * reemplazarán la identidad del item".
 *
 * The second is the live state of the links already in the manuscript. A note
 * that has moved on since it was linked has to be reported (§13), and only
 * something that can read both the note and the snapshot can notice.
 *
 * A plain class implementing the store contract by hand, like the other stores
 * here — testable without mounting anything.
 */

export interface NotesSnapshot {
  query: string
  searching: boolean
  results: NoteSearchHit[]
  /** The note being read in full, if any. */
  open: NoteSearchHit | null
  /** Collections the manuscript is associated with; empty means the whole corpus. */
  scope: string[]
  error: string | null
}

const EMPTY: NotesSnapshot = {
  query: '',
  searching: false,
  results: [],
  open: null,
  scope: [],
  error: null,
}

type Subscriber = (value: NotesSnapshot) => void

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export class WritingNotesStore {
  #state: NotesSnapshot = { ...EMPTY }
  #subscribers = new Set<Subscriber>()
  #store: () => ReturnType<typeof getStore>
  #searchToken = 0

  constructor(store: () => ReturnType<typeof getStore> = getStore) {
    this.#store = store
  }

  subscribe(run: Subscriber): () => void {
    this.#subscribers.add(run)
    run(this.#state)
    return () => this.#subscribers.delete(run)
  }

  #set(patch: Partial<NotesSnapshot>) {
    this.#state = { ...this.#state, ...patch }
    for (const run of this.#subscribers) run(this.#state)
  }

  get snapshot(): NotesSnapshot {
    return this.#state
  }

  /**
   * Narrows future searches to the manuscript's collections.
   *
   * An empty list means the whole corpus rather than nothing, which is the
   * opposite of what the repository does with an empty array — and deliberately
   * so. There, an empty scope was asked for; here, a manuscript with no
   * associated collection has simply not narrowed anything.
   */
  setScope(collectionIds: string[]): void {
    this.#set({ scope: collectionIds })
  }

  async search(rawQuery: string): Promise<void> {
    const query = rawQuery.trim()
    this.#searchToken += 1
    const token = this.#searchToken

    this.#set({ query: rawQuery, searching: true, error: null })
    try {
      const results = await this.#store().notes.search({
        query: query || undefined,
        collectionIds: this.#state.scope.length > 0 ? this.#state.scope : undefined,
      })
      if (token !== this.#searchToken) return
      this.#set({ searching: false, results })
    } catch (error) {
      if (token !== this.#searchToken) return
      this.#set({ searching: false, error: message(error) })
    }
  }

  openNote(noteId: string): void {
    const found = this.#state.results.find((note) => note.id === noteId) ?? null
    this.#set({ open: found })
  }

  closeNote(): void {
    this.#set({ open: null })
  }

  /** The note as it stands now, for checking a link against its snapshot. */
  async readNote(noteId: string): Promise<{ exists: boolean; content: string | null }> {
    try {
      const found = await this.#store().notes.findById(noteId)
      return found ? { exists: true, content: found.content } : { exists: false, content: null }
    } catch {
      // Not knowing is not the same as the note being gone. Reporting a
      // deletion because a read failed would be a claim this cannot make, and
      // `resolveNoteLink` turns a null content into `unverifiable`.
      return { exists: true, content: null }
    }
  }
}

export const writingNotes = new WritingNotesStore()
