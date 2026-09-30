/**
 * The browser's tabs, seen from the frontend.
 *
 * The backend owns the tab list (a page can open a tab while the person looks
 * at another one), so this only reads what it sends: a snapshot of every tab
 * and which one is active. Everything in a tab came from a web page (its title
 * and address), so the view renders it as text and never as markup.
 */

/** One tab; mirrors `Tab` in `navegador/tabs.rs`. `url` is null until it has a page. */
export type BrowserTab = {
  id: number
  url: string | null
  title: string | null
  /** Why the last navigation of this tab was refused, until one works. */
  blocked: string | null
}

/** Mirrors `BrowserState` in `navegador/tabs.rs`. */
export type BrowserState = {
  tabs: BrowserTab[]
  active: number | null
  /** Grows with every state the backend makes; the higher one is the newer. */
  revision: number
}

/** The browser's own limit, unrelated to how many workspace tabs the app allows. */
export const MAX_TABS = 4

/** Longest tab title shown, in characters. */
const MAX_TITLE = 60

export const EMPTY_BROWSER: BrowserState = { tabs: [], active: null, revision: 0 }

/** Whether one more tab may open when `count` already exist. */
export function canOpenTab(count: number): boolean {
  return count < MAX_TABS
}

/** The host of an address; a blob address belongs to the host inside it. */
export function hostOf(url: string | null | undefined): string | null {
  if (!url) return null
  try {
    const parsed = new URL(url)
    if (parsed.protocol === 'blob:') return hostOf(parsed.pathname)
    return parsed.hostname || null
  } catch {
    return null
  }
}

function cut(text: string): string {
  const chars = Array.from(text)
  return chars.length > MAX_TITLE ? `${chars.slice(0, MAX_TITLE - 1).join('')}…` : text
}

/** What a tab is called: its page title, else its host; null for a blank tab. */
export function tabTitle(tab: BrowserTab): string | null {
  const title = tab.title?.trim()
  if (title) return cut(title)
  const host = hostOf(tab.url)
  return host ? cut(host) : null
}

export type TabItem = {
  id: number
  label: string
  url: string | null
  active: boolean
  hasPage: boolean
}

/** What the strip shows, in order; a tab with no name gets `fallback`. */
export function describeTabs(state: BrowserState, fallback: string): TabItem[] {
  return state.tabs.map((tab) => ({
    id: tab.id,
    label: tabTitle(tab) ?? fallback,
    url: tab.url,
    active: tab.id === state.active,
    hasPage: tab.url !== null,
  }))
}

export function activeTab(state: BrowserState): BrowserTab | null {
  return state.tabs.find((tab) => tab.id === state.active) ?? null
}

function text(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

function parseTab(raw: unknown): BrowserTab | null {
  if (typeof raw !== 'object' || raw === null) return null
  const item = raw as Record<string, unknown>
  const id = item.id
  if (typeof id !== 'number' || !Number.isInteger(id) || id < 0) return null
  return { id, url: text(item.url), title: text(item.title), blocked: text(item.blocked) }
}

/**
 * Read what the backend sent. Anything that is not a browser state is an empty
 * browser, never an exception: a state comes from an event or a command, and a
 * bad one must not take the view down.
 */
export function parseBrowserState(raw: unknown): BrowserState {
  if (typeof raw !== 'object' || raw === null || !Array.isArray((raw as BrowserState).tabs)) {
    return EMPTY_BROWSER
  }
  const source = raw as { tabs: unknown[]; active?: unknown; revision?: unknown }
  const tabs = source.tabs
    .map(parseTab)
    .filter((tab): tab is BrowserTab => tab !== null)
    .slice(0, MAX_TABS)
  const active = tabs.some((tab) => tab.id === source.active) ? (source.active as number) : null
  const revision =
    typeof source.revision === 'number' && Number.isFinite(source.revision) ? source.revision : 0
  return { tabs, active, revision }
}

/**
 * Whether `next` should replace `current`. A command's answer and an event can
 * arrive in either order; the higher revision is the newer. A state with no
 * revision (zero) is not versioned and always applies.
 */
export function isNewer(next: BrowserState, current: BrowserState): boolean {
  return next.revision === 0 || next.revision > current.revision
}
