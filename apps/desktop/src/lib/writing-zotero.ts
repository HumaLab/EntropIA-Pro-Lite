import { invoke } from '@tauri-apps/api/core'

/**
 * The Zotero tab's state (plan-editor.md §6.3, §11).
 *
 * # Why the state is held rather than inferred
 *
 * §11.3 forbids claiming Zotero is closed or not installed without evidence,
 * and the backend already refuses to say either — it reports five states, each
 * of them something observed. This store's job is to keep that honesty intact
 * on the way to the screen: it holds what the probe said, and the panel says
 * exactly that and no more.
 *
 * # Why a failure here is never an empty library
 *
 * A library that could not be reached and a library with no matches look the
 * same if all you keep is a list. They are very different things to tell
 * someone hunting for a reference, so the state travels beside the results.
 */

/** What the backend is willing to assert (§11.3). Never "closed", never "not installed". */
export type ZoteroState =
  | { state: 'available' }
  | { state: 'endpoint_unavailable' }
  | { state: 'api_disabled' }
  | { state: 'timeout' }
  | { state: 'invalid_response'; detail: string }

export interface LibraryPage {
  /** Each item as CSL-JSON text, ready for the renderer with no conversion. */
  items: string[]
  /** `Last-Modified-Version` — the only instance identity Zotero 9 offers. */
  version: number | null
  /** What the library says it holds for this query, when it says so. */
  total: number | null
  has_more: boolean
}

/** The copy of the library kept on disk, as the backend hands it over. */
interface MirrorView {
  items: string[]
  version: number | null
}

/** What a sync did. `items` is the whole library, and only when it changed. */
interface SyncOutcome {
  items: string[] | null
  version: number | null
  fetched: number
  removed: number
}

/** One work, read out of its CSL-JSON just enough to list it. */
export interface LibraryEntry {
  key: string
  title: string
  authors: string
  year: string
  /** The untouched CSL-JSON. What gets cited, and what gets snapshotted. */
  csl_json: string
}

export interface ZoteroSnapshot {
  /** Null until the first probe answers. */
  status: ZoteroState | null
  probing: boolean
  loading: boolean
  query: string
  entries: LibraryEntry[]
  /** How much of the library has been read. */
  loaded: number
  /** What the library says it holds for the current query. */
  total: number | null
  error: string | null
}

const EMPTY: ZoteroSnapshot = {
  status: null,
  probing: false,
  loading: false,
  query: '',
  entries: [],
  loaded: 0,
  total: null,
  error: null,
}

/** How many rows the list shows. Filtering happens over the whole library. */
const VISIBLE = 200

type Subscriber = (value: ZoteroSnapshot) => void

function message(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'message' in error) {
    return String((error as { message: unknown }).message)
  }
  return error instanceof Error ? error.message : String(error)
}

/** Reads the few fields a list needs, without disturbing the CSL-JSON itself. */
function describe(csl_json: string): LibraryEntry | null {
  try {
    const item = JSON.parse(csl_json) as Record<string, unknown>
    const authors = Array.isArray(item.author)
      ? (item.author as Array<Record<string, unknown>>)
          .map((a) => String(a.family ?? a.literal ?? '').trim())
          .filter(Boolean)
          .join(', ')
      : ''
    const issued = item.issued as { 'date-parts'?: number[][] } | undefined
    const year = issued?.['date-parts']?.[0]?.[0]
    return {
      key: String(item.id ?? ''),
      title: String(item.title ?? ''),
      authors,
      year: year ? String(year) : '',
      csl_json,
    }
  } catch {
    // An item that will not parse is skipped rather than shown as a blank row.
    // The library is not ours to repair.
    return null
  }
}

export class WritingZoteroStore {
  #state: ZoteroSnapshot = { ...EMPTY }
  #subscribers = new Set<Subscriber>()
  #all: LibraryEntry[] = []
  #restoring: Promise<void> | null = null
  #syncing: Promise<void> | null = null

