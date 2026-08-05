import { describe, it, expect, vi, beforeEach } from 'vitest'
import {
  pickAndImportFiles,
  getAssetUrl,
  SUPPORTED_FORMATS,
  classifyFileType,
  classifyFiles,
  importFilesFromPaths,
  importSingleFile,
  deleteAssetFile,
  generatePdfThumbnail,
  loadAudioPreviewBlob,
  deletePdfThumbnail,
  duplicateAssetFile,
  getNextAssetCopyName,
} from './file-import'

type OpenSelection = string[] | string | null

// Tauri APIs are already mocked globally in test-setup.ts

describe('classifyFileType', () => {
  it('classifies png as image', () => {
    expect(classifyFileType('photo.png')).toBe('image')
  })

  it('classifies jpg as image', () => {
    expect(classifyFileType('photo.jpg')).toBe('image')
  })

  it('classifies jpeg as image', () => {
    expect(classifyFileType('photo.jpeg')).toBe('image')
  })

  it('classifies webp as image', () => {
    expect(classifyFileType('photo.webp')).toBe('image')
  })

  it('classifies tiff as image', () => {
    expect(classifyFileType('scan.tiff')).toBe('image')
  })

  it('classifies tif as image', () => {
    expect(classifyFileType('scan.tif')).toBe('image')
  })

  it('classifies pdf as pdf', () => {
    expect(classifyFileType('document.pdf')).toBe('pdf')
  })

  it('returns null for unsupported extensions', () => {
    expect(classifyFileType('script.exe')).toBeNull()
  })

  it('handles uppercase extensions', () => {
    expect(classifyFileType('PHOTO.PNG')).toBe('image')
    expect(classifyFileType('DOC.PDF')).toBe('pdf')
  })
})

describe('SUPPORTED_FORMATS', () => {
  it('includes all image, pdf, and audio formats', () => {
    expect(SUPPORTED_FORMATS).toContain('png')
    expect(SUPPORTED_FORMATS).toContain('jpg')
    expect(SUPPORTED_FORMATS).toContain('jpeg')
    expect(SUPPORTED_FORMATS).toContain('webp')
    expect(SUPPORTED_FORMATS).toContain('tiff')
    expect(SUPPORTED_FORMATS).toContain('tif')
    expect(SUPPORTED_FORMATS).toContain('pdf')
    // Audio formats
    expect(SUPPORTED_FORMATS).toContain('wav')
    expect(SUPPORTED_FORMATS).toContain('mp3')
    expect(SUPPORTED_FORMATS).toContain('flac')
    expect(SUPPORTED_FORMATS).toContain('m4a')
    expect(SUPPORTED_FORMATS).toContain('aac')
    expect(SUPPORTED_FORMATS).toContain('ogg')
    // 6 image + 1 pdf + 6 audio = 13
    expect(SUPPORTED_FORMATS).toHaveLength(13)
  })
})

describe('getAssetUrl', () => {
  it('delegates to convertFileSrc', async () => {
    const { convertFileSrc } = await import('@tauri-apps/api/core')
    vi.mocked(convertFileSrc).mockReturnValue('https://asset.localhost/path/to/file.png')

    const result = getAssetUrl('/path/to/file.png')
    expect(result).toBe('https://asset.localhost/path/to/file.png')
    expect(convertFileSrc).toHaveBeenCalledWith('/path/to/file.png')
  })
})

describe('pickAndImportFiles', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('returns empty array when user cancels dialog', async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    vi.mocked(open).mockResolvedValue(null)

    const result = await pickAndImportFiles('coll-1', 'item-1')
    expect(result).toEqual([])
  })

  it('copies selected files and returns ImportedFile array', async () => {
    const { open } = await import('@tauri-apps/plugin-dialog')
    const { copyFile, mkdir } = await import('@tauri-apps/plugin-fs')
    const { appDataDir, join } = await import('@tauri-apps/api/path')

    vi.mocked(open).mockResolvedValue(['C:/photos/sunset.png'] as OpenSelection)
    vi.mocked(mkdir).mockResolvedValue(undefined)
    vi.mocked(copyFile).mockResolvedValue(undefined)
    vi.mocked(appDataDir).mockResolvedValue('/mock/app-data')
    vi.mocked(join).mockImplementation((...parts: string[]) => Promise.resolve(parts.join('/')))

    const result = await pickAndImportFiles('coll-1', 'item-1')

    expect(result).toHaveLength(1)
    expect(result[0]).toMatchObject({
      originalName: 'sunset.png',
      type: 'image',
    })
    expect(result[0]!.destPath).toContain('coll-1')
    expect(result[0]!.destPath).toContain('item-1')
    expect(mkdir).toHaveBeenCalled()
    expect(copyFile).toHaveBeenCalled()
  })
})

