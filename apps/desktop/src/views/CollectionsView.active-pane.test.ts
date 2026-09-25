import { fireEvent, render, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import CollectionsView from './CollectionsView.svelte'
import { locale } from '$lib/i18n'

/**
 * The sidebar's "new collection" and filter are window-level events, and in
 * split view two CollectionsView instances can be mounted at once. The spec
 * hands such events to the active pane's view only.
 */
const { paneRef, workspaceRef } = vi.hoisted(() => ({
  paneRef: { current: 'pane-a' },
  workspaceRef: { activeTabId: 'pane-b', forgetCollection: () => {} },
}))

vi.mock('$lib/db', () => ({
  getStore: () => ({
    collections: {
      findAll: vi
        .fn()
        .mockResolvedValue([
          { id: 'col-1', name: 'Historia', description: null, createdAt: 1, updatedAt: 1 },
        ]),
      findAllNonEmpty: vi.fn().mockResolvedValue([]),
      countItems: vi.fn().mockResolvedValue(0),
      create: vi.fn(),
      update: vi.fn(),
      delete: vi.fn(),
    },
  }),
}))

vi.mock('$lib/pane-context', () => ({
  getNavigation: () => ({ navigate: vi.fn() }),
  getPaneId: () => paneRef.current,
}))

vi.mock('$lib/workspace', () => ({ workspace: workspaceRef }))

function renderPane(paneId: string) {
  paneRef.current = paneId
  return render(CollectionsView).container
}

describe('CollectionsView in split view: window events reach the active pane only', () => {
  beforeEach(() => {
    locale.set('es')
    workspaceRef.activeTabId = 'pane-b'
  })

  it('opens the create form in the active pane only', async () => {
    const left = renderPane('pane-a')
    const right = renderPane('pane-b')

    window.dispatchEvent(new CustomEvent('entropia:create-collection'))

    await waitFor(() => expect(right.querySelector('.create-form')).not.toBeNull())
    expect(left.querySelector('.create-form')).toBeNull()
  })

  it("focuses the active pane's form even when the other pane has one open", async () => {
    const left = renderPane('pane-a')
    const right = renderPane('pane-b')
    await fireEvent.click(await within(left).findByRole('button', { name: 'Nueva colección' }))
    expect(left.querySelector('.create-form')).not.toBeNull()

    window.dispatchEvent(new CustomEvent('entropia:create-collection'))

    await waitFor(() =>
      expect(right.querySelector('.create-form input')).toBe(document.activeElement)
    )
  })

  it('filters the active pane only', async () => {
    const left = renderPane('pane-a')
    const right = renderPane('pane-b')
    await within(left).findAllByText('Historia')
    await within(right).findAllByText('Historia')

    window.dispatchEvent(new CustomEvent('entropia:filter-collections', { detail: 'zzz' }))

    await waitFor(() => expect(within(right).queryAllByText('Historia')).toHaveLength(0))
    expect(within(left).queryAllByText('Historia').length).toBeGreaterThan(0)
  })
})
