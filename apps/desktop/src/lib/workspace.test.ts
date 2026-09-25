import { describe, it, expect, beforeEach } from 'vitest'
import { WorkspaceStore, MAX_TABS, resetTabIdSequenceForTests } from './workspace'

describe('WorkspaceStore tab lifecycle', () => {
  let ws: WorkspaceStore

  beforeEach(() => {
    resetTabIdSequenceForTests()
    ws = new WorkspaceStore()
  })

  it('starts with a single Home tab, active', () => {
    expect(ws.tabs).toHaveLength(1)
    expect(ws.tabs[0]!.navigation.current).toEqual({ name: 'home' })
    expect(ws.activeTabId).toBe(ws.tabs[0]!.id)
  })

  it('activeNavigation resolves to the active tab store', () => {
    expect(ws.activeNavigation).toBe(ws.tabs[0]!.navigation)
  })

  it('navigationFor returns the matching tab store and throws for an unknown id', () => {
    expect(ws.navigationFor(ws.tabs[0]!.id)).toBe(ws.tabs[0]!.navigation)
    expect(() => ws.navigationFor('missing')).toThrow('[workspace] Unknown tab: missing')
  })

  it('openTab opens a new Home tab and activates it', () => {
    const id = ws.openTab()
    expect(id).not.toBeNull()
    expect(ws.tabs).toHaveLength(2)
    expect(ws.activeTabId).toBe(id)
    expect(ws.navigationFor(id!).current).toEqual({ name: 'home' })
  })

  it('openTab can seed a non-Home starting view', () => {
    const id = ws.openTab({ name: 'settings' })
    expect(ws.navigationFor(id!).current).toEqual({ name: 'settings' })
  })

  it('caps at four tabs: the fifth openTab returns null and adds nothing', () => {
    ws.openTab()
    ws.openTab()
    ws.openTab()
    expect(ws.tabs).toHaveLength(MAX_TABS)
    const fifth = ws.openTab()
    expect(fifth).toBeNull()
    expect(ws.tabs).toHaveLength(MAX_TABS)
  })

  it('each tab owns an independent navigation history', () => {
    const secondId = ws.openTab()!
    ws.navigationFor(secondId).navigate({ name: 'collections' })
    expect(ws.tabs[0]!.navigation.current).toEqual({ name: 'home' })
    expect(ws.navigationFor(secondId).current).toEqual({ name: 'collections' })
  })

  it('closeTab is a no-op on the last remaining tab', () => {
    const onlyId = ws.tabs[0]!.id
    ws.closeTab(onlyId)
    expect(ws.tabs).toHaveLength(1)
    expect(ws.activeTabId).toBe(onlyId)
  })

  it('closing the active leftmost tab activates the tab that is now leftmost', () => {
    const secondId = ws.openTab()!
    const firstId = ws.tabs.find((t) => t.id !== secondId)!.id
    ws.activateTab(firstId)

    ws.closeTab(firstId)

    expect(ws.tabs.map((t) => t.id)).toEqual([secondId])
    expect(ws.activeTabId).toBe(secondId)
  })

  it('closing an inactive tab leaves the active tab untouched', () => {
    const secondId = ws.openTab()!
    const firstId = ws.tabs.find((t) => t.id !== secondId)!.id
    ws.activateTab(secondId)

    ws.closeTab(firstId)

    expect(ws.activeTabId).toBe(secondId)
    expect(ws.tabs).toHaveLength(1)
  })

  it('closing an active middle tab activates its now-neighbouring left tab (deferred minor from Task 1.1)', () => {
    const firstId = ws.tabs[0]!.id
    const secondId = ws.openTab()!
    const thirdId = ws.openTab()!
    ws.activateTab(secondId)

    ws.closeTab(secondId)

    expect(ws.tabs.map((t) => t.id)).toEqual([firstId, thirdId])
    expect(ws.activeTabId).toBe(firstId)
  })

  it('activateTab switches the active tab', () => {
    const secondId = ws.openTab()!
    const firstId = ws.tabs.find((t) => t.id !== secondId)!.id
    ws.activateTab(firstId)
    expect(ws.activeTabId).toBe(firstId)
  })

  it('activateTab on an unknown id is a no-op', () => {
    const before = ws.activeTabId
    ws.activateTab('missing')
    expect(ws.activeTabId).toBe(before)
  })

  it('subscribe emits the current snapshot immediately and on every change', () => {
    const snapshots: string[] = []
    const unsubscribe = ws.subscribe((snap) => snapshots.push(snap.activeTabId))
    expect(snapshots).toHaveLength(1)

    const id = ws.openTab()
    expect(snapshots.at(-1)).toBe(id)
    unsubscribe()
  })

  it("re-emits when a tab's own navigation changes", () => {
    let emits = 0
    const unsubscribe = ws.subscribe(() => {
      emits++
    })
    emits = 0

    ws.activeNavigation.navigate({ name: 'collections' })

    expect(emits).toBeGreaterThan(0)
    unsubscribe()
  })

  it('a fresh workspace snapshot has split === null', () => {
    let split: unknown
    const unsubscribe = ws.subscribe((snap) => {
      split = snap.split
    })
    expect(split).toBeNull()
    unsubscribe()
  })
})

