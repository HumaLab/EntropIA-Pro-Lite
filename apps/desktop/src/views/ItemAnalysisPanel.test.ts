import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import ItemAnalysisPanel from './ItemAnalysisPanel.svelte'
import type { ItemNlpState } from '$lib/nlp'

vi.mock('@entropia/ui', async () => {
  const MockEntityViewer = (await import('./__mocks__/MockEntityViewer.svelte')).default
  const MockMapViewer = (await import('./__mocks__/MockMapViewer.svelte')).default
  const ActualStatusBadge = (
    await import('../../../../packages/ui/src/components/StatusBadge/StatusBadge.svelte')
  ).default

  return {
    EntityViewer: MockEntityViewer,
    MapViewer: MockMapViewer,
    StatusBadge: ActualStatusBadge,
  }
})

const idleNlpState: ItemNlpState = { fts: 'idle', embed: 'idle', ner: 'idle', triples: 'idle' }

function makeProps(onCreateEntity = vi.fn()) {
  return {
    assetsCount: 1,
    selectedAsset: true,
    selectedAssetIndex: 0,
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
    triples: [],
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
    onSaveMapLocation: vi.fn(),
    onResetMapLocation: vi.fn(),
  }
}

describe('ItemAnalysisPanel', () => {
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
