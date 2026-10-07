import { invoke } from '@tauri-apps/api/core'
import { fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

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
    fuzzy: true,
    bibliographyProgress: null as unknown,
    bibliographySync: {
      loading: false,
      error: null as string | null,
      requested: null as {
        batchId: string
        taskId: string
        created: boolean
        requeued: boolean
      } | null,
    },
  }

  const subscribers = new Set<(value: typeof snapshot) => void>()
  const emit = () => {
    // A fresh object per change: Svelte 5 state compares by identity, so a
    // mutated-then-resent object would never repaint anything.
    const value = { ...snapshot, selection: { ...snapshot.selection } } as typeof snapshot
    for (const run of subscribers) run(value)
  }

  return {
    zoteroStore: {
      snapshot,
      subscribe: vi.fn((run: (value: typeof snapshot) => void) => {
        subscribers.add(run)
        run(snapshot)
        return () => {
          subscribers.delete(run)
        }
      }),
      connect: vi.fn(async () => {}),
      sync: vi.fn(async () => {}),
      requestBibliographySync: vi.fn(async () => {}),
      search: vi.fn(),
      searchLibrary: vi.fn(async () => {}),
      setFuzzy: vi.fn(async () => {}),
      select: vi.fn((libraryType: string, libraryId: string) => {
        snapshot.selection = { libraryType, libraryId } as typeof snapshot.selection
        emit()
      }),
    },
  }
})

vi.mock('$lib/writing-zotero', async (importOriginal) => ({
  ...(await importOriginal<typeof import('$lib/writing-zotero')>()),
  writingZotero: zoteroStore,
}))

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
  zoteroStore.snapshot.bibliographySync = {
    loading: false,
    error: null,
    requested: null,
  }
  zoteroStore.snapshot.bibliographyProgress = null
  zoteroStore.snapshot.status = { state: 'available' }
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

