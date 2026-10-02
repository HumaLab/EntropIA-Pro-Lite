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
}

export function bibliographySearchWorks(
  text: string,
  options: {
    topK?: number
    filters?: BibliographySearchFilters
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
    },
  })
}
