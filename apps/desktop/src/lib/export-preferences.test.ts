import { get } from 'svelte/store'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { CITATION_FIDELITY } from './export-fidelity'
import {
  CITATION_CHOICES,
  DEFAULT_EXPORT_PREFERENCES,
  citationsForFormat,
  parseExportPreferences,
} from './export-preferences'

/**
 * How a manuscript is exported, chosen once in the Export tab and used by every
 * download that follows.
 *
 * The preferences are the writer's, not the document's: they outlive switching
 * tabs, switching documents and restarting the application. The defaults are a
 * first-run fallback only — once anything has been saved, the saved value wins,
 * including a saved `false`.
 */

const settings = vi.hoisted(() => ({
  stored: new Map<string, string>(),
  settingsGet: vi.fn(),
  settingsSet: vi.fn(),
}))

vi.mock('./settings', async (importOriginal) => {
  const original = await importOriginal<typeof import('./settings')>()
  return {
    ...original,
    settingsGet: settings.settingsGet,
    settingsSet: settings.settingsSet,
  }
})

beforeEach(() => {
  vi.resetModules()
  settings.stored.clear()
  settings.settingsGet.mockReset()
  settings.settingsSet.mockReset()
  settings.settingsGet.mockImplementation(async (key: string) => settings.stored.get(key) ?? null)
  settings.settingsSet.mockImplementation(async (key: string, value: string) => {
    settings.stored.set(key, value)
  })
})

/** A fresh module, as a restarted application would load it. */
async function freshStore() {
  return import('./export-preferences')
}

describe('the first-run defaults', () => {
  it('keep the quoted text in the flow with a note, and include the bibliography', () => {
    expect(DEFAULT_EXPORT_PREFERENCES).toEqual({
      citations: 'quote_with_note',
      bibliography: true,
    })
  })

  it('apply only when nothing has been saved', () => {
    expect(parseExportPreferences(null, null)).toEqual(DEFAULT_EXPORT_PREFERENCES)
  })

  it('give way to a saved choice, including a saved "no bibliography"', () => {
    expect(parseExportPreferences('footnote', 'false')).toEqual({
      citations: 'footnote',
      bibliography: false,
    })
  })

  it('stand in for a saved value this version does not recognise', () => {
    expect(parseExportPreferences('marginalia', 'maybe')).toEqual(DEFAULT_EXPORT_PREFERENCES)
  })
})

describe('the four corpus citation choices', () => {
  it('are exactly the representations the fidelity matrix knows', () => {
    expect(CITATION_CHOICES.map((choice) => choice.id).sort()).toEqual(
      Object.keys(CITATION_FIDELITY).sort()
    )
  })
})

describe('a representation the format cannot carry', () => {
  it('becomes a footnote, the one every format carries natively', () => {
    expect(citationsForFormat('comment', 'markdown')).toBe('footnote')
  })

  it('is kept wherever the format admits it', () => {
    expect(citationsForFormat('comment', 'docx')).toBe('comment')
    expect(citationsForFormat('comment', 'html')).toBe('comment')
    expect(citationsForFormat('quote_with_note', 'markdown')).toBe('quote_with_note')
  })
})

describe('the persisted preferences', () => {
  it('start from the defaults when nothing was ever saved', async () => {
    const store = await freshStore()
    await store.loadExportPreferences()

    expect(get(store.exportPreferences)).toEqual(DEFAULT_EXPORT_PREFERENCES)
    expect(settings.settingsSet).not.toHaveBeenCalled()
  })

  it('survive a restart', async () => {
    const before = await freshStore()
    await before.loadExportPreferences()
    await before.setExportPreferences({ citations: 'footnote', bibliography: false })

    const after = await freshStore()
    await after.loadExportPreferences()

    expect(get(after.exportPreferences)).toEqual({ citations: 'footnote', bibliography: false })
  })

  it('are not overwritten by a load that finishes after the writer chose', async () => {
    const pending: ((value: string | null) => void)[] = []
    settings.settingsGet.mockImplementation(
      () => new Promise<string | null>((resolve) => pending.push(resolve))
    )
    const store = await freshStore()
    const loading = store.loadExportPreferences()

    await store.setExportPreferences({ citations: 'inline', bibliography: true })
    for (const answer of pending) answer(null)
    await loading

    expect(get(store.exportPreferences)).toEqual({ citations: 'inline', bibliography: true })
  })

  it('keep working when the settings cannot be read', async () => {
    settings.settingsGet.mockRejectedValue(new Error('locked'))
    const store = await freshStore()
    await store.loadExportPreferences()

    expect(get(store.exportPreferences)).toEqual(DEFAULT_EXPORT_PREFERENCES)
  })

  it('keep the choice on screen when the settings cannot be written', async () => {
    settings.settingsSet.mockRejectedValue(new Error('locked'))
    const store = await freshStore()

    await store.setExportPreferences({ citations: 'comment', bibliography: false })

    expect(get(store.exportPreferences)).toEqual({ citations: 'comment', bibliography: false })
  })
})
