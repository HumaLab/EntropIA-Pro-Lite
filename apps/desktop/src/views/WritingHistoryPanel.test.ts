import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import WritingHistoryPanel from './WritingHistoryPanel.svelte'
import { locale } from '$lib/i18n'

const heading = (level: number, text: string) => ({
  type: 'heading',
  attrs: { level },
  content: [{ type: 'text', text }],
})
const paragraph = (text: string) => ({ type: 'paragraph', content: [{ type: 'text', text }] })
const manuscript = (intro: string, body: string) => ({
  schemaVersion: 1,
  doc: {
    type: 'doc',
    content: [
      heading(1, 'Introducción'),
      paragraph(intro),
      heading(1, 'Desarrollo'),
      paragraph(body),
    ],
  },
})

const { writingMock } = vi.hoisted(() => ({
  writingMock: {
    content: null as unknown,
    versions: new Map<number, unknown>(),
    listVersions: vi.fn(),
    readVersion: vi.fn(),
    restoreVersion: vi.fn(),
    restoreSection: vi.fn(),
    subscribe(run: (state: { content: unknown }) => void) {
      run({ content: writingMock.content })
      return () => {}
    },
  },
}))

vi.mock('$lib/writing', () => ({ writing: writingMock }))

describe('WritingHistoryPanel per-section history', () => {
  beforeEach(() => {
    locale.set('es')
    // v1 → v2 changes only "Desarrollo"; v2 → v3 changes only "Introducción".
    writingMock.versions = new Map([
      [1, manuscript('Tema.', 'Cuerpo.')],
      [2, manuscript('Tema.', 'Cuerpo nuevo.')],
      [3, manuscript('Tema nuevo.', 'Cuerpo nuevo.')],
    ])
    writingMock.content = manuscript('Tema de hoy.', 'Cuerpo de hoy.')
    writingMock.listVersions.mockResolvedValue(
      [3, 2, 1].map((n) => ({ version_number: n, created_at: n * 1000, reason: 'auto' }))
    )
    writingMock.readVersion.mockImplementation(async (n: number) => writingMock.versions.get(n))
    writingMock.restoreSection.mockResolvedValue(true)
  })

  async function chooseSection(name: string) {
    await fireEvent.click(await screen.findByRole('button', { name: /Apartado/ }))
    await fireEvent.click(await screen.findByRole('menuitemradio', { name }))
  }

  it('lists only the versions where the chosen section changed', async () => {
    render(WritingHistoryPanel, { props: { onclose: () => {} } })
    const list = await screen.findByRole('list')
    await waitFor(() => expect(within(list).getAllByRole('button')).toHaveLength(3))

    await chooseSection('Introducción')
    await waitFor(() => expect(within(list).getAllByRole('button')).toHaveLength(2))

    await chooseSection('Desarrollo')
    await waitFor(() => expect(within(list).getAllByRole('button')).toHaveLength(2))

    await chooseSection('Todos los apartados')
    await waitFor(() => expect(within(list).getAllByRole('button')).toHaveLength(3))
  })

  it('restores only the chosen section from the picked version', async () => {
    render(WritingHistoryPanel, { props: { onclose: () => {} } })
    await chooseSection('Desarrollo')
    const list = await screen.findByRole('list')
    await waitFor(() => expect(within(list).getAllByRole('button')).toHaveLength(2))
    await fireEvent.click(within(list).getAllByRole('button')[1]!)

    const restore = await screen.findByRole('button', { name: 'Restaurar solo este apartado' })
    expect(screen.getByText(/Solo «Desarrollo» vuelve a esta versión/)).toBeInTheDocument()
    await fireEvent.click(restore)
    await waitFor(() =>
      expect(writingMock.restoreSection).toHaveBeenCalledWith(
        1,
        expect.objectContaining({ title: 'Desarrollo', level: 1, ordinal: 1 })
      )
    )
    expect(writingMock.restoreVersion).not.toHaveBeenCalled()
  })

  it('offers no single-section restore while every section is shown', async () => {
    render(WritingHistoryPanel, { props: { onclose: () => {} } })
    const list = await screen.findByRole('list')
    await waitFor(() => expect(within(list).getAllByRole('button')).toHaveLength(3))
    await fireEvent.click(within(list).getAllByRole('button')[0]!)
    await screen.findByRole('button', { name: 'Restaurar esta versión' })
    expect(screen.queryByRole('button', { name: 'Restaurar solo este apartado' })).toBeNull()
  })
})