describe('WorkspaceStore cross-tab pruning and the Writing single-tab rule', () => {
  let ws: WorkspaceStore

  beforeEach(() => {
    resetTabIdSequenceForTests()
    ws = new WorkspaceStore()
  })

  it('forgetCollection prunes a collection from every tab, including an inactive one', () => {
    const secondId = ws.openTab()!
    const firstId = ws.tabs.find((t) => t.id !== secondId)!.id
    const collectionView = { name: 'collection' as const, id: 'c1', collectionName: 'A' }

    ws.navigationFor(firstId).navigate(collectionView)
    ws.navigationFor(secondId).navigate(collectionView)
    ws.activateTab(firstId)
    // secondId is now the *inactive* tab — the failure mode this guards
    // against is pruning only the active tab's history.

    ws.forgetCollection('c1')

    expect(ws.navigationFor(firstId).current).toEqual({ name: 'home' })
    expect(ws.navigationFor(secondId).current).toEqual({ name: 'home' })
  })

  it('forgetItem prunes an item from every tab, including an inactive one', () => {
    const secondId = ws.openTab()!
    const firstId = ws.tabs.find((t) => t.id !== secondId)!.id
    const itemView = {
      name: 'item' as const,
      collectionId: 'c1',
      collectionName: 'A',
      itemId: 'doc-1',
      itemTitle: 'Doc',
    }

    ws.navigationFor(firstId).navigate(itemView)
    ws.navigationFor(secondId).navigate(itemView)
    ws.activateTab(firstId)
    // secondId is now the *inactive* tab.

    ws.forgetItem('doc-1')

    expect(ws.navigationFor(firstId).current).toEqual({ name: 'home' })
    expect(ws.navigationFor(secondId).current).toEqual({ name: 'home' })
  })

  it('forgetAsset prunes an asset from every tab, including an inactive one', () => {
    const secondId = ws.openTab()!
    const firstId = ws.tabs.find((t) => t.id !== secondId)!.id
    const itemView = {
      name: 'item' as const,
      collectionId: 'c1',
      collectionName: 'A',
      itemId: 'doc-1',
      itemTitle: 'Doc',
      assetId: 'asset-1',
      assetLabel: 'Page 1',
    }

    ws.navigationFor(firstId).navigate(itemView)
    ws.navigationFor(secondId).navigate(itemView)
    ws.activateTab(firstId)
    // secondId is now the *inactive* tab.

    ws.forgetAsset('asset-1')

    expect(ws.navigationFor(firstId).current).toEqual({ name: 'home' })
    expect(ws.navigationFor(secondId).current).toEqual({ name: 'home' })
  })

  it('forgetResearch prunes a research job from every tab, including an inactive one', () => {
    const secondId = ws.openTab()!
    const firstId = ws.tabs.find((t) => t.id !== secondId)!.id
    const researchView = { name: 'investigation' as const, jobId: 'job-1', title: 'Q' }

    ws.navigationFor(firstId).navigate(researchView)
    ws.navigationFor(secondId).navigate(researchView)
    ws.activateTab(firstId)
    // secondId is now the *inactive* tab.

    ws.forgetResearch('job-1')

    expect(ws.navigationFor(firstId).current).toEqual({ name: 'home' })
    expect(ws.navigationFor(secondId).current).toEqual({ name: 'home' })
  })

  it('forgetWriting prunes a writing document from every tab, including an inactive one', () => {
    const secondId = ws.openTab()!
    const firstId = ws.tabs.find((t) => t.id !== secondId)!.id
    const writingView = {
      name: 'writing' as const,
      documentId: 'w1',
      documentTitle: 'Manuscript',
    }

    ws.navigationFor(firstId).navigate(writingView)
    ws.navigationFor(secondId).navigate(writingView)
    ws.activateTab(firstId)
    // secondId is now the *inactive* tab.

    ws.forgetWriting('w1')

    expect(ws.navigationFor(firstId).current).toEqual({ name: 'home' })
    expect(ws.navigationFor(secondId).current).toEqual({ name: 'home' })
  })

  it('navigateActive navigates the active tab for a non-writing view', () => {
    ws.navigateActive({ name: 'collections' })
    expect(ws.activeNavigation.current).toEqual({ name: 'collections' })
  })

  it('navigateActive to writing opens it on the active tab when no tab shows it yet', () => {
    ws.navigateActive({ name: 'writing' })
    expect(ws.activeNavigation.current).toEqual({ name: 'writing' })
  })

  it('navigateActive to writing from another tab activates the tab that already shows it, unchanged', () => {
    const writingTabId = ws.activeTabId
    ws.navigateActive({ name: 'writing', documentId: 'w1', documentTitle: 'Manuscript' })

    const otherTabId = ws.openTab()!
    expect(ws.activeTabId).toBe(otherTabId)

    ws.navigateActive({ name: 'writing' })

    expect(ws.activeTabId).toBe(writingTabId)
    // The tab that requested it is left exactly where it was — no orphaned
    // navigate call on the tab that did not get Writing.
    expect(ws.navigationFor(otherTabId).current).toEqual({ name: 'home' })
    expect(ws.navigationFor(writingTabId).current).toEqual({
      name: 'writing',
      documentId: 'w1',
      documentTitle: 'Manuscript',
    })
  })
})

