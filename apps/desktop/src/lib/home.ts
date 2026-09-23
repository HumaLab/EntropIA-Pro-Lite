/**
 * Data for the Inicio (home) view: the corpus-wide stats line and the merged
 * "recent activity" list across collections, writing documents and research
 * jobs (odd/tasks/home-view.md, T2).
 */

import type { CorpusStats } from '@entropia/store'
import { getStore } from './db'
import { writing } from './writing'
import { researchList } from './research'
import type { View } from './navigation'

export interface HomeCollectionSource {
  id: string
  name: string
  updatedAt: number
  itemCount: number
}

export interface HomeWritingDocumentSource {
  id: string
  title: string
  updated_at: number
  /** Already loaded by `listDocuments()` — reused here for the Continuar word count (T3f). */
  current_content_json: string
}

export interface HomeResearchJobSource {
  id: string
  title: string
}

/** One row from `ItemRepo.findRecentlyImported` — a document, not a workspace. */
export interface HomeRecentlyImportedSource {
  id: string
  title: string
  collectionId: string
  collectionName: string
  createdAt: number
}

export type HomeRecentEntryKind = 'collection' | 'writing' | 'research'

export interface HomeRecentEntry {
  kind: HomeRecentEntryKind
  id: string
  title: string
  /** Item count for a collection; `null` for writing documents and research jobs. */
  size: number | null
  /**
   * The word count for a writing document's manuscript, computed from its
   * already-loaded `current_content_json` (T3f). `null` for a collection or
   * research entry, and for a writing entry outside the top 3 slots shown in
   * Continuar — see {@link attachContinuarWordCounts}.
   */
  wordCount: number | null
  /**
   * `null` when the source carries no modified time at all — today, only
   * research jobs (`ResearchJobSummary` has no timestamp field; research.ts
   * is owned by another session and is not modified here). Entries with a
   * `null` timestamp always sort after every timestamped entry.
   */
  updatedAt: Date | null
  view: View
}

export interface HomeRecentSources {
  collections: HomeCollectionSource[]
  writing: HomeWritingDocumentSource[]
  research: HomeResearchJobSource[]
}

/**
 * Merges the three home-relevant sources into one list sorted by modified
 * time, most recent first. Pure and side-effect free so ordering, mixing,
 * missing timestamps and empty inputs can be tested directly.
 */
export function mergeRecentActivity(sources: HomeRecentSources): HomeRecentEntry[] {
  const collectionEntries: HomeRecentEntry[] = sources.collections.map((collection) => ({
    kind: 'collection',
    id: collection.id,
    title: collection.name,
    size: collection.itemCount,
    wordCount: null,
    updatedAt: new Date(collection.updatedAt),
    view: { name: 'collection', id: collection.id, collectionName: collection.name },
  }))

  // wordCount is computed later, and only for the entries that make it into
  // Continuar's top 3 (attachContinuarWordCounts) — never here, where every
  // writing document in the workspace would pay for the walk.
  const writingEntries: HomeRecentEntry[] = sources.writing.map((document) => ({
    kind: 'writing',
    id: document.id,
    title: document.title,
    size: null,
    wordCount: null,
    updatedAt: new Date(document.updated_at),
    view: { name: 'writing', documentId: document.id, documentTitle: document.title },
  }))

  const researchEntries: HomeRecentEntry[] = sources.research.map((job) => ({
    kind: 'research',
    id: job.id,
    title: job.title,
    size: null,
    wordCount: null,
    updatedAt: null,
    view: { name: 'investigation', jobId: job.id, title: job.title },
  }))

  return [...collectionEntries, ...writingEntries, ...researchEntries].sort((a, b) => {
    if (a.updatedAt && b.updatedAt) return b.updatedAt.getTime() - a.updatedAt.getTime()
    if (a.updatedAt && !b.updatedAt) return -1
    if (!a.updatedAt && b.updatedAt) return 1
    return 0
  })
}

/**
 * Counts words in a manuscript's canonical JSON — packages/ui's
 * `CanonicalDocument` envelope, `{ schemaVersion, doc }`, where `doc` is a
 * Tiptap/ProseMirror document tree (T3f).
 *
 * Pure and tolerant on purpose: this runs over `current_content_json` exactly
 * as stored, without going through `parseCanonical` (which validates against
 * the live Tiptap schema and is overkill for a display-only count). Invalid
 * JSON or an unrecognised shape yields `null` rather than throwing, so one
 * malformed manuscript never breaks the Continuar panel — that entry simply
 * shows no word-count datum.
 */
export function countManuscriptWords(input: unknown): number | null {
  let candidate: unknown = input
  if (typeof input === 'string') {
    try {
      candidate = JSON.parse(input)
    } catch {
      return null
    }
  }
  if (typeof candidate !== 'object' || candidate === null) return null

  let count = 0
  const walk = (node: unknown): void => {
    if (Array.isArray(node)) {
      for (const child of node) walk(child)
      return
    }
    if (typeof node !== 'object' || node === null) return
    const record = node as { text?: unknown; content?: unknown }
    if (typeof record.text === 'string') {
      count += record.text.split(/\s+/u).filter((word) => word.length > 0).length
    }
    if (Array.isArray(record.content)) walk(record.content)
  }

  const envelope = candidate as { doc?: unknown }
  walk(envelope.doc ?? candidate)
  return count
}

