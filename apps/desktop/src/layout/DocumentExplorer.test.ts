import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import DocumentExplorer from './DocumentExplorer.svelte'

const state = vi.hoisted(() => {
  const subscribers = new Set<(value: unknown) => void>()

  const snapshot = {
    history: [
      { name: 'collections' as const },
      { name: 'collection' as const, id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item' as const,
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Acta 1',
      },
    ],
    current: {
      name: 'item' as const,
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-1',
      itemTitle: 'Acta 1',
    },
    canGoBack: true,
    breadcrumb: ['Colecciones', 'Colección 1', 'Acta 1'],
  }

  const store = {
    collections: {
      findAll: vi.fn().mockResolvedValue([
        { id: 'col-1', name: 'Colección 1', description: null, createdAt: 1, updatedAt: 1 },
        { id: 'col-2', name: 'Colección 2', description: null, createdAt: 1, updatedAt: 1 },
      ]),
      countItems: vi.fn().mockImplementation(async (id: string) => (id === 'col-1' ? 2 : 1)),
    },
    items: {
      findCardSummariesByCollection: vi.fn().mockImplementation(async (collectionId: string) => {
        if (collectionId === 'col-2') {
          return [
            {
              id: 'item-3',
              title: 'Acta 3',
              collectionId: 'col-2',
              metadata: null,
              createdAt: 1,
              updatedAt: 3,
              assetCount: 1,
              primaryAssetId: 'asset-4',
              primaryAssetPath: 'docs/acta-3.pdf',
              primaryAssetType: 'pdf',
            },
          ]
        }

        return [
          {
            id: 'item-1',
            title: 'Acta 1',
            collectionId: 'col-1',
            metadata: null,
            createdAt: 1,
            updatedAt: 2,
            assetCount: 2,
            primaryAssetId: 'asset-1',
            primaryAssetPath: 'docs/acta-1.pdf',
            primaryAssetType: 'pdf',
          },
          {
            id: 'item-2',
            title: 'Acta 2',
            collectionId: 'col-1',
            metadata: null,
            createdAt: 1,
            updatedAt: 1,
            assetCount: 1,
            primaryAssetId: 'asset-3',
            primaryAssetPath: 'docs/foto-acta-2.png',
            primaryAssetType: 'image',
          },
        ]
      }),
      findByCollection: vi.fn().mockImplementation(async (collectionId: string) => {
        if (collectionId === 'col-2') {
          return [
            {
              id: 'item-3',
              title: 'Acta 3',
              collectionId: 'col-2',
              metadata: null,
              createdAt: 1,
              updatedAt: 3,
            },
          ]
        }

        return [
          {
            id: 'item-1',
            title: 'Acta 1',
            collectionId: 'col-1',
            metadata: null,
            createdAt: 1,
            updatedAt: 2,
          },
          {
            id: 'item-2',
            title: 'Acta 2',
            collectionId: 'col-1',
            metadata: null,
            createdAt: 1,
            updatedAt: 1,
          },
        ]
      }),
    },
    assets: {
      findByItem: vi.fn().mockImplementation(async (itemId: string) => {
        if (itemId === 'item-2') {
          return [
            {
              id: 'asset-3',
              itemId: 'item-2',
              path: 'docs/foto-acta-2.png',
              type: 'image',
              size: 12,
              sortIndex: 0,
              createdAt: 1,
            },
          ]
        }

        if (itemId === 'item-3') {
          return [
            {
              id: 'asset-4',
              itemId: 'item-3',
              path: 'docs/acta-3.pdf',
              type: 'pdf',
              size: 14,
              sortIndex: 0,
              createdAt: 1,
            },
          ]
        }

        return [
          {
            id: 'asset-1',
            itemId: 'item-1',
            path: 'docs/acta-1.pdf',
            type: 'pdf',
            size: 10,
            sortIndex: 0,
            createdAt: 1,
          },
          {
            id: 'asset-2',
            itemId: 'item-1',
            path: 'docs/11111111-1111-4111-8111-111111111111_acta-1-audio.mp3',
            type: 'audio',
            size: 10,
            sortIndex: 1,
            createdAt: 1,
          },
        ]
      }),
    },
  }

  function emit() {
    const payload = {
      history: [...snapshot.history],
      current: { ...snapshot.current },
      canGoBack: snapshot.canGoBack,
      breadcrumb: [...snapshot.breadcrumb],
    }
    subscribers.forEach((run) => run(payload))
  }

  return {
    subscribers,
    snapshot,
    store,
    navigate: vi.fn(),
    replace: vi.fn(),
    resetToPath: vi.fn(),
    emit,
  }
})

