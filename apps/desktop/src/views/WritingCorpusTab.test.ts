import { render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import type { CorpusSnapshot } from '$lib/writing-corpus'

const { snapshot } = vi.hoisted(() => ({
  snapshot: {
    query: 'fileteros',
    searching: false,
    error: null,
    fuzzy: true,
    results: [
      {
        itemId: 'it1',
        title: '1965-04-10 - UH - Huelga STIA',
        collectionId: 'col1',
        rank: -1,
        snippet: [
          { text: '…paro de los ', isMatch: false },
          { text: 'fileteros', isMatch: true },
          { text: ' del puerto…', isMatch: false },
        ],
      },
      { itemId: 'it2', title: 'IMG_3026', collectionId: 'col1', rank: -2 },
    ],
    openItem: null,
    pages: [],
    openPageId: null,
    pageText: '',
    pageTextKind: null,
    highlight: [],
  },
}))

vi.mock('$lib/writing-corpus', async (importOriginal) => {
  const original = await importOriginal<typeof import('$lib/writing-corpus')>()
  return {
    ...original,
    writingCorpus: {
      snapshot,
      subscribe: (fn: (value: CorpusSnapshot) => void) => {
        fn(snapshot)
        return () => {}
      },
      loadPreferences: async () => {},
      search: async () => {},
      openItem: async () => {},
    },
  }
})

import WritingCorpusTab from './WritingCorpusTab.svelte'

describe('WritingCorpusTab results', () => {
  // Release 1.8.7 findings: results listed titles only.
  it('shows each hit with the passage it matched, the matched word marked', () => {
    render(WritingCorpusTab, { props: { oninsertcitation: vi.fn() } })

    const passages = screen.getAllByTestId('corpus-snippet')
    expect(passages).toHaveLength(1)
    expect(passages[0]).toHaveTextContent('…paro de los fileteros del puerto…')
    expect(screen.getByText('fileteros', { selector: 'mark' })).toBeInTheDocument()
    // A hit whose passage could not be read still lists, by its title.
    expect(screen.getByText('IMG_3026')).toBeInTheDocument()
  })
})
