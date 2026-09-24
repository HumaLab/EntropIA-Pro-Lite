import { eq, and, like, or, asc, desc, sql } from 'drizzle-orm'
import type { DrizzleClient, DbClient } from '../types'
import { items, assets, collections, processingTasks } from '../schema'
import { FtsRepo, compileCardSearchQuery, type CardSearchPlan, type FtsResult } from './fts.repo'

export type Item = typeof items.$inferSelect
export type NewItem = typeof items.$inferInsert

export type CollectionItemCardSummary = Item & {
  assetCount: number
  /**
   * The same leaf/viewable assets as `assetCount`, split by media
   * (`assets.type`) so the card chip can word itself as pages, images or
   * audio instead of a media-blind "N assets". Optional so a caller that
   * only has `assetCount` (a fallback path with no per-media breakdown)
   * still satisfies this type; absent means "unknown", not zero.
   */
  pdfPageCount?: number
  imageCount?: number
  audioCount?: number
  primaryAssetId: string | null
  primaryAssetPath: string | null
  primaryAssetType: string | null
  /**
   * The directory this document was imported from, derived by the database
   * from the importer's metadata. `null` for a document that arrived without
   * a source path. It is the group heading, never part of the title.
   */
  sourceDir: string | null
  /**
   * Set when only a close variant of the searched words brought this card in:
   * the variants it holds (see FtsRepo). Absent on an exact match.
   */
  foundAs?: string[]
}

type CollectionItemCardSummaryRow = {
  id: string
  title: string
  collection_id: string
  metadata: string | null
  created_at: number
  updated_at: number
  asset_count: number | null
  pdf_page_count: number | null
  image_count: number | null
  audio_count: number | null
  primary_asset_id: string | null
  primary_asset_path: string | null
  primary_asset_type: string | null
  source_dir: string | null
}

/**
 * Collection-wide statistics for the header stats line:
 * - items: total documents in the collection
 * - assets: total viewable assets (images, PDF pages, audio files, ...) in
 *   the collection
 * - pdfPages / images / audios: the same viewable assets, split by media
 *   (`assets.type`). A split PDF page always carries `type = 'pdf'` — see
 *   `splitPdfIntoPageAssets` — so a viewable asset's own type already tells
 *   the media apart; the container that owns those pages is never viewable,
 *   so it never needs a separate marker. `pdfPages + images + audios ===
 *   assets` (every viewable asset is exactly one of the three).
 * - ocr / embeddings / ner / triples: distinct viewable assets that have
 *   that analysis stage applied. NER and triples can be stored at item level
 *   (asset_id NULL); in that case every asset of the item counts.
 * - stt: distinct viewable assets with a non-empty speech-to-text
 *   transcription — the same per-asset unit as `ocr`.
 * Each counter is independent — one asset may be counted in several stages.
 */
export type CollectionStats = {
  items: number
  assets: number
  ocr: number
  embeddings: number
  ner: number
  triples: number
  pdfPages: number
  images: number
  audios: number
  stt: number
}

type CollectionStatsRow = {
  items_count: number | null
  assets_count: number | null
  ocr_count: number | null
  embed_count: number | null
  ner_count: number | null
  triples_count: number | null
  pdf_pages_count: number | null
  images_count: number | null
  audios_count: number | null
  stt_count: number | null
}

/**
 * Corpus-wide statistics for the home overview's text pipeline: OCR / STT ->
 * Texto -> Embeddings, plus how much OCR/embedding work the processing queue
 * still has open. Every pipeline figure (`ocr`, `ocrUniverse`, `stt`,
 * `sttUniverse`, `text`, `textUniverse`, `embeddings`) is at VIEWABLE-FILE
 * (asset) granularity, not document granularity — a document with two OCRed
 * pages and one still-scanned page counts 2 OCRed and 3 in the OCR universe,
 * not "1 document with OCR" (odd/tasks/home-view.md T7). `collections` and
 * `items` stay document-level: they count collections and documents
 * (`items` rows), never assets.
 *
 * A "viewable file" is a leaf asset — a split PDF page, an image, or an
 * audio file — never the PDF container that owns split pages (a container
 * always has children and is excluded, the same `viewable_assets` filter
 * `getCollectionStats` uses).
 *
 * OCR and STT are each measured against their OWN universe of applicable
 * files, not against every viewable file in the corpus (odd/tasks/home-view.md
 * T3i) — mixing audio files into the OCR denominator, or images/PDF pages
 * into the STT denominator, produced a misleading low percentage for both.
 *
 * - `ocrUniverse`: viewable files that are an IMAGE, or a SCANNED PDF page —
 *   a PDF page asset with no non-empty `method = 'native'` extraction. A PDF
 *   page that already carries a native text layer never needs OCR, so it is
 *   excluded from this universe entirely (it can still count in `text`). A
 *   PDF page not yet processed for native text counts as scanned.
 * - `ocr`: the subset of `ocrUniverse` that has at least one OCR-derived
 *   extraction (`method` other than `'native'`) whose text is non-empty. By
 *   construction `ocr <= ocrUniverse` — OCR text on a file outside the
 *   universe (should it ever occur) is never counted here.
 * - `sttUniverse`: viewable AUDIO files.
 * - `stt`: the subset of `sttUniverse` with at least one non-empty
 *   speech-to-text transcription (the `transcriptions` table). By
 *   construction `stt <= sttUniverse`.
 * - `textUniverse`: every viewable file in the corpus (pages, images and
 *   audios together) — the Texto denominator.
 * - `text`: viewable files with ANY non-empty usable text — the union of
 *   OCR, STT and every other extraction source that exists (native PDF text
 *   layer included), measured over `textUniverse`. By construction
 *   `ocr <= text`, `stt <= text` and `text <= textUniverse`.
 * - `embeddings`: viewable files that have BOTH a vector (`vec_assets`) AND
 *   text — the intersection with `text`, so `embeddings <= text` by
 *   construction. A file can have a queued/embedded vector without ever
 *   having usable text (e.g. embedding was requested before extraction);
 *   that file must never count here.
 *
 * `pendingOcr`/`pendingEmbeddings` count assets with a non-terminal
 * `processing_tasks` row of that kind — the same "active" definition the
 * queue's own `idx_processing_tasks_active_unique` index uses (any state
 * other than `succeeded`, `failed`, `skipped` or `cancelled`).
 */
export type CorpusStats = {
  collections: number
  items: number
  ocr: number
  ocrUniverse: number
  stt: number
  sttUniverse: number
  text: number
  textUniverse: number
  embeddings: number
  pendingOcr: number
  pendingEmbeddings: number
}

type CorpusStatsRow = {
  collections_count: number | null
  items_count: number | null
  ocr_count: number | null
  ocr_universe_count: number | null
  stt_count: number | null
  stt_universe_count: number | null
  text_count: number | null
  text_universe_count: number | null
  embed_count: number | null
  pending_ocr_count: number | null
  pending_embed_count: number | null
}

