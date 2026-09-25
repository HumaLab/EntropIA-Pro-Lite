import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import TopBar from './TopBar.svelte'
import { locale } from '$lib/i18n'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })

  return { promise, resolve, reject }
}

const {
  navigateActiveMock,
  storeRef,
  minimizeMock,
  toggleMaximizeMock,
  closeWindowMock,
  toggleSplitMock,
  splitRef,
} = vi.hoisted(() => {
  return {
    navigateActiveMock: vi.fn(),
    storeRef: {
      current: {
        items: { searchGlobal: vi.fn() },
        collections: { findById: vi.fn() },
      },
    },
    minimizeMock: vi.fn(),
    toggleMaximizeMock: vi.fn(),
    closeWindowMock: vi.fn(),
    toggleSplitMock: vi.fn(),
    // Read fresh inside `subscribe`'s `run()` on every render, so a test can
    // flip it before `render(TopBar)` to exercise the pressed state.
    splitRef: {
      current: null as { leftId: string; rightId: string; ratio: number } | null,
    },
  }
})

// TopBar now hosts `TabStrip`, which reads `workspace.tabs`/`activeTabId`
// through `workspace.subscribe`. A single, static Home tab is enough here —
// TabStrip's own behavior (adding, closing, grouping tabs) is covered by
// TabStrip.test.ts, not this file. The split toggle (Task 3.3) also reads
// `workspace.split` through the same subscription, so `splitRef` lets a test
// drive it without a real WorkspaceStore.
vi.mock('$lib/workspace', () => ({
  MAX_TABS: 4,
  workspace: {
    tabs: [{ id: 'tab-1', navigation: { current: { name: 'home' as const } } }],
    activeTabId: 'tab-1',
    subscribe(run: (value: unknown) => void) {
      run({
        tabs: [{ id: 'tab-1', navigation: { current: { name: 'home' as const } } }],
        activeTabId: 'tab-1',
        split: splitRef.current,
      })
      return () => {}
    },
    activateTab: vi.fn(),
    closeTab: vi.fn(),
    openTab: vi.fn(),
    navigateActive: navigateActiveMock,
    toggleSplit: toggleSplitMock,
  },
}))

vi.mock('$lib/db', () => ({
  getStore: () => storeRef.current,
}))

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    minimize: minimizeMock,
    toggleMaximize: toggleMaximizeMock,
    close: closeWindowMock,
  }),
}))

