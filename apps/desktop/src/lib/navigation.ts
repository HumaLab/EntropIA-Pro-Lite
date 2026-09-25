/**
 * Navigation store for the desktop app.
 * Exposes imperative API plus a lightweight subscription mechanism
 * so Svelte components can react to navigation changes.
 */

import { locale, t } from './i18n'

export type View =
  | { name: 'home' }
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
      /**
       * A character range into the asset's extracted text, when the view was
       * opened by following a citation (§10.2 step 4).
       *
       * Offsets rather than geometry, because that is the anchor a citation
       * stores and the only addressing the corpus persists — the G3 entry of
       * Unit 4 records why there is no pixel mapping to offer instead.
       *
       * The quoted text travels with them because the two have different jobs.
       * The offsets are the durable anchor, checked against the raw extraction.
       * The text is what locates the fragment in the *rendered* pane, which
       * `renderOcrHtml` has rewritten and where those offsets name nothing.
       */
      citationRange?: { start: number; end: number; text: string } | null
      /**
       * A note to open, when the view was reached by following a note link
       * (§13). The manuscript keeps its snapshot either way; this is only about
       * showing the writer the note it came from.
       */
      noteId?: string | null
    }
  | { name: 'db-browser' }
  | { name: 'rag-chat' }
  | { name: 'research' }
  | { name: 'investigation'; jobId: string; title: string }
  // Escritura (plan-editor.md §6). `documentId` is optional so the section can
  // open on its document list and then deep-link into one.
  | { name: 'writing'; documentId?: string | null; documentTitle?: string | null }
  | { name: 'settings' }

type RootSectionView = Extract<
  View,
  { name: 'home' | 'settings' | 'db-browser' | 'rag-chat' | 'research' | 'writing' }
>

type NavigationSnapshot = {
  history: View[]
  current: View
  canGoBack: boolean
  breadcrumb: string[]
}

type NavigationSubscriber = (snapshot: NavigationSnapshot) => void

/**
 * Structural equality over a `View`'s own fields (including nested objects
 * like `citationRange`), used to make pushing the current view a no-op —
 * browser tabs don't grow a history entry when you click the link you're
 * already on.
 */
function viewsEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false
  const aRecord = a as Record<string, unknown>
  const bRecord = b as Record<string, unknown>
  const aKeys = Object.keys(aRecord)
  const bKeys = Object.keys(bRecord)
  if (aKeys.length !== bKeys.length) return false
  return aKeys.every((key) => viewsEqual(aRecord[key], bRecord[key]))
}

/**
 * Upper bound on how deep history can grow. Paging through many documents
 * or sections in one session should not accumulate an unbounded array —
 * drop the oldest entries first, but the root screen (`history[0]`, `home`
 * in production) always survives so Back still terminates somewhere sane.
 */
const HISTORY_CAP = 200

export class NavigationStore {
  private _history: View[] = [{ name: 'home' }]
  private readonly _subscribers = new Set<NavigationSubscriber>()
  private readonly _unsubscribeLocale: () => void

  constructor() {
    this._unsubscribeLocale = locale.subscribe(() => {
      this.emit()
    })
  }