/**
 * One recently-imported document, for the home overview's "Actividad
 * reciente" panel: distinct from the corpus stats above (which only count),
 * and distinct from Continuar (which resumes collections/writing/research,
 * never individual documents).
 */
export type RecentlyImportedItem = {
  id: string
  title: string
  collectionId: string
  collectionName: string
  createdAt: number
}

type RecentlyImportedItemRow = {
  id: string
  title: string
  collection_id: string
  collection_name: string
  created_at: number
}

/**
 * Position in the collection's ordering.
 *
 * A collection is read one imported directory at a time, so the ordering is
 * `(group, title COLLATE NOCASE, id)` and the cursor has to name the group as
 * well as the row. It carries two facts about the group rather than one:
 * `sourceDir` identifies it, and `groupFirstImport` is where it sorts. The
 * second is what keeps the cursor placeable when the group it named is deleted
 * mid-scroll — without it, a vanished directory would take the reader's
 * position with it.
 *
 * Both are optional so a cursor built by an older caller, or by a view that
 * only knows a title and an id, still resolves.
 */
export type ItemCursor = {
  title: string
  id: string
  sourceDir?: string | null
  groupFirstImport?: string | null
}

/** One keyset page plus the cursor that continues it. */
export type ItemPage = {
  items: CollectionItemCardSummary[]
  nextCursor: ItemCursor | null
  hasMore: boolean
}

const DEFAULT_PAGE_SIZE = 100

const EMPTY_PAGE: ItemPage = { items: [], nextCursor: null, hasMore: false }

// The collation is spelled out on both sides so the comparison matches the
// `title COLLATE NOCASE` term of idx_items_collection_title. Comparing the bare
// column would fall back to BINARY and disagree with the ORDER BY.
const KEYSET_AFTER_SQL = `(
            i.title COLLATE NOCASE > ?
            OR (i.title COLLATE NOCASE = ? AND i.id > ?)
          )`

const KEYSET_BEFORE_SQL = `(
            i.title COLLATE NOCASE < ?
            OR (i.title COLLATE NOCASE = ? AND i.id < ?)
          )`

const FTS_FILTER_SQL = 'i.rowid IN (SELECT f.rowid FROM fts_items f WHERE fts_items MATCH ?)'

/**
 * The card summary projection shared by the full-collection query, the
 * paginated query, and the two sibling queries. Kept in one place so a change
 * to what a card shows cannot drift between the four callers.
 */
const CARD_SUMMARY_SOURCE_SQL = `
        SELECT
          i.id,
          i.title,
          i.collection_id,
          i.metadata,
          i.created_at,
          i.updated_at,
          (SELECT COUNT(*)
             FROM assets leaf
             WHERE leaf.item_id = i.id
               AND NOT EXISTS (
                 SELECT 1 FROM assets child WHERE child.parent_asset_id = leaf.id
               )
          ) AS asset_count,
          (SELECT COUNT(*)
             FROM assets leaf
             WHERE leaf.item_id = i.id
               AND leaf.type = 'pdf'
               AND NOT EXISTS (
                 SELECT 1 FROM assets child WHERE child.parent_asset_id = leaf.id
               )
          ) AS pdf_page_count,
          (SELECT COUNT(*)
             FROM assets leaf
             WHERE leaf.item_id = i.id
               AND leaf.type = 'image'
               AND NOT EXISTS (
                 SELECT 1 FROM assets child WHERE child.parent_asset_id = leaf.id
               )
          ) AS image_count,
          (SELECT COUNT(*)
             FROM assets leaf
             WHERE leaf.item_id = i.id
               AND leaf.type = 'audio'
               AND NOT EXISTS (
                 SELECT 1 FROM assets child WHERE child.parent_asset_id = leaf.id
               )
          ) AS audio_count,
          pa.id AS primary_asset_id,
          pa.path AS primary_asset_path,
          pa.type AS primary_asset_type,
          i.source_dir
        FROM items i
        LEFT JOIN assets pa ON pa.id = (
          SELECT p.id
          FROM assets p
          WHERE p.item_id = i.id AND p.parent_asset_id IS NULL
          ORDER BY
            CASE p.type
              WHEN 'image' THEN 0
              WHEN 'pdf' THEN 1
              ELSE 2
            END,
            p.sort_index ASC,
            p.created_at ASC
          LIMIT 1
        )`

function mapCardSummaryRow(row: CollectionItemCardSummaryRow): CollectionItemCardSummary {
  return {
    id: row.id,
    title: row.title,
    collectionId: row.collection_id,
    metadata: row.metadata,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
    assetCount: Number(row.asset_count ?? 0),
    pdfPageCount: Number(row.pdf_page_count ?? 0),
    imageCount: Number(row.image_count ?? 0),
    audioCount: Number(row.audio_count ?? 0),
    primaryAssetId: row.primary_asset_id,
    primaryAssetPath: row.primary_asset_path,
    primaryAssetType: row.primary_asset_type,
    sourceDir: row.source_dir ?? null,
  }
}

/** One directory group of a collection, and where it sorts. */
type DirectoryGroup = { sourceDir: string | null; firstImport: string | null }

/**
 * Turn an over-fetched row set (limit + 1) into a page. The extra row is the
 * evidence that a next page exists; it is never delivered.
 *
 * `groupOf` supplies the sort position of the last delivered row's group, so
 * the cursor can name where to resume even if that directory is later removed.
 */
function buildPage(
  rows: CollectionItemCardSummary[],
  limit: number,
  groupOf: (sourceDir: string | null) => string | null = () => null
): ItemPage {
  const hasMore = rows.length > limit
  const items = hasMore ? rows.slice(0, limit) : rows
  const last = items[items.length - 1]

  return {
    items,
    hasMore,
    nextCursor:
      hasMore && last
        ? {
            title: last.title,
            id: last.id,
            sourceDir: last.sourceDir,
            groupFirstImport: groupOf(last.sourceDir),
          }
        : null,
  }
}

/**
 * Where a cursor resumes in an ordered group list.
 *
 * The named directory first: that is the ordinary case, and it is exact. When
 * it is gone — the whole directory deleted while someone was reading it — the
 * recorded import time still places it, and reading continues at the first
 * group that now sorts at or after where it used to be. Falling back to the
 * start of the list instead would re-deliver everything already seen.
 */
function resumeIndex(groups: DirectoryGroup[], cursor: ItemCursor): number {
  const named = groups.findIndex((group) => group.sourceDir === (cursor.sourceDir ?? null))
  if (named >= 0) return named

  const from = cursor.groupFirstImport
  if (from == null) return 0

  const after = groups.findIndex((group) => group.firstImport != null && group.firstImport >= from)
  return after >= 0 ? after : groups.length
}

export { compileCardSearchQuery }
export type { CardSearchPlan }

/**
 * Normalize the two accepted search inputs into one plan. An empty or
 * whitespace-only query is not a search at all, so it returns undefined and the
 * page comes back unfiltered rather than empty.
 */
