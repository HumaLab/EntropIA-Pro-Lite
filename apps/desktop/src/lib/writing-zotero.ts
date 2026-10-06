import { invoke } from '@tauri-apps/api/core'
import {
  bibliographySearchPassages,
  bibliographySearchWorks,
  type BibliographyPassage,
  type BibliographyPassagesResponse,
  type BibliographySearchHit,
  type BibliographySearchResponse,
  type PassageMatchKind,
} from './bibliography-search'
import { SearchPreferences, searchPreferences } from './search-preferences'
import { matchesQuery } from './text-fold'
import {
  newBatchRequestId,
  processingSyncBibliographyLibrary,
  type BibliographySyncResponse,
} from './batch-processing'
import { t } from './i18n'

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
  /**
   * Set only on works found by what their passages say (not by title, author
   * or profile) in the last search: how the words matched and which ones.
   */
  content?: { kind: Exclude<PassageMatchKind, 'meaning'>; terms: string[] }
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

/**
 * What the scheduler's task for a requested sync is doing, exactly as
 * `processing_bibliography_sync_status` reads it. `state` is the task's own
 * vocabulary: `pending`, `running`, `retry_wait`, `blocked`, `interrupted`,
 * `succeeded`, `failed` or `cancelled`.
 *
 * The derived counts describe the whole unsettled backlog window — the
 * oldest sync window still holding unsettled fichas/pasajes work, reaching
 * back across newer syncs that found nothing new — while `state`,
 * `newProfiles`/`newExtractions` and the pages progress describe the
 * requested sync alone.
 */
export interface BibliographySyncStatus {
  state: string
  errorCode: string | null
  errorMessage: string | null
  progressDone: number
  progressTotal: number | null
  itemsSeen: number | null
  remoteTotal: number | null
  /** Works new or changed since the last sync (zero with `succeeded`: up to date). */
  newProfiles: number
  /** Attachments new since the last sync. */
  newExtractions: number
  /** Live profile tasks (fichas) of the backlog window: settled over queued. */
  profilesDone: number
  profilesTotal: number
  /** Live extraction tasks (pasajes) of the backlog window: settled over queued. */
  extractionsDone: number
  extractionsTotal: number
  /** Live blocked derived work of the backlog window: parked on an owner-side change, never done. */
  profilesBlocked: number
  extractionsBlocked: number
  /** What each kind's blocked work is parked on; null while that kind has none. */
  profilesBlockedReason: BibliographyBlockedReason | null
  extractionsBlockedReason: BibliographyBlockedReason | null
  /** What the derived backlog still needs in ms; null while it cannot be estimated. */
  etaMs: number | null
}

/** Why one kind of blocked work waits: a stable code beside its recorded message. */
export interface BibliographyBlockedReason {
  code: string | null
  message: string | null
}

/** The last thing known about the sync, or why it could not be read. */
export interface BibliographySyncProgress {
  status: BibliographySyncStatus | null
  unreadable: string | null
}

/** Task states that will still change by themselves while the app runs. */
const SYNC_FOLLOWED_STATES = new Set(['pending', 'running', 'retry_wait', 'interrupted'])
const SYNC_POLL_MS = 1500

/** The derived backlog of the window: fichas (works) and pasajes (passages). */
export interface BibliographyDerivedProgress {
  worksDone: number
  worksTotal: number
  worksBlocked: number
  passagesDone: number
  passagesTotal: number
  passagesBlocked: number
  etaMs: number | null
  /** Tasks still unsettled in the window; zero = nothing left to show. */
  remaining: number
  /** Unsettled work that can still move by itself; zero = nothing left to follow. */
  remainingActive: number
  /** Unsettled work parked on a change only the owner can make. */
  blocked: number
  /** Why the parked work waits, in the app's words where it can word them. */
  blockedReason: string
}

/** What a block is about, in the app's own words where it can name it. */
type BlockedFlavor = 'embedding' | 'ocr' | 'other'