  subscribe(run: Subscriber): () => void {
    this.#subscribers.add(run)
    run(this.#state)
    return () => this.#subscribers.delete(run)
  }

  #set(patch: Partial<ZoteroSnapshot>) {
    this.#state = { ...this.#state, ...patch }
    for (const run of this.#subscribers) run(this.#state)
  }

  get snapshot(): ZoteroSnapshot {
    return this.#state
  }

  /** Asks what can be said about Zotero, and says only that. */
  async probe(): Promise<ZoteroState | null> {
    this.#set({ probing: true })
    try {
      const status = await invoke<ZoteroState>('writing_zotero_probe')
      this.#set({ status, probing: false, error: null })
      return status
    } catch (error) {
      // The probe is not supposed to fail; if it does, that is our problem and
      // not a diagnosis of Zotero, so nothing is claimed about the library.
      this.#set({ probing: false, error: message(error) })
      return null
    }
  }

  /**
   * Lists the copy kept on disk, then brings it up to date with Zotero.
   *
   * The copy comes first and needs nothing from Zotero, so the library is on
   * screen at once — and stays there when Zotero is closed. Opening the tab is
   * the request; a button to be allowed to cite was one step too many.
   */
  async connect(library = '0'): Promise<void> {
    this.#restoring ??= this.#restore(library)
    await this.#restoring
    const status = await this.probe()
    if (status?.state === 'available') await this.sync(library)
  }

  /**
   * Asks Zotero what changed since the copy was made, and takes only that.
   *
   * One small request when nothing changed. A sync already running is joined
   * rather than started again.
   */
  sync(library = '0'): Promise<void> {
    this.#syncing ??= this.#sync(library).finally(() => {
      this.#syncing = null
    })
    return this.#syncing
  }

  async #restore(library: string): Promise<void> {
    try {
      const copy = await invoke<MirrorView>('writing_zotero_cached', { library })
      // A sync that finished first holds a newer library than the copy.
      if (this.#state.loaded === 0) this.#hold(copy.items)
    } catch {
      // No copy is only a slower start: the sync reads the library anyway.
    }
  }

  async #sync(library: string): Promise<void> {
    this.#set({ loading: true, error: null })
    try {
      const outcome = await invoke<SyncOutcome>('writing_zotero_sync', { library })
      if (outcome.items) this.#hold(outcome.items)
      this.#set({ loading: false })
    } catch (error) {
      // The copy is still the library as it last was, so it stays listed. With
      // no copy the list is empty — beside an error, never as an answer: a
      // library that could not be read is not a library with nothing in it.
      this.#set({ loading: false, error: message(error) })
    }
  }

  /** Makes `items` the library the list filters. */
  #hold(items: string[]) {
    this.#all = items.map(describe).filter((entry): entry is LibraryEntry => entry !== null)
    this.#set({ loaded: this.#all.length, entries: this.#filtered() })
  }

  /**
   * Asks Zotero as well as the list.
   *
   * The list already holds the whole library, so it answers for titles,
   * authors and years on its own. Zotero's search also reaches full text and
   * notes — a match inside a PDF comes back as the work it belongs to — and
   * whatever it finds only there is added below the list's matches. The
   * library that was read is left alone: a search is not a new library.
   */
  async searchLibrary(query: string, library = '0'): Promise<void> {
    this.#set({ query })
    const needle = query.trim()
    if (!needle) {
      this.#set({ entries: this.#filtered() })
      return
    }

    try {
      const page = await invoke<LibraryPage>('writing_zotero_search', { library, query: needle })
      // The box moved on while Zotero was answering; this answers nothing now.
      if (this.#state.query !== query) return
      const matched = this.#filtered()
      const shown = new Set(matched.map((entry) => entry.csl_json))
      const found = page.items
        .map(describe)
        .filter((entry): entry is LibraryEntry => entry !== null && !shown.has(entry.csl_json))
      this.#set({
        total: page.total,
        entries: [...matched, ...found].slice(0, VISIBLE),
        error: null,
      })
    } catch (error) {
      if (this.#state.query !== query) return
      this.#set({ error: message(error) })
    }
  }

  /** Narrows what is already on screen. Typing never asks the library. */
  search(query: string): void {
    this.#set({ query, entries: this.#filtered(query) })
  }

  /** Filters what has been read. The library is not asked again to type. */
  #filtered(query = this.#state.query): LibraryEntry[] {
    const needle = query.trim().toLowerCase()
    if (!needle) return this.#all.slice(0, VISIBLE)
    return this.#all
      .filter(
        (entry) =>
          entry.title.toLowerCase().includes(needle) ||
          entry.authors.toLowerCase().includes(needle) ||
          entry.year.includes(needle)
      )
      .slice(0, VISIBLE)
  }
}

export const writingZotero = new WritingZoteroStore()
