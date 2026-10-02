import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  ADDED_LIBRARIES_STORAGE_KEY,
  addLibraryChecked,
  libraryLabel,
  loadLibraries,
  mergeLibraries,
  parseAddedLibraries,
  readAddedLibraries,
} from './writing-zotero-libraries'

/**
 * E1c-2 (UI half): the library list is a merge of what the backend knows
 * (personal always first, catalog names win) and what the person added by
 * hand, kept in localStorage. RED first: the module does not exist yet.
 */

const mockInvoke = vi.mocked(invoke)

const PERSONAL = { libraryType: 'user', libraryId: '0', name: null, source: 'personal' } as const
const SEMINARIO = {
  libraryType: 'group',
  libraryId: '6680944',
  name: 'Seminario',
  source: 'catalog',
} as const
const MIRROR_USER = {
  libraryType: 'user',
  libraryId: '5',
  name: null,
  source: 'mirror',
} as const

beforeEach(() => {
  mockInvoke.mockReset()
  localStorage.clear()
})

describe('the persisted key', () => {
  it('uses the stable localStorage key the slice agreed on', () => {
    expect(ADDED_LIBRARIES_STORAGE_KEY).toBe('entropia:zotero:added-libraries')
  })
})

describe('reading what was added by hand', () => {
  it('keeps only well-formed entries and trims their ids', () => {
    localStorage.setItem(
      ADDED_LIBRARIES_STORAGE_KEY,
      JSON.stringify([
        { libraryType: 'group', libraryId: '6680944', unverified: true },
        { libraryType: 'user', libraryId: ' 9 ', unverified: false },
        { libraryType: 'group', libraryId: '   ' },
        { libraryType: 'archive', libraryId: '3' },
        { libraryType: 'user' },
        'group/7',
        null,
      ])
    )

    expect(readAddedLibraries()).toEqual([
      { libraryType: 'group', libraryId: '6680944', unverified: true },
      { libraryType: 'user', libraryId: '9', unverified: false },
    ])
  })

  it('reads an empty list when nothing usable was stored', () => {
    expect(readAddedLibraries()).toEqual([])

    localStorage.setItem(ADDED_LIBRARIES_STORAGE_KEY, '{not json')
    expect(readAddedLibraries()).toEqual([])

    localStorage.setItem(ADDED_LIBRARIES_STORAGE_KEY, JSON.stringify({ libraryId: '1' }))
    expect(readAddedLibraries()).toEqual([])
  })

  it('parses a raw value without touching storage', () => {
    expect(parseAddedLibraries(null)).toEqual([])
    expect(parseAddedLibraries(JSON.stringify([{ libraryType: 'user', libraryId: '0' }]))).toEqual([
      { libraryType: 'user', libraryId: '0', unverified: false },
    ])
  })
})

describe('merging what Zotero knows with what was added', () => {
  it('always offers the personal library first, even knowing nothing else', () => {
    expect(mergeLibraries([], [])).toEqual([
      { libraryType: 'user', libraryId: '0', name: null, source: 'personal', unverified: false },
    ])
  })

  it('keeps the known order and lets the known entry win over a manual duplicate', () => {
    const merged = mergeLibraries([PERSONAL, SEMINARIO, MIRROR_USER] as never, [
      { libraryType: 'group', libraryId: '6680944', unverified: true },
      { libraryType: 'user', libraryId: '9', unverified: true },
    ])

    expect(merged.map((e) => [e.libraryType, e.libraryId])).toEqual([
      ['user', '0'],
      ['group', '6680944'],
      ['user', '5'],
      ['user', '9'],
    ])
    // The catalog name survives the duplicate, and a verified source clears
    // the unverified hint: the backend knows this library.
    expect(merged[1]).toMatchObject({ name: 'Seminario', source: 'catalog', unverified: false })
    expect(merged[3]).toMatchObject({ source: 'added', unverified: true })
  })

  it('treats user/1 and group/1 as different libraries', () => {
    const merged = mergeLibraries(
      [],
      [
        { libraryType: 'user', libraryId: '1', unverified: false },
        { libraryType: 'group', libraryId: '1', unverified: false },
      ]
    )

    expect(merged).toHaveLength(3)
  })

  it('collapses a manual duplicate of the personal library', () => {
    const merged = mergeLibraries([], [{ libraryType: 'user', libraryId: '0', unverified: true }])

    expect(merged).toEqual([
      { libraryType: 'user', libraryId: '0', name: null, source: 'personal', unverified: false },
    ])
  })
})

