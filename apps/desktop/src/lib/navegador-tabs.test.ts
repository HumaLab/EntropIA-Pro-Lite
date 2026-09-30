import { describe, expect, it } from 'vitest'
import tabsRs from '../../src-tauri/src/navegador/tabs.rs?raw'
import {
  EMPTY_BROWSER,
  MAX_TABS,
  activeTab,
  canOpenTab,
  describeTabs,
  hostOf,
  isNewer,
  parseBrowserState,
  tabTitle,
  type BrowserState,
  type BrowserTab,
} from './navegador-tabs'

const tab = (id: number, patch: Partial<BrowserTab> = {}): BrowserTab => ({
  id,
  url: null,
  title: null,
  blocked: null,
  ...patch,
})

const state = (tabs: BrowserTab[], active: number | null, revision = 1): BrowserState => ({
  tabs,
  active,
  revision,
})

describe('the tab limit', () => {
  it('is four per browser', () => {
    expect(MAX_TABS).toBe(4)
  })

  it('lets a tab open until the browser is full', () => {
    expect(canOpenTab(0)).toBe(true)
    expect(canOpenTab(MAX_TABS - 1)).toBe(true)
    expect(canOpenTab(MAX_TABS)).toBe(false)
    expect(canOpenTab(MAX_TABS + 1)).toBe(false)
  })
})

describe('hostOf', () => {
  it('reads the host of an address', () => {
    expect(hostOf('https://www.example.com/a?b=1')).toBe('www.example.com')
    expect(hostOf('http://example.org:8080/')).toBe('example.org')
  })

  it('reads the host a blob address belongs to', () => {
    expect(hostOf('blob:https://github.com/6c1f2d3e')).toBe('github.com')
  })

  it('has none for what is not an address', () => {
    expect(hostOf(null)).toBeNull()
    expect(hostOf('')).toBeNull()
    expect(hostOf('not a url')).toBeNull()
    expect(hostOf('about:blank')).toBeNull()
  })
})

describe('tabTitle', () => {
  it('prefers the page title, then the host', () => {
    expect(tabTitle(tab(1, { url: 'https://a.test/x', title: 'A page' }))).toBe('A page')
    expect(tabTitle(tab(1, { url: 'https://a.test/x', title: '   ' }))).toBe('a.test')
    expect(tabTitle(tab(1, { url: 'https://a.test/x' }))).toBe('a.test')
  })

  it('has nothing to show for a blank tab', () => {
    expect(tabTitle(tab(1))).toBeNull()
  })

  it('keeps a title as plain text, cut to a length that fits a tab', () => {
    const long = 'x'.repeat(200)
    expect(tabTitle(tab(1, { title: long }))!.length).toBeLessThanOrEqual(60)
    expect(tabTitle(tab(1, { title: '<b>bold</b>' }))).toBe('<b>bold</b>')
  })
})

describe('describeTabs', () => {
  const browser = state(
    [
      tab(1, { url: 'https://a.test/', title: 'First' }),
      tab(3),
      tab(4, { url: 'https://c.test/' }),
    ],
    3
  )

  it('lists every tab in order and marks the active one', () => {
    const items = describeTabs(browser, 'New tab')
    expect(items.map((item) => item.id)).toEqual([1, 3, 4])
    expect(items.map((item) => item.active)).toEqual([false, true, false])
  })

  it('names a blank tab with the fallback and a tab with a page by its title or host', () => {
    expect(describeTabs(browser, 'New tab').map((item) => item.label)).toEqual([
      'First',
      'New tab',
      'c.test',
    ])
  })

  it('says which tabs have a page', () => {
    expect(describeTabs(browser, 'New tab').map((item) => item.hasPage)).toEqual([
      true,
      false,
      true,
    ])
  })

  it('is empty for a browser that is not open', () => {
    expect(describeTabs(EMPTY_BROWSER, 'New tab')).toEqual([])
  })
})

