import { invoke } from '@tauri-apps/api/core'
import { fireEvent, render, screen } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const mockInvoke = vi.mocked(invoke)

const { zoteroStore } = vi.hoisted(() => {
  const snapshot = {
    status: { state: 'available' },
    probing: false,
    loading: false,
    query: '',
    entries: [
      {
        key: '37C8RJP8',
        itemVersion: 9756,
        libraryType: 'user',
        libraryId: '0',
        title: 'Los orígenes',
        authors: 'Moore',
        year: '1973',
        csl_json: JSON.stringify({
          id: 'moore1973',
          type: 'book',
          title: 'Los orígenes',
          author: [{ family: 'Moore', given: 'Barrington' }],
          issued: { 'date-parts': [[1973]] },
        }),
      },
    ],
    loaded: 1,
    total: 1,
    error: null,
    selection: { libraryType: 'user', libraryId: '0' },
  }

  return {
    zoteroStore: {
      snapshot,
      subscribe: vi.fn((run: (value: typeof snapshot) => void) => {
        run(snapshot)
        return () => {}
      }),
      connect: vi.fn(async () => {}),
      sync: vi.fn(async () => {}),
      search: vi.fn(),
      searchLibrary: vi.fn(async () => {}),
      select: vi.fn((libraryType: string, libraryId: string) => {
        snapshot.selection = { libraryType, libraryId } as typeof snapshot.selection
      }),
    },
  }
})

vi.mock('$lib/writing-zotero', () => ({ writingZotero: zoteroStore }))

import WritingZoteroTab from './WritingZoteroTab.svelte'

const PERSONAL = { libraryType: 'user', libraryId: '0', name: null, source: 'personal' }
const SEMINARIO = {
  libraryType: 'group',
  libraryId: '6680944',
  name: 'Seminario',
  source: 'catalog',
}

/** Answers known_libraries; the store itself is mocked, so nothing else is asked. */
function answerKnownLibraries(known: unknown[]) {
  mockInvoke.mockImplementation(((cmd: string) => {
    if (cmd === 'writing_zotero_known_libraries') return Promise.resolve(known)
    return Promise.reject(new Error(`unexpected ${cmd}`))
  }) as never)
}

beforeEach(() => {
  vi.clearAllMocks()
  mockInvoke.mockReset()
  localStorage.clear()
  zoteroStore.snapshot.selection = { libraryType: 'user', libraryId: '0' }
})

describe('the Zotero listing citation seam', () => {
  it('inserts native Zotero identity separately from the CSL id', async () => {
    answerKnownLibraries([PERSONAL])
    const oncite = vi.fn(() => 'citation-1')
    const csl = zoteroStore.snapshot.entries[0]!.csl_json

    render(WritingZoteroTab, { props: { oncite } })

    expect(await screen.findByText('Los orígenes')).toBeInTheDocument()
    await fireEvent.click(screen.getByRole('button', { name: 'Citar' }))

    expect(oncite).toHaveBeenCalledWith({
      sourceOrigin: 'local',
      sourceInstanceId: null,
      itemKey: '37C8RJP8',
      itemVersion: 9756,
      libraryType: 'user',
      libraryId: '0',
      metadataSnapshot: csl,
    })
  })
})

/**
 * E1c-2 (UI half): the library selector. RED first: none of this is rendered
 * yet, so every query below misses.
 */
describe('E1c-2 library selector', () => {
  it('lists known libraries merged with manual entries, personal first', async () => {
    answerKnownLibraries([PERSONAL, SEMINARIO])
    localStorage.setItem(
      'entropia:zotero:added-libraries',
      JSON.stringify([{ libraryType: 'user', libraryId: '9', unverified: false }])
    )

    render(WritingZoteroTab, { props: {} })

    // The offered list arrives after the mount; wait for it before reading.
    await screen.findByRole('option', { name: 'Seminario' })
    const selector = await screen.findByLabelText('Biblioteca')
    const options = [...(selector as HTMLSelectElement).options].map((o) => o.text)
    expect(options[0]).toBe('Personal')
    expect(options).toContain('Seminario')
    expect(options).toContain('user/9')
  })

  it('selecting an entry selects it in the store and reconnects', async () => {
    answerKnownLibraries([PERSONAL, SEMINARIO])
    render(WritingZoteroTab, { props: {} })
    await screen.findByRole('option', { name: 'Seminario' })
    const selector = await screen.findByLabelText('Biblioteca')
    const connected = zoteroStore.connect.mock.calls.length

    await fireEvent.change(selector, { target: { value: 'group/6680944' } })

    expect(zoteroStore.select).toHaveBeenCalledWith('group', '6680944')
    expect(zoteroStore.connect.mock.calls.length).toBeGreaterThan(connected)
  })

  it('reflects the current selection', async () => {
    answerKnownLibraries([PERSONAL, SEMINARIO])
    zoteroStore.snapshot.selection = { libraryType: 'group', libraryId: '6680944' }

    render(WritingZoteroTab, { props: {} })

    await screen.findByRole('option', { name: 'Seminario' })
    expect(((await screen.findByLabelText('Biblioteca')) as HTMLSelectElement).value).toBe(
      'group/6680944'
    )
  })

  it('opens the tab on the personal library when the selector is untouched', async () => {
    answerKnownLibraries([PERSONAL, SEMINARIO])
    render(WritingZoteroTab, { props: {} })

    await screen.findByText('Los orígenes')
    expect(zoteroStore.select).not.toHaveBeenCalled()
    expect(((await screen.findByLabelText('Biblioteca')) as HTMLSelectElement).value).toBe(
      'user/0'
    )
  })
})