function toSearchPlan(search: string | CardSearchPlan | undefined): CardSearchPlan | undefined {
  if (search === undefined) return undefined
  if (typeof search !== 'string') return search
  return search.trim() ? compileCardSearchQuery(search) : undefined
}

export class ItemRepo {
  private ftsRepo: FtsRepo | null
  private rawClient?: DbClient

  constructor(
    private db: DrizzleClient,
    rawClient?: DbClient
  ) {
    this.rawClient = rawClient
    this.ftsRepo = rawClient ? new FtsRepo(rawClient) : null
  }

  async create(data: Omit<NewItem, 'id' | 'createdAt' | 'updatedAt'>): Promise<Item> {
    const now = Date.now()
    const createdItem: Item = {
      id: crypto.randomUUID(),
      title: data.title,
      collectionId: data.collectionId,
      metadata: data.metadata ?? null,
      createdAt: now,
      updatedAt: now,
    }

    if (this.rawClient) {
      // Validate that the parent collection exists before inserting (FK constraint)
      const collectionExists = await this.rawClient.select(
        'SELECT id FROM collections WHERE id = ?',
        [createdItem.collectionId]
      )
      if (collectionExists.length === 0) {
        throw new Error(
          `Cannot create item: collection "${createdItem.collectionId}" does not exist`
        )
      }

      await this.rawClient.execute(
        'INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)',
        [
          createdItem.id,
          createdItem.title,
          createdItem.collectionId,
          createdItem.metadata,
          createdItem.createdAt,
          createdItem.updatedAt,
        ]
      )
    } else {
      await this.db.insert(items).values(createdItem)
    }

    return createdItem
  }

  async findByCollection(collectionId: string): Promise<Item[]> {
    if (this.rawClient) {
      const rows = await this.rawClient.select<CollectionItemCardSummaryRow>(
        `SELECT id, title, collection_id, metadata, created_at, updated_at
         FROM items
         WHERE collection_id = ?
         ORDER BY title COLLATE NOCASE ASC, id ASC`,
        [collectionId]
      )

      return rows.map((row) => ({
        id: row.id,
        title: row.title,
        collectionId: row.collection_id,
        metadata: row.metadata,
        createdAt: row.created_at,
        updatedAt: row.updated_at,
      }))
    }

    return this.db
      .select()
      .from(items)
      .where(eq(items.collectionId, collectionId))
      .orderBy(asc(items.title), asc(items.id))
  }

  async findCardSummariesByCollection(
    collectionId: string,
    query = ''
  ): Promise<CollectionItemCardSummary[]> {
    if (!this.rawClient) {
      const baseItems = query.trim()
        ? await this.searchByText(collectionId, query)
        : await this.findByCollection(collectionId)

      return baseItems.map((item) => ({
        ...item,
        assetCount: 0,
        primaryAssetId: null,
        primaryAssetPath: null,
        primaryAssetType: null,
        sourceDir: null,
      }))
    }

    const trimmedQuery = query.trim()
    const matchedIds = trimmedQuery
      ? (await this.searchByText(collectionId, trimmedQuery)).map((item) => item.id)
      : []
    if (trimmedQuery && matchedIds.length === 0) return []

    const params: unknown[] = trimmedQuery ? matchedIds : [collectionId]
    const filterSql = trimmedQuery
      ? `i.id IN (${matchedIds.map(() => '?').join(', ')})`
      : 'i.collection_id = ?'

    // This path loads the whole collection at once, so the group's sort
    // position can be a correlated subquery rather than the resolved group list
    // the paged path needs — there is no index to protect here.
    const rows = await this.rawClient.select<CollectionItemCardSummaryRow>(
      `${CARD_SUMMARY_SOURCE_SQL}
        WHERE ${filterSql}
        ORDER BY
          (SELECT MIN(g.imported_at)
             FROM items g
            WHERE g.collection_id = i.collection_id
              AND g.source_dir IS i.source_dir) IS NULL ASC,
          (SELECT MIN(g.imported_at)
             FROM items g
            WHERE g.collection_id = i.collection_id
              AND g.source_dir IS i.source_dir) ASC,
          i.source_dir ASC,
          i.title COLLATE NOCASE ASC,
          i.id ASC
      `,
      params
    )

    return rows.map(mapCardSummaryRow)
  }

  /**
   * One page of collection card summaries, addressed by a keyset cursor.
   *
   * The cursor is the `(title, id)` pair of the last row already delivered, not
   * a row offset. That is a correctness requirement rather than an
   * optimization: with OFFSET, deleting a document mid-scroll shifts every
   * later page and silently skips a row, and inserting one produces a
   * duplicate. A data-valued cursor is stable under both, and it costs the
   * same at page 1 and page 100.
   *
   * `search` is a plan compiled once by {@link compileCardSearchQuery}. The
   * strict -> relaxed OR -> LIKE ordering is resolved per call against the
   * whole collection rather than against the current page, so every page of one
   * search resolves to the same branch and pagination stays coherent.
   */
  async findCardSummariesPage(
    collectionId: string,
    options: {
      cursor?: ItemCursor | null
      limit?: number
      /** A raw user query, or a plan already compiled by
       *  {@link compileCardSearchQuery}. Accepting the raw string keeps FTS5
       *  syntax out of the callers: no view should know what a MATCH is. */
      search?: string | CardSearchPlan
    } = {}
  ): Promise<ItemPage> {
    const limit = Math.max(1, options.limit ?? DEFAULT_PAGE_SIZE)
    const cursor = options.cursor ?? null
    const search = toSearchPlan(options.search)

    if (!this.rawClient) {
      return this.findCardSummariesPageWithDrizzle(collectionId, cursor, limit, search)
    }

    const conditions = ['i.collection_id = ?']
    const params: unknown[] = [collectionId]

    let widened = false
    if (search) {
      const filter = await this.resolveSearchFilter(collectionId, search)
      if (filter === null) return EMPTY_PAGE
      conditions.push(filter.sql)
      params.push(...filter.params)
      widened = filter.fuzzy === true
    }

    const groups = await this.resolveDirectoryGroups(collectionId, conditions, params)
    if (groups.length === 0) return EMPTY_PAGE

    const start = cursor ? resumeIndex(groups, cursor) : 0

    // One row beyond the page is the evidence that a next page exists, without
    // a second COUNT over the whole collection.
    const wanted = limit + 1
    const collected: CollectionItemCardSummary[] = []

    // A page ends where the limit does, not where a directory does: a group
    // smaller than the page is topped up from the next one, so the reader never
    // gets a short page just because a legajo held four scans.
    for (let index = start; index < groups.length && collected.length < wanted; index += 1) {
      const group = groups[index]
      if (!group) break

      const rows = await this.selectGroupRows(
        group.sourceDir,
        conditions,
        params,
        // Only the group the cursor stopped inside resumes mid-way. Every group
        // after it starts at its first document.
        index === start ? cursor : null,
        wanted - collected.length
      )
      collected.push(...rows.map(mapCardSummaryRow))
    }

    if (widened && search) await this.markApproximate(collected, search)

    const firstImportOf = new Map(groups.map((group) => [group.sourceDir, group.firstImport]))
    return buildPage(collected, limit, (sourceDir) => firstImportOf.get(sourceDir) ?? null)
  }

