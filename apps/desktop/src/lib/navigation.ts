/**
 * Navigation store for the desktop app.
 * Exposes imperative API plus a lightweight subscription mechanism
 * so Svelte components can react to navigation changes.
 */

import { locale, t } from './i18n'

export type View =
  | { name: 'collections' }
  | { name: 'collection'; id: string; collectionName: string }
  | {
      name: 'item'
      collectionId: string
      collectionName: string
      itemId: string
      itemTitle: string
      assetId?: string | null
      assetLabel?: string | null
    }
  | { name: 'db-browser' }
  | { name: 'rag-chat' }
  | { name: 'settings' }

type RootSectionView = Extract<View, { name: 'settings' | 'db-browser' | 'rag-chat' }>

type NavigationSnapshot = {
  history: View[]
  current: View
  canGoBack: boolean
  breadcrumb: string[]
}

type NavigationSubscriber = (snapshot: NavigationSnapshot) => void

export class NavigationStore {
  private _history: View[] = [{ name: 'collections' }]
  private readonly _subscribers = new Set<NavigationSubscriber>()

  constructor() {
    locale.subscribe(() => {
      this.emit()
    })
  }

  subscribe(run: NavigationSubscriber): () => void {
    this._subscribers.add(run)
    run(this.snapshot())
    return () => {
      this._subscribers.delete(run)
    }
  }

  private snapshot(): NavigationSnapshot {
    const history = [...this._history]
    const current = history.at(-1)!
    return {
      history,
      current,
      canGoBack: history.length > 1,
      breadcrumb: this.breadcrumbForView(current),
    }
  }

  private breadcrumbForView(view: View): string[] {
    const root = t('nav.collections')

    if (view.name === 'collections') return [root]
    if (view.name === 'collection') return [root, view.collectionName]
    if (view.name === 'item') {
      const breadcrumb = [root, view.collectionName]
      if (view.assetLabel) breadcrumb.push(view.assetLabel)
      return breadcrumb
    }
    if (view.name === 'db-browser') return [root, t('nav.dbBrowser')]
    if (view.name === 'rag-chat') return [root, t('nav.ragChat')]
    return [root, t('nav.settings')]
  }

  private emit(): void {
    const snapshot = this.snapshot()
    this._subscribers.forEach((run) => run(snapshot))
  }

  get current(): View {
    return this._history.at(-1)!
  }

  get canGoBack(): boolean {
    return this._history.length > 1
  }

  get breadcrumb(): string[] {
    return this.snapshot().breadcrumb
  }

  navigate(view: View): void {
    this._history = [...this._history, view]
    this.emit()
  }

  /** Navigate to a root-level section using a canonical breadcrumb path. */
  openRootSection(view: RootSectionView): void {
    this.resetToPath([{ name: 'collections' }, view])
  }

  /** Replace the full history with a canonical path. */
  resetToPath(path: [View, ...View[]]): void {
    this._history = [...path]
    this.emit()
  }

  /** Replace the current view — useful for navigating between sibling items without stacking. */
  replace(view: View): void {
    if (this._history.length === 0) {
      this._history = [view]
    } else {
      this._history = [...this._history.slice(0, -1), view]
    }
    this.emit()
  }

  back(): void {
    if (this._history.length > 1) {
      this._history = this._history.slice(0, -1)
      this.emit()
    }
  }
}

export const navigation = new NavigationStore()
