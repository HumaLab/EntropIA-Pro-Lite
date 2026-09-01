import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import ItemAnalysisPanel from './ItemAnalysisPanel.svelte'
import itemAnalysisPanelSource from './ItemAnalysisPanel.svelte?raw'
import type { ItemNlpState } from '$lib/nlp'

vi.mock('@entropia/ui', async () => {
  const MockEntityViewer = (await import('./__mocks__/MockEntityViewer.svelte')).default
  const MockMapViewer = (await import('./__mocks__/MockMapViewer.svelte')).default
  const ActualStatusBadge = (
    await import('../../../../packages/ui/src/components/StatusBadge/StatusBadge.svelte')
  ).default

  const ActualActionIcon = (
    await import('../../../../packages/ui/src/components/Button/ActionIcon.svelte')
  ).default

  return {
    ActionIcon: ActualActionIcon,
    EntityViewer: MockEntityViewer,
    MapViewer: MockMapViewer,
    StatusBadge: ActualStatusBadge,
  }
})

vi.mock('@entropia/ui/components/MapViewer', async () => ({
  MapViewer: (await import('./__mocks__/MockMapViewer.svelte')).default,
}))

const idleNlpState: ItemNlpState = { fts: 'idle', embed: 'idle', ner: 'idle', triples: 'idle' }

type Triple = { id: string; subject: string; predicate: string; object: string }

function triple(id: string, subject: string, predicate: string, object: string): Triple {
  return { id, subject, predicate, object }
}

function makeProps(
  onCreateEntity = vi.fn(),
  overrides: Partial<Record<string, unknown>> = {}
) {
  return {
    assetsCount: 1,
    selectedAsset: true,
    nlpState: idleNlpState,
    llmAvailable: true,
    geoMarkers: [],
    visible: true,
    entities: [],
    editingEntityId: null,
    editingEntityValue: '',
    newEntityType: 'person' as const,
    newEntityValue: 'Juana Rouco',
    entityActionError: null,
    triples: [] as Triple[],
    tripleActionError: null as string | null,
    translate: (key: string) => key,
    onIndexFts: vi.fn(),
    onEmbedAsset: vi.fn(),
    onExtractEntities: vi.fn(),
    onExtractTriples: vi.fn(),
    onEntityClick: vi.fn(),
    onEditValueChange: vi.fn(),
    onSaveEntity: vi.fn(),
    onCancelEntityEdit: vi.fn(),
    onDeleteEntity: vi.fn(),
    onNewEntityTypeChange: vi.fn(),
    onNewEntityValueChange: vi.fn(),
    onCreateEntity,
    onCreateTriple: vi.fn().mockResolvedValue(true),
    onSaveTriple: vi.fn().mockResolvedValue(true),
    onDeleteTriple: vi.fn(),
    onSaveMapLocation: vi.fn(),
    onResetMapLocation: vi.fn(),
    ...overrides,
  }
}