/**
 * The exact default titles `WritingStore.createDocument` gives a new document
 * (`writing.newDocumentTitle` in `$lib/i18n`), for every locale the app ships.
 * Compared verbatim rather than through `t()`: a document created while the
 * app was in one locale must still read as untitled once Continuar is shown
 * in the other. `home.test.ts` keeps this list equal to `t('writing.newDocumentTitle')`.
 */
const DEFAULT_WRITING_TITLES: readonly string[] = ['Sin título', 'Untitled']

/**
 * Whether a writing document's *stored* title should be shown as untitled —
 * empty/whitespace-only, or still the app's default title (T3f). Display
 * only: callers must never write this back as the stored title.
 */
export function isUntitledWritingTitle(title: string): boolean {
  const trimmed = title.trim()
  return trimmed === '' || DEFAULT_WRITING_TITLES.includes(trimmed)
}

/**
 * Attaches a word-count datum to the writing entries that will actually be
 * shown in Continuar (the first `limit` slots of the already-sorted list),
 * leaving every other entry untouched. Kept separate from
 * {@link mergeRecentActivity} so the (comparatively expensive) manuscript
 * walk never runs for a writing document that Continuar will not display.
 */
export function attachContinuarWordCounts(
  entries: HomeRecentEntry[],
  writingSources: HomeWritingDocumentSource[],
  limit = 3
): HomeRecentEntry[] {
  const contentById = new Map(writingSources.map((doc) => [doc.id, doc.current_content_json]))
  return entries.map((entry, index) => {
    if (index >= limit || entry.kind !== 'writing') return entry
    const json = contentById.get(entry.id)
    if (json === undefined) return entry
    return { ...entry, wordCount: countManuscriptWords(json) }
  })
}

/**
 * One entry in "Actividad reciente": a recently imported document. Kept as
 * its own shape (not folded into {@link HomeRecentEntry}) so the two panels
 * can never accidentally show the same kind of thing — Continuar resumes a
 * workspace (collection/writing/research), Actividad reciente reports what
 * changed in the corpus (documents).
 */
export interface HomeActivityEntry {
  id: string
  title: string
  collectionName: string
  createdAt: Date
  view: View
}

/** Pure and side-effect free: the repository already returns newest first. */
export function mapRecentlyImported(sources: HomeRecentlyImportedSource[]): HomeActivityEntry[] {
  return sources.map((source) => ({
    id: source.id,
    title: source.title,
    collectionName: source.collectionName,
    createdAt: new Date(source.createdAt),
    view: {
      name: 'item',
      collectionId: source.collectionId,
      collectionName: source.collectionName,
      itemId: source.id,
      itemTitle: source.title,
    },
  }))
}

export interface HomeSnapshot {
  stats: CorpusStats
  /** Up to 3 resumable workspaces — collections, writing documents, research. */
  continuar: HomeRecentEntry[]
  /** Up to 5 recently imported documents — never the same entities as Continuar. */
  activity: HomeActivityEntry[]
  isFirstRun: boolean
}

/** Never throws: a source that fails to load is treated as empty. */
async function loadWritingSources(): Promise<HomeWritingDocumentSource[]> {
  try {
    await writing.listDocuments()
    return writing.snapshot.documents.map((document) => ({
      id: document.id,
      title: document.title,
      updated_at: document.updated_at,
      current_content_json: document.current_content_json,
    }))
  } catch {
    return []
  }
}

/** Never throws: a source that fails to load is treated as empty. */
async function loadResearchSources(): Promise<HomeResearchJobSource[]> {
  try {
    const response = await researchList()
    return response.jobs.map((job) => ({ id: job.id, title: job.title }))
  } catch {
    return []
  }
}

/** Never throws: a source that fails to load is treated as empty. */
async function loadRecentlyImportedSources(): Promise<HomeRecentlyImportedSource[]> {
  try {
    return await getStore().items.findRecentlyImported(5)
  } catch {
    return []
  }
}

/**
 * Loads the whole home overview: corpus stats, the merged Continuar list and
 * the recently-imported Actividad reciente list. One source failing (e.g. the
 * research command erroring) never fails the whole snapshot — it is treated
 * as having nothing to contribute.
 */
export async function loadHomeSnapshot(): Promise<HomeSnapshot> {
  const store = getStore()

  const [stats, collectionRows, writingSources, researchSources, recentlyImportedSources] =
    await Promise.all([
      store.items.getCorpusStats(),
      store.collections.findAll(),
      loadWritingSources(),
      loadResearchSources(),
      loadRecentlyImportedSources(),
    ])

  const collectionSources: HomeCollectionSource[] = await Promise.all(
    collectionRows.map(async (collection) => ({
      id: collection.id,
      name: collection.name,
      updatedAt: collection.updatedAt,
      itemCount: await store.collections.countItems(collection.id),
    }))
  )

  const continuar = attachContinuarWordCounts(
    mergeRecentActivity({
      collections: collectionSources,
      writing: writingSources,
      research: researchSources,
    }),
    writingSources
  )

  const activity = mapRecentlyImported(recentlyImportedSources)

  const isFirstRun =
    collectionRows.length === 0 && writingSources.length === 0 && researchSources.length === 0

  return { stats, continuar, activity, isFirstRun }
}