function blockedReasonFlavor(
  kind: 'profiles' | 'extractions',
  reason: BibliographyBlockedReason | null
): { flavor: BlockedFlavor; text: string } | null {
  if (!reason) return null
  const code = reason.code ?? ''
  const message = reason.message ?? ''
  if (code === 'configuration_required_embedding') {
    return { flavor: 'embedding', text: t('writing.zoteroBlockedReasonEmbedding') }
  }
  if (code === 'configuration_required_ocr') {
    return { flavor: 'ocr', text: t('writing.zoteroBlockedReasonOcr') }
  }
  if (code === 'configuration_required') {
    // Rows recorded before the subcodes existed, worded by kind: the OCR
    // executor signs its messages with the stable `configuration:` prefix,
    // and a profile's plain configuration block is the embedding engine's.
    if (message.startsWith('configuration:')) {
      return { flavor: 'ocr', text: t('writing.zoteroBlockedReasonOcr') }
    }
    if (kind === 'profiles') {
      return { flavor: 'embedding', text: t('writing.zoteroBlockedReasonEmbedding') }
    }
  }
  // Anything else (contract changes, blocks with no stable vocabulary) is
  // shown as the executor recorded it: only its own message is honest.
  return { flavor: 'other', text: message || code }
}

/**
 * Why the parked work waits, in the app's own words where a stable code names
 * the reason and in the executor's recorded message otherwise. Each kind
 * names its own block — fichas wait on the embedding configuration
 * (OpenRouter), pasajes on the OCR configuration (GLM-OCR) — and when both
 * kinds are parked the answer names both, briefly.
 */
export function bibliographyBlockedReason(status: BibliographySyncStatus): string {
  const reasons = [
    blockedReasonFlavor('profiles', status.profilesBlockedReason),
    blockedReasonFlavor('extractions', status.extractionsBlockedReason),
  ].filter((entry): entry is { flavor: BlockedFlavor; text: string } => entry !== null)
  const flavors = new Set(reasons.map((entry) => entry.flavor))
  if (flavors.has('embedding') && flavors.has('ocr')) {
    return t('writing.zoteroBlockedReasonEmbeddingAndOcr')
  }
  return [...new Set(reasons.map((entry) => entry.text))].join(' · ')
}

/** Reads the live derived-work counts out of one sync status. */
export function bibliographyDerivedProgress(
  status: BibliographySyncStatus
): BibliographyDerivedProgress {
  const worksTotal = status.profilesTotal ?? 0
  const worksDone = status.profilesDone ?? 0
  const worksBlocked = status.profilesBlocked ?? 0
  const passagesTotal = status.extractionsTotal ?? 0
  const passagesDone = status.extractionsDone ?? 0
  const passagesBlocked = status.extractionsBlocked ?? 0
  const blocked = worksBlocked + passagesBlocked
  const remaining = worksTotal - worksDone + (passagesTotal - passagesDone)
  return {
    worksDone,
    worksTotal,
    worksBlocked,
    passagesDone,
    passagesTotal,
    passagesBlocked,
    etaMs: status.etaMs ?? null,
    remaining,
    remainingActive: remaining - blocked,
    blocked,
    blockedReason: blocked > 0 ? bibliographyBlockedReason(status) : '',
  }
}

/**
 * Whether the follower must keep reading: the task can still change by
 * itself, or its derived backlog still holds work that can move without the
 * owner. Parked (`blocked`) work is not followed: nothing about it will
 * change until the owner changes the configuration, and the next kick finds
 * it again when that happens. The screen keeps moving on the derived work,
 * not on the sync task alone.
 */
function bibliographyFollowPending(status: BibliographySyncStatus): boolean {
  if (SYNC_FOLLOWED_STATES.has(status.state)) return true
  return status.state === 'succeeded' && bibliographyDerivedProgress(status).remainingActive > 0
}

/**
 * The remaining time in human terms: `<1 min`, `N min`, `N h M min`. The
 * surrounding sentence belongs to the caller; this only names the span.
 */
export function formatEtaMs(etaMs: number): string {
  const totalMinutes = Math.floor(etaMs / 60_000)
  if (totalMinutes < 1) return t('writing.zoteroEtaUnderMinute')
  const hours = Math.floor(totalMinutes / 60)
  const minutes = totalMinutes % 60
  if (hours === 0) return t('writing.zoteroEtaMinutes', { minutes })
  return t('writing.zoteroEtaHoursMinutes', { hours, minutes })
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
  /** What the requested sync is really doing; null before any request. */
  bibliographyProgress: BibliographySyncProgress | null
  semanticStatus: SemanticSearchStatus
  /** Whether close variants of the words are searched too (the shared preference). */
  fuzzy: boolean
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
  bibliographyProgress: null,
  semanticStatus: 'idle',
  fuzzy: true,
}

/** Passages asked for the content leg; works are made of however many of them rank. */
const CONTENT_PASSAGES = 30

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