describe('naming an entry', () => {
  it('prefers the catalog name, falls back to Personal and derived labels', () => {
    expect(
      libraryLabel(
        {
          libraryType: 'group',
          libraryId: '1',
          name: 'Seminario',
          source: 'catalog',
          unverified: false,
        },
        'Personal'
      )
    ).toBe('Seminario')
    expect(
      libraryLabel(
        { libraryType: 'user', libraryId: '0', name: null, source: 'personal', unverified: false },
        'Personal'
      )
    ).toBe('Personal')
    expect(
      libraryLabel(
        { libraryType: 'user', libraryId: '9', name: null, source: 'added', unverified: false },
        'Personal'
      )
    ).toBe('user/9')
    expect(
      libraryLabel(
        {
          libraryType: 'group',
          libraryId: '6680944',
          name: null,
          source: 'mirror',
          unverified: false,
        },
        'Personal'
      )
    ).toBe('group/6680944')
  })
})

describe('loading the offered list', () => {
  it('asks the backend once and merges the manual entries', async () => {
    mockInvoke.mockImplementation(((cmd: string) =>
      cmd === 'writing_zotero_known_libraries'
        ? Promise.resolve([PERSONAL, SEMINARIO])
        : Promise.reject(new Error(`unexpected ${cmd}`))) as never)
    localStorage.setItem(
      ADDED_LIBRARIES_STORAGE_KEY,
      JSON.stringify([{ libraryType: 'user', libraryId: '9', unverified: false }])
    )

    const libraries = await loadLibraries()

    expect(mockInvoke).toHaveBeenCalledWith('writing_zotero_known_libraries')
    expect(libraries.map((e) => [e.libraryType, e.libraryId])).toEqual([
      ['user', '0'],
      ['group', '6680944'],
      ['user', '9'],
    ])
  })

  it('still offers personal plus manual entries when the backend cannot be asked', async () => {
    mockInvoke.mockRejectedValue(new Error('Zotero is not answering'))
    localStorage.setItem(
      ADDED_LIBRARIES_STORAGE_KEY,
      JSON.stringify([{ libraryType: 'group', libraryId: '7', unverified: true }])
    )

    const libraries = await loadLibraries()

    expect(libraries.map((e) => [e.libraryType, e.libraryId])).toEqual([
      ['user', '0'],
      ['group', '7'],
    ])
  })
})

describe('checking a library before adding it', () => {
  function checkAnswer(reply: unknown) {
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'writing_zotero_known_libraries') return Promise.resolve([PERSONAL])
      if (cmd === 'writing_zotero_check_library') return Promise.resolve(reply)
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
  }

  it('refuses a blank id without asking Zotero', async () => {
    const outcome = await addLibraryChecked('group', '   ')

    expect(outcome).toEqual({ ok: false, error: 'invalid' })
    expect(mockInvoke).not.toHaveBeenCalledWith('writing_zotero_check_library', expect.anything())
    expect(localStorage.getItem(ADDED_LIBRARIES_STORAGE_KEY)).toBeNull()
  })

  it('adds an available library as verified', async () => {
    checkAnswer({ status: 'available', version: 12 })

    const outcome = await addLibraryChecked('group', '6680944')

    expect(mockInvoke).toHaveBeenCalledWith('writing_zotero_check_library', {
      libraryType: 'group',
      libraryId: '6680944',
    })
    expect(outcome.ok).toBe(true)
    if (outcome.ok) {
      expect(outcome.selected).toEqual({ libraryType: 'group', libraryId: '6680944' })
      expect(outcome.libraries.find((e) => e.libraryId === '6680944')).toMatchObject({
        unverified: false,
      })
    }
    expect(JSON.parse(localStorage.getItem(ADDED_LIBRARIES_STORAGE_KEY)!)).toEqual([
      { libraryType: 'group', libraryId: '6680944', unverified: false },
    ])
  })

  it('adds an unverifiable library too, keeping the hint', async () => {
    checkAnswer({ status: 'unverifiable' })

    const outcome = await addLibraryChecked('user', '9')

    expect(outcome.ok).toBe(true)
    if (outcome.ok) {
      expect(outcome.libraries.find((e) => e.libraryId === '9')).toMatchObject({ unverified: true })
    }
    expect(JSON.parse(localStorage.getItem(ADDED_LIBRARIES_STORAGE_KEY)!)).toEqual([
      { libraryType: 'user', libraryId: '9', unverified: true },
    ])
  })

  it('shows not_found inline and adds nothing', async () => {
    checkAnswer({ status: 'not_found' })

    const outcome = await addLibraryChecked('group', '404')

    expect(outcome).toEqual({ ok: false, error: 'not_found' })
    expect(localStorage.getItem(ADDED_LIBRARIES_STORAGE_KEY)).toBeNull()
  })

  it('shows a check failure inline and adds nothing', async () => {
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'writing_zotero_known_libraries') return Promise.resolve([PERSONAL])
      if (cmd === 'writing_zotero_check_library')
        return Promise.reject(new Error('the port did not answer'))
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)

    const outcome = await addLibraryChecked('group', '7')

    expect(outcome).toEqual({ ok: false, error: 'check_failed' })
    expect(localStorage.getItem(ADDED_LIBRARIES_STORAGE_KEY)).toBeNull()
  })
})