describe('TopBar', () => {
  beforeEach(() => {
    locale.set('es')
    localStorage.clear()
    delete document.documentElement.dataset.theme
    vi.useFakeTimers()
    navigateActiveMock.mockReset()
    minimizeMock.mockReset()
    toggleMaximizeMock.mockReset()
    closeWindowMock.mockReset()
    toggleSplitMock.mockReset()
    splitRef.current = null
    storeRef.current.items.searchGlobal.mockReset()
    storeRef.current.collections.findById.mockReset()
  })

  afterEach(() => {
    vi.useRealTimers()
    localStorage.clear()
    delete document.documentElement.dataset.theme
  })

  it('renders accessible controls for navigation and global search', () => {
    render(TopBar)

    expect(
      screen.getByRole('button', { name: 'Abrir navegador de base de datos' })
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Abrir chat de investigación' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Abrir configuración' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Minimizar ventana' })).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Maximizar o restaurar ventana' })
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Cerrar ventana' })).toBeInTheDocument()
    expect(
      screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })
    ).toBeInTheDocument()
  })

  it('navigates to db browser from the database icon button', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir navegador de base de datos' }))

    expect(navigateActiveMock).toHaveBeenCalledWith({ name: 'db-browser' })
  })

  it('opens Inicio from the first icon button, a house', async () => {
    const { container } = render(TopBar)

    const button = screen.getByRole('button', { name: 'Abrir Inicio' })
    // First of the section icons: nothing but the search sits before it.
    expect(container.querySelector('.topbar__icon-btn')).toBe(button)
    expect(button.querySelector('svg')).not.toBeNull()

    await fireEvent.click(button)

    expect(navigateActiveMock).toHaveBeenCalledWith({ name: 'home' })
  })

  it('opens Colecciones from the second icon button, right after Inicio', async () => {
    const { container } = render(TopBar)

    const button = screen.getByRole('button', { name: 'Abrir Colecciones' })
    expect(container.querySelectorAll('.topbar__icon-btn')[1]).toBe(button)

    await fireEvent.click(button)

    expect(navigateActiveMock).toHaveBeenCalledWith({ name: 'collections' })
  })

  it('navigates to the research chat from the chat icon button', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir chat de investigación' }))

    expect(navigateActiveMock).toHaveBeenCalledWith({ name: 'rag-chat' })
  })

  it('opens the research agent from its section icon', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir agente de investigación' }))

    expect(navigateActiveMock).toHaveBeenCalledWith({ name: 'research' })
  })

  it('opens Escritura from its section icon without naming a document', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir Escritura' }))

    // No document id: when another tab owns Writing, the workspace only
    // activates it and leaves whatever document it shows in place.
    expect(navigateActiveMock).toHaveBeenCalledWith({ name: 'writing' })
  })

  it('opens settings as a canonical root section', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir configuración' }))

    expect(navigateActiveMock).toHaveBeenCalledWith({ name: 'settings' })
  })

  it('toggles split view from the split icon button, unpressed while split is off', async () => {
    render(TopBar)

    const button = screen.getByRole('button', { name: 'Alternar vista dividida' })
    expect(button).not.toHaveAttribute('aria-pressed')

    await fireEvent.click(button)

    expect(toggleSplitMock).toHaveBeenCalledTimes(1)
  })

  it('marks the split toggle pressed while split view is on', () => {
    splitRef.current = { leftId: 'tab-1', rightId: 'tab-2', ratio: 0.5 }

    render(TopBar)

    expect(screen.getByRole('button', { name: 'Alternar vista dividida' })).toHaveAttribute(
      'aria-pressed',
      'true'
    )
  })

  /**
   * Final visual check (split view): with 8e57aa7f the pane that stays on
   * screen no longer remounts when split is toggled, so an open editor keeps
   * its text — but a mouse click on the toggle still steals focus from it
   * the way clicking any focusable button does, so the caret was lost even
   * though the DOM node never went away. The toggle must hand focus back to
   * whatever had it right before the click — captured on `pointerdown`,
   * before the browser's own default mousedown action moves focus to the
   * button — unless the button itself already had focus (a keyboard user
   * who tabbed to it and pressed Enter/Space): there, no `pointerdown` ever
   * fires, so nothing is captured and focus is correctly left alone.
   */
  describe('split toggle focus restore', () => {
    it('returns focus to the element that had it before a mouse click on the toggle', async () => {
      const editor = document.createElement('textarea')
      document.body.appendChild(editor)
      editor.focus()
      expect(document.activeElement).toBe(editor)

      render(TopBar)
      const button = screen.getByRole('button', { name: 'Alternar vista dividida' })

      // Mirrors the real sequence: pointerdown fires (and is captured) while
      // the editor still has focus, THEN the browser's own default action
      // moves focus to the button, THEN click fires.
      await fireEvent.pointerDown(button)
      button.focus()
      await fireEvent.click(button)

      expect(toggleSplitMock).toHaveBeenCalledTimes(1)
      expect(document.activeElement).toBe(editor)

      editor.remove()
    })

    it('leaves focus on the toggle for a keyboard activation (no preceding pointerdown)', async () => {
      render(TopBar)
      const button = screen.getByRole('button', { name: 'Alternar vista dividida' })

      // Focus arrived via Tab, not a pointerdown; Enter/Space then fires a
      // click with the button already focused.
      button.focus()
      expect(document.activeElement).toBe(button)

      await fireEvent.click(button)

      expect(toggleSplitMock).toHaveBeenCalledTimes(1)
      expect(document.activeElement).toBe(button)
    })

    it('does not try to restore focus to an element that left the document', async () => {
      const editor = document.createElement('textarea')
      document.body.appendChild(editor)
      editor.focus()

      render(TopBar)
      const button = screen.getByRole('button', { name: 'Alternar vista dividida' })

      await fireEvent.pointerDown(button)
      editor.remove()
      button.focus()

      await expect(fireEvent.click(button)).resolves.not.toThrow()
      expect(toggleSplitMock).toHaveBeenCalledTimes(1)
    })
  })

  it('shows the product name as plain text with the EntropIA mark on its left', () => {
    const { container } = render(TopBar)

    const title = container.querySelector('.topbar__app-title')
    expect(title).not.toBeNull()
    expect(title!.tagName).not.toBe('BUTTON')
    expect(title!.closest('button, a')).toBeNull()
    expect(title).toHaveAttribute('data-tauri-drag-region')
    const mark = title!.querySelector('.topbar__app-mark')
    expect(mark).not.toBeNull()
    expect(mark).toHaveAttribute('aria-hidden', 'true')
    // The mark comes before the name.
    expect(title!.firstElementChild).toBe(mark)
  })

  it('masks the title mark with the transparent e, never with the disc-shaped hlab-mark', () => {
    // hlab-mark.png is a white disc behind a black 'e'. As a CSS mask only its
    // opacity counts, so it painted a solid circle in the title colour.
    const source = readFileSync(resolve(import.meta.dirname, 'TopBar.svelte'), 'utf-8')
    expect(source).toContain("import appMark from '../assets/entropia-mark.png'")
    expect(source).not.toMatch(/import appMark from '[^']*hlab-mark\.png'/)
  })

  // Regression guard (Stage 2 visual fix #2): `.topbar__leading` used to be
  // the grid's only `1fr` track, so it absorbed all the free space and
  // pushed the tab strip flush against the search box instead of letting it
  // start right after the title and grow. `.topbar__center` (the tab strip)
  // must be the flexible track, left-aligned within it, while `.topbar__leading`
  // shrinks to the title's own width. Verified at both the default layout and
  // the <900px breakpoint (search moves to its own row there, so it drops out
  // of the row template, but leading/center/actions keep the same relation).
  it('gives the tab strip (not the title) the flexible grid track, left-aligned', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'TopBar.svelte'), 'utf-8')

    const baseStart = source.indexOf('  .topbar {')
    const baseRule = source.slice(baseStart, source.indexOf('}', baseStart))
    expect(baseRule).toMatch(
      /grid-template-columns:\s*auto minmax\(0, 1fr\) minmax\(220px, 320px\) auto;/
    )

    const centerStart = source.indexOf('  .topbar__center {')
    const centerRule = source.slice(centerStart, source.indexOf('}', centerStart))
    expect(centerRule).toMatch(/justify-content:\s*flex-start;/)

    const narrowStart = source.indexOf('@media (max-width: 900px)')
    const narrowBlock = source.slice(narrowStart, source.indexOf('.topbar__leading', narrowStart))
    expect(narrowBlock).toMatch(/grid-template-columns:\s*auto minmax\(0, 1fr\) auto;/)
  })

  describe('window drag region', () => {
    // Tauri's drag script (tauri 2.x src/window/scripts/drag.js) only reads
    // `data-tauri-drag-region` on the exact mousedown target, never on an
    // ancestor. So every empty container of the bar must carry it, and no
    // control may: a button, the search field or a menu keeps its click.
    it('marks the bar and every empty container as a drag region', () => {
      const { container } = render(TopBar)

      for (const selector of [
        '.topbar',
        '.topbar__leading',
        '.topbar__back-slot',
        '.topbar__center',
        '.topbar__actions',
        '.topbar__window-controls',
      ]) {
        expect(container.querySelector(selector), selector).toHaveAttribute(
          'data-tauri-drag-region'
        )
      }
    })

    it('never marks a control, so clicks on buttons, the search and menus still work', () => {
      const { container } = render(TopBar)

      const marked = container.querySelectorAll(
        'button[data-tauri-drag-region], input[data-tauri-drag-region], [role="combobox"][data-tauri-drag-region], a[data-tauri-drag-region], .global-search[data-tauri-drag-region], .global-search [data-tauri-drag-region]'
      )
      expect(marked).toHaveLength(0)
    })

    it('keeps text and images in the bar from being selected or dragged as content', () => {
      const source = readFileSync(resolve(import.meta.dirname, 'TopBar.svelte'), 'utf-8')
      const start = source.indexOf('  .topbar {')
      const rule = source.slice(start, source.indexOf('}', start))
      expect(rule).toMatch(/user-select:\s*none;/)
      expect(rule).toMatch(/-webkit-user-drag:\s*none;/)
      // The search field stays editable and selectable.
      expect(source).toMatch(/\.global-search__input\s*\{[^}]*user-select:\s*text;/)
    })
  })

  it('forwards custom window controls to the current Tauri window', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Minimizar ventana' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Maximizar o restaurar ventana' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Cerrar ventana' }))

    expect(minimizeMock).toHaveBeenCalledTimes(1)
    expect(toggleMaximizeMock).toHaveBeenCalledTimes(1)
    expect(closeWindowMock).toHaveBeenCalledTimes(1)
  })
  // Theme, contrast, zoom, typography and language moved to Configuración →
  // Apariencia (user decision, 2026-09-24; see AppearanceTab.test.ts).
  it('no longer renders theme, contrast, zoom, typography or language controls', () => {
    render(TopBar)

    expect(screen.queryByRole('button', { name: 'Oscuro' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Contraste normal' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Zoom' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Idioma' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^Tipografía/ })).not.toBeInTheDocument()
  })

  it('updates translated top bar labels when locale changes', async () => {
    render(TopBar)

    locale.set('en')

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Open settings' })).toBeInTheDocument()
      expect(
        screen.getByRole('combobox', { name: 'Search documents by name or text' })
      ).toBeInTheDocument()
    })
  })

  it('uses an icon-only clear button for global search', async () => {
    render(TopBar)

    const input = screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })
    await fireEvent.input(input, { target: { value: 'acta' } })

    expect(input).toHaveAttribute('type', 'text')
    const clearButton = screen.getByRole('button', { name: 'Limpiar búsqueda' })
    expect(clearButton).not.toHaveTextContent('×')
    expect(clearButton).toHaveAttribute('data-tooltip', 'Limpiar búsqueda')
    expect(clearButton).toHaveClass(
      'icon-button',
      'icon-button--ghost',
      'icon-button--sm',
      'search-clear-button'
    )
    expect(clearButton.querySelector('svg')).toHaveAttribute('width', '14')
  })

  it('shows results and navigates to the selected item', async () => {
    storeRef.current.items.searchGlobal.mockResolvedValueOnce([
      { id: 'item-1', title: 'Acta fundacional', collectionId: 'col-1' },
    ])
    storeRef.current.collections.findById.mockResolvedValueOnce({
      id: 'col-1',
      name: 'Archivo',
    })

    render(TopBar)

    const input = screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })
    await fireEvent.input(input, { target: { value: 'acta' } })
    vi.advanceTimersByTime(300)

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /Acta fundacional/i })).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('option', { name: /Acta fundacional/i }))

    expect(navigateActiveMock).toHaveBeenNthCalledWith(1, {
      name: 'collection',
      id: 'col-1',
      collectionName: 'Archivo',
    })
    expect(navigateActiveMock).toHaveBeenNthCalledWith(2, {
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Archivo',
      itemId: 'item-1',
      itemTitle: 'Acta fundacional',
    })
  })

  it('exposes combobox semantics for the global search dropdown', async () => {
    storeRef.current.items.searchGlobal.mockResolvedValueOnce([
      { id: 'item-1', title: 'Acta fundacional', collectionId: 'col-1' },
    ])
    storeRef.current.collections.findById.mockResolvedValueOnce({
      id: 'col-1',
      name: 'Archivo',
    })

    render(TopBar)

    const input = screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })
    expect(input).toHaveAttribute('aria-expanded', 'false')

    await fireEvent.input(input, { target: { value: 'acta' } })
    vi.advanceTimersByTime(300)

    await waitFor(() => {
      expect(
        screen.getByRole('listbox', { name: 'Buscar documentos por nombre o texto' })
      ).toBeInTheDocument()
    })

    expect(input).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByRole('option', { name: /Acta fundacional/i })).toBeInTheDocument()
  })

  it('navigates global search results with arrow keys and selects with Enter', async () => {
    storeRef.current.items.searchGlobal.mockResolvedValueOnce([
      { id: 'item-1', title: 'Acta fundacional', collectionId: 'col-1' },
      { id: 'item-2', title: 'Acta vigente', collectionId: 'col-1' },
    ])
    storeRef.current.collections.findById.mockResolvedValue({
      id: 'col-1',
      name: 'Archivo',
    })

    render(TopBar)

    const input = screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })
    await fireEvent.input(input, { target: { value: 'acta' } })
    vi.advanceTimersByTime(300)

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /Acta fundacional/i })).toBeInTheDocument()
    })

    await fireEvent.keyDown(input, { key: 'ArrowDown' })
    expect(screen.getByRole('option', { name: /Acta fundacional/i })).toHaveAttribute(
      'aria-selected',
      'true'
    )
    expect(input).toHaveAttribute('aria-activedescendant', 'topbar-global-search-listbox-option-0')

    await fireEvent.keyDown(input, { key: 'ArrowDown' })
    expect(screen.getByRole('option', { name: /Acta vigente/i })).toHaveAttribute(
      'aria-selected',
      'true'
    )

    await fireEvent.keyDown(input, { key: 'Enter' })

    expect(navigateActiveMock).toHaveBeenNthCalledWith(2, {
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Archivo',
      itemId: 'item-2',
      itemTitle: 'Acta vigente',
    })
  })

  it('does not select a global search result on Enter while IME composition is active', async () => {
    storeRef.current.items.searchGlobal.mockResolvedValueOnce([
      { id: 'item-1', title: 'Acta fundacional', collectionId: 'col-1' },
    ])
    storeRef.current.collections.findById.mockResolvedValue({
      id: 'col-1',
      name: 'Archivo',
    })

    render(TopBar)

    const input = screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })
    await fireEvent.input(input, { target: { value: 'acta' } })
    vi.advanceTimersByTime(300)

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /Acta fundacional/i })).toBeInTheDocument()
    })

    await fireEvent.keyDown(input, { key: 'ArrowDown' })
    await fireEvent.keyDown(input, { key: 'Enter', isComposing: true })

    expect(navigateActiveMock).not.toHaveBeenCalled()
  })

  it('keeps results open while focus moves within the search container', async () => {
    storeRef.current.items.searchGlobal.mockResolvedValueOnce([
      { id: 'item-1', title: 'Acta fundacional', collectionId: 'col-1' },
    ])
    storeRef.current.collections.findById.mockResolvedValueOnce({
      id: 'col-1',
      name: 'Archivo',
    })

    render(TopBar)

    const input = screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })
    await fireEvent.input(input, { target: { value: 'acta' } })
    vi.advanceTimersByTime(300)

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /Acta fundacional/i })).toBeInTheDocument()
    })

    const option = screen.getByRole('option', { name: /Acta fundacional/i })
    await fireEvent.focusOut(input, { relatedTarget: option })
    expect(screen.getByRole('option', { name: /Acta fundacional/i })).toBeInTheDocument()

    await fireEvent.focusOut(input, { relatedTarget: document.body })
    expect(screen.queryByRole('option', { name: /Acta fundacional/i })).not.toBeInTheDocument()
  })

  it('ignores stale global search results when a newer query finishes first', async () => {
    const firstSearch = deferred<Array<{ id: string; title: string; collectionId: string }>>()
    const secondSearch = deferred<Array<{ id: string; title: string; collectionId: string }>>()

    storeRef.current.items.searchGlobal
      .mockReturnValueOnce(firstSearch.promise)
      .mockReturnValueOnce(secondSearch.promise)
    storeRef.current.collections.findById.mockResolvedValue({
      id: 'col-1',
      name: 'Archivo',
    })

    render(TopBar)

    const input = screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })
    await fireEvent.input(input, { target: { value: 'acta' } })
    vi.advanceTimersByTime(300)

    await waitFor(() => {
      expect(storeRef.current.items.searchGlobal).toHaveBeenCalledWith('acta', 20)
    })

    await fireEvent.input(input, { target: { value: 'vigente' } })
    vi.advanceTimersByTime(300)

    secondSearch.resolve([{ id: 'item-new', title: 'Acta vigente', collectionId: 'col-1' }])

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /Acta vigente/i })).toBeInTheDocument()
    })

    firstSearch.resolve([{ id: 'item-old', title: 'Acta vieja', collectionId: 'col-1' }])
    await Promise.resolve()

    expect(screen.getByRole('option', { name: /Acta vigente/i })).toBeInTheDocument()
    expect(screen.queryByRole('option', { name: /Acta vieja/i })).not.toBeInTheDocument()
  })

  it('shows a localized error when the current global search fails', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    storeRef.current.items.searchGlobal.mockRejectedValueOnce(new Error('search failed'))

    render(TopBar)

    const input = screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })
    await fireEvent.input(input, { target: { value: 'acta' } })
    await vi.advanceTimersByTimeAsync(300)

    await waitFor(() => {
      expect(
        screen.getByText('No se pudo completar la búsqueda. Probá de nuevo.')
      ).toBeInTheDocument()
    })

    expect(consoleErrorSpy).toHaveBeenCalledWith('[Search] error:', expect.any(Error))
    consoleErrorSpy.mockRestore()
  })

  it('ignores stale global search failures after a newer query succeeds', async () => {
    const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
    const firstSearch = deferred<Array<{ id: string; title: string; collectionId: string }>>()
    const secondSearch = deferred<Array<{ id: string; title: string; collectionId: string }>>()

    storeRef.current.items.searchGlobal
      .mockReturnValueOnce(firstSearch.promise)
      .mockReturnValueOnce(secondSearch.promise)
    storeRef.current.collections.findById.mockResolvedValue({
      id: 'col-1',
      name: 'Archivo',
    })

    render(TopBar)

    const input = screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })
    await fireEvent.input(input, { target: { value: 'acta' } })
    vi.advanceTimersByTime(300)

    await waitFor(() => {
      expect(storeRef.current.items.searchGlobal).toHaveBeenCalledWith('acta', 20)
    })

    await fireEvent.input(input, { target: { value: 'vigente' } })
    vi.advanceTimersByTime(300)

    secondSearch.resolve([{ id: 'item-new', title: 'Acta vigente', collectionId: 'col-1' }])

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /Acta vigente/i })).toBeInTheDocument()
    })

    firstSearch.reject(new Error('stale search failed'))
    await Promise.resolve()

    expect(screen.getByRole('option', { name: /Acta vigente/i })).toBeInTheDocument()
    expect(
      screen.queryByText('No se pudo completar la búsqueda. Probá de nuevo.')
    ).not.toBeInTheDocument()
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })
})