describe('activeTab', () => {
  it('is the tab the state points at', () => {
    const browser = state([tab(1), tab(2, { url: 'https://b.test/' })], 2)
    expect(activeTab(browser)?.id).toBe(2)
  })

  it('is none without tabs or with an active id that is not in the list', () => {
    expect(activeTab(EMPTY_BROWSER)).toBeNull()
    expect(activeTab(state([tab(1)], 9))).toBeNull()
    expect(activeTab(state([tab(1)], null))).toBeNull()
  })
})

describe('parseBrowserState', () => {
  it('accepts what the backend sends', () => {
    const raw = {
      tabs: [{ id: 2, url: 'https://a.test/', title: 'A', blocked: null }, { id: 5 }],
      active: 5,
      revision: 7,
    }
    expect(parseBrowserState(raw)).toEqual({
      tabs: [
        { id: 2, url: 'https://a.test/', title: 'A', blocked: null },
        { id: 5, url: null, title: null, blocked: null },
      ],
      active: 5,
      revision: 7,
    })
  })

  it('takes anything else for an empty browser instead of throwing', () => {
    for (const raw of [undefined, null, 'x', 3, [], { url: 'https://a.test/' }, { tabs: 'no' }]) {
      expect(parseBrowserState(raw)).toEqual(EMPTY_BROWSER)
    }
  })

  it('drops a tab without a usable id and ignores an active id that is not listed', () => {
    const parsed = parseBrowserState({
      tabs: [{ id: 'x' }, { id: -1 }, { id: 1.5 }, null, { id: 2 }],
      active: 9,
      revision: 2,
    })
    expect(parsed.tabs.map((t) => t.id)).toEqual([2])
    expect(parsed.active).toBeNull()
  })

  it('keeps no more tabs than a browser may have', () => {
    const tabs = Array.from({ length: 10 }, (_, i) => ({ id: i + 1 }))
    expect(parseBrowserState({ tabs, active: 1, revision: 1 }).tabs).toHaveLength(MAX_TABS)
  })

  it('reads a missing or odd revision as zero', () => {
    expect(parseBrowserState({ tabs: [{ id: 1 }], active: 1 }).revision).toBe(0)
    expect(parseBrowserState({ tabs: [{ id: 1 }], active: 1, revision: 'x' }).revision).toBe(0)
  })
})

describe('isNewer', () => {
  it('takes a higher revision and refuses an older or equal one', () => {
    const current = state([tab(1)], 1, 5)
    expect(isNewer(state([], null, 6), current)).toBe(true)
    expect(isNewer(state([], null, 5), current)).toBe(false)
    expect(isNewer(state([], null, 4), current)).toBe(false)
  })

  it('takes any state over the empty one the store starts with', () => {
    expect(isNewer(state([tab(1)], 1, 1), EMPTY_BROWSER)).toBe(true)
  })

  it('always takes a state that carries no revision', () => {
    // The backend numbers every state from 1; zero means "not versioned".
    expect(isNewer(state([tab(1)], 1, 0), state([], null, 9))).toBe(true)
  })
})

describe('what the backend enforces', () => {
  it('has the same tab limit', () => {
    const limit = /pub const MAX_TABS: usize = (\d+);/.exec(tabsRs)?.[1]
    expect(Number(limit)).toBe(MAX_TABS)
  })

  it('sends the tab fields this side reads, and no others', () => {
    const fields = (name: string) => {
      const body = new RegExp(String.raw`pub struct ${name} \{([^}]*)\}`).exec(tabsRs)?.[1] ?? ''
      return [...body.matchAll(/pub (\w+):/g)].map((match) => match[1]).sort()
    }
    expect(fields('Tab')).toEqual(['blocked', 'id', 'title', 'url'])
    expect(fields('BrowserState')).toEqual(['active', 'revision', 'tabs'])
    const parsed = parseBrowserState({ tabs: [{ id: 1 }], active: 1, revision: 1 })
    expect(Object.keys(parsed.tabs[0]!).sort()).toEqual(fields('Tab'))
    expect(Object.keys(parsed).sort()).toEqual(fields('BrowserState'))
  })
})
