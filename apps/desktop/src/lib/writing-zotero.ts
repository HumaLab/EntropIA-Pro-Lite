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

const PAGE_SIZE = 100
/**
 * How many pages are asked for at once.
 *
 * Zotero builds CSL-JSON for every item it sends, so a page is not free on its
 * side either. Measured on a library of ~2,800 works (29 pages): one at a time
 * took 112 s, four 25 s, eight 13 s — without flooding the local API with
 * every page at once.
 */
const CONCURRENCY = 8
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
  #reading: Promise<void> | null = null

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
   * Probes, and reads the library as soon as Zotero says it can be read.
   *
   * Opening the tab is the request; a separate button to be allowed to cite
   * was one step too many.
   */
  async connect(library = '0'): Promise<void> {
    const status = await this.probe()
    if (status?.state === 'available') await this.ensureLoaded(library)
  }

  /** Reads the library unless it already has been, or is being, read. */
  ensureLoaded(library = '0'): Promise<void> {
    if (this.#state.loaded > 0) return Promise.resolve()
    this.#reading ??= this.load(library).finally(() => {
      this.#reading = null
    })
    return this.#reading
  }

  /**
   * Reads the whole library.
   *
   * The backend clamps every request to a page, so a library arrives in pages
   * whether or not the caller wanted them. There is no ceiling: a ceiling is how
   * a reference that *is* in the library went missing from the list.
   */
  async load(library = '0'): Promise<void> {
    this.#set({ loading: true, error: null })
    try {
      const { entries, total } = await this.#readAll(library)
      this.#all = entries
      this.#set({
        loading: false,
        loaded: entries.length,
        total,
        entries: this.#filtered(),
      })
    } catch (error) {
      // A library that could not be read is not an empty library, and the two
      // must not look the same to someone hunting for a reference.
      this.#all = []
      this.#set({ loading: false, loaded: 0, error: message(error), entries: [] })
    }
  }

  /**
   * Asks Zotero as well as the list.
   *
   * The list already holds the whole library, so it answers for titles,
   * authors and years on its own. Zotero's search also reaches full text and
   * notes — a match inside a PDF comes back as the work it belongs to — and
   * whatever it finds only there is added below the list's matches.
   * The library that was read is left alone: a search is not a new library.
   */
  async searchLibrary(query: string, library = '0'): Promise<void> {
    this.#set({ query })
    const needle = query.trim()
    if (!needle) {
      if (this.#state.loaded === 0) await this.ensureLoaded(library)
      else this.#set({ entries: this.#filtered() })
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

  #page(library: string, start: number): Promise<LibraryPage> {
    return invoke<LibraryPage>('writing_zotero_items', { library, start, limit: PAGE_SIZE })
  }

  /**
   * Every page of the library, in the order the library gave them.
   *
   * The first page carries `Total-Results`, which makes the rest plannable, so
   * those pages are asked for together. Without that count there is nothing to
   * plan with, and the library is walked until it says there is no more.
   */
  async #readAll(library: string): Promise<{ entries: LibraryEntry[]; total: number | null }> {
    const first = await this.#page(library, 0)
    const pages: string[][] = [first.items]

    if (first.has_more && first.total !== null) {
      const starts: number[] = []
      for (let start = PAGE_SIZE; start < first.total; start += PAGE_SIZE) starts.push(start)
      const rest: string[][] = new Array(starts.length)
      let next = 0
      const worker = async () => {
        while (next < starts.length) {
          const index = next++
          rest[index] = (await this.#page(library, starts[index]!)).items
        }
      }
      await Promise.all(Array.from({ length: Math.min(CONCURRENCY, starts.length) }, worker))
      pages.push(...rest)
    } else {
      let page = first
      let start = 0
      // An empty page ends the walk even if the library claims more, so a
      // confused answer cannot page forever.
      while (page.has_more && page.items.length > 0) {
        start += PAGE_SIZE
        page = await this.#page(library, start)
        pages.push(page.items)
      }
    }

    // Pages over a library that changed while being read can overlap; the
    // same work is still one work to choose. "Same" is the whole item, never
    // the citation key alone: two different works can share a key.
    const seen = new Set<string>()
    const entries: LibraryEntry[] = []
    for (const csl of pages.flat()) {
      const entry = describe(csl)
      if (!entry || seen.has(csl)) continue
      seen.add(csl)
      entries.push(entry)
    }
    return { entries, total: first.total }
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