  /**
   * A collection's directory groups, oldest import first.
   *
   * This is an aggregate, so it cannot live on the rows themselves — and that
   * is the point. Materialising each group's import time onto every item would
   * mean writing it on every insert path, including the one the sync engine
   * owns, and a column the sync path forgets is a divergence waiting to happen.
   * A collection holds a handful of directories (twenty in the largest real
   * archive), so resolving them once per page request is cheaper than carrying
   * that risk.
   *
   * Directories with no import time sort last: a document that arrived without
   * a source path has no claim on a position among the ones that did.
   */
  private async resolveDirectoryGroups(
    collectionId: string,
    conditions: string[],
    params: unknown[]
  ): Promise<DirectoryGroup[]> {
    const rows = await this.rawClient!.select<{
      source_dir: string | null
      first_import: string | null
    }>(
      `SELECT i.source_dir AS source_dir, MIN(i.imported_at) AS first_import
         FROM items i
        WHERE ${conditions.join('\n          AND ')}
        GROUP BY i.source_dir
        ORDER BY first_import IS NULL ASC, first_import ASC, i.source_dir ASC
      `,
      params
    )

    return rows.map((row) => ({
      sourceDir: row.source_dir ?? null,
      firstImport: row.first_import ?? null,
    }))
  }

  /** One group's slice of a page, in document-name order. */
  private async selectGroupRows(
    sourceDir: string | null,
    conditions: string[],
    baseParams: unknown[],
    cursor: ItemCursor | null,
    limit: number
  ): Promise<CollectionItemCardSummaryRow[]> {
    // `IS` rather than `=` so the group of documents without a source path is
    // selectable like any other.
    const groupConditions = [...conditions, 'i.source_dir IS ?']
    const params = [...baseParams, sourceDir]

    if (cursor) {
      groupConditions.push(KEYSET_AFTER_SQL)
      params.push(cursor.title, cursor.title, cursor.id)
    }
    params.push(limit)

    return this.rawClient!.select<CollectionItemCardSummaryRow>(
      `${CARD_SUMMARY_SOURCE_SQL}
        WHERE ${groupConditions.join('\n          AND ')}
        ORDER BY i.title COLLATE NOCASE ASC, i.id ASC
        LIMIT ?
      `,
      params
    )
  }

  /**
   * The card summary immediately before `cursor` in collection order.
   * One indexed row, never the whole collection.
   */
  async findPreviousCardSummary(
    collectionId: string,
    cursor: ItemCursor
  ): Promise<CollectionItemCardSummary | null> {
    return this.findSiblingCardSummary(collectionId, cursor, 'previous')
  }

  /** The card summary immediately after `cursor` in collection order. */
  async findNextCardSummary(
    collectionId: string,
    cursor: ItemCursor
  ): Promise<CollectionItemCardSummary | null> {
    return this.findSiblingCardSummary(collectionId, cursor, 'next')
  }

  private async findSiblingCardSummary(
    collectionId: string,
    cursor: ItemCursor,
    direction: 'previous' | 'next'
  ): Promise<CollectionItemCardSummary | null> {
    const forward = direction === 'next'
    const keyset = forward ? KEYSET_AFTER_SQL : KEYSET_BEFORE_SQL
    const order = forward
      ? 'i.title COLLATE NOCASE ASC, i.id ASC'
      : 'i.title COLLATE NOCASE DESC, i.id DESC'

    if (!this.rawClient) {
      const all = await this.findCardSummariesByCollection(collectionId)
      const index = all.findIndex((row) => row.id === cursor.id)
      if (index < 0) return null
      return all[forward ? index + 1 : index - 1] ?? null
    }

    const rows = await this.rawClient.select<CollectionItemCardSummaryRow>(
      `${CARD_SUMMARY_SOURCE_SQL}
        WHERE i.collection_id = ?
          AND ${keyset}
        ORDER BY ${order}
        LIMIT 1
      `,
      [collectionId, cursor.title, cursor.title, cursor.id]
    )

    const row = rows[0]
    return row ? mapCardSummaryRow(row) : null
  }

  /**
   * Resolve one search plan to a SQL predicate, preserving the existing
   * strict -> relaxed OR -> LIKE ordering.
   *
   * Returns `null` when the plan can never match anything (the query sanitized
   * away entirely), so the caller can answer with an empty page without a
   * round trip.
   *
   * The FTS branches filter with `i.rowid IN (SELECT ... MATCH ?)` rather than
   * an expanded `i.id IN (?, ?, ...)`. Expanding one placeholder per matched id
   * is what forced the old hardcoded limit of 50: raising it would eventually
   * blow past SQLITE_MAX_VARIABLE_NUMBER.
   */
  private async resolveSearchFilter(
    collectionId: string,
    plan: CardSearchPlan
  ): Promise<{ sql: string; params: unknown[]; fuzzy?: boolean } | null> {
    if (!plan.strictMatch && !plan.raw) return null

    const likeFilter = {
      sql: '(i.title LIKE ? OR i.metadata LIKE ?)',
      params: [`%${plan.raw}%`, `%${plan.raw}%`],
    }

    if (!plan.strictMatch) return likeFilter

    // The approximate match holds the strict one, so when it is present it is
    // tried first: exact documents and misread ones, filtered together.
    if (plan.fuzzyMatch && (await this.ftsMatchesAnything(collectionId, plan.fuzzyMatch))) {
      return { sql: FTS_FILTER_SQL, params: [plan.fuzzyMatch], fuzzy: true }
    }

    if (await this.ftsMatchesAnything(collectionId, plan.strictMatch)) {
      return { sql: FTS_FILTER_SQL, params: [plan.strictMatch] }
    }

    if (plan.relaxedMatch && (await this.ftsMatchesAnything(collectionId, plan.relaxedMatch))) {
      return { sql: FTS_FILTER_SQL, params: [plan.relaxedMatch] }
    }

    return likeFilter
  }

  /**
   * Marks the cards on this page that only a variant brought in, with the
   * variants each one holds — the label the card shows.
   *
   * Asked per page rather than computed in the page query: the page is at most
   * a few dozen rows, and the alternative is threading a second MATCH into a
   * SELECT that already carries the group and keyset conditions.
   */
  private async markApproximate(
    rows: CollectionItemCardSummary[],
    plan: CardSearchPlan
  ): Promise<void> {
    const variants = plan.variants ?? []
    if (!this.rawClient || !this.ftsRepo || rows.length === 0 || variants.length === 0) return

    try {
      const placeholders = rows.map(() => '?').join(', ')
      const exact = await this.rawClient.select<{ item_id: string }>(
        `SELECT i.id AS item_id
           FROM fts_items f
           JOIN items i ON i.rowid = f.rowid
          WHERE fts_items MATCH ? AND i.id IN (${placeholders})`,
        [plan.strictMatch, ...rows.map((row) => row.id)]
      )
      const found = new Set(exact.map((row) => row.item_id))
      const approximate = rows.filter((row) => !found.has(row.id))
      if (approximate.length === 0) return

      const held = await this.ftsRepo.variantsHeldBy(
        approximate.map((row) => row.id),
        variants
      )
      for (const row of approximate) row.foundAs = held.get(row.id) ?? []
    } catch {
      // The label is an extra. Without it the cards are the same cards.
    }
  }

