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

const { navigateActiveMock, storeRef, minimizeMock, toggleMaximizeMock, closeWindowMock } =
  vi.hoisted(() => {
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
    }
  })

// TopBar now hosts `TabStrip`, which reads `workspace.tabs`/`activeTabId`
// through `workspace.subscribe`. A single, static Home tab is enough here —
// TabStrip's own behavior (adding, closing, grouping tabs) is covered by
// TabStrip.test.ts, not this file.
vi.mock('$lib/workspace', () => ({
  MAX_TABS: 4,
  workspace: {
    tabs: [{ id: 'tab-1', navigation: { current: { name: 'home' as const } } }],
    activeTabId: 'tab-1',
    subscribe(run: (value: unknown) => void) {
      run({
        tabs: [{ id: 'tab-1', navigation: { current: { name: 'home' as const } } }],
        activeTabId: 'tab-1',
        split: null,
      })
      return () => {}
    },
    activateTab: vi.fn(),
    closeTab: vi.fn(),
    openTab: vi.fn(),
    navigateActive: navigateActiveMock,
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

  it('opens settings as a canonical root section', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir configuración' }))

    expect(navigateActiveMock).toHaveBeenCalledWith({ name: 'settings' })
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
