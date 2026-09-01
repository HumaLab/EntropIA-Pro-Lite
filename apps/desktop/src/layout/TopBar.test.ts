import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import TopBar from './TopBar.svelte'
import { locale } from '$lib/i18n'
import type { View } from '$lib/navigation'
import {
  DOCUMENT_ASSET_DELETED_EVENT,
  DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
} from '$lib/document-explorer'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })

  return { promise, resolve, reject }
}

type NavigationSnapshot = {
  history: View[]
  current: View
  canGoBack: boolean
  breadcrumb: string[]
}

const {
  navigationStore,
  setNavigationState,
  navigateMock,
  replaceMock,
  resetToPathMock,
  openRootSectionMock,
  backMock,
  invokeMock,
  removeMock,
  deleteAssetFileMock,
  deleteImageThumbnailMock,
  deletePdfThumbnailMock,
  storeRef,
  minimizeMock,
  toggleMaximizeMock,
  closeWindowMock,
} = vi.hoisted(() => {
  let current: NavigationSnapshot = {
    history: [{ name: 'collections' as const }],
    current: { name: 'collections' as const },
    canGoBack: false,
    breadcrumb: ['Collections'],
  }
  const subscribers = new Set<(value: NavigationSnapshot) => void>()

  return {
    navigationStore: {
      subscribe(run: (value: NavigationSnapshot) => void) {
        subscribers.add(run)
        run(current)
        return () => subscribers.delete(run)
      },
    },
    setNavigationState(value: typeof current) {
      current = value
      subscribers.forEach((run) => run(current))
    },
    navigateMock: vi.fn(),
    replaceMock: vi.fn(),
    resetToPathMock: vi.fn(),
    openRootSectionMock: vi.fn(),
    backMock: vi.fn(),
    invokeMock: vi.fn(),
    removeMock: vi.fn(),
    deleteAssetFileMock: vi.fn(),
    deleteImageThumbnailMock: vi.fn(),
    deletePdfThumbnailMock: vi.fn(),
    storeRef: {
      current: {
        items: { searchGlobal: vi.fn(), findByCollection: vi.fn() },
        collections: { findById: vi.fn() },
        assets: { findByItem: vi.fn(), deleteWithCascade: vi.fn() },
      },
    },
    minimizeMock: vi.fn(),
    toggleMaximizeMock: vi.fn(),
    closeWindowMock: vi.fn(),
  }
})

vi.mock('$lib/navigation', () => ({
  navigation: {
    subscribe: navigationStore.subscribe,
    navigate: navigateMock,
    replace: replaceMock,
    resetToPath: resetToPathMock,
    openRootSection: openRootSectionMock,
    back: backMock,
  },
}))