  /**
   * Whether one MATCH expression hits anything in this collection at all.
   *
   * Deliberately cursor-independent: the branch a search resolves to must not
   * change between page 1 and page 5, or the cursor would be walking a
   * different result set than the one it came from.
   */
  private async ftsMatchesAnything(collectionId: string, match: string): Promise<boolean> {
    if (!this.rawClient) return false

    try {
      const rows = await this.rawClient.select(
        `SELECT 1 AS hit
           FROM fts_items f
           JOIN items i ON i.rowid = f.rowid
          WHERE fts_items MATCH ?
            AND i.collection_id = ?
          LIMIT 1`,
        [match, collectionId]
      )
      return rows.length > 0
    } catch {
      // fts_items may be missing or corrupt; the LIKE seam still answers.
      return false
    }
  }

  /**
   * Drizzle-only page path, for the same no-raw-client mode the rest of this
   * repository already supports. Asset columns are not available here, exactly
   * as in {@link findCardSummariesByCollection}'s fallback.
   */
  private async findCardSummariesPageWithDrizzle(
    collectionId: string,
    cursor: ItemCursor | null,
    limit: number,
    search?: CardSearchPlan
  ): Promise<ItemPage> {
    const base = search?.raw
      ? await this.searchByText(collectionId, search.raw)
      : await this.findByCollection(collectionId)

    const after = cursor
      ? base.filter((row) => {
          const byTitle = row.title.localeCompare(cursor.title, undefined, {
            sensitivity: 'accent',
          })
          return byTitle > 0 || (byTitle === 0 && row.id > cursor.id)
        })
      : base

    return buildPage(
      after.slice(0, limit + 1).map((item) => ({
        ...item,
        assetCount: 0,
        primaryAssetId: null,
        primaryAssetPath: null,
        primaryAssetType: null,
        // The Drizzle path does not read the generated column, so it cannot
        // group. It is the fallback for a client without raw SQL, not the path
        // the app takes.
        sourceDir: null,
      })),
      limit
    )
  }

  /**
   * Collection-wide statistics for the header stats line:
   * - items: total documents in the collection
   * - assets: viewable assets — leaf rows only, i.e. excluding parent
   *   containers that own page children (matches the item-card asset counts)
   * - pdfPages / images / audios: the same viewable assets split by media
   *   (`va.type`); a split PDF page is always stored as `type = 'pdf'`
   *   (see `splitPdfIntoPageAssets`), so the type column alone tells the
   *   media apart once containers are excluded
   * - ocr: distinct viewable assets with at least one extraction
   * - stt: distinct viewable assets with a non-empty transcription (same
   *   per-asset unit as `ocr`)
   * - embeddings: distinct viewable assets with a row in vec_assets
   * - ner: distinct viewable assets with a direct entity, or whose item has
   *   item-level entities (asset_id NULL)
   * - triples: distinct viewable assets with a direct triple, or whose item
   *   has item-level triples (asset_id NULL)
   */
  async getCollectionStats(collectionId: string): Promise<CollectionStats> {
    if (this.rawClient) {
      const rows = await this.rawClient.select<CollectionStatsRow>(
        `
          WITH viewable_assets AS (
            SELECT a.id, a.item_id, a.type AS type
              FROM assets a
             WHERE NOT EXISTS (
               SELECT 1 FROM assets child WHERE child.parent_asset_id = a.id
             )
          )
          SELECT
            (SELECT COUNT(*)
               FROM items i
              WHERE i.collection_id = ?
            ) AS items_count,
            (SELECT COUNT(*)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
            ) AS assets_count,
            (SELECT COUNT(DISTINCT va.id)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND EXISTS (
                  SELECT 1 FROM extractions e WHERE e.asset_id = va.id
                )
            ) AS ocr_count,
            (SELECT COUNT(DISTINCT va.id)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND EXISTS (
                  SELECT 1 FROM vec_assets v WHERE v.asset_id = va.id
                )
            ) AS embed_count,
            (SELECT COUNT(DISTINCT va.id)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND (
                  EXISTS (SELECT 1 FROM entities en WHERE en.asset_id = va.id)
                  OR EXISTS (
                    SELECT 1 FROM entities en
                    WHERE en.item_id = i.id AND en.asset_id IS NULL
                  )
                )
            ) AS ner_count,
            (SELECT COUNT(DISTINCT va.id)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND (
                  EXISTS (SELECT 1 FROM triples tr WHERE tr.asset_id = va.id)
                  OR EXISTS (
                    SELECT 1 FROM triples tr
                    WHERE tr.item_id = i.id AND tr.asset_id IS NULL
                  )
                )
            ) AS triples_count,
            (SELECT COUNT(*)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND va.type = 'pdf'
            ) AS pdf_pages_count,
            (SELECT COUNT(*)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND va.type = 'image'
            ) AS images_count,
            (SELECT COUNT(*)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND va.type = 'audio'
            ) AS audios_count,
            (SELECT COUNT(DISTINCT va.id)
               FROM viewable_assets va
               JOIN items i ON i.id = va.item_id
              WHERE i.collection_id = ?
                AND EXISTS (
                  SELECT 1 FROM transcriptions t
                   WHERE t.asset_id = va.id
                     AND t.text_content IS NOT NULL AND TRIM(t.text_content) <> ''
                )
            ) AS stt_count
        `,
        [
          collectionId,
          collectionId,
          collectionId,
          collectionId,
          collectionId,
          collectionId,
          collectionId,
          collectionId,
          collectionId,
          collectionId,
        ]
      )

      const row = rows[0] ?? ({} as CollectionStatsRow)
      return {
        items: Number(row.items_count ?? 0),
        assets: Number(row.assets_count ?? 0),
        ocr: Number(row.ocr_count ?? 0),
        embeddings: Number(row.embed_count ?? 0),
        ner: Number(row.ner_count ?? 0),
        triples: Number(row.triples_count ?? 0),
        pdfPages: Number(row.pdf_pages_count ?? 0),
        images: Number(row.images_count ?? 0),
        audios: Number(row.audios_count ?? 0),
        stt: Number(row.stt_count ?? 0),
      }
    }

    // Drizzle fallback (no raw client): one aggregate per statistic.
    // Leaf filter mirrors the raw query: assets that own page children are
    // parent containers and never count.
    const leafFilter = sql`NOT EXISTS (
      SELECT 1 FROM assets child WHERE child.parent_asset_id = ${assets.id}
    )`
    const [
      itemsRows,
      assetsRows,
      ocrRows,
      embedRows,
      nerRows,
      triplesRows,
      pdfPagesRows,
      imagesRows,
      audiosRows,
      sttRows,
    ] = await Promise.all([
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(items)
        .where(eq(items.collectionId, collectionId)),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(and(eq(items.collectionId, collectionId), leafFilter)),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(
          and(
            eq(items.collectionId, collectionId),
            leafFilter,
            sql`EXISTS (SELECT 1 FROM extractions e WHERE e.asset_id = ${assets.id})`
          )
        ),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(
          and(
            eq(items.collectionId, collectionId),
            leafFilter,
            sql`EXISTS (SELECT 1 FROM vec_assets v WHERE v.asset_id = ${assets.id})`
          )
        ),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(
          and(
            eq(items.collectionId, collectionId),
            leafFilter,
            sql`(
              EXISTS (SELECT 1 FROM entities en WHERE en.asset_id = ${assets.id})
              OR EXISTS (
                SELECT 1 FROM entities en
                WHERE en.item_id = ${items.id} AND en.asset_id IS NULL
              )
            )`
          )
        ),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(
          and(
            eq(items.collectionId, collectionId),
            leafFilter,
            sql`(
              EXISTS (SELECT 1 FROM triples tr WHERE tr.asset_id = ${assets.id})
              OR EXISTS (
                SELECT 1 FROM triples tr
                WHERE tr.item_id = ${items.id} AND tr.asset_id IS NULL
              )
            )`
          )
        ),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(and(eq(items.collectionId, collectionId), leafFilter, eq(assets.type, 'pdf'))),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(and(eq(items.collectionId, collectionId), leafFilter, eq(assets.type, 'image'))),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(and(eq(items.collectionId, collectionId), leafFilter, eq(assets.type, 'audio'))),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .innerJoin(items, eq(assets.itemId, items.id))
        .where(
          and(
            eq(items.collectionId, collectionId),
            leafFilter,
            sql`EXISTS (
              SELECT 1 FROM transcriptions t
               WHERE t.asset_id = ${assets.id}
                 AND t.text_content IS NOT NULL AND TRIM(t.text_content) <> ''
            )`
          )
        ),
    ])

    return {
      items: Number(itemsRows[0]?.count ?? 0),
      assets: Number(assetsRows[0]?.count ?? 0),
      ocr: Number(ocrRows[0]?.count ?? 0),
      embeddings: Number(embedRows[0]?.count ?? 0),
      ner: Number(nerRows[0]?.count ?? 0),
      triples: Number(triplesRows[0]?.count ?? 0),
      pdfPages: Number(pdfPagesRows[0]?.count ?? 0),
      images: Number(imagesRows[0]?.count ?? 0),
      audios: Number(audiosRows[0]?.count ?? 0),
      stt: Number(sttRows[0]?.count ?? 0),
    }
  }

