import { beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

import { assetFileFacts } from './asset-file-facts'

describe('assetFileFacts', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('asks by asset id and leaves the hash out unless it is asked for', async () => {
    const facts = { orientation: 6, width: 3000, height: 4000, sha256: null }
    invokeMock.mockResolvedValue(facts)

    await expect(assetFileFacts('asset-1')).resolves.toEqual(facts)

    expect(invokeMock).toHaveBeenCalledWith('asset_file_facts', {
      assetId: 'asset-1',
      withSha: false,
    })
  })

  it('asks for the hash when there is one to compare with', async () => {
    invokeMock.mockResolvedValue({ orientation: 1, width: 10, height: 20, sha256: 'abc' })

    await assetFileFacts('asset-1', { withSha: true })

    expect(invokeMock).toHaveBeenCalledWith('asset_file_facts', {
      assetId: 'asset-1',
      withSha: true,
    })
  })
})
