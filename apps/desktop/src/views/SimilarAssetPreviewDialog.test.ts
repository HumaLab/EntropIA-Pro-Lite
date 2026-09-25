import { cleanup, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import SimilarAssetPreviewDialog from './SimilarAssetPreviewDialog.svelte'

vi.mock('@entropia/ui', async () => {
  const actual = await vi.importActual<typeof import('@entropia/ui')>('@entropia/ui')
  const MockActionIcon = (await import('./__mocks__/MockActionIcon.svelte')).default
  const MockDocumentViewer = (await import('./__mocks__/MockDocumentViewer.svelte')).default
  return { ...actual, ActionIcon: MockActionIcon, DocumentViewer: MockDocumentViewer }
})

vi.mock('../components/OcrRichText.svelte', async () => ({
  default: (await import('./__mocks__/MockOcrRichText.svelte')).default,
}))

vi.mock('$lib/file-import', () => ({
  getAssetUrl: (path: string) => `https://asset.localhost/${path}`,
  loadAudioPreviewBlob: vi.fn(),
}))

afterEach(() => {
  cleanup()
  document.body.innerHTML = ''
})

describe('SimilarAssetPreviewDialog', () => {
  // A `.work-pane` is a CSS size container, which makes it the containing block
  // of every `position: fixed` descendant: the full-window preview would be
  // clipped to the pane that opened it.
  it('floats its overlay out of the pane it was opened from', async () => {
    const pane = document.createElement('div')
    pane.className = 'work-pane'
    document.body.appendChild(pane)

    render(SimilarAssetPreviewDialog, {
      target: pane,
      props: {
        asset: {
          assetId: 'asset-1',
          itemId: 'item-1',
          title: 'Carta manuscrita',
          collectionId: 'col-1',
          assetPath: 'archivo/carta.jpg',
          assetType: 'image',
          textPreview: 'Preview',
          similarity: 0.9,
        },
        translate: (key: string) => key,
        documentViewerLabels: {},
        loadFullText: async () => null,
        onclose: vi.fn(),
      },
    })

    await screen.findByRole('dialog', { name: 'Carta manuscrita' })
    const overlay = document.querySelector('.asset-preview__overlay')
    expect(overlay).not.toBeNull()
    expect(pane.contains(overlay)).toBe(false)
    expect(overlay?.parentElement).toBe(document.body)
  })
})
