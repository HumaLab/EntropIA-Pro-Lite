import { describe, it, expect, beforeEach } from 'vitest'
import { NavigationStore, type View } from './navigation'
import { locale } from './i18n'

describe('NavigationStore', () => {
  let nav: NavigationStore

  beforeEach(() => {
    nav = new NavigationStore()
    locale.set('es')
  })

  it('starts at home view', () => {
    expect(nav.current).toEqual({ name: 'home' })
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
    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
  })

  it('back is no-op at root', () => {
    nav.back()
    expect(nav.current).toEqual({ name: 'home' })
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
    expect(nav.current).toEqual({ name: 'home' })
  })

  it('breadcrumb builds from the current view parent chain', () => {
    expect(nav.breadcrumb).toEqual(['Inicio'])

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

  it('can go back from settings to home', () => {
    nav.navigate({ name: 'settings' })
    nav.back()
    expect(nav.current).toEqual({ name: 'home' })
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
    nav.navigate({
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Archivo',
      itemId: 'i1',
      itemTitle: 'Acta',
    })

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
    nav.back()
    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
  })

  it.each([
    ['research', { name: 'research' } as const],
    ['rag-chat', { name: 'rag-chat' } as const],
    ['settings', { name: 'settings' } as const],
    ['writing', { name: 'writing' } as const],
  ])('back from %s opened from an item returns to that item', (_name, section) => {
    const item: View = {
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Archivo',
      itemId: 'i1',
      itemTitle: 'Acta',
      assetId: 'a1',
      assetLabel: 'acta.png',
    }
    nav.navigate({ name: 'collection', id: 'c1', collectionName: 'Archivo' })
    nav.navigate(item)

    nav.openRootSection(section)
    nav.back()

    expect(nav.current).toEqual(item)
  })

  it('collections hierarchy still works after returning from a root section', () => {
    const collection: View = { name: 'collection', id: 'c1', collectionName: 'Archivo' }
    const item: View = {
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Archivo',
      itemId: 'i1',
      itemTitle: 'Acta',
      assetId: 'a1',
      assetLabel: 'acta.png',
    }
    nav.navigate(collection)
    nav.navigate(item)

    nav.openRootSection({ name: 'research' })
    nav.back()
    expect(nav.current).toEqual(item)

    nav.back()
    expect(nav.current).toEqual(collection)

    nav.back()
    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
  })

  it('back from a root section opened at home returns to home', () => {
    nav.openRootSection({ name: 'research' })
    nav.back()

    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
  })

  it('back from a root section opened from a collection returns to that collection', () => {
    const collection: View = { name: 'collection', id: 'c1', collectionName: 'Archivo' }
    nav.navigate(collection)

    nav.openRootSection({ name: 'settings' })
    nav.back()

    expect(nav.current).toEqual(collection)
  })

  it('switching root sections from an item does not stack or loop', () => {
    const item: View = {
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Archivo',
      itemId: 'i1',
      itemTitle: 'Acta',
      assetId: 'a1',
      assetLabel: 'acta.png',
    }
    nav.navigate({ name: 'collection', id: 'c1', collectionName: 'Archivo' })
    nav.navigate(item)

    nav.openRootSection({ name: 'research' })
    nav.navigate({ name: 'investigation', jobId: 'j1', title: 'Pregunta' })
    nav.openRootSection({ name: 'rag-chat' })
    nav.openRootSection({ name: 'settings' })
    nav.openRootSection({ name: 'writing' })

    nav.back()

    expect(nav.current).toEqual(item)
  })

  it('falls back to home when a root section has no hierarchy origin', () => {
    nav.resetToPath([{ name: 'research' }])
    nav.openRootSection({ name: 'settings' })

    expect(nav.current).toEqual({ name: 'settings' })

    nav.back()

    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
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

    expect(snapshots.at(-1)).toEqual(['Home'])
    unsubscribe()
  })

  it('home breadcrumb shows Inicio', () => {
    expect(nav.breadcrumb).toEqual(['Inicio'])
  })

  it('navigating to collections from home works and back returns to home', () => {
    nav.navigate({ name: 'collections' })
    expect(nav.current).toEqual({ name: 'collections' })
    expect(nav.breadcrumb).toEqual(['Colecciones'])
    expect(nav.canGoBack).toBe(true)

    nav.back()
    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
  })

  it('openRootSection from home preserves home as the origin to go back to', () => {
    nav.openRootSection({ name: 'settings' })
    nav.openRootSection({ name: 'rag-chat' })

    expect(nav.current).toEqual({ name: 'rag-chat' })

    nav.back()

    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
  })
})
