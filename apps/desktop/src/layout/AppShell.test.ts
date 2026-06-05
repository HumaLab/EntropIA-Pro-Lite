import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import AppShellHost from './__fixtures__/AppShellHost.svelte'
import { locale } from '$lib/i18n'

type EventListenerCallback = (event: { payload: unknown }) => void

const { invokeMock, listenMock, navigationStore, storeRef } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn<(eventName: string, callback: EventListenerCallback) => Promise<() => void>>(
    () => Promise.resolve(vi.fn()),
  ),
  navigationStore: {
    subscribe(run: (value: unknown) => void) {
      run({
        history: [{ name: 'collections' }],
        current: { name: 'collections' },
        canGoBack: false,
        breadcrumb: ['Collections'],
      })
      return () => {}
    },
  },
  storeRef: {
    current: {
      collections: {
        findAll: vi.fn().mockResolvedValue([]),
        countItems: vi.fn().mockResolvedValue(0),
        findById: vi.fn().mockResolvedValue(null),
      },
      assets: { findByItem: vi.fn().mockResolvedValue([]) },
      items: {
        searchGlobal: vi.fn().mockResolvedValue([]),
        findByCollection: vi.fn().mockResolvedValue([]),
      },
    },
  },
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}))

vi.mock('$lib/navigation', () => ({
  navigation: {
    subscribe: navigationStore.subscribe,
    navigate: vi.fn(),
    back: vi.fn(),
  },
}))

vi.mock('$lib/db', () => ({
  getStore: () => storeRef.current,
}))

