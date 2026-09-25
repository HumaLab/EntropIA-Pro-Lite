import { render, screen, waitFor } from '@testing-library/svelte'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import WorkPane from './WorkPane.svelte'
import { workspace } from '$lib/workspace'
import { locale } from '$lib/i18n'

vi.mock('$lib/db', () => ({
  getStore: () => ({ items: { findPreviousCardSummary: vi.fn(), findNextCardSummary: vi.fn() } }),
}))

vi.mock('$lib/route-loader', async () => {
  const { default: WritingStub } = await import('./__fixtures__/WritingStub.svelte')
  return {
    loadRouteView: (name: string) =>
      name === 'writing'
        ? Promise.resolve({ default: WritingStub })
        : Promise.reject(new Error(`unstubbed route in this test: ${name}`)),
  }
})

function resetWorkspace() {
  while (workspace.tabs.length > 1) workspace.closeTab(workspace.tabs.at(-1)!.id)
  workspace.activeNavigation.resetToPath([{ name: 'home' }])
}

describe('Writing stays single-tab with two panes mounted (Review Focus #3)', () => {
  beforeEach(() => {
    locale.set('es')
    resetWorkspace()
  })

  it('requesting Writing from a second, already-mounted pane activates the first pane instead of mounting a second WritingView', async () => {
    workspace.navigateActive({ name: 'writing' })
    const writingTabId = workspace.activeTabId
    const secondTabId = workspace.openTab()!

    render(WorkPane, { paneId: writingTabId })
    render(WorkPane, { paneId: secondTabId })

    await waitFor(() => expect(screen.getAllByTestId('writing-stub')).toHaveLength(1))

    // The second pane's own section-icon click reaches here as
    // workspace.navigateActive({ name: 'writing' }) — exercised directly,
    // since TopBar's button is chrome, not pane-scoped.
    workspace.navigateActive({ name: 'writing' })

    expect(workspace.activeTabId).toBe(writingTabId)
    expect(workspace.navigationFor(secondTabId).current).toEqual({ name: 'home' })
    await waitFor(() => expect(screen.getAllByTestId('writing-stub')).toHaveLength(1))
  })
})
