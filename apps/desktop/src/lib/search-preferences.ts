import { SETTINGS_KEYS, settingsGet, settingsSet } from './settings'

/**
 * Whether searches also try close variants of the words (OCR misreadings,
 * typos, other spellings of a rare name).
 *
 * One setting for every search surface — the Corpus tab, collection search,
 * the agent's lexical retrieval — kept in `app_settings`, so it is the same in
 * Lite and Pro and the Rust side can read it too. On unless someone turns it
 * off: the approximate results always come after the exact ones and are
 * labelled, so leaving it on costs nothing a reader cannot see.
 */

interface SettingsBacking {
  get(key: string): Promise<string | null>
  set(key: string, value: string): Promise<void>
}

const OFF = 'off'
const ON = 'on'

export class SearchPreferences {
  constructor(private readonly backing: SettingsBacking = { get: settingsGet, set: settingsSet }) {}

  async fuzzyEnabled(): Promise<boolean> {
    try {
      return (await this.backing.get(SETTINGS_KEYS.SEARCH_FUZZY)) !== OFF
    } catch {
      // Not knowing is not a choice to turn it off.
      return true
    }
  }

  async setFuzzyEnabled(enabled: boolean): Promise<void> {
    await this.backing.set(SETTINGS_KEYS.SEARCH_FUZZY, enabled ? ON : OFF)
  }
}

export const searchPreferences = new SearchPreferences()
