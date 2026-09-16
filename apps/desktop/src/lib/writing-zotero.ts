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
  /** How far through the library we have read. */
  loaded: number
  hasMore: boolean
  error: string | null
}

const EMPTY: ZoteroSnapshot = {
  status: null,
  probing: false,
  loading: false,
  query: '',
  entries: [],
  loaded: 0,
  hasMore: false,
  error: null,
}

const PAGE_SIZE = 100
/**
 * How much of a library this will read before stopping.
 *
 * Zotero libraries reach tens of thousands of items and this reads them to
 * filter in memory, which is fine for a few thousand and absurd beyond that. A
 * ceiling that reports itself is better than one that quietly truncates: the
 * panel says it stopped, rather than implying the rest does not exist.
 */
const MAX_ITEMS = 2_000

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
   * Reads the library, a page at a time.
   *
   * Paging is not optional on this side either: the backend clamps every
   * request, so a library arrives in pages whether or not the caller wanted
   * them. What is optional is how many pages to ask for, and that stops at a
   * ceiling which reports itself.
   */
  async load(library = '0'): Promise<void> {
    this.#set({ loading: true, error: null })
    this.#all = []
    let start = 0

    try {
      for (;;) {
        const page = await invoke<LibraryPage>('writing_zotero_items', {
          library,
          start,
          limit: PAGE_SIZE,
        })
        for (const csl of page.items) {
          const entry = describe(csl)
          if (entry) this.#all.push(entry)
        }
        start += PAGE_SIZE
        if (!page.has_more || this.#all.length >= MAX_ITEMS) {
          this.#set({
            loading: false,
            loaded: this.#all.length,
            hasMore: page.has_more,
            entries: this.#filtered(),
          })
          return
        }
      }
    } catch (error) {
      // A library that could not be read is not an empty library, and the two
      // must not look the same to someone hunting for a reference.
      this.#set({ loading: false, error: message(error), entries: [] })
    }
  }

  search(query: string): void {
    this.#set({ query, entries: this.#filtered(query) })
  }

  /** Filters what has been read. The library is not asked again to type. */
  #filtered(query = this.#state.query): LibraryEntry[] {
    const needle = query.trim().toLowerCase()
    if (!needle) return this.#all.slice(0, 200)
    return this.#all
      .filter(
        (entry) =>
          entry.title.toLowerCase().includes(needle) ||
          entry.authors.toLowerCase().includes(needle) ||
          entry.year.includes(needle)
      )
      .slice(0, 200)
  }
}

export const writingZotero = new WritingZoteroStore()
