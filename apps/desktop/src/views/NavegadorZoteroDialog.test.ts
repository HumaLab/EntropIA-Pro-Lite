/** @vitest-environment jsdom */

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import type { ZoteroCopy } from '$lib/navegador-zotero'
import NavegadorZoteroDialog from './NavegadorZoteroDialog.svelte'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const row = (patch: Partial<ZoteroCopy> = {}): ZoteroCopy => ({
  id: 'k1',
  sourceId: 's1',
  captureId: null,
  libraryType: 'user',
  libraryId: '0',
  libraryName: null,
  state: 'queued',
  itemKey: null,
  detail: null,
  errorCode: null,
  errorMessage: null,
  attempts: 0,
  createdAt: 1,
  updatedAt: 1,
  ...patch,
})

const KNOWN = [
  { libraryType: 'user', libraryId: '0', name: null, source: 'personal' },
  { libraryType: 'group', libraryId: '6680944', name: 'prueba', source: 'catalog' },
]

let requested: unknown[] = []
let answers: { request: () => unknown; run: () => unknown }
let live: { reachable: boolean; libraries: unknown[] }
let statusFor: (args: { library: { libraryId: string } }) => unknown
let opened: unknown[] = []
let statusCalls: unknown[] = []

const ABSENT = {
  state: 'absent',
  source: 'none',
  itemKey: null,
  pdf: 'none',
  pendingFields: [],
  keptFields: [],
  pdfCapture: null,
  canComplete: false,
}
const PRESENT = {
  state: 'present',
  source: 'zotero',
  itemKey: 'OLDKEY22',
  pdf: 'none',
  pendingFields: [],
  keptFields: [],
  pdfCapture: null,
  canComplete: false,
}

const onclose = vi.fn()

function open(props: Record<string, unknown> = {}) {
  return render(NavegadorZoteroDialog, {
    props: { source: { id: 's1', title: 'La cuestión social' }, onclose, ...props },
  })
}

beforeEach(() => {
  locale.set('es')
  onclose.mockReset()
  requested = []
  opened = []
  statusCalls = []
  live = {
    reachable: true,
    libraries: [
      { libraryType: 'user', libraryId: '0', name: null },
      { libraryType: 'group', libraryId: '6680944', name: 'prueba' },
    ],
  }
  statusFor = () => ABSENT
  localStorage.clear()
  answers = {
    request: () => row(),
    run: () => ({ reachable: true, copies: [row({ state: 'copied', itemKey: 'ABCD2345' })] }),
  }
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
    if (command === 'writing_zotero_known_libraries') return KNOWN
    if (command === 'navegador_zotero_libraries') return live
    if (command === 'navegador_zotero_status') {
      statusCalls.push(args)
      return statusFor(args as { library: { libraryId: string } })
    }
    if (command === 'writing_zotero_open_item') {
      opened.push(args)
      return undefined
    }
    if (command === 'navegador_zotero_copy_request') {
      requested.push(args)
      return answers.request()
    }
    if (command === 'navegador_zotero_copy_run') return answers.run()
    throw new Error(`unexpected command ${command}`)
  })
})

describe('live libraries', () => {
  it('lists the groups Zotero reports, not only what the archive knows', async () => {
    open()
    expect(await screen.findByRole('radio', { name: 'prueba' })).toBeTruthy()
    expect(vi.mocked(invoke).mock.calls.map((call) => call[0])).not.toContain(
      'writing_zotero_known_libraries'
    )
  })

  it('falls back to the known libraries when Zotero does not answer, and says so', async () => {
    live = { reachable: false, libraries: [] }
    open()
    expect(await screen.findByRole('radio', { name: 'prueba' })).toBeTruthy()
    expect(screen.getByText(/muestran las bibliotecas conocidas/)).toBeTruthy()
  })

  it('falls back too when asking Zotero for the list fails', async () => {
    const inner = vi.mocked(invoke).getMockImplementation()!
    vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
      if (command === 'navegador_zotero_libraries') throw new Error('boom')
      return inner(command, args as never)
    })
    open()
    expect(await screen.findByRole('radio', { name: 'prueba' })).toBeTruthy()
    expect(screen.getByText(/muestran las bibliotecas conocidas/)).toBeTruthy()
  })
})

