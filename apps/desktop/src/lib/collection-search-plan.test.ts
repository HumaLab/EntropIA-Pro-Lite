import { describe, expect, it, vi } from 'vitest'
import { CollectionSearchPlanner } from './collection-search-plan'
import { SearchPreferences } from './search-preferences'

function setup(fuzzySetting: string | null = null) {
  let setting = fuzzySetting
  const prefs = new SearchPreferences({
    get: async () => setting,
    set: async (_key, value) => {
      setting = value
    },
  })
  const compile = vi.fn(async (query: string, options: { fuzzy?: boolean }) => ({
    raw: query,
    strictMatch: `"${query}"`,
    relaxedMatch: null,
    likeTerms: [query],
    fuzzyMatch: options.fuzzy ? `("${query}" OR "variant")` : null,
  }))
  return { planner: new CollectionSearchPlanner(compile, prefs), compile, prefs }
}

describe('collection search plan', () => {
  it('compiles a query once and reuses the plan for every page', async () => {
    const { planner, compile } = setup()

    const first = await planner.planFor('sindicato')
    const second = await planner.planFor('sindicato')

    expect(compile).toHaveBeenCalledTimes(1)
    expect(second).toBe(first)
  })

  it('asks for approximate search when the switch is on', async () => {
    const { planner, compile } = setup(null)

    await planner.planFor('sindicato')

    expect(compile).toHaveBeenCalledWith('sindicato', { fuzzy: true })
  })

  it('compiles again when the switch changes, even for the same words', async () => {
    const { planner, compile, prefs } = setup(null)
    await planner.planFor('sindicato')

    await prefs.setFuzzyEnabled(false)
    await planner.planFor('sindicato')

    expect(compile).toHaveBeenLastCalledWith('sindicato', { fuzzy: false })
  })

  it('leaves a blank query as it is: no search, not an empty result', async () => {
    const { planner, compile } = setup()

    expect(await planner.planFor('   ')).toBe('   ')
    expect(compile).not.toHaveBeenCalled()
  })

  it('falls back to the plain query even when compiling throws before it starts', async () => {
    const planner = new CollectionSearchPlanner(
      () => {
        throw new TypeError('store has no fts')
      },
      new SearchPreferences({ get: async () => null, set: async () => {} })
    )

    expect(await planner.planFor('sindicato')).toBe('sindicato')
  })

  it('falls back to the plain query when compiling fails', async () => {
    const planner = new CollectionSearchPlanner(
      async () => {
        throw new Error('no vocabulary')
      },
      new SearchPreferences({ get: async () => null, set: async () => {} })
    )

    expect(await planner.planFor('sindicato')).toBe('sindicato')
  })
})