  async getCorpusStats(): Promise<CorpusStats> {
    if (this.rawClient) {
      const rows = await this.rawClient.select<CorpusStatsRow>(`
          WITH viewable_assets AS (
            SELECT a.id, a.item_id, a.type AS type
              FROM assets a
             WHERE NOT EXISTS (
               SELECT 1 FROM assets child WHERE child.parent_asset_id = a.id
             )
          ),
          -- OCR universe: a viewable IMAGE, or a viewable PDF page with no
          -- non-empty native text layer (a "scanned" page). A page not yet
          -- checked for a native layer counts as scanned. No semicolons in
          -- this statement, comments included: db_select rejects any.
          ocr_universe_assets AS (
            SELECT va.id
              FROM viewable_assets va
             WHERE va.type = 'image'
                OR (
                  va.type = 'pdf'
                  AND NOT EXISTS (
                    SELECT 1 FROM extractions e
                     WHERE e.asset_id = va.id
                       AND e.method = 'native'
                       AND e.text_content IS NOT NULL AND TRIM(e.text_content) <> ''
                  )
                )
          ),
          -- OCR numerator: the subset of ocr_universe_assets with a non-empty
          -- OCR-derived extraction (method other than 'native').
          asset_ocr AS (
            SELECT DISTINCT oua.id
              FROM ocr_universe_assets oua
              JOIN extractions e ON e.asset_id = oua.id
             WHERE e.method <> 'native'
               AND e.text_content IS NOT NULL AND TRIM(e.text_content) <> ''
          ),
          -- STT universe: a viewable AUDIO asset.
          stt_universe_assets AS (
            SELECT id FROM viewable_assets WHERE type = 'audio'
          ),
          asset_stt AS (
            SELECT DISTINCT su.id
              FROM stt_universe_assets su
              JOIN transcriptions t ON t.asset_id = su.id
             WHERE t.text_content IS NOT NULL AND TRIM(t.text_content) <> ''
          ),
          -- Texto numerator: any viewable file (page, image or audio) with
          -- ANY non-empty text, native extraction or OCR extraction or
          -- transcription alike.
          asset_text AS (
            SELECT va.id
              FROM viewable_assets va
             WHERE EXISTS (
                     SELECT 1 FROM extractions e
                      WHERE e.asset_id = va.id
                        AND e.text_content IS NOT NULL AND TRIM(e.text_content) <> ''
                   )
                OR EXISTS (
                     SELECT 1 FROM transcriptions t
                      WHERE t.asset_id = va.id
                        AND t.text_content IS NOT NULL AND TRIM(t.text_content) <> ''
                   )
          ),
          -- Embeddings numerator: a viewable file with text AND a vec_assets
          -- row (a vector without text must never count).
          asset_embeddings AS (
            SELECT DISTINCT va.id
              FROM viewable_assets va
              JOIN vec_assets v ON v.asset_id = va.id
             WHERE va.id IN (SELECT id FROM asset_text)
          )
          SELECT
            (SELECT COUNT(*) FROM collections) AS collections_count,
            (SELECT COUNT(*) FROM items) AS items_count,
            (SELECT COUNT(*) FROM asset_ocr) AS ocr_count,
            (SELECT COUNT(*) FROM ocr_universe_assets) AS ocr_universe_count,
            (SELECT COUNT(*) FROM asset_stt) AS stt_count,
            (SELECT COUNT(*) FROM stt_universe_assets) AS stt_universe_count,
            (SELECT COUNT(*) FROM asset_text) AS text_count,
            (SELECT COUNT(*) FROM viewable_assets) AS text_universe_count,
            (SELECT COUNT(*) FROM asset_embeddings) AS embed_count,
            (SELECT COUNT(*)
               FROM processing_tasks pt
              WHERE pt.kind = 'ocr'
                AND pt.state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled')
            ) AS pending_ocr_count,
            (SELECT COUNT(*)
               FROM processing_tasks pt
              WHERE pt.kind = 'embedding'
                AND pt.state NOT IN ('succeeded', 'failed', 'skipped', 'cancelled')
            ) AS pending_embed_count
        `)

      const row = rows[0] ?? ({} as CorpusStatsRow)
      return {
        collections: Number(row.collections_count ?? 0),
        items: Number(row.items_count ?? 0),
        ocr: Number(row.ocr_count ?? 0),
        ocrUniverse: Number(row.ocr_universe_count ?? 0),
        stt: Number(row.stt_count ?? 0),
        sttUniverse: Number(row.stt_universe_count ?? 0),
        text: Number(row.text_count ?? 0),
        textUniverse: Number(row.text_universe_count ?? 0),
        embeddings: Number(row.embed_count ?? 0),
        pendingOcr: Number(row.pending_ocr_count ?? 0),
        pendingEmbeddings: Number(row.pending_embed_count ?? 0),
      }
    }

    // Drizzle fallback (no raw client): one aggregate per statistic, each an
    // EXISTS predicate over individual viewable (leaf) assets — file
    // granularity, mirroring the raw path's asset-level CTEs above without
    // reproducing them verbatim.
    const leafFilter = sql`NOT EXISTS (
      SELECT 1 FROM assets child WHERE child.parent_asset_id = ${assets.id}
    )`
    const nativeTextExists = sql`
      EXISTS (
        SELECT 1 FROM extractions e
         WHERE e.asset_id = ${assets.id}
           AND e.method = 'native'
           AND e.text_content IS NOT NULL AND TRIM(e.text_content) <> ''
      )
    `
    // OCR universe: an image asset, or a PDF asset with no non-empty native
    // text layer (mirrors ocr_universe_assets in the raw path above).
    const ocrUniversePredicate = sql`(
      ${assets.type} = 'image'
      OR (${assets.type} = 'pdf' AND NOT ${nativeTextExists})
    )`
    const ocrTextExists = sql`
      EXISTS (
        SELECT 1 FROM extractions e
         WHERE e.asset_id = ${assets.id}
           AND e.method <> 'native'
           AND e.text_content IS NOT NULL AND TRIM(e.text_content) <> ''
      )
    `
    const transcriptionTextExists = sql`
      EXISTS (
        SELECT 1 FROM transcriptions t
         WHERE t.asset_id = ${assets.id}
           AND t.text_content IS NOT NULL AND TRIM(t.text_content) <> ''
      )
    `
    const anyExtractionTextExists = sql`
      EXISTS (
        SELECT 1 FROM extractions e
         WHERE e.asset_id = ${assets.id}
           AND e.text_content IS NOT NULL AND TRIM(e.text_content) <> ''
      )
    `
    const vecAssetExists = sql`EXISTS (SELECT 1 FROM vec_assets v WHERE v.asset_id = ${assets.id})`
    const textExists = sql`(${anyExtractionTextExists} OR ${transcriptionTextExists})`
    const ocrPredicate = and(leafFilter, ocrUniversePredicate, ocrTextExists)
    const ocrUniverseFilter = and(leafFilter, ocrUniversePredicate)
    const sttPredicate = and(leafFilter, eq(assets.type, 'audio'), transcriptionTextExists)
    const sttUniverseFilter = and(leafFilter, eq(assets.type, 'audio'))
    const textPredicate = and(leafFilter, textExists)
    const embedPredicate = and(leafFilter, vecAssetExists, textExists)

    const [
      collectionsRows,
      itemsRows,
      ocrRows,
      ocrUniverseRows,
      sttRows,
      sttUniverseRows,
      textRows,
      textUniverseRows,
      embedRows,
      pendingOcrRows,
      pendingEmbedRows,
    ] = await Promise.all([
      this.db.select({ count: sql<number>`count(*)` }).from(collections),
      this.db.select({ count: sql<number>`count(*)` }).from(items),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .where(ocrPredicate),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .where(ocrUniverseFilter),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .where(sttPredicate),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .where(sttUniverseFilter),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .where(textPredicate),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .where(leafFilter),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(assets)
        .where(embedPredicate),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(processingTasks)
        .where(
          and(
            eq(processingTasks.kind, 'ocr'),
            sql`${processingTasks.state} NOT IN ('succeeded', 'failed', 'skipped', 'cancelled')`
          )
        ),
      this.db
        .select({ count: sql<number>`count(*)` })
        .from(processingTasks)
        .where(
          and(
            eq(processingTasks.kind, 'embedding'),
            sql`${processingTasks.state} NOT IN ('succeeded', 'failed', 'skipped', 'cancelled')`
          )
        ),
    ])

    return {
      collections: Number(collectionsRows[0]?.count ?? 0),
      items: Number(itemsRows[0]?.count ?? 0),
      ocr: Number(ocrRows[0]?.count ?? 0),
      ocrUniverse: Number(ocrUniverseRows[0]?.count ?? 0),
      stt: Number(sttRows[0]?.count ?? 0),
      sttUniverse: Number(sttUniverseRows[0]?.count ?? 0),
      text: Number(textRows[0]?.count ?? 0),
      textUniverse: Number(textUniverseRows[0]?.count ?? 0),
      embeddings: Number(embedRows[0]?.count ?? 0),
      pendingOcr: Number(pendingOcrRows[0]?.count ?? 0),
      pendingEmbeddings: Number(pendingEmbedRows[0]?.count ?? 0),
    }
  }

