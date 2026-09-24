import { render, screen } from '@testing-library/svelte'
import { describe, it, expect } from 'vitest'
import ProviderFixture from './pane-context-fixtures/ProviderFixture.svelte'
import { NavigationStore } from './navigation'
import { getNavigation, getPaneId } from './pane-context'
import { workspace } from './workspace'

describe('pane-context', () => {
  it("a component under a provider reads that provider's navigation and pane id", () => {
    const nav = new NavigationStore()
    nav.navigate({ name: 'collections' })

    render(ProviderFixture, { navigation: nav, paneId: 'pane-a' })

    expect(screen.getByTestId('probe-current').textContent).toBe('collections')
    expect(screen.getByTestId('probe-pane-id').textContent).toBe('pane-a')
  })

  it('falls back to the workspace active tab outside any provider', () => {
    expect(getNavigation()).toBe(workspace.activeNavigation)
    expect(getPaneId()).toBe(workspace.activeTabId)
  })
})
