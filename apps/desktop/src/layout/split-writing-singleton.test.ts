import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import WorkPane from './WorkPane.svelte'
import { workspace } from '$lib/workspace'
import { locale } from '$lib/i18n'
import WritingStub from './__fixtures__/WritingStub.svelte'

vi.mock('$lib/db', () => ({
  getStore: () => ({ items: { findPreviousCardSummary: vi.fn(), findNextCardSummary: vi.fn() } }),
}))

// A spy, not a bare function, so a test can assert a pane showing the
// writing-elsewhere notice never calls `loadRouteView('writing')` at all
// (controller fix round 2, item b).
const { loadRouteViewMock } = vi.hoisted(() => ({ loadRouteViewMock: vi.fn() }))

vi.mock('$lib/route-loader', () => ({
  loadRouteView: loadRouteViewMock,
}))

function resetWorkspace() {
  while (workspace.tabs.length > 1) workspace.closeTab(workspace.tabs.at(-1)!.id)
  workspace.activeNavigation.resetToPath([{ name: 'home' }])
}

function stubWritingRoute() {
  loadRouteViewMock
    .mockReset()
    .mockImplementation((name: string) =>
      name === 'writing'
        ? Promise.resolve({ default: WritingStub })
        : Promise.reject(new Error(`unstubbed route in this test: ${name}`))
    )
}

describe('Writing stays single-tab with two panes mounted (Review Focus #3)', () => {
  beforeEach(() => {
    locale.set('es')
    resetWorkspace()
    stubWritingRoute()
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
//
// Controller fix round 2: the original version of this test (tab B =
// workspace.activeTabId, i.e. the pre-existing tab FIRST in tab-list order;
// tab A = workspace.openTab(), SECOND) did not discriminate the bug it
// claimed to cover — "first in tab-list order" (the old, wrong logic) also
// happens to pick B here, since B already sits first in the list, so the
// test passed even against the unfixed code and never caught the reachable
// failure the controller found in review. It also navigated tab A before
// tab A was ever mounted, never exercising a pane reacting to a nav change
// WHILE it's on screen. Rewritten below to actually discriminate: tab A is
// first in tab-list order (it exists before tab B is opened) and already
// has writing in its OWN history before B ever shows it, so "first in
// tab-list order" would (wrongly) pick A — the RED evidence for this
// rewrite (run against the pre-fix-round-1 list-order code, commit
// 56efebf8) is recorded in the task report.
describe('Writing single-tab ownership survives a later arrival via history (deferred edge)', () => {
  beforeEach(() => {
    locale.set('es')
    resetWorkspace()
    stubWritingRoute()
  })

  it('keeps the incumbent WritingView mounted when a MOUNTED pane reaches writing later via its own history (Back)', async () => {
    const tabA = workspace.activeTabId
    const tabB = workspace.openTab()!

    // Tab A "opened Writing, then left" — its own history holds `writing`
    // below `home` (current), set directly so the shape is exact rather
    // than reconstructed through navigate()/back() round-trips. Tab A is
    // FIRST in tab-list order (it existed before tab B was opened).
    workspace
      .navigationFor(tabA)
      .resetToPath([
        { name: 'writing', documentId: 'doc-1', documentTitle: 'Doc 1' },
        { name: 'home' },
      ])

    // Tab B (opened AFTER A, so SECOND in tab-list order) opens Writing —
    // nobody owns it yet (tab A currently shows `home`), so B becomes the
    // incumbent.
    workspace.navigationFor(tabB).navigate({
      name: 'writing',
      documentId: 'doc-1',
      documentTitle: 'Doc 1',
    })
    render(WorkPane, { paneId: tabB })
    await waitFor(() => expect(screen.getAllByTestId('writing-stub')).toHaveLength(1))
    expect(loadRouteViewMock).toHaveBeenCalledWith('writing')

    // Mount tab A too — both panes visible, as split view would show them.
    // It currently shows `home`, not the notice: its own history hasn't
    // reached `writing` again yet.
    render(WorkPane, { paneId: tabA })
    expect(screen.getAllByTestId('writing-stub')).toHaveLength(1)

    // Isolate the calls that happen from here on, so the assertion below
    // proves the *notice* pane specifically never loads the writing route,
    // not merely that the count "stays the same" for some unrelated reason.
    loadRouteViewMock.mockClear()

    // Tab A goes Back WHILE MOUNTED: its own history pops back onto
    // `writing`, bypassing workspace.navigateActive()'s redirect entirely —
    // the same hazard the workspace-level `writingOwnerId` tests cover, now
    // exercised against an already-mounted WorkPane reacting live. The
    // incumbent (tab B) must keep its live WritingView; tab A must react by
    // showing the notice instead of mounting a second one.
    workspace.navigationFor(tabA).back()

    await waitFor(() =>
      expect(screen.getByText('Escritura está abierta en otra pestaña.')).toBeInTheDocument()
    )
    // Still exactly one WritingView — tab B's — never a second one for A.
    expect(screen.getAllByTestId('writing-stub')).toHaveLength(1)
    // The notice pane never lazily loads the writing route module at all.
    expect(loadRouteViewMock).not.toHaveBeenCalledWith('writing')

    await fireEvent.click(screen.getByRole('button', { name: 'Ir a esa pestaña' }))

    expect(workspace.activeTabId).toBe(tabB)
    // Still exactly one — activating the owner never mounts a second one.
    expect(screen.getAllByTestId('writing-stub')).toHaveLength(1)
  })
})
