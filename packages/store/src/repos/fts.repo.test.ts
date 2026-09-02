import { describe, it, expect, beforeEach } from 'vitest'
import { sanitizeFts5Query, compileCardSearchQuery, FtsRepo } from './fts.repo'
import type { DbClient } from '../types'

// ============================================================================
// sanitizeFts5Query — pure function tests (no mocks needed)
// ============================================================================
describe('sanitizeFts5Query', () => {
  it('returns empty string for empty input', () => {
    expect(sanitizeFts5Query('')).toBe('')
  })

  it('returns empty string for whitespace-only input', () => {
    expect(sanitizeFts5Query('   ')).toBe('')
  })

  it('wraps single word in quotes', () => {
    expect(sanitizeFts5Query('cabildo')).toBe('"cabildo"')
  })

  it('wraps multiple words in individual quotes', () => {
    expect(sanitizeFts5Query('acta cabildo')).toBe('"acta" "cabildo"')
  })

  it('strips AND operator keyword', () => {
    const result = sanitizeFts5Query('acta AND cabildo')
    expect(result).toBe('"acta" "cabildo"')
  })

  it('strips OR operator keyword', () => {
    const result = sanitizeFts5Query('acta OR cabildo')
    expect(result).toBe('"acta" "cabildo"')
  })

  it('strips special chars: parentheses, asterisks, dashes', () => {
    const result = sanitizeFts5Query('(acta) -cabildo*')
    // special chars stripped from tokens, then each valid token quoted
    expect(result).not.toContain('(')
    expect(result).not.toContain(')')
    expect(result).not.toContain('*')
    expect(result).not.toContain('-')
  })

  it('handles mixed operators and plain text', () => {
    const result = sanitizeFts5Query('acta AND (cabildo OR gobernador)')
    // Operators stripped, parentheses stripped, valid tokens quoted
    expect(result).toContain('"acta"')
    expect(result).toContain('"cabildo"')
    expect(result).toContain('"gobernador"')
  })

  it('handles three-word phrase with no special chars', () => {
    expect(sanitizeFts5Query('real audiencia provincial')).toBe('"real" "audiencia" "provincial"')
  })

  it('strips colons, commas, and dots from tokens', () => {
    const result = sanitizeFts5Query('acta: cabildo, 1810.')
    expect(result).not.toContain(':')
    expect(result).not.toContain(',')
    expect(result).not.toContain('.')
    expect(result).toContain('"acta"')
    expect(result).toContain('"1810"')
  })

  it('handles NOT keyword — strips it', () => {
    const result = sanitizeFts5Query('cabildo NOT gobernador')
    expect(result).not.toContain('NOT')
    expect(result).toContain('"cabildo"')
    expect(result).toContain('"gobernador"')
  })

  it('handles NEAR keyword — strips it', () => {
    const result = sanitizeFts5Query('cabildo NEAR gobernador')
    expect(result).not.toContain('NEAR')
  })
})

// ============================================================================
// FtsRepo — uses DbClient (raw SQL)
// ============================================================================
function createMockDbClient(): DbClient & {
  _executedSql: string[]
  _selectResults: unknown[]
  _selectCalls: Array<{ sql: string; params?: unknown[] }>
  _selectResultsQueue: unknown[][]
} {
  const executedSql: string[] = []
  const selectCalls: Array<{ sql: string; params?: unknown[] }> = []
  let selectResults: unknown[] = []
  let selectResultsQueue: unknown[][] = []

  return {
    _executedSql: executedSql,
    get _selectResults() {
      return selectResults
    },
    set _selectResults(v: unknown[]) {
      selectResults = v
    },
    get _selectCalls() {
      return selectCalls
    },
    get _selectResultsQueue() {
      return selectResultsQueue
    },
    set _selectResultsQueue(v: unknown[][]) {
      selectResultsQueue = v
    },

    async execute(sql: string, _params?: unknown[]) {
      executedSql.push(sql)
      return { rowsAffected: 1 }
    },

    async select<T>(sql: string, params?: unknown[]): Promise<T[]> {
      executedSql.push(sql)
      selectCalls.push({ sql, params })
      if (selectResultsQueue.length > 0) {
        return (selectResultsQueue.shift() ?? []) as T[]
      }
      return selectResults as T[]
    },

    async selectRows(sql: string, params?: unknown[]): Promise<unknown[][]> {
      const rows = await this.select<Record<string, unknown>>(sql, params)
      return rows.map((row) => Object.values(row))
    },

    async executeBatch(_sql: string): Promise<void> {
      // No-op for unit tests
    },
  }
}

