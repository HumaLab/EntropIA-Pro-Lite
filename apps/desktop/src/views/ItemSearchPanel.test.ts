import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import type { ComponentProps } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import ItemSearchPanel from './ItemSearchPanel.svelte'
import { locale, t } from '$lib/i18n'

afterEach(cleanup)
beforeEach(() => locale.set('es'))

type Props = ComponentProps<typeof ItemSearchPanel>

function setup(overrides: Partial<Props> = {}) {
  const props: Props = {
    assetsCount: 1,
    selectedAsset: true,
    selectedAssetIndex: 0,
    ftsQuery: '',
    ftsResults: [],
    ftsSearching: false,
    ftsSearchError: null,
    ftsIndexedRows: 12,
    ftsDebug: null,
    ftsReadinessKey: null,
    similarAssets: [
      {
        assetId: 'asset-9',
        itemId: 'item-9',
        title: 'Acta',
        collectionId: 'col-1',
        assetPath: '/imports/acta.pdf',
        assetType: 'pdf',
        similarity: 0.8,
      },
    ],
    similarAssetsReadinessKey: null,
    isDev: false,
    translate: t,
    onFtsInput: vi.fn(),
    onFtsKeydown: vi.fn(),
    onFtsClear: vi.fn(),
    onNavigateToFtsItem: vi.fn(),
    onPreviewSimilarAsset: vi.fn(),
    ...overrides,
  }
  return render(ItemSearchPanel, props)
}

describe('ItemSearchPanel', () => {
  it('keeps the technical metadata of a similar page behind a disclosure', async () => {
    const { container } = setup()
    expect(container.querySelector('details')).toBeNull()

    const header = screen.getByRole('button', { name: /acta\.pdf/ })
    expect(header).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByText(/página asset-9/)).not.toBeVisible()

    await fireEvent.click(header)
    expect(header).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByText(/página asset-9 · documento item-9 · colección col-1/)).toBeVisible()
  })

  it('opens the developer FTS panel from a disclosure header', async () => {
    setup({ isDev: true })
    const header = screen.getByRole('button', { name: 'FTS Debug (solo dev)' })
    expect(header).toHaveAttribute('aria-expanded', 'false')

    await fireEvent.click(header)
    expect(header).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByText('12')).toBeVisible()
  })

  it('previews a similar page from its card', async () => {
    const onPreviewSimilarAsset = vi.fn()
    setup({ onPreviewSimilarAsset })
    await fireEvent.click(screen.getByTestId('similar-asset-asset-9'))
    expect(onPreviewSimilarAsset).toHaveBeenCalledWith(
      expect.objectContaining({ assetId: 'asset-9' })
    )
  })
})