describe('E2b-4 selected-library synchronization', () => {
  it('keeps background synchronization distinct from Refresh and targets the selection', async () => {
    answerKnownLibraries([PERSONAL, SEMINARIO])
    render(WritingZoteroTab, { props: {} })

    await fireEvent.click(await screen.findByRole('button', { name: /^Biblioteca/ }))
    await fireEvent.click(await screen.findByRole('menuitemradio', { name: 'Seminario' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Sincronizar biblioteca' }))

    expect(zoteroStore.select).toHaveBeenCalledWith('group', '6680944')
    expect(zoteroStore.requestBibliographySync).toHaveBeenCalledOnce()
    expect(zoteroStore.sync).not.toHaveBeenCalled()

    await fireEvent.click(screen.getByRole('button', { name: 'Actualizar' }))
    expect(zoteroStore.sync).toHaveBeenCalledOnce()
  })

  it('shows an honest loading state while scheduler admission is pending', async () => {
    answerKnownLibraries([PERSONAL])
    zoteroStore.snapshot.bibliographySync = {
      loading: true,
      error: null,
      requested: null,
    }

    render(WritingZoteroTab, { props: {} })

    const button = await screen.findByRole('button', { name: 'Sincronizar biblioteca' })
    expect(button).toBeDisabled()
    expect(button).toHaveAttribute('aria-busy', 'true')
    expect(screen.getByText('Solicitando la sincronización…')).toHaveAttribute('role', 'status')
  })

  it('reports only that the request was accepted until the scheduler says more', async () => {
    answerKnownLibraries([PERSONAL])
    zoteroStore.snapshot.bibliographySync = {
      loading: false,
      error: null,
      requested: {
        batchId: 'batch-bibliography',
        taskId: 'task-bibliography',
        created: true,
        requeued: false,
      },
    }

    render(WritingZoteroTab, { props: {} })

    const notice = await screen.findByText('Solicitud aceptada. Consultando el estado…')
    expect(notice).toHaveAttribute('role', 'status')
    expect(screen.queryByText(/El procesamiento continúa/)).not.toBeInTheDocument()
  })

  it('disables the synchronization with a reason while Zotero does not answer', async () => {
    answerKnownLibraries([PERSONAL])
    zoteroStore.snapshot.status = { state: 'endpoint_unavailable' }

    render(WritingZoteroTab, { props: {} })

    expect(await screen.findByRole('button', { name: 'Sincronizar biblioteca' })).toBeDisabled()
    expect(
      screen.getByText('Para sincronizar, Zotero tiene que estar respondiendo en el puerto local.')
    ).toBeInTheDocument()
  })

  describe('what the scheduler really did', () => {
    const accepted = {
      batchId: 'batch-bibliography',
      taskId: 'task-bibliography',
      created: true,
      requeued: false,
    }
    const progress = (overrides: Record<string, unknown>) => ({
      status: {
        state: 'pending',
        errorCode: null,
        errorMessage: null,
        progressDone: 0,
        progressTotal: null,
        itemsSeen: null,
        remoteTotal: null,
        newProfiles: 0,
        newExtractions: 0,
        ...overrides,
      },
      unreadable: null,
    })

    async function renderWith(overrides: Record<string, unknown>) {
      answerKnownLibraries([PERSONAL])
      zoteroStore.snapshot.bibliographySync = { loading: false, error: null, requested: accepted }
      zoteroStore.snapshot.bibliographyProgress = progress(overrides)
      render(WritingZoteroTab, { props: {} })
      return await screen.findByRole('button', { name: 'Sincronizar biblioteca' })
    }

    it('says it is synchronizing, with progress, and blocks a second press', async () => {
      const button = await renderWith({ state: 'running', progressDone: 10, progressTotal: 40 })

      expect(screen.getByText('Sincronizando… 10 de 40 obras')).toHaveAttribute('role', 'status')
      expect(button).toBeDisabled()
    })

    it('says the library is up to date when nothing new was queued', async () => {
      const button = await renderWith({ state: 'succeeded', itemsSeen: 2812 })

      expect(screen.getByText('Biblioteca al día')).toBeInTheDocument()
      expect(button).toBeEnabled()
    })

    it('reports the new works and attachments of a finished sync', async () => {
      await renderWith({ state: 'succeeded', newProfiles: 3, newExtractions: 2 })

      expect(
        screen.getByText('Sincronizada. Obras nuevas o actualizadas: 3. Adjuntos nuevos: 2.')
      ).toBeInTheDocument()
    })

    it('shows live fichas/pasajes progress with a humane ETA while the derived work runs', async () => {
      await renderWith({
        state: 'succeeded',
        newProfiles: 450,
        newExtractions: 400,
        profilesDone: 120,
        profilesTotal: 450,
        extractionsDone: 30,
        extractionsTotal: 400,
        etaMs: 720_000,
      })

      expect(
        screen.getByText('Fichas 120/450 · Pasajes 30/400 · ~12 min restantes')
      ).toHaveAttribute('role', 'status')
    })

    it('shows the same progress without an estimate while the ETA cannot be known', async () => {
      await renderWith({
        state: 'succeeded',
        newProfiles: 450,
        newExtractions: 400,
        profilesDone: 120,
        profilesTotal: 450,
        extractionsDone: 30,
        extractionsTotal: 400,
        etaMs: null,
      })

      expect(screen.getByText('Fichas 120/450 · Pasajes 30/400')).toBeInTheDocument()
    })

    it('reports the finished sync again once the derived work drains', async () => {
      await renderWith({
        state: 'succeeded',
        newProfiles: 3,
        newExtractions: 2,
        profilesDone: 3,
        profilesTotal: 3,
        extractionsDone: 2,
        extractionsTotal: 2,
        etaMs: 0,
      })

      expect(
        screen.getByText('Sincronizada. Obras nuevas o actualizadas: 3. Adjuntos nuevos: 2.')
      ).toBeInTheDocument()
    })

    it('says it is paused when Zotero stopped answering, never that it continues', async () => {
      await renderWith({ state: 'retry_wait', errorCode: 'zotero_unreachable' })

      expect(
        screen.getByText('En pausa: Zotero no responde. Se reintenta solo cuando conteste.')
      ).toBeInTheDocument()
      expect(screen.queryByText(/El procesamiento continúa/)).not.toBeInTheDocument()
    })

    it('shows a failed sync as an alert with its reason', async () => {
      await renderWith({ state: 'failed', errorMessage: 'library missing' })

      expect(screen.getByRole('alert')).toHaveTextContent(
        'La sincronización falló: library missing'
      )
    })

    it('names the parked backlog and how to unblock it instead of an endless progress line', async () => {
      await renderWith({
        state: 'succeeded',
        newProfiles: 2812,
        profilesDone: 0,
        profilesTotal: 2812,
        extractionsDone: 0,
        extractionsTotal: 0,
        profilesBlocked: 2812,
        extractionsBlocked: 0,
        profilesBlockedReason: {
          code: 'configuration_required_embedding',
          message: 'OpenRouter API key no configurada.',
        },
        extractionsBlockedReason: null,
        etaMs: null,
      })

      expect(screen.getByRole('alert')).toHaveTextContent(
        'Fichas 0/2812 · Pasajes 0/0 · 2812 en espera: configurá OpenRouter en Configuración'
      )
    })

    it('keeps the estimate for the work that can move and names what waits beside it', async () => {
      await renderWith({
        state: 'succeeded',
        newProfiles: 450,
        newExtractions: 400,
        profilesDone: 120,
        profilesTotal: 450,
        extractionsDone: 30,
        extractionsTotal: 400,
        profilesBlocked: 70,
        extractionsBlocked: 0,
        profilesBlockedReason: {
          code: 'configuration_required_embedding',
          message: 'OpenRouter API key no configurada.',
        },
        extractionsBlockedReason: null,
        etaMs: 720_000,
      })

      const line = screen.getByText(
        'Fichas 120/450 · Pasajes 30/400 · ~12 min restantes · 70 en espera: configurá OpenRouter en Configuración'
      )
      expect(line).toHaveAttribute('role', 'status')
    })

    it('words a block without a stable code with its own recorded message', async () => {
      await renderWith({
        state: 'succeeded',
        profilesDone: 0,
        profilesTotal: 5,
        profilesBlocked: 5,
        extractionsDone: 0,
        extractionsTotal: 0,
        profilesBlockedReason: { code: 'source_unstable', message: 'the source moved' },
        extractionsBlockedReason: null,
        etaMs: null,
      })

      expect(screen.getByRole('alert')).toHaveTextContent(
        'Fichas 0/5 · Pasajes 0/0 · 5 en espera: the source moved'
      )
    })

    it('names the OCR configuration when the pasajes are what waits', async () => {
      await renderWith({
        state: 'succeeded',
        profilesDone: 264,
        profilesTotal: 264,
        profilesBlocked: 0,
        extractionsDone: 0,
        extractionsTotal: 82,
        extractionsBlocked: 82,
        extractionsBlockedReason: {
          code: 'configuration_required_ocr',
          message:
            'configuration: GLM-OCR no está configurado. Andá a Configuración > OCR y cargá una API key',
        },
        etaMs: null,
      })

      expect(screen.getByRole('alert')).toHaveTextContent(
        'Fichas 264/264 · Pasajes 0/82 · 82 en espera: configurá GLM-OCR en Configuración › OCR'
      )
    })

    it('mentions both configurations when fichas and pasajes wait on different ones', async () => {
      await renderWith({
        state: 'succeeded',
        profilesDone: 0,
        profilesTotal: 264,
        profilesBlocked: 264,
        profilesBlockedReason: {
          code: 'configuration_required_embedding',
          message: 'OpenRouter API key no configurada.',
        },
        extractionsDone: 0,
        extractionsTotal: 82,
        extractionsBlocked: 82,
        extractionsBlockedReason: {
          code: 'configuration_required_ocr',
          message: 'configuration: GLM-OCR no está configurado.',
        },
        etaMs: null,
      })

      expect(screen.getByRole('alert')).toHaveTextContent(
        'Fichas 0/264 · Pasajes 0/82 · 346 en espera: configurá OpenRouter en Configuración y GLM-OCR en Configuración › OCR'
      )
    })

    it('keeps the backlog an earlier sync left behind after a newer sync reports no new work', async () => {
      await renderWith({
        state: 'succeeded',
        itemsSeen: 40,
        newProfiles: 0,
        newExtractions: 0,
        profilesDone: 0,
        profilesTotal: 264,
        profilesBlocked: 0,
        extractionsDone: 0,
        extractionsTotal: 82,
        extractionsBlocked: 82,
        extractionsBlockedReason: {
          code: 'configuration_required_ocr',
          message:
            'configuration: GLM-OCR no está configurado. Andá a Configuración > OCR y cargá una API key',
        },
        etaMs: null,
      })

      // The newer sync queued nothing new — the older backlog is still the
      // truth about the library, never «Sincronizada» over hundreds of
      // pending tasks.
      expect(screen.queryByText('Biblioteca al día')).not.toBeInTheDocument()
      expect(screen.queryByText(/Sincronizada\./)).not.toBeInTheDocument()
      expect(
        screen.getByText(
          'Fichas 0/264 · Pasajes 0/82 · 82 en espera: configurá GLM-OCR en Configuración › OCR'
        )
      ).toBeInTheDocument()
    })
  })

  it('shows scheduler admission errors next to the synchronization action', async () => {
    answerKnownLibraries([PERSONAL])
    zoteroStore.snapshot.bibliographySync = {
      loading: false,
      error: 'catalog unavailable',
      requested: null,
    }

    render(WritingZoteroTab, { props: {} })

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'No se pudo solicitar la sincronización: catalog unavailable'
    )
  })
})

/**
 * E1c-2 (UI half): the library picker. The canonical radio pattern is the
 * one BibliotecaView paints: a ToolbarMenu whose checked entry is the
 * selection — never a native select.
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
    const trigger = await screen.findByRole('button', { name: /^Biblioteca/ })
    expect(trigger).toHaveAttribute('aria-haspopup', 'menu')
    await fireEvent.click(trigger)
    const entries = await screen.findAllByRole('menuitemradio')
    expect(entries.map((entry) => entry.textContent?.trim())).toEqual([
      'Personal',
      'Seminario',
      'user/9',
    ])
  })

  it('selecting an entry selects it in the store and reconnects', async () => {
    answerKnownLibraries([PERSONAL, SEMINARIO])
    render(WritingZoteroTab, { props: {} })
    const connected = zoteroStore.connect.mock.calls.length

    await fireEvent.click(await screen.findByRole('button', { name: /^Biblioteca/ }))
    await fireEvent.click(await screen.findByRole('menuitemradio', { name: 'Seminario' }))

    expect(zoteroStore.select).toHaveBeenCalledWith('group', '6680944')
    expect(zoteroStore.connect.mock.calls.length).toBeGreaterThan(connected)
  })

  it('reflects the current selection in the trigger and checks its entry', async () => {
    answerKnownLibraries([PERSONAL, SEMINARIO])
    zoteroStore.snapshot.selection = { libraryType: 'group', libraryId: '6680944' }

    render(WritingZoteroTab, { props: {} })

    const trigger = await screen.findByRole('button', { name: /^Biblioteca/ })
    expect(trigger.textContent).toContain('Seminario')
    await fireEvent.click(trigger)
    expect(await screen.findByRole('menuitemradio', { name: 'Seminario' })).toHaveAttribute(
      'aria-checked',
      'true'
    )
  })

  it('opens the tab on the personal library when the selector is untouched', async () => {
    answerKnownLibraries([PERSONAL, SEMINARIO])
    render(WritingZoteroTab, { props: {} })

    await screen.findByText('Los orígenes')
    expect(zoteroStore.select).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: /^Biblioteca/ }).textContent).toContain('Personal')
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
    await screen.findByRole('button', { name: /^Biblioteca/ })
    await fireEvent.click(screen.getByRole('button', { name: 'Agregar biblioteca' }))
  }

  /** The type picker is the same radio menu as the library picker. */
  async function chooseType(label: 'Usuario' | 'Grupo') {
    await fireEvent.click(await screen.findByRole('button', { name: /^Tipo/ }))
    await fireEvent.click(await screen.findByRole('menuitemradio', { name: label }))
  }

  it('adds an available library and selects it', async () => {
    await openAddForm()

    await chooseType('Grupo')
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
      if (cmd === 'writing_zotero_check_library') return Promise.resolve({ status: 'unverifiable' })
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
    render(WritingZoteroTab, { props: {} })
    await screen.findByRole('button', { name: /^Biblioteca/ })
    await fireEvent.click(screen.getByRole('button', { name: 'Agregar biblioteca' }))

    await chooseType('Usuario')
    await fireEvent.input(screen.getByLabelText('ID'), { target: { value: '9' } })
    await fireEvent.click(screen.getByRole('button', { name: /^Agregar$/ }))

    expect(await screen.findByText('user/9 (sin verificar)')).toBeInTheDocument()
    expect(zoteroStore.select).toHaveBeenCalledWith('user', '9')
  })

  it('shows a not_found error inline and adds nothing', async () => {
    answerKnownLibraries([PERSONAL])
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'writing_zotero_known_libraries') return Promise.resolve([PERSONAL])
      if (cmd === 'writing_zotero_check_library') return Promise.resolve({ status: 'not_found' })
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
    render(WritingZoteroTab, { props: {} })
    await screen.findByRole('button', { name: /^Biblioteca/ })
    await fireEvent.click(screen.getByRole('button', { name: 'Agregar biblioteca' }))

    await chooseType('Grupo')
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

/**
 * The project rule: no native select anywhere. Both pickers wear the shared
 * ToolbarMenu radio pattern (the one BibliotecaView paints), and the icons
 * go through ActionIcon like everywhere else.
 */
describe('the shared menu is the only picker this tab paints', () => {
  it('renders no native select in the library picker or the add form', async () => {
    answerKnownLibraries([PERSONAL, SEMINARIO])
    const { container } = render(WritingZoteroTab, { props: {} })
    await screen.findByRole('button', { name: /^Biblioteca/ })
    await fireEvent.click(screen.getByRole('button', { name: 'Agregar biblioteca' }))
    await screen.findByRole('button', { name: /^Tipo/ })

    expect(container.querySelector('select')).toBeNull()
    expect(screen.queryByRole('combobox')).toBeNull()
  })
})

/**
 * E1c-3 (UI half): the work details (ficha) affordance. RED first: no row
 * offers a ficha yet, so the button query misses.
 */
describe('E1c-3 opening the work details (ficha)', () => {
  it('renders a row as the work and its actions only, with no stray text', async () => {
    answerKnownLibraries([PERSONAL])
    render(WritingZoteroTab, { props: {} })

    const title = await screen.findByText('Los orígenes')
    const row = title.closest('li') as HTMLElement
    // A loose text node becomes a third flex item and pushes the actions
    // away from the right edge by a different amount on every row.
    const strayText = Array.from(row.childNodes)
      .filter((node) => node.nodeType === Node.TEXT_NODE)
      .map((node) => node.textContent?.trim())
      .filter(Boolean)
    expect(strayText).toEqual([])
  })

  it('cites with a quote icon whose accessible name is still Citar', async () => {
    answerKnownLibraries([PERSONAL])
    render(WritingZoteroTab, { props: {} })

    await screen.findByText('Los orígenes')
    const cite = screen.getByRole('button', { name: 'Citar' })
    expect(cite.textContent?.trim()).toBe('')
    expect(cite.querySelector('svg')).not.toBeNull()
  })

  it('offers a ficha per row that opens without touching the selection', async () => {
    answerKnownLibraries([PERSONAL])
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'writing_zotero_known_libraries') return Promise.resolve([PERSONAL])
      if (cmd === 'writing_zotero_item_detail') return Promise.resolve({ status: 'not_in_catalog' })
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
    render(WritingZoteroTab, { props: {} })

    await screen.findByText('Los orígenes')
    const selected = zoteroStore.select.mock.calls.length

    await fireEvent.click(screen.getByRole('button', { name: 'Ver ficha' }))

    expect(await screen.findByText('Copia local sin verificar ahora')).toBeInTheDocument()
    // Opening the ficha neither reselects nor reloads the library list.
    expect(zoteroStore.select.mock.calls.length).toBe(selected)
    expect(screen.getByText('Los orígenes')).toBeInTheDocument()
  })

  it('closes the ficha back to the list and keeps citing independent', async () => {
    answerKnownLibraries([PERSONAL])
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'writing_zotero_known_libraries') return Promise.resolve([PERSONAL])
      if (cmd === 'writing_zotero_item_detail') return Promise.resolve({ status: 'not_in_catalog' })
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
    const oncite = vi.fn(() => 'citation-1')
    render(WritingZoteroTab, { props: { oncite } })

    await screen.findByText('Los orígenes')
    await fireEvent.click(screen.getByRole('button', { name: 'Ver ficha' }))
    await screen.findByText('Copia local sin verificar ahora')

    await fireEvent.click(screen.getByRole('button', { name: 'Volver a referencias' }))
    expect(screen.queryByText('Copia local sin verificar ahora')).not.toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Citar' }))
    expect(oncite).toHaveBeenCalledOnce()
  })
})

describe('B2 search by meaning in the Zotero tab', () => {
  type Entry = (typeof zoteroStore.snapshot.entries)[number] & {
    semantic?: true
    content?: { kind: 'exact' | 'approximate'; terms: string[] }
  }
  type Snapshot = Omit<typeof zoteroStore.snapshot, 'entries'> & {
    semanticStatus: string
    entries: Entry[]
  }
  const snapshot = zoteroStore.snapshot as unknown as Snapshot
  const original = zoteroStore.snapshot.entries[0]!

  afterEach(() => {
    snapshot.semanticStatus = 'idle'
    snapshot.query = ''
    snapshot.entries = [original]
  })

  it('says the library is not synced when the meaning search had nothing to read', async () => {
    answerKnownLibraries([PERSONAL])
    snapshot.query = 'revoluciones'
    snapshot.semanticStatus = 'not_synced'

    render(WritingZoteroTab, { props: {} })

    expect(await screen.findByText(/todavía no está sincronizada en EntropIA/i)).toBeInTheDocument()
  })

  it('says only the text match ran when there is no active embedding generation', async () => {
    answerKnownLibraries([PERSONAL])
    snapshot.query = 'revoluciones'
    snapshot.semanticStatus = 'lexical_only'

    render(WritingZoteroTab, { props: {} })

    expect(await screen.findByText(/sin espacio semántico activo/i)).toBeInTheDocument()
  })

  it('says the meaning search failed without hiding the text results', async () => {
    answerKnownLibraries([PERSONAL])
    snapshot.query = 'revoluciones'
    snapshot.semanticStatus = 'failed'

    render(WritingZoteroTab, { props: {} })

    expect(await screen.findByText(/falló la búsqueda por significado/i)).toBeInTheDocument()
    expect(screen.getByText('Los orígenes')).toBeInTheDocument()
  })

  it('stays silent when the meaning search worked or nothing was searched', async () => {
    answerKnownLibraries([PERSONAL])
    snapshot.query = 'revoluciones'
    snapshot.semanticStatus = 'ok'

    render(WritingZoteroTab, { props: {} })

    await screen.findByText('Los orígenes')
    expect(screen.queryByText(/sincronizada en EntropIA/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/sin espacio semántico/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/falló la búsqueda por significado/i)).not.toBeInTheDocument()
  })

  it('tags works found only by meaning', async () => {
    answerKnownLibraries([PERSONAL])
    snapshot.query = 'revoluciones'
    snapshot.semanticStatus = 'ok'
    snapshot.entries = [
      original,
      {
        ...original,
        key: 'SEM1',
        title: 'Otra obra',
        csl_json: JSON.stringify({ id: 'sem1', title: 'Otra obra' }),
        semantic: true,
      },
    ]

    render(WritingZoteroTab, { props: {} })

    expect(await screen.findByText('Otra obra')).toBeInTheDocument()
    expect(screen.getAllByText('Por significado')).toHaveLength(1)
  })
  it('tags works found by what their passages say, with how the words matched', async () => {
    answerKnownLibraries([PERSONAL])
    snapshot.query = 'plan federal'
    snapshot.semanticStatus = 'ok'
    snapshot.entries = [
      original,
      {
        ...original,
        key: 'CON1',
        title: 'La producción del espacio',
        csl_json: JSON.stringify({ id: 'con1', title: 'La producción del espacio' }),
        content: { kind: 'exact', terms: ['plan', 'federal'] },
      },
      {
        ...original,
        key: 'CON2',
        title: 'Otra obra',
        csl_json: JSON.stringify({ id: 'con2', title: 'Otra obra' }),
        content: { kind: 'approximate', terms: ['crocitto'] },
      },
    ]

    render(WritingZoteroTab, { props: {} })

    expect(await screen.findByText('La producción del espacio')).toBeInTheDocument()
    expect(screen.getAllByText('Por contenido')).toHaveLength(2)
    expect(screen.getByText('Exacto: plan, federal')).toBeInTheDocument()
    expect(screen.getByText('Aproximado: crocitto')).toBeInTheDocument()
  })

  it('offers the shared approximate-matching switch and hands the choice to the store', async () => {
    answerKnownLibraries([PERSONAL])
    snapshot.query = ''
    snapshot.entries = [original]

    render(WritingZoteroTab, { props: {} })

    const toggle = await screen.findByRole('checkbox', {
      name: 'Incluir coincidencias aproximadas',
    })
    expect(toggle).toBeChecked()
    await fireEvent.click(toggle)
    expect(zoteroStore.setFuzzy).toHaveBeenCalledWith(false)
  })

  it('cannot cite a catalog-only work that has no CSL to snapshot', async () => {
    answerKnownLibraries([PERSONAL])
    snapshot.query = 'dignidad'
    snapshot.semanticStatus = 'ok'
    snapshot.entries = [
      {
        ...original,
        key: 'NOCSL',
        title: 'Obra sin CSL',
        csl_json: '',
        semantic: true,
      },
    ]

    render(WritingZoteroTab, { props: { oncite: vi.fn(() => 'citation-1') } })

    expect(await screen.findByText('Obra sin CSL')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Citar/i })).toBeDisabled()
    expect(screen.getByRole('button', { name: /Ver ficha|Ficha|Detalles/i })).toBeEnabled()
  })
})
