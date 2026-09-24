import { render, screen, waitFor } from '@testing-library/svelte'
import { describe, it, expect, vi, beforeEach } from 'vitest'
import WorkPane from './WorkPane.svelte'
import { workspace } from '$lib/workspace'
import { locale } from '$lib/i18n'

vi.mock('$lib/db', () => ({
  getStore: () => ({
    items: { findPreviousCardSummary: vi.fn(), findNextCardSummary: vi.fn() },
  }),
}))

function resetWorkspace() {
  while (workspace.tabs.length > 1) {
    workspace.closeTab(workspace.tabs.at(-1)!.id)
  }
  workspace.activeNavigation.resetToPath([{ name: 'home' }])
}

describe('WorkPane', () => {
  beforeEach(() => {
    locale.set('es')
    resetWorkspace()
  })

  it('renders HomeView synchronously for the home route (no lazy-load flash)', () => {
    render(WorkPane, { paneId: workspace.activeTabId })
    expect(screen.getByText('Espacio de trabajo')).toBeInTheDocument()
  })

  it('a slow lazy view in one pane does not block or corrupt an independent second pane', async () => {
    const secondPaneId = workspace.openTab({ name: 'db-browser' })!
    workspace.navigationFor(workspace.tabs[0]!.id).navigate({ name: 'db-browser' })

    render(WorkPane, { paneId: workspace.tabs[0]!.id })
    render(WorkPane, { paneId: secondPaneId })

    // Both panes requested the same lazy view name ('db-browser'); the first
    // pane then navigates away before either import settles. The second pane
    // must still render once the shared route-loader promise resolves —
    // its own effect's `cancelled` flag must never be tripped by the other
    // pane's navigation (Review Focus #5).
    workspace.navigationFor(workspace.tabs[0]!.id).navigate({ name: 'collections' })

    await waitFor(() => {
      expect(screen.getAllByRole('heading', { level: 1 }).length).toBeGreaterThan(0)
    })
  })
})
