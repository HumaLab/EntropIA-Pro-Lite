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
  openedPath: string | null
  /** Why the original file would not open, when it would not. */
  openError: string | null
}

/** A passage with its page text and cited ranges; opens nothing. */
export function bibliographyPassageContext(chunkId: string): Promise<BibliographyPassageContext> {
  return invoke<BibliographyPassageContext>('bibliography_passage_context', { chunkId })
}

/** Opens the passage's original file in the operating system's viewer. */
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