describe('ItemAnalysisPanel', () => {
  it('reflows analysis controls inside narrow sidebars', () => {
    expect(itemAnalysisPanelSource).toContain('container-type: inline-size;')
    const narrowLayout = itemAnalysisPanelSource.slice(
      itemAnalysisPanelSource.indexOf('@container (max-width: 30rem)'),
      itemAnalysisPanelSource.indexOf('@container (max-width: 16rem)')
    )
    expect(narrowLayout).toContain('grid-template-columns: repeat(2, minmax(0, 1fr));')
    expect(narrowLayout).toContain('grid-column: 1 / -1;')
    expect(itemAnalysisPanelSource).toMatch(
      /\.entity-editor__create select,\s*\.entity-editor__create input\s*\{[^}]*box-sizing: border-box;[^}]*width: 100%;[^}]*max-width: 100%;/s
    )
  })

  it('keeps the predicate column centered', () => {
    expect(itemAnalysisPanelSource).toMatch(
      /\.triple-cell--predicate\s*\{[^}]*text-align: center;/s
    )
  })

  it('does not widen the reading grid to make room for the row controls', () => {
    // Los tres anchos de lectura no se tocan: los controles flotan sobre el
    // borde y solo la fila en edición gana una pista propia.
    expect(itemAnalysisPanelSource).toMatch(
      /\.triple-item\s*\{[^}]*grid-template-columns: 1fr 1fr 1fr;/s
    )
    expect(itemAnalysisPanelSource).toMatch(
      /\.triple-item--editing\s*\{[^}]*grid-template-columns: 1fr 1fr 1fr auto;/s
    )
  })

  it('edits one triple inline and persists the three trimmed fields', async () => {
    const onSaveTriple = vi.fn().mockResolvedValue(true)
    render(
      ItemAnalysisPanel,
      makeProps(vi.fn(), {
        triples: [
          triple('t-1', 'los tripulantes', 'se reintegraron', 'a sus tareas'),
          triple('t-2', 'la actividad', 'quedó', 'reiniciada'),
        ],
        onSaveTriple,
      })
    )

    await fireEvent.click(screen.getByTestId('triple-edit-t-1'))

    const subject = screen.getByRole('textbox', { name: 'item.tripleSubjectAria' })
    const predicate = screen.getByRole('textbox', { name: 'item.triplePredicateAria' })
    const object = screen.getByRole('textbox', { name: 'item.tripleObjectAria' })
    expect((subject as HTMLInputElement).value).toBe('los tripulantes')
    expect((predicate as HTMLInputElement).value).toBe('se reintegraron')
    expect((object as HTMLInputElement).value).toBe('a sus tareas')

    await fireEvent.input(subject, { target: { value: '  la tripulación  ' } })
    await fireEvent.input(predicate, { target: { value: 'se reintegró' } })
    await fireEvent.click(screen.getByTestId('triple-save-t-1'))

    expect(onSaveTriple).toHaveBeenCalledTimes(1)
    expect(onSaveTriple).toHaveBeenCalledWith('t-1', {
      subject: 'la tripulación',
      predicate: 'se reintegró',
      object: 'a sus tareas',
    })
    // La otra fila nunca entró en edición.
    expect(screen.getByTestId('triple-edit-t-2')).toBeInTheDocument()
  })

  it('closes the row after a successful save and reopens nothing else', async () => {
    render(
      ItemAnalysisPanel,
      makeProps(vi.fn(), {
        triples: [triple('t-1', 'la actividad', 'quedó', 'reiniciada')],
      })
    )

    await fireEvent.click(screen.getByTestId('triple-edit-t-1'))
    await fireEvent.click(screen.getByTestId('triple-save-t-1'))

    expect(screen.queryByRole('textbox', { name: 'item.tripleSubjectAria' })).toBeNull()
    expect(screen.getByTestId('triple-edit-t-1')).toBeInTheDocument()
  })

  it('keeps the row open with the typed values when the save fails', async () => {
    const onSaveTriple = vi.fn().mockResolvedValue(false)
    render(
      ItemAnalysisPanel,
      makeProps(vi.fn(), {
        triples: [triple('t-1', 'la actividad', 'quedó', 'reiniciada')],
        onSaveTriple,
      })
    )

    await fireEvent.click(screen.getByTestId('triple-edit-t-1'))
    const object = screen.getByRole('textbox', { name: 'item.tripleObjectAria' })
    await fireEvent.input(object, { target: { value: 'suspendida' } })
    await fireEvent.click(screen.getByTestId('triple-save-t-1'))

    const stillOpen = screen.getByRole('textbox', { name: 'item.tripleObjectAria' })
    expect((stillOpen as HTMLInputElement).value).toBe('suspendida')
  })

  it('cancels the edit with Escape and with the cancel control', async () => {
    const onSaveTriple = vi.fn().mockResolvedValue(true)
    render(
      ItemAnalysisPanel,
      makeProps(vi.fn(), {
        triples: [triple('t-1', 'la actividad', 'quedó', 'reiniciada')],
        onSaveTriple,
      })
    )

    await fireEvent.click(screen.getByTestId('triple-edit-t-1'))
    await fireEvent.keyDown(screen.getByRole('textbox', { name: 'item.tripleSubjectAria' }), {
      key: 'Escape',
    })
    expect(screen.queryByRole('textbox', { name: 'item.tripleSubjectAria' })).toBeNull()

    await fireEvent.click(screen.getByTestId('triple-edit-t-1'))
    await fireEvent.click(screen.getByTestId('triple-cancel-t-1'))
    expect(screen.queryByRole('textbox', { name: 'item.tripleSubjectAria' })).toBeNull()
    expect(onSaveTriple).not.toHaveBeenCalled()
  })

  it('saves on Enter but not while IME composition is active', async () => {
    const onSaveTriple = vi.fn().mockResolvedValue(true)
    render(
      ItemAnalysisPanel,
      makeProps(vi.fn(), {
        triples: [triple('t-1', 'la actividad', 'quedó', 'reiniciada')],
        onSaveTriple,
      })
    )

    await fireEvent.click(screen.getByTestId('triple-edit-t-1'))
    const subject = screen.getByRole('textbox', { name: 'item.tripleSubjectAria' })

    await fireEvent.keyDown(subject, { key: 'Enter', isComposing: true })
    expect(onSaveTriple).not.toHaveBeenCalled()

    await fireEvent.keyDown(subject, { key: 'Enter' })
    expect(onSaveTriple).toHaveBeenCalledTimes(1)
  })

  it('refuses to save a triple with an empty field', async () => {
    const onSaveTriple = vi.fn().mockResolvedValue(true)
    render(
      ItemAnalysisPanel,
      makeProps(vi.fn(), {
        triples: [triple('t-1', 'la actividad', 'quedó', 'reiniciada')],
        onSaveTriple,
      })
    )

    await fireEvent.click(screen.getByTestId('triple-edit-t-1'))
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.triplePredicateAria' }), {
      target: { value: '   ' },
    })

    expect(screen.getByTestId('triple-save-t-1')).toBeDisabled()
    await fireEvent.click(screen.getByTestId('triple-save-t-1'))
    expect(onSaveTriple).not.toHaveBeenCalled()
  })

  it('requires a second click to delete and only removes that row', async () => {
    const onDeleteTriple = vi.fn()
    render(
      ItemAnalysisPanel,
      makeProps(vi.fn(), {
        triples: [
          triple('t-1', 'la actividad', 'quedó', 'reiniciada'),
          triple('t-2', 'los tripulantes', 'se reintegraron', 'a sus tareas'),
        ],
        onDeleteTriple,
      })
    )

    const deleteButton = screen.getByTestId('triple-delete-t-1')
    await fireEvent.click(deleteButton)
    expect(onDeleteTriple).not.toHaveBeenCalled()
    expect(deleteButton).toHaveAttribute('title', 'item.tripleConfirmDeleteTitle')

    await fireEvent.click(screen.getByTestId('triple-delete-t-1'))
    expect(onDeleteTriple).toHaveBeenCalledTimes(1)
    expect(onDeleteTriple).toHaveBeenCalledWith('t-1')
  })

  it('disarms a pending delete on Escape', async () => {
    const onDeleteTriple = vi.fn()
    render(
      ItemAnalysisPanel,
      makeProps(vi.fn(), {
        triples: [triple('t-1', 'la actividad', 'quedó', 'reiniciada')],
        onDeleteTriple,
      })
    )

    await fireEvent.click(screen.getByTestId('triple-delete-t-1'))
    await fireEvent.keyDown(screen.getByTestId('triple-delete-t-1'), { key: 'Escape' })
    expect(screen.getByTestId('triple-delete-t-1')).toHaveAttribute(
      'title',
      'item.tripleDeleteTitle'
    )

    await fireEvent.click(screen.getByTestId('triple-delete-t-1'))
    expect(onDeleteTriple).not.toHaveBeenCalled()
  })

  it('surfaces a triple action error under the list', () => {
    render(
      ItemAnalysisPanel,
      makeProps(vi.fn(), {
        triples: [triple('t-1', 'la actividad', 'quedó', 'reiniciada')],
        tripleActionError: 'No se pudo guardar la tripleta.',
      })
    )

    expect(screen.getByText('No se pudo guardar la tripleta.')).toBeInTheDocument()
  })

  it('opens a draft row from the add action and persists the three trimmed fields', async () => {
    const onCreateTriple = vi.fn().mockResolvedValue(true)
    render(
      ItemAnalysisPanel,
      makeProps(vi.fn(), {
        triples: [triple('t-1', 'la actividad', 'quedó', 'reiniciada')],
        onCreateTriple,
      })
    )

    expect(screen.queryByTestId('triple-new-row')).toBeNull()
    await fireEvent.click(screen.getByTestId('triple-add'))
    expect(screen.getByTestId('triple-new-row')).toBeInTheDocument()

    await fireEvent.input(screen.getByRole('textbox', { name: 'item.newTripleSubjectAria' }), {
      target: { value: '  la asamblea  ' },
    })
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.newTriplePredicateAria' }), {
      target: { value: 'resolvió' },
    })
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.newTripleObjectAria' }), {
      target: { value: 'levantar la medida' },
    })
    await fireEvent.click(screen.getByTestId('triple-new-save'))

    expect(onCreateTriple).toHaveBeenCalledTimes(1)
    expect(onCreateTriple).toHaveBeenCalledWith({
      subject: 'la asamblea',
      predicate: 'resolvió',
      object: 'levantar la medida',
    })
    expect(screen.queryByTestId('triple-new-row')).toBeNull()
  })

  it('offers the add action even when the item has no triples yet', async () => {
    const onCreateTriple = vi.fn().mockResolvedValue(true)
    render(ItemAnalysisPanel, makeProps(vi.fn(), { triples: [], onCreateTriple }))

    expect(screen.getByText('item.noTriples')).toBeInTheDocument()
    await fireEvent.click(screen.getByTestId('triple-add'))

    // El vacío cede el lugar al borrador en vez de convivir con él.
    expect(screen.queryByText('item.noTriples')).toBeNull()
    expect(screen.getByTestId('triple-new-row')).toBeInTheDocument()
  })

  it('creates nothing when the draft is cancelled', async () => {
    const onCreateTriple = vi.fn().mockResolvedValue(true)
    render(ItemAnalysisPanel, makeProps(vi.fn(), { triples: [], onCreateTriple }))

    await fireEvent.click(screen.getByTestId('triple-add'))
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.newTripleSubjectAria' }), {
      target: { value: 'la asamblea' },
    })
    await fireEvent.click(screen.getByTestId('triple-new-cancel'))

    expect(onCreateTriple).not.toHaveBeenCalled()
    expect(screen.queryByTestId('triple-new-row')).toBeNull()

    // Y el borrador arranca limpio la próxima vez.
    await fireEvent.click(screen.getByTestId('triple-add'))
    expect(
      (screen.getByRole('textbox', { name: 'item.newTripleSubjectAria' }) as HTMLInputElement).value
    ).toBe('')
  })

  it('discards the draft with Escape and saves it with Enter, but not while composing', async () => {
    const onCreateTriple = vi.fn().mockResolvedValue(true)
    render(ItemAnalysisPanel, makeProps(vi.fn(), { triples: [], onCreateTriple }))

    await fireEvent.click(screen.getByTestId('triple-add'))
    await fireEvent.keyDown(screen.getByRole('textbox', { name: 'item.newTripleSubjectAria' }), {
      key: 'Escape',
    })
    expect(screen.queryByTestId('triple-new-row')).toBeNull()
    expect(onCreateTriple).not.toHaveBeenCalled()

    await fireEvent.click(screen.getByTestId('triple-add'))
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.newTripleSubjectAria' }), {
      target: { value: 'la asamblea' },
    })
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.newTriplePredicateAria' }), {
      target: { value: 'resolvió' },
    })
    const object = screen.getByRole('textbox', { name: 'item.newTripleObjectAria' })
    await fireEvent.input(object, { target: { value: 'levantar la medida' } })

    await fireEvent.keyDown(object, { key: 'Enter', isComposing: true })
    expect(onCreateTriple).not.toHaveBeenCalled()

    await fireEvent.keyDown(object, { key: 'Enter' })
    expect(onCreateTriple).toHaveBeenCalledTimes(1)
  })

  it('refuses to save a draft with an empty field', async () => {
    const onCreateTriple = vi.fn().mockResolvedValue(true)
    render(ItemAnalysisPanel, makeProps(vi.fn(), { triples: [], onCreateTriple }))

    await fireEvent.click(screen.getByTestId('triple-add'))
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.newTripleSubjectAria' }), {
      target: { value: 'la asamblea' },
    })
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.newTriplePredicateAria' }), {
      target: { value: '   ' },
    })

    expect(screen.getByTestId('triple-new-save')).toBeDisabled()
    await fireEvent.click(screen.getByTestId('triple-new-save'))
    expect(onCreateTriple).not.toHaveBeenCalled()
  })

  it('keeps the draft open with its values when the insert fails', async () => {
    const onCreateTriple = vi.fn().mockResolvedValue(false)
    render(ItemAnalysisPanel, makeProps(vi.fn(), { triples: [], onCreateTriple }))

    await fireEvent.click(screen.getByTestId('triple-add'))
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.newTripleSubjectAria' }), {
      target: { value: 'la asamblea' },
    })
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.newTriplePredicateAria' }), {
      target: { value: 'resolvió' },
    })
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.newTripleObjectAria' }), {
      target: { value: 'levantar la medida' },
    })
    await fireEvent.click(screen.getByTestId('triple-new-save'))

    expect(screen.getByTestId('triple-new-row')).toBeInTheDocument()
    expect(
      (screen.getByRole('textbox', { name: 'item.newTripleSubjectAria' }) as HTMLInputElement).value
    ).toBe('la asamblea')
  })

  it('leaves an open row edit untouched while a draft is being written', async () => {
    const onSaveTriple = vi.fn().mockResolvedValue(true)
    const onCreateTriple = vi.fn().mockResolvedValue(true)
    render(
      ItemAnalysisPanel,
      makeProps(vi.fn(), {
        triples: [triple('t-1', 'la actividad', 'quedó', 'reiniciada')],
        onSaveTriple,
        onCreateTriple,
      })
    )

    await fireEvent.click(screen.getByTestId('triple-edit-t-1'))
    await fireEvent.input(screen.getByRole('textbox', { name: 'item.tripleObjectAria' }), {
      target: { value: 'suspendida' },
    })
    await fireEvent.click(screen.getByTestId('triple-add'))

    // Abrir el alta no cancela ni pisa la edición en curso.
    expect(
      (screen.getByRole('textbox', { name: 'item.tripleObjectAria' }) as HTMLInputElement).value
    ).toBe('suspendida')
    expect(screen.getByTestId('triple-new-row')).toBeInTheDocument()
  })

  it('does not create the entity on Enter while IME composition is active', async () => {
    const onCreateEntity = vi.fn()
    render(ItemAnalysisPanel, makeProps(onCreateEntity))

    const input = screen.getByRole('textbox', { name: 'item.newEntityValue' })

    await fireEvent.keyDown(input, { key: 'Enter', isComposing: true })
    expect(onCreateEntity).not.toHaveBeenCalled()

    await fireEvent.keyDown(input, { key: 'Enter' })
    expect(onCreateEntity).toHaveBeenCalledTimes(1)
  })
})
