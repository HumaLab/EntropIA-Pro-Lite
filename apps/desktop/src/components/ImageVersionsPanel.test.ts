import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import ImageVersionsPanel from './ImageVersionsPanel.svelte'

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn<(_command: string, _args?: unknown) => Promise<unknown>>(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('$lib/db', () => ({
  getStore: () => ({ extractions: { findByAsset: async () => undefined } }),
}))
vi.mock('$lib/layouts', () => ({ getLayoutByAsset: async () => null }))
vi.mock('$lib/file-import', () => ({
  getAssetUrl: (path: string) => `https://asset.localhost/${path}`,
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((res) => {
    resolve = res
  })
  return { promise, resolve }
}

type ListedFile = ReturnType<typeof file>

function file(path: string, version: number, current: boolean) {
  return {
    path,
    version,
    current,
    orientation: 1,
    width: 100,
    height: 200,
    sha256: null,
    modifiedMs: 1_700_000_000_000,
    size: 2048,
  }
}

const assetA = { id: 'asset-a', path: 'docs/a_v2.jpg' }
const assetB = { id: 'asset-b', path: 'docs/b.jpg' }
const filesOfA = [file('docs/a.jpg', 0, false), file('docs/a_v2.jpg', 2, true)]
const filesOfB = [file('docs/b.jpg', 0, true), file('docs/b_v3.jpg', 3, false)]

const rowPaths = () =>
  screen.queryAllByTestId('image-version').map((row) => row.getAttribute('data-path'))
const useButtons = () => screen.queryAllByRole('button', { name: 'use' })
const useButton = (path: string) =>
  within(
    screen.getAllByTestId('image-version').find((row) => row.getAttribute('data-path') === path)!
  ).getByRole('button', { name: 'use' })

describe('ImageVersionsPanel', () => {
  let listings: Record<string, Array<Promise<ListedFile[]> | ListedFile[]>>
  const onUseVersion = vi.fn()
  const translate = (key: string) => (key === 'item.imageVersionUse' ? 'use' : key)

  beforeEach(() => {
    onUseVersion.mockReset()
    listings = {}
    invokeMock.mockReset()
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command !== 'list_image_versions') return null
      const { assetId } = args as { assetId: string }
      const queue = listings[assetId] ?? []
      // The last answer of a queue repeats.
      return queue.length > 1 ? queue.shift() : queue[0]
    })
  })

  async function openOnA() {
    listings[assetA.id] = [filesOfA]
    const view = render(ImageVersionsPanel, {
      asset: assetA,
      translate: translate as never,
      onUseVersion,
    })
    await waitFor(() => {
      expect(rowPaths()).toEqual(['docs/a.jpg', 'docs/a_v2.jpg'])
    })
    return view
  }

  it('drops the versions of the previous image at once when the image changes', async () => {
    const listingOfB = deferred<ListedFile[]>()
    listings[assetB.id] = [listingOfB.promise]
    const { rerender } = await openOnA()

    await rerender({ asset: assetB })

    // The listing of B has not answered: nothing of A may be offered meanwhile.
    expect(rowPaths()).toEqual([])
    expect(useButtons()).toEqual([])
    expect(screen.getByRole('status')).toHaveTextContent('item.imageVersionsLoading')

    listingOfB.resolve(filesOfB)
    await waitFor(() => {
      expect(rowPaths()).toEqual(['docs/b.jpg', 'docs/b_v3.jpg'])
    })
    expect(onUseVersion).not.toHaveBeenCalled()
  })

  it('says which image a chosen version was listed for', async () => {
    listings[assetB.id] = [filesOfB]
    const { rerender } = await openOnA()

    await fireEvent.click(useButton('docs/a.jpg'))
    expect(onUseVersion).toHaveBeenLastCalledWith(
      expect.objectContaining({ path: 'docs/a.jpg' }),
      'asset-a'
    )

    await rerender({ asset: assetB })
    await waitFor(() => {
      expect(rowPaths()).toEqual(['docs/b.jpg', 'docs/b_v3.jpg'])
    })
    await fireEvent.click(useButton('docs/b_v3.jpg'))
    expect(onUseVersion).toHaveBeenLastCalledWith(
      expect.objectContaining({ path: 'docs/b_v3.jpg' }),
      'asset-b'
    )
  })

  it('keeps the list on screen while the same image reloads', async () => {
    const reload = deferred<ListedFile[]>()
    const { rerender } = await openOnA()
    listings[assetA.id] = [reload.promise]

    // An edit landed: same asset, another current file.
    await rerender({ asset: { id: assetA.id, path: 'docs/a.jpg' }, refreshKey: 'edited' })

    expect(rowPaths()).toEqual(['docs/a.jpg', 'docs/a_v2.jpg'])

    reload.resolve([file('docs/a.jpg', 0, true), file('docs/a_v2.jpg', 2, false)])
    await waitFor(() => {
      expect(useButton('docs/a.jpg')).toBeDisabled()
    })
    expect(useButton('docs/a_v2.jpg')).toBeEnabled()
  })
})
