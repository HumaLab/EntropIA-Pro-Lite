import { fireEvent, render, screen } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const { zoteroStore } = vi.hoisted(() => {
  const snapshot = {
    status: { state: 'available' },
    probing: false,
    loading: false,
    query: '',
    entries: [
      {
        key: '37C8RJP8',
        itemVersion: 9756,
        libraryType: 'user',
        libraryId: '0',
        title: 'Los orígenes',
        authors: 'Moore',
        year: '1973',
        csl_json: JSON.stringify({
          id: 'moore1973',
          type: 'book',
          title: 'Los orígenes',
          author: [{ family: 'Moore', given: 'Barrington' }],
          issued: { 'date-parts': [[1973]] },
        }),
      },
    ],
    loaded: 1,
    total: 1,
    error: null,
  }

  return {
    zoteroStore: {
      snapshot,
      subscribe: vi.fn((run: (value: typeof snapshot) => void) => {
        run(snapshot)
        return () => {}
      }),
      connect: vi.fn(async () => {}),
      sync: vi.fn(async () => {}),
      search: vi.fn(),
      searchLibrary: vi.fn(async () => {}),
    },
  }
})

vi.mock('$lib/writing-zotero', () => ({ writingZotero: zoteroStore }))

import WritingZoteroTab from './WritingZoteroTab.svelte'

describe('the Zotero listing citation seam', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('inserts native Zotero identity separately from the CSL id', async () => {
    const oncite = vi.fn(() => 'citation-1')
    const csl = zoteroStore.snapshot.entries[0]!.csl_json

    render(WritingZoteroTab, { props: { oncite } })

    expect(await screen.findByText('Los orígenes')).toBeInTheDocument()
    await fireEvent.click(screen.getByRole('button', { name: 'Citar' }))

    expect(oncite).toHaveBeenCalledWith({
      itemKey: '37C8RJP8',
      itemVersion: 9756,
      libraryType: 'user',
      libraryId: '0',
      metadataSnapshot: csl,
    })
  })
})
