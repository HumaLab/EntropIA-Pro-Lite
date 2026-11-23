import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import SearchIndexSettingsCard from './SearchIndexSettingsCard.svelte'

afterEach(cleanup)
beforeEach(() => {
  locale.set('es')
  invoke.mockReset()
})

const LABEL = 'Unir palabras cortadas con guion al final de línea'

it('shows the stored setting', async () => {
  invoke.mockResolvedValueOnce(false)
  render(SearchIndexSettingsCard)

  expect(invoke).toHaveBeenCalledWith('fts_get_rejoin_hyphens')
  const box = await screen.findByRole('checkbox', { name: new RegExp(LABEL) })
  await vi.waitFor(() => expect(box).not.toBeChecked())
})

it('is on when the archive has no setting yet', async () => {
  invoke.mockResolvedValueOnce(true)
  render(SearchIndexSettingsCard)

  const box = await screen.findByRole('checkbox', { name: new RegExp(LABEL) })
  await vi.waitFor(() => expect(box).toBeChecked())
})

it('saves the change and says the index is being rebuilt', async () => {
  invoke.mockResolvedValueOnce(true).mockResolvedValueOnce(undefined)
  render(SearchIndexSettingsCard)
  const box = await screen.findByRole('checkbox', { name: new RegExp(LABEL) })
  await vi.waitFor(() => expect(box).toBeChecked())

  await fireEvent.click(box)

  expect(invoke).toHaveBeenLastCalledWith('fts_set_rejoin_hyphens', { enabled: false })
  expect(
    await screen.findByText('El índice de búsqueda se actualiza en segundo plano.')
  ).toBeInTheDocument()
})

it('puts the box back when the save fails', async () => {
  invoke.mockResolvedValueOnce(true).mockRejectedValueOnce('locked')
  render(SearchIndexSettingsCard)
  const box = await screen.findByRole('checkbox', { name: new RegExp(LABEL) })
  await vi.waitFor(() => expect(box).toBeChecked())

  await fireEvent.click(box)

  expect(await screen.findByRole('alert')).toHaveTextContent('No se pudo guardar: locked')
  expect(box).toBeChecked()
})
