import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import AppShellHost from './__fixtures__/AppShellHost.svelte'
import { mountLog as workPaneMountLog } from './__mocks__/MockWorkPane.svelte'
import { LOCAL_ML } from '$lib/capabilities'
import { locale } from '$lib/i18n'
import { PRODUCT_NAME_BADGE } from '$lib/product'
import { MIN_PANE_PX } from '$lib/split-ratio'
import { workspace } from '$lib/workspace'

type EventListenerCallback = (event: { payload: unknown }) => void

const { invokeMock, listenMock, storeRef } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn<(eventName: string, callback: EventListenerCallback) => Promise<() => void>>(
    () => Promise.resolve(vi.fn())
  ),
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

// AppShell now renders WorkPane directly (Task 2.3) instead of projecting a
// `children` snippet, so the real component would pull in CollectionsView /
// HomeView and their own store-heavy dependencies here. That coverage
// already belongs to WorkPane.test.ts (Task 2.2) and each view's own test
// (Task 1.5); this file only owns AppShell's chrome (sidebar, footer,
// Ctrl+B, deps/runtime banners), so WorkPane is stubbed the same way the
// fixture's `children` snippet used to stand in for "some content".
vi.mock('./WorkPane.svelte', async () => {
  const { default: MockWorkPane } = await import('./__mocks__/MockWorkPane.svelte')
  return { default: MockWorkPane }
})

vi.mock('$lib/db', () => ({
  getStore: () => storeRef.current,
}))

