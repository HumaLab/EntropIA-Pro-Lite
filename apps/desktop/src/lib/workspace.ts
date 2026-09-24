/**
 * Workspace store for the desktop app: owns the tab list and the active
 * tab (Stage 1), then the split-view group and its ratio (Stage 3).
 *
 * Each tab owns its own `NavigationStore` instance (see `./navigation`), so
 * two tabs never share history. The module-level `navigation` singleton is
 * retired in Task 1.6 once nothing imports it anymore — every consumer
 * reaches a `NavigationStore` either through a tab (`navigationFor`,
 * `activeNavigation`) or through `./pane-context`'s `getNavigation()`.
 */

import { NavigationStore, type View } from './navigation'

export interface Tab {
  readonly id: string
  readonly navigation: NavigationStore
}

/**
 * Two tabs shown side by side (Task 3.1). Always `null` until Stage 3 wires
 * up split-view interactions; the field lives on the snapshot from Stage 1
 * onward so the Stage-2 tab-strip component can read it without a shape
 * change later.
 */
export interface SplitGroup {
  leftId: string
  rightId: string
  ratio: number
}

export interface WorkspaceSnapshot {
  tabs: readonly Tab[]
  activeTabId: string
  split: SplitGroup | null
}

type WorkspaceSubscriber = (snapshot: WorkspaceSnapshot) => void

/** Chrome-style: four tabs is the ceiling, matching the design's `+` cap. */
export const MAX_TABS = 4

let tabIdSeq = 0
/** Test-only: reset the id counter so assertions on generated ids are stable
 *  across files that each construct their own `WorkspaceStore`. */
export function resetTabIdSequenceForTests(): void {
  tabIdSeq = 0
}

function nextTabId(): string {
  tabIdSeq += 1
  return `tab-${tabIdSeq}`
}

export class WorkspaceStore {
  // Field initializers (not constructor assignments) because `createTab()`
  // subscribes the new tab's navigation, and `NavigationStore.subscribe`
  // invokes its callback synchronously — which calls `this.emit()` before
  // the constructor body's own assignments would otherwise run.
  private tabList: Tab[] = []
  private activeId = ''
  private readonly split: SplitGroup | null = null
  private readonly subscribers = new Set<WorkspaceSubscriber>()
  private readonly tabUnsubscribes = new Map<string, () => void>()

  constructor() {
    const home = this.createTab()
    this.tabList = [home]
    this.activeId = home.id
  }

  private createTab(): Tab {
    const nav = new NavigationStore()
    const tab: Tab = { id: nextTabId(), navigation: nav }
    // A tab's own navigation changes (including locale-driven breadcrumb
    // re-emits) must be visible to anything deriving tab titles from
    // `$workspace`, so the workspace re-emits whenever any tab's history does.
    this.tabUnsubscribes.set(
      tab.id,
      nav.subscribe(() => this.emit())
    )
    return tab
  }

  subscribe(run: WorkspaceSubscriber): () => void {
    this.subscribers.add(run)
    run(this.snapshot())
    return () => {
      this.subscribers.delete(run)
    }
  }

  protected snapshot(): WorkspaceSnapshot {
    return { tabs: [...this.tabList], activeTabId: this.activeId, split: this.split }
  }

  protected emit(): void {
    const snapshot = this.snapshot()
    this.subscribers.forEach((run) => run(snapshot))
  }

  get tabs(): readonly Tab[] {
    return this.tabList
  }

  get activeTabId(): string {
    return this.activeId
  }

  get activeNavigation(): NavigationStore {
    return this.navigationFor(this.activeId)
  }

  navigationFor(tabId: string): NavigationStore {
    const tab = this.tabList.find((candidate) => candidate.id === tabId)
    if (!tab) throw new Error(`[workspace] Unknown tab: ${tabId}`)
    return tab.navigation
  }

  /** A new tab opens on Home, browser-tab style. Returns the new tab's id,
   *  or `null` when the four-tab cap is already reached (the `+` button is
   *  disabled at that point, but the store enforces it independently). */
  openTab(view: View = { name: 'home' }): string | null {
    if (this.tabList.length >= MAX_TABS) return null
    const tab = this.createTab()
    if (view.name !== 'home') tab.navigation.navigate(view)
    this.tabList = [...this.tabList, tab]
    this.activeId = tab.id
    this.emit()
    return tab.id
  }

  /** The last remaining tab cannot be closed. */
  closeTab(tabId: string): void {
    if (this.tabList.length <= 1) return
    const index = this.tabList.findIndex((tab) => tab.id === tabId)
    if (index === -1) return

    this.tabUnsubscribes.get(tabId)?.()
    this.tabUnsubscribes.delete(tabId)
    const remaining = this.tabList.filter((tab) => tab.id !== tabId)

    if (this.activeId === tabId) {
      const fallbackIndex = Math.min(Math.max(0, index - 1), remaining.length - 1)
      this.activeId = remaining[fallbackIndex]!.id
    }

    this.tabList = remaining
    this.emit()
  }

  activateTab(tabId: string): void {
    if (tabId === this.activeId) return
    if (!this.tabList.some((tab) => tab.id === tabId)) return
    this.activeId = tabId
    this.emit()
  }

  /**
   * Push `view` on the active tab — unless it is Writing and some tab
   * already shows it, in which case that tab is activated instead (spec,
   * Rules across tabs: "Writing can be open in only one tab").
   */
  navigateActive(view: View): void {
    if (view.name === 'writing') {
      const writingTab = this.tabList.find((tab) => tab.navigation.current.name === 'writing')
      if (writingTab) {
        this.activateTab(writingTab.id)
        return
      }
    }
    this.activeNavigation.navigate(view)
  }

  private forgetAcrossTabs(prune: (nav: NavigationStore) => void): void {
    this.tabList.forEach((tab) => prune(tab.navigation))
  }

  /** A deleted collection takes its documents with it, in every tab. */
  forgetCollection(collectionId: string): void {
    this.forgetAcrossTabs((nav) => nav.forgetCollection(collectionId))
  }

  /** A deleted document, in every tab. */
  forgetItem(itemId: string): void {
    this.forgetAcrossTabs((nav) => nav.forgetItem(itemId))
  }

  /** A deleted page, in every tab. */
  forgetAsset(assetId: string): void {
    this.forgetAcrossTabs((nav) => nav.forgetAsset(assetId))
  }

  /** A discarded writing document, in every tab. */
  forgetWriting(documentId: string): void {
    this.forgetAcrossTabs((nav) => nav.forgetWriting(documentId))
  }

  /** A deleted research job, in every tab. */
  forgetResearch(jobId: string): void {
    this.forgetAcrossTabs((nav) => nav.forgetResearch(jobId))
  }
}

export const workspace = new WorkspaceStore()