  /**
   * The most recently imported documents across the whole corpus, newest
   * first, each carrying its collection's name — the home overview's
   * "Actividad reciente" panel (odd/tasks/home-view.md T3b). One indexed
   * query, no per-row lookups.
   */
  async findRecentlyImported(limit: number): Promise<RecentlyImportedItem[]> {
    if (this.rawClient) {
      const rows = await this.rawClient.select<RecentlyImportedItemRow>(
        `SELECT i.id, i.title, i.collection_id, c.name AS collection_name, i.created_at
           FROM items i
           JOIN collections c ON c.id = i.collection_id
          ORDER BY i.created_at DESC, i.id DESC
          LIMIT ?`,
        [limit]
      )

      return rows.map((row) => ({
        id: row.id,
        title: row.title,
        collectionId: row.collection_id,
        collectionName: row.collection_name,
        createdAt: row.created_at,
      }))
    }

    return this.db
      .select({
        id: items.id,
        title: items.title,
        collectionId: items.collectionId,
        collectionName: collections.name,
        createdAt: items.createdAt,
      })
      .from(items)
      .innerJoin(collections, eq(items.collectionId, collections.id))
      .orderBy(desc(items.createdAt), desc(items.id))
      .limit(limit)
  }

  /**
   * The item this collection already holds for this exact source file — same
   * path (compared without case, as Windows does), size and modification
   * time — or null. Importing that file again would only duplicate it.
   */
  async findImportedFromSource(
    collectionId: string,
    source: { originalPath: string; sizeBytes: number; modifiedAt: number | null }
  ): Promise<string | null> {
    if (!this.rawClient) return null

    const rows = await this.rawClient.select<{ id: string }>(
      `SELECT id FROM items
        WHERE collection_id = ?
          AND lower(json_extract(metadata, '$.__entropia_file_metadata.originalPath')) = lower(?)
          AND json_extract(metadata, '$.__entropia_file_metadata.sizeBytes') = ?
          AND json_extract(metadata, '$.__entropia_file_metadata.modifiedAt') IS ?
        LIMIT 1`,
      [collectionId, source.originalPath, source.sizeBytes, source.modifiedAt]
    )
    return rows[0]?.id ?? null
  }