vi.mock('$lib/navigation', () => ({
  navigation: {
    subscribe(run: (value: unknown) => void) {
      state.subscribers.add(run)
      state.emit()
      return () => state.subscribers.delete(run)
    },
    navigate: state.navigate,
    replace: state.replace,
    resetToPath: state.resetToPath,
  },
}))

vi.mock('$lib/db', () => ({
  getStore: () => state.store,
}))

function persistOpenTree(collections: string[] = [], items: string[] = []) {
  localStorage.setItem(
    'entropia-document-explorer-tree',
    JSON.stringify({
      collections,
      items,
    })
  )
}

function setCurrentNavigationView(current: (typeof state.snapshot.history)[number]) {
  state.snapshot.current = current as typeof state.snapshot.current
  state.snapshot.history = [{ name: 'collections' as const }, current]
  state.snapshot.canGoBack = true
  state.emit()
}

describe('DocumentExplorer', () => {
  beforeEach(() => {
    locale.set('es')
    localStorage.clear()
    state.snapshot.history = [
      { name: 'collections' as const },
      { name: 'collection' as const, id: 'col-1', collectionName: 'Colección 1' },
      {
        name: 'item' as const,
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-1',
        itemTitle: 'Acta 1',
      },
    ]
    state.snapshot.current = {
      name: 'item' as const,
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-1',
      itemTitle: 'Acta 1',
    }
    state.snapshot.canGoBack = true
    state.snapshot.breadcrumb = ['Colecciones', 'Colección 1', 'Acta 1']
    state.navigate.mockReset()
    state.replace.mockReset()
    state.resetToPath.mockReset()
    state.store.collections.findAll.mockClear()
    state.store.collections.countItems.mockClear()
    state.store.items.findCardSummariesByCollection.mockClear()
    state.store.items.findByCollection.mockClear()
    state.store.assets.findByItem.mockClear()
  })

  it('expands collection nodes without navigating and lazy-loads documents', async () => {
    render(DocumentExplorer)

    const expandCollection = await screen.findByRole('button', {
      name: 'Expandir colección Colección 2',
    })

    await fireEvent.click(expandCollection)

    expect(state.navigate).not.toHaveBeenCalled()
    expect(state.replace).not.toHaveBeenCalled()

    await waitFor(() => {
      expect(state.store.items.findCardSummariesByCollection).toHaveBeenCalledWith('col-2')
    })

    expect(await screen.findByRole('treeitem', { name: 'Acta 3' })).toBeInTheDocument()
    expect(state.store.assets.findByItem).not.toHaveBeenCalledWith('item-3')
  })

  it('renders active hierarchy and replaces sibling item navigation', async () => {
    persistOpenTree(['col-1'], ['item-1'])
    render(DocumentExplorer)

    await screen.findByText('Colección 1')
    await screen.findByText('Acta 2')
    await screen.findByText('acta-1.pdf')

    await fireEvent.click(screen.getByRole('button', { name: 'Acta 2' }))

    expect(state.replace).toHaveBeenCalledWith({
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-2',
      itemTitle: 'Acta 2',
      assetId: 'asset-3',
      assetLabel: 'foto-acta-2.png',
    })
    expect(state.resetToPath).not.toHaveBeenCalled()
  })

  it('rebuilds canonical path when clicking a collection from another collection', async () => {
    render(DocumentExplorer)

    const collectionButton = (await screen.findByText('Colección 2')).closest('button')

    if (!collectionButton) {
      throw new Error('Expected collection button to be rendered')
    }

    await fireEvent.click(collectionButton)

    expect(state.resetToPath).toHaveBeenCalledWith([
      { name: 'collections' },
      { name: 'collection', id: 'col-2', collectionName: 'Colección 2' },
    ])
    expect(state.replace).not.toHaveBeenCalled()
    expect(state.navigate).not.toHaveBeenCalled()
  })

  it('does not append the same folder when repeating file and folder clicks', async () => {
    persistOpenTree(['col-1'])

    render(DocumentExplorer)

    const collectionButton = (await screen.findByText('Colección 1')).closest('button')

    if (!collectionButton) {
      throw new Error('Expected collection button to be rendered')
    }

    await screen.findByText('Acta 2')
    expect(state.store.assets.findByItem).not.toHaveBeenCalledWith('item-2')

    await fireEvent.click(screen.getByRole('button', { name: 'Acta 2' }))

    expect(state.replace).toHaveBeenLastCalledWith({
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-2',
      itemTitle: 'Acta 2',
      assetId: 'asset-3',
      assetLabel: 'foto-acta-2.png',
    })

    setCurrentNavigationView({
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-2',
      itemTitle: 'Acta 2',
    })

    state.navigate.mockClear()
    state.replace.mockClear()
    state.resetToPath.mockClear()

    await fireEvent.click(collectionButton)

    expect(state.replace).toHaveBeenLastCalledWith({
      name: 'collection',
      id: 'col-1',
      collectionName: 'Colección 1',
    })
    expect(state.navigate).not.toHaveBeenCalled()
    expect(state.resetToPath).not.toHaveBeenCalled()

    setCurrentNavigationView({ name: 'collection', id: 'col-1', collectionName: 'Colección 1' })

    state.navigate.mockClear()
    state.replace.mockClear()
    state.resetToPath.mockClear()

    await fireEvent.click(screen.getByRole('button', { name: 'Acta 2' }))
    expect(state.navigate).toHaveBeenLastCalledWith({
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-2',
      itemTitle: 'Acta 2',
      assetId: 'asset-3',
      assetLabel: 'foto-acta-2.png',
    })

    setCurrentNavigationView({
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-2',
      itemTitle: 'Acta 2',
    })

    state.navigate.mockClear()
    state.replace.mockClear()
    state.resetToPath.mockClear()

    await fireEvent.click(collectionButton)

    expect(state.replace).toHaveBeenLastCalledWith({
      name: 'collection',
      id: 'col-1',
      collectionName: 'Colección 1',
    })
    expect(state.navigate).not.toHaveBeenCalled()
    expect(state.resetToPath).not.toHaveBeenCalled()
  })

  it('rebuilds canonical path when clicking an item from another collection', async () => {
    render(DocumentExplorer)

    await fireEvent.click(
      await screen.findByRole('button', {
        name: 'Expandir colección Colección 2',
      })
    )

    const targetItem = (await screen.findByText('Acta 3')).closest('button')

    if (!targetItem) {
      throw new Error('Expected item button to be rendered')
    }

    await fireEvent.click(targetItem)

    expect(state.resetToPath).toHaveBeenCalledWith([
      { name: 'collections' },
      { name: 'collection', id: 'col-2', collectionName: 'Colección 2' },
      {
        name: 'item',
        collectionId: 'col-2',
        collectionName: 'Colección 2',
        itemId: 'item-3',
        itemTitle: 'Acta 3',
        assetId: 'asset-4',
        assetLabel: 'acta-3.pdf',
      },
    ])
    expect(state.replace).not.toHaveBeenCalled()
    expect(state.navigate).not.toHaveBeenCalled()
  })

  it('keeps multi-asset document nodes expandable and nested', async () => {
    persistOpenTree(['col-1'])
    setCurrentNavigationView({ name: 'collection', id: 'col-1', collectionName: 'Colección 1' })

    render(DocumentExplorer)

    const expandItem = await screen.findByRole('button', {
      name: 'Expandir documento Acta 1',
    })

    await fireEvent.click(expandItem)

    expect(state.navigate).not.toHaveBeenCalled()
    expect(state.replace).not.toHaveBeenCalled()

    expect(screen.getByRole('treeitem', { name: 'Acta 1' })).toHaveAttribute('aria-expanded', 'true')
    expect(await screen.findByRole('treeitem', { name: 'acta-1.pdf' })).toBeInTheDocument()
    expect(await screen.findByRole('treeitem', { name: 'acta-1-audio.mp3' })).toBeInTheDocument()
  })

  it('replaces current item with selected multi-asset breadcrumb context without repeating selection', async () => {
    persistOpenTree(['col-1'], ['item-1'])

    render(DocumentExplorer)

    const audioAssetButton = (await screen.findByText('acta-1-audio.mp3')).closest('button')

    if (!audioAssetButton) {
      throw new Error('Expected audio asset button to be rendered')
    }

    await fireEvent.click(audioAssetButton)

    const selectedAssetView = {
      name: 'item' as const,
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-1',
      itemTitle: 'Acta 1',
      assetId: 'asset-2',
      assetLabel: 'acta-1-audio.mp3',
    }

    expect(state.replace).toHaveBeenLastCalledWith(selectedAssetView)

    state.snapshot.current = selectedAssetView as typeof state.snapshot.current
    state.snapshot.breadcrumb = ['Colecciones', 'Colección 1', 'Acta 1', 'acta-1-audio.mp3']
    state.emit()
    state.replace.mockClear()

    await fireEvent.click(audioAssetButton)

    expect(state.replace).not.toHaveBeenCalled()
  })

  it('renders single-asset items as non-expandable document rows with asset context', async () => {
    persistOpenTree(['col-1'])

    render(DocumentExplorer)

    await screen.findByText('Acta 2')
    expect(state.store.assets.findByItem).not.toHaveBeenCalledWith('item-2')

    const singleAssetNode = screen.getByRole('treeitem', { name: 'Acta 2' })
    expect(singleAssetNode).toHaveAttribute('aria-level', '2')
    expect(singleAssetNode.querySelector('.explorer__row')).toHaveClass('explorer__row--item')
    expect(singleAssetNode.querySelector('.explorer__node')).toHaveClass('explorer__node--item')
    expect(singleAssetNode).not.toHaveAttribute('aria-expanded')
    expect(screen.queryByRole('button', { name: 'Expandir documento Acta 2' })).not.toBeInTheDocument()
    expect(screen.queryByRole('treeitem', { name: 'foto-acta-2.png' })).not.toBeInTheDocument()
    expect(screen.getByText('image')).toBeInTheDocument()
    expect(state.store.assets.findByItem).not.toHaveBeenCalledWith('item-2')
  })

  it('navigates single-asset rows with asset context instead of dispatching a select request', async () => {
    persistOpenTree(['col-1'])
    const assetSelectRequests: CustomEvent[] = []
    const handleAssetSelectRequest = (event: Event) => {
      assetSelectRequests.push(event as CustomEvent)
    }
    window.addEventListener(
      'entropia:document-explorer-asset-select-request',
      handleAssetSelectRequest
    )

    try {
      render(DocumentExplorer)

      await fireEvent.click(await screen.findByRole('button', { name: 'Acta 2' }))

      expect(state.replace).toHaveBeenLastCalledWith({
        name: 'item',
        collectionId: 'col-1',
        collectionName: 'Colección 1',
        itemId: 'item-2',
        itemTitle: 'Acta 2',
        assetId: 'asset-3',
        assetLabel: 'foto-acta-2.png',
      })
      expect(assetSelectRequests).toHaveLength(0)
    } finally {
      window.removeEventListener(
        'entropia:document-explorer-asset-select-request',
        handleAssetSelectRequest
      )
    }
  })

  it('derives visual indentation structurally from a shared tree level source', async () => {
    persistOpenTree(['col-1'], ['item-1'])

    render(DocumentExplorer)

    const collectionNode = await screen.findByRole('treeitem', { name: 'Colección 1' })
    const itemNode = await screen.findByRole('treeitem', { name: 'Acta 1' })
    const assetNode = await screen.findByRole('treeitem', { name: 'acta-1.pdf' })
    const singleAssetNode = screen.getByRole('treeitem', { name: 'Acta 2' })

    const collectionRow = collectionNode.querySelector('.explorer__row')
    const itemRow = itemNode.querySelector('.explorer__row')
    const assetRow = assetNode.querySelector('.explorer__row')
    const singleAssetRow = singleAssetNode.querySelector('.explorer__row')

    expect(collectionRow).toHaveClass('explorer__row--collection')
    expect(itemRow).toHaveClass('explorer__row--item')
    expect(assetRow).toHaveClass('explorer__row--asset')
    expect(singleAssetRow).toHaveClass('explorer__row--item')
    expect(collectionNode).toHaveAttribute('aria-level', '1')
    expect(itemNode).toHaveAttribute('aria-level', '2')
    expect(assetNode).toHaveAttribute('aria-level', '3')
    expect(singleAssetNode).toHaveAttribute('aria-level', '2')

    expect(collectionRow).toHaveAttribute('style', '--tree-level: 0;')
    expect(itemRow).toHaveAttribute('style', '--tree-level: 1;')
    expect(assetRow).toHaveAttribute('style', '--tree-level: 2;')
    expect(singleAssetRow).toHaveAttribute('style', '--tree-level: 1;')

    expect(collectionRow?.querySelector('.explorer__chevron-spacer')).toBeNull()
    expect(itemRow?.querySelector('.explorer__chevron')).toBeInTheDocument()
    expect(assetRow?.querySelector('.explorer__chevron-spacer')).toBeInTheDocument()
    expect(singleAssetRow?.querySelector('.explorer__chevron-spacer')).toBeInTheDocument()
  })

  it('keeps the document explorer open and removes the internal collapse control', async () => {
    localStorage.setItem('entropia-document-explorer-open', 'false')

    render(DocumentExplorer)

    expect(await screen.findByRole('tree', { name: 'Explorador de documentos' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Cerrar explorador de documentos' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Abrir explorador de documentos' })).not.toBeInTheDocument()
  })

  it('allows manually collapsing the active collection', async () => {
    persistOpenTree(['col-1'], ['item-1'])

    render(DocumentExplorer)

    expect(await screen.findByRole('treeitem', { name: 'Acta 1' })).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Colapsar colección Colección 1' }))

    expect(screen.getByRole('treeitem', { name: 'Colección 1' })).toHaveAttribute(
      'aria-expanded',
      'false'
    )
    expect(screen.getByRole('treeitem', { name: 'Colección 1' })).toHaveAttribute(
      'aria-selected',
      'true'
    )
    expect(screen.queryByRole('treeitem', { name: 'Acta 1' })).not.toBeInTheDocument()
  })

  it('allows manually collapsing the active item while keeping the selected asset', async () => {
    persistOpenTree(['col-1'], ['item-1'])

    render(DocumentExplorer)

    const assetButton = (await screen.findByText('acta-1.pdf')).closest('button')

    if (!assetButton) {
      throw new Error('Expected asset button to be rendered')
    }

    await fireEvent.click(assetButton)
    await fireEvent.click(screen.getByRole('button', { name: 'Colapsar documento Acta 1' }))

    expect(state.replace).toHaveBeenLastCalledWith({
      name: 'item',
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-1',
      itemTitle: 'Acta 1',
      assetId: 'asset-1',
      assetLabel: 'acta-1.pdf',
    })
    expect(screen.getByRole('treeitem', { name: 'Acta 1' })).toHaveAttribute(
      'aria-selected',
      'true'
    )
    expect(screen.queryByRole('treeitem', { name: 'acta-1.pdf' })).not.toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Expandir documento Acta 1' }))

    expect(await screen.findByRole('treeitem', { name: 'acta-1.pdf' })).toHaveAttribute(
      'aria-current',
      'true'
    )
  })

  it('does not auto-expand a closed parent when navigation selects an item', async () => {
    setCurrentNavigationView({ name: 'collections' })

    render(DocumentExplorer)

    expect(await screen.findByRole('treeitem', { name: 'Colección 1' })).toHaveAttribute(
      'aria-expanded',
      'false'
    )
    expect(screen.queryByRole('treeitem', { name: 'Acta 1' })).not.toBeInTheDocument()

    state.snapshot.current = {
      name: 'item' as const,
      collectionId: 'col-1',
      collectionName: 'Colección 1',
      itemId: 'item-2',
      itemTitle: 'Acta 2',
    }
    state.snapshot.breadcrumb = ['Colecciones', 'Colección 1', 'Acta 2']
    state.emit()

    await waitFor(() => {
      expect(state.store.items.findCardSummariesByCollection).toHaveBeenCalledWith('col-1')
    })
    expect(screen.getByRole('treeitem', { name: 'Colección 1' })).toHaveAttribute(
      'aria-expanded',
      'false'
    )
    expect(screen.queryByRole('treeitem', { name: 'Acta 2' })).not.toBeInTheDocument()
  })

  it('does not auto-expand a closed item when an asset becomes selected', async () => {
    persistOpenTree(['col-1'])

    render(DocumentExplorer)

    await screen.findByRole('treeitem', { name: 'Acta 1' })
    await fireEvent.click(screen.getByRole('button', { name: 'Colapsar documento Acta 1' }))

    window.dispatchEvent(
      new CustomEvent('entropia:document-explorer-asset-selected', {
        detail: { itemId: 'item-1', assetId: 'asset-1' },
      })
    )

    expect(screen.getByRole('treeitem', { name: 'Acta 1' })).toHaveAttribute(
      'aria-expanded',
      'false'
    )
    expect(screen.queryByRole('treeitem', { name: 'acta-1.pdf' })).not.toBeInTheDocument()
  })

  it('persists expanded nodes and restores them with the active path open', async () => {
    persistOpenTree(['col-2'], ['item-3'])

    render(DocumentExplorer)

    expect(await screen.findByRole('treeitem', { name: 'Acta 3' })).toBeInTheDocument()
    expect(await screen.findByRole('treeitem', { name: 'Acta 1' })).toBeInTheDocument()
    expect(await screen.findByRole('treeitem', { name: 'acta-1.pdf' })).toBeInTheDocument()

    await waitFor(() => {
      expect(state.store.items.findCardSummariesByCollection).toHaveBeenCalledWith('col-2')
      expect(state.store.assets.findByItem).toHaveBeenCalledWith('item-3')
      expect(state.store.items.findCardSummariesByCollection).toHaveBeenCalledWith('col-1')
      expect(state.store.assets.findByItem).toHaveBeenCalledWith('item-1')
    })
  })

  it('caps huge persisted open trees while preserving the active path outside the cap', async () => {
    const backgroundCollections = Array.from({ length: 20 }, (_, index) => `col-bg-${index + 1}`)
    const backgroundItems = Array.from({ length: 20 }, (_, index) => `item-bg-${index + 1}`)
    persistOpenTree(backgroundCollections, backgroundItems)

    render(DocumentExplorer)

    expect(await screen.findByRole('treeitem', { name: 'Acta 1' })).toBeInTheDocument()
    expect(await screen.findByRole('treeitem', { name: 'acta-1.pdf' })).toBeInTheDocument()

    await waitFor(() => {
      expect(state.store.items.findCardSummariesByCollection).toHaveBeenCalledWith('col-bg-20')
      expect(state.store.assets.findByItem).toHaveBeenCalledWith('item-bg-20')
      expect(state.store.items.findCardSummariesByCollection).toHaveBeenCalledWith('col-1')
      expect(state.store.assets.findByItem).toHaveBeenCalledWith('item-1')
    })

    expect(state.store.items.findCardSummariesByCollection).not.toHaveBeenCalledWith('col-bg-1')
    expect(state.store.items.findCardSummariesByCollection).not.toHaveBeenCalledWith('col-bg-4')
    expect(state.store.assets.findByItem).not.toHaveBeenCalledWith('item-bg-1')
    expect(state.store.assets.findByItem).not.toHaveBeenCalledWith('item-bg-4')
  })

  it('renders centralized svg icons for explorer controls and nodes', async () => {
    persistOpenTree(['col-1'], ['item-1'])

    const { container } = render(DocumentExplorer)

    await screen.findByText('Colección 1')
    await screen.findByText('Acta 1')
    await screen.findByText('acta-1.pdf')
    await screen.findByText('acta-1-audio.mp3')

    const collectionButton = (await screen.findByText('Colección 1')).closest('button')
    const itemButton = (await screen.findByText('Acta 1')).closest('button')
    const pdfAssetButton = (await screen.findByText('acta-1.pdf')).closest('button')
    const audioAssetButton = (await screen.findByText('acta-1-audio.mp3')).closest('button')

    if (!collectionButton || !itemButton || !pdfAssetButton || !audioAssetButton) {
      throw new Error('Expected explorer node buttons to be rendered')
    }

    expect(collectionButton.querySelector('svg')).not.toBeNull()
    expect(itemButton.querySelector('svg')).not.toBeNull()
    expect(pdfAssetButton.querySelector('svg')).not.toBeNull()
    expect(audioAssetButton.querySelector('svg')).not.toBeNull()
    expect(container.querySelectorAll('svg').length).toBeGreaterThanOrEqual(7)
  })

  it('renders centralized svg icons for flattened image assets', async () => {
    persistOpenTree(['col-1'])

    render(DocumentExplorer)

    await screen.findByText('Acta 2')

    const imageAssetButton = (await screen.findByText('Acta 2')).closest('button')

    if (!imageAssetButton) {
      throw new Error('Expected image asset button to be rendered')
    }

    expect(imageAssetButton.querySelector('svg')).not.toBeNull()
    expect(screen.getByText('image')).toBeInTheDocument()
    expect(state.store.assets.findByItem).not.toHaveBeenCalledWith('item-2')
  })

  it('refreshes cached collection items and counts when the collection changes', async () => {
    persistOpenTree(['col-1'])

    render(DocumentExplorer)

    await screen.findByText('Acta 2')
    expect(screen.getByText('2')).toBeInTheDocument()

    state.store.items.findCardSummariesByCollection.mockImplementation(async (collectionId: string) => {
      if (collectionId === 'col-1') {
        return [
          {
            id: 'item-4',
            title: 'Acta 4',
            collectionId: 'col-1',
            metadata: null,
            createdAt: 1,
            updatedAt: 4,
            assetCount: 0,
            primaryAssetId: null,
            primaryAssetPath: null,
            primaryAssetType: null,
          },
        ]
      }

      return [
        {
          id: 'item-3',
          title: 'Acta 3',
          collectionId: 'col-2',
          metadata: null,
          createdAt: 1,
          updatedAt: 3,
          assetCount: 1,
          primaryAssetId: 'asset-4',
          primaryAssetPath: 'docs/acta-3.pdf',
          primaryAssetType: 'pdf',
        },
      ]
    })
    state.store.collections.countItems.mockImplementation(async (id: string) =>
      id === 'col-1' ? 1 : 1
    )

    window.dispatchEvent(
      new CustomEvent('entropia:document-explorer-collection-changed', {
        detail: { collectionId: 'col-1', itemId: 'item-1' },
      })
    )

    expect(await screen.findByText('Acta 4')).toBeInTheDocument()
    expect(screen.queryByText('Acta 1')).not.toBeInTheDocument()
    expect(screen.queryByText('Acta 2')).not.toBeInTheDocument()
    expect(screen.queryByText('2')).not.toBeInTheDocument()
  })
})
