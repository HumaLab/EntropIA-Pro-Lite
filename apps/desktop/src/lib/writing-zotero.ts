import { invoke } from '@tauri-apps/api/core'
import {
  bibliographySearchWorks,
  type BibliographySearchHit,
  type BibliographySearchResponse,
} from './bibliography-search'
import {
  newBatchRequestId,
  processingSyncBibliographyLibrary,
  type BibliographySyncResponse,
} from './batch-processing'

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

/** One work returned by the local connector or mirror. */
export interface ZoteroItem {
  /** Native Zotero item key; deliberately not the CSL `id`. */
  key: string
  itemVersion: number
  libraryType: string
  libraryId: string
  /** The untouched CSL-JSON. What gets cited, and what gets snapshotted. */
  cslJson: string
}

export interface LibraryPage {
  /** Items with native Zotero identity beside their CSL-JSON. */
  items: ZoteroItem[]
  /** `Last-Modified-Version` — the only instance identity Zotero 9 offers. */
  version: number | null
  /** What the library says it holds for this query, when it says so. */
  total: number | null
  has_more: boolean
}

/** The copy of the library kept on disk, as the backend hands it over. */
interface MirrorView {
  items: ZoteroItem[]
  version: number | null
}

/** What a sync did. `items` is the whole library, and only when it changed. */
interface SyncOutcome {
  items: ZoteroItem[] | null
  version: number | null
  fetched: number
  removed: number
}

/** One work, read out of its CSL-JSON just enough to list it. */
export interface LibraryEntry {
  /** Native Zotero identity, kept separate from the CSL citation id. */
  key: string
  itemVersion: number
  libraryType: string
  libraryId: string
  title: string
  authors: string
  year: string
  /** The untouched CSL-JSON. What gets cited, and what gets snapshotted. */
  csl_json: string
  /** Set only on works found by meaning (not by any text match) in the last search. */
  semantic?: true
}

/**
 * What the meaning-based half of the last search could say. It travels beside
 * the results so a missing semantic leg is never mistaken for "no similar
 * works": `not_synced` (library never synced into EntropIA), `lexical_only`
 * (no active embedding generation or the query could not be embedded) and
 * `failed` are each stated, never inferred.
 */
export type SemanticSearchStatus = 'idle' | 'ok' | 'not_synced' | 'lexical_only' | 'failed'

export interface ZoteroLibrarySelection {
  libraryType: 'user' | 'group'
  libraryId: string
}

/** Admission state only: the background worker may still be pending. */
export interface BibliographySyncRequestState {
  loading: boolean
  error: string | null
  requested: BibliographySyncResponse | null
}

/** The personal default: exactly user/0, unchanged by E1c-1. */
const PERSONAL: ZoteroLibrarySelection = { libraryType: 'user', libraryId: '0' }

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
  /** Which library the list belongs to (E1c-1: explicit selection). */
  selection: ZoteroLibrarySelection
  /** Manual scheduler admission for this selection, not worker completion. */
  bibliographySync: BibliographySyncRequestState
  semanticStatus: SemanticSearchStatus
}