describe('AppShell', () => {
  beforeEach(() => {
    locale.set('es')
    // `workspace` is a real module singleton (Task 2.3: AppShell now derives
    // its chrome from it directly, not a frozen `activeNavigation` capture),
    // so it outlives each test — reset it to one tab on Collections, the
    // suite's previous default, before every run.
    while (workspace.tabs.length > 1) {
      workspace.closeTab(workspace.tabs.at(-1)!.id)
    }
    workspace.activeNavigation.resetToPath([{ name: 'collections' }])
    workPaneMountLog.length = 0
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

    // The breadcrumb moved out of TopBar into each pane's own location strip
    // (Task 2.4, WorkPane.svelte) — WorkPane is mocked in this file (see
    // above), so it belongs to WorkPane.test.ts, not here.
    expect(screen.getByTestId('app-shell-child')).toHaveTextContent('Contenido de prueba')
    expect(
      within(screen.getByRole('contentinfo')).getByText(PRODUCT_NAME_BADGE)
    ).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'GitHub' })).toBeInTheDocument()
    expect(screen.getByText('Desarrollado por')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Colapsar panel (Ctrl+B)' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Nueva colección' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Filtrar colecciones' })).toBeInTheDocument()
  })

  it('shows the document explorer in the sidebar from the collections root view', async () => {
    render(AppShellHost)

    expect(
      await screen.findByRole('complementary', { name: 'Explorador de documentos' })
    ).toBeInTheDocument()
    expect(screen.queryByText('Abrí una colección para ver el explorador')).not.toBeInTheDocument()
  })

  it.each(['db-browser', 'rag-chat', 'settings'] as const)(
    'hides the whole sidebar on the %s root section so it reserves no width',
    async (viewName) => {
      workspace.activeNavigation.resetToPath([{ name: viewName }])

      render(AppShellHost)

      await waitFor(() => {
        expect(screen.getByTestId('app-shell-child')).toBeInTheDocument()
      })

      expect(screen.queryByRole('complementary', { name: 'Panel lateral' })).not.toBeInTheDocument()
      expect(
        screen.queryByRole('complementary', { name: 'Explorador de documentos' })
      ).not.toBeInTheDocument()
      expect(
        screen.queryByRole('button', { name: 'Colapsar panel (Ctrl+B)' })
      ).not.toBeInTheDocument()
    }
  )

  it('keeps the sidebar on a collection and on an item view', async () => {
    workspace.activeNavigation.resetToPath([
      { name: 'collection', id: 'col-1', collectionName: 'Col 1' },
    ])
    const collectionRender = render(AppShellHost)

    expect(
      await screen.findByRole('complementary', { name: 'Explorador de documentos' })
    ).toBeInTheDocument()

    collectionRender.unmount()
    workspace.activeNavigation.resetToPath([
      {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Col 1',
        itemId: 'item-1',
        itemTitle: 'Item 1',
      },
    ])
    render(AppShellHost)

    expect(
      await screen.findByRole('complementary', { name: 'Explorador de documentos' })
    ).toBeInTheDocument()
  })

  it('remounts WorkPane keyed by the active tab, and updates its own chrome, when the active tab switches', async () => {
    const tabAId = workspace.activeTabId

    render(AppShellHost)

    expect(screen.getByTestId('app-shell-child')).toHaveAttribute('data-pane-id', tabAId)
    expect(
      await screen.findByRole('complementary', { name: 'Explorador de documentos' })
    ).toBeInTheDocument()
    expect(workPaneMountLog).toEqual([tabAId])

    // A tab opens on a root section outside the Collections hierarchy, so
    // AppShell's own sidebar-visibility chrome should flip too — not just
    // the pane content.
    const tabBId = workspace.openTab({ name: 'settings' })!

    await waitFor(() => {
      expect(screen.getByTestId('app-shell-child')).toHaveAttribute('data-pane-id', tabBId)
    })
    // A new mountLog entry (not just the same id repeated) proves the
    // `{#key wsSnapshot.activeTabId}` block actually destroyed and recreated
    // WorkPane, rather than re-propping the same instance in place.
    expect(workPaneMountLog).toEqual([tabAId, tabBId])
    expect(
      screen.queryByRole('complementary', { name: 'Explorador de documentos' })
    ).not.toBeInTheDocument()
  })

  it('updates its own chrome when the active tab navigates without switching tabs', async () => {
    render(AppShellHost)

    expect(
      await screen.findByRole('complementary', { name: 'Explorador de documentos' })
    ).toBeInTheDocument()

    workspace.activeNavigation.navigate({ name: 'settings' })

    await waitFor(() => {
      expect(
        screen.queryByRole('complementary', { name: 'Explorador de documentos' })
      ).not.toBeInTheDocument()
    })
  })

  it('keeps the entropic constellation visible behind workspace surfaces', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'AppShell.svelte'), 'utf-8')

    expect(source).toContain(
      "<EntropicConstellation animated={$activeNav.current.name === 'home'} />"
    )
    expect(source).toContain('color-mix(in srgb, var(--surface-app) 72%, transparent)')
    expect(source).toContain('color-mix(in srgb, var(--surface-app) 42%, transparent)')
  })

  it('lifts the workspace veils on Inicio so the animated constellation shows', () => {
    // .workspace (72 %) and .content (42 %) together cover ~84 % of the canvas:
    // enough to make even hlab-strength points invisible. On Inicio, the only
    // view that animates the field, both veils step aside.
    const source = readFileSync(resolve(import.meta.dirname, 'AppShell.svelte'), 'utf-8')

    expect(source).toMatch(
      /<div\s+class="workspace"\s+class:workspace--home=\{\$activeNav\.current\.name === 'home'\}/
    )
    expect(source).toMatch(/class:content--home=\{\$activeNav\.current\.name === 'home'\}/)
    expect(source).toMatch(/\.workspace--home\s*\{\s*background:\s*transparent;/)
    expect(source).toMatch(/\.content--home\s*\{\s*background:\s*transparent;/)
  })

  it('does not add extra status bar clearance to main content', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'AppShell.svelte'), 'utf-8')

    expect(source).toMatch(/\.shell\s*\{\s*--statusbar-height: 30px;/)
    expect(source).toMatch(
      /<main\s+class="content"\s+class:content--item=\{\$activeNav\.current\.name === 'item'\}/
    )
    expect(source).toMatch(/\.content\s*\{[\s\S]*?padding: 0 var\(--space-5\);/)
    expect(source).not.toContain(
      'padding-block-end: calc(var(--statusbar-height) + var(--space-4) / 10);'
    )
    expect(source).toMatch(/\.content--item\s*\{\s*padding-block-end: 0;/)
    expect(source).toMatch(/\.statusbar\s*\{[\s\S]*?height: var\(--statusbar-height\);/)
  })

  describe('split view: pane inner spacing', () => {
    // `.content`'s own padding only ever reached the two edges touching the
    // window — the edge each pane shares with the divider got none, so a
    // card sat flush against it. Each `.content__pane` now carries the same
    // inset independently, and `.content__split` cancels `.content`'s own
    // padding first so a single pane still nets exactly one inset, not two.
    const source = readFileSync(resolve(import.meta.dirname, 'AppShell.svelte'), 'utf-8')

    it('gives every pane its own inline padding, not just the outer two edges', () => {
      expect(source).toMatch(/\.content__pane\s*\{[\s\S]*?padding-inline:\s*var\(--space-5\);/)
    })

    it("cancels .content's own inline padding on the split row, so a single pane nets one inset", () => {
      expect(source).toMatch(
        /\.content__split\s*\{[\s\S]*?margin-inline:\s*calc\(-1 \* var\(--space-5\)\);/
      )
    })
  })

  it('opens external links through the desktop bridge', async () => {
    render(AppShellHost)

    await fireEvent.click(screen.getByRole('link', { name: 'GitHub' }))
    expect(invokeMock).toHaveBeenCalledWith('open_external_url', {
      url: 'https://github.com/HumaLab/EntropIA-Pro-Lite',
    })

    await fireEvent.click(screen.getByRole('link', { name: 'HLab' }))
    expect(invokeMock).toHaveBeenCalledWith('open_external_url', {
      url: 'https://hlab.com.ar/',
    })
  })

  it('toggles the sidebar with Ctrl+B except when typing in editable targets', async () => {
    render(AppShellHost)

    const editable = document.createElement('div')
    editable.setAttribute('contenteditable', 'true')
    document.body.appendChild(editable)

    try {
      // Plain Ctrl+B collapses the sidebar.
      await fireEvent.keyDown(document.body, { key: 'b', ctrlKey: true })
      expect(screen.getByRole('button', { name: 'Expandir panel (Ctrl+B)' })).toBeInTheDocument()

      // Ctrl+B from a contenteditable surface (e.g. the note editor) is ignored.
      await fireEvent.keyDown(editable, { key: 'b', ctrlKey: true })
      expect(screen.getByRole('button', { name: 'Expandir panel (Ctrl+B)' })).toBeInTheDocument()

      // Plain Ctrl+B expands it again.
      await fireEvent.keyDown(document.body, { key: 'b', ctrlKey: true })
      expect(screen.getByRole('button', { name: 'Colapsar panel (Ctrl+B)' })).toBeInTheDocument()

      // Ctrl+B from a text input is ignored too.
      await fireEvent.click(screen.getByRole('button', { name: 'Filtrar colecciones' }))
      const filterInput = screen.getByPlaceholderText('Filtrar colecciones...')
      await fireEvent.keyDown(filterInput, { key: 'b', ctrlKey: true })
      expect(screen.getByRole('button', { name: 'Colapsar panel (Ctrl+B)' })).toBeInTheDocument()
    } finally {
      editable.remove()
    }
  })

  it('reacts to locale changes in footer and sidebar copy', async () => {
    render(AppShellHost)

    locale.set('en')

    expect(await screen.findByText('Archive, OCR, and assisted analysis.')).toBeInTheDocument()
    expect(screen.getByText('Developed by')).toBeInTheDocument()
    expect(screen.getByRole('complementary', { name: 'Sidebar' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Collapse sidebar (Ctrl+B)' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'New collection' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Filter collections' })).toBeInTheDocument()
    expect(screen.getByRole('complementary', { name: 'Document explorer' })).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Filter collections' }))

    expect(screen.getByPlaceholderText('Filter collections...')).toBeInTheDocument()
  })

  it.runIf(LOCAL_ML)(
    'boots without awaiting a fresh dependency probe and updates from completion events',
    async () => {
      let depsCompleteHandler:
        | ((event: {
            payload: { results: Array<{ id: string; status: { type: string } }> }
          }) => void)
        | undefined

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

      // Critical-missing is announced through the single persistent banner channel.
      expect(
        await screen.findByText('Algunas funciones de IA no están disponibles.')
      ).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Configurar dependencias' })).toBeInTheDocument()
    }
  )

  it.runIf(LOCAL_ML)(
    'announces critical-missing deps through the banner only, never a coexisting toast',
    async () => {
      let depsCompleteHandler:
        | ((event: {
            payload: { results: Array<{ id: string; status: { type: string } }> }
          }) => void)
        | undefined

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

      // The actionable banner is the single critical-missing channel.
      const banner = await screen.findByText('Algunas funciones de IA no están disponibles.')
      expect(banner).toBeInTheDocument()

      // The legacy toast must NOT coexist with the banner for this state (#27):
      // its title, body copy, and dismiss control are all gone.
      expect(screen.queryByText('Dependencias de IA pendientes')).not.toBeInTheDocument()
      expect(
        screen.queryByText(
          'Se necesitan Python y paquetes para OCR/transcripción; embeddings usan OpenRouter.'
        )
      ).not.toBeInTheDocument()
      expect(screen.queryByRole('button', { name: 'Cerrar' })).not.toBeInTheDocument()

      // Only one alert region carries the critical-missing message — no duplicate channel.
      const criticalAlerts = screen
        .getAllByRole('alert')
        .filter((el) => el.textContent?.includes('Algunas funciones de IA no están disponibles'))
      expect(criticalAlerts).toHaveLength(1)
    }
  )

  it.runIf(LOCAL_ML)(
    'shows runtime health alerts when the managed runtime is damaged',
    async () => {
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
      expect(screen.getByRole('button', { name: 'Reparar runtime' })).toBeInTheDocument()
      expect(screen.getByText(/ocr, transcription/i)).toBeInTheDocument()
    }
  )

  it.runIf(LOCAL_ML)('shows fixture runtime alerts without repair action', async () => {
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
          summary:
            'Runtime de desarrollo detectado para linux-x86_64: faltan payloads externos de release',
          blockedCapabilities: ['ocr', 'transcription', 'nlp'],
          details: [
            'La app 0.0.10 arrancó correctamente, pero este runtime-pack todavía está en modo fixture/dev (app_version declarada: 0.0.10).',
          ],
          guidance: [
            'Esto no indica una caída: la UI puede abrir, pero OCR/NLP/transcripción quedan bloqueados hasta inyectar los payloads de release.',
          ],
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
        'Runtime de desarrollo detectado para linux-x86_64: faltan payloads externos de release'
      )
    ).toBeInTheDocument()
    expect(screen.getByText(/app no se cay/i)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Reparar runtime →' })).not.toBeInTheDocument()
  })

  it.runIf(LOCAL_ML)(
    'does not show a global runtime alert when deps are installed and only fixture release packaging is pending',
    async () => {
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
            summary:
              'Runtime de desarrollo detectado para linux-x86_64: faltan payloads externos de release',
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
    }
  )

  it.runIf(LOCAL_ML)(
    'shows a global runtime alert when release source wiring is blocked',
    async () => {
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
        'No hay una fuente confiable disponible para bootstrap'
      )
      expect(screen.getByText(/ocr, transcription, nlp/i)).toBeInTheDocument()
    }
  )

  // Regression guard (Stage 2 visual fix #1): `.content` used to be a plain
  // block (only `flex: 1` as a *row*-flex item of `.workspace`), so its
  // child `WorkPane` — itself `display: flex; flex-direction: column; flex:
  // 1` — had no flex *container* to size against and collapsed to its
  // content height instead of filling the pane, leaving dead space below
  // the view. `.content` must itself be a column flex container with
  // `min-height: 0` so that chain resolves. The real proof is the user's
  // own visual check of the running app; this only guards the CSS rule that
  // makes it possible.
  it('keeps `.content` a column flex container so WorkPane fills the pane height', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'AppShell.svelte'), 'utf-8')
    const start = source.indexOf('  .content {')
    const rule = source.slice(start, source.indexOf('}', start))
    expect(rule).toMatch(/display:\s*flex;/)
    expect(rule).toMatch(/flex-direction:\s*column;/)
    expect(rule).toMatch(/min-height:\s*0;/)
  })

  // Task 3.3: split rendering + the active-pane rule (spec, Split view: "The
  // active pane is the last one the user clicked or focused, marked with a
  // thin accent border").
  describe('split view', () => {
    it('renders both panes side by side, keyed by tab id, when split is on', () => {
      const leftId = workspace.activeTabId
      workspace.toggleSplit()
      const rightId = workspace.split!.rightId

      render(AppShellHost)

      const panes = screen.getAllByTestId('app-shell-child')
      expect(panes).toHaveLength(2)
      expect(panes[0]).toHaveAttribute('data-pane-id', leftId)
      expect(panes[1]).toHaveAttribute('data-pane-id', rightId)
      expect(workPaneMountLog).toEqual([leftId, rightId])
    })

    // Final review item 5: a 1 -> 2 pane toggle used to switch template
    // branches, remounting the active pane (editor undo, cursor and scroll
    // lost, ItemView reloaded, a WritingView torn down mid-save).
    it('keeps the active pane mounted while split view is turned on and off', async () => {
      const leftId = workspace.activeTabId

      const { container } = render(AppShellHost)
      expect(workPaneMountLog).toEqual([leftId])
      // One pane alone carries no active-pane border: there is nothing to
      // tell it apart from.
      expect(container.querySelector('.content__pane--active')).toBeNull()

      workspace.toggleSplit()
      const rightId = workspace.split!.rightId
      await waitFor(() => expect(screen.getAllByTestId('app-shell-child')).toHaveLength(2))
      expect(workPaneMountLog).toEqual([leftId, rightId])

      workspace.toggleSplit()
      await waitFor(() => expect(screen.getAllByTestId('app-shell-child')).toHaveLength(1))
      expect(screen.getByTestId('app-shell-child')).toHaveAttribute('data-pane-id', leftId)
      expect(workPaneMountLog).toEqual([leftId, rightId])
    })

    it('activates the pane on pointerdown, before any click handler runs, and moves the accent border to it', async () => {
      const leftId = workspace.activeTabId
      workspace.toggleSplit()
      const rightId = workspace.split!.rightId

      const { container } = render(AppShellHost)

      const leftChild = container.querySelector(`[data-pane-id="${leftId}"]`)!
      const rightChild = container.querySelector(`[data-pane-id="${rightId}"]`)!
      const leftWrapper = leftChild.closest('.content__pane')!
      const rightWrapper = rightChild.closest('.content__pane')!

      expect(leftWrapper).toHaveClass('content__pane--active')
      expect(rightWrapper).not.toHaveClass('content__pane--active')

      // A click inside the inactive pane must make it active — exercised as
      // pointerdown, the event AppShell listens for so activation happens
      // before whatever click handler the click eventually reaches (e.g. a
      // TopBar section icon acting on "the active pane").
      await fireEvent.pointerDown(rightChild)

      expect(workspace.activeTabId).toBe(rightId)
      expect(rightWrapper).toHaveClass('content__pane--active')
      expect(leftWrapper).not.toHaveClass('content__pane--active')
    })
  })

  describe('split view: active-pane indicator is discreet', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'AppShell.svelte'), 'utf-8')
    const styles = source.slice(source.indexOf('<style>'))

    function ruleFor(selector: string): string {
      const at = styles.indexOf(selector)
      expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
      const rule = styles.slice(at)
      return rule.slice(0, rule.indexOf('}'))
    }

    it('never draws the indicator in the bright accent color', () => {
      // --color-accent reads as a glow against the very dark surfaces here —
      // too bright, and its crisp full-perimeter corners drew the eye far
      // more than "the active pane" needed to.
      expect(ruleFor('.content__pane--active {')).not.toMatch(/--color-accent\b/)
    })

    it('uses a low-contrast existing border token instead', () => {
      expect(ruleFor('.content__pane--active {')).toMatch(
        /box-shadow:\s*inset 0 0 0 1px var\(--(border-subtle|color-border-strong|color-border)\);?\s*$/
      )
    })

    it('has no glow: a single 0-blur, 0-spread inset ring, no second shadow layer', () => {
      const rule = ruleFor('.content__pane--active {')
      const shadow = /box-shadow:\s*([^;]+);/.exec(rule)?.[1] ?? ''
      expect(shadow.split(',').length).toBe(1)
      expect(shadow).toMatch(/^inset 0 0 0 1px /)
    })

    it('adds no corner accent: no ::before/::after tied to the active pane', () => {
      expect(styles).not.toMatch(/\.content__pane--active::(before|after)/)
    })

    it('gives the inactive pane the same geometry, transparent, so nothing shifts on activation', () => {
      // Same property on the base class, not just the modifier: activating a
      // pane must never introduce or remove a box-shadow layer, only change
      // its color, or the pane's painted layout could shift.
      expect(ruleFor('.content__pane {')).toMatch(/box-shadow:\s*inset 0 0 0 1px transparent;/)
    })
  })

  // Visual round 3, item 2: the drawer's z-index (2) was lower than the
  // shared sticky page-header's z-index (20, app.css `.page-header` /
  // `.collections-intro` / `.settings-view__sticky-header`), and nothing
  // between them isolated the pane's own stacking order from the drawer's —
  // so a WorkPane's own sticky header painted ABOVE the drawer that is
  // supposed to cover it. Empirically confirmed in a real Chromium engine
  // (Playwright) before this fix: `document.elementFromPoint` over the
  // overlap returned the header, not the drawer, until the drawer's
  // z-index was raised past the header's.
  describe('split view: explorer drawer stacking', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'AppShell.svelte'), 'utf-8')
    const styles = source.slice(source.indexOf('<style>'))

    function ruleFor(selector: string): string {
      const at = styles.indexOf(selector)
      expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
      const rule = styles.slice(at)
      return rule.slice(0, rule.indexOf('}'))
    }

    it("gives the pane its own stacking context, so a WorkPane's internal z-index can never leak past it", () => {
      expect(ruleFor('.content__pane {')).toMatch(/isolation:\s*isolate;/)
    })

    it('paints the drawer above any in-pane sticky header (max known: --page-header at z-index 20)', () => {
      const rule = ruleFor('.explorer-drawer {')
      const zIndex = Number(/z-index:\s*(\d+);/.exec(rule)?.[1])
      expect(zIndex).toBeGreaterThan(20)
    })

    it('stays below app-level overlays (ToolbarMenu 210, ConfirmDialog/dialogs 1000+, TooltipLayer 1300)', () => {
      const rule = ruleFor('.explorer-drawer {')
      const zIndex = Number(/z-index:\s*(\d+);/.exec(rule)?.[1])
      expect(zIndex).toBeLessThan(100)
    })
  })

  // Task 3.4: responsive stacking (spec, Responsive) — the `watchStacking`
  // wiring, `SplitDivider`'s orientation, and the render-time ratio clamp
  // are all pure-glue unit logic already covered elsewhere (`resize-stacking
  // .test.ts`, `split-ratio.test.ts`), but the controller review (fix round
  // 1) overruled the brief's "no component test needed" call: remounts have
  // already bitten this feature twice (see task-3.3-report.md), and this
  // harness already mocks `WorkPane` with a mount log, so the check is cheap.
  describe('split view: responsive stacking (Task 3.4)', () => {
    type ResizeEntry = { target?: Element; contentRect: { width: number } }
    type ResizeCallback = (entries: ResizeEntry[]) => void

    // `.content__split` carries TWO independent `ResizeObserver` consumers
    // once this global is stubbed: `watchStacking`'s own (this task), and
    // Svelte's internal one backing `bind:clientWidth`/`clientHeight` (also
    // used here, for the ratio clamp). Firing a well-formed entry — with a
    // real `target` so Svelte's own dispatch (which indexes listeners by
    // `entry.target` in a `WeakMap`) doesn't throw — to every captured
    // instance reaches `watchStacking`'s callback correctly without having
    // to guess which instance is which; `unobserve` is implemented (a
    // no-op) so Svelte's own teardown on unmount doesn't throw either.
    class FakeResizeObserver {
      static instances: FakeResizeObserver[] = []
      callback: ResizeCallback
      constructor(callback: ResizeCallback) {
        this.callback = callback
        FakeResizeObserver.instances.push(this)
      }
      observe() {}
      unobserve() {}
      disconnect() {}
    }

    function fireResize(target: Element, width: number): void {
      const entry: ResizeEntry = { target, contentRect: { width } }
      FakeResizeObserver.instances.forEach((observer) => observer.callback([entry]))
    }

    // `bind:clientWidth`/`clientHeight` (the render-time ratio clamp's size
    // source) reads a real layout property happy-dom never computes — stub
    // it for exactly the one element under test, restored byte-for-byte
    // afterward so no other test's `clientWidth`/`clientHeight` reads shift.
    function stubClientSize(className: string, width: number, height: number): () => void {
      const widthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth')!
      const heightDescriptor = Object.getOwnPropertyDescriptor(
        HTMLElement.prototype,
        'clientHeight'
      )!
      Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
        configurable: true,
        get(this: HTMLElement) {
          return this.classList.contains(className) ? width : widthDescriptor.get!.call(this)
        },
      })
      Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
        configurable: true,
        get(this: HTMLElement) {
          return this.classList.contains(className) ? height : heightDescriptor.get!.call(this)
        },
      })
      return () => {
        Object.defineProperty(HTMLElement.prototype, 'clientWidth', widthDescriptor)
        Object.defineProperty(HTMLElement.prototype, 'clientHeight', heightDescriptor)
      }
    }

    beforeEach(() => {
      FakeResizeObserver.instances = []
      vi.stubGlobal('ResizeObserver', FakeResizeObserver)
    })
    afterEach(() => {
      vi.unstubAllGlobals()
      // `workspace.setSplitRatio` persists (best-effort) to localStorage
      // (see `workspace.test.ts`'s own key literal); clear it so this
      // test's stored 0.3 never leaks into a later `toggleSplit()`'s
      // `loadRatio()` elsewhere in this file.
      localStorage.removeItem('entropia-workspace-split-ratio')
    })

    it('stacks + turns the divider horizontal on a narrow resize, reverts on a wide one, without remounting either pane', async () => {
      const leftId = workspace.activeTabId
      workspace.toggleSplit()
      const rightId = workspace.split!.rightId

      const { container } = render(AppShellHost)
      const splitEl = container.querySelector('.content__split')!
      // Scoped to `.content__split`, not the whole document: the sidebar's
      // own resize handle (`DocumentExplorer`) also carries
      // `role="separator"` and renders before it in DOM order.
      const divider = splitEl.querySelector('[role="separator"]')!
      expect(workPaneMountLog).toEqual([leftId, rightId])

      // Crosses below the two-pane-fits threshold (spec, Responsive).
      fireResize(splitEl, 2 * MIN_PANE_PX - 1)
      await waitFor(() => expect(splitEl).toHaveClass('content__split--stacked'))
      expect(divider).toHaveAttribute('aria-orientation', 'horizontal')
      expect(workPaneMountLog).toEqual([leftId, rightId])

      // Crosses back above it.
      fireResize(splitEl, 2 * MIN_PANE_PX + 40)
      await waitFor(() => expect(splitEl).not.toHaveClass('content__split--stacked'))
      expect(divider).toHaveAttribute('aria-orientation', 'vertical')
      // Same two mounted instances throughout: orientation switching never
      // remounts a pane (both stay keyed by tab id only, never by `stacked`).
      expect(workPaneMountLog).toEqual([leftId, rightId])
    })

    it('persists the split ratio when a drag ends, not on every pointermove', async () => {
      workspace.toggleSplit()
      const restore = stubClientSize('content__split', 1000, 800)
      try {
        const { container } = render(AppShellHost)
        const divider = container.querySelector('.content__split [role="separator"]')!
        const setItem = vi.spyOn(Storage.prototype, 'setItem')
        const ratioWrites = () =>
          setItem.mock.calls.filter(([key]) => key === 'entropia-workspace-split-ratio')

        await fireEvent.pointerDown(divider, { pointerId: 1, clientX: 500, clientY: 10 })
        await fireEvent.pointerMove(divider, { pointerId: 1, clientX: 550, clientY: 10 })
        await fireEvent.pointerMove(divider, { pointerId: 1, clientX: 600, clientY: 10 })
        expect(ratioWrites()).toHaveLength(0)

        await fireEvent.pointerUp(divider, { pointerId: 1, clientX: 600, clientY: 10 })
        expect(ratioWrites()).toHaveLength(1)
        setItem.mockRestore()
      } finally {
        restore()
      }
    })

    it('clamps the render-time ratio to the current container size, without rewriting the stored ratio', () => {
      workspace.toggleSplit()
      // Stored below the pixel floor an 800px-wide container allows (0.4),
      // but still inside the store's own [0.25, 0.75] ratio bound, so it
      // survives `setSplitRatio` unchanged and only gets clamped at render
      // time below.
      workspace.setSplitRatio(0.3)
      expect(workspace.split!.ratio).toBe(0.3)

      const restore = stubClientSize('content__split', 800, 800)
      try {
        const { container } = render(AppShellHost)
        const splitEl = container.querySelector('.content__split')!
        const divider = splitEl.querySelector('[role="separator"]')!
        const leftPane = splitEl.querySelector('.content__pane') as HTMLElement

        // 320 / 800 = 0.4 — clamped up from the stored 0.3 so the left pane
        // never renders below 320px on this container.
        expect(divider).toHaveAttribute('aria-valuenow', '40')
        expect(leftPane.style.getPropertyValue('flex-basis')).toBe('40%')
      } finally {
        restore()
      }

      // The clamp is a render-time-only correction: the stored ratio itself
      // is never rewritten just because the container was narrow.
      expect(workspace.split!.ratio).toBe(0.3)
    })
  })
  // Split view has ONE document explorer: closed by default, and when opened
  // it is a drawer inside the ACTIVE pane rather than a docked column that
  // squeezes both panes (explorer-drawer design).
  describe('split view: explorer drawer', () => {
    const EXPLORER = { name: 'Explorador de documentos' }
    const DRAWER = { name: 'Explorador de documentos del panel activo' }

    function paneStyles(container: HTMLElement): string[] {
      return [...container.querySelectorAll<HTMLElement>('.content__pane')].map(
        (pane) =>
          `${pane.style.getPropertyValue('flex-basis')}|${pane.style.getPropertyValue('flex-grow')}`
      )
    }

    it('closes the explorer when split view turns on', async () => {
      render(AppShellHost)
      expect(await screen.findByRole('complementary', EXPLORER)).toBeInTheDocument()

      workspace.toggleSplit()

      await waitFor(() =>
        expect(screen.queryByRole('complementary', EXPLORER)).not.toBeInTheDocument()
      )
      const toggle = screen.getByRole('button', { name: 'Expandir panel (Ctrl+B)' })
      expect(toggle).toHaveAttribute('aria-expanded', 'false')
      expect(screen.queryByRole('region', DRAWER)).not.toBeInTheDocument()
    })

    it('opens as a drawer inside the active pane without resizing either pane', async () => {
      const leftId = workspace.activeTabId
      workspace.toggleSplit()
      const { container } = render(AppShellHost)
      const before = paneStyles(container)

      const toggle = screen.getByRole('button', { name: 'Expandir panel (Ctrl+B)' })
      await fireEvent.click(toggle)

      const drawer = await screen.findByRole('region', DRAWER)
      const activePane = container
        .querySelector(`[data-pane-id="${leftId}"]`)!
        .closest('.content__pane')!
      expect(activePane).toHaveClass('content__pane--active')
      expect(activePane.contains(drawer)).toBe(true)
      expect(within(drawer).getByRole('complementary', EXPLORER)).toBeInTheDocument()
      // One explorer only: the docked sidebar holds none while the drawer is open.
      expect(screen.getAllByRole('complementary', EXPLORER)).toHaveLength(1)
      expect(container.querySelector('.sidebar')!.contains(drawer)).toBe(false)
      expect(toggle).toHaveAttribute('aria-expanded', 'true')
      expect(toggle).toHaveAttribute('aria-controls', drawer.id)
      await waitFor(() => expect(drawer.contains(document.activeElement)).toBe(true))
      expect(paneStyles(container)).toEqual(before)
    })

    it('closes instead of jumping when the other pane becomes active', async () => {
      workspace.toggleSplit()
      const rightId = workspace.split!.rightId
      const { container } = render(AppShellHost)

      await fireEvent.click(screen.getByRole('button', { name: 'Expandir panel (Ctrl+B)' }))
      await screen.findByRole('region', DRAWER)

      await fireEvent.pointerDown(container.querySelector(`[data-pane-id="${rightId}"]`)!)

      expect(workspace.activeTabId).toBe(rightId)
      await waitFor(() => expect(screen.queryByRole('region', DRAWER)).not.toBeInTheDocument())
    })

    it('closes on Escape and returns focus to the toggle', async () => {
      workspace.toggleSplit()
      render(AppShellHost)

      const toggle = screen.getByRole('button', { name: 'Expandir panel (Ctrl+B)' })
      toggle.focus()
      await fireEvent.click(toggle)
      const drawer = await screen.findByRole('region', DRAWER)
      await waitFor(() => expect(drawer.contains(document.activeElement)).toBe(true))

      await fireEvent.keyDown(document.activeElement!, { key: 'Escape' })

      await waitFor(() => expect(screen.queryByRole('region', DRAWER)).not.toBeInTheDocument())
      expect(document.activeElement).toBe(toggle)
      expect(toggle).toHaveAttribute('aria-expanded', 'false')
    })

    it('closes when the pane content outside the drawer is clicked', async () => {
      const leftId = workspace.activeTabId
      workspace.toggleSplit()
      const { container } = render(AppShellHost)

      await fireEvent.click(screen.getByRole('button', { name: 'Expandir panel (Ctrl+B)' }))
      const drawer = await screen.findByRole('region', DRAWER)

      // A click inside the drawer keeps it open.
      await fireEvent.pointerDown(within(drawer).getByRole('complementary', EXPLORER))
      expect(screen.getByRole('region', DRAWER)).toBe(drawer)

      await fireEvent.pointerDown(container.querySelector(`[data-pane-id="${leftId}"]`)!)

      await waitFor(() => expect(screen.queryByRole('region', DRAWER)).not.toBeInTheDocument())
      expect(workspace.activeTabId).toBe(leftId)
    })

    it.each([
      ['open', 'Colapsar panel (Ctrl+B)', true],
      ['collapsed', 'Expandir panel (Ctrl+B)', false],
    ] as const)(
      'restores the docked sidebar %s when split view turns off',
      async (_state, toggleName, explorerShown) => {
        render(AppShellHost)
        if (!explorerShown) {
          await fireEvent.keyDown(document.body, { key: 'b', ctrlKey: true })
        }

        workspace.toggleSplit()
        await fireEvent.click(
          await screen.findByRole('button', { name: 'Expandir panel (Ctrl+B)' })
        )
        await screen.findByRole('region', DRAWER)

        workspace.toggleSplit()

        await waitFor(() => expect(screen.queryByRole('region', DRAWER)).not.toBeInTheDocument())
        expect(screen.getByRole('button', { name: toggleName })).toBeInTheDocument()
        const sidebar = screen.getByRole('complementary', { name: 'Panel lateral' })
        if (explorerShown) {
          expect(within(sidebar).getByRole('complementary', EXPLORER)).toBeInTheDocument()
        } else {
          expect(screen.queryByRole('complementary', EXPLORER)).not.toBeInTheDocument()
        }
      }
    )
  })
})
