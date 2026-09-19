import { describe, expect, it, vi } from 'vitest'
import { SearchPreferences } from './search-preferences'

function backing(initial: string | null) {
  let value = initial
  return {
    get: vi.fn(async () => value),
    set: vi.fn(async (_key: string, next: string) => {
      value = next
    }),
  }
}

describe('approximate search preference', () => {
  it('is on until someone turns it off', async () => {
    expect(await new SearchPreferences(backing(null)).fuzzyEnabled()).toBe(true)
  })

  it('stays off once turned off', async () => {
    expect(await new SearchPreferences(backing('off')).fuzzyEnabled()).toBe(false)
  })

  it('reads anything else as on, so a stray value never silently disables it', async () => {
    expect(await new SearchPreferences(backing('???')).fuzzyEnabled()).toBe(true)
  })

  it('persists the choice under the one key every search reads', async () => {
    const store = backing(null)
    const prefs = new SearchPreferences(store)

    await prefs.setFuzzyEnabled(false)

    expect(store.set).toHaveBeenCalledWith('search_fuzzy', 'off')
    expect(await prefs.fuzzyEnabled()).toBe(false)
  })

  it('falls back to on when the setting cannot be read', async () => {
    const prefs = new SearchPreferences({
      get: async () => {
        throw new Error('db locked')
      },
      set: async () => {},
    })

    expect(await prefs.fuzzyEnabled()).toBe(true)
  })
})