describe('which PDF goes along', () => {
  it('says which saved PDF will be attached, by the day it was saved', async () => {
    statusFor = () => ({
      ...ABSENT,
      pdfCapture: { id: 'c9', savedAt: '2026-10-02T09:30:00Z' },
    })
    open()
    expect(await screen.findByText(/Se adjuntará el PDF guardado el 2026-10-02/)).toBeTruthy()
  })

  it('says when none will be attached', async () => {
    open()
    expect(await screen.findByText(/no tiene un PDF guardado/)).toBeTruthy()
  })

  it('says nothing about a PDF while it does not know', async () => {
    statusFor = () => {
      throw new Error('boom')
    }
    open()
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    expect(screen.queryByText(/PDF guardado/)).toBeNull()
  })
})

describe('completing an item that is already there', () => {
  it('offers to complete it instead of only opening it when there is something to do', async () => {
    statusFor = () => ({
      ...PRESENT,
      pdf: 'parent_exists',
      pdfCapture: { id: 'c9', savedAt: '2026-10-02T09:30:00Z' },
      canComplete: true,
    })
    answers.run = () => ({
      reachable: true,
      copies: [row({ state: 'linked', itemKey: 'OLDKEY22' })],
    })
    open()
    await screen.findByText(/Ya está en Zotero/)
    expect(screen.queryByRole('button', { name: 'Abrir en Zotero' })).toBeNull()
    await fireEvent.click(screen.getByRole('button', { name: 'Completar en Zotero' }))
    await screen.findByText(/Ya estaba en «Mi biblioteca»/)
    expect(requested).toHaveLength(1)
    expect(opened).toHaveLength(0)
  })

  it('still just opens it when there is nothing to complete', async () => {
    statusFor = () => PRESENT
    open()
    await screen.findByText(/Ya está en Zotero/)
    expect(screen.getByRole('button', { name: 'Abrir en Zotero' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Completar en Zotero' })).toBeNull()
  })
})

describe('already in Zotero', () => {
  it('checks the source in the chosen library by id as soon as the dialog opens', async () => {
    open({ capture: { id: 'c1', title: 'Informe' } })
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    await waitFor(() => expect(statusCalls).toHaveLength(1))
    expect(statusCalls[0]).toEqual({
      sourceId: 's1',
      captureId: 'c1',
      library: { libraryType: 'user', libraryId: '0', libraryName: null },
    })
  })

  it('says so up front and offers to open it instead of copying', async () => {
    statusFor = () => PRESENT
    open()
    await screen.findByText(/Ya está en Zotero \(«Mi biblioteca»\)/)
    expect(screen.queryByRole('button', { name: 'Copiar' })).toBeNull()
    await fireEvent.click(screen.getByRole('button', { name: 'Abrir en Zotero' }))
    await waitFor(() =>
      expect(opened).toEqual([{ libraryType: 'user', libraryId: '0', itemKey: 'OLDKEY22' }])
    )
    expect(requested).toHaveLength(0)
    await waitFor(() => expect(onclose).toHaveBeenCalled())
  })

  it('lists the fields it could not fill or update and the ones the person edited', async () => {
    statusFor = () => ({
      ...PRESENT,
      pendingFields: ['accessDate', 'websiteTitle'],
      keptFields: ['title'],
    })
    open()
    await screen.findByText(/Ya está en Zotero/)
    expect(screen.getByText(/fecha de acceso, sitio web/)).toBeTruthy()
    expect(screen.getByText(/editaste en Zotero.*título/)).toBeTruthy()
  })

  it('says a PDF cannot join a page that already exists', async () => {
    statusFor = () => ({ ...PRESENT, pdf: 'parent_exists' })
    open({ capture: { id: 'c1', title: 'Informe' } })
    await screen.findByText(/Ya está en Zotero/)
    expect(screen.getByText(/El PDF no se adjuntó/)).toBeTruthy()
  })

  it('checks again when another library is chosen', async () => {
    statusFor = (args) => (args.library.libraryId === '0' ? PRESENT : ABSENT)
    open()
    await screen.findByText(/Ya está en Zotero/)
    await fireEvent.click(screen.getByRole('radio', { name: 'prueba' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Copiar' })).toBeTruthy())
    expect(statusCalls).toHaveLength(2)
    expect(screen.queryByText(/Ya está en Zotero/)).toBeNull()
  })

  it('says when the answer comes from our own record because Zotero is closed', async () => {
    statusFor = () => ({ ...PRESENT, source: 'record', pdf: 'unknown' })
    open()
    await screen.findByText(/Ya está en Zotero/)
    expect(screen.getByText(/sale del registro de EntropIA/)).toBeTruthy()
  })

  it('a failing check never blocks copying', async () => {
    statusFor = () => {
      throw new Error('zotero_api_disabled: off')
    }
    open()
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    await waitFor(() => expect(statusCalls).toHaveLength(1))
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await waitFor(() => expect(requested).toHaveLength(1))
  })
})

describe('NavegadorZoteroDialog', () => {
  it('offers the personal library, preselected, and the groups by name', async () => {
    open()
    const personal = await screen.findByRole('radio', { name: 'Mi biblioteca' })
    expect((personal as HTMLInputElement).checked).toBe(true)
    expect(screen.getByRole('radio', { name: 'prueba' })).toBeTruthy()
  })

  it('tells the person that a closed Zotero makes the copy wait', async () => {
    open()
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    expect(screen.getByText(/espera en cola/i)).toBeTruthy()
  })

  it('copies the page to the personal library and says it is done', async () => {
    open()
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await screen.findByText(/Copiado a «Mi biblioteca»/)
    expect(requested).toEqual([
      {
        sourceId: 's1',
        captureId: null,
        library: { libraryType: 'user', libraryId: '0', libraryName: null },
      },
    ])
    expect(vi.mocked(invoke).mock.calls.map((call) => call[0])).toContain(
      'navegador_zotero_copy_run'
    )
  })

  it('sends the chosen group with its name', async () => {
    open()
    await fireEvent.click(await screen.findByRole('radio', { name: 'prueba' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await waitFor(() => expect(requested).toHaveLength(1))
    expect(requested[0]).toMatchObject({
      library: { libraryType: 'group', libraryId: '6680944', libraryName: 'prueba' },
    })
  })

  it('a PDF capture goes along with its page', async () => {
    statusFor = () => ({
      ...ABSENT,
      pdfCapture: { id: 'c1', savedAt: '2026-10-02T09:30:00Z' },
    })
    open({ capture: { id: 'c1', title: 'Informe' } })
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    expect(screen.getByText(/con el PDF «Informe» adjunto/)).toBeTruthy()
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await waitFor(() => expect(requested).toHaveLength(1))
    expect(requested[0]).toMatchObject({ captureId: 'c1' })
  })

  it('a closed Zotero leaves the copy in the queue and says so', async () => {
    answers.run = () => ({ reachable: false, copies: [row({ state: 'waiting' })] })
    open()
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await screen.findByText(/queda en cola/)
  })

  it('an item that was already there is reported as linked, not as copied', async () => {
    answers.run = () => ({
      reachable: true,
      copies: [row({ state: 'linked', itemKey: 'OLDKEY22' })],
    })
    open()
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await screen.findByText(/Ya estaba en «Mi biblioteca»/)
  })

  it('a failed run shows the reason in words', async () => {
    answers.run = () => ({
      reachable: true,
      copies: [row({ state: 'failed', errorCode: 'library_unavailable', errorMessage: 'x' })],
    })
    open()
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await screen.findByText(/no está disponible para escribir/i)
  })

  it('a copy that could not even be queued stays on the choice with the reason', async () => {
    answers.request = () => {
      throw new Error('not_found: there is no such source')
    }
    open()
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await screen.findByText(/ya no existe/i)
    expect(screen.getByRole('button', { name: 'Copiar' })).toBeTruthy()
  })

  it('a failing run does not lose the queued copy: it reads as waiting', async () => {
    answers.run = () => {
      throw new Error('boom')
    }
    answers.request = () => row({ state: 'queued' })
    open()
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    await fireEvent.click(screen.getByRole('button', { name: 'Copiar' }))
    await screen.findByText(/queda en cola/)
  })

  it('speaks English when asked', async () => {
    locale.set('en')
    open()
    await screen.findByRole('radio', { name: 'My library' })
    expect(screen.getByRole('button', { name: 'Copy' })).toBeTruthy()
  })
})