describe('WorkspaceStore split view', () => {
  let ws: WorkspaceStore

  beforeEach(() => {
    resetTabIdSequenceForTests()
    ws = new WorkspaceStore()
    localStorage.clear()
  })

  it('toggleSplit with fewer than four tabs pairs the active tab with a new Home tab to its right', () => {
    const activeId = ws.activeTabId
    ws.toggleSplit()

    expect(ws.tabs).toHaveLength(2)
    expect(ws.split).toEqual({ leftId: activeId, rightId: ws.tabs[1]!.id, ratio: 0.5 })
    expect(ws.navigationFor(ws.tabs[1]!.id).current).toEqual({ name: 'home' })
  })

  it('toggleSplit at the four-tab cap pairs with the right neighbour when the active tab is not last', () => {
    ws.openTab()
    ws.openTab()
    ws.openTab()
    const secondTabId = ws.tabs[1]!.id
    ws.activateTab(secondTabId)

    ws.toggleSplit()

    expect(ws.tabs).toHaveLength(4)
    expect(ws.split).toEqual({ leftId: secondTabId, rightId: ws.tabs[2]!.id, ratio: 0.5 })
  })

  it('toggleSplit at the four-tab cap pairs with the left neighbour when the active tab is last', () => {
    ws.openTab()
    ws.openTab()
    ws.openTab()
    const lastTabId = ws.tabs[3]!.id
    ws.activateTab(lastTabId)

    ws.toggleSplit()

    expect(ws.tabs).toHaveLength(4)
    expect(ws.split).toEqual({ leftId: ws.tabs[2]!.id, rightId: lastTabId, ratio: 0.5 })
  })

  it('toggling split off ungroups the pair without closing anything', () => {
    ws.toggleSplit()
    const tabCount = ws.tabs.length

    ws.toggleSplit()

    expect(ws.split).toBeNull()
    expect(ws.tabs).toHaveLength(tabCount)
  })

  it('closing a grouped tab dissolves the group (Review Focus #2)', () => {
    ws.toggleSplit()
    const rightId = ws.split!.rightId

    ws.closeTab(rightId)

    expect(ws.split).toBeNull()
    expect(ws.tabs).toHaveLength(1)
  })

  it('closing a tab that is not in the group leaves the group intact', () => {
    ws.toggleSplit()
    const thirdId = ws.openTab()!

    ws.closeTab(thirdId)

    expect(ws.split).not.toBeNull()
    expect(ws.tabs).toHaveLength(2)
  })

  it('visiblePaneIds is the pair when the active tab is a member, otherwise the active tab alone', () => {
    ws.toggleSplit()
    const { leftId, rightId } = ws.split!
    expect(ws.visiblePaneIds).toEqual([leftId, rightId])

    const thirdId = ws.openTab()!
    expect(ws.activeTabId).toBe(thirdId)
    expect(ws.visiblePaneIds).toEqual([thirdId])

    // Selecting either grouped tab makes the pair reappear (spec, Split view).
    ws.activateTab(leftId)
    expect(ws.visiblePaneIds).toEqual([leftId, rightId])
  })

  it('setSplitRatio clamps to [0.15, 0.85] and persists to localStorage', () => {
    ws.toggleSplit()
    ws.setSplitRatio(0.05)
    expect(ws.split!.ratio).toBe(0.15)
    ws.setSplitRatio(0.99)
    expect(ws.split!.ratio).toBe(0.85)
    ws.setSplitRatio(0.4)
    expect(localStorage.getItem('entropia-workspace-split-ratio')).toBe('0.4')
  })

  it('a fresh WorkspaceStore reads a persisted ratio for its next toggleSplit', () => {
    localStorage.setItem('entropia-workspace-split-ratio', '0.3')
    const fresh = new WorkspaceStore()
    fresh.toggleSplit()
    expect(fresh.split!.ratio).toBe(0.3)
  })
})
