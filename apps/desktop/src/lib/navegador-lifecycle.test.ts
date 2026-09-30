import { describe, expect, it, vi } from 'vitest'
import {
  installNavegadorLifecycle,
  shouldReleaseBrowser,
  watchNavegadorTabs,
} from './navegador-lifecycle'
import { WorkspaceStore } from './workspace'

describe('shouldReleaseBrowser', () => {
  it('releases the browser when the last Navegador tab closes', () => {
    expect(shouldReleaseBrowser('navegador', ['home', 'writing'])).toBe(true)
    expect(shouldReleaseBrowser('navegador', [])).toBe(true)
  })

  it('keeps it while another tab still shows the Navegador', () => {
    expect(shouldReleaseBrowser('navegador', ['home', 'navegador'])).toBe(false)
  })

  it('leaves it alone when the closed tab was not the Navegador', () => {
    expect(shouldReleaseBrowser('home', ['navegador'])).toBe(false)
    expect(shouldReleaseBrowser('writing', ['home'])).toBe(false)
  })
})

describe('watchNavegadorTabs', () => {
  it('releases the browser when the Navegador tab is closed', () => {
    const workspace = new WorkspaceStore()
    const release = vi.fn()
    watchNavegadorTabs(workspace, release)
    const id = workspace.openTab({ name: 'navegador' })!
    workspace.closeTab(id)
    expect(release).toHaveBeenCalledTimes(1)
  })

  it('does nothing for other tabs, or while another Navegador tab remains', () => {
    const workspace = new WorkspaceStore()
    const release = vi.fn()
    watchNavegadorTabs(workspace, release)
    const settings = workspace.openTab({ name: 'settings' })!
    workspace.closeTab(settings)
    const first = workspace.openTab({ name: 'navegador' })!
    const second = workspace.openTab({ name: 'navegador' })!
    workspace.closeTab(first)
    expect(release).not.toHaveBeenCalled()
    workspace.closeTab(second)
    expect(release).toHaveBeenCalledTimes(1)
  })

  it('does nothing when a tab that left the Navegador behind is closed', () => {
    const workspace = new WorkspaceStore()
    const release = vi.fn()
    watchNavegadorTabs(workspace, release)
    const id = workspace.openTab({ name: 'navegador' })!
    workspace.navigationFor(id).navigate({ name: 'settings' })
    workspace.closeTab(id)
    expect(release).not.toHaveBeenCalled()
  })

  it('stops watching when told to', () => {
    const workspace = new WorkspaceStore()
    const release = vi.fn()
    const stop = watchNavegadorTabs(workspace, release)
    stop()
    workspace.closeTab(workspace.openTab({ name: 'navegador' })!)
    expect(release).not.toHaveBeenCalled()
  })

  it('does not let a failing release break closing the tab', async () => {
    const workspace = new WorkspaceStore()
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    watchNavegadorTabs(workspace, () => Promise.reject(new Error('boom')))
    const id = workspace.openTab({ name: 'navegador' })!
    expect(() => workspace.closeTab(id)).not.toThrow()
    await Promise.resolve()
    await Promise.resolve()
    expect(warn).toHaveBeenCalled()
    warn.mockRestore()
  })
})

describe('installNavegadorLifecycle', () => {
  it('closes the browser and forgets the panel when the Navegador tab closes', async () => {
    const workspace = new WorkspaceStore()
    const calls: string[] = []
    installNavegadorLifecycle(workspace, {
      session: { close: async () => void calls.push('close') },
      store: { clearAll: () => void calls.push('clear') },
    })
    workspace.closeTab(workspace.openTab({ name: 'navegador' })!)
    await Promise.resolve()
    expect(calls).toEqual(['clear', 'close'])
  })

  it('leaves the browser and the panel alone for any other tab', async () => {
    const workspace = new WorkspaceStore()
    const calls: string[] = []
    installNavegadorLifecycle(workspace, {
      session: { close: async () => void calls.push('close') },
      store: { clearAll: () => void calls.push('clear') },
    })
    workspace.closeTab(workspace.openTab({ name: 'settings' })!)
    await Promise.resolve()
    expect(calls).toEqual([])
  })
})
