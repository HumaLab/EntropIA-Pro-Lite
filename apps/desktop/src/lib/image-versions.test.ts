import { beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

import {
  imageVersionsNeedSha,
  listImageVersions,
  markImageVersions,
  type ImageVersionFile,
} from './image-versions'

function file(overrides: Partial<ImageVersionFile> & { path: string }): ImageVersionFile {
  return {
    version: 0,
    orientation: 1,
    width: 1000,
    height: 1400,
    sha256: null,
    modifiedMs: 1_700_000_000_000,
    size: 2048,
    current: false,
    ...overrides,
  }
}

const original = file({ path: 'docs/photo.jpg', sha256: 'sha-original' })
const cropped = file({
  path: 'docs/photo_v2.jpg',
  version: 2,
  width: 500,
  height: 560,
  sha256: 'sha-v2',
  current: true,
})

describe('markImageVersions', () => {
  it('marks the version whose hash is the stamp of the layout', () => {
    const marked = markImageVersions([original, cropped], {
      layout: { sourceSha256: 'sha-v2', imageWidth: 500, imageHeight: 560 },
      extraction: null,
    })

    expect(marked.map((v) => [v.path, v.matchesLayout, v.matchesText])).toEqual([
      ['docs/photo.jpg', false, false],
      ['docs/photo_v2.jpg', true, false],
    ])
  })

  it('goes by the stamp and not by the size when the layout has one', () => {
    // Same size as the layout, other bytes: an erase keeps the size.
    const erased = file({ path: 'docs/photo_v3.jpg', version: 3, sha256: 'sha-v3' })

    const marked = markImageVersions([original, erased], {
      layout: { sourceSha256: 'sha-original', imageWidth: 1000, imageHeight: 1400 },
      extraction: null,
    })

    expect(marked.map((v) => v.matchesLayout)).toEqual([true, false])
  })

  it('marks, for a layout with no stamp, the untagged version of the size it was computed on', () => {
    const sideways = file({ path: 'docs/photo.jpg', orientation: 6 })
    const upright = file({ path: 'docs/photo_v2.jpg', version: 2 })
    const small = file({ path: 'docs/photo_v3.jpg', version: 3, width: 500, height: 560 })

    const marked = markImageVersions([sideways, upright, small], {
      layout: { sourceSha256: null, imageWidth: 1000, imageHeight: 1400 },
      extraction: null,
    })

    expect(marked.map((v) => v.matchesLayout)).toEqual([false, true, false])
  })

  it('marks the version the text was read from, and none for a text with no stamp', () => {
    const stamped = markImageVersions([original, cropped], {
      layout: null,
      extraction: { sourceSha256: 'sha-original' },
    })
    expect(stamped.map((v) => v.matchesText)).toEqual([true, false])

    const unstamped = markImageVersions([original, cropped], {
      layout: null,
      extraction: { sourceSha256: null },
    })
    expect(unstamped.map((v) => v.matchesText)).toEqual([false, false])
  })

  it('marks nothing when there is neither a layout nor a text', () => {
    const marked = markImageVersions([original, cropped], { layout: null, extraction: null })

    expect(marked.some((v) => v.matchesLayout || v.matchesText)).toBe(false)
  })
})

describe('imageVersionsNeedSha', () => {
  it('asks for the hashes only when a stamp exists to compare them with', () => {
    const unstampedLayout = { sourceSha256: null, imageWidth: 1000, imageHeight: 1400 }

    expect(imageVersionsNeedSha({ layout: null, extraction: null })).toBe(false)
    expect(
      imageVersionsNeedSha({ layout: unstampedLayout, extraction: { sourceSha256: null } })
    ).toBe(false)
    expect(
      imageVersionsNeedSha({
        layout: { ...unstampedLayout, sourceSha256: 'sha-original' },
        extraction: null,
      })
    ).toBe(true)
    expect(
      imageVersionsNeedSha({ layout: unstampedLayout, extraction: { sourceSha256: 'sha-v2' } })
    ).toBe(true)
  })
})

describe('listImageVersions', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('asks by asset id and hashes nothing when neither row is stamped', async () => {
    invokeMock.mockResolvedValue([file({ path: 'docs/photo.jpg', current: true })])

    const versions = await listImageVersions('asset-1', {
      layout: { sourceSha256: null, imageWidth: 1000, imageHeight: 1400 },
      extraction: { sourceSha256: null },
    })

    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('list_image_versions', {
      assetId: 'asset-1',
      withSha: false,
    })
    expect(versions).toEqual([
      {
        ...file({ path: 'docs/photo.jpg', current: true }),
        matchesLayout: true,
        matchesText: false,
      },
    ])
  })

  it('asks for the hashes when the layout is stamped', async () => {
    invokeMock.mockResolvedValue([original, cropped])

    const versions = await listImageVersions('asset-1', {
      layout: { sourceSha256: 'sha-original', imageWidth: 1000, imageHeight: 1400 },
      extraction: { sourceSha256: 'sha-v2' },
    })

    expect(invokeMock).toHaveBeenCalledWith('list_image_versions', {
      assetId: 'asset-1',
      withSha: true,
    })
    expect(versions.map((v) => [v.matchesLayout, v.matchesText])).toEqual([
      [true, false],
      [false, true],
    ])
  })
})