describe('FtsRepo', () => {
  let client: ReturnType<typeof createMockDbClient>
  let repo: FtsRepo

  beforeEach(() => {
    client = createMockDbClient()
    repo = new FtsRepo(client)
  })

  describe('isItemIndexed', () => {
    it('probes by rowid, never by the contentless item_id column', async () => {
      client._selectResults = [{ indexed: 1 }]

      await repo.isItemIndexed('item-7')

      const call = client._selectCalls.find(({ sql }) => sql.includes('FROM fts_items'))
      // fts_items se declara con content='': sus columnas se leen como NULL,
      // así que filtrar por item_id no coincide con nada. La identidad del
      // índice es fts_items.rowid = items.rowid, igual que en indexItem.
      expect(call?.sql).toContain('WHERE rowid = (SELECT rowid FROM items WHERE id = ?)')
      // Ningún `item_id` sin calificar: los que aparecen son `a.item_id`, de
      // la tabla assets. La columna de fts_items no se toca nunca.
      expect(call?.sql.match(/(?<![.\w])item_id/g) ?? []).toEqual([])
    })

    it('also requires text to have been indexed, from the same sources as the rebuild', async () => {
      client._selectResults = [{ indexed: 1 }]

      await repo.isItemIndexed('item-7')

      const call = client._selectCalls.find(({ sql }) => sql.includes('FROM fts_items'))
      // La migración 0004 siembra una fila por cada item existente, así que la
      // sola pertenencia también daría por indexado un documento sin cuerpo.
      expect(call?.sql).toContain('FROM extractions e')
      expect(call?.sql).toContain('FROM transcriptions t')
      expect(call?.sql).toContain("TRIM(COALESCE(e.text_content, '')) <> ''")
      expect(call?.params).toEqual(['item-7', 'item-7', 'item-7'])
    })

    it('reports true only when the item has a row in the index', async () => {
      client._selectResults = [{ indexed: 2 }]
      expect(await repo.isItemIndexed('item-1')).toBe(true)

      client._selectResults = [{ indexed: 0 }]
      expect(await repo.isItemIndexed('item-1')).toBe(false)
    })

    it('reports false when the query returns no row at all', async () => {
      client._selectResults = []
      expect(await repo.isItemIndexed('item-1')).toBe(false)
    })
  })

  describe('indexItem', () => {
    it('executes INSERT OR REPLACE into fts_items with explicit rowid', async () => {
      client._selectResults = [{ rowid: 42 }]

      await repo.indexItem('item-1', 'Acta del Cabildo', '{}', 'extracted text content')
      const insertSql = client._executedSql.find(
        (sql) =>
          sql.includes('INSERT OR REPLACE INTO fts_items(rowid, item_id, title, metadata, extracted_text)')
      )
      expect(insertSql).toBeDefined()
    })

    it('looks up the source item rowid before indexing', async () => {
      client._selectResults = [{ rowid: 7 }]

      await repo.indexItem('item-42', 'Title', '', '')
      const rowidLookupSql = client._executedSql.find(
        (sql) => sql.includes('SELECT rowid FROM items WHERE id = ? LIMIT 1')
      )
      expect(rowidLookupSql).toBeDefined()
    })

    it('throws when the source item row does not exist', async () => {
      client._selectResults = []

      await expect(repo.indexItem('missing-item', 'Title', '', '')).rejects.toThrow(
        'Cannot index FTS item: item "missing-item" does not exist'
      )
    })
  })

  describe('search', () => {
    it('returns empty array when no results found', async () => {
      client._selectResults = []
      const results = await repo.search('nonexistent term')
      expect(results).toEqual([])
    })

    it('returns FtsResult array when matches found', async () => {
      client._selectResults = [
        { item_id: 'item-1', rank: -0.5 },
        { item_id: 'item-2', rank: -1.2 },
      ]

      const results = await repo.search('cabildo')
      expect(results).toHaveLength(2)
      expect(results[0]!.itemId).toBe('item-1')
      expect(results[1]!.itemId).toBe('item-2')
    })

    it('returns empty array for empty query without executing SELECT', async () => {
      const initialCount = client._executedSql.length
      const results = await repo.search('')
      expect(results).toEqual([])
      // Should not have executed any SELECT for empty query
      expect(client._executedSql.length).toBe(initialCount)
    })

    it('uses MATCH in the search SQL', async () => {
      client._selectResults = []
      await repo.search('cabildo')
      const hasMATCH = client._executedSql.some((sql) => sql.includes('MATCH'))
      expect(hasMATCH).toBe(true)
    })

    it('joins items by rowid because fts_items is contentless', async () => {
      client._selectResults = []
      await repo.search('cabildo')
      const joinSql = client._executedSql.find((sql) => sql.includes('JOIN items i ON i.rowid = f.rowid'))
      expect(joinSql).toBeDefined()
      expect(joinSql).toContain('bm25(fts_items)')
    })

    it('falls back to OR query when strict search returns no rows', async () => {
      client._selectResultsQueue = [[], [{ item_id: 'item-10', rank: -1.1 }]]

      const results = await repo.search('Sindicato Obrero de la Industria del Pescado')

      expect(client._selectCalls.length).toBe(2)
      expect(client._selectCalls[1]?.params?.[0]).toContain(' OR ')
      expect(results).toEqual([{ itemId: 'item-10', rank: -1.1 }])
    })

    it('returns debug metadata with strict strategy', async () => {
      client._selectResults = [{ item_id: 'item-1', rank: -0.5 }]

      const response = await repo.searchWithDebug('cabildo')

      expect(response.debug.rawQuery).toBe('cabildo')
      expect(response.debug.sanitizedQuery).toBe('"cabildo"')
      expect(response.debug.strategy).toBe('strict')
      expect(response.debug.matchCount).toBe(1)
      expect(response.debug.resultIds).toEqual(['item-1'])
    })
  })

  describe('stats', () => {
    it('returns total rows from fts_items', async () => {
      client._selectResults = [{ total_rows: 35 }]

      const stats = await repo.stats()

      expect(stats).toEqual({ totalRows: 35 })
    })
  })

  describe('removeItem', () => {
    it('rebuilds the contentless index instead of deleting by item_id', async () => {
      await repo.removeItem('item-1')
      const hasDeleteAll = client._executedSql.some((sql) =>
        sql.includes("INSERT INTO fts_items(fts_items) VALUES ('delete-all')")
      )
      const hasCanonicalInsert = client._executedSql.some((sql) =>
        sql.includes('INSERT INTO fts_items(rowid, item_id, title, metadata, extracted_text)')
      )

      expect(hasDeleteAll).toBe(true)
      expect(hasCanonicalInsert).toBe(true)
      expect(
        client._executedSql.some((sql) => sql.includes('DELETE FROM fts_items WHERE item_id'))
      ).toBe(false)
    })

    it('resolves without error when removing non-existent item', async () => {
      await expect(repo.removeItem('ghost-item')).resolves.toBeUndefined()
    })
  })
})

