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
 * Two tabs shown side by side (Task 3.1). `null` when split view is off; the
 * field lives on the snapshot from Stage 1 onward so the Stage-2 tab-strip
 * component can read it without a shape change later.
 */
export interface SplitGroup {
  readonly leftId: string
  readonly rightId: string
  readonly ratio: number
}

export type SplitState = SplitGroup | null

export interface WorkspaceSnapshot {
  tabs: readonly Tab[]
  activeTabId: string
  split: SplitGroup | null
  writingOwnerId: string | null
}

type WorkspaceSubscriber = (snapshot: WorkspaceSnapshot) => void

/** Chrome-style: four tabs is the ceiling, matching the design's `+` cap. */
export const MAX_TABS = 4
const SPLIT_RATIO_STORAGE_KEY = 'entropia-workspace-split-ratio'
// Each pane can shrink to at most 25% of the split (spec: user rule, split
// view). This is the coarse bound applied before any DOM measurement exists;
// `clampSplitRatio` (split-ratio.ts) applies the same [0.25, 0.75] bound
// together with the 320px-per-pane floor once a container size is known,
// taking whichever of the two is stricter.
const MIN_RATIO = 0.25
const MAX_RATIO = 0.75

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
  private splitState: SplitState = null
  // The tab that "owns" Writing (spec, Rules across tabs: "Writing can be
  // open in only one tab"). Set on first arrival, when nobody owns it yet;
  // cleared only when the owner itself leaves `writing` or its tab closes —
  // never stolen by another tab that reaches `writing` while someone already
  // owns it (controller fix round 1, Task 3.3 review: a pane can reach
  // `writing` on its own NavigationStore directly, via Back/forward history,
  // bypassing navigateActive()'s redirect — "first tab in tab-list order"
  // could then hand ownership away from an incumbent actively showing it).
  private writingOwnerTabId: string | null = null
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
    // `syncWritingOwner` runs first so a change that affects ownership is
    // already reflected in the snapshot this emit carries.
    this.tabUnsubscribes.set(
      tab.id,
      nav.subscribe(() => {
        this.syncWritingOwner(tab.id, nav.current.name === 'writing')
        this.emit()
      })
    )
    return tab
  }

  /**
   * Called on every navigation change of `changedTabId` (any method that
   * mutates a `NavigationStore` — `navigate`, `back`, `forget*`,
   * `resetToPath`, `replace` — all funnel through its own `subscribe`).
   *
   * `changedIsWriting` is passed directly rather than looked up via
   * `this.tabList.find(...)`: at the very first (synchronous) subscribe
   * call inside `createTab()`, and at `openTab(view)`'s own initial
   * `navigate(view)` call — which CAN grant ownership right here, before
   * the new tab is pushed into `tabList` — a list lookup would silently
   * miss it. The "release ownership, find a remaining incumbent" branch
   * below is the only one that reads `tabList`, and it only ever runs for a
   * tab that IS the current owner; such a tab is always list-resident by
   * then, because a tab that grants itself ownership before being pushed is
   * pushed immediately afterward, in the same synchronous call, before
   * anything else can navigate it again.
   */
  private syncWritingOwner(changedTabId: string, changedIsWriting: boolean): void {
    if (this.writingOwnerTabId === null) {
      if (changedIsWriting) this.writingOwnerTabId = changedTabId
      return
    }
    if (this.writingOwnerTabId !== changedTabId) return
    if (changedIsWriting) return
    // The owner navigated away from writing — release ownership. If another
    // tab already shows writing (the exact hazard this tracks), that
    // incumbent becomes the new owner instead of staying ownerless.
    this.writingOwnerTabId =
      this.tabList.find((t) => t.id !== changedTabId && t.navigation.current.name === 'writing')
        ?.id ?? null
  }

  subscribe(run: WorkspaceSubscriber): () => void {
    this.subscribers.add(run)
    run(this.snapshot())
    return () => {
      this.subscribers.delete(run)
    }
  }

  protected snapshot(): WorkspaceSnapshot {
    return {
      tabs: [...this.tabList],
      activeTabId: this.activeId,
      split: this.splitState,
      writingOwnerId: this.writingOwnerTabId,
    }
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

  get split(): SplitState {
    return this.splitState
  }

  /** The tab that owns Writing, or `null` if no tab currently shows it. */
  get writingOwnerId(): string | null {
    return this.writingOwnerTabId
  }

  /** The pair when the active tab is one of its members (so the group is
   *  "shown"), otherwise the active tab alone — the group persists either
   *  way and reappears the moment either of its tabs is reselected (spec,
   *  Split view). */
  get visiblePaneIds(): readonly string[] {
    if (
      this.splitState &&
      (this.activeId === this.splitState.leftId || this.activeId === this.splitState.rightId)
    ) {
      return [this.splitState.leftId, this.splitState.rightId]
    }
    return [this.activeId]
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
    this.tabList[index]!.navigation.dispose()
    const remaining = this.tabList.filter((tab) => tab.id !== tabId)

    // A dangling split pointing at a closed tab is never valid (Review Focus #2).
    if (
      this.splitState &&
      (this.splitState.leftId === tabId || this.splitState.rightId === tabId)
    ) {
      this.splitState = null
    }

    // The owner tab closed — hand ownership to a remaining incumbent (the
    // same hazard `syncWritingOwner` tracks) rather than leaving it stuck on
    // a closed tab id.
    if (this.writingOwnerTabId === tabId) {
      this.writingOwnerTabId =
        remaining.find((tab) => tab.navigation.current.name === 'writing')?.id ?? null
    }

    if (this.activeId === tabId) {
      const fallbackIndex = Math.min(Math.max(0, index - 1), remaining.length - 1)
      this.activeId = remaining[fallbackIndex]!.id
    }

    this.tabList = remaining
    this.emit()
  }

  /** Releases every tab's subscriptions and drops this store's subscribers. */
  dispose(): void {
    this.tabUnsubscribes.forEach((unsubscribe) => unsubscribe())
    this.tabUnsubscribes.clear()
    this.tabList.forEach((tab) => tab.navigation.dispose())
    this.subscribers.clear()
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
   * Rules across tabs: "Writing can be open in only one tab"). A request
   * for a specific document is carried over to that tab, so it shows the
   * document asked for rather than whichever one it had open.
   */
  navigateActive(view: View): void {
    if (
      view.name === 'writing' &&
      this.writingOwnerTabId !== null &&
      this.writingOwnerTabId !== this.activeId
    ) {
      const owner = this.writingOwnerTabId
      this.activateTab(owner)
      if (view.documentId) this.navigationFor(owner).navigate(view)
      return
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

  /**
   * Turning split on pairs the active tab with a new Home tab inserted to
   * its right; at the four-tab cap it pairs with the right neighbour, or
   * the left neighbour when the active tab is last (spec, Split view). The
   * active tab itself does not change. Turning split off ungroups the pair
   * — both tabs remain, and the active one stays active.
   */
  toggleSplit(): void {
    if (this.splitState) {
      this.splitState = null
      this.emit()
      return
    }

    const activeIndex = this.tabList.findIndex((tab) => tab.id === this.activeId)
    let leftId: string
    let rightId: string

    if (this.tabList.length < MAX_TABS) {
      const newTab = this.createTab()
      const insertAt = activeIndex + 1
      this.tabList = [...this.tabList.slice(0, insertAt), newTab, ...this.tabList.slice(insertAt)]
      leftId = this.activeId
      rightId = newTab.id
    } else if (activeIndex === this.tabList.length - 1) {
      leftId = this.tabList[activeIndex - 1]!.id
      rightId = this.activeId
    } else {
      leftId = this.activeId
      rightId = this.tabList[activeIndex + 1]!.id
    }

    this.splitState = { leftId, rightId, ratio: this.loadRatio() }
    this.emit()
  }

  /**
   * Clamped to [0.25, 0.75] and persisted (best-effort) to localStorage.
   * A drag in progress passes `persist: false` so storage is written once,
   * when the gesture settles, rather than on every pointermove.
   */
  setSplitRatio(ratio: number, { persist = true }: { persist?: boolean } = {}): void {
    if (!this.splitState) return
    const clamped = Math.min(MAX_RATIO, Math.max(MIN_RATIO, ratio))
    this.splitState = { ...this.splitState, ratio: clamped }
    if (persist) this.persistRatio(clamped)
    this.emit()
  }

  private loadRatio(): number {
    try {
      const raw = localStorage.getItem(SPLIT_RATIO_STORAGE_KEY)
      const value = raw ? Number(raw) : NaN
      return Number.isFinite(value) ? Math.min(MAX_RATIO, Math.max(MIN_RATIO, value)) : 0.5
    } catch {
      // storage unavailable — the default 50/50 ratio applies this session
      return 0.5
    }
  }

  private persistRatio(ratio: number): void {
    try {
      localStorage.setItem(SPLIT_RATIO_STORAGE_KEY, String(ratio))
    } catch {
      // storage unavailable — the ratio stays session-only
    }
  }
}

export const workspace = new WorkspaceStore()
