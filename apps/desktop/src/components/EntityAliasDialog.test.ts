import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import EntityAliasDialog from './EntityAliasDialog.svelte'
import { getEntityAliasResolver, invalidateEntityAliases } from '$lib/entity-aliases'
import { locale } from '$lib/i18n'

const { upsertMock, listMock } = vi.hoisted(() => ({
  upsertMock: vi.fn(),
  listMock: vi.fn(),
}))

vi.mock('$lib/db', () => ({
  getStore: () => ({
    entityAliases: { upsert: upsertMock, list: listMock },
  }),
}))

function openDialog(overrides: Record<string, unknown> = {}) {
  return render(EntityAliasDialog, {
    onclose: vi.fn(),
    ...overrides,
    alias: 'Mdme. Cuevas',
    suggestions: ['Carmela Cuevas', 'Cuevas'],
  })
}

beforeEach(() => {
  locale.set('es')
  vi.clearAllMocks()
  invalidateEntityAliases()
  listMock.mockResolvedValue([])
  upsertMock.mockResolvedValue({
    id: 'a1',
    aliasKey: 'mdme. cuevas',
    alias: 'Mdme. Cuevas',
    canonical: 'Carmela Cuevas',
    createdAt: 1,
    updatedAt: 1,
  })
})

describe('EntityAliasDialog', () => {
  it('prefills the mention as the alias and asks for the canonical name', () => {
    openDialog()

    expect(screen.getByDisplayValue('Mdme. Cuevas')).toBeInTheDocument()
    const canonical = screen.getByLabelText('Nombre canónico')
    expect((canonical as HTMLInputElement).value).toBe('')
  })

  it('fills the canonical name when a suggestion is picked', async () => {
    openDialog()

    await fireEvent.click(screen.getByRole('button', { name: 'Carmela Cuevas' }))

    expect(screen.getByLabelText('Nombre canónico')).toHaveValue('Carmela Cuevas')
  })

  it('saves the alias and refreshes the shared resolver without restarting', async () => {
    const onclose = vi.fn()
    const onsaved = vi.fn()
    listMock.mockResolvedValue([
      {
        id: 'a1',
        aliasKey: 'mdme. cuevas',
        alias: 'Mdme. Cuevas',
        canonical: 'Carmela Cuevas',
        createdAt: 1,
        updatedAt: 1,
      },
    ])
    openDialog({ onclose, onsaved })

    await fireEvent.input(screen.getByLabelText('Nombre canónico'), {
      target: { value: 'Carmela Cuevas' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar' }))

    await waitFor(() => {
      expect(upsertMock).toHaveBeenCalledWith({
        alias: 'Mdme. Cuevas',
        canonical: 'Carmela Cuevas',
      })
    })
    // The shared cache now resolves the alias: entity list and Relaciones 3D
    // read it without restarting.
    await waitFor(() => {
      expect(getEntityAliasResolver()('Mdme. Cuevas')).toBe('Carmela Cuevas')
    })
    expect(onsaved).toHaveBeenCalledOnce()
    expect(onclose).toHaveBeenCalledOnce()
  })

  it('shows a readable message on a repo validation error and saves nothing', async () => {
    const onclose = vi.fn()
    upsertMock.mockRejectedValue(
      new Error(
        'Cannot point "Mdme. Cuevas" at "Cuevas": "Cuevas" is itself an alias — resolution is single-hop, so the owner picks the canonical directly.'
      )
    )
    openDialog({ onclose })

    await fireEvent.input(screen.getByLabelText('Nombre canónico'), {
      target: { value: 'Cuevas' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar' }))

    // Plain Spanish, never the raw exception.
    expect(await screen.findByText(/ya es un alias de otro nombre/)).toBeInTheDocument()
    expect(screen.queryByText(/Cannot point/)).toBeNull()
    // Nothing was saved and the dialog stays open with the typed value.
    expect(listMock).not.toHaveBeenCalled()
    expect(onclose).not.toHaveBeenCalled()
    expect(screen.getByLabelText('Nombre canónico')).toHaveValue('Cuevas')
  })
})