describe('importFilesFromPaths', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('filters unsupported files and reports them', async () => {
    const { copyFile, mkdir } = await import('@tauri-apps/plugin-fs')
    const { appDataDir, join } = await import('@tauri-apps/api/path')

    vi.mocked(mkdir).mockResolvedValue(undefined)
    vi.mocked(copyFile).mockResolvedValue(undefined)
    vi.mocked(appDataDir).mockResolvedValue('/mock/app-data')
    vi.mocked(join).mockImplementation((...parts: string[]) => Promise.resolve(parts.join('/')))

    const result = await importFilesFromPaths(
      ['C:/docs/scan.pdf', 'C:/docs/readme.docx'],
      'c1',
      'i1'
    )

    expect(result.imported).toHaveLength(1)
    expect(result.imported[0]?.originalName).toBe('scan.pdf')
    expect(result.rejected).toEqual(['readme.docx'])
  })

  it('skips duplicate source paths in the same import batch', async () => {
    const { copyFile, mkdir } = await import('@tauri-apps/plugin-fs')
    const { appDataDir, join } = await import('@tauri-apps/api/path')

    vi.mocked(mkdir).mockResolvedValue(undefined)
    vi.mocked(copyFile).mockResolvedValue(undefined)
    vi.mocked(appDataDir).mockResolvedValue('/mock/app-data')
    vi.mocked(join).mockImplementation((...parts: string[]) => Promise.resolve(parts.join('/')))

    const duplicatePath = 'C:/docs/acta.pdf'
    const result = await importFilesFromPaths([duplicatePath, duplicatePath], 'c1', 'i1')

    expect(result.imported).toHaveLength(1)
    expect(result.skippedDuplicatePaths).toBe(1)
    expect(copyFile).toHaveBeenCalledTimes(1)
  })
})

describe('classifyFiles', () => {
  it('classifies a mixed batch of files', () => {
    const result = classifyFiles([
      'C:/docs/scan.pdf',
      'C:/photos/sunset.png',
      'C:/audio/interview.mp3',
      'C:/docs/readme.docx',
    ])

    expect(result.classified).toHaveLength(3)
    expect(result.classified[0]).toMatchObject({ name: 'scan.pdf', type: 'pdf' })
    expect(result.classified[1]).toMatchObject({ name: 'sunset.png', type: 'image' })
    expect(result.classified[2]).toMatchObject({ name: 'interview.mp3', type: 'audio' })
    expect(result.rejected).toEqual(['readme.docx'])
  })

  it('skips duplicate source paths silently', () => {
    const duplicatePath = 'C:/docs/acta.pdf'
    const result = classifyFiles([duplicatePath, duplicatePath, duplicatePath])

    expect(result.classified).toHaveLength(1)
    expect(result.rejected).toEqual([])
  })

  it('handles case-insensitive duplicate paths', () => {
    const result = classifyFiles(['C:/docs/acta.pdf', 'c:/docs/acta.pdf'])

    expect(result.classified).toHaveLength(1)
  })

  it('returns empty arrays for empty input', () => {
    const result = classifyFiles([])
    expect(result.classified).toEqual([])
    expect(result.rejected).toEqual([])
  })
})

describe('importSingleFile', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('copies a single file and returns ImportedFile metadata', async () => {
    const { copyFile, mkdir } = await import('@tauri-apps/plugin-fs')
    const { appDataDir, join } = await import('@tauri-apps/api/path')

    vi.mocked(mkdir).mockResolvedValue(undefined)
    vi.mocked(copyFile).mockResolvedValue(undefined)
    vi.mocked(appDataDir).mockResolvedValue('/mock/app-data')
    vi.mocked(join).mockImplementation((...parts: string[]) => Promise.resolve(parts.join('/')))

    const result = await importSingleFile('C:/photos/sunset.png', 'coll-1', 'item-1')

    expect(result).toMatchObject({
      originalName: 'sunset.png',
      type: 'image',
    })
    expect(result.destPath).toContain('coll-1')
    expect(result.destPath).toContain('item-1')
    expect(mkdir).toHaveBeenCalled()
    expect(copyFile).toHaveBeenCalled()
  })

  it('throws for unsupported file types', async () => {
    await expect(importSingleFile('C:/docs/readme.docx', 'coll-1', 'item-1')).rejects.toThrow(
      'Unsupported file format'
    )
  })
})

describe('deleteAssetFile', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('successfully removes an existing file', async () => {
    const { remove } = await import('@tauri-apps/plugin-fs')
    vi.mocked(remove).mockResolvedValue(undefined)

    await expect(deleteAssetFile('/path/to/file.pdf')).resolves.toBeUndefined()
    expect(remove).toHaveBeenCalledWith('/path/to/file.pdf')
  })

  it('continues silently when file does not exist (ENOENT)', async () => {
    const { remove } = await import('@tauri-apps/plugin-fs')
    vi.mocked(remove).mockRejectedValue(new Error('ENOENT: no such file or directory'))

    await expect(deleteAssetFile('/path/to/missing.pdf')).resolves.toBeUndefined()
    expect(remove).toHaveBeenCalledWith('/path/to/missing.pdf')
  })

  it('continues silently when file is not found (NotFound variant)', async () => {
    const { remove } = await import('@tauri-apps/plugin-fs')
    vi.mocked(remove).mockRejectedValue(new Error('NotFound: file not found'))

    await expect(deleteAssetFile('/path/to/missing.pdf')).resolves.toBeUndefined()
  })

  it('throws on permission errors', async () => {
    const { remove } = await import('@tauri-apps/plugin-fs')
    vi.mocked(remove).mockRejectedValue(new Error('Permission denied'))

    await expect(deleteAssetFile('/path/to/locked.pdf')).rejects.toThrow(
      'Failed to delete asset file: Permission denied'
    )
  })

  it('throws on unknown filesystem errors', async () => {
    const { remove } = await import('@tauri-apps/plugin-fs')
    vi.mocked(remove).mockRejectedValue(new Error('Unknown IO error'))

    await expect(deleteAssetFile('/path/to/file.pdf')).rejects.toThrow(
      'Failed to delete asset file: Unknown IO error'
    )
  })
})

