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
  localStorage.clear()
  answers = {
    request: () => row(),
    run: () => ({ reachable: true, copies: [row({ state: 'copied', itemKey: 'ABCD2345' })] }),
  }
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation(async (command: string, args?: unknown) => {
    if (command === 'writing_zotero_known_libraries') return KNOWN
    if (command === 'navegador_zotero_copy_request') {
      requested.push(args)
      return answers.request()
    }
    if (command === 'navegador_zotero_copy_run') return answers.run()
    throw new Error(`unexpected command ${command}`)
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
    open({ capture: { id: 'c1', title: 'Informe' } })
    await screen.findByRole('radio', { name: 'Mi biblioteca' })
    expect(screen.getByText(/PDF/)).toBeTruthy()
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
