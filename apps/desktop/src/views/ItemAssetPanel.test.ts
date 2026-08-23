import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import type { Asset } from '@entropia/store'
import type { ViewerType } from '@entropia/ui'
import type { AssetOcrState } from '$lib/ocr'
import type { AssetTranscriptionState } from '$lib/transcription'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import MockItemAssetPanelDocumentViewer from './__mocks__/MockItemAssetPanelDocumentViewer.svelte'
import MockOcrRichText from './__mocks__/MockOcrRichText.svelte'
import ItemAssetPanel from './ItemAssetPanel.svelte'

const { clipboardWriteTextMock, exportOcrTextMock } = vi.hoisted(() => ({
  clipboardWriteTextMock: vi.fn<(value: string) => Promise<void>>(),
  exportOcrTextMock: vi.fn(),
}))

vi.mock('$lib/ocr-export', () => ({
  exportOcrText: exportOcrTextMock,
}))

vi.mock('../components/OcrRichText.svelte', () => ({
  default: MockOcrRichText,
}))

vi.mock('@entropia/ui', async () => {
  const actual = await vi.importActual<typeof import('@entropia/ui')>('@entropia/ui')
  return {
    ...actual,
    DocumentViewer: MockItemAssetPanelDocumentViewer,
  }
})

const source = '# Fuente\n\n<div>HTML</div>\n\n![](page=0,bbox=[1,2,3,4])\n'

type MakePropsOptions = {
  selectedAsset?: Partial<Asset> | null
  viewerType?: ViewerType
  ocrEditedText?: string
  transcriptionState?: AssetTranscriptionState | null
  transcriptionEditedText?: string
}

function makeProps({
  selectedAsset: selectedAssetOverride,
  viewerType = 'image',
  ocrEditedText = source,
  transcriptionState = null,
  transcriptionEditedText = '',
}: MakePropsOptions = {}) {
  const selectedAsset =
    selectedAssetOverride === null
      ? null
      : ({
          id: 'asset-1',
          type: 'image',
          path: 'C:/assets/scan.png',
          ...selectedAssetOverride,
        } as Asset)

  const ocrState: AssetOcrState = { status: 'done', progress: 100, method: 'glm_ocr' }

  return {
    selectedAsset,
    viewerSrc: 'asset://scan',
    viewerType,
    annotations: [],
    layoutRegions: [],
    showLayoutOverlay: false,
    hoveredLayoutRegionId: null,
    selectedLayoutRegionId: null,
    layoutReferenceWidth: 100,
    layoutReferenceHeight: 100,
    selectedAnnotationId: null,
    annotationTool: 'select' as const,
    annotationColor: '#ff0000',
    editTool: 'none' as const,
    canUndo: false,
    canRedo: false,
    viewerPage: 0,
    annotationSaveError: null,
    ocrState,
    ocrEditedText,
    transcriptionState,
    transcriptionEditedText,
    documentViewerLabels: {} as never,
    annotationToolbarLabels: {} as never,
    translate: (key: string, params?: Record<string, string | number>) =>
      params?.count === undefined ? key : `${key}:${params.count}`,
    onAnnotationsChange: vi.fn(),
    onSelectedAnnotationIdChange: vi.fn(),
    onAnnotationToolChange: vi.fn(),
    onAnnotationColorChange: vi.fn(),
    onLayoutRegionHoverChange: vi.fn(),
    onLayoutRegionSelect: vi.fn(),
    onEditSelect: vi.fn(),
    onEditToolChange: vi.fn(),
    onRotateLeft: vi.fn(),
    onRotateRight: vi.fn(),
    onFineRotateCommit: vi.fn(),
    onUndo: vi.fn(),
    onRedo: vi.fn(),
    onDuplicateAsset: vi.fn(),
    duplicateAssetDisabled: false,
    onPageChange: vi.fn(),
    onDimensionsChange: vi.fn(),
  }
}

async function openExtractedTextTab() {
  await fireEvent.click(screen.getByRole('tab', { name: 'item.extractedTextTab' }))
}

beforeEach(() => {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText: clipboardWriteTextMock },
  })
  clipboardWriteTextMock.mockReset().mockResolvedValue(undefined)
  exportOcrTextMock.mockReset().mockResolvedValue('/exports/scan-texto-extraido.md')
})

afterEach(() => {
  vi.clearAllMocks()
})


