import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, it, expect, afterEach, beforeEach } from 'vitest'
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

  it('the + button opens a new tab and activates it', async () => {
    render(TabStrip)
    await fireEvent.click(screen.getByRole('button', { name: 'Abrir nueva pestaña' }))
    expect(workspace.tabs).toHaveLength(2)
    expect(workspace.activeTabId).toBe(workspace.tabs[1]!.id)
  })

  it('the + button disables at the four-tab cap', () => {
    render(TabStrip)
    workspace.openTab()
    workspace.openTab()
    workspace.openTab()
    expect(workspace.tabs).toHaveLength(MAX_TABS)
    expect(screen.getByRole('button', { name: 'Abrir nueva pestaña' })).toBeDisabled()
  })

  it('clicking a tab activates it, and its close control removes it', async () => {
    render(TabStrip)
    const secondId = workspace.openTab()!

    const tabs = screen.getAllByRole('tab')
    expect(tabs).toHaveLength(2)

    await fireEvent.click(tabs[0]!)
    expect(workspace.activeTabId).not.toBe(secondId)

    const closeButtons = screen.getAllByRole('button', { name: /^Cerrar/ })
    await fireEvent.click(closeButtons[0]!)
    expect(workspace.tabs).toHaveLength(1)
  })
})
