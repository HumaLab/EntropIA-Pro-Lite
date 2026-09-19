import type { CardSearchPlan, FtsSearchOptions } from '@entropia/store'
import { searchPreferences, type SearchPreferences } from './search-preferences'

type Compile = (query: string, options: FtsSearchOptions) => Promise<CardSearchPlan>

/**
 * The search plan a collection pages through.
 *
 * Compiled once per query and switch position, then reused for every page: its
 * approximate branch comes from the index vocabulary, which can be reloaded
 * between pages, and a plan that changed mid-scroll would page through two
 * different result sets. The switch is part of the key, so turning approximate
 * search off takes effect on the next search without clearing anything.
 *
 * A blank query is handed back untouched — the repository reads it as "no
 * search". And if compiling fails the raw query is handed back too, which the
 * repository searches exactly, as it did before approximate search existed.
 */
export class CollectionSearchPlanner {
  #cached: { key: string; plan: Promise<string | CardSearchPlan> } | null = null

  constructor(
    private readonly compile: Compile,
    private readonly prefs: SearchPreferences = searchPreferences
  ) {}

  async planFor(query: string): Promise<string | CardSearchPlan> {
    if (!query.trim()) return query
    const fuzzy = await this.prefs.fuzzyEnabled()
    const key = `${fuzzy ? 'fuzzy' : 'exact'}:${query}`
    if (this.#cached?.key !== key) {
      this.#cached = {
        key,
        // Started inside a promise so a compile that throws before returning
        // one is caught too, not only one that rejects.
        plan: Promise.resolve()
          .then(() => this.compile(query, { fuzzy }))
          .catch(() => query),
      }
    }
    return this.#cached.plan
  }
}