// ============================================================================
// compileCardSearchQuery — the search plan consumed by the paginated card query
// ============================================================================
describe('compileCardSearchQuery', () => {
  it('preserves strict search, relaxed OR retry, and LIKE fallback ordering', () => {
    const plan = compileCardSearchQuery('red fox')

    expect(plan.strictMatch).toContain('red')
    expect(plan.strictMatch).toContain('fox')
    expect(plan.relaxedMatch).toContain('OR')
    expect(plan.likeTerms).toEqual(['red', 'fox'])
  })

  it('compiles the same strict expression sanitizeFts5Query already produces', () => {
    expect(compileCardSearchQuery('red fox').strictMatch).toBe(sanitizeFts5Query('red fox'))
  })

  it('offers no relaxed retry for a single-token query', () => {
    const plan = compileCardSearchQuery('cabildo')

    expect(plan.strictMatch).toBe('"cabildo"')
    expect(plan.relaxedMatch).toBeNull()
    expect(plan.likeTerms).toEqual(['cabildo'])
  })

  it('returns an empty plan for input that sanitizes away entirely', () => {
    for (const raw of ['', '   ', 'AND OR NOT']) {
      const plan = compileCardSearchQuery(raw)

      expect(plan.strictMatch).toBe('')
      expect(plan.relaxedMatch).toBeNull()
      expect(plan.likeTerms).toEqual([])
    }
  })

  it('strips FTS5 operators and special characters before building the plan', () => {
    const plan = compileCardSearchQuery('acta AND (san-martin)')

    expect(plan.strictMatch).toBe('"acta" "sanmartin"')
    expect(plan.relaxedMatch).toBe('"acta" OR "sanmartin"')
    expect(plan.likeTerms).toEqual(['acta', 'sanmartin'])
  })

  it('deduplicates repeated tokens across every branch of the plan', () => {
    const plan = compileCardSearchQuery('fox fox red')

    expect(plan.relaxedMatch).toBe('"fox" OR "red"')
    expect(plan.likeTerms).toEqual(['fox', 'red'])
  })
})