describe('generatePdfThumbnail', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('calls invoke with correct arguments and converts path to URL', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    const { convertFileSrc } = await import('@tauri-apps/api/core')

    vi.mocked(invoke).mockResolvedValueOnce('C:\\app-data\\thumbnails\\asset-123.png')
    vi.mocked(convertFileSrc).mockReturnValueOnce('https://asset.localhost/C:/app-data/thumbnails/asset-123.png')

    const result = await generatePdfThumbnail('/path/to/document.pdf', 'asset-123')

    expect(invoke).toHaveBeenCalledWith('generate_pdf_thumbnail', {
      assetPath: '/path/to/document.pdf',
      assetId: 'asset-123',
    })
    expect(convertFileSrc).toHaveBeenCalledWith('C:\\app-data\\thumbnails\\asset-123.png')
    expect(result).toBe('https://asset.localhost/C:/app-data/thumbnails/asset-123.png')
  })

  it('returns cached thumbnail path when invoke returns it', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    const { convertFileSrc } = await import('@tauri-apps/api/core')

    vi.mocked(invoke).mockResolvedValueOnce('/cached/thumb.png')
    vi.mocked(convertFileSrc).mockReturnValueOnce('https://asset.localhost/cached/thumb.png')

    const result = await generatePdfThumbnail('/path/to/document.pdf', 'existing-asset')

    expect(result).toBe('https://asset.localhost/cached/thumb.png')
  })
})

describe('asset duplication', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('assigns the next copy suffix from the original base name', () => {
    expect(
      getNextAssetCopyName('C:/assets/document_c2.pdf', [
        'C:/assets/document.pdf',
        'C:/assets/document_c1.pdf',
        'C:/assets/document_c2.pdf',
      ])
    ).toBe('document_c3.pdf')
  })

  it('keeps numbering across edited image versions and format changes', () => {
    expect(
      getNextAssetCopyName('C:/assets/photo_c1_v3.png', [
        'C:/assets/photo.jpg',
        'C:/assets/photo_c1_v3.png',
        'C:/assets/photo_c2.webp',
      ])
    ).toBe('photo_c3.png')
  })

  it('copies the selected file without changing its extension', async () => {
    const { copyFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(copyFile).mockResolvedValue(undefined)
    vi.spyOn(crypto, 'randomUUID').mockReturnValue('11111111-1111-4111-8111-111111111111')

    const result = await duplicateAssetFile('C:\\assets\\scan.pdf', [
      'C:\\assets\\scan.pdf',
    ])

    expect(result).toEqual({
      name: 'scan_c1.pdf',
      path: 'C:\\assets\\11111111-1111-4111-8111-111111111111_scan_c1.pdf',
    })
    expect(copyFile).toHaveBeenCalledWith('C:\\assets\\scan.pdf', result.path)
  })
})

describe('loadAudioPreviewBlob', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('prepares the preview through Rust and reads the cached WAV', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    const { readFile } = await import('@tauri-apps/plugin-fs')

    vi.mocked(invoke).mockResolvedValueOnce('C:\\app-data\\audio-previews\\asset.wav')
    vi.mocked(readFile).mockResolvedValueOnce(new Uint8Array([1, 2, 3]))

    const blob = await loadAudioPreviewBlob('C:\\app-data\\assets\\source.wav')

    expect(invoke).toHaveBeenCalledWith('prepare_audio_preview', {
      assetPath: 'C:\\app-data\\assets\\source.wav',
    })
    expect(readFile).toHaveBeenCalledWith('C:\\app-data\\audio-previews\\asset.wav')
    expect(blob.type).toBe('audio/wav')
    expect(await blob.arrayBuffer()).toEqual(new Uint8Array([1, 2, 3]).buffer)
  })
})

describe('deletePdfThumbnail', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('calls invoke with assetId', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    vi.mocked(invoke).mockResolvedValueOnce(undefined)

    await deletePdfThumbnail('asset-456')

    expect(invoke).toHaveBeenCalledWith('delete_pdf_thumbnail', { assetId: 'asset-456' })
  })

  it('propagates errors from invoke', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    vi.mocked(invoke).mockRejectedValueOnce(new Error('Thumbnail deletion failed'))

    await expect(deletePdfThumbnail('asset-789')).rejects.toThrow('Thumbnail deletion failed')
  })
})