describe('AppShell', () => {
  beforeEach(() => {
    locale.set('es')
    invokeMock.mockReset().mockImplementation((command: string) => {
      if (command === 'deps_get_cached_statuses') {
        return Promise.resolve([])
      }

      if (command === 'runtime_get_status') {
        return Promise.resolve({
          state: 'healthy',
          packVersion: null,
          repairNeeded: false,
          repairAvailable: false,
          summary: 'Runtime listo',
          blockedCapabilities: [],
          details: [],
          guidance: [],
          bootstrapEligible: false,
          bootstrapRequired: false,
          activeOperation: null,
        })
      }

      return Promise.resolve(undefined)
    })
    listenMock.mockClear().mockImplementation(() => Promise.resolve(vi.fn()))
    storeRef.current.items.searchGlobal.mockClear()
    storeRef.current.items.findByCollection.mockClear()
    storeRef.current.collections.findAll.mockClear()
    storeRef.current.collections.countItems.mockClear()
    storeRef.current.assets.findByItem.mockClear()
    storeRef.current.collections.findById.mockClear()
  })

  it('renders the app frame, visible footer actions, and projected content', () => {
    render(AppShellHost)

    expect(screen.getByRole('navigation', { name: 'Breadcrumb' })).toBeInTheDocument()
    expect(screen.getByTestId('app-shell-child')).toHaveTextContent('Contenido de prueba')
    expect(screen.getByText('EntropIA Pro')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'GitHub' })).toBeInTheDocument()
    expect(screen.getByText('Desarrollado por')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Colapsar panel lateral' })).toHaveAttribute(
      'aria-expanded',
      'true',
    )
    expect(screen.getByRole('button', { name: 'Nueva colección' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Filtrar colecciones' })).toHaveAttribute(
      'aria-expanded',
      'false',
    )
  })

  it('keeps the entropic constellation visible behind workspace surfaces', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'AppShell.svelte'), 'utf-8')

    expect(source).toContain('<EntropicConstellation />')
    expect(source).toContain('color-mix(in srgb, var(--color-bg) 34%, transparent)')
    expect(source).toContain('color-mix(in srgb, var(--color-bg) 24%, transparent)')
  })

  it('opens external links through the desktop bridge', async () => {
    render(AppShellHost)

    await fireEvent.click(screen.getByRole('link', { name: 'GitHub' }))
    expect(invokeMock).toHaveBeenCalledWith('open_external_url', {
      url: 'https://github.com/hlabrepo/EntropIA-Pro',
    })

    await fireEvent.click(screen.getByRole('link', { name: 'HLab' }))
    expect(invokeMock).toHaveBeenCalledWith('open_external_url', {
      url: 'https://hlab.com.ar/',
    })
  })

  it('reacts to locale changes in footer copy', async () => {
    render(AppShellHost)

    locale.set('en')

    expect(await screen.findByText('Archive, OCR, and assisted analysis.')).toBeInTheDocument()
    expect(screen.getByText('Developed by')).toBeInTheDocument()
  })

  it('boots without awaiting a fresh dependency probe and updates from completion events', async () => {
    let depsCompleteHandler: ((event: { payload: { results: Array<{ id: string; status: { type: string } }> } }) => void) | undefined

    listenMock.mockImplementation((eventName: string, callback: EventListenerCallback) => {
      if (eventName === 'deps://complete') {
        depsCompleteHandler = callback as typeof depsCompleteHandler
      }

      return Promise.resolve(vi.fn())
    })

    render(AppShellHost)

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('deps_get_cached_statuses')
    })
    expect(invokeMock).not.toHaveBeenCalledWith('deps_check_all')
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()

    depsCompleteHandler?.({
      payload: {
        results: [
          { id: 'Python', status: { type: 'missing' } },
          { id: 'Fastembed', status: { type: 'installed' } },
          { id: 'PaddlePaddle', status: { type: 'missing' } },
          { id: 'PaddleOcr', status: { type: 'installed' } },
        ],
      },
    })

    expect(await screen.findByText('Dependencias de IA pendientes')).toBeInTheDocument()
    expect(
      screen.getByText('Se necesitan Python y paquetes para OCR/transcripción; embeddings usan OpenRouter.'),
    ).toBeInTheDocument()
  })

  it('shows runtime health alerts when the managed runtime is damaged', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'deps_get_cached_statuses') {
        return Promise.resolve([])
      }

      if (command === 'runtime_get_status') {
        return Promise.resolve({
          state: 'damaged',
          packVersion: '2026.05.0',
          repairNeeded: true,
          repairAvailable: true,
          summary: 'Runtime dañado',
          blockedCapabilities: ['ocr', 'transcription'],
          details: ['Checksum inválido'],
          guidance: ['Ejecutá la reparación del runtime desde Ajustes > Dependencias.'],
          bootstrapEligible: true,
          bootstrapRequired: true,
          activeOperation: null,
        })
      }

      return Promise.resolve(undefined)
    })

    render(AppShellHost)

    expect(await screen.findByRole('alert')).toHaveTextContent('Runtime dañado')
    expect(screen.getByRole('button', { name: 'Reparar runtime →' })).toBeInTheDocument()
    expect(screen.getByText(/ocr, transcription/i)).toBeInTheDocument()
  })

  it('shows fixture runtime alerts without repair action', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'deps_get_cached_statuses') {
        return Promise.resolve([
          { id: 'Python', status: { type: 'missing' } },
          { id: 'Fastembed', status: { type: 'missing' } },
          { id: 'PaddlePaddle', status: { type: 'missing' } },
          { id: 'PaddleOcr', status: { type: 'missing' } },
        ])
      }

      if (command === 'runtime_get_status') {
        return Promise.resolve({
          state: 'fixture',
          packVersion: '2026.05.0',
          repairNeeded: false,
          repairAvailable: false,
          summary: 'Runtime de desarrollo detectado para linux-x86_64: faltan payloads externos de release',
          blockedCapabilities: ['ocr', 'transcription', 'nlp'],
          details: ['La app 0.0.10 arrancó correctamente, pero este runtime-pack todavía está en modo fixture/dev (app_version declarada: 0.0.10).'],
          guidance: ['Esto no indica una caída: la UI puede abrir, pero OCR/NLP/transcripción quedan bloqueados hasta inyectar los payloads de release.'],
          bootstrapEligible: false,
          bootstrapRequired: true,
          activeOperation: null,
        })
      }

      return Promise.resolve(undefined)
    })

    render(AppShellHost)

    expect(
      await screen.findByText(
        'Runtime de desarrollo detectado para linux-x86_64: faltan payloads externos de release',
      ),
    ).toBeInTheDocument()
    expect(screen.getByText(/app no se cay/i)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Reparar runtime →' })).not.toBeInTheDocument()
  })

  it('does not show a global runtime alert when deps are installed and only fixture release packaging is pending', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'deps_get_cached_statuses') {
        return Promise.resolve([
          { id: 'Python', status: { type: 'installed' } },
          { id: 'Fastembed', status: { type: 'installed' } },
          { id: 'PaddlePaddle', status: { type: 'installed' } },
          { id: 'PaddleOcr', status: { type: 'installed' } },
        ])
      }

      if (command === 'runtime_get_status') {
        return Promise.resolve({
          state: 'fixture',
          packVersion: '2026.05.0',
          repairNeeded: false,
          repairAvailable: false,
          summary: 'Runtime de desarrollo detectado para linux-x86_64: faltan payloads externos de release',
          blockedCapabilities: ['ocr', 'transcription', 'nlp'],
          details: ['payloads offline pendientes'],
          guidance: ['Inyectar payloads externos antes de distribuir offline'],
          bootstrapEligible: false,
          bootstrapRequired: true,
          activeOperation: null,
        })
      }

      return Promise.resolve(undefined)
    })

    render(AppShellHost)

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('deps_get_cached_statuses')
      expect(invokeMock).toHaveBeenCalledWith('runtime_get_status')
    })
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('shows a global runtime alert when release source wiring is blocked', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'deps_get_cached_statuses') {
        return Promise.resolve([
          { id: 'Python', status: { type: 'installed' } },
          { id: 'Fastembed', status: { type: 'installed' } },
          { id: 'PaddlePaddle', status: { type: 'installed' } },
          { id: 'PaddleOcr', status: { type: 'installed' } },
        ])
      }

      if (command === 'runtime_get_status') {
        return Promise.resolve({
          state: 'blocked_source_unavailable',
          packVersion: '2026.05.0',
          repairNeeded: false,
          repairAvailable: false,
          summary: 'No hay una fuente confiable disponible para bootstrap',
          blockedCapabilities: ['ocr', 'transcription', 'nlp'],
          details: ['source pendiente'],
          guidance: ['Reintentá cuando exista una fuente confiable'],
          bootstrapEligible: false,
          bootstrapRequired: true,
          activeOperation: null,
        })
      }

      return Promise.resolve(undefined)
    })

    render(AppShellHost)

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'No hay una fuente confiable disponible para bootstrap',
    )
    expect(screen.getByText(/ocr, transcription, nlp/i)).toBeInTheDocument()
  })

})
