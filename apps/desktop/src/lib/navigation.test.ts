import { describe, it, expect, beforeEach } from 'vitest'
import { NavigationStore, type View } from './navigation'
import { locale } from './i18n'

describe('NavigationStore', () => {
  let nav: NavigationStore

  beforeEach(() => {
    nav = new NavigationStore()
    locale.set('es')
  })

  it('starts at collections view', () => {
    expect(nav.current).toEqual({ name: 'collections' })
  })

  it('canGoBack is false at root', () => {
    expect(nav.canGoBack).toBe(false)
  })

  it('navigate adds view to history and updates current', () => {
    const view: View = { name: 'collection', id: 'c1', collectionName: 'My Collection' }
    nav.navigate(view)
    expect(nav.current).toEqual(view)
    expect(nav.canGoBack).toBe(true)
  })

  it('navigate to item view shows item as current', () => {
    const collectionView: View = { name: 'collection', id: 'c1', collectionName: 'Coll A' }
    const itemView: View = {
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Coll A',
      itemId: 'i1',
      itemTitle: 'Document 1',
    }
    nav.navigate(collectionView)
    nav.navigate(itemView)
    expect(nav.current).toEqual(itemView)
  })

  it('back removes last view and updates current', () => {
    nav.navigate({ name: 'collection', id: 'c1', collectionName: 'Test' })
    nav.back()
    expect(nav.current).toEqual({ name: 'collections' })
    expect(nav.canGoBack).toBe(false)
  })

  it('back is no-op at root', () => {
    nav.back()
    expect(nav.current).toEqual({ name: 'collections' })
    expect(nav.canGoBack).toBe(false)
  })

  it('back traverses full history correctly', () => {
    nav.navigate({ name: 'collection', id: 'c1', collectionName: 'A' })
    nav.navigate({
      name: 'item',
      collectionId: 'c1',
      collectionName: 'A',
      itemId: 'i1',
      itemTitle: 'Doc',
    })
    nav.back()
    expect(nav.current).toEqual({ name: 'collection', id: 'c1', collectionName: 'A' })
    nav.back()
    expect(nav.current).toEqual({ name: 'collections' })
  })

  it('breadcrumb builds from the current view parent chain', () => {
    expect(nav.breadcrumb).toEqual(['Colecciones'])

    nav.navigate({ name: 'collection', id: 'c1', collectionName: 'Photos' })
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Photos'])

    nav.navigate({
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Photos',
      itemId: 'i1',
      itemTitle: 'Sunset.jpg',
    })
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Photos'])
  })

  it('breadcrumb shows the selected asset without the redundant item level', () => {
    nav.navigate({
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Resoluciones SOIP',
      itemId: 'i1',
      itemTitle: '114',
      assetId: 'asset-2',
      assetLabel: '114_page_2.png',
    })

    expect(nav.breadcrumb).toEqual(['Colecciones', 'Resoluciones SOIP', '114_page_2.png'])
  })

  it('uses the selected asset as the leaf when its label matches the item title', () => {
    nav.navigate({
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Photos',
      itemId: 'i1',
      itemTitle: 'Sunset.jpg',
      assetId: 'asset-1',
      assetLabel: 'Sunset.jpg',
    })

    expect(nav.breadcrumb).toEqual(['Colecciones', 'Photos', 'Sunset.jpg'])
  })

  it('does not repeat breadcrumb levels when history contains repeated parent nodes', () => {
    const collectionView: View = { name: 'collection', id: 'c1', collectionName: 'Photos' }
    const itemView: View = {
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Photos',
      itemId: 'i1',
      itemTitle: 'Sunset.jpg',
    }

    nav.navigate(collectionView)
    nav.navigate(itemView)
    nav.navigate(collectionView)
    nav.navigate(itemView)

    expect(nav.breadcrumb).toEqual(['Colecciones', 'Photos'])

    nav.back()

    expect(nav.breadcrumb).toEqual(['Colecciones', 'Photos'])
  })

  it('breadcrumb updates after back', () => {
    nav.navigate({ name: 'collection', id: 'c1', collectionName: 'Docs' })
    nav.navigate({
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Docs',
      itemId: 'i1',
      itemTitle: 'Report',
    })
    nav.back()
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Docs'])
  })

  it('navigates to settings view', () => {
    nav.navigate({ name: 'settings' })
    expect(nav.current).toEqual({ name: 'settings' })
    expect(nav.canGoBack).toBe(true)
  })

  it('navigates to db browser view', () => {
    nav.navigate({ name: 'db-browser' })
    expect(nav.current).toEqual({ name: 'db-browser' })
    expect(nav.canGoBack).toBe(true)
  })

  it('db browser breadcrumb shows Base de datos', () => {
    nav.navigate({ name: 'db-browser' })
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Base de datos'])
  })

  it('navigates to rag chat view', () => {
    nav.navigate({ name: 'rag-chat' })
    expect(nav.current).toEqual({ name: 'rag-chat' })
    expect(nav.canGoBack).toBe(true)
  })

  it('rag chat breadcrumb shows Chat', () => {
    nav.navigate({ name: 'rag-chat' })
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Chat'])
  })

  it('openRootSection rebuilds canonical breadcrumb for rag chat', () => {
    nav.navigate({ name: 'collection', id: 'c1', collectionName: 'Archivo' })

    nav.openRootSection({ name: 'rag-chat' })

    expect(nav.current).toEqual({ name: 'rag-chat' })
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Chat'])
    expect(nav.canGoBack).toBe(true)
  })

  it('settings breadcrumb shows Configuracion', () => {
    nav.navigate({ name: 'settings' })
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Configuración'])
  })

  it('can go back from settings to collections', () => {
    nav.navigate({ name: 'settings' })
    nav.back()
    expect(nav.current).toEqual({ name: 'collections' })
    expect(nav.canGoBack).toBe(false)
  })

  it('replace works with settings view', () => {
    nav.navigate({ name: 'collection', id: 'c1', collectionName: 'Test' })
    nav.replace({ name: 'settings' })
    expect(nav.current).toEqual({ name: 'settings' })
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Configuración'])
  })

  it('openRootSection rebuilds canonical breadcrumb for settings', () => {
    nav.navigate({ name: 'collection', id: 'c1', collectionName: 'Archivo' })
    nav.navigate({ name: 'item', collectionId: 'c1', collectionName: 'Archivo', itemId: 'i1', itemTitle: 'Acta' })

    nav.openRootSection({ name: 'settings' })

    expect(nav.current).toEqual({ name: 'settings' })
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Configuración'])
    expect(nav.canGoBack).toBe(true)
  })

  it('openRootSection replaces previous root sections instead of accumulating history', () => {
    nav.openRootSection({ name: 'settings' })
    nav.openRootSection({ name: 'db-browser' })
    nav.openRootSection({ name: 'settings' })

    expect(nav.current).toEqual({ name: 'settings' })
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Configuración'])
  })

  it('resetToPath rebuilds canonical history for cross-collection item navigation', () => {
    nav.navigate({ name: 'collection', id: 'c1', collectionName: 'Origen' })
    nav.navigate({
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Origen',
      itemId: 'i1',
      itemTitle: 'Documento origen',
    })

    nav.resetToPath([
      { name: 'collections' },
      { name: 'collection', id: 'c2', collectionName: 'Destino' },
      {
        name: 'item',
        collectionId: 'c2',
        collectionName: 'Destino',
        itemId: 'i2',
        itemTitle: 'Documento destino',
      },
    ])

    expect(nav.breadcrumb).toEqual(['Colecciones', 'Destino'])

    nav.back()

    expect(nav.current).toEqual({ name: 'collection', id: 'c2', collectionName: 'Destino' })
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Destino'])
  })

  it('emits localized breadcrumbs again when locale changes', () => {
    const snapshots: string[][] = []
    const unsubscribe = nav.subscribe((snapshot) => {
      snapshots.push(snapshot.breadcrumb)
    })

    locale.set('en')

    expect(snapshots.at(-1)).toEqual(['Collections'])
    unsubscribe()
  })
})
