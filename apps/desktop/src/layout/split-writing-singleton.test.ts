import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
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

// Controller fix round 1 (Task 3.3 review): the reachable failure this
// covers is a pane reaching `writing` through its OWN NavigationStore
// history (what Back/forward would call) — bypassing navigateActive()'s
// redirect above entirely, unlike the scenario in the describe above.
describe('Writing single-tab ownership survives a later arrival via history (deferred edge)', () => {
  beforeEach(() => {
    locale.set('es')
    resetWorkspace()
  })

  it('keeps the incumbent WritingView mounted; the later arrival shows the notice, never a second WritingView', async () => {
    const tabB = workspace.activeTabId
    const tabA = workspace.openTab()!

    // Tab B reaches `writing` FIRST and becomes the incumbent (owner). Tab A
    // reaches it SECOND, directly on its own NavigationStore — simulating
    // Back into old history — even though tab A was created (and so sits
    // first in tab-list order) before tab B was opened.
    workspace.navigationFor(tabB).navigate({
      name: 'writing',
      documentId: 'doc-1',
      documentTitle: 'Doc 1',
    })
    render(WorkPane, { paneId: tabB })
    await waitFor(() => expect(screen.getAllByTestId('writing-stub')).toHaveLength(1))

    workspace.navigationFor(tabA).navigate({
      name: 'writing',
      documentId: 'doc-1',
      documentTitle: 'Doc 1',
    })
    render(WorkPane, { paneId: tabA })

    // Tab B's WritingView is still the only one mounted — never unmounted
    // and never duplicated — while tab A shows the notice instead.
    await waitFor(() => expect(screen.getAllByTestId('writing-stub')).toHaveLength(1))
    expect(screen.getByText('Escritura está abierta en otra pestaña.')).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Ir a esa pestaña' }))

    expect(workspace.activeTabId).toBe(tabB)
    // Still exactly one — activating the owner never mounts a second one.
    expect(screen.getAllByTestId('writing-stub')).toHaveLength(1)
  })
})