  /**
   * Stops following the locale. A tab's store lives as long as its tab, and
   * the locale store is module-wide, so without this every closed tab stays
   * reachable (and keeps re-emitting) for the rest of the session.
   */
  dispose(): void {
    this._unsubscribeLocale()
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

    // The start page has no breadcrumb: it is where the path begins, not a stop on it.
    if (view.name === 'home') return []
    if (view.name === 'collections') return [root]
    if (view.name === 'collection') return [root, view.collectionName]
    if (view.name === 'item') {
      const breadcrumb = [root, view.collectionName]
      if (view.assetLabel) breadcrumb.push(view.assetLabel)
      return breadcrumb
    }
    if (view.name === 'db-browser') return [root, t('nav.dbBrowser')]
    if (view.name === 'rag-chat') return [root, t('nav.ragChat')]
    if (view.name === 'research') return [root, t('nav.research')]
    if (view.name === 'investigation') return [root, t('nav.research'), view.title]
    if (view.name === 'writing') {
      return view.documentTitle
        ? [root, t('writing.title'), view.documentTitle]
        : [root, t('writing.title')]
    }
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

  /**
   * Push a screen, browser-tab style: every distinct screen change adds one
   * history entry, and Back always pops exactly one. A no-op when `view` is
   * the screen already showing, so re-selecting the current section or
   * document doesn't grow history.
   */
  navigate(view: View): void {
    if (viewsEqual(view, this.current)) return
    const next = [...this._history, view]
    this._history = this.capHistory(next)
    this.emit()
  }

  /**
   * Push a top-level section (Chat, Investigación, Escritura, Configuración,
   * Base de datos). Kept as its own name so section-icon call sites read
   * intent-first, but it is exactly `navigate` now — no history rebuild.
   */
  openRootSection(view: RootSectionView): void {
    this.navigate(view)
  }

  /** Drop the oldest entries once history exceeds the cap, keeping the root. */
  private capHistory(history: View[]): View[] {
    if (history.length <= HISTORY_CAP) return history
    const overflow = history.length - HISTORY_CAP
    return [history[0]!, ...history.slice(1 + overflow)]
  }

  /**
   * Replace the full history with a canonical path.
   *
   * No longer used by ordinary navigation flows — those push the single
   * screen the user actually moved to, so Back can return to what was there
   * before. Kept for tests and startup/reset scenarios that need to seed a
   * known history in one call.
   */
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

  /**
   * Remove every history entry matching `predicate` — a deleted subject no
   * longer has a screen to return to — then collapse the consecutive
   * duplicates removal can leave behind (two entries that were only distinct
   * because of what used to sit between them), and never leave history
   * empty: `home` survives even a predicate that clears everything else.
   *
   * If the removed entries included the current one, whatever is left on
   * top becomes current — the same landing `back()` would produce. A no-op,
   * no emit, when nothing actually changes, so a caller's own `replace()`
   * for the current subject (already run before this) and this prune never
   * fight: pruning what `replace` already moved away from is idempotent.
   */
  forget(predicate: (view: View) => boolean): void {
    const filtered = this._history.filter((view) => !predicate(view))
    const collapsed = this.collapseConsecutiveDuplicates(filtered)
    const next: View[] = collapsed.length > 0 ? collapsed : [{ name: 'home' }]
    if (this.historyEqual(next, this._history)) return
    this._history = next
    this.emit()
  }

  /** A deleted collection takes its documents with it: neither has a screen left. */
  forgetCollection(collectionId: string): void {
    this.forget(
      (view) =>
        (view.name === 'collection' && view.id === collectionId) ||
        (view.name === 'item' && view.collectionId === collectionId)
    )
  }

  /** A deleted document removes every page of it, whatever the asset. */
  forgetItem(itemId: string): void {
    this.forget((view) => view.name === 'item' && view.itemId === itemId)
  }

  /** A deleted page leaves the document's other pages untouched. */
  forgetAsset(assetId: string): void {
    this.forget((view) => view.name === 'item' && view.assetId === assetId)
  }

  /** A discarded writing document (plan-editor.md §6). */
  forgetWriting(documentId: string): void {
    this.forget((view) => view.name === 'writing' && view.documentId === documentId)
  }

  /** A deleted research job (Investigación). */
  forgetResearch(jobId: string): void {
    this.forget((view) => view.name === 'investigation' && view.jobId === jobId)
  }

  private collapseConsecutiveDuplicates(history: View[]): View[] {
    const result: View[] = []
    for (const view of history) {
      if (result.length === 0 || !viewsEqual(view, result.at(-1))) {
        result.push(view)
      }
    }
    return result
  }

  private historyEqual(a: View[], b: View[]): boolean {
    return a.length === b.length && a.every((view, index) => viewsEqual(view, b[index]))
  }
}
