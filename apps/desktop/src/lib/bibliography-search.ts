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
