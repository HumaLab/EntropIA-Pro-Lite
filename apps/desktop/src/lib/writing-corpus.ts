import { getStore } from '$lib/db'

/**
 * The Corpus tab's state (plan-editor.md §6.3, §10.1).
 *
 * # What this is not
 *
 * It is not a retrieval engine. Unit 4 is explicit that a second one must not
 * be written, so every search here goes through `store.fts` — the same index,
 * the same sanitising and the same strict-then-relaxed strategy the item view
 * already uses. What this store adds is only what Escritura needs on top:
 * hydrating a hit into something citable, and walking down to the page whose
 * text a citation will quote.
 *
 * # Why the walk goes item, then page
 *
 * `fts_items` is indexed per item and is contentless — the text lives in
 * `extractions`, one row per asset. And an asset *is* a page: migration 0024
 * gives `assets.page_number`, so a PDF arrives already split. So a hit names an
 * item, the item offers its pages, and the page carries the text. That chain is
 * also exactly the anchor a citation stores, which is why the panel is built
 * along it rather than flattening it.
 *
 * A plain class implementing the store contract by hand, like `WritingStore`
 * and the other global stores here — deliberately not a `.svelte.ts`, so it is
 * testable without mounting anything.
 */

const SEARCH_LIMIT = 20

/** One search hit, hydrated into something a writer can act on. */
export interface CorpusHit {
  itemId: string
  title: string
  /** Not nullable: `items.collection_id` is `NOT NULL` in the schema. */
  collectionId: string
  rank: number
  /**
   * Set only when the search did not find the words as typed: the close
   * variants this document holds instead (an OCR misreading, or the spelling
   * a typo was corrected to). Shown so the writer knows why it is here.
   */
  foundAs?: string[]
}

/** One page of an open item. The asset is the page (migration 0024). */
export interface CorpusPage {
  assetId: string
  pageNumber: number | null
  /** `assets.type`: 'image' | 'pdf' | 'audio'. */
  type: string
}

export interface CorpusOpenItem {
  itemId: string
  title: string
  collectionId: string
}

export interface CorpusSnapshot {
  query: string
  searching: boolean
  results: CorpusHit[]
  openItem: CorpusOpenItem | null
  pages: CorpusPage[]
  openPageId: string | null
  /** The extracted text of the open page. Empty when there is none to quote. */
  pageText: string
  error: string | null
}

const EMPTY: CorpusSnapshot = {
  query: '',
  searching: false,
  results: [],
  openItem: null,
  pages: [],
  openPageId: null,
  pageText: '',
  error: null,
}

type Subscriber = (value: CorpusSnapshot) => void

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export class WritingCorpusStore {
  #state: CorpusSnapshot = { ...EMPTY }
  #subscribers = new Set<Subscriber>()
  #store: () => ReturnType<typeof getStore>
  /**
   * Which search is the current one. Typing fast starts several; without this
   * the slowest wins and the panel shows results for a query the writer has
   * already moved on from.
   */
  #searchToken = 0

  constructor(store: () => ReturnType<typeof getStore> = getStore) {
    this.#store = store
  }

  subscribe(run: Subscriber): () => void {
    this.#subscribers.add(run)
    run(this.#state)
    return () => this.#subscribers.delete(run)
  }

  #set(patch: Partial<CorpusSnapshot>) {
    this.#state = { ...this.#state, ...patch }
    for (const run of this.#subscribers) run(this.#state)
  }

  get snapshot(): CorpusSnapshot {
    return this.#state
  }

  async search(rawQuery: string): Promise<void> {
    const query = rawQuery.trim()
    this.#searchToken += 1
    const token = this.#searchToken

    if (!query) {
      this.#set({ query: rawQuery, searching: false, results: [], error: null })
      return
    }

    this.#set({ query: rawQuery, searching: true, error: null })
    try {
      const store = this.#store()
      // Only documents with text: one nothing was read from has nothing to
      // quote, and would otherwise match by its title alone.
      const hits = await store.fts.search(query, SEARCH_LIMIT, { withTextOnly: true })
      const hydrated = await Promise.all(
        hits.map(async (hit) => {
          const item = await store.items.findById(hit.itemId)
          // A hit whose item is gone is dropped rather than rendered as a blank
          // row: the index can outlive what it points at.
          if (!item) return null
          return {
            itemId: hit.itemId,
            title: item.title,
            collectionId: item.collectionId,
            rank: hit.rank,
            ...(hit.approximate ? { foundAs: hit.variants ?? [] } : {}),
          } satisfies CorpusHit
        })
      )
      if (token !== this.#searchToken) return
      this.#set({
        searching: false,
        results: hydrated.filter((hit): hit is CorpusHit => hit !== null),
      })
    } catch (error) {
      if (token !== this.#searchToken) return
      this.#set({ searching: false, error: message(error) })
    }
  }

  /** Opens an item and lists the pages a citation could point at. */
  async openItem(itemId: string): Promise<void> {
    try {
      const store = this.#store()
      const item = await store.items.findById(itemId)
      if (!item) {
        this.#set({ error: `no item ${itemId}` })
        return
      }
      const assets = await store.assets.findByItem(itemId)
      const pages = assets
        .map((asset) => ({
          assetId: asset.id,
          pageNumber: asset.pageNumber ?? null,
          type: asset.type,
        }))
        .sort((a, b) => (a.pageNumber ?? 0) - (b.pageNumber ?? 0))

      this.#set({
        openItem: { itemId, title: item.title, collectionId: item.collectionId },
        pages,
        openPageId: null,
        pageText: '',
        error: null,
      })
    } catch (error) {
      this.#set({ error: message(error) })
    }
  }

  /** Reads the page's extracted text — what a citation quotes and anchors into. */
  async openPage(assetId: string): Promise<void> {
    try {
      const extraction = await this.#store().extractions.findByAsset(assetId)
      // A page with no extraction is not a failure. It has simply never been
      // read, and there is nothing to quote from it yet.
      this.#set({ openPageId: assetId, pageText: extraction?.textContent ?? '', error: null })
    } catch (error) {
      this.#set({ openPageId: assetId, pageText: '', error: message(error) })
    }
  }

  closeItem(): void {
    this.#set({ openItem: null, pages: [], openPageId: null, pageText: '' })
  }
}

export const writingCorpus = new WritingCorpusStore()
