import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import BackupTab from './BackupTab.svelte'
import { locale } from '$lib/i18n'

/**
 * Copias de seguridad: the automatic-backup frequency is a choice the tab
 * reads from and writes to the settings store.
 */

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

const stored: Record<string, string | null> = {}

describe('BackupTab', () => {
  beforeEach(() => {
    locale.set('es')
    for (const key of Object.keys(stored)) delete stored[key]
    invokeMock.mockReset().mockImplementation((command: string, args?: { key?: string }) => {
      if (command === 'settings_get') return Promise.resolve(stored[args?.key ?? ''] ?? null)
      return Promise.resolve(undefined)
    })
  })

  it('shows the stored frequency on a SelectTrigger named by its label', async () => {
    stored.backup_frequency = 'weekly'
    render(BackupTab)

    const trigger = await screen.findByRole('button', {
      name: 'Copia automática Una vez por semana',
    })
    expect(trigger).toHaveClass('select-trigger', 'select-trigger--quiet', 'select-trigger--sm')
  })

  it('persists the chosen frequency', async () => {
    render(BackupTab)
    const trigger = await screen.findByRole('button', { name: 'Copia automática Nunca' })

    await fireEvent.click(trigger)
    await fireEvent.click(screen.getByRole('menuitemradio', { name: 'Todos los días' }))

    expect(invokeMock).toHaveBeenCalledWith('settings_set', {
      key: 'backup_frequency',
      value: 'daily',
    })
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: 'Copia automática Todos los días' })
      ).toBeInTheDocument()
    )
  })

  // Audit item 17: the backup keeps running after the user leaves the tab.
  it('keeps the backup in progress after leaving and coming back, until it ends', async () => {
    stored['backend_grant.backup_dir'] = 'D:/copias'
    let finishBackup: (path: string) => void = () => {}
    invokeMock.mockImplementation((command: string, args?: { key?: string }) => {
      if (command === 'settings_get') return Promise.resolve(stored[args?.key ?? ''] ?? null)
      if (command === 'backup_run_now') {
        return new Promise<string>((resolve) => {
          finishBackup = resolve
        })
      }
      return Promise.resolve(undefined)
    })

    const first = render(BackupTab)
    const runNow = await screen.findByRole('button', { name: 'Hacer una copia ahora' })
    await waitFor(() => expect(runNow).toBeEnabled())
    await fireEvent.click(runNow)
    first.unmount()

    render(BackupTab)
    const again = await screen.findByRole('button', { name: 'Hacer una copia ahora' })
    await waitFor(() => expect(again).toBeDisabled())

    stored['backend_grant.backup_last_at'] = '1700000000000'
    finishBackup('D:/copias/entropia.zip')
    await waitFor(() => expect(again).toBeEnabled())
    expect(invokeMock.mock.calls.filter(([command]) => command === 'backup_run_now')).toHaveLength(
      1
    )
  })
})