/** A passage's work as a list entry, for works the held list does not have. */
function entryFromPassage(passage: BibliographyPassage): LibraryEntry {
  return {
    key: passage.itemKey,
    itemVersion: 0,
    libraryType: passage.libraryType,
    libraryId: passage.libraryNativeId,
    title: passage.title,
    authors: passage.authors,
    year: passage.year ? String(passage.year) : '',
    csl_json: passage.cslJson,
  }
}

type ContentMatch = {
  passage: BibliographyPassage
  content: NonNullable<LibraryEntry['content']>
}

/**
 * The works whose passages carry the words, in the order their best passage
 * ranked, each with how it matched. Passages found only by meaning say
 * nothing about the words and are left to the meaning leg.
 */
function contentMatches(answer: BibliographyPassagesResponse): ContentMatch[] {
  const byWork = new Map<string, ContentMatch>()
  for (const passage of answer.passages ?? []) {
    if (passage.matchKind === 'meaning') continue
    const found = byWork.get(passage.itemKey)
    if (!found) {
      byWork.set(passage.itemKey, {
        passage,
        content: { kind: passage.matchKind, terms: [...passage.matchTerms] },
      })
      continue
    }
    // An exact passage outranks an approximate one as the work's reason.
    if (found.content.kind === 'approximate' && passage.matchKind === 'exact') {
      found.content = { kind: 'exact', terms: [...passage.matchTerms] }
    } else if (found.content.kind === passage.matchKind) {
      for (const term of passage.matchTerms) {
        if (!found.content.terms.includes(term)) found.content.terms.push(term)
      }
    }
  }
  return [...byWork.values()]
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
  /** Bumped to retire the status follower of an older request or selection. */
  #followToken = 0
  /** The follower of a sync requested in this session, while one is live. */
  #taskFollow: Promise<void> | null = null
  /** The restart-safe follower of the latest sync's backlog, while one is live. */
  #backlogFollow: Promise<void> | null = null
  #selection: ZoteroLibrarySelection = { ...PERSONAL }
  /** Bumped on every effective selection change; late responses compare it. */
  #epoch = 0
  #prefs: SearchPreferences
  /** Read once, on the first search: after that the state is the truth. */
  #prefsLoaded: Promise<void> | null = null

  constructor(prefs: SearchPreferences = searchPreferences) {
    this.#prefs = prefs
  }

  /** Reads the saved switch, once, so the panel shows it before any search. */
  loadPreferences(): Promise<void> {
    this.#prefsLoaded ??= this.#prefs.fuzzyEnabled().then((fuzzy) => this.#set({ fuzzy }))
    return this.#prefsLoaded
  }

  /** Turns approximate search on or off, remembers it, and searches again. */
  async setFuzzy(fuzzy: boolean): Promise<void> {
    await this.loadPreferences()
    this.#set({ fuzzy })
    try {
      await this.#prefs.setFuzzyEnabled(fuzzy)
    } catch (error) {
      // The switch still applies to this session; only remembering it failed.
      this.#set({ error: message(error) })
    }
    if (this.#state.query.trim()) await this.searchLibrary(this.#state.query)
  }

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
    this.#followToken += 1
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
      bibliographyProgress: null,
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
      bibliographyProgress: null,
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
      const follow = this.#followBibliographySync(requested.taskId, ++this.#followToken).finally(
        () => {
          if (this.#taskFollow === follow) this.#taskFollow = null
        }
      )
      this.#taskFollow = follow
    } catch (error) {
      if (epoch !== this.#epoch || !this.#sameSelection(selection)) return
      this.#set({
        bibliographySync: { loading: false, error: message(error), requested: null },
      })
    }
  }

  /**
   * Reads the scheduler's own task until it settles, so the screen says what
   * the sync is doing instead of what was asked for. Stops when the task
   * reaches a state that will not change by itself AND its derived backlog
   * (fichas and pasajes) has drained, when another request or another library
   * replaces it, or when the status cannot be read.
   */
  async #followBibliographySync(taskId: string, token: number): Promise<void> {
    while (token === this.#followToken) {
      let progress: BibliographySyncProgress
      try {
        const status = await invoke<BibliographySyncStatus>('processing_bibliography_sync_status', {
          taskId,
        })
        if (typeof status?.state !== 'string') throw new Error('unreadable sync status')
        progress = { status, unreadable: null }
      } catch (error) {
        progress = { status: null, unreadable: message(error) }
      }
      if (token !== this.#followToken) return
      this.#set({ bibliographyProgress: progress })
      if (!progress.status || !bibliographyFollowPending(progress.status)) return
      await new Promise((resolve) => setTimeout(resolve, SYNC_POLL_MS))
    }
  }

  /**
   * Follows the latest bibliography sync whose derived work may still be
   * draining — the restart-safe half of the follower. After an app restart no
   * sync was requested in this session, so nothing else would find the backlog
   * still draining from before; the footer asks for it at startup and whenever
   * batch work appears. Idempotent: a follow already live is joined, and a
   * sync requested in this session keeps the screen (this follower stands
   * down for it). Stops when the newest task's backlog drains, as the
   * requested follower does.
   */
  followBibliographyBacklog(): Promise<void> {
    if (this.#backlogFollow) return this.#backlogFollow
    if (this.#taskFollow) return this.#taskFollow
    const token = ++this.#followToken
    const follow = this.#followLatestBibliographySync(token).finally(() => {
      if (this.#backlogFollow === follow) this.#backlogFollow = null
    })
    this.#backlogFollow = follow
    return follow
  }

  /**
   * Reads the newest sync's status until it settles and its backlog drains.
   * Unlike the requested follower there is no task id to name and no button
   * waiting for an answer, so a missing or unreadable status claims nothing
   * at all and simply stops.
   */
  async #followLatestBibliographySync(token: number): Promise<void> {
    while (token === this.#followToken) {
      let progress: BibliographySyncProgress
      try {
        const status = await invoke<BibliographySyncStatus | null>(
          'processing_latest_bibliography_sync_status'
        )
        progress =
          status && typeof status.state === 'string'
            ? { status, unreadable: null }
            : { status: null, unreadable: null }
      } catch (error) {
        progress = { status: null, unreadable: message(error) }
      }
      if (token !== this.#followToken) return
      if (!progress.status) return
      this.#set({ bibliographyProgress: progress })
      if (!bibliographyFollowPending(progress.status)) return
      await new Promise((resolve) => setTimeout(resolve, SYNC_POLL_MS))
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
    // The saved switch decides how the held list is matched, so it is read first.
    await this.loadPreferences()
    if (this.#state.query !== query) return

    // Each leg shows as soon as it answers: a closed or slow Zotero never
    // holds the meaning-based hits back, and the other way round.
    let zotero: PromiseSettledResult<LibraryPage> | null = null
    let meaning: PromiseSettledResult<BibliographySearchResponse> | null = null
    let content: PromiseSettledResult<BibliographyPassagesResponse> | null = null
    const stale = () =>
      epoch !== this.#epoch || !this.#sameSelection(selection) || this.#state.query !== query
    const show = () => {
      // The box moved on while they were answering, or the library did:
      // a late answer of another query or another selection changes nothing.
      if (stale()) return
      this.#set(this.#searchPatch(zotero, meaning, content))
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
      // The saved switch is read before this leg only: the other two never
      // wait for it.
      settle(
        this.loadPreferences().then(() =>
          bibliographySearchPassages(needle, {
            topK: CONTENT_PASSAGES,
            fuzzy: this.#state.fuzzy,
            zoteroLibrary: selection,
          })
        ),
        (result) => (content = result)
      ),
    ])
  }

  /** What the box shows given whichever of the two searches has answered. */
  #searchPatch(
    zotero: PromiseSettledResult<LibraryPage> | null,
    meaning: PromiseSettledResult<BibliographySearchResponse> | null,
    content: PromiseSettledResult<BibliographyPassagesResponse> | null = null
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

    // Last of all: works whose passages carry the words. A work already listed
    // (by text or by meaning) is not listed twice; one the held list lacks is
    // listed from the catalog's own copy, like a meaning hit.
    if (content?.status === 'fulfilled') {
      const keys = new Set(entries.map((entry) => entry.key))
      const library = new Map(this.#all.map((entry) => [entry.key, entry]))
      for (const { passage, content: found } of contentMatches(content.value)) {
        if (keys.has(passage.itemKey)) continue
        const entry = library.get(passage.itemKey) ?? entryFromPassage(passage)
        keys.add(entry.key)
        entries.push({ ...entry, content: found })
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
    const needle = query.trim()
    if (!needle) return this.#all.slice(0, VISIBLE)
    // Accents and case never matter; the approximate switch adds typo
    // tolerance on top, like the corpus search.
    const fuzzy = this.#state.fuzzy
    return this.#all
      .filter(
        (entry) =>
          matchesQuery(entry.title, needle, fuzzy) ||
          matchesQuery(entry.authors, needle, fuzzy) ||
          entry.year.includes(needle)
      )
      .slice(0, VISIBLE)
  }
}

export const writingZotero = new WritingZoteroStore()
