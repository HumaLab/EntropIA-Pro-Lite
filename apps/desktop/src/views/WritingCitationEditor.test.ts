import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import WritingCitationEditor from './WritingCitationEditor.svelte'
import { renderCluster } from '$lib/writing-csl'

vi.mock('$lib/writing-csl', () => ({
  DEFAULT_STYLE: { kind: 'bundled', name: 'apa' },
  isCslError: (value: unknown) =>
    typeof value === 'object' && value !== null && 'code' in value && 'message' in value,
  renderCluster: vi.fn(),
}))

const LONG_TITLE =
  'Organización y lucha obrera en la Argentina contemporánea: una historia deliberadamente extensa'

const ITEMS = [
  {
    itemKey: 'A',
    title: LONG_TITLE,
    snapshot: JSON.stringify({ id: 'A', title: LONG_TITLE }),
    locator: '',
    locatorType: 'page',
    suppressAuthor: false,
  },
  {
    itemKey: 'B',
    title: 'Una segunda referencia',
    snapshot: JSON.stringify({ id: 'B', title: 'Una segunda referencia' }),
    locator: '3',
    locatorType: 'chapter',
    suppressAuthor: false,
  },
]

const mockRenderCluster = vi.mocked(renderCluster)

function mount(overrides: Record<string, unknown> = {}) {
  return render(WritingCitationEditor, {
    props: {
      items: ITEMS.map((item) => ({ ...item })),
      affixes: { prefix: '', suffix: '' },
      onapply: vi.fn(),
      onclose: vi.fn(),
      ...overrides,
    },
  })
}

beforeEach(() => {
  mockRenderCluster.mockReset()
  mockRenderCluster.mockImplementation(async (items) => ({
    text: `${items[0]?.prefix ?? ''}[${items
      .map((item) => `${item.locator_kind}:${item.locator ?? ''}:${item.suppress_author}`)
      .join(';')}]${items[0]?.suffix ?? ''}`,
    author_suppressed: true,
  }))
})

describe('the lateral citation editor', () => {
  it('offers explicit back and cancel actions that discard the draft', async () => {
    const onclose = vi.fn()
    mount({ onclose })

    await fireEvent.click(screen.getByRole('button', { name: 'Volver a referencias' }))
    expect(onclose).toHaveBeenCalledOnce()

    await fireEvent.click(screen.getByRole('button', { name: 'Cancelar' }))
    expect(onclose).toHaveBeenCalledTimes(2)
  })

  it('exposes the full long title through the global tooltip', () => {
    mount()

    const title = screen.getByText(LONG_TITLE)
    expect(title).toHaveAttribute('data-tooltip', LONG_TITLE)
  })

  it('removes one work without allowing the last work to be removed', async () => {
    mount()

    const removeButtons = screen.getAllByRole('button', { name: 'Quitar esta obra de la cita' })
    expect(removeButtons).toHaveLength(2)
    await fireEvent.click(removeButtons[0]!)

    expect(screen.queryByText(LONG_TITLE)).not.toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: 'Quitar esta obra de la cita' })
    ).not.toBeInTheDocument()
  })

  it('updates locator data, affixes, suppression and preview before applying', async () => {
    const onapply = vi.fn()
    mount({ items: [{ ...ITEMS[0]! }], onapply })

    await fireEvent.input(screen.getByLabelText('Localizador'), { target: { value: '45-50' } })
    const locatorKind = screen.getByLabelText('Tipo de localizador') as HTMLSelectElement
    locatorKind.value = 'section'
    await fireEvent.change(locatorKind)
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Ya nombré al autor en mi frase' }))
    await fireEvent.input(screen.getByLabelText('Antes de la cita'), {
      target: { value: 'véase ' },
    })
    await fireEvent.input(screen.getByLabelText('Después de la cita'), {
      target: { value: ', passim' },
    })

    await waitFor(() => {
      expect(screen.getByText('véase [section:45-50:true], passim')).toBeInTheDocument()
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Listo' }))
    expect(onapply).toHaveBeenCalledWith(
      expect.objectContaining({
        items: [
          expect.objectContaining({
            itemKey: 'A',
            locator: '45-50',
            locatorType: 'section',
            suppressAuthor: true,
          }),
        ],
        prefix: 'véase ',
        suffix: ', passim',
        renderedText: 'véase [section:45-50:true], passim',
      })
    )
  })
})
