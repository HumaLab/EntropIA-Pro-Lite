import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  checkExtractionVersion,
  extractionVersionNeeds,
  extractionVersionVerdict,
} from './extraction-version'

const { assetFileFactsMock } = vi.hoisted(() => ({ assetFileFactsMock: vi.fn() }))

vi.mock('./asset-file-facts', () => ({ assetFileFacts: assetFileFactsMock }))

const file = (sha256: string | null) => ({ orientation: 1, width: 1000, height: 1400, sha256 })

describe('extraction version rule', () => {
  it('says a stamped text of an image belongs to the file with that hash', () => {
    const extraction = { sourceSha256: 'sha-a' }
    expect(extractionVersionNeeds('image', extraction)).toBe('facts_with_sha')
    expect(extractionVersionVerdict('image', extraction, file('sha-a'))).toBe('matches')
    expect(extractionVersionVerdict('image', extraction, file('sha-b'))).toBe('other_version')
  })

  it('does not vouch for a stamped text when the file has no hash to compare with', () => {
    expect(extractionVersionVerdict('image', { sourceSha256: 'sha-a' }, file(null))).toBe(
      'other_version'
    )
  })

  it.each([null, ''])('says nothing about a text with no stamp (%j)', (sourceSha256) => {
    expect(extractionVersionNeeds('image', { sourceSha256 })).toBe('nothing')
    expect(extractionVersionVerdict('image', { sourceSha256 }, file('sha-b'))).toBe('matches')
    expect(extractionVersionVerdict('image', { sourceSha256 }, null)).toBe('matches')
  })

  it('never checks anything that is not an image asset', () => {
    const extraction = { sourceSha256: 'sha-of-another-file' }
    expect(extractionVersionNeeds('pdf', extraction)).toBe('nothing')
    expect(extractionVersionVerdict('pdf', extraction, file('sha-b'))).toBe('matches')
    expect(extractionVersionNeeds('audio', extraction)).toBe('nothing')
  })
})

describe('checkExtractionVersion', () => {
  beforeEach(() => {
    assetFileFactsMock.mockReset()
  })

  it('hashes the current file of an image whose text is stamped', async () => {
    assetFileFactsMock.mockResolvedValue(file('sha-b'))

    await expect(
      checkExtractionVersion({ id: 'a1', type: 'image' }, { sourceSha256: 'sha-a' })
    ).resolves.toBe('other_version')
    expect(assetFileFactsMock).toHaveBeenCalledWith('a1', { withSha: true })

    assetFileFactsMock.mockResolvedValue(file('sha-a'))
    await expect(
      checkExtractionVersion({ id: 'a1', type: 'image' }, { sourceSha256: 'sha-a' })
    ).resolves.toBe('matches')
  })

  it('reads no file for a text with no stamp, nor for a PDF', async () => {
    await expect(
      checkExtractionVersion({ id: 'a1', type: 'image' }, { sourceSha256: null })
    ).resolves.toBe('matches')
    await expect(
      checkExtractionVersion({ id: 'p1', type: 'pdf' }, { sourceSha256: 'sha-a' })
    ).resolves.toBe('matches')
    expect(assetFileFactsMock).not.toHaveBeenCalled()
  })

  it('answers unknown, without throwing, when the file cannot be read', async () => {
    assetFileFactsMock.mockRejectedValue(new Error('Failed to read docs/photo.jpg'))

    await expect(
      checkExtractionVersion({ id: 'a1', type: 'image' }, { sourceSha256: 'sha-a' })
    ).resolves.toBe('unknown')
  })
})
