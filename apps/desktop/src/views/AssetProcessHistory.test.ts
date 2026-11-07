import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import AssetProcessHistory from './AssetProcessHistory.svelte'
import { t } from '$lib/i18n'

const { findByAssetMock, saveMock, writeFileMock } = vi.hoisted(() => ({
  findByAssetMock: vi.fn(),
  saveMock: vi.fn(),
  writeFileMock: vi.fn(),
}))

vi.mock('$lib/db', () => ({
  getStore: () => ({ assetProcessEvents: { findByAsset: findByAssetMock } }),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: saveMock }))
vi.mock('@tauri-apps/plugin-fs', () => ({ writeFile: writeFileMock }))

const asset = { id: 'a1', path: 'assets/c/i/scan.png', type: 'image' as const }

const events = [
  {
    id: 'e1',
    assetId: 'a1',
    process: 'import',
    tool: 'EntropIA (importación de archivo)',
    toolVersion: null,
    model: null,
    modelVersion: null,
    appVersion: '1.4.0',
    params: { originalName: 'scan.png' },
    inputSha256: null,
    outputSha256: 'a'.repeat(64),
    createdAt: 1_000,
  },
  {
    id: 'e2',
    assetId: 'a1',
    process: 'ocr',
    tool: 'GLM-OCR API (z.ai layout_parsing)',
    toolVersion: 'paas/v4',
    model: 'glm-ocr',
    modelVersion: null,
    appVersion: '1.4.0',
    params: { pageCount: 1 },
    inputSha256: null,
    outputSha256: 'b'.repeat(64),
    createdAt: 2_000,
  },
]

describe('AssetProcessHistory', () => {
  beforeEach(() => {
    findByAssetMock.mockReset().mockResolvedValue(events)
    saveMock.mockReset()
    writeFileMock.mockReset().mockResolvedValue(undefined)
  })

  it('lists processes newest first with tool, model and versions', async () => {
    render(AssetProcessHistory, { asset, translate: t })

    const items = await screen.findAllByRole('listitem')
    expect(items).toHaveLength(2)
    expect(items[0]).toHaveTextContent('OCR')
    expect(items[0]).toHaveTextContent('GLM-OCR API (z.ai layout_parsing)')
    expect(items[0]).toHaveTextContent('paas/v4')
    expect(items[0]).toHaveTextContent('glm-ocr')
    expect(items[0]).toHaveTextContent('1.4.0')
    expect(items[1]).toHaveTextContent('Importación')
    expect(findByAssetMock).toHaveBeenCalledWith('a1')
  })

  it('reloads when the refresh key changes', async () => {
    const { rerender } = render(AssetProcessHistory, { asset, translate: t, refreshKey: '1' })
    await screen.findAllByRole('listitem')
    await rerender({ refreshKey: '2' })
    await waitFor(() => expect(findByAssetMock).toHaveBeenCalledTimes(2))
  })

  it('says so when nothing was recorded', async () => {
    findByAssetMock.mockResolvedValue([])
    render(AssetProcessHistory, { asset, translate: t })
    expect(await screen.findByTestId('asset-process-history-empty')).toBeInTheDocument()
  })

  it('exports the history as JSON, oldest first', async () => {
    saveMock.mockResolvedValue('/tmp/historial.json')
    render(AssetProcessHistory, { asset, translate: t })
    await screen.findAllByRole('listitem')

    await fireEvent.click(screen.getByTestId('asset-process-history-export'))

    await waitFor(() => expect(writeFileMock).toHaveBeenCalled())
    const [path, bytes] = writeFileMock.mock.calls[0] as [string, Uint8Array]
    expect(path).toBe('/tmp/historial.json')
    const exported = JSON.parse(new TextDecoder().decode(bytes))
    expect(exported.format).toBe('entropia-asset-process-history')
    expect(exported.asset.id).toBe('a1')
    expect(exported.events.map((event: { process: string }) => event.process)).toEqual([
      'import',
      'ocr',
    ])
    expect(await screen.findByTestId('asset-process-history-message')).toHaveTextContent(
      'Historial exportado.'
    )
  })
})