describe('E1c-2 adding a library by hand', () => {
  async function openAddForm() {
    answerKnownLibraries([PERSONAL])
    mockInvoke.mockImplementation(((cmd: string, _args?: Record<string, unknown>) => {
      if (cmd === 'writing_zotero_known_libraries') return Promise.resolve([PERSONAL])
      if (cmd === 'writing_zotero_check_library')
        return Promise.resolve({ status: 'available', version: 3 })
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
    render(WritingZoteroTab, { props: {} })
    await screen.findByLabelText('Biblioteca')
    await fireEvent.click(screen.getByRole('button', { name: 'Agregar biblioteca' }))
  }

  it('adds an available library and selects it', async () => {
    await openAddForm()

    await fireEvent.change(screen.getByLabelText('Tipo'), { target: { value: 'group' } })
    await fireEvent.input(screen.getByLabelText('ID'), { target: { value: '6680944' } })
    await fireEvent.click(screen.getByRole('button', { name: /^Agregar$/ }))

    expect(await screen.findByText('group/6680944')).toBeInTheDocument()
    expect(zoteroStore.select).toHaveBeenCalledWith('group', '6680944')
    expect(JSON.parse(localStorage.getItem('entropia:zotero:added-libraries')!)).toEqual([
      { libraryType: 'group', libraryId: '6680944', unverified: false },
    ])
  })

  it('adds an unverifiable library too, with a factual hint', async () => {
    answerKnownLibraries([PERSONAL])
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'writing_zotero_known_libraries') return Promise.resolve([PERSONAL])
      if (cmd === 'writing_zotero_check_library')
        return Promise.resolve({ status: 'unverifiable' })
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
    render(WritingZoteroTab, { props: {} })
    await screen.findByLabelText('Biblioteca')
    await fireEvent.click(screen.getByRole('button', { name: 'Agregar biblioteca' }))

    await fireEvent.change(screen.getByLabelText('Tipo'), { target: { value: 'user' } })
    await fireEvent.input(screen.getByLabelText('ID'), { target: { value: '9' } })
    await fireEvent.click(screen.getByRole('button', { name: /^Agregar$/ }))

    expect(await screen.findByText('user/9 (sin verificar)')).toBeInTheDocument()
    expect(zoteroStore.select).toHaveBeenCalledWith('user', '9')
  })

  it('shows a not_found error inline and adds nothing', async () => {
    answerKnownLibraries([PERSONAL])
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'writing_zotero_known_libraries') return Promise.resolve([PERSONAL])
      if (cmd === 'writing_zotero_check_library')
        return Promise.resolve({ status: 'not_found' })
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
    render(WritingZoteroTab, { props: {} })
    await screen.findByLabelText('Biblioteca')
    await fireEvent.click(screen.getByRole('button', { name: 'Agregar biblioteca' }))

    await fireEvent.change(screen.getByLabelText('Tipo'), { target: { value: 'group' } })
    await fireEvent.input(screen.getByLabelText('ID'), { target: { value: '404' } })
    await fireEvent.click(screen.getByRole('button', { name: /^Agregar$/ }))

    expect(await screen.findByRole('alert')).toBeInTheDocument()
    expect(zoteroStore.select).not.toHaveBeenCalled()
    expect(localStorage.getItem('entropia:zotero:added-libraries')).toBeNull()
  })

  it('validates a blank id inline without asking Zotero', async () => {
    await openAddForm()

    await fireEvent.input(screen.getByLabelText('ID'), { target: { value: '   ' } })
    await fireEvent.click(screen.getByRole('button', { name: /^Agregar$/ }))

    expect(await screen.findByRole('alert')).toBeInTheDocument()
    expect(mockInvoke.mock.calls.some(([cmd]) => cmd === 'writing_zotero_check_library')).toBe(
      false
    )
    expect(zoteroStore.select).not.toHaveBeenCalled()
  })
})
