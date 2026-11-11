import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import EntityAliasesTab from './EntityAliasesTab.svelte'
import { locale } from '$lib/i18n'

const { listMock, upsertMock, deleteMock } = vi.hoisted(() => ({
  listMock: vi.fn(),
  upsertMock: vi.fn(),
  deleteMock: vi.fn(),
}))

vi.mock('$lib/db', () => ({
  getStore: () => ({
    entityAliases: { list: listMock, upsert: upsertMock, delete: deleteMock },
  }),
}))

type AliasRow = {
  id: string
  aliasKey: string
  alias: string
  canonical: string
  createdAt: number
  updatedAt: number
}

function row(
  id: string,
  alias: string,
  canonical: string,
  aliasKey = alias.toLowerCase()
): AliasRow {
  return { id, aliasKey, alias, canonical, createdAt: 1, updatedAt: 1 }
}

const ROWS = [
  row('a1', 'Mdme. Cuevas', 'Carmela Cuevas'),
  row('a2', 'Sra. Gómez', 'Ana Gómez'),
  row('a3', 'C. Cuevas', 'Carmela Cuevas'),
]

beforeEach(() => {
  locale.set('es')
  vi.clearAllMocks()
  listMock.mockResolvedValue(ROWS)
  upsertMock.mockImplementation(
    async ({ alias, canonical }: { alias: string; canonical: string }) =>
      row('a9', alias, canonical)
  )
  deleteMock.mockResolvedValue(undefined)
})

function rowOrder(): string[] {
  return screen
    .getAllByTestId(/^entity-alias-row-/)
    .map((element) => element.getAttribute('data-testid')?.replace('entity-alias-row-', '') ?? '')
}

describe('EntityAliasesTab', () => {
  it('lists every alias sorted by canonical name, then by alias', async () => {
    render(EntityAliasesTab)

    await waitFor(() => expect(listMock).toHaveBeenCalledOnce())
    // Ana Gómez first; then the two Carmela Cuevas rows, alias order.
    expect(await screen.findByTestId('entity-alias-row-a2')).toBeInTheDocument()
    expect(rowOrder()).toEqual(['a2', 'a3', 'a1'])
    expect(screen.getByText('Mdme. Cuevas')).toBeInTheDocument()
    expect(screen.getAllByText('Carmela Cuevas').length).toBeGreaterThan(0)
  })

  it('edits the canonical name through the repo and reloads the list', async () => {
    render(EntityAliasesTab)
    await screen.findByTestId('entity-alias-row-a1')

    await fireEvent.click(screen.getByTestId('entity-alias-edit-a1'))
    const input = screen.getByLabelText('Nombre canónico')
    expect(input).toHaveValue('Carmela Cuevas')
    await fireEvent.input(input, { target: { value: 'Carmen Cuevas' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar' }))

    await waitFor(() => {
      expect(upsertMock).toHaveBeenCalledWith({
        alias: 'Mdme. Cuevas',
        canonical: 'Carmen Cuevas',
      })
    })
    // Edits refresh the shared resolver, like the dialog does: the refresh
    // reloads the list once and the tab reads it again right after.
    await waitFor(() => expect(listMock).toHaveBeenCalledTimes(3))
  })

  it('shows a readable message when the edit is rejected and keeps it open', async () => {
    upsertMock.mockRejectedValue(
      new Error(
        'Cannot alias "Mdme. Cuevas" to itself: alias and canonical normalize to the same value.'
      )
    )
    render(EntityAliasesTab)
    await screen.findByTestId('entity-alias-row-a1')

    await fireEvent.click(screen.getByTestId('entity-alias-edit-a1'))
    await fireEvent.input(screen.getByLabelText('Nombre canónico'), {
      target: { value: 'Mdme. Cuevas' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar' }))

    expect(await screen.findByText(/son el mismo/)).toBeInTheDocument()
    expect(screen.queryByText(/normalize to the same value/)).toBeNull()
    // The edit stays open with the typed value; the list is not reloaded.
    expect(screen.getByLabelText('Nombre canónico')).toHaveValue('Mdme. Cuevas')
    expect(listMock).toHaveBeenCalledTimes(1)
  })

  it('asks for confirmation before deleting, then deletes and reloads', async () => {
    render(EntityAliasesTab)
    await screen.findByTestId('entity-alias-row-a1')

    await fireEvent.click(screen.getByTestId('entity-alias-delete-a1'))
    // The confirm step names the alias being removed.
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText(/Mdme\. Cuevas/)).toBeInTheDocument()
    expect(deleteMock).not.toHaveBeenCalled()

    await fireEvent.click(within(dialog).getByRole('button', { name: 'Eliminar' }))

    await waitFor(() => expect(deleteMock).toHaveBeenCalledWith('a1'))
    await waitFor(() => expect(listMock).toHaveBeenCalledTimes(3))
  })

  it('deletes nothing when the confirmation is cancelled', async () => {
    render(EntityAliasesTab)
    await screen.findByTestId('entity-alias-row-a1')

    await fireEvent.click(screen.getByTestId('entity-alias-delete-a1'))
    const dialog = await screen.findByRole('dialog')
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Cancelar' }))

    expect(deleteMock).not.toHaveBeenCalled()
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  })

  it('explains aliases in one sentence when the list is empty', async () => {
    listMock.mockResolvedValue([])
    render(EntityAliasesTab)

    expect(await screen.findByText(/Todavía no hay alias/)).toBeInTheDocument()
    expect(screen.queryByTestId(/^entity-alias-row-/)).toBeNull()
  })
})