  async findById(id: string): Promise<Item | null> {
    const rows = await this.db.select().from(items).where(eq(items.id, id))

    return rows[0] ?? null
  }

  async update(id: string, data: Partial<Pick<NewItem, 'title' | 'metadata'>>): Promise<Item> {
    const rows = await this.db
      .update(items)
      .set({ ...data, updatedAt: Date.now() })
      .where(eq(items.id, id))
      .returning()

    return rows[0]!
  }

  async delete(id: string): Promise<void> {
    await this.db.delete(items).where(eq(items.id, id))
  }

  /**
   * Delete an item and ALL its associated data in a single atomic transaction.
   * This is used when the last asset of an item is removed — the item becomes
   * an orphan and should be fully cleaned up.
   *
   * Cleanup order (dependencies first):
   * 1. Extractions (FK → assets)
   * 2. Assets (FK → items)
   * 3. Entities (FK → items)
   * 4. Triples (FK → items)
   * 5. Asset embeddings (item_id in vec_assets)
   * 6. FTS rebuild from canonical rowid sources
   * 7. Notes (FK → items)
   * 8. Item itself
   *
   * @throws Error if rawClient is not available
   * @throws Error if the transaction fails
   */
  async deleteWithCascade(id: string): Promise<void> {
    if (!this.rawClient) {
      throw new Error('deleteWithCascade requires a rawClient for transactional execution')
    }

    const esc = id.replace(/'/g, "''")

    // Get the parent collection ID before deleting the item (needed for auto-cleanup)
    const parentRows = await this.rawClient.select(
      `SELECT collection_id FROM items WHERE id = '${esc}'`,
      []
    )
    const collectionId = parentRows[0]?.collection_id as string | undefined
    const escCollectionId = collectionId !== undefined ? collectionId.replace(/'/g, "''") : ''

    // Phase 1: Atomic transaction for core tables (always exist)
    try {
      await this.rawClient.executeBatch(`
        BEGIN;
        DELETE FROM extractions WHERE asset_id IN (SELECT id FROM assets WHERE item_id = '${esc}');
        DELETE FROM layouts WHERE asset_id IN (SELECT id FROM assets WHERE item_id = '${esc}');
        DELETE FROM llm_results WHERE (target_type = 'asset' OR target_type = 'unknown') AND target_id IN (SELECT id FROM assets WHERE item_id = '${esc}');
        DELETE FROM llm_results WHERE target_id = '${esc}' AND (target_type = 'item' OR target_type = 'unknown');
        DELETE FROM assets WHERE item_id = '${esc}';
        DELETE FROM entities WHERE item_id = '${esc}';
        DELETE FROM triples WHERE item_id = '${esc}';
        DELETE FROM notes WHERE item_id = '${esc}';
        DELETE FROM items WHERE id = '${esc}';
        DELETE FROM collections WHERE id = '${escCollectionId}' AND id NOT IN (SELECT DISTINCT collection_id FROM items);
        COMMIT;
      `)
    } catch (e) {
      try {
        await this.rawClient.executeBatch('ROLLBACK')
      } catch {
        /* rollback is best-effort; preserve the original failure */
      }

      throw new Error(
        `Failed to delete item cascade for ${id}: ${e instanceof Error ? e.message : String(e)}`
      )
    }

    // Phase 2: Best-effort cleanup for optional tables / derived indexes
    try {
      await this.ftsRepo?.rebuildIndex()
    } catch {
      /* table may not exist — non-fatal */
    }

    try {
      await this.rawClient.execute(`DELETE FROM vec_assets WHERE item_id = '${esc}'`)
    } catch {
      /* table may not exist — non-fatal */
    }
  }

  /**
   * Search items by text.
   * - If a rawClient was provided (FTS5 available), tries FTS5 first.
   *   If FTS5 returns results, fetches those items from Drizzle and returns them.
   * - Falls back to SQL LIKE on title and metadata if FTS5 is unavailable or returns nothing.
   */
  async searchByText(collectionId: string, query: string): Promise<Item[]> {
    // Try FTS5 first if rawClient is available
    if (this.ftsRepo && query.trim()) {
      const ftsResults = await this.ftsRepo.search(query, 50)
      if (ftsResults.length > 0) {
        // Fetch the actual items from Drizzle using the IDs returned by FTS5
        const ids = ftsResults.map((r) => r.itemId)
        const rows = await this.db
          .select()
          .from(items)
          .where(
            and(
              eq(items.collectionId, collectionId),
              // Filter to items whose IDs are in the FTS5 result set
              // We use an OR chain over all matched IDs
              ids.length === 1 ? eq(items.id, ids[0]!) : or(...ids.map((id) => eq(items.id, id)))!
            )
          )
          .orderBy(asc(items.title), asc(items.id))

        return rows
      }
    }

    // Fallback: SQL LIKE on title and metadata
    const pattern = `%${query}%`
    return this.db
      .select()
      .from(items)
      .where(
        and(
          eq(items.collectionId, collectionId),
          or(like(items.title, pattern), like(items.metadata, pattern))
        )
      )
      .orderBy(asc(items.title), asc(items.id))
  }

  /**
   * FTS5-based search. Returns FtsResult[] with itemId and rank.
   * Requires a rawClient (DbClient) to be provided at construction time.
   * Returns empty array if no rawClient or empty query.
   */
  async searchByFts5(query: string, _collectionId?: string): Promise<FtsResult[]> {
    if (!this.ftsRepo || !query.trim()) return []
    return this.ftsRepo.search(query, 50)
  }

  /**
   * Search items across ALL collections.
   * Tries FTS5 first, falls back to SQL LIKE on title and metadata.
   */
  async searchGlobal(query: string, limit = 20): Promise<Item[]> {
    if (!query.trim()) return []

    // Try FTS5 first
    if (this.ftsRepo) {
      const ftsResults = await this.ftsRepo.search(query, limit)
      if (ftsResults.length > 0) {
        const ids = ftsResults.map((r) => r.itemId)
        return this.db
          .select()
          .from(items)
          .where(
            ids.length === 1 ? eq(items.id, ids[0]!) : or(...ids.map((id) => eq(items.id, id)))!
          )
          .orderBy(asc(items.title), asc(items.id))
      }
    }

    // Fallback: SQL LIKE on title and metadata
    const pattern = `%${query}%`
    return this.db
      .select()
      .from(items)
      .where(or(like(items.title, pattern), like(items.metadata, pattern)))
      .orderBy(asc(items.title), asc(items.id))
      .limit(limit)
  }
}
