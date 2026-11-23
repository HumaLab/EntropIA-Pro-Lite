import { invoke } from '@tauri-apps/api/core'
import type { HighlightSegment } from '$lib/item-view-search'

type ItemSnippet = { itemId: string; segments: HighlightSegment[] }

/**
 * A passage around each result's first match, keyed by item id, read from
 * the same text the search index holds. A result found only by its title or
 * metadata has none. Snippets are an extra: when the call fails, the results
 * still show, without them.
 */
export async function fetchSearchSnippets(
  itemIds: string[],
  terms: string[]
): Promise<Map<string, HighlightSegment[]>> {
  if (itemIds.length === 0 || terms.length === 0) return new Map()
  try {
    const rows = await invoke<ItemSnippet[]>('fts_snippets', { itemIds, terms })
    return new Map(rows.map((row) => [row.itemId, row.segments]))
  } catch {
    return new Map()
  }
}

/** One row of a document's full-text search results. */
export type ItemFtsResult = {
  itemId: string
  title: string
  collectionId: string
  collectionName: string | null
  /** The passage around the first match; null when the text has none. */
  snippet: HighlightSegment[] | null
  /** For an approximate result, the variants it was found as; null otherwise. */
  foundAs: string[] | null
}
