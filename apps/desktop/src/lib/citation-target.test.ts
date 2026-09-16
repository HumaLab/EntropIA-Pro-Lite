import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { resolveCitationTarget, type CitationAnchor } from './citation-target'
import { hashSourceText } from './source-selection'

/**
 * Returning to the source (plan-editor.md §10.2).
 *
 * The interesting cases are all step 5: what happens when the anchor no longer
 * resolves. A citation must never be removed because its source moved — §10.3
 * is explicit that a citation outlives its source — so every one of these ends
 * with the citation intact and the honest thing on screen.
 */

const PAGE = 'el molino de viento giraba despacio sobre la loma'
const QUOTED = PAGE.slice(3, 9) // 'molino'

async function anchorFor(overrides: Partial<CitationAnchor> = {}): Promise<CitationAnchor> {
  return {
    assetId: 'as1',
    pageNumber: 12,
    startChar: 3,
    endChar: 9,
    quotedText: QUOTED,
    sourceTextHash: await hashSourceText(QUOTED),
    ...overrides,
  }
}

describe('resolveCitationTarget', () => {
  it('opens and highlights when the source still says what was cited', async () => {
    const target = await resolveCitationTarget(await anchorFor(), {
      assetExists: true,
      extractedText: PAGE,
    })

    expect(target).toMatchObject({
      integrity: 'valid',
      canOpen: true,
      canHighlight: true,
      assetId: 'as1',
      pageNumber: 12,
      start: 3,
      end: 9,
    })
  })

  /**
   * The case a stored position cannot catch: the range still resolves, it just
   * resolves to different words. Only the content hash can tell.
   */
  it('opens without highlighting when the words at that range changed', async () => {
    const edited = 'el granero de viento giraba despacio sobre la loma'

    const target = await resolveCitationTarget(await anchorFor(), {
      assetExists: true,
      extractedText: edited,
    })

    expect(target.integrity).toBe('source_modified')
    expect(target.canOpen).toBe(true)
    expect(target.canHighlight).toBe(false)
  })

  it('opens without highlighting when the range falls outside the text now', async () => {
    const target = await resolveCitationTarget(await anchorFor({ startChar: 400, endChar: 460 }), {
      assetExists: true,
      extractedText: PAGE,
    })

    expect(target.integrity).toBe('source_modified')
    expect(target.canHighlight).toBe(false)
  })

  it('shows only the record when the asset is gone', async () => {
    const target = await resolveCitationTarget(await anchorFor(), {
      assetExists: false,
      extractedText: null,
    })

    expect(target.integrity).toBe('source_missing')
    expect(target.canOpen).toBe(false)
    expect(target.canHighlight).toBe(false)
  })

  it('shows only the record when the citation never named an asset', async () => {
    const target = await resolveCitationTarget(await anchorFor({ assetId: null }), {
      assetExists: true,
      extractedText: PAGE,
    })

    expect(target.integrity).toBe('source_missing')
  })

  /** A page that has never been read is not a page that changed. */
  it('opens the page but claims nothing when it has no text yet', async () => {
    const target = await resolveCitationTarget(await anchorFor(), {
      assetExists: true,
      extractedText: null,
    })

    expect(target.integrity).toBe('unverifiable')
    expect(target.canOpen).toBe(true)
    expect(target.canHighlight).toBe(false)
  })

  /**
   * An older citation recorded before the hash existed. The range is all it
   * has, so it is honoured — but not called verified, because it was not.
   */
  it('highlights an unhashed citation without claiming it is verified', async () => {
    const target = await resolveCitationTarget(await anchorFor({ sourceTextHash: null }), {
      assetExists: true,
      extractedText: PAGE,
    })

    expect(target.integrity).toBe('unverifiable')
    expect(target.canHighlight).toBe(true)
    expect(target.start).toBe(3)
  })

  it('opens the page when the citation recorded no range', async () => {
    const target = await resolveCitationTarget(
      await anchorFor({ startChar: null, endChar: null }),
      { assetExists: true, extractedText: PAGE }
    )

    expect(target.integrity).toBe('unverifiable')
    expect(target.canOpen).toBe(true)
    expect(target.canHighlight).toBe(false)
  })
})

/**
 * The four states are the column's, not a second set of names for the same
 * thing. If the migration ever grows a fifth, this is what says so.
 */
describe('the integrity vocabulary', () => {
  it('matches the values the column accepts', () => {
    const migration = readFileSync(
      resolve(
        import.meta.dirname,
        '../../../../packages/store/src/migrations/0035_writing_workspace.sql'
      ),
      'utf-8'
    )
    const at = migration.indexOf('CHECK (integrity_status IN (')
    const list = migration.slice(at + 'CHECK (integrity_status IN ('.length)
    const allowed = [...list.slice(0, list.indexOf(')')).matchAll(/'([a-z_]+)'/g)].map(
      ([, value]) => value!
    )

    const source = readFileSync(resolve(import.meta.dirname, 'citation-target.ts'), 'utf-8')
    const line = source.slice(source.indexOf('export type CitationIntegrity ='))
    const declared = [...line.slice(0, line.indexOf('\n')).matchAll(/'([a-z_]+)'/g)].map(
      ([, value]) => value!
    )

    expect(declared.sort()).toEqual(allowed.sort())
  })
})
