import type { DbClient } from '../types'

export interface FtsResult {
  itemId: string
  rank: number
}

export interface FtsSearchDebug {
  rawQuery: string
  sanitizedQuery: string
  strategy: 'empty' | 'strict' | 'relaxed'
  matchCount: number
  resultIds: string[]
}

export interface FtsSearchResponse {
  results: FtsResult[]
  debug: FtsSearchDebug
}

export interface FtsStats {
  totalRows: number
}

interface ItemRowidRow {
  rowid: number
}

// FTS5 operator keywords to strip from user queries
const FTS5_OPERATORS = /\b(AND|OR|NOT|NEAR)\b/g
// Special characters to strip from individual tokens
const FTS5_SPECIAL_CHARS = /[()"\-*^:,./\\]/g

/**
 * Sanitize a raw user query for safe use in FTS5 MATCH expressions.
 * - Removes FTS5 operator keywords (AND, OR, NOT, NEAR)
 * - Strips special characters that are FTS5 operators
 * - Wraps each remaining token in double quotes
 * - Returns empty string for empty/whitespace-only input
 */
export function sanitizeFts5Query(raw: string): string {
  if (!raw.trim()) return ''

  // Remove operator keywords
  const withoutOps = raw.replace(FTS5_OPERATORS, ' ')

  // Split on whitespace, strip special chars from each token, filter empties
  const tokens = withoutOps
    .split(/\s+/)
    .map((token) => token.replace(FTS5_SPECIAL_CHARS, ''))
    .filter((token) => token.length > 0)

  if (tokens.length === 0) return ''

  return tokens.map((t) => `"${t}"`).join(' ')
}

/**
 * A user query compiled once into every branch the search contract needs.
 *
 * The paginated card query cannot call {@link FtsRepo.searchWithDebug}, because
 * that materializes a bounded id list and the whole point of pagination is not
 * to. Compiling the query instead lets the repository apply the same
 * strict -> relaxed OR -> LIKE ordering as a SQL predicate, so the observable
 * search behavior stays identical on both paths.
 */
export interface CardSearchPlan {
  /** Trimmed original input. The LIKE seam matches it as one pattern, exactly
   *  as the non-paginated fallback always has. */
  raw: string
  /** Sanitized strict MATCH expression. Empty when nothing survives sanitizing. */
  strictMatch: string
  /** Quoted OR retry, or null when the query has fewer than two distinct tokens. */
  relaxedMatch: string | null
  /** Distinct sanitized tokens. Empty means there is no FTS branch to try. */
  likeTerms: string[]
}

/**
 * Compile a raw user query into the three-branch search plan.
 * Pure: no database access, so callers can reuse one plan across pages.
 */
export function compileCardSearchQuery(query: string): CardSearchPlan {
  const strictMatch = sanitizeFts5Query(query)
  const terms = extractSanitizedTerms(strictMatch)

  return {
    raw: query.trim(),
    strictMatch,
    relaxedMatch: terms.length > 1 ? terms.map((term) => `"${term}"`).join(' OR ') : null,
    likeTerms: terms,
  }
}

function extractSanitizedTerms(safeQuery: string): string[] {
  const terms = Array.from(safeQuery.matchAll(/"([^"]+)"/g))
    .map((m) => m[1]?.trim() ?? '')
    .filter((t) => t.length > 0)

  return Array.from(new Set(terms))
}

export class FtsRepo {
  constructor(private client: DbClient) {}

  private static readonly REBUILD_INSERT_SQL = `INSERT INTO fts_items(rowid, item_id, title, metadata, extracted_text)
SELECT
  i.rowid,
  i.id,
  i.title,
  COALESCE(i.metadata, ''),
  COALESCE((
    SELECT GROUP_CONCAT(text_part, ' ')
    FROM (
      SELECT text_part
      FROM (
        SELECT COALESCE(e.text_content, '') AS text_part,
               0 AS source_order,
               COALESCE(a.sort_index, 0) AS sort_index,
               e.created_at AS created_at
        FROM extractions e
        JOIN assets a ON a.id = e.asset_id
        WHERE a.item_id = i.id

        UNION ALL

        SELECT COALESCE(t.text_content, '') AS text_part,
               1 AS source_order,
               COALESCE(a.sort_index, 0) AS sort_index,
               t.created_at AS created_at
        FROM transcriptions t
        JOIN assets a ON a.id = t.asset_id
        WHERE a.item_id = i.id
      ) ordered_text
      ORDER BY source_order ASC, sort_index ASC, created_at ASC
    )
  ), '')
FROM items i`

  private async getItemRowid(itemId: string): Promise<number> {
    const rows = await this.client.select<ItemRowidRow>(
      'SELECT rowid FROM items WHERE id = ? LIMIT 1',
      [itemId]
    )

    const rowid = rows[0]?.rowid
    if (rowid === undefined || rowid === null) {
      throw new Error(`Cannot index FTS item: item "${itemId}" does not exist`)
    }

    return rowid
  }

  private mapResults(rows: Array<{ item_id: string; rank: number }>): FtsResult[] {
    return rows.map((row) => ({
      itemId: row.item_id,
      rank: row.rank,
    }))
  }

  private async runMatchQuery(query: string, limit: number) {
    return this.client.select<{ item_id: string; rank: number }>(
      `SELECT i.id AS item_id, bm25(fts_items) AS rank
       FROM fts_items f
       JOIN items i ON i.rowid = f.rowid
       WHERE fts_items MATCH ?
       ORDER BY rank
       LIMIT ?`,
      [query, limit]
    )
  }

  /**
   * Insert or replace an item's indexed fields in fts_items using the
   * canonical identity contract: fts_items.rowid = items.rowid.
   */
  async indexItem(
    itemId: string,
    title: string,
    metadata: string,
    extractedText: string
  ): Promise<void> {
    const rowid = await this.getItemRowid(itemId)

    await this.client.execute(
      `INSERT OR REPLACE INTO fts_items(rowid, item_id, title, metadata, extracted_text) VALUES (?, ?, ?, ?, ?)`,
      [rowid, itemId, title, metadata, extractedText]
    )
  }

  /**
   * Rebuild the full FTS index from canonical item rows.
   *
   * Contentless FTS5 tables cannot be safely mutated with ad-hoc DELETEs by
   * item_id. Rebuilding from `items.rowid` keeps row identity aligned and fixes
   * drift after source-row deletes.
   */
  async rebuildIndex(): Promise<void> {
    await this.client.execute(`INSERT INTO fts_items(fts_items) VALUES ('delete-all')`)
    await this.client.execute(FtsRepo.REBUILD_INSERT_SQL)
  }

  /**
   * Search fts_items using FTS5 MATCH. Returns ranked results.
   * Returns empty array for empty/whitespace query (no DB call).
   */
  async search(query: string, limit = 20): Promise<FtsResult[]> {
    const response = await this.searchWithDebug(query, limit)
    return response.results
  }

  async searchWithDebug(query: string, limit = 20): Promise<FtsSearchResponse> {
    const safeQuery = sanitizeFts5Query(query)
    if (!safeQuery) {
      return {
        results: [],
        debug: {
          rawQuery: query,
          sanitizedQuery: safeQuery,
          strategy: 'empty',
          matchCount: 0,
          resultIds: [],
        },
      }
    }

    const rows = await this.runMatchQuery(safeQuery, limit)

    if (rows.length > 0) {
      const results = this.mapResults(rows)
      return {
        results,
        debug: {
          rawQuery: query,
          sanitizedQuery: safeQuery,
          strategy: 'strict',
          matchCount: results.length,
          resultIds: results.map((row) => row.itemId),
        },
      }
    }

    // Fallback mode: looser query for long inputs (OR over tokens)
    // Example: "Sindicato Obrero de la Industria del Pescado"
    // strict MATCH with all tokens can be too restrictive.
    const terms = extractSanitizedTerms(safeQuery)
    if (terms.length <= 1) {
      return {
        results: [],
        debug: {
          rawQuery: query,
          sanitizedQuery: safeQuery,
          strategy: 'strict',
          matchCount: 0,
          resultIds: [],
        },
      }
    }

    const relaxedQuery = terms.map((t) => `"${t}"`).join(' OR ')
    const relaxedRows = await this.runMatchQuery(relaxedQuery, limit)
    const results = this.mapResults(relaxedRows)

    return {
      results,
      debug: {
        rawQuery: query,
        sanitizedQuery: safeQuery,
        strategy: 'relaxed',
        matchCount: results.length,
        resultIds: results.map((row) => row.itemId),
      },
    }
  }

  /**
   * Whether the item currently has a row in the index.
   *
   * The UI reads this to tell "never indexed" from "indexed, not running now";
   * job status alone cannot, since it resets to idle on every reload.
   *
   * Two conditions, because neither is sufficient on its own:
   *
   * - A row in fts_items, probed by rowid. fts_items is contentless, so its
   *   `item_id` column reads back as NULL and filtering on it never matches;
   *   rowid is the same identity contract `indexItem` writes under.
   * - Text to have indexed. Migration 0004 seeds a row for every item that
   *   existed at the time, so membership alone also reports documents whose
   *   indexed body is empty.
   *
   * The text condition mirrors the sources REBUILD_INSERT_SQL concatenates.
   */
  async isItemIndexed(itemId: string): Promise<boolean> {
    const rows = await this.client.select<{ indexed: number }>(
      `SELECT COUNT(*) AS indexed
       FROM fts_items
       WHERE rowid = (SELECT rowid FROM items WHERE id = ?)
         AND EXISTS (
           SELECT 1
           FROM extractions e
           JOIN assets a ON a.id = e.asset_id
           WHERE a.item_id = ? AND TRIM(COALESCE(e.text_content, '')) <> ''

           UNION ALL

           SELECT 1
           FROM transcriptions t
           JOIN assets a ON a.id = t.asset_id
           WHERE a.item_id = ? AND TRIM(COALESCE(t.text_content, '')) <> ''
         )`,
      [itemId, itemId, itemId]
    )

    return (rows[0]?.indexed ?? 0) > 0
  }

  async stats(): Promise<FtsStats> {
    const rows = await this.client.select<{ total_rows: number }>(
      'SELECT COUNT(*) AS total_rows FROM fts_items'
    )

    return {
      totalRows: rows[0]?.total_rows ?? 0,
    }
  }

  /**
   * Remove an item's row from fts_items by rebuilding from canonical sources.
   *
   * Call this after the source item row has been deleted.
   */
  async removeItem(_itemId: string): Promise<void> {
    await this.rebuildIndex()
  }
}