vi.mock('$lib/db', () => ({
  getStore: () => storeRef.current,
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

vi.mock('@tauri-apps/plugin-fs', () => ({
  remove: removeMock,
}))

vi.mock('$lib/file-import', () => ({
  deleteAssetFile: deleteAssetFileMock,
  deleteImageThumbnail: deleteImageThumbnailMock,
  deletePdfThumbnail: deletePdfThumbnailMock,
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
    navigateMock.mockReset()
    replaceMock.mockReset()
    resetToPathMock.mockReset()
    openRootSectionMock.mockReset()
    backMock.mockReset()
    invokeMock.mockReset().mockResolvedValue(undefined)
    removeMock.mockReset().mockResolvedValue(undefined)
    deleteAssetFileMock.mockReset().mockResolvedValue(undefined)
    deleteImageThumbnailMock.mockReset().mockResolvedValue(undefined)
    deletePdfThumbnailMock.mockReset().mockResolvedValue(undefined)
    minimizeMock.mockReset()
    toggleMaximizeMock.mockReset()
    closeWindowMock.mockReset()
    storeRef.current.items.searchGlobal.mockReset()
    storeRef.current.items.findByCollection.mockReset()
    storeRef.current.collections.findById.mockReset()
    storeRef.current.assets.findByItem.mockReset()
    storeRef.current.assets.deleteWithCascade.mockReset()
    storeRef.current.items.findByCollection.mockResolvedValue([
      { id: 'item-0', title: 'Acta 0', collectionId: 'col-1' },
      { id: 'item-1', title: 'Acta 1', collectionId: 'col-1' },
      { id: 'item-2', title: 'Acta 2', collectionId: 'col-1' },
    ])
    setNavigationState({
      history: [
        { name: 'collections' },
        { name: 'collection', id: 'col-1', collectionName: 'Archivo' },
      ],
      current: { name: 'collection', id: 'col-1', collectionName: 'Archivo' },
      canGoBack: true,
      breadcrumb: ['Collections', 'Archivo'],
    })
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
    expect(screen.getByRole('button', { name: 'Oscuro' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Abrir configuración' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Minimizar ventana' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Maximizar o restaurar ventana' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Cerrar ventana' })).toBeInTheDocument()
    expect(screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })).toBeInTheDocument()
    expect(screen.getByRole('navigation', { name: 'Breadcrumb' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Documento anterior' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Documento siguiente' })).not.toBeInTheDocument()
  })

  it('navigates through parent breadcrumb segments while keeping the asset as a leaf', async () => {
    setNavigationState({
      history: [
        { name: 'collections' },
        { name: 'collection', id: 'col-1', collectionName: 'Archivo' },
        {
          name: 'item',
          collectionId: 'col-1',
          collectionName: 'Archivo',
          itemId: 'item-1',
          itemTitle: 'Acta 1',
          assetId: 'asset-1',
          assetLabel: 'acta-1.pdf',
        },
      ],
      current: {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Archivo',
        itemId: 'item-1',
        itemTitle: 'Acta 1',
        assetId: 'asset-1',
        assetLabel: 'acta-1.pdf',
      },
      canGoBack: true,
      breadcrumb: ['Colecciones', 'Archivo', 'acta-1.pdf'],
    })

    render(TopBar)

    const collectionsCrumb = screen.getByRole('button', { name: 'Colecciones' })
    const collectionCrumb = screen.getByRole('button', { name: 'Archivo' })
    const assetCrumb = screen.getByText('acta-1.pdf').closest('[aria-current="page"]')

    expect(assetCrumb).not.toBeNull()
    expect(screen.queryByText('Acta 1')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'acta-1.pdf' })).not.toBeInTheDocument()

    await fireEvent.click(collectionsCrumb)
    expect(resetToPathMock).toHaveBeenLastCalledWith([{ name: 'collections' }])

    await fireEvent.click(collectionCrumb)
    expect(resetToPathMock).toHaveBeenLastCalledWith([
      { name: 'collections' },
      { name: 'collection', id: 'col-1', collectionName: 'Archivo' },
    ])
  })

  it('deletes the active asset and selects the next remaining asset', async () => {
    const currentAsset = {
      id: 'asset-1',
      itemId: 'item-1',
      path: 'docs/11111111-1111-4111-8111-111111111111_acta-1.png',
      type: 'image',
      size: 10,
      sortIndex: 0,
      createdAt: 1,
      parentAssetId: null,
    }
    const nextAsset = {
      ...currentAsset,
      id: 'asset-2',
      path: 'docs/22222222-2222-4222-8222-222222222222_acta-2.png',
      sortIndex: 1,
    }
    storeRef.current.assets.findByItem
      .mockResolvedValueOnce([currentAsset, nextAsset])
      .mockResolvedValueOnce([nextAsset])
    storeRef.current.assets.deleteWithCascade.mockResolvedValue(currentAsset)
    setNavigationState({
      history: [],
      current: {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Archivo',
        itemId: 'item-1',
        itemTitle: 'Acta 1',
        assetId: 'asset-1',
        assetLabel: 'acta-1.png',
      },
      canGoBack: true,
      breadcrumb: ['Colecciones', 'Archivo', 'acta-1.png'],
    })
    const deletedEvents: Event[] = []
    const changedEvents: Event[] = []
    const onDeleted = (event: Event) => deletedEvents.push(event)
    const onChanged = (event: Event) => changedEvents.push(event)
    window.addEventListener(DOCUMENT_ASSET_DELETED_EVENT, onDeleted)
    window.addEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, onChanged)

    try {
      render(TopBar)
      await fireEvent.click(screen.getByRole('button', { name: 'Eliminar asset activo' }))
      expect(screen.getByText(/¿Seguro que querés eliminar acta-1\.png\?/)).toBeInTheDocument()

      await fireEvent.click(screen.getByRole('button', { name: 'Eliminar asset' }))

      await waitFor(() => {
        expect(storeRef.current.assets.deleteWithCascade).toHaveBeenCalledWith('asset-1')
      })
      expect(invokeMock).toHaveBeenCalledWith('delete_asset_files', {
        assetPath: currentAsset.path,
      })
      expect(deleteImageThumbnailMock).toHaveBeenCalledWith('asset-1')
      await waitFor(() => {
        expect(replaceMock).toHaveBeenCalledWith({
          name: 'item',
          collectionId: 'col-1',
          collectionName: 'Archivo',
          itemId: 'item-1',
          itemTitle: 'Acta 1',
          assetId: 'asset-2',
          assetLabel: 'acta-2.png',
        })
      })
      expect(deletedEvents).toHaveLength(1)
      expect(changedEvents).toHaveLength(1)
    } finally {
      window.removeEventListener(DOCUMENT_ASSET_DELETED_EVENT, onDeleted)
      window.removeEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, onChanged)
    }
  })

  it('returns to the collection after deleting the last asset', async () => {
    const currentAsset = {
      id: 'asset-1',
      itemId: 'item-1',
      path: 'docs/acta-1.pdf',
      type: 'pdf',
      size: 10,
      sortIndex: 0,
      createdAt: 1,
      parentAssetId: null,
    }
    storeRef.current.assets.findByItem
      .mockResolvedValueOnce([currentAsset])
      .mockResolvedValueOnce([])
    storeRef.current.assets.deleteWithCascade.mockResolvedValue(currentAsset)
    setNavigationState({
      history: [],
      current: {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Archivo',
        itemId: 'item-1',
        itemTitle: 'Acta 1',
        assetId: 'asset-1',
        assetLabel: 'acta-1.pdf',
      },
      canGoBack: true,
      breadcrumb: ['Colecciones', 'Archivo', 'acta-1.pdf'],
    })

    render(TopBar)
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar asset activo' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Eliminar asset' }))

    await waitFor(() => {
      expect(resetToPathMock).toHaveBeenCalledWith([
        { name: 'collections' },
        { name: 'collection', id: 'col-1', collectionName: 'Archivo' },
      ])
    })
    expect(deleteAssetFileMock).toHaveBeenCalledWith('docs/acta-1.pdf')
    expect(deletePdfThumbnailMock).toHaveBeenCalledWith('asset-1')
    expect(removeMock).toHaveBeenCalledWith('docs/acta-1.pages', { recursive: true })
  })

  it('navigates to db browser from the database icon button', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir navegador de base de datos' }))

    expect(openRootSectionMock).toHaveBeenCalledWith({ name: 'db-browser' })
  })

  it('navigates to the research chat from the chat icon button', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir chat de investigación' }))

    expect(openRootSectionMock).toHaveBeenCalledWith({ name: 'rag-chat' })
  })

  it('opens settings as a canonical root section', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir configuración' }))

    expect(openRootSectionMock).toHaveBeenCalledWith({ name: 'settings' })
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
  it('toggles and persists the less dark theme from the topbar', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Oscuro' }))

    expect(document.documentElement.dataset.theme).toBe('dim')
    expect(localStorage.getItem('entropia-theme')).toBe('dim')
    // El botón cicla entre tres temas, no es un interruptor de dos estados:
    // su etiqueta anuncia el próximo tema y nunca se marca como presionado.
    expect(screen.getByRole('button', { name: 'Cálido' })).not.toHaveAttribute('aria-pressed')
  })

  it('updates translated top bar labels when locale changes', async () => {
    render(TopBar)

    locale.set('en')

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Open settings' })).toBeInTheDocument()
      expect(screen.getByRole('combobox', { name: 'Search documents by name or text' })).toBeInTheDocument()
    })
  })

  it('changes the interface language from the topbar selector', async () => {
    render(TopBar)

    await fireEvent.click(screen.getByRole('button', { name: 'Idioma' }))
    expect(screen.getByRole('menu', { name: 'Idioma' })).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('menuitemradio', { name: 'EN' }))

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Language' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Open settings' })).toBeInTheDocument()
    })
  })

  it('uses an icon-only clear button for global search', async () => {
    render(TopBar)

    const input = screen.getByRole('combobox', { name: 'Buscar documentos por nombre o texto' })
    await fireEvent.input(input, { target: { value: 'acta' } })

    expect(input).toHaveAttribute('type', 'text')
    const clearButton = screen.getByRole('button', { name: 'Limpiar búsqueda' })
    expect(clearButton).not.toHaveTextContent('×')
    expect(clearButton).toHaveAttribute('title', 'Limpiar búsqueda')
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

    expect(navigateMock).toHaveBeenNthCalledWith(1, {
      name: 'collection',
      id: 'col-1',
      collectionName: 'Archivo',
    })
    expect(navigateMock).toHaveBeenNthCalledWith(2, {
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
      expect(screen.getByRole('listbox', { name: 'Buscar documentos por nombre o texto' })).toBeInTheDocument()
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
    expect(input).toHaveAttribute(
      'aria-activedescendant',
      'topbar-global-search-listbox-option-0'
    )

    await fireEvent.keyDown(input, { key: 'ArrowDown' })
    expect(screen.getByRole('option', { name: /Acta vigente/i })).toHaveAttribute(
      'aria-selected',
      'true'
    )

    await fireEvent.keyDown(input, { key: 'Enter' })

    expect(navigateMock).toHaveBeenNthCalledWith(2, {
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

    expect(navigateMock).not.toHaveBeenCalled()
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

    secondSearch.resolve([
      { id: 'item-new', title: 'Acta vigente', collectionId: 'col-1' },
    ])

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /Acta vigente/i })).toBeInTheDocument()
    })

    firstSearch.resolve([
      { id: 'item-old', title: 'Acta vieja', collectionId: 'col-1' },
    ])
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
      expect(screen.getByText('No se pudo completar la búsqueda. Probá de nuevo.')).toBeInTheDocument()
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

    secondSearch.resolve([
      { id: 'item-new', title: 'Acta vigente', collectionId: 'col-1' },
    ])

    await waitFor(() => {
      expect(screen.getByRole('option', { name: /Acta vigente/i })).toBeInTheDocument()
    })

    firstSearch.reject(new Error('stale search failed'))
    await Promise.resolve()

    expect(screen.getByRole('option', { name: /Acta vigente/i })).toBeInTheDocument()
    expect(screen.queryByText('No se pudo completar la búsqueda. Probá de nuevo.')).not.toBeInTheDocument()
    expect(consoleErrorSpy).not.toHaveBeenCalled()
    consoleErrorSpy.mockRestore()
  })

  it('renders sibling document controls and replaces navigation within the same collection', async () => {
    setNavigationState({
      history: [
        { name: 'collections' },
        { name: 'collection', id: 'col-1', collectionName: 'Archivo' },
        {
          name: 'item',
          collectionId: 'col-1',
          collectionName: 'Archivo',
          itemId: 'item-1',
          itemTitle: 'Acta 1',
        },
      ],
      current: {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Archivo',
        itemId: 'item-1',
        itemTitle: 'Acta 1',
      },
      canGoBack: true,
      breadcrumb: ['Collections', 'Archivo', 'Acta 1'],
    })

    render(TopBar)

    const previousButton = await screen.findByRole('button', { name: 'Documento anterior' })
    const nextButton = await screen.findByRole('button', { name: 'Documento siguiente' })

    expect(previousButton).toBeEnabled()
    expect(nextButton).toBeEnabled()

    await fireEvent.click(nextButton)

    expect(replaceMock).toHaveBeenCalledWith({
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Archivo',
      itemId: 'item-2',
      itemTitle: 'Acta 2',
    })
  })

  it('disables sibling controls at collection boundaries', async () => {
    storeRef.current.items.findByCollection.mockResolvedValueOnce([
      { id: 'item-1', title: 'Acta 1', collectionId: 'col-1' },
      { id: 'item-2', title: 'Acta 2', collectionId: 'col-1' },
    ])
    setNavigationState({
      history: [
        { name: 'collections' },
        { name: 'collection', id: 'col-1', collectionName: 'Archivo' },
        {
          name: 'item',
          collectionId: 'col-1',
          collectionName: 'Archivo',
          itemId: 'item-1',
          itemTitle: 'Acta 1',
        },
      ],
      current: {
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Archivo',
        itemId: 'item-1',
        itemTitle: 'Acta 1',
      },
      canGoBack: true,
      breadcrumb: ['Collections', 'Archivo', 'Acta 1'],
    })

    render(TopBar)

    expect(await screen.findByRole('button', { name: 'Documento anterior' })).toBeDisabled()
    expect(await screen.findByRole('button', { name: 'Documento siguiente' })).toBeEnabled()
  })
})
