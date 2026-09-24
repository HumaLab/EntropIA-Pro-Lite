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
