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
    updatedAt: new Date(collection.updatedAt),
    view: { name: 'collection', id: collection.id, collectionName: collection.name },
  }))

  const writingEntries: HomeRecentEntry[] = sources.writing.map((document) => ({
    kind: 'writing',
    id: document.id,
    title: document.title,
    size: null,
    updatedAt: new Date(document.updated_at),
    view: { name: 'writing', documentId: document.id, documentTitle: document.title },
  }))

  const researchEntries: HomeRecentEntry[] = sources.research.map((job) => ({
    kind: 'research',
    id: job.id,
    title: job.title,
    size: null,
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

  const continuar = mergeRecentActivity({
    collections: collectionSources,
    writing: writingSources,
    research: researchSources,
  })

  const activity = mapRecentlyImported(recentlyImportedSources)

  const isFirstRun =
    collectionRows.length === 0 && writingSources.length === 0 && researchSources.length === 0

  return { stats, continuar, activity, isFirstRun }
}
