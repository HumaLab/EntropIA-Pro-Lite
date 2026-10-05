import { invoke } from '@tauri-apps/api/core'

export interface BibliographySearchFilters {
  libraryIds?: string[]
  yearFrom?: number
  yearTo?: number
  itemTypes?: string[]
  tags?: string[]
}

export interface BibliographySearchHit {
  itemId: string
  itemKey: string
  libraryId: string
  title: string
  method: 'lexical' | 'vector' | 'hybrid'
  lexicalScore: number | null
  vectorScore: number | null
  fusedScore: number
  contractHash: string | null
  generationId: string | null
  /** CSL family names (or literal names), comma-separated; empty when none. */
  authors: string
  year: number | null
  /** Display name of the Zotero library the work belongs to. */
  libraryName: string
  /** The library's native Zotero identity: what the ficha opens by. */
  libraryType: string
  libraryNativeId: string
  /** The catalog's last CSL-JSON, the ficha's offline fallback. */
  cslJson: string
}

export interface BibliographySearchResponse {
  hits: BibliographySearchHit[]
  vectorAvailable: boolean
  activeGenerationId: string | null
  contractHash: string
  /**
   * False only when the search was scoped to a Zotero library that was never
   * synced into EntropIA: there was nothing to search, which is not the same
   * as searching it and finding nothing.
   */
  librarySynced: boolean
}

/** One synced Zotero library, with how much of it can be queried. */
export interface BibliographyLibraryStatusRow {
  libraryType: string
  libraryId: string
  name: string
  works: number
  passages: number
}

export interface BibliographyLibraryStatus {
  libraries: BibliographyLibraryStatusRow[]
  /** An embedding generation is active: without it passages cannot be ranked. */
  vectorReady: boolean
}

/** Which libraries are synced into EntropIA and whether passages are searchable. */
export function bibliographyLibraryStatus(): Promise<BibliographyLibraryStatus> {
  return invoke<BibliographyLibraryStatus>('bibliography_library_status')
}

export interface BibliographyPassageContext {
  chunkId: string
  itemId: string
  itemKey: string
  title: string
  /** The chunk's own text. */
  text: string
  /** (page, start, end) in Unicode scalars of that page's text. */
  spans: Array<[number, number, number]>
  pages: Array<{
    pageNumber: number
    text: string
    highlights: Array<[number, number]>
  }>
  /** What the app can show of the original: a PDF file or an HTML snapshot's stored text. */
  originalKind: 'pdf' | 'html' | null
  /**
   * The PDF the viewer was just granted (one file, at runtime). Only set by
   * `bibliographyOpenPassage`; HTML snapshots and the plain context carry none.
   */
  originalPath: string | null
  /** Why the original cannot be shown, when it cannot. */
  openError: string | null
}

/** A passage with its page text and cited ranges; opens nothing. */
export function bibliographyPassageContext(chunkId: string): Promise<BibliographyPassageContext> {
  return invoke<BibliographyPassageContext>('bibliography_passage_context', { chunkId })
}

/**
 * Prepares the passage's original for the in-app viewer: the backend resolves
 * the registered attachment, validates the PDF and allows that one file on the
 * asset protocol. Nothing opens outside the app.
 */
export function bibliographyOpenPassage(chunkId: string): Promise<BibliographyPassageContext> {
  return invoke<BibliographyPassageContext>('bibliography_open_passage', { chunkId })
}

export function bibliographySearchWorks(
  text: string,
  options: {
    topK?: number
    filters?: BibliographySearchFilters
    /** One Zotero library as the Writing tab names it; resolved by the backend. */
    zoteroLibrary?: { libraryType: 'user' | 'group'; libraryId: string }
  } = {}
): Promise<BibliographySearchResponse> {
  return invoke<BibliographySearchResponse>('bibliography_search_works', {
    request: {
      text,
      topK: options.topK ?? 20,
      libraryIds: options.filters?.libraryIds ?? null,
      yearFrom: options.filters?.yearFrom ?? null,
      yearTo: options.filters?.yearTo ?? null,
      itemTypes: options.filters?.itemTypes ?? null,
      tags: options.filters?.tags ?? null,
      zoteroLibraryType: options.zoteroLibrary?.libraryType ?? null,
      zoteroLibraryId: options.zoteroLibrary?.libraryId ?? null,
    },
  })
}

export interface BibliographyPassage {
  chunkId: string
  itemId: string
  itemKey: string
  title: string
  authors: string
  year: number | null
  libraryName: string
  libraryType: string
  libraryNativeId: string
  /** The catalog's CSL-JSON, what a citation snapshots. Empty if unreadable. */
  cslJson: string
  snippet: string
  /** PDF = pages, HTML snapshot = paragraphs; null when the text is gone. */
  location: { kind: 'pages' | 'paragraphs'; from: number; to: number } | null
  score: number
  /**
   * How the passage was found: its text carries the words as typed (`exact`),
   * a close variant of them (`approximate`, `matchTerms` are the variants it
   * holds), or only its vector was near (`meaning`).
   */
  matchKind: PassageMatchKind
  /** The words behind an `exact` or `approximate` match; empty for `meaning`. */
  matchTerms: string[]
}

export type PassageMatchKind = 'exact' | 'approximate' | 'meaning'

export interface BibliographyPassagesResponse {
  passages: BibliographyPassage[]
  /** Why nothing was searched (same codes as the chat's notice), else null. */
  notice: string | null
  /** Short, key-free cause behind `failed` / `embedding_unavailable`. */
  noticeDetail?: string | null
}

/** Passages of the synced libraries for a query (text and meaning; says why if none). */
export function bibliographySearchPassages(
  text: string,
  options: {
    topK?: number
    /** Also match close variants of the words (the shared search preference). */
    fuzzy?: boolean
    /** One Zotero library as the Writing tab names it; resolved by the backend. */
    zoteroLibrary?: { libraryType: 'user' | 'group'; libraryId: string }
  } = {}
): Promise<BibliographyPassagesResponse> {
  return invoke<BibliographyPassagesResponse>('bibliography_search_passages', {
    request: {
      text,
      topK: options.topK ?? 12,
      // Left out when not given: the backend then follows the saved preference
      // and searches every synced library.
      ...(options.fuzzy === undefined ? {} : { fuzzy: options.fuzzy }),
      ...(options.zoteroLibrary
        ? {
            zoteroLibraryType: options.zoteroLibrary.libraryType,
            zoteroLibraryId: options.zoteroLibrary.libraryId,
          }
        : {}),
    },
  })
}