const EMPTY_BIBLIOGRAPHY_SYNC: BibliographySyncRequestState = {
  loading: false,
  error: null,
  requested: null,
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
  selection: { ...PERSONAL },
  bibliographySync: { ...EMPTY_BIBLIOGRAPHY_SYNC },
  semanticStatus: 'idle',
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

/** A catalog hit as a list entry, from the catalog's own copy of the work. */
function entryFromHit(hit: BibliographySearchHit): LibraryEntry {
  return {
    key: hit.itemKey,
    itemVersion: 0,
    libraryType: hit.libraryType,
    libraryId: hit.libraryNativeId,
    title: hit.title,
    authors: hit.authors,
    year: hit.year ? String(hit.year) : '',
    csl_json: hit.cslJson,
  }
}

/** Reads the few fields a list needs, without disturbing the CSL-JSON itself. */
function describe(source: ZoteroItem | string): LibraryEntry | null {
  const legacy = typeof source === 'string'
  const csl_json = legacy ? source : source.cslJson

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
      // CSL-only responses predate the transport identity. Current
      // connector/mirror items always take this key from Zotero, not CSL.
      key: legacy ? String(item.id ?? '') : source.key,
      itemVersion: legacy ? 0 : source.itemVersion,
      libraryType: legacy ? 'user' : source.libraryType,
      libraryId: legacy ? '0' : source.libraryId,
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
  #state: ZoteroSnapshot = {
    ...EMPTY,
    selection: { ...EMPTY.selection },
    bibliographySync: { ...EMPTY.bibliographySync },
  }
  #subscribers = new Set<Subscriber>()
  #all: LibraryEntry[] = []
  #restoring: Promise<void> | null = null
  #syncing: Promise<void> | null = null
  #bibliographySyncing: { epoch: number; promise: Promise<void> } | null = null
  #selection: ZoteroLibrarySelection = { ...PERSONAL }
  /** Bumped on every effective selection change; late responses compare it. */
  #epoch = 0

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

  /** The library the list belongs to. A copy: mutating it changes nothing. */
  get selection(): ZoteroLibrarySelection {
    return { ...this.#selection }
  }

  /**
   * Switches the library the store reads (E1c-1: explicit selection, no UI).
   *
   * Library B never shows library A's data: in-flight bookkeeping is reset
   * so the next connect()/sync() fetches B, and the held list is cleared at
   * once. Late answers from the previous selection carry its epoch and are
   * discarded on arrival. Selecting the current library changes nothing.
   */
  select(libraryType: ZoteroLibrarySelection['libraryType'], libraryId: string): void {
    if (this.#selection.libraryType === libraryType && this.#selection.libraryId === libraryId) {
      return
    }
    this.#selection = { libraryType, libraryId }
    this.#epoch += 1
    this.#restoring = null
    this.#syncing = null
    this.#bibliographySyncing = null
    this.#all = []
    this.#set({
      selection: { ...this.#selection },
      entries: [],
      loaded: 0,
      total: null,
      query: '',
      loading: false,
      error: null,
      bibliographySync: { ...EMPTY_BIBLIOGRAPHY_SYNC },
      semanticStatus: 'idle',
    })
  }

  #sameSelection(selection: ZoteroLibrarySelection): boolean {
    return (
      this.#selection.libraryType === selection.libraryType &&
      this.#selection.libraryId === selection.libraryId
    )
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
  async connect(): Promise<void> {
    const epoch = this.#epoch
    const selection = { ...this.#selection }
    const restoring = (this.#restoring ??= this.#restore(selection, epoch))
    await restoring
    if (epoch !== this.#epoch) return
    const status = await this.probe()
    if (epoch !== this.#epoch) return
    if (status?.state === 'available') await this.sync()
  }

  /**
   * Asks Zotero what changed since the copy was made, and takes only that.
   *
   * One small request when nothing changed. A sync already running is joined
   * rather than started again.
   */
  sync(): Promise<void> {
    if (!this.#syncing) {
      const epoch = this.#epoch
      const selection = { ...this.#selection }
      const task = this.#sync(selection, epoch).finally(() => {
        if (this.#syncing === task) this.#syncing = null
      })
      this.#syncing = task
    }
    return this.#syncing
  }

  /**
   * Requests durable background synchronization for the selected library.
   *
   * This resolves when the scheduler accepts the request, not when its worker
   * finishes. Concurrent requests for the same selection join one IPC call.
   */
  requestBibliographySync(): Promise<void> {
    const current = this.#bibliographySyncing
    if (current?.epoch === this.#epoch) return current.promise

    const epoch = this.#epoch
    const selection = { ...this.#selection }
    const task = this.#requestBibliographySync(selection, epoch).finally(() => {
      if (this.#bibliographySyncing?.promise === task) this.#bibliographySyncing = null
    })
    this.#bibliographySyncing = { epoch, promise: task }
    return task
  }

  async #requestBibliographySync(selection: ZoteroLibrarySelection, epoch: number): Promise<void> {
    this.#set({
      bibliographySync: { loading: true, error: null, requested: null },
    })
    try {
      const requested = await processingSyncBibliographyLibrary(
        newBatchRequestId(),
        selection.libraryType,
        selection.libraryId
      )
      if (epoch !== this.#epoch || !this.#sameSelection(selection)) return
      this.#set({
        bibliographySync: { loading: false, error: null, requested },
      })
    } catch (error) {
      if (epoch !== this.#epoch || !this.#sameSelection(selection)) return
      this.#set({
        bibliographySync: { loading: false, error: message(error), requested: null },
      })
    }
  }

  async #restore(selection: ZoteroLibrarySelection, epoch: number): Promise<void> {
    try {
      const copy = await invoke<MirrorView>('writing_zotero_cached', {
        libraryType: selection.libraryType,
        libraryId: selection.libraryId,
      })
      // A late answer from the previous selection changes nothing: the epoch
      // subsumes the loaded===0 fast path below, which keeps its original
      // meaning within one selection only.
      if (epoch !== this.#epoch) return
      if (!this.#sameSelection(selection)) return
      // A sync that finished first holds a newer library than the copy.
      if (this.#state.loaded === 0) this.#hold(copy.items)
    } catch {
      // No copy is only a slower start: the sync reads the library anyway.
    }
  }

  async #sync(selection: ZoteroLibrarySelection, epoch: number): Promise<void> {
    this.#set({ loading: true, error: null })
    try {
      const outcome = await invoke<SyncOutcome>('writing_zotero_sync', {
        libraryType: selection.libraryType,
        libraryId: selection.libraryId,
      })
      if (epoch !== this.#epoch) return
      if (!this.#sameSelection(selection)) return
      if (outcome.items) this.#hold(outcome.items)
      this.#set({ loading: false })
    } catch (error) {
      if (epoch !== this.#epoch) return
      if (!this.#sameSelection(selection)) return
      // The copy is still the library as it last was, so it stays listed. With
      // no copy the list is empty — beside an error, never as an answer: a
      // library that could not be read is not a library with nothing in it.
      this.#set({ loading: false, error: message(error) })
    }
  }

  /** Makes `items` the library the list filters. */
  #hold(items: Array<ZoteroItem | string>) {
    this.#all = items.map(describe).filter((entry): entry is LibraryEntry => entry !== null)
    this.#set({ loaded: this.#all.length, entries: this.#filtered() })
  }

  /**
   * Asks Zotero and the bibliography as well as the list.
   *
   * The list already holds the whole library, so it answers for titles,
   * authors and years on its own. Zotero's search also reaches full text and
   * notes — a match inside a PDF comes back as the work it belongs to — and
   * whatever it finds only there is added below the list's matches. The
   * library that was read is left alone: a search is not a new library.
   *
   * The bibliography's hybrid search (`search_works`: lexical plus semantic
   * over work profiles) runs beside it, scoped to this library. Ranking rule:
   * the two engines' scores are on unrelated scales and are never added or
   * compared. Text matches (list, then Zotero-only) come first, in their own
   * order; then every work only the bibliography found, in the order it
   * ranked them, flagged `semantic` when its hit used the vector leg. A hit
   * is the same work as a text match when the Zotero item key is equal
   * (the search is already scoped to one library). A hit whose key is not in
   * the library read from Zotero is dropped: it could not be cited.
   */
  async searchLibrary(query: string): Promise<void> {
    const selection = { ...this.#selection }
    const epoch = this.#epoch
    this.#set({ query })
    const needle = query.trim()
    if (!needle) {
      this.#set({ entries: this.#filtered(), semanticStatus: 'idle' })
      return
    }

    // Each leg shows as soon as it answers: a closed or slow Zotero never
    // holds the meaning-based hits back, and the other way round.
    let zotero: PromiseSettledResult<LibraryPage> | null = null
    let meaning: PromiseSettledResult<BibliographySearchResponse> | null = null
    const stale = () =>
      epoch !== this.#epoch || !this.#sameSelection(selection) || this.#state.query !== query
    const show = () => {
      // The box moved on while they were answering, or the library did:
      // a late answer of another query or another selection changes nothing.
      if (stale()) return
      this.#set(this.#searchPatch(zotero, meaning))
    }
    const settle = <T>(
      promise: Promise<T>,
      keep: (result: PromiseSettledResult<T>) => void
    ): Promise<void> =>
      promise
        .then(
          (value) => keep({ status: 'fulfilled', value }),
          (reason) => keep({ status: 'rejected', reason })
        )
        .then(show)

    await Promise.all([
      settle(
        invoke<LibraryPage>('writing_zotero_search', {
          libraryType: selection.libraryType,
          libraryId: selection.libraryId,
          query: needle,
        }),
        (result) => (zotero = result)
      ),
      settle(
        bibliographySearchWorks(needle, { zoteroLibrary: selection }),
        (result) => (meaning = result)
      ),
    ])
  }

  /** What the box shows given whichever of the two searches has answered. */
  #searchPatch(
    zotero: PromiseSettledResult<LibraryPage> | null,
    meaning: PromiseSettledResult<BibliographySearchResponse> | null
  ): Partial<ZoteroSnapshot> {
    const matched = this.#filtered()
    const shown = new Set(matched.map((entry) => entry.csl_json))
    const found =
      zotero?.status === 'fulfilled'
        ? zotero.value.items
            .map(describe)
            .filter((entry): entry is LibraryEntry => entry !== null && !shown.has(entry.csl_json))
        : []
    const entries = [...matched, ...found]

    let semanticStatus: SemanticSearchStatus = meaning === null ? 'idle' : 'failed'
    if (meaning?.status === 'fulfilled') {
      const answer = meaning.value
      semanticStatus = !answer.librarySynced
        ? 'not_synced'
        : !answer.vectorAvailable
          ? 'lexical_only'
          : 'ok'
      const keys = new Set(entries.map((entry) => entry.key))
      const library = new Map(this.#all.map((entry) => [entry.key, entry]))
      for (const hit of answer.hits) {
        if (keys.has(hit.itemKey)) continue
        // A work the held list lacks (a list from an older copy, or one not
        // refreshed yet) is still a work of the catalog: listed from the
        // catalog's own title, authors, year and CSL-JSON, which is also what
        // a citation snapshots. Without CSL it is shown but cannot be cited.
        const entry = library.get(hit.itemKey) ?? entryFromHit(hit)
        keys.add(entry.key)
        entries.push(hit.method === 'lexical' ? entry : { ...entry, semantic: true })
      }
    }

    return {
      semanticStatus,
      ...(zotero?.status === 'fulfilled'
        ? { total: zotero.value.total, error: null }
        : zotero?.status === 'rejected'
          ? { error: message(zotero.reason) }
          : {}),
      entries: entries.slice(0, VISIBLE),
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
