import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, it, expect, afterEach, beforeEach } from 'vitest'
import { flushSync } from 'svelte'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import TabStrip from './TabStrip.svelte'
import { workspace, MAX_TABS } from '$lib/workspace'
import { locale } from '$lib/i18n'

function resetWorkspace() {
  while (workspace.tabs.length > 1) {
    workspace.closeTab(workspace.tabs.at(-1)!.id)
  }
  workspace.activeNavigation.resetToPath([{ name: 'home' }])
}

describe('TabStrip', () => {
  beforeEach(() => {
    locale.set('es')
    resetWorkspace()
  })
  afterEach(() => resetWorkspace())

  it('renders one tab with no close control when it is the only tab', () => {
    render(TabStrip)
    expect(screen.getAllByRole('tab')).toHaveLength(1)
    expect(screen.queryByRole('button', { name: /^Cerrar/ })).not.toBeInTheDocument()
  })

  it("gives the tab button an explicit aria-label equal to the view's title — the visible label alone is not enough, since it hides below the 900px breakpoint", () => {
    render(TabStrip)
    expect(screen.getByRole('tab')).toHaveAttribute('aria-label', 'Inicio')
  })

  it('the + button opens a new tab and activates it', async () => {
    render(TabStrip)
    await fireEvent.click(screen.getByRole('button', { name: 'Abrir nueva pestaña' }))
    expect(workspace.tabs).toHaveLength(2)
    expect(workspace.activeTabId).toBe(workspace.tabs[1]!.id)
  })

  it('the + button disables at the four-tab cap', () => {
    render(TabStrip)
    // Mutating the store directly, rather than through a rendered control
    // Svelte's own event dispatch already flushes: the component owns no
    // synchronous flush of its own, so the caller does, exactly like any
    // other external-store integration.
    flushSync(() => {
      workspace.openTab()
      workspace.openTab()
      workspace.openTab()
    })
    expect(workspace.tabs).toHaveLength(MAX_TABS)
    expect(screen.getByRole('button', { name: 'Abrir nueva pestaña' })).toBeDisabled()
  })

  it('clicking a tab activates it, and its close control removes it', async () => {
    render(TabStrip)
    const secondId = flushSync(() => workspace.openTab())!

    const tabs = screen.getAllByRole('tab')
    expect(tabs).toHaveLength(2)

    await fireEvent.click(tabs[0]!)
    expect(workspace.activeTabId).not.toBe(secondId)

    const closeButtons = screen.getAllByRole('button', { name: /^Cerrar/ })
    await fireEvent.click(closeButtons[0]!)
    expect(workspace.tabs).toHaveLength(1)
  })

  // Visual round 3, item 3: the grouped-tab underline used the full accent
  // color at 2px — too bright/thick against these dark surfaces, the same
  // glow problem the active-pane ring had before eb4567d0 muted it to
  // --color-border-strong. The underline must stay recognisable but move to
  // the same low-contrast family, thinner, with no second shadow layer (no
  // glow) — the active tab itself is still marked by its own background
  // wash, untouched here.
  describe('grouped-tab underline is delicate', () => {
    const source = readFileSync(resolve(import.meta.dirname, 'TabStrip.svelte'), 'utf-8')
    const styles = source.slice(source.indexOf('<style>'))

    function ruleFor(selector: string): string {
      const at = styles.indexOf(selector)
      expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
      const rule = styles.slice(at)
      return rule.slice(0, rule.indexOf('}'))
    }

    it('never draws the underline in the bright accent color', () => {
      expect(ruleFor('.tab-strip__tab--grouped {')).not.toMatch(/--color-accent\b/)
    })

    it('uses a low-contrast existing border token instead, from the same muted family as the active-pane border', () => {
      expect(ruleFor('.tab-strip__tab--grouped {')).toMatch(
        /box-shadow:\s*inset 0 -1px 0 var\(--(border-subtle|color-border-strong|color-border)\);?\s*$/
      )
    })

    it('has no glow: a single 0-blur, 0-spread 1px inset underline, no second shadow layer', () => {
      const rule = ruleFor('.tab-strip__tab--grouped {')
      const shadow = /box-shadow:\s*([^;]+);/.exec(rule)?.[1] ?? ''
      expect(shadow.split(',').length).toBe(1)
      expect(shadow).toMatch(/^inset 0 -1px 0 /)
    })

    it('adds no corner/glow pseudo-element tied to the grouped tab', () => {
      expect(styles).not.toMatch(/\.tab-strip__tab--grouped::(before|after)/)
    })

    it('still keeps the active tab recognisable via its own background wash, unchanged', () => {
      expect(ruleFor('.tab-strip__tab--active {')).toMatch(/background:\s*var\(--color-accent-faint\);/)
    })
  })
})