describe('ItemAssetPanel', () => {
  it('keeps actions hidden with the document tab and shows them only in extracted text', async () => {
    render(ItemAssetPanel, makeProps())

    const copy = screen.getByRole('button', { name: 'item.copyExtractedTextAria', hidden: true })
    const download = screen.getByRole('button', { name: 'item.downloadExtractedTextAria', hidden: true })
    expect(copy).not.toBeVisible()
    expect(download).not.toBeVisible()

    await openExtractedTextTab()

    expect(screen.getByRole('button', { name: 'item.copyExtractedTextAria' })).toBeVisible()
    expect(screen.getByRole('button', { name: 'item.downloadExtractedTextAria' })).toBeVisible()
  })

  it('keeps both actions absent for empty OCR content', async () => {
    render(ItemAssetPanel, makeProps({ ocrEditedText: '   ' }))

    await openExtractedTextTab()
    const textPane = screen.getByRole('tabpanel', { name: 'item.extractedTextTab' })

    expect(
      within(textPane).queryByRole('button', { name: 'item.copyExtractedTextAria' })
    ).not.toBeInTheDocument()
    expect(
      within(textPane).queryByRole('button', { name: 'item.downloadExtractedTextAria' })
    ).not.toBeInTheDocument()
  })

  it('keeps both actions absent for audio transcription content', async () => {
    render(
      ItemAssetPanel,
      makeProps({
        selectedAsset: {
          id: 'asset-audio-1',
          type: 'audio',
          path: 'C:/assets/interview.mp3',
        },
        viewerType: 'audio',
        transcriptionState: { status: 'done', progress: 100, language: 'es', durationMs: 93000 },
        transcriptionEditedText: 'Transcripción lista',
      })
    )


    await openExtractedTextTab()
    const textPane = screen.getByRole('tabpanel', { name: 'item.extractedTextTab' })

    expect(
      within(textPane).queryByRole('button', { name: 'item.copyExtractedTextAria' })
    ).not.toBeInTheDocument()
    expect(
      within(textPane).queryByRole('button', { name: 'item.downloadExtractedTextAria' })
    ).not.toBeInTheDocument()
  })

  it('copies the exact OCR source string without trimming or rendering it', async () => {
    render(ItemAssetPanel, makeProps())
    await openExtractedTextTab()
    await fireEvent.click(screen.getByRole('button', { name: 'item.copyExtractedTextAria' }))

    expect(clipboardWriteTextMock).toHaveBeenCalledWith(source)
  })

  it('drops pending copy feedback when the selected asset changes', async () => {
    let resolveClipboard!: () => void
    clipboardWriteTextMock.mockReturnValueOnce(
      new Promise<void>((resolve) => {
        resolveClipboard = resolve
      })
    )

    const { rerender } = render(ItemAssetPanel, makeProps())
    await openExtractedTextTab()
    await fireEvent.click(screen.getByRole('button', { name: 'item.copyExtractedTextAria' }))

    await rerender(
      makeProps({
        selectedAsset: {
          id: 'asset-2',
          type: 'image',
          path: 'C:/assets/scan-2.png',
        },
        ocrEditedText: '# Nuevo\n\ncontenido',
      })
    )

    resolveClipboard()

    await waitFor(() => {
      expect(screen.queryByText('item.copyExtractedTextSuccess')).not.toBeInTheDocument()
      expect(screen.queryByText('item.copyExtractedTextError')).not.toBeInTheDocument()
    })
  })

  it('keeps copy and export feedback screen-reader-only', async () => {
    render(ItemAssetPanel, makeProps())
    await openExtractedTextTab()

    await fireEvent.click(screen.getByRole('button', { name: 'item.copyExtractedTextAria' }))

    await waitFor(() => {
      expect(screen.getByText('item.copyExtractedTextSuccess')).toHaveClass('sr-only')
    })

    let resolveExport!: (value: string | null) => void
    exportOcrTextMock.mockReturnValueOnce(
      new Promise<string | null>((resolve) => {
        resolveExport = resolve
      })
    )

    await fireEvent.click(screen.getByRole('button', { name: 'item.downloadExtractedTextAria' }))
    await fireEvent.click(screen.getByRole('menuitem', { name: 'item.exportExtractedTextPdf' }))

    expect(screen.getByText('item.exportExtractedTextWorking')).toHaveClass('sr-only')

    resolveExport('/exports/scan-texto-extraido.pdf')

    await waitFor(() => {
      expect(screen.queryByText('item.exportExtractedTextWorking')).not.toBeInTheDocument()
    })

    exportOcrTextMock.mockRejectedValueOnce(new Error('provider unavailable'))

    await fireEvent.click(screen.getByRole('button', { name: 'item.downloadExtractedTextAria' }))
    await fireEvent.click(screen.getByRole('menuitem', { name: 'item.exportExtractedTextMarkdown' }))

    await waitFor(() => {
      expect(screen.getByText('item.exportExtractedTextError')).toHaveClass('sr-only')
    })
  })
  it('opens exactly the three download formats and routes the selected format', async () => {
    render(ItemAssetPanel, makeProps())
    await openExtractedTextTab()
    await fireEvent.click(screen.getByRole('button', { name: 'item.downloadExtractedTextAria' }))

    expect(screen.getByRole('menu', { name: 'item.downloadExtractedTextMenu' })).toBeInTheDocument()
    expect(screen.getByRole('menuitem', { name: 'item.exportExtractedTextMarkdown' })).toBeVisible()
    expect(screen.getByRole('menuitem', { name: 'item.exportExtractedTextPdf' })).toBeVisible()
    expect(screen.getByRole('menuitem', { name: 'item.exportExtractedTextDocx' })).toBeVisible()
    expect(screen.getAllByRole('menuitem')).toHaveLength(3)

    await fireEvent.click(screen.getByRole('menuitem', { name: 'item.exportExtractedTextDocx' }))

    expect(exportOcrTextMock).toHaveBeenCalledWith(
      expect.objectContaining({ source, assetUrl: 'asset://scan', sourceType: 'image' }),
      'docx',
      'scan-texto-extraido.docx'
    )
  })

  it('disables both actions while an export is pending and closes on Escape', async () => {
    let resolveExport!: (path: string | null) => void
    exportOcrTextMock.mockReturnValueOnce(
      new Promise<string | null>((resolve) => {
        resolveExport = resolve
      })
    )

    render(ItemAssetPanel, makeProps())
    await openExtractedTextTab()
    const download = screen.getByRole('button', { name: 'item.downloadExtractedTextAria' })
    await fireEvent.click(download)
    await fireEvent.click(screen.getByRole('menuitem', { name: 'item.exportExtractedTextPdf' }))

    expect(screen.getByRole('button', { name: 'item.copyExtractedTextAria' })).toBeDisabled()
    expect(download).toBeDisabled()

    resolveExport('/exports/scan-texto-extraido.pdf')
    await waitFor(() => expect(download).not.toBeDisabled())

    await fireEvent.click(download)
    expect(screen.getByRole('menu')).toBeInTheDocument()
    await fireEvent.keyDown(window, { key: 'Escape' })
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    expect(document.activeElement).toBe(download)
  })
})
