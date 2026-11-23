import { beforeEach, expect, it, vi } from 'vitest'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import { fetchSearchSnippets } from './search-snippets'

beforeEach(() => invoke.mockReset())

it('keys each passage by its item', async () => {
  const segments = [
    { text: 'La ', isMatch: false },
    { text: 'asamblea', isMatch: true },
  ]
  invoke.mockResolvedValueOnce([{ itemId: 'a', segments }])

  const snippets = await fetchSearchSnippets(['a', 'b'], ['asamblea'])

  expect(invoke).toHaveBeenCalledWith('fts_snippets', {
    itemIds: ['a', 'b'],
    terms: ['asamblea'],
  })
  expect(snippets.get('a')).toEqual(segments)
  expect(snippets.has('b')).toBe(false)
})

it('asks nothing without results or terms', async () => {
  expect((await fetchSearchSnippets([], ['asamblea'])).size).toBe(0)
  expect((await fetchSearchSnippets(['a'], [])).size).toBe(0)
  expect(invoke).not.toHaveBeenCalled()
})

it('leaves the results without passages when the call fails', async () => {
  invoke.mockRejectedValueOnce('DB lock error')
  expect((await fetchSearchSnippets(['a'], ['asamblea'])).size).toBe(0)
})
