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
    expect(nav.breadcrumb).toEqual([])

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

  it('openRootSection pushes each distinct section so Back unwinds one at a time', () => {
    nav.openRootSection({ name: 'settings' })
    nav.openRootSection({ name: 'db-browser' })
    nav.openRootSection({ name: 'settings' })

    expect(nav.current).toEqual({ name: 'settings' })
    expect(nav.breadcrumb).toEqual(['Colecciones', 'Configuración'])

    nav.back()
    expect(nav.current).toEqual({ name: 'db-browser' })
    nav.back()
    expect(nav.current).toEqual({ name: 'settings' })
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

  it('switching root sections from an item stacks, and Back unwinds through each one', () => {
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
    expect(nav.current).toEqual({ name: 'settings' })
    nav.back()
    expect(nav.current).toEqual({ name: 'rag-chat' })
    nav.back()
    expect(nav.current).toEqual({ name: 'investigation', jobId: 'j1', title: 'Pregunta' })
    nav.back()
    expect(nav.current).toEqual({ name: 'research' })
    nav.back()

    expect(nav.current).toEqual(item)
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
    nav.navigate({ name: 'collections' })
    const snapshots: string[][] = []
    const unsubscribe = nav.subscribe((snapshot) => {
      snapshots.push(snapshot.breadcrumb)
    })

    locale.set('en')

    expect(snapshots.at(-1)).toEqual(['Collections'])
    unsubscribe()
  })

  it('stops following the locale once disposed', () => {
    nav.navigate({ name: 'collections' })
    const snapshots: string[][] = []
    const unsubscribe = nav.subscribe((snapshot) => {
      snapshots.push(snapshot.breadcrumb)
    })
    const before = snapshots.length

    nav.dispose()
    locale.set('en')

    expect(snapshots).toHaveLength(before)
    unsubscribe()
  })

  it('home has no breadcrumb: the page is the start, not a place in a path', () => {
    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.breadcrumb).toEqual([])
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

  it('openRootSection from home pushes each section so Back returns through settings first', () => {
    nav.openRootSection({ name: 'settings' })
    nav.openRootSection({ name: 'rag-chat' })

    expect(nav.current).toEqual({ name: 'rag-chat' })

    nav.back()
    expect(nav.current).toEqual({ name: 'settings' })

    nav.back()
    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
  })

  it('navigate is a no-op when the view equals the current one', () => {
    nav.navigate({ name: 'collections' })
    expect(nav.current).toEqual({ name: 'collections' })

    nav.navigate({ name: 'collections' })
    expect(nav.canGoBack).toBe(true)

    nav.back()
    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
  })

  it('tapping the current section icon twice adds nothing', () => {
    nav.openRootSection({ name: 'rag-chat' })
    nav.openRootSection({ name: 'rag-chat' })

    expect(nav.current).toEqual({ name: 'rag-chat' })

    nav.back()
    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
  })

  it('Inicio -> Chat -> Investigacion -> Escritura, Back x3 unwinds one screen at a time', () => {
    nav.navigate({ name: 'rag-chat' })
    nav.navigate({ name: 'research' })
    nav.navigate({ name: 'writing' })

    expect(nav.current).toEqual({ name: 'writing' })

    nav.back()
    expect(nav.current).toEqual({ name: 'research' })

    nav.back()
    expect(nav.current).toEqual({ name: 'rag-chat' })

    nav.back()
    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
  })

  it('Inicio -> Colecciones -> collection A -> document 1 -> Chat, then Back all the way home', () => {
    const collectionA: View = { name: 'collection', id: 'a', collectionName: 'A' }
    const document1: View = {
      name: 'item',
      collectionId: 'a',
      collectionName: 'A',
      itemId: 'doc-1',
      itemTitle: 'Documento 1',
    }

    nav.navigate({ name: 'collections' })
    nav.navigate(collectionA)
    nav.navigate(document1)
    nav.openRootSection({ name: 'rag-chat' })

    nav.back()
    expect(nav.current).toEqual(document1)

    nav.back()
    expect(nav.current).toEqual(collectionA)

    nav.back()
    expect(nav.current).toEqual({ name: 'collections' })

    nav.back()
    expect(nav.current).toEqual({ name: 'home' })
    expect(nav.canGoBack).toBe(false)
  })

  it('paging through sibling documents (next, next, back, back) unwinds one document at a time', () => {
    const doc1: View = {
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Archivo',
      itemId: 'doc-1',
      itemTitle: 'Documento 1',
    }
    const doc2: View = { ...doc1, itemId: 'doc-2', itemTitle: 'Documento 2' }
    const doc3: View = { ...doc1, itemId: 'doc-3', itemTitle: 'Documento 3' }

    nav.navigate(doc1)
    // The prev/next arrows push a different document (TopBar.svelte
    // navigateToSibling): a different item is a different screen.
    nav.navigate(doc2)
    nav.navigate(doc3)

    nav.back()
    expect(nav.current).toEqual(doc2)

    nav.back()
    expect(nav.current).toEqual(doc1)
  })

  it('paging to a different asset of the same document does not add a Back stop', () => {
    const page1: View = {
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Archivo',
      itemId: 'doc-1',
      itemTitle: 'Documento 1',
      assetId: 'asset-1',
      assetLabel: 'page 1',
    }
    const page2: View = { ...page1, assetId: 'asset-2', assetLabel: 'page 2' }

    nav.navigate({ name: 'collection', id: 'c1', collectionName: 'Archivo' })
    nav.navigate(page1)
    // Same document, only the selected asset changes: ItemView's own effect
    // uses `replace`, never `navigate`, for this.
    nav.replace(page2)

    expect(nav.current).toEqual(page2)

    nav.back()
    expect(nav.current).toEqual({ name: 'collection', id: 'c1', collectionName: 'Archivo' })
  })

  it('a breadcrumb click from a document to its collection pushes: Back returns to the document', () => {
    const item: View = {
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Archivo',
      itemId: 'i1',
      itemTitle: 'Acta',
    }
    const collectionView: View = { name: 'collection', id: 'c1', collectionName: 'Archivo' }

    nav.navigate(collectionView)
    nav.navigate(item)
    // TopBar's navigateToBreadcrumb now pushes the crumb's own view.
    nav.navigate(collectionView)

    expect(nav.current).toEqual(collectionView)

    nav.back()
    expect(nav.current).toEqual(item)
  })

  it('Escritura list -> open a document -> Back returns to the list', () => {
    const list: View = { name: 'writing', documentId: null, documentTitle: null }
    const doc: View = { name: 'writing', documentId: 'w1', documentTitle: 'Manuscrito' }

    nav.navigate(list)
    nav.navigate(doc)

    nav.back()
    expect(nav.current).toEqual(list)
  })

  it('deleting the open writing document replaces it: Back never lands on the deleted one', () => {
    const list: View = { name: 'writing', documentId: null, documentTitle: null }
    const doc: View = { name: 'writing', documentId: 'w1', documentTitle: 'Manuscrito' }

    nav.navigate(list)
    nav.navigate(doc)
    // The document no longer exists once deleted, so there is nowhere to
    // return to but the list: replace, not push.
    nav.replace(list)

    expect(nav.current).toEqual(list)

    // Back never resurrects the deleted document — it unwinds to the list
    // that was already there before it was opened.
    nav.back()
    expect(nav.current).toEqual(list)
    nav.back()
    expect(nav.current).toEqual({ name: 'home' })
  })

  it('deleting the current asset replaces it: Back never lands on the deleted asset', () => {
    const collectionView: View = { name: 'collection', id: 'c1', collectionName: 'Archivo' }
    const withDeletedAsset: View = {
      name: 'item',
      collectionId: 'c1',
      collectionName: 'Archivo',
      itemId: 'i1',
      itemTitle: 'Acta',
      assetId: 'asset-doomed',
      assetLabel: 'page 1',
    }
    const withNextAsset: View = { ...withDeletedAsset, assetId: 'asset-2', assetLabel: 'page 2' }

    nav.navigate(collectionView)
    nav.navigate(withDeletedAsset)
    nav.replace(withNextAsset)

    expect(nav.current).toEqual(withNextAsset)

    nav.back()
    expect(nav.current).toEqual(collectionView)
  })

  it('opening Settings from the batch/sync status indicator pushes: Back returns to the previous screen', () => {
    nav.navigate({ name: 'rag-chat' })
    nav.openRootSection({ name: 'settings' })

    expect(nav.current).toEqual({ name: 'settings' })

    nav.back()
    expect(nav.current).toEqual({ name: 'rag-chat' })
  })

  describe('forget', () => {
    it('removes every matching entry anywhere in history, not only the current one', () => {
      const collectionA: View = { name: 'collection', id: 'a', collectionName: 'A' }
      const docInA: View = {
        name: 'item',
        collectionId: 'a',
        collectionName: 'A',
        itemId: 'doc-1',
        itemTitle: 'Doc 1',
      }
      const collectionB: View = { name: 'collection', id: 'b', collectionName: 'B' }

      nav.navigate(collectionA)
      nav.navigate(docInA)
      nav.navigate(collectionB)

      nav.forget((view) => view.name === 'collection' && view.id === 'a')

      nav.back()
      expect(nav.current).toEqual(docInA)
      nav.back()
      expect(nav.current).toEqual({ name: 'home' })
    })

    it('collapses consecutive duplicates left behind by removal', () => {
      const collectionA: View = { name: 'collection', id: 'a', collectionName: 'A' }
      const docInA: View = {
        name: 'item',
        collectionId: 'a',
        collectionName: 'A',
        itemId: 'doc-1',
        itemTitle: 'Doc 1',
      }

      // home -> A -> doc -> A(again, a distinct push since current was doc)
      nav.navigate(collectionA)
      nav.navigate(docInA)
      nav.navigate(collectionA)

      nav.forget((view) => view.name === 'item' && view.itemId === 'doc-1')

      // The two now-adjacent A entries collapse into one: only one Back stop.
      expect(nav.current).toEqual(collectionA)
      nav.back()
      expect(nav.current).toEqual({ name: 'home' })
      expect(nav.canGoBack).toBe(false)
    })

    it('never leaves history empty: the root home survives even a match-everything predicate', () => {
      nav.navigate({ name: 'collections' })
      nav.navigate({ name: 'settings' })

      nav.forget(() => true)

      expect(nav.current).toEqual({ name: 'home' })
      expect(nav.canGoBack).toBe(false)
    })

    it('lands on the new top of history when the current view is removed', () => {
      const doc1: View = {
        name: 'item',
        collectionId: 'c1',
        collectionName: 'Archivo',
        itemId: 'doc-1',
        itemTitle: 'Documento 1',
      }
      const doc2: View = { ...doc1, itemId: 'doc-2', itemTitle: 'Documento 2' }

      nav.navigate(doc1)
      nav.navigate(doc2)

      nav.forget((view) => view.name === 'item' && view.itemId === 'doc-2')

      expect(nav.current).toEqual(doc1)
      expect(nav.canGoBack).toBe(true)

      nav.back()
      expect(nav.current).toEqual({ name: 'home' })
    })

    it('emits exactly once per prune', () => {
      nav.navigate({ name: 'collection', id: 'a', collectionName: 'A' })
      nav.navigate({ name: 'collection', id: 'b', collectionName: 'B' })

      let emits = 0
      const unsubscribe = nav.subscribe(() => {
        emits++
      })
      emits = 0 // subscribe itself runs once synchronously; only count what follows

      nav.forget((view) => view.name === 'collection' && view.id === 'a')

      expect(emits).toBe(1)
      unsubscribe()
    })

    it('is a no-op (no emit) when nothing matches', () => {
      nav.navigate({ name: 'collection', id: 'a', collectionName: 'A' })

      let emits = 0
      const unsubscribe = nav.subscribe(() => {
        emits++
      })
      emits = 0

      nav.forget((view) => view.name === 'collection' && view.id === 'zzz')

      expect(emits).toBe(0)
      unsubscribe()
    })
  })

  describe('typed forget helpers', () => {
    it('forgetCollection removes the collection view and every one of its documents', () => {
      const collectionA: View = { name: 'collection', id: 'a', collectionName: 'A' }
      const docInA: View = {
        name: 'item',
        collectionId: 'a',
        collectionName: 'A',
        itemId: 'doc-1',
        itemTitle: 'Doc 1',
      }
      const collections: View = { name: 'collections' }

      nav.navigate(collections)
      nav.navigate(collectionA)
      nav.navigate(docInA)
      nav.navigate(collections)

      nav.forgetCollection('a')

      // Back never reaches the deleted collection or its document.
      nav.back()
      expect(nav.current).toEqual({ name: 'home' })
    })

    it('forgetItem removes item entries for that document whatever the asset', () => {
      const page1: View = {
        name: 'item',
        collectionId: 'c1',
        collectionName: 'Archivo',
        itemId: 'doc-1',
        itemTitle: 'Documento 1',
        assetId: 'asset-1',
        assetLabel: 'page 1',
      }
      const page2: View = { ...page1, assetId: 'asset-2', assetLabel: 'page 2' }
      const doc2: View = {
        name: 'item',
        collectionId: 'c1',
        collectionName: 'Archivo',
        itemId: 'doc-2',
        itemTitle: 'Documento 2',
      }

      nav.navigate(page1)
      nav.navigate(page2)
      nav.navigate(doc2)

      nav.forgetItem('doc-1')

      nav.back()
      expect(nav.current).toEqual({ name: 'home' })
    })

    it('forgetAsset removes only entries for that specific page, keeping other pages of the item', () => {
      const page1: View = {
        name: 'item',
        collectionId: 'c1',
        collectionName: 'Archivo',
        itemId: 'doc-1',
        itemTitle: 'Documento 1',
        assetId: 'asset-1',
        assetLabel: 'page 1',
      }
      const page2: View = { ...page1, assetId: 'asset-2', assetLabel: 'page 2' }
      const page3: View = { ...page1, assetId: 'asset-3', assetLabel: 'page 3' }

      nav.navigate(page1)
      nav.navigate(page2)
      nav.navigate(page3)

      nav.forgetAsset('asset-2')

      nav.back()
      expect(nav.current).toEqual(page1)
    })

    it('forgetWriting removes entries for that document: Back skips it', () => {
      const writingX: View = { name: 'writing', documentId: 'w-x', documentTitle: 'X' }
      const writingY: View = { name: 'writing', documentId: 'w-y', documentTitle: 'Y' }

      nav.navigate(writingX)
      nav.navigate(writingY)

      nav.forgetWriting('w-x')

      nav.back()
      expect(nav.current).toEqual({ name: 'home' })
    })

    it('forgetResearch removes entries for that job', () => {
      const jobA: View = { name: 'investigation', jobId: 'job-a', title: 'A' }
      const jobB: View = { name: 'investigation', jobId: 'job-b', title: 'B' }

      nav.navigate(jobA)
      nav.navigate(jobB)

      nav.forgetResearch('job-a')

      nav.back()
      expect(nav.current).toEqual({ name: 'home' })
    })
  })

  it('caps history growth but always keeps the root', () => {
    for (let i = 0; i < 250; i++) {
      nav.navigate({ name: 'investigation', jobId: `job-${i}`, title: `Job ${i}` })
    }

    let historyLength = 0
    const unsubscribe = nav.subscribe((snapshot) => {
      historyLength = snapshot.history.length
    })
    unsubscribe()

    expect(historyLength).toBeLessThanOrEqual(200)
    expect(nav.current).toEqual({ name: 'investigation', jobId: 'job-249', title: 'Job 249' })

    // Popping all the way back still terminates at home: the cap never drops it.
    while (nav.canGoBack) nav.back()
    expect(nav.current).toEqual({ name: 'home' })
  })
})
