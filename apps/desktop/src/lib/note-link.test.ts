import { describe, expect, it } from 'vitest'
import { buildNoteLink, resolveNoteLink } from './note-link'
import { hashSourceText } from './source-selection'

/**
 * Linking a note, and what happens when it moves on (plan-editor.md §13, §13.1).
 *
 * Every case here ends with the manuscript untouched. §13 says a diverged note
 * is reported and never overwrites the article; §13.1 says a deleted note is
 * reported without removing the text or the snapshot. So what is asserted is
 * always what was *noticed*, never what was changed.
 */

const NOTE = { id: 'n1', itemId: 'it1', content: 'los obreros del filet reanudaron' }

async function linkFor(overrides: Record<string, unknown> = {}) {
  return { ...(await buildNoteLink(NOTE)), ...overrides } as {
    noteId: string | null
    contentSnapshot: string | null
    contentHash: string | null
  }
}

describe('buildNoteLink', () => {
  it('records the note, its text and a fingerprint of it', async () => {
    const link = await buildNoteLink(NOTE)

    expect(link).toMatchObject({
      noteId: 'n1',
      itemId: 'it1',
      contentSnapshot: NOTE.content,
    })
    expect(link.contentHash).toBe(await hashSourceText(NOTE.content))
  })

  /**
   * The snapshot is what the writer put in their manuscript, so it is stored
   * rather than re-read. A link that fetched the note's current text would be
   * the automatic overwrite §13 forbids.
   */
  it('stores the text rather than a reference to fetch it', async () => {
    const link = await buildNoteLink(NOTE)

    expect(link.contentSnapshot).toBe(NOTE.content)
  })
})

describe('resolveNoteLink', () => {
  it('is quiet while the note still says what was linked', async () => {
    const state = await resolveNoteLink(await linkFor(), {
      exists: true,
      content: NOTE.content,
    })

    expect(state).toEqual({ integrity: 'valid', current: null })
  })

  /** The whole point of the hash: notice, and offer — do not apply. */
  it('reports the divergence and offers the new text without applying it', async () => {
    const changed = 'los obreros del filet siguen de paro'

    const state = await resolveNoteLink(await linkFor(), { exists: true, content: changed })

    expect(state.integrity).toBe('source_modified')
    expect(state.current).toBe(changed)
  })

  /** §13.1: a deleted note is reported; the text and snapshot stay. */
  it('reports a note that is gone', async () => {
    const state = await resolveNoteLink(await linkFor(), { exists: false, content: null })

    expect(state.integrity).toBe('source_missing')
  })

  it('reports a link that never named a note', async () => {
    const state = await resolveNoteLink(await linkFor({ noteId: null }), {
      exists: true,
      content: NOTE.content,
    })

    expect(state.integrity).toBe('source_missing')
  })

  /**
   * A link made before the hash existed. It is not claimed to agree and not
   * claimed to have diverged, because neither is known.
   */
  it('claims nothing about a link with no fingerprint', async () => {
    const state = await resolveNoteLink(await linkFor({ contentHash: null }), {
      exists: true,
      content: 'cualquier otra cosa',
    })

    expect(state).toEqual({ integrity: 'unverifiable', current: null })
  })

  it('does not offer the note text when nothing diverged', async () => {
    const state = await resolveNoteLink(await linkFor(), {
      exists: true,
      content: NOTE.content,
    })

    expect(state.current).toBeNull()
  })
})

/**
 * The asymmetry is the feature. There is no path from copying a note's words to
 * acquiring a live relationship with it, because a copy has no attributes at
 * all — it is text.
 */
describe('copying is not linking', () => {
  it('has no shape a link could be mistaken for', async () => {
    const link = await buildNoteLink(NOTE)

    // A copy is the note's content and nothing else. There is no function here
    // that turns that string into something `resolveNoteLink` would accept.
    expect(Object.keys(link).sort()).toEqual([
      'contentHash',
      'contentSnapshot',
      'itemId',
      'noteId',
    ])
  })
})
